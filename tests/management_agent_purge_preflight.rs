#![cfg(windows)]
#[path = "../src/platform/windows_agent_guard.rs"]
mod install_guard;
#[path = "../src/management_agent/purge_preflight.rs"]
mod purge_preflight;

fn fixture() -> std::path::PathBuf {
    base::lab_guard::require_lab_os("win11");
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let root = std::env::temp_dir().join(format!("purge-preflight-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    root
}

#[test]
fn maintenance_file_refuses_and_preserves_original_bytes() {
    let root = fixture();
    let path = root.join("maintenance.json");
    let original = b"existing maintenance belongs to another transaction";
    std::fs::write(&path, original).unwrap();
    let observed = purge_preflight::acquire_fixture(&path);
    assert_eq!(observed.err().as_deref(), Some("purge execution maintenance present"));
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directory_refuses_without_removing_contents() {
    let root = fixture(); let path = root.join("maintenance.json");
    std::fs::create_dir(&path).unwrap(); std::fs::write(path.join("owner"), b"foreign").unwrap();
    assert_eq!(purge_preflight::acquire_fixture(&path).err().as_deref(), Some("purge execution maintenance present"));
    assert_eq!(std::fs::read(path.join("owner")).unwrap(), b"foreign");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn absent_allows_stop_markers_but_recheck_refuses_new_maintenance() {
    let root = fixture(); let path = root.join("maintenance.json");
    std::fs::write(root.join("disable-recovery"), b"keep").unwrap();
    std::fs::write(root.join("RustDesk2.toml"), b"[options]\nstop-service='Y'").unwrap();
    let context = purge_preflight::acquire_fixture(&path).unwrap();
    context.recheck().unwrap();
    std::fs::write(&path, b"new maintenance").unwrap();
    assert_eq!(context.recheck().unwrap_err(), "purge execution maintenance present");
    assert_eq!(std::fs::read(&path).unwrap(), b"new maintenance");
    assert_eq!(std::fs::read(root.join("disable-recovery")).unwrap(), b"keep");
    assert_eq!(std::fs::read(root.join("RustDesk2.toml")).unwrap(), b"[options]\nstop-service='Y'");
    drop(context); std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_query_is_unknown_not_absent() {
    let root = fixture(); std::fs::write(root.join("owner"), b"preserve").unwrap();
    let error = purge_preflight::acquire_fixture(&root.join("invalid\0name")).err().unwrap();
    assert!(error.starts_with("purge execution maintenance unknown:"), "{error}");
    assert_eq!(std::fs::read(root.join("owner")).unwrap(), b"preserve");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn junction_refuses_and_preserves_target() {
    let root = fixture(); let target = root.join("target"); let marker = root.join("maintenance.json");
    std::fs::create_dir(&target).unwrap(); std::fs::write(target.join("owner"), b"keep target").unwrap();
    let status = std::process::Command::new(r"C:\Windows\System32\cmd.exe")
        .args(["/c", "mklink", "/J"]).arg(&marker).arg(&target).status().unwrap();
    assert!(status.success());
    assert_eq!(purge_preflight::acquire_fixture(&marker).err().as_deref(), Some("purge execution maintenance present"));
    assert_eq!(std::fs::read(target.join("owner")).unwrap(), b"keep target");
    std::fs::remove_dir(&marker).unwrap(); std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn competing_child() {
    let Some(path) = std::env::var_os("PURGE_PREFLIGHT_MARKER") else { return; };
    assert_eq!(purge_preflight::acquire_fixture(std::path::Path::new(&path)).err().as_deref(), Some("purge execution busy"));
}

#[test]
fn real_child_busy_until_context_dropped() {
    let root = fixture(); let path = root.join("maintenance.json");
    let context = purge_preflight::acquire_fixture(&path).unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "competing_child", "--nocapture"])
        .env("PURGE_PREFLIGHT_MARKER", &path).output().unwrap();
    assert!(child.status.success(), "{}", String::from_utf8_lossy(&child.stderr));
    assert!(String::from_utf8_lossy(&child.stdout).contains("1 passed"));
    drop(context); drop(purge_preflight::acquire_fixture(&path).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}
