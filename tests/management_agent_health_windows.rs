#![cfg(windows)]
#[path = "../src/management_agent/handoff.rs"]
mod handoff;
#[path = "../src/management_agent/health.rs"]
mod health;
#[path = "../src/management_agent/health_windows.rs"]
mod health_windows;

fn pipe() -> String {
    format!(
        r"\\.\pipe\RustDeskAgentTestHealth-{}-{}",
        std::process::id(),
        hex::encode(hbb_common::sodiumoxide::randombytes::randombytes(8))
    )
}

#[hbb_common::tokio::test]
#[ignore = "Requires v2-capable installed client in isolated SYSTEM lab VM"]
async fn installed_activity_pipe_confirms_idle_and_preserves_legacy_response() {
    base::lab_guard::require_lab();
    let exe = std::path::Path::new(r"C:\Program Files\RustDesk\RustDesk.exe");
    let legacy = health_windows::probe(r"\\.\pipe\RustDesk\management-health-v1", exe, 2026100502, std::time::Duration::from_secs(3)).await.unwrap();
    assert!(legacy.local_responsive);
    assert_eq!(legacy.no_active_sessions, None);
    let activity = health_windows::probe(r"\\.\pipe\RustDesk\management-health-v2", exe, 2026100502, std::time::Duration::from_secs(3)).await.unwrap();
    assert!(activity.local_responsive);
    assert_eq!(activity.no_active_sessions, Some(true));
    assert!(!activity.registered, "production endpoints remain isolated");
    assert_eq!(activity.pid, legacy.pid);
    assert_eq!(activity.process_created_at, legacy.process_created_at);
    println!("{}", serde_json::to_string(&activity).unwrap());
}

#[hbb_common::tokio::test]
async fn activity_pipe_reports_idle_and_busy_while_legacy_pipe_stays_compatible() {
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
    let path = pipe();
    let idle = Arc::new(AtomicBool::new(false));
    let observed = idle.clone();
    let listener = health_windows::bind(&path).unwrap();
    let server = hbb_common::tokio::spawn(health_windows::serve_activity(
        listener, "1.5.3".into(), 2026100502, || false, move || observed.load(Ordering::SeqCst),
    ));
    let exe = std::env::current_exe().unwrap();
    let response = health_windows::probe(&path, &exe, 2026100502, std::time::Duration::from_secs(2)).await.unwrap();
    assert_eq!(response.no_active_sessions, Some(false));
    idle.store(true, Ordering::SeqCst);
    let response = health_windows::probe(&path, &exe, 2026100502, std::time::Duration::from_secs(2)).await.unwrap();
    assert_eq!(response.no_active_sessions, Some(true));
    assert!(!response.registered);
    server.abort(); let _ = server.await;
    let legacy_path = pipe();
    let listener = health_windows::bind(&legacy_path).unwrap();
    let server = hbb_common::tokio::spawn(health_windows::serve(listener, "1.5.3".into(), 2026100502, || false));
    let response = health_windows::probe(&legacy_path, &exe, 2026100502, std::time::Duration::from_secs(2)).await.unwrap();
    assert_eq!(response.no_active_sessions, None);
    assert!(!serde_json::to_value(response).unwrap().as_object().unwrap().contains_key("no_active_sessions"));
    server.abort(); let _ = server.await;
}

#[hbb_common::tokio::test]
async fn actual_health_pipe_binds_server_process_build_and_separates_network_status() {
    if let Some(executable) = std::env::var_os("RUSTDESK_ACTUAL_HEALTH_EXECUTABLE") {
        base::lab_guard::require_lab();
        let response = health_windows::probe(
            r"\\.\pipe\RustDesk\management-health-v1",
            std::path::Path::new(&executable),
            2026100502,
            std::time::Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert!(response.local_responsive);
        assert!(
            !response.registered,
            "isolated production endpoints must not report registration"
        );
        println!("{}", serde_json::to_string(&response).unwrap());
        return;
    }
    let path = pipe();
    let listener = health_windows::bind(&path).unwrap();
    let exe = std::env::current_exe().unwrap();
    let server = hbb_common::tokio::spawn(health_windows::serve(
        listener,
        "1.5.3".into(),
        2026100502,
        || false,
    ));
    let response =
        health_windows::probe(&path, &exe, 2026100502, std::time::Duration::from_secs(2))
            .await
            .unwrap();
    assert!(response.local_responsive);
    assert!(!response.registered);
    assert_eq!(response.pid, std::process::id());
    assert!(
        health_windows::probe(&path, &exe, 2026100503, std::time::Duration::from_secs(2))
            .await
            .unwrap_err()
            .contains("mismatch")
    );
    assert!(health_windows::probe(
        &path,
        &exe.with_file_name("other.exe"),
        2026100502,
        std::time::Duration::from_secs(2)
    )
    .await
    .unwrap_err()
    .contains("executable"));
    server.abort();
    let _ = server.await;
}

#[hbb_common::tokio::test]
async fn absent_and_stalled_health_endpoints_fail_with_bounded_timeout() {
    let path = pipe();
    let exe = std::env::current_exe().unwrap();
    assert!(health_windows::probe(
        &path,
        &exe,
        2026100502,
        std::time::Duration::from_millis(100)
    )
    .await
    .is_err());
    let mut listener = health_windows::bind(&path).unwrap();
    let server = hbb_common::tokio::spawn(async move {
        let _connected = listener.next().await.unwrap().unwrap();
        hbb_common::tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    });
    let timer = std::time::Instant::now();
    assert!(health_windows::probe(
        &path,
        &exe,
        2026100502,
        std::time::Duration::from_millis(100)
    )
    .await
    .unwrap_err()
    .contains("timeout"));
    assert!(timer.elapsed() < std::time::Duration::from_secs(1));
    server.abort();
    let _ = server.await;
}

#[hbb_common::tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocking_helper_probe_uses_native_deadline_without_nested_runtime() {
    let path = pipe();
    let listener = health_windows::bind(&path).unwrap();
    let server = hbb_common::tokio::spawn(health_windows::serve(
        listener,
        "1.5.3".into(),
        2026100502,
        || false,
    ));
    let exe = std::env::current_exe().unwrap();
    let probe_path = path.clone();
    let probe_exe = exe.clone();
    let result = hbb_common::tokio::task::spawn_blocking(move || {
        health_windows::probe_blocking(
            &probe_path,
            &probe_exe,
            2026100502,
            std::time::Duration::from_secs(2),
        )
    })
    .await
    .unwrap()
    .unwrap();
    assert!(result.local_responsive);
    assert!(!result.registered);
    server.abort();
    let _ = server.await;
    let mut stalled = health_windows::bind(&path).unwrap();
    let held = hbb_common::tokio::spawn(async move {
        let _stream = stalled.next().await.unwrap().unwrap();
        hbb_common::tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    });
    let timer = std::time::Instant::now();
    let error = hbb_common::tokio::task::spawn_blocking(move || {
        health_windows::probe_blocking(
            &path,
            &exe,
            2026100502,
            std::time::Duration::from_millis(100),
        )
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.contains("timeout"), "{error}");
    assert!(timer.elapsed() < std::time::Duration::from_secs(2));
    held.abort();
    let _ = held.await;
}

#[hbb_common::tokio::test]
async fn oversized_and_mutating_requests_do_not_disrupt_next_valid_probe() {
    use hbb_common::tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = pipe();
    let listener = health_windows::bind(&path).unwrap();
    let server = hbb_common::tokio::spawn(health_windows::serve(
        listener,
        "1.5.3".into(),
        2026100502,
        || false,
    ));
    for bytes in [
        vec![b'x'; 4097],
        br#"{"schema":1,"nonce":"invalid","command":"stop"}"#.to_vec(),
    ] {
        let mut connected = parity_tokio_ipc::Endpoint::connect(&path).await.unwrap();
        connected.write_u32_le(bytes.len() as u32).await.unwrap();
        if bytes.len() <= 4096 {
            connected.write_all(&bytes).await.unwrap();
        }
        let result = hbb_common::tokio::time::timeout(
            std::time::Duration::from_secs(1),
            connected.read_u32_le(),
        )
        .await
        .unwrap();
        assert!(result.is_err());
    }
    health_windows::probe(
        &path,
        &std::env::current_exe().unwrap(),
        2026100502,
        std::time::Duration::from_secs(2),
    )
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
}
