use super::*;
use std::process::Command;
use windows::core::PCWSTR;

fn fixture() -> PathBuf {
    assert_eq!(
        std::env::var("COMPUTERNAME").unwrap(),
        "RDLAB-WIN11",
        "guest-only"
    );
    let root = std::env::temp_dir().join(format!(
        "rd-exe-desktop-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    root
}
fn bytes(target: &Path) -> Vec<u8> {
    let _com = com().unwrap();
    let link: Shell::IShellLinkW =
        unsafe { Com::CoCreateInstance(&Shell::ShellLink, None, Com::CLSCTX_INPROC_SERVER) }
            .unwrap();
    let wide: Vec<u16> = target
        .to_str()
        .unwrap()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe { link.SetPath(PCWSTR(wide.as_ptr())) }.unwrap();
    let persist: Com::IPersistStream = link.cast().unwrap();
    let stream = unsafe { Shell::SHCreateMemStream(None) }.unwrap();
    unsafe { persist.Save(&stream, true) }.unwrap();
    let mut stat = Com::STATSTG::default();
    unsafe { stream.Stat(&mut stat, Com::STATFLAG_NONAME) }.unwrap();
    let mut out = vec![0; stat.cbSize as usize];
    let mut count = 0;
    unsafe {
        stream.Seek(0, Com::STREAM_SEEK_SET, None).unwrap();
        stream
            .Read(out.as_mut_ptr().cast(), out.len() as u32, Some(&mut count))
            .ok()
            .unwrap();
    }
    assert_eq!(count as usize, out.len());
    out
}
fn execute(plan: &DesktopPlan) -> bool {
    execute_commands(plan.commands().unwrap())
}
fn execute_commands(commands: String) -> bool {
    if commands.is_empty() {
        return true;
    }
    if !commands.contains("-EncodedCommand ") {
        let root = fixture();
        let batch = root.join("write.cmd");
        std::fs::write(
            &batch,
            format!(
                "@echo off\r\nset \"RUSTDESK_OUTPUT_DIR={}\"\r\n{commands}",
                root.display()
            ),
        )
        .unwrap();
        let ok = Command::new(r"C:\Windows\System32\cmd.exe")
            .args(["/d", "/c"])
            .arg(batch)
            .status()
            .unwrap()
            .success();
        std::fs::remove_dir_all(root).unwrap();
        return ok;
    }
    let encoded = commands
        .split("-EncodedCommand ")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", encoded])
        .status()
        .unwrap()
        .success()
}
#[test]
fn generated_install_script_refuses_changed_shortcut_before_mutation() {
    let root = fixture();
    let target = root.join("RustDesk.exe");
    let path = root.join("RustDesk.lnk");
    let plan = prepare_at(&root, Some(&target), true, bytes(&target)).unwrap();
    std::fs::write(&path, b"foreign appeared").unwrap();
    let marker = root.join("uninstall-started");
    let script=plan.guard_uninstall(&format!("    if exist \"%PUBLIC%\\Desktop\\RustDesk.lnk\" del /f /q \"%PUBLIC%\\Desktop\\RustDesk.lnk\"\r\necho mutation>\"{}\"",marker.display())).unwrap();
    let batch = root.join("install.cmd");
    std::fs::write(&batch, script).unwrap();
    let status = Command::new(r"C:\Windows\System32\cmd.exe")
        .args(["/d", "/c"])
        .arg(&batch)
        .status()
        .unwrap();
    let mutated = marker.exists();
    let original = std::fs::read(&path).unwrap();
    drop(plan);
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        !status.success(),
        "changed shortcut must abort install script"
    );
    assert!(!mutated);
    assert_eq!(original, b"foreign appeared");
}
#[test]
fn preflight_detects_changes_before_uninstall() {
    let root = fixture();
    let target = root.join("RustDesk.exe");
    let path = root.join("RustDesk.lnk");
    let absent = prepare_at(&root, Some(&target), true, bytes(&target)).unwrap();
    assert!(execute_commands(absent.preflight_commands().unwrap()));
    std::fs::write(&path, bytes(&target)).unwrap();
    assert!(!execute_commands(absent.preflight_commands().unwrap()));
    drop(absent);
    let present = prepare_at(&root, Some(&target), true, bytes(&target)).unwrap();
    assert!(execute_commands(present.preflight_commands().unwrap()));
    std::fs::write(&path, b"foreign replacement").unwrap();
    assert!(!execute_commands(present.preflight_commands().unwrap()));
    assert_eq!(std::fs::read(&path).unwrap(), b"foreign replacement");
    drop(present);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn missing_upgrade_selected_does_not_create() {
    let root = fixture();
    let old = root.join("RustDesk.exe");
    let plan = prepare_at(&root, Some(&old), true, bytes(&old)).unwrap();
    assert!(execute(&plan));
    let absent = !root.join("RustDesk.lnk").exists();
    drop(plan);
    std::fs::remove_dir_all(root).unwrap();
    assert!(
        absent,
        "legacy desktopicon=true recreated missing upgrade shortcut"
    );
}
#[test]
fn present_upgrade_recreates_new_target_after_uninstall_even_option_off() {
    let root = fixture();
    let old = root.join("old/RustDesk.exe");
    let new = root.join("new/RustDesk.exe");
    let path = root.join("RustDesk.lnk");
    std::fs::write(&path, bytes(&old)).unwrap();
    let plan = prepare_at(&root, Some(&old), false, bytes(&new)).unwrap();
    assert!(execute(&plan));
    let target = std::fs::read(&path).ok().and_then(|b| link_target(&b).ok());
    drop(plan);
    std::fs::remove_dir_all(root).unwrap();
    assert!(target.is_some_and(|target| same_path(&target, &new)));
}
#[test]
fn first_install_respects_option() {
    for selected in [false, true] {
        let root = fixture();
        let target = root.join("RustDesk.exe");
        let plan = prepare_at(&root, None, selected, bytes(&target)).unwrap();
        assert!(execute(&plan));
        assert_eq!(root.join("RustDesk.lnk").exists(), selected);
        drop(plan);
        std::fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn foreign_corrupt_and_locked_are_errors_and_keep_bytes() {
    let root = fixture();
    let target = root.join("RustDesk.exe");
    let path = root.join("RustDesk.lnk");
    for original in [bytes(&root.join("foreign.exe")), b"bad link".to_vec()] {
        std::fs::write(&path, &original).unwrap();
        assert!(prepare_at(&root, Some(&target), true, bytes(&target)).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    let original = bytes(&target);
    std::fs::write(&path, &original).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(prepare_at(&root, Some(&target), true, bytes(&target)).is_err());
    drop(lock);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn concurrent_file_is_not_overwritten() {
    let root = fixture();
    let target = root.join("RustDesk.exe");
    let plan = prepare_at(&root, None, true, bytes(&target)).unwrap();
    let path = root.join("RustDesk.lnk");
    std::fs::write(&path, b"other owner").unwrap();
    assert!(!execute(&plan));
    assert_eq!(std::fs::read(&path).unwrap(), b"other owner");
    drop(plan);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn after_preflight_user_delete_or_foreign_replace_survives_actual_uninstall_phase() {
    for delete in [false, true] {
        let root = fixture();
        let target = root.join("RustDesk.exe");
        let path = root.join("RustDesk.lnk");
        std::fs::write(&path, bytes(&target)).unwrap();
        let plan = prepare_at(&root, Some(&target), true, bytes(&target)).unwrap();
        assert!(execute_commands(plan.preflight_commands().unwrap()));
        std::fs::remove_file(&path).unwrap();
        if !delete {
            std::fs::write(&path, b"foreign-after-preflight").unwrap();
        }
        let legacy = "    if exist \"%PUBLIC%\\Desktop\\RustDesk.lnk\" del /f /q \"%PUBLIC%\\Desktop\\RustDesk.lnk\"";
        let uninstall = plan.guard_uninstall(legacy).unwrap();
        // Execute only the post-preflight portion to place the race precisely
        // after the successful check, not before it.
        let tail = uninstall
            .strip_prefix(&(plan.preflight_commands().unwrap() + "\r\n"))
            .unwrap();
        let public = root.join("public");
        std::fs::create_dir(&public).unwrap();
        // Match the legacy PUBLIC/Desktop layout without touching a real desktop.
        let desktop = public.join("Desktop");
        std::fs::create_dir(&desktop).unwrap();
        if !delete {
            std::fs::write(desktop.join("RustDesk.lnk"), b"foreign-after-preflight").unwrap();
        }
        let script = root.join("race.cmd");
        std::fs::write(
            &script,
            format!(
                "@echo off\r\nset \"PUBLIC={}\"\r\n{tail}\r\n",
                public.display()
            ),
        )
        .unwrap();
        Command::new(r"C:\Windows\System32\cmd.exe")
            .args(["/d", "/c"])
            .arg(script)
            .status()
            .unwrap();
        let foreign_legacy_safe = delete
            || std::fs::read(desktop.join("RustDesk.lnk")).ok().as_deref()
                == Some(b"foreign-after-preflight");
        let _ = execute(&plan);
        let safe = if delete {
            !path.exists()
        } else {
            std::fs::read(&path).unwrap() == b"foreign-after-preflight"
        };
        drop(plan);
        std::fs::remove_dir_all(root).unwrap();
        assert!(foreign_legacy_safe, "legacy uninstall deleted replacement");
        assert!(safe, "user deletion recreated or foreign file overwritten");
    }
}

#[test]
fn long_file_script_updates_unicode_desktop_and_reads_back_new_com_target() {
    let root = fixture();
    let desktop = root.join("桌面-Ж-😀");
    std::fs::create_dir(&desktop).unwrap();
    let old = root.join("旧版-Ж.exe");
    let new = root.join("新版-😀.exe");
    let path = desktop.join("RustDesk.lnk");
    std::fs::write(&path, bytes(&old)).unwrap();
    let plan = prepare_at(&desktop, Some(&old), true, bytes(&new)).unwrap();
    let commands = plan.commands().unwrap();
    assert!(
        commands.contains("-File "),
        "fixture must execute long file branch"
    );
    let success = execute_commands(commands);
    let actual = link_target(&std::fs::read(&path).unwrap()).unwrap();
    drop(plan);
    std::fs::remove_dir_all(root).unwrap();
    assert!(success, "long script execution failed");
    assert!(
        same_path(&actual, &new),
        "long script reported success without updating Unicode shortcut: {actual:?}"
    );
}
