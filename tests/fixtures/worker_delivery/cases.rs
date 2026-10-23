#![cfg(windows)]

// The build driver extracts Worker/start_checked verbatim from the current production source.
include!(env!("T01_WORKER_EXTRACT"));

#[path = "../../../src/management_agent/state_store.rs"]
mod state_store;

#[test]
#[ignore = "Win10 owner; real production worker scheduling, isolated prepare/layout boundaries"]
fn t01_real_worker_must_deliver_available_exact_approval_before_prepare() {
    base::lab_guard::require_lab_os("win10");
    assert_eq!(std::env::var("RUSTDESK_T01_PRODUCER").as_deref(), Ok("red-formal-sos-1501"));
    let root = std::path::Path::new(r"C:\RustDeskAgentLab")
        .join(format!("t01-worker-delivery-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir(&root).unwrap();
    let reply: serde_json::Value = serde_json::from_slice(&std::fs::read(
        r"C:\RustDeskAgentLab\t01-delivery-input\resolved.json").unwrap()).unwrap();
    let manifest: hbb_common::UpdateManifest = serde_json::from_value(reply["manifest"].clone()).unwrap();
    let target = package_gate::verify(&manifest, "windows-x86_64-msi-sos", 2026101501).unwrap();
    assert_eq!(base::update::update_target_release(&manifest, target).unwrap().1, 2026101501);
    std::fs::write(root.join("available-manifest.json"), serde_json::to_vec(&reply["manifest"]).unwrap()).unwrap();
    std::fs::write(root.join("identity"), b"retained identity").unwrap();
    std::fs::write(root.join("config"), b"retained config").unwrap();
    let before = ["available-manifest.json", "identity", "config"].map(|p| std::fs::read(root.join(p)).unwrap());
    *ROOT.lock().unwrap() = Some(root.clone());
    let worker = worker_module::first_install::start().unwrap();
    worker.notify().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let report = root.join("first-install-status/last-check.json");
    while !report.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    drop(worker);
    assert!(report.exists(), "SETUP BLOCKED: production worker did not publish report");
    let observed: serde_json::Value = serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert!(["available-manifest.json", "identity", "config"].map(|p| std::fs::read(root.join(p)).unwrap()) == before,
        "approved input and identity/config bytes retained");
    println!("T01 worker fixture: {}; observed={observed}", root.display());
    assert_eq!(PREPARES.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(root.join("release-cache/first-install.json").exists(),
        "BUSINESS RED: production worker reached prepare without delivering available exact-build approval");
}

#[path = "../../../src/management_agent/package_gate.rs"]
mod package_gate;
static ROOT: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);
static PREPARES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
