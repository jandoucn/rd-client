#![cfg(windows)]
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/platform/windows_agent_registration.rs"]
mod platform_registration;
#[path = "../src/management_agent/registration_reconnect.rs"]
mod registration_reconnect;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use registration_reconnect::{Budget, Decision, Observation, Owner, Request, Watchdog, ACTION};
use std::cell::Cell;

fn owner() -> Owner {
    Owner {
        pid: 100,
        created_at: 200,
        build_seq: 2026100502,
    }
}
fn observation() -> Observation {
    Observation {
        owner: owner(),
        ready: true,
        registered: false,
        allowed: true,
        fixed_id_reachable: true,
    }
}
fn request() -> Vec<u8> {
    serde_json::to_vec(&Request {
        schema: 1,
        action: ACTION.into(),
        nonce: "a".repeat(64),
        expected_owner: owner(),
    })
    .unwrap()
}
fn vm() {
    base::lab_guard::require_lab();
}

#[test]
fn automatic_r0_full_queue_does_not_block_main_loop() {
    vm();
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    let (entered, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let mut first = true;
    let worker = registration_reconnect::worker_fixture(move |_| {
        if first {
            first = false;
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        done.send(()).unwrap();
        Ok(())
    })
    .unwrap();
    worker.notify(&"a".repeat(32)).unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let begin = Instant::now();
    for _ in 0..1000 {
        worker.notify(&"a".repeat(32)).unwrap();
    }
    assert!(begin.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    drop(worker);
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(finished.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn automatic_r0_persisted_attempt_survives_worker_restart_and_lost_ack() {
    vm();
    use registration_reconnect::DurableCycle;
    if let Some(root) = std::env::var_os("R0_BUDGET_CHILD") {
        let root = std::path::PathBuf::from(root);
        assert_eq!(
            root.parent(),
            Some(std::path::Path::new(r"C:\RustDeskAgentLab"))
        );
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("r0-auto-"));
        let mut cycle = DurableCycle::default();
        let o = observation();
        let bytes = std::fs::read(root.join("budget.json")).unwrap();
        for t in [0, 60, 120, 180, 240, 300] {
            assert_ne!(
                cycle
                    .step(
                        &root,
                        &"a".repeat(32),
                        &o,
                        t,
                        1400 + t,
                        || panic!("new process cooldown probe"),
                        || panic!("new process cooldown check"),
                        || panic!("new process replay")
                    )
                    .unwrap(),
                Decision::Request
            );
        }
        assert_eq!(std::fs::read(root.join("budget.json")).unwrap(), bytes);
        return;
    }
    let root = std::path::Path::new(r"C:\RustDeskAgentLab")
        .join(format!("r0-auto-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let id = "a".repeat(32);
    let o = observation();
    let mut cycle = DurableCycle::default();
    let calls = Cell::new(0);
    for t in [0, 60, 120, 180, 240] {
        assert_ne!(
            cycle
                .step(
                    &root,
                    &id,
                    &o,
                    t,
                    1000 + t,
                    || Ok(true),
                    || Ok(()),
                    || {
                        calls.set(calls.get() + 1);
                        Ok(())
                    }
                )
                .unwrap(),
            Decision::Request
        );
    }
    let result = cycle.step(
        &root,
        &id,
        &o,
        300,
        1300,
        || Ok(true),
        || Ok(()),
        || {
            calls.set(calls.get() + 1);
            Err("ack lost".into())
        },
    );
    assert_eq!(result.unwrap_err(), "ack lost");
    assert_eq!(calls.get(), 1);
    let record = std::fs::read(root.join("budget.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&record).unwrap();
    assert_eq!(value["attempts"], 1);
    assert_eq!(value["budget"]["next_at"], 4900);
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "automatic_r0_persisted_attempt_survives_worker_restart_and_lost_ack",
            "--nocapture",
        ])
        .env("R0_BUDGET_CHILD", &root)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let mut restarted = DurableCycle::default();
    for t in [0, 60, 120, 180, 240, 300] {
        assert_ne!(
            restarted
                .step(
                    &root,
                    &id,
                    &o,
                    t,
                    1400 + t,
                    || Ok(true),
                    || Ok(()),
                    || panic!("restart replay")
                )
                .unwrap(),
            Decision::Request
        );
    }
    assert_eq!(std::fs::read(root.join("budget.json")).unwrap(), record);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_r0_corrupt_budget_and_final_gate_refuse_preserving_records() {
    vm();
    use registration_reconnect::DurableCycle;
    let root = std::path::Path::new(r"C:\RustDeskAgentLab")
        .join(format!("r0-auto-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    std::fs::write(root.join("budget.json"), b"invalid existing budget").unwrap();
    let calls = Cell::new(0);
    let mut cycle = DurableCycle::default();
    let id = "a".repeat(32);
    assert!(cycle
        .step(
            &root,
            &id,
            &observation(),
            300,
            1300,
            || Ok(true),
            || Ok(()),
            || {
                calls.set(1);
                Ok(())
            }
        )
        .is_err());
    assert_eq!(calls.get(), 0);
    assert_eq!(
        std::fs::read(root.join("budget.json")).unwrap(),
        b"invalid existing budget"
    );
    std::fs::remove_file(root.join("budget.json")).unwrap();
    let mut cycle = DurableCycle::default();
    for t in [0, 60, 120, 180, 240] {
        cycle
            .step(
                &root,
                &id,
                &observation(),
                t,
                1000 + t,
                || Ok(true),
                || Ok(()),
                || panic!("early action"),
            )
            .unwrap();
    }
    assert_eq!(
        cycle
            .step(
                &root,
                &id,
                &observation(),
                300,
                1300,
                || Ok(true),
                || Err("disabled before action".into()),
                || panic!("gate bypass")
            )
            .unwrap_err(),
        "disabled before action"
    );
    assert!(!root.join("budget.json").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_r0_cas_lock_and_unknown_record_preserve_budget_without_action() {
    vm();
    use registration_reconnect::DurableCycle;
    use std::os::windows::fs::OpenOptionsExt;
    let root = std::path::Path::new(r"C:\RustDeskAgentLab")
        .join(format!("r0-auto-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let id = "a".repeat(32);
    let o = observation();
    let mut cycle = DurableCycle::default();
    for t in [0, 60, 120, 180, 240] {
        cycle
            .step(
                &root,
                &id,
                &o,
                t,
                1000 + t,
                || panic!("early probe"),
                || panic!("early recheck"),
                || panic!("early action"),
            )
            .unwrap();
    }
    let foreign = b"third party preserve";
    assert_eq!(
        cycle
            .step(
                &root,
                &id,
                &o,
                300,
                1300,
                || Ok(true),
                || {
                    std::fs::write(root.join("budget.json"), foreign).unwrap();
                    Ok(())
                },
                || panic!("CAS bypass")
            )
            .unwrap_err(),
        "R0 budget CAS conflict"
    );
    assert_eq!(std::fs::read(root.join("budget.json")).unwrap(), foreign);
    std::fs::remove_file(root.join("budget.json")).unwrap();
    let mut cycle = DurableCycle::default();
    for t in [0, 60, 120, 180, 240, 300] {
        cycle
            .step(
                &root,
                &id,
                &o,
                t,
                2000 + t,
                || Ok(true),
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
    }
    let original = std::fs::read(root.join("budget.json")).unwrap();
    for change in ["schema", "installation", "counter", "cooldown"] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match change {
            "schema" => value["schema"] = 2.into(),
            "installation" => value["installation"] = "b".repeat(32).into(),
            "counter" => value["attempts"] = 0.into(),
            _ => value["budget"]["next_at"] = 1.into(),
        };
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(root.join("budget.json"), &bytes).unwrap();
        assert!(DurableCycle::default()
            .step(
                &root,
                &id,
                &o,
                0,
                6000,
                || panic!("invalid record probe"),
                || panic!("invalid record check"),
                || panic!("invalid record action")
            )
            .is_err());
        assert_eq!(std::fs::read(root.join("budget.json")).unwrap(), bytes);
    }
    std::fs::write(root.join("budget.json"), &original).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(root.join("budget.lock"))
        .unwrap();
    assert_eq!(
        DurableCycle::default()
            .step(
                &root,
                &id,
                &o,
                0,
                6000,
                || panic!("busy probe"),
                || panic!("busy check"),
                || panic!("busy action")
            )
            .unwrap_err(),
        "R0 budget writer busy"
    );
    drop(lock);
    let mut restarted = DurableCycle::default();
    for t in [0, 60, 120, 180, 240] {
        restarted
            .step(
                &root,
                &id,
                &o,
                t,
                6000 + t,
                || panic!("early"),
                || Ok(()),
                || Ok(()),
            )
            .unwrap();
    }
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(root.join("budget.json"))
        .unwrap();
    assert!(restarted
        .step(
            &root,
            &id,
            &o,
            300,
            6300,
            || Ok(true),
            || Ok(()),
            || panic!("failed publish action")
        )
        .is_err());
    assert_eq!(std::fs::read(root.join("budget.json")).unwrap(), original);
    drop(locked);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_r0_scaled_bounded_worker_real_pipe_once_then_restart_cooldown() {
    vm();
    use registration_reconnect::DurableCycle;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc,
        },
        time::Duration,
    };
    let root = std::path::Path::new(r"C:\RustDeskAgentLab")
        .join(format!("r0-auto-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let id = "a".repeat(32);
    let (pipe, owner, calls, server) = listener(1);
    let callback_probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let local = callback_probe.local_addr().unwrap();
    let probe_thread = std::thread::spawn(move || {
        let (socket, _) = callback_probe.accept().unwrap();
        drop(socket);
    });
    let (done, completed) = mpsc::sync_channel(1);
    let root_worker = root.clone();
    let steps = Arc::new(AtomicUsize::new(0));
    let worker_steps = steps.clone();
    let mut cycle = DurableCycle::default();
    let worker = registration_reconnect::worker_fixture(move |tick| {
        let n = worker_steps.fetch_add(1, Ordering::SeqCst);
        let mono = (n as u64) * 30;
        let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let o = Observation {
            owner: owner.clone(),
            ready: true,
            registered: false,
            allowed: true,
            fixed_id_reachable: false,
        };
        let result = cycle.step(
            &root_worker,
            &tick.installation,
            &o,
            mono,
            1000 + mono,
            || {
                std::net::TcpStream::connect_timeout(&local, Duration::from_secs(1))
                    .map_err(|e| e.to_string())?;
                Ok(true)
            },
            || Ok(()),
            || {
                let reply = rt.block_on(registration_reconnect::request_fixture(
                    &pipe,
                    &std::env::current_exe().unwrap(),
                    &owner,
                ))?;
                assert!(reply.accepted);
                Err("ACK lost after actual callback".into())
            },
        );
        done.send((mono, result)).unwrap();
        Ok(())
    })
    .unwrap();
    for n in 0..13 {
        let begin = std::time::Instant::now();
        worker.notify(&id).unwrap();
        assert!(begin.elapsed() < Duration::from_secs(1));
        let (mono, result) = completed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(mono, n * 30);
        if n == 10 {
            assert_eq!(result.unwrap_err(), "ACK lost after actual callback");
        } else {
            assert_ne!(result.unwrap(), Decision::Request);
        }
    }
    drop(worker);
    server.join().unwrap();
    probe_thread.join().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let before = std::fs::read(root.join("budget.json")).unwrap();
    let mut restarted = DurableCycle::default();
    let mut o = observation();
    o.owner.created_at += 1;
    for t in [0, 60, 120, 180, 240, 300] {
        assert_ne!(
            restarted
                .step(
                    &root,
                    &id,
                    &o,
                    t,
                    1500 + t,
                    || panic!("restart cooldown probe"),
                    || panic!("restart cooldown check"),
                    || panic!("restart cooldown action")
                )
                .unwrap(),
            Decision::Request
        );
    }
    assert_eq!(std::fs::read(root.join("budget.json")).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn registration_protocol_rejects_unknown_action_nonce_schema_owner_and_size_without_callback() {
    vm();
    let original = request();
    let mut cases = vec![vec![b'x'; 4097], b"{}".to_vec()];
    for (key, value) in [
        ("action", serde_json::json!("shell")),
        ("schema", serde_json::json!(2)),
        ("nonce", serde_json::json!("bad")),
        ("target", serde_json::json!("http://unapproved")),
    ] {
        let mut v: serde_json::Value = serde_json::from_slice(&original).unwrap();
        v[key] = value;
        cases.push(serde_json::to_vec(&v).unwrap());
    }
    for field in ["pid", "created_at", "build_seq"] {
        let mut v: serde_json::Value = serde_json::from_slice(&original).unwrap();
        v["expected_owner"][field] = 999.into();
        cases.push(serde_json::to_vec(&v).unwrap());
    }
    for bytes in cases {
        let called = Cell::new(0);
        let result = registration_reconnect::dispatch(&bytes, &owner(), || {
            called.set(called.get() + 1);
            Ok(())
        });
        assert!(result.is_err(), "invalid request accepted");
        assert_eq!(called.get(), 0);
    }
    let called = Cell::new(0);
    registration_reconnect::dispatch(&original, &owner(), || {
        called.set(1);
        Ok(())
    })
    .unwrap();
    assert_eq!(called.get(), 1);
}

#[test]
fn registration_watchdog_requires_continuous_five_minutes_and_one_persisted_attempt() {
    vm();
    let mut w = Watchdog::default();
    let mut budget = Budget::default();
    let o = observation();
    let saves = Cell::new(0);
    for elapsed in [0, 60, 120, 180, 240, 299] {
        assert_eq!(
            w.observe(&o, elapsed, 1000 + elapsed, &mut budget, |_| panic!(
                "early persistence"
            ))
            .unwrap(),
            Decision::Wait
        );
    }
    assert_eq!(
        w.observe(&o, 300, 1300, &mut budget, |next| {
            saves.set(saves.get() + 1);
            assert_eq!(next.last_attempt, 1300);
            assert!(next.next_at > 1300);
            Ok(())
        })
        .unwrap(),
        Decision::Request
    );
    for elapsed in [330, 360, 400] {
        assert_ne!(
            w.observe(&o, elapsed, 1000 + elapsed, &mut budget, |_| panic!(
                "duplicate persistence"
            ))
            .unwrap(),
            Decision::Request
        );
    }
    assert_eq!(saves.get(), 1);
}

#[test]
fn registration_watchdog_failed_persistence_preserves_budget_and_never_grants_request() {
    vm();
    let mut w = Watchdog::default();
    let mut budget = Budget::default();
    let o = observation();
    for elapsed in [0, 60, 120, 180, 240] {
        let _ = w.observe(&o, elapsed, 1000 + elapsed, &mut budget, |_| {
            panic!("early persistence")
        });
    }
    let before = budget.clone();
    let result = w.observe(&o, 300, 1300, &mut budget, |_| Err("disk denied".into()));
    assert_eq!(result.unwrap_err(), "disk denied");
    assert_eq!(budget, before);
}

#[test]
fn registration_watchdog_breaks_gaps_owner_changes_unknown_and_clock_rollback() {
    vm();
    for reason in [
        "gap",
        "owner",
        "not-ready",
        "blocked",
        "registered",
        "clock",
    ] {
        let mut w = Watchdog::default();
        let mut b = Budget::default();
        let mut o = observation();
        for t in [0, 60, 120, 180, 240] {
            assert_eq!(
                w.observe(&o, t, 1000 + t, &mut b, |_| panic!("early"))
                    .unwrap(),
                Decision::Wait
            );
        }
        match reason {
            "owner" => o.owner.created_at += 1,
            "not-ready" => o.ready = false,
            "blocked" => o.allowed = false,
            "registered" => o.registered = true,
            _ => {}
        }
        let (mono, wall) = match reason {
            "gap" => (400, 1400),
            "clock" => (300, 999),
            _ => (300, 1300),
        };
        assert_ne!(
            w.observe(&o, mono, wall, &mut b, |_| panic!(
                "invalid observation spent budget"
            ))
            .unwrap(),
            Decision::Request
        );
        assert_eq!(b, Budget::default());
    }
    let mut w = Watchdog::default();
    let mut b = Budget {
        last_attempt: 1100,
        next_at: 4700,
    };
    let mut o = observation();
    o.fixed_id_reachable = false;
    for t in [0, 60, 120, 180, 240, 300] {
        assert_ne!(
            w.observe(&o, t, 1200 + t, &mut b, |_| panic!("offline"))
                .unwrap(),
            Decision::Request
        );
    }
    o.fixed_id_reachable = true;
    assert_ne!(
        w.observe(&o, 330, 1530, &mut b, |_| panic!("cooldown"))
            .unwrap(),
        Decision::Request
    );
}

#[test]
fn registration_real_files_gates_and_install_lock_refuse_without_callback() {
    vm();
    let root = std::path::Path::new(r"C:\RustDeskAgentLab")
        .join(format!("r0-v224-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    let config = root.join("RustDesk2.toml");
    std::fs::write(
        root.join("agent.json"),
        br#"{"disabled":false,"management_agent_enabled":true}"#,
    )
    .unwrap();
    std::fs::write(root.join("service-recovery-enabled"), b"yes").unwrap();
    let check = || platform_registration::check_gates(&root, &config, true, false, true, "N");
    for text in [
        "[options]\nstop-service='Y'",
        "[options]\nstop-service='unknown'",
        "[options]",
        "broken",
    ] {
        std::fs::write(&config, text).unwrap();
        assert!(check().is_err());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), text);
    }
    std::fs::write(&config, "[options]\nstop-service='N'").unwrap();
    check().unwrap();
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        std::fs::write(root.join(marker), b"keep").unwrap();
        assert!(check().unwrap_err().contains("blocked"));
        assert_eq!(std::fs::read(root.join(marker)).unwrap(), b"keep");
        std::fs::remove_file(root.join(marker)).unwrap();
    }
    for (role, deploy, idle, stop) in [
        (false, false, true, "N"),
        (true, true, true, "N"),
        (true, false, false, "N"),
        (true, false, true, "?"),
        (true, false, true, "Y"),
    ] {
        assert!(
            platform_registration::check_gates(&root, &config, role, deploy, idle, stop).is_err()
        );
    }
    std::fs::remove_file(root.join("service-recovery-enabled")).unwrap();
    assert!(check().is_err());
    let (tx, rx) = std::sync::mpsc::channel();
    let (done, wait) = std::sync::mpsc::channel();
    let child = std::thread::spawn(move || {
        platform_registration::with_install_gate(|| {
            tx.send(()).unwrap();
            wait.recv().unwrap();
            Ok(())
        })
        .unwrap()
    });
    rx.recv().unwrap();
    assert!(
        platform_registration::with_install_gate(|| panic!("lock bypass"))
            .unwrap_err()
            .contains("busy")
    );
    done.send(()).unwrap();
    child.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

fn listener(
    connections: usize,
) -> (
    String,
    Owner,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::thread::JoinHandle<()>,
) {
    let pipe = format!(
        r"\\.\pipe\RustDeskRegistrationTest-{}",
        uuid::Uuid::new_v4().simple()
    );
    let target = pipe.clone();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = registration_reconnect::bind_fixture(&target).unwrap();
            let p = handoff::ObservedProcess::open(std::process::id()).unwrap();
            let owner = Owner {
                pid: p.owner().pid,
                created_at: p.owner().created_at,
                build_seq: 2026100502,
            };
            drop(p);
            tx.send(owner).unwrap();
            registration_reconnect::serve_fixture(
                listener,
                2026100502,
                move || {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                },
                connections,
            )
            .await
            .unwrap();
        });
    });
    (pipe, rx.recv().unwrap(), calls, thread)
}

#[test]
fn registration_real_pipe_checks_owner_path_build_before_send_and_limits_repeat() {
    vm();
    let (pipe, owner, calls, server) = listener(6);
    let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let exe = std::env::current_exe().unwrap();
        for field in ["pid", "created", "build", "path"] {
            let mut wrong = owner.clone();
            match field {
                "pid" => wrong.pid += 1,
                "created" => wrong.created_at += 1,
                "build" => wrong.build_seq += 1,
                _ => {}
            }
            let path = if field == "path" {
                std::path::PathBuf::from(r"C:\Windows\System32\cmd.exe")
            } else {
                exe.clone()
            };
            assert!(
                registration_reconnect::request_fixture(&pipe, &path, &wrong)
                    .await
                    .is_err()
            );
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
        let reply = registration_reconnect::request_fixture(&pipe, &exe, &owner)
            .await
            .unwrap();
        assert!(reply.accepted);
        assert!(!reply.registration_verified);
        let reply = registration_reconnect::request_fixture(&pipe, &exe, &owner)
            .await
            .unwrap();
        assert!(!reply.accepted);
        assert_eq!(reply.error.as_deref(), Some("registration cooldown"));
    });
    server.join().unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn registration_real_pipe_invalid_packets_never_restart() {
    vm();
    let (pipe, _, calls, server) = listener(5);
    let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async{
        use hbb_common::tokio::io::{AsyncReadExt,AsyncWriteExt};
        for case in ["action","nonce","owner","extra","oversize"]{
            let mut stream=parity_tokio_ipc::Endpoint::connect(&pipe).await.unwrap();let count=stream.read_u32_le().await.unwrap();let mut bytes=vec![0;count as usize];stream.read_exact(&mut bytes).await.unwrap();let hello:serde_json::Value=serde_json::from_slice(&bytes).unwrap();
            let mut value=serde_json::json!({"schema":1,"action":ACTION,"nonce":hello["challenge"],"expected_owner":hello["owner"]});
            match case{"action"=>value["action"]="exec".into(),"nonce"=>value["nonce"]="b".repeat(64).into(),"owner"=>value["expected_owner"]["created_at"]=1.into(),"extra"=>value["target"]="other".into(),_=>{}}
            if case=="oversize"{stream.write_u32_le(4097).await.unwrap();}else{let bytes=serde_json::to_vec(&value).unwrap();stream.write_u32_le(bytes.len() as u32).await.unwrap();stream.write_all(&bytes).await.unwrap();}
            let mut byte=[0];let _=hbb_common::tokio::time::timeout(std::time::Duration::from_secs(3),stream.read(&mut byte)).await;
        }
    });
    server.join().unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn registration_pipe_acl_denies_restricted_users_token() {
    vm();
    let runtime = hbb_common::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _entered = runtime.enter();
    let pipe = format!(
        r"\\.\pipe\RustDeskRegistrationTest-{}",
        uuid::Uuid::new_v4().simple()
    );
    let _listener = registration_reconnect::bind_fixture(&pipe).unwrap();
    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut std::ffi::c_void,
        attributes: u32,
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenProcessToken(
            process: *mut std::ffi::c_void,
            access: u32,
            token: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn CreateWellKnownSid(
            kind: i32,
            domain: *const std::ffi::c_void,
            sid: *mut std::ffi::c_void,
            size: *mut u32,
        ) -> i32;
        fn CreateRestrictedToken(
            token: *mut std::ffi::c_void,
            flags: u32,
            disable_count: u32,
            disable: *const SidAndAttributes,
            privilege_count: u32,
            privileges: *const std::ffi::c_void,
            restrict_count: u32,
            restrict: *const SidAndAttributes,
            result: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn ImpersonateLoggedOnUser(token: *mut std::ffi::c_void) -> i32;
        fn RevertToSelf() -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    let mut token = std::ptr::null_mut();
    let mut restricted = std::ptr::null_mut();
    let mut sid = [0u64; 16];
    let mut size = 128;
    unsafe {
        assert_ne!(
            OpenProcessToken(GetCurrentProcess(), 0xf01ff, &mut token),
            0
        );
        assert_ne!(
            CreateWellKnownSid(27, std::ptr::null(), sid.as_mut_ptr().cast(), &mut size),
            0
        );
        let entry = SidAndAttributes {
            sid: sid.as_mut_ptr().cast(),
            attributes: 0,
        };
        assert_ne!(
            CreateRestrictedToken(
                token,
                1,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
                1,
                &entry,
                &mut restricted
            ),
            0
        );
        assert_ne!(ImpersonateLoggedOnUser(restricted), 0);
    }
    let result = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&pipe);
    unsafe {
        assert_ne!(RevertToSelf(), 0);
        CloseHandle(restricted);
        CloseHandle(token);
    }
    assert_eq!(
        result.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
}

#[test]
fn registration_real_pipe_gate_refusal_does_not_invoke_restart() {
    vm();
    let root = std::path::Path::new(r"C:\RustDeskAgentLab")
        .join(format!("r0-v224-{}", uuid::Uuid::new_v4()));
    workspace::create(&root).unwrap();
    std::fs::write(
        root.join("agent.json"),
        br#"{"disabled":false,"management_agent_enabled":true}"#,
    )
    .unwrap();
    std::fs::write(root.join("service-recovery-enabled"), b"yes").unwrap();
    std::fs::write(root.join("RustDesk2.toml"), "[options]\nstop-service='N'").unwrap();
    std::fs::write(root.join("maintenance.json"), b"preserve").unwrap();
    let pipe = format!(
        r"\\.\pipe\RustDeskRegistrationTest-{}",
        uuid::Uuid::new_v4().simple()
    );
    let server_pipe = pipe.clone();
    let gate_root = root.clone();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = count.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = registration_reconnect::bind_fixture(&server_pipe).unwrap();
            let p = handoff::ObservedProcess::open(std::process::id()).unwrap();
            let owner = Owner {
                pid: p.owner().pid,
                created_at: p.owner().created_at,
                build_seq: 2026100502,
            };
            drop(p);
            tx.send(owner).unwrap();
            registration_reconnect::serve_fixture(
                listener,
                2026100502,
                move || {
                    platform_registration::with_install_gate(|| {
                        platform_registration::check_gates(
                            &gate_root,
                            &gate_root.join("RustDesk2.toml"),
                            true,
                            false,
                            true,
                            "N",
                        )?;
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    })
                },
                1,
            )
            .await
            .unwrap();
        });
    });
    let owner = rx.recv().unwrap();
    let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let reply = rt
        .block_on(registration_reconnect::request_fixture(
            &pipe,
            &std::env::current_exe().unwrap(),
            &owner,
        ))
        .unwrap();
    assert!(!reply.accepted);
    assert!(!reply.registration_verified);
    assert_eq!(reply.error.as_deref(), Some("registration locally blocked"));
    server.join().unwrap();
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(root.join("maintenance.json")).unwrap(),
        b"preserve"
    );
    std::fs::remove_dir_all(root).unwrap();
}
