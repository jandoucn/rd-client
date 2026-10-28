#![cfg(windows)]

#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/health.rs"]
mod health;
#[path = "../src/management_agent/health_windows.rs"]
mod health_windows;
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/package_download.rs"]
mod package_download;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/service_control.rs"]
mod service_control;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const BUILD: u64 = 2026101501;
const AGENT: &str = r"C:\ProgramData\RustDesk\ManagementAgent";

fn lab() {
    base::lab_guard::require_lab_os("win10");
    assert_eq!(
        std::env::var("RUSTDESK_T01_DELIVERY").as_deref(),
        Ok("ordinary-msi-1501")
    );
}

fn input() -> (
    serde_json::Value,
    hbb_common::UpdateManifest,
    String,
    PathBuf,
) {
    lab();
    let root = PathBuf::from(r"C:\RustDeskAgentLab\t01-delivery-input");
    let reply: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("resolved.json")).unwrap()).unwrap();
    let key = reply["target_key"].as_str().unwrap().to_owned();
    let reference = reply["package_id"].as_str().unwrap();
    let manifest = package_gate::bind_resolved(&reply, reference, &key, BUILD)
        .expect("INPUT BLOCKED: API package reference/build mismatch; not a business RED");
    let target = package_gate::verify(&manifest, &key, BUILD).expect(
        "INPUT BLOCKED: must use an unchanged formally signed manifest; not a business RED",
    );
    assert_eq!(
        base::update::update_target_release(&manifest, target)
            .unwrap()
            .1,
        BUILD
    );
    let rollback = package_gate::bind_resolved(
        &reply["rollback"],
        reply["rollback"]["package_id"].as_str().unwrap(),
        &key,
        BUILD,
    )
    .unwrap();
    package_gate::verify(&rollback, &key, BUILD).unwrap();
    assert_eq!(
        reply["rollback"]["package_id"], reply["package_id"],
        "same-build reinstall fixture"
    );
    let package = root.join(package_gate::cache_name(&target.primary, "msi").unwrap());
    let _lock = package_gate::lock_verified_file(&package, target).expect(
        "INPUT BLOCKED: original release MSI missing or differs from signature; not a business RED",
    );
    (reply, manifest, key, package)
}

fn digest(path: &Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    hex::encode(hash.finalize())
}

fn healthy(key: &str) -> serde_json::Value {
    use windows_service::service::{ServiceStartType, ServiceState};
    let client = service_control::Contract {
        name: "RustDesk".into(),
        executable: r"C:\Program Files\RustDesk\RustDesk.exe".into(),
        arguments: vec!["--service".into()],
        start_type: ServiceStartType::AutoStart,
    };
    let agent = service_control::Contract {
        name: "RustDeskManagementAgent".into(),
        executable: Path::new(AGENT).join("rustdesk-management-agent.exe"),
        arguments: vec![
            "--service".into(),
            "--edition".into(),
            if key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            }
            .into(),
            "--package-kind".into(),
            "msi".into(),
            "--normal-install-path".into(),
            r"C:\Program Files\RustDesk".into(),
        ],
        start_type: ServiceStartType::AutoStart,
    };
    let mut services = Vec::new();
    for contract in [&client, &agent] {
        let observation =
            service_control::inspect(contract).expect("SETUP BLOCKED: fixed service contract");
        assert_eq!(observation.state, ServiceState::Running);
        services.push(serde_json::json!({"name":contract.name,"pid":observation.pid}));
    }
    let response = health_windows::probe_blocking(
        r"\\.\pipe\RustDesk\management-health-v2",
        &client.executable,
        BUILD,
        Duration::from_secs(3),
    )
    .expect("SETUP BLOCKED: real 1501 IPC health required; not producer RED");
    assert_eq!(
        response.no_active_sessions,
        Some(true),
        "SETUP BLOCKED: active or unknown sessions"
    );
    serde_json::json!({"observed_at_utc":chrono::Utc::now().to_rfc3339(),
        "observer_pid":std::process::id(),"services":services,"client_health_pid":response.pid})
}

#[test]
#[ignore = "Win10 owner only; formal inputs; no installer, service mutation, or network"]
fn t01_formal_approval_tamper_refused_preserves_original_inputs() {
    let (reply, manifest, key, package) = input();
    let before = digest(&package);
    let mut changed = manifest.clone();
    changed.targets.get_mut(&key).unwrap().sha256 = "0".repeat(64);
    assert_eq!(
        package_gate::verify(&changed, &key, BUILD).unwrap_err(),
        "install manifest signature verification failed"
    );
    let mut changed_reply = reply.clone();
    changed_reply["target_key"] = "windows-x86_64-exe-standard".into();
    assert_eq!(
        package_gate::bind_resolved(
            &changed_reply,
            reply["package_id"].as_str().unwrap(),
            &key,
            BUILD
        )
        .unwrap_err(),
        "approved package command binding mismatch"
    );
    assert_eq!(digest(&package), before);
    assert!(!Path::new(AGENT)
        .join("release-cache/first-install.json")
        .exists());
}

#[test]
#[ignore = "Win10 owner only; missing-approval preservation observation; reinstall needs independent review and baseline"]
fn t01_ordinary_msi_missing_approval_preserves_installation_observation() {
    let (reply, manifest, key, package) = input();
    let agent = Path::new(AGENT);
    let sidecar = agent.join("release-cache/first-install.json");
    assert!(
        !sidecar.exists(),
        "SETUP BLOCKED: do not remove existing approval to manufacture RED"
    );
    assert!(
        !agent.join("rescue").exists(),
        "SETUP BLOCKED: preserve existing rescue assets"
    );
    assert!(
        !agent.join("transactions").exists(),
        "SETUP BLOCKED: use ordinary-install baseline without transactions"
    );
    let state_path = agent.join("state/agent.json");
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(state["target_key"], key);
    assert_eq!(state["management_agent_enabled"], true);
    assert_eq!(state["disabled"], false);
    let before_health = healthy(&key);
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        assert!(
            !agent.join("state").join(marker).exists(),
            "SETUP BLOCKED: local gate {marker}"
        );
    }
    let identity_path = Path::new(
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
    );
    let identity =
        std::fs::read(identity_path).expect("SETUP BLOCKED: ordinary install identity required");
    let config_path = identity_path.with_file_name("RustDesk2.toml");
    let config = std::fs::read(&config_path).unwrap();
    let intent: hbb_common::toml::Value =
        hbb_common::toml::from_str(std::str::from_utf8(&config).unwrap()).unwrap();
    assert!(
        matches!(
            intent
                .get("options")
                .and_then(|v| v.get("stop-service"))
                .and_then(|v| v.as_str()),
            None | Some("" | "N")
        ),
        "SETUP BLOCKED: user stopped service"
    );
    let protected = [
        PathBuf::from(r"C:\Program Files\RustDesk\RustDesk.exe"),
        PathBuf::from(r"C:\Program Files\RustDesk\librustdesk.dll"),
        agent.join("rustdesk-management-agent.exe"),
    ];
    let hashes = protected.iter().map(|p| digest(p)).collect::<Vec<_>>();
    let target = package_gate::verify(&manifest, &key, BUILD).unwrap();
    let metadata = msi_metadata::inspect(&package).unwrap();
    let registered = msi_metadata::installed_source(&metadata.product_code).expect(
        "SETUP BLOCKED: formal 1501 MSI must already be installed on disposable Win10 baseline",
    );
    let _registered = package_gate::lock_verified_file(&registered.source, target).unwrap();
    assert_eq!(
        registered.package_name,
        package.file_name().unwrap().to_str().unwrap()
    );

    let evidence =
        Path::new(r"C:\RustDeskAgentLab").join(format!("t01-delivery-{}", uuid::Uuid::new_v4()));
    workspace::create(&evidence).unwrap();
    std::fs::write(
        evidence.join("resolved.json"),
        serde_json::to_vec(&reply).unwrap(),
    )
    .unwrap();
    println!("T01 evidence: {}", evidence.display());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let source = package.clone();
    let name = package.file_name().unwrap().to_str().unwrap().to_owned();
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(65);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(e) => panic!("INPUT BLOCKED: local package fixture: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() < 8192);
        }
        assert!(String::from_utf8_lossy(&request).starts_with(&format!("GET /{name} HTTP/1.1\r\n")));
        let mut file = std::fs::File::open(source).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            file.metadata().unwrap().len()
        )
        .unwrap();
        std::io::copy(&mut file, &mut stream).unwrap();
    });
    let downloaded =
        package_download::download_fixture(&evidence.join("download"), target, address);
    server.join().unwrap();
    let (downloaded, locked) =
        downloaded.expect("INPUT BLOCKED: formal signed local download failed");
    assert_eq!(digest(&downloaded), target.sha256);
    drop(locked);
    drop(_registered);
    let log = evidence.join("ordinary-msi.log");
    let report_path = agent.join("first-install-status/last-check.json");
    let exit = std::process::Command::new(r"C:\Windows\System32\msiexec.exe")
        .arg("/i")
        .arg(&downloaded)
        .args(["/qn", "/norestart", "/L*v"])
        .arg(&log)
        .status()
        .unwrap();
    assert_eq!(
        exit.code(),
        Some(0),
        "SETUP BLOCKED: install failure/reboot is not producer RED: {exit}"
    );
    let install_completed_at_utc = chrono::Utc::now().to_rfc3339();
    // 安装过程中发布的报告只作为基线；必须等待安装返回后的下一次发布。
    let baseline_report_time = std::fs::metadata(&report_path)
        .and_then(|m| m.modified())
        .ok();
    let baseline_captured_at_utc = chrono::Utc::now().to_rfc3339();
    let registered = msi_metadata::installed_source(&metadata.product_code).unwrap();
    let _registered = package_gate::lock_verified_file(&registered.source, target).unwrap();
    let deadline = Instant::now() + Duration::from_secs(360);
    let (report, published_time) = loop {
        let modified = std::fs::metadata(&report_path)
            .and_then(|m| m.modified())
            .ok();
        if modified.is_some() && modified != baseline_report_time {
            if let Ok(bytes) = std::fs::read(&report_path) {
                let after_read = std::fs::metadata(&report_path)
                    .and_then(|m| m.modified())
                    .ok();
                if after_read == modified {
                    if let Ok(report) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        break (report, modified);
                    }
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "SETUP BLOCKED: no actual service worker report; not business RED"
        );
        std::thread::sleep(Duration::from_millis(250));
    };
    let after_health = healthy(&key);
    assert!(
        std::fs::read(identity_path).unwrap() == identity,
        "identity preservation failed (contents redacted)"
    );
    assert!(
        std::fs::read(&config_path).unwrap() == config,
        "configuration preservation failed (contents redacted)"
    );
    for (path, hash) in protected.iter().zip(&hashes) {
        assert_eq!(&digest(path), hash);
    }
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).unwrap();
    assert_eq!(
        after["agent_installation_id"],
        state["agent_installation_id"]
    );
    std::fs::write(evidence.join("observed.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "observation_kind":"missing_approval_preservation","worker_report":report,"sidecar_exists":sidecar.exists(),
        "install_completed_at_utc":install_completed_at_utc,"baseline_captured_at_utc":baseline_captured_at_utc,
        "baseline_report_modified_unix_ms":baseline_report_time.and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t|t.as_millis()),
        "published_report_modified_unix_ms":published_time.and_then(|t|t.duration_since(std::time::UNIX_EPOCH).ok()).map(|t|t.as_millis()),
        "report_observed_at_utc":chrono::Utc::now().to_rfc3339(),"observer_pid":std::process::id(),
        "before_health":before_health,"after_health":after_health,
        "installation_id":after["agent_installation_id"],"package_id":reply["package_id"],
        "identity_preserved":true,"installed_files_preserved":true
    })).unwrap()).unwrap();
    assert!(
        !sidecar.exists(),
        "SETUP CHANGED: approval appeared; missing-approval observation no longer applies"
    );
    assert_eq!(report["state"], "unavailable");
    assert_eq!(report["error_code"], "waiting_approval");
    assert_eq!(report["ready"], false);
    assert!(!agent.join("rescue/current").exists());
}
