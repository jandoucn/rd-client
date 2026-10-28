#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
// 平台代码里有 `crate::platform::windows::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod platform {
    pub mod windows {
        pub(crate) use crate::system_dirs;
    }
}
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/platform/windows_exe_migration.rs"]
mod migration;
#[path = "../libs/portable/src/update_args.rs"]
mod update_args;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;
use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
    UpdateManifest, UpdateTarget,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[path = "../src/management_agent/package_gate.rs"]
mod signature_gate;

#[test]
fn explicit_pending_event_refused_before_any_approval_io() {
    use base::update::{PendingUpdateEvent, UpdateSource};
    let event = PendingUpdateEvent { transaction_id: "a".repeat(32), from_version: "1.5.3".into(),
        from_build_seq: 2026100504, version: "1.5.3".into(), build_seq: 2026100505,
        source: UpdateSource::Mirror, command_id: Some("original-command".into()) };
    let mut args = vec!["--update".into()];
    args.extend(event.cli_args());
    assert_eq!(PendingUpdateEvent::from_cli_args(&args), Some(event));
    assert_eq!(migration::run(&args).unwrap_err(), "EXE update-event consumer unavailable; transaction not started");
}

#[test]
fn approval_directory_is_rejected_as_non_regular() {
    let args = vec!["--management-agent-approved-manifest".into(), r"C:\RustDeskAgentLab".into(),
        "--management-agent-approved-sha256".into(), "a".repeat(64)];
    assert_eq!(migration::approved_manifest(&args).unwrap_err(), "EXE approval must be a regular non-reparse file");
}

#[test]
fn approval_local_file_bounds_and_device_paths() {
    for path in [r"\\server\share\manifest.json", r"\\?\C:\manifest.json", r"\\.\pipe\manifest", r"C:relative", r"C:\file:stream"] {
        assert_eq!(migration::read_local_approval(Path::new(path)).unwrap_err(), "EXE approval requires a local disk path");
    }
    let path = std::env::temp_dir().join(format!("approval-{}.json", uuid::Uuid::new_v4()));
    let old = vec![b' ';262145];
    std::fs::write(&path,&old).unwrap();
    assert_eq!(migration::read_local_approval(&path).unwrap_err(), "EXE approval exceeds size limit");
    assert_eq!(std::fs::read(&path).unwrap(),old);
    std::fs::write(&path,b"{}").unwrap();
    assert_eq!(migration::read_local_approval(&path).unwrap(),b"{}");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn approval_rejects_real_junction_without_reading_target() {
    let root = std::env::temp_dir().join(format!("approval-junction-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let target = root.join("target");
    let link = root.join("link");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("manifest.json"),b"preserve-target").unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/d","/c","mklink","/J"]).arg(&link).arg(&target).status().unwrap();
    assert!(status.success(),"actual junction required; not a skipped fixture");
    let result = migration::read_local_approval(&link.join("manifest.json"));
    std::fs::remove_dir(&link).unwrap();
    assert_eq!(result.unwrap_err(),"EXE approval must be a regular non-reparse file");
    assert_eq!(std::fs::read(target.join("manifest.json")).unwrap(),b"preserve-target");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_entry_requires_fixed_approval_before_installed_state_or_network() {
    let args = vec!["--update".into(), "--management-agent-outer-package".into(),
        r"C:\RustDeskAgentLab\not-executed.exe".into()];
    assert_eq!(migration::run(&args).unwrap_err(), "EXE fixed approval arguments missing or ambiguous");
}

#[test]
fn signature_encodings_keep_exact_cryptographic_contract() {
    use hbb_common::base64::engine::general_purpose::URL_SAFE_NO_PAD;
    hbb_common::sodiumoxide::init().unwrap();
    let (public, secret) = sign::gen_keypair();
    let key = "windows-x86_64-exe-standard";
    let mut manifest = UpdateManifest {
        schema: 2, product: "rustdesk-yan".into(), edition: "multi".into(),
        channel: "stable".into(), ..Default::default()
    };
    let mut target = UpdateTarget {
        version: "1.5.3".into(), build_seq: 2026100505, source_commit: "fixture".into(),
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/package.exe".into(),
        size: 13, sha256: hex::encode(Sha256::digest(b"exact-package")),
        signature_key_id: "fixture-key".into(), ..Default::default()
    };
    let signature = sign::sign_detached(
        &base::update::update_signature_payload(&manifest, key, &target).unwrap(), &secret);
    for encoded in [STANDARD.encode(signature), URL_SAFE_NO_PAD.encode(signature)] {
        target.signature = encoded.clone();
        manifest.targets.insert(key.into(), target.clone());
        signature_gate::verify_with_key(&manifest, key, 2026100504, "fixture-key", public.as_ref()).unwrap();
        for invalid in [format!("{encoded}\n"), format!("!{encoded}"), "AA".into()] {
            let mut bad = manifest.clone();
            bad.targets.get_mut(key).unwrap().signature = invalid;
            assert!(signature_gate::verify_with_key(&bad, key, 2026100504, "fixture-key", public.as_ref()).is_err());
            assert_eq!(manifest.targets[key].signature, encoded);
        }
        let mut bad = manifest.clone();
        bad.targets.get_mut(key).unwrap().size += 1;
        assert_eq!(signature_gate::verify_with_key(&bad, key, 2026100504, "fixture-key", public.as_ref()).unwrap_err(),
            "install manifest signature verification failed");
        let (other, _) = sign::gen_keypair();
        assert_eq!(signature_gate::verify_with_key(&manifest, key, 2026100504, "fixture-key", other.as_ref()).unwrap_err(),
            "install manifest signature verification failed");
    }
}

#[test]
fn wrapper_preserves_legacy_update_with_pending_event_and_rejects_ambiguous_outer() {
    let path = r"C:\RustDeskAgentLab\signed package.exe";
    assert_eq!(
        migration::outer_argument(&[
            "--update".into(),
            "--management-agent-outer-package".into(),
            path.into(),
            "--update-event".into(),
            "a".repeat(32),
            "1.5.3".into(),
            "2026100504".into(),
            "1.5.3".into(),
            "2026100505".into(),
            "primary".into()
        ])
        .unwrap(),
        PathBuf::from(path)
    );
    for args in [
        vec!["--update".into()],
        vec![
            "--update".into(),
            "--management-agent-outer-package".into(),
            "relative.exe".into(),
        ],
        vec![
            "--update".into(),
            "--management-agent-outer-package".into(),
            path.into(),
            "--management-agent-outer-package".into(),
            path.into(),
        ],
    ] {
        assert!(migration::outer_argument(&args).is_err());
    }
}

#[test]
fn signed_catalog_exact_build_kind_edition_and_bytes_or_refuse() {
    base::lab_guard::require_lab_os("win10");
    hbb_common::sodiumoxide::init().unwrap();
    let path = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("migration-{}.exe", uuid::Uuid::new_v4().simple()));
    std::fs::write(&path, b"exact-package").unwrap();
    let (public, secret) = sign::gen_keypair();
    let key = "windows-x86_64-exe-standard";
    let mut manifest = UpdateManifest {
        schema: 2,
        product: "rustdesk-yan".into(),
        edition: "multi".into(),
        channel: "stable".into(),
        ..Default::default()
    };
    let mut target = UpdateTarget {
        version: "1.5.3".into(),
        build_seq: 2026100505,
        source_commit: "fixture".into(),
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/package.exe".into(),
        size: 13,
        sha256: hex::encode(Sha256::digest(b"exact-package")),
        signature_key_id: "fixture-key".into(),
        ..Default::default()
    };
    target.signature = STANDARD.encode(sign::sign_detached(
        &base::update::update_signature_payload(&manifest, key, &target).unwrap(),
        &secret,
    ));
    manifest.targets.insert(key.into(), target);
    let approval = path.with_extension("json");
    let approval_bytes = serde_json::to_vec(&manifest).unwrap();
    std::fs::write(&approval, &approval_bytes).unwrap();
    let approval_args = vec!["--management-agent-approved-manifest".into(),
        approval.to_string_lossy().into_owned(), "--management-agent-approved-sha256".into(),
        hex::encode(Sha256::digest(&approval_bytes))];
    let fixed = migration::approved_manifest(&approval_args).unwrap();
    let plan = serde_json::json!({"transaction_id":"a".repeat(32),"installation_id":"b".repeat(32),
        "target_key":key,"target_version":"1.5.3","target_build_seq":2026100505u64,
        "package":{"relative_path":"package.exe","size":13,"sha256":manifest.targets[key].sha256}});
    let binding = migration::approval_binding(&serde_json::to_vec(&plan).unwrap(), &approval_bytes,
        &"a".repeat(32), &"b".repeat(32), key,"1.5.3",2026100505).unwrap();
    let bound:serde_json::Value=serde_json::from_slice(&binding).unwrap();
    assert_eq!(bound["approval_sha256"],hex::encode(Sha256::digest(&approval_bytes)));
    assert!(migration::approval_binding(&serde_json::to_vec(&plan).unwrap(), &approval_bytes,
        &"a".repeat(32), &"c".repeat(32), key,"1.5.3",2026100505).is_err());
    migration::bind_fixture(&fixed, "exe", "standard", "1.5.3", 2026100505, &path, public.as_ref()).unwrap();
    let mut changed_approval = approval_bytes.clone();
    changed_approval.push(b' ');
    std::fs::write(&approval, &changed_approval).unwrap();
    assert_eq!(migration::approved_manifest(&approval_args).unwrap_err(), "EXE fixed approval digest mismatch");
    assert_eq!(std::fs::read(&path).unwrap(), b"exact-package");
    std::fs::remove_file(approval).unwrap();
    migration::bind_fixture(
        &manifest,
        "exe",
        "standard",
        "1.5.3",
        2026100505,
        &path,
        public.as_ref(),
    )
    .unwrap();
    for (kind, edition, build) in [
        ("msi", "standard", 2026100505),
        ("exe", "sos", 2026100505),
        ("exe", "standard", 2026100506),
    ] {
        assert!(migration::bind_fixture(
            &manifest,
            kind,
            edition,
            "1.5.3",
            build,
            &path,
            public.as_ref()
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"exact-package");
    }
    let mut changed = manifest.clone();
    changed.targets.get_mut(key).unwrap().build_seq += 1;
    assert!(migration::bind_fixture(
        &changed,
        "exe",
        "standard",
        "1.5.3",
        2026100505,
        &path,
        public.as_ref()
    )
    .is_err());
    std::fs::write(&path, b"changed-bytes").unwrap();
    assert!(migration::bind_fixture(
        &manifest,
        "exe",
        "standard",
        "1.5.3",
        2026100505,
        &path,
        public.as_ref()
    )
    .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"changed-bytes");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn helper_started_is_not_installed_only_bound_committed_succeeds() {
    let plan = serde_json::to_vec(
        &serde_json::json!({"transaction_id":"a".repeat(32),"installation_id":"b".repeat(32)}),
    )
    .unwrap();
    let tx = serde_json::json!({"id":"a".repeat(32),"phase":"committed"});
    let receipt = serde_json::json!({"schema":1,"transaction_id":"a".repeat(32),"installation_id":"b".repeat(32),"plan_sha256":hex::encode(Sha256::digest(&plan)),"phase":"committed","error":null,"reboot_required":false});
    migration::terminal(
        &plan,
        &serde_json::to_vec(&tx).unwrap(),
        &serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    for phase in [
        "package_verified",
        "stopping_services",
        "applying",
        "validating",
        "rollback_failed",
    ] {
        let mut t = tx.clone();
        t["phase"] = phase.into();
        let mut r = receipt.clone();
        r["phase"] = phase.into();
        assert!(migration::terminal(
            &plan,
            &serde_json::to_vec(&t).unwrap(),
            &serde_json::to_vec(&r).unwrap()
        )
        .is_err());
    }
    let mut t = tx.clone();
    t["phase"] = "rolled_back".into();
    let mut r = receipt.clone();
    r["phase"] = "rolled_back".into();
    r["error"] = "startup failed".into();
    assert!(migration::terminal(
        &plan,
        &serde_json::to_vec(&t).unwrap(),
        &serde_json::to_vec(&r).unwrap()
    )
    .unwrap_err()
    .contains("[update-rolled-back]"));
    let mut r = receipt.clone();
    r["transaction_id"] = "c".repeat(32).into();
    assert!(migration::terminal(
        &plan,
        &serde_json::to_vec(&tx).unwrap(),
        &serde_json::to_vec(&r).unwrap()
    )
    .is_err());
}

#[test]
fn actual_pending_event_parser_keeps_optional_command_id_after_portable_forwarding() {
    use base::update::{PendingUpdateEvent, UpdateSource};
    for command_id in [None, Some("c".repeat(32))] {
        let event = PendingUpdateEvent {
            transaction_id: "a".repeat(32),
            from_version: "1.5.3".into(),
            from_build_seq: 2026100504,
            version: "1.5.3".into(),
            build_seq: 2026100505,
            source: UpdateSource::Primary,
            command_id,
        };
        let mut args = vec!["--management-agent-exe-update".into()];
        args.extend(event.cli_args());
        if args.last().is_some_and(String::is_empty) {
            args.pop();
        }
        let mut legacy_args = args.clone();
        legacy_args[0] = "--update".into();
        let legacy_forwarded =
            update_args::forwarded(&legacy_args, Path::new(r"C:\RustDeskAgentLab\outer.exe"))
                .iter()
                .map(|v| v.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
        assert_eq!(legacy_forwarded, legacy_args);
        assert_eq!(PendingUpdateEvent::from_cli_args(&legacy_forwarded), Some(event.clone()));
        let forwarded = update_args::forwarded(&args, Path::new(r"C:\RustDeskAgentLab\outer.exe"))
            .iter()
            .map(|v| v.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(PendingUpdateEvent::from_cli_args(&forwarded), Some(event));
        assert_eq!(
            migration::outer_argument(&forwarded).unwrap(),
            PathBuf::from(r"C:\RustDeskAgentLab\outer.exe")
        );
    }
}

#[test]
fn readonly_status_fixture_child() {
    let Ok(mode) = std::env::var("EXE_STATUS_TEST_MODE") else {
        return;
    };
    use std::io::Write;
    match mode.as_str() {
        "hang" => std::thread::sleep(std::time::Duration::from_secs(2)),
        "flood" => {
            let b = [b'x'; 65536];
            for _ in 0..16 {
                if std::io::stdout().write_all(&b).is_err() {
                    break;
                }
            }
        }
        "ok" => print!("{{\"ready\":true}}"),
        _ => panic!("invalid fixture"),
    }
}

#[test]
fn readonly_status_tail_fixture_child() {
    let Some(root) = std::env::var_os("EXE_STATUS_TAIL_ROOT") else {
        return;
    };
    use std::io::Write;
    let root = PathBuf::from(root);
    if std::env::var("EXE_STATUS_TAIL_HOLDER").ok().as_deref() == Some("yes") {
        std::fs::write(root.join("holder-ready"), b"ready").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !root.join("holder-release").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::process::exit(0);
    }
    if std::env::var("EXE_STATUS_TAIL_CASE").ok().as_deref() == Some("inherited") {
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "readonly_status_tail_fixture_child",
                "--nocapture",
            ])
            .env("EXE_STATUS_TAIL_HOLDER", "yes")
            .stdout(std::process::Stdio::inherit())
            .spawn()
            .unwrap();
        let owner = handoff::ObservedProcess::open(child.id())
            .unwrap()
            .owner()
            .clone();
        std::fs::write(
            root.join("holder-owner.json"),
            serde_json::to_vec(&owner).unwrap(),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !root.join("holder-ready").exists() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    std::fs::write(root.join("ready"), b"ready").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !root.join("release").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let bytes = if std::env::var("EXE_STATUS_TAIL_CASE").unwrap() == "limit" {
        vec![b'x'; 2048]
    } else {
        b"{\"ready\":true,\"tail\":\"after-empty-peek\"}".to_vec()
    };
    std::io::stdout().write_all(&bytes).unwrap();
    std::io::stdout().flush().unwrap();
    std::process::exit(
        if std::env::var("EXE_STATUS_TAIL_CASE").unwrap() == "error" {
            7
        } else {
            0
        },
    );
}

#[test]
fn readonly_status_exit_after_empty_peek_drains_tail_with_limits() {
    base::lab_guard::require_lab_os("win10");
    for case in ["json", "limit", "error", "inherited"] {
        let root = Path::new(r"C:\RustDeskAgentLab")
            .join(format!("status-tail-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("sentinel"), b"preserve-existing-data").unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "readonly_status_tail_fixture_child",
                "--nocapture",
            ])
            .env("EXE_STATUS_TAIL_ROOT", &root)
            .env("EXE_STATUS_TAIL_CASE", case);
        let mut released = false;
        let result = migration::bounded_status_after_empty(
            &mut command,
            std::time::Duration::from_secs(10),
            1024,
            &mut |child| {
                if released {
                    return Ok(());
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while !root.join("ready").exists() {
                    if std::time::Instant::now() >= deadline {
                        return Err("fixture readiness timeout".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                std::fs::write(
                    root.join("release"),
                    b"write only after parent observed Peek=0",
                )
                .unwrap();
                released = true;
                while child.try_wait().map_err(|e| e.to_string())?.is_none() {
                    if std::time::Instant::now() >= deadline {
                        return Err("fixture exit timeout".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Ok(())
            },
        );
        assert!(released);
        assert_eq!(
            std::fs::read(root.join("sentinel")).unwrap(),
            b"preserve-existing-data"
        );
        match case {
            "json" | "inherited" => {
                let bytes = result.unwrap();
                assert!(
                    bytes.ends_with(b"{\"ready\":true,\"tail\":\"after-empty-peek\"}"),
                    "tail lost after controlled Peek/exit ordering: {}",
                    String::from_utf8_lossy(&bytes)
                );
            }
            "limit" => assert!(result.unwrap_err().contains("output limit")),
            _ => assert!(result.unwrap_err().contains("status exit")),
        }
        if case == "inherited" {
            let owner: handoff::Owner =
                serde_json::from_slice(&std::fs::read(root.join("holder-owner.json")).unwrap())
                    .unwrap();
            assert!(
                handoff::ObservedProcess::owner_alive(&owner).unwrap(),
                "must return while inheriting process still holds write pipe"
            );
            std::fs::write(root.join("holder-release"), b"release").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while handoff::ObservedProcess::owner_alive(&owner).unwrap() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn readonly_status_timeout_and_output_cap_are_bounded_with_owned_child() {
    for mode in ["hang", "flood"] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "readonly_status_fixture_child", "--nocapture"])
            .env("EXE_STATUS_TEST_MODE", mode);
        let start = std::time::Instant::now();
        let result =
            migration::bounded_status(&mut command, std::time::Duration::from_millis(500), 1024);
        assert!(result.is_err(), "{mode} must fail before returning output");
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}

#[test]
fn committed_before_helper_exit_and_cleanup_must_remain_pending() {
    let root = Path::new(r"C:\RustDeskAgentLab").join(format!(
        "migration-cleanup-{}",
        uuid::Uuid::new_v4().simple()
    ));
    workspace::create(&root).unwrap();
    let plan = serde_json::to_vec(
        &serde_json::json!({"transaction_id":"a".repeat(32),"installation_id":"b".repeat(32)}),
    )
    .unwrap();
    std::fs::write(root.join("prepared.json"), &plan).unwrap();
    std::fs::write(
        root.join("transaction.json"),
        br#"{"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","phase":"committed"}"#,
    )
    .unwrap();
    let receipt=serde_json::to_vec(&serde_json::json!({"schema":1,"transaction_id":"a".repeat(32),"installation_id":"b".repeat(32),"plan_sha256":hex::encode(Sha256::digest(&plan)),"phase":"committed","error":null,"reboot_required":false})).unwrap();
    std::fs::write(root.join("execution-result.json"), &receipt).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "readonly_status_fixture_child", "--nocapture"])
        .env("EXE_STATUS_TEST_MODE", "hang")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let owner = handoff::ObservedProcess::open(child.id())
        .unwrap()
        .owner()
        .clone();
    std::fs::write(
        root.join("handoff.json"),
        serde_json::to_vec(&serde_json::json!({"helper":owner})).unwrap(),
    )
    .unwrap();
    let result = migration::completed(&root);
    child.wait().unwrap();
    assert!(result.unwrap_err().contains("[update-pending]"));
    assert!(migration::completed(&root)
        .unwrap_err()
        .contains("[update-pending]"));
    assert_eq!(
        std::fs::read(root.join("execution-result.json")).unwrap(),
        receipt
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_receipt_hash_and_exited_helper_required_for_terminal() {
    let root = Path::new(r"C:\RustDeskAgentLab")
        .join(format!("migration-final-{}", uuid::Uuid::new_v4().simple()));
    workspace::create(&root).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "readonly_status_fixture_child", "--nocapture"])
        .env("EXE_STATUS_TEST_MODE", "hang")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let owner = handoff::ObservedProcess::open(child.id())
        .unwrap()
        .owner()
        .clone();
    let plan = serde_json::to_vec(
        &serde_json::json!({"transaction_id":"a".repeat(32),"installation_id":"b".repeat(32)}),
    )
    .unwrap();
    let tx = br#"{"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","phase":"committed"}"#;
    let binding = serde_json::to_vec(&serde_json::json!({"helper":owner})).unwrap();
    let stopped = b"stopped-state-bound-by-result";
    let result=serde_json::to_vec(&serde_json::json!({"schema":1,"transaction_id":"a".repeat(32),"installation_id":"b".repeat(32),"plan_sha256":hex::encode(Sha256::digest(&plan)),"phase":"committed","error":null,"reboot_required":false,"stopped_state_sha256":hex::encode(Sha256::digest(stopped))})).unwrap();
    for (name, bytes) in [
        ("prepared.json", plan.as_slice()),
        ("transaction.json", tx.as_slice()),
        ("handoff.json", binding.as_slice()),
        ("stopped-state.json", stopped.as_slice()),
        ("execution-result.json", result.as_slice()),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    let receipt = serde_json::json!({"schema":1,"cleanup_finished":true,"helper":owner,"transaction_id":"a".repeat(32),"installation_id":"b".repeat(32),"phase":"committed","plan_sha256":hex::encode(Sha256::digest(&plan)),"transaction_sha256":hex::encode(Sha256::digest(tx)),"handoff_sha256":hex::encode(Sha256::digest(&binding)),"execution_result_sha256":hex::encode(Sha256::digest(&result))});
    std::fs::write(
        root.join("cleanup-complete.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert!(migration::completed(&root)
        .unwrap_err()
        .contains("[update-pending]"));
    child.wait().unwrap();
    migration::completed(&root).unwrap();
    for field in [
        "cleanup_finished",
        "execution_result_sha256",
        "handoff_sha256",
    ] {
        let mut bad = receipt.clone();
        bad[field] = serde_json::Value::Null;
        std::fs::write(
            root.join("cleanup-complete.json"),
            serde_json::to_vec(&bad).unwrap(),
        )
        .unwrap();
        assert!(migration::completed(&root)
            .unwrap_err()
            .contains("[update-pending]"));
    }
    std::fs::remove_dir_all(root).unwrap();
}
