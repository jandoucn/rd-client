#![cfg(windows)]
#[path = "../src/platform/windows_agent_guard.rs"]
mod guard;

static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn maintenance_blocks_another_thread_and_releases_after_drop() {
    let _serial = TEST_MUTEX.lock().unwrap();
    let acquired = guard::InstallGuard::try_acquire()
        .expect("guard access")
        .expect("free guard");
    let blocked = std::thread::spawn(|| {
        guard::InstallGuard::try_acquire()
            .expect("guard access")
            .is_none()
    })
    .join()
    .expect("thread");
    assert!(blocked);
    drop(acquired);
    let reacquired = std::thread::spawn(|| {
        guard::InstallGuard::try_acquire()
            .expect("guard access")
            .is_some()
    })
    .join()
    .expect("thread");
    assert!(reacquired);
}

#[test]
fn maintenance_blocks_another_process() {
    if std::env::var_os("RUSTDESK_GUARD_TEST_CHILD").is_some() {
        assert!(guard::InstallGuard::try_acquire()
            .expect("child guard access")
            .is_none());
        return;
    }
    let _serial = TEST_MUTEX.lock().unwrap();
    let _acquired = guard::InstallGuard::try_acquire()
        .expect("guard access")
        .expect("free guard");
    let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "maintenance_blocks_another_process",
            "--test-threads=1",
        ])
        .env("RUSTDESK_GUARD_TEST_CHILD", "1")
        .status()
        .expect("child process");
    assert!(status.success());
}
