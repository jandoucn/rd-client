use super::*;
fn sd_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
                DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            },
        },
    };
    #[link(name = "advapi32")]
    extern "system" {
        fn GetSecurityDescriptorLength(sd: *const std::ffi::c_void) -> u32;
    }
    let name = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    assert_eq!(
        unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(name.as_ptr()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                &mut sd,
            )
        }
        .0,
        0
    );
    let size = unsafe { GetSecurityDescriptorLength(sd.0) };
    assert!(size > 0 && size < 65536);
    let bytes = unsafe { std::slice::from_raw_parts(sd.0.cast::<u8>(), size as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(sd.0))) };
    bytes
}
fn set_acl(path: &Path, sddl: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                },
                SetFileSecurityW, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
                PSECURITY_DESCRIPTOR,
            },
        },
    };
    let text = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let name = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(text.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )
    }
    .unwrap();
    assert!(
        unsafe {
            SetFileSecurityW(
                PCWSTR(name.as_ptr()),
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                sd,
            )
        }
        .as_bool(),
        "{}: {} ({sddl})",
        path.display(),
        std::io::Error::last_os_error()
    );
    unsafe { LocalFree(Some(HLOCAL(sd.0))) };
}
#[test]
fn preservation_installed_state_acl_read_boundary() {
    #[link(name = "ntdll")]
    extern "system" {
        fn RtlAdjustPrivilege(privilege: u32, enable: u8, thread: u8, previous: *mut u8) -> i32;
    }
    let mut previous = 0u8;
    assert_eq!(unsafe { RtlAdjustPrivilege(18, 1, 0, &mut previous) }, 0);
    base::lab_guard::require_lab_os("win11");
    let root =
        Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\preservation-state-read-fixture");
    if root.exists() {
        std::fs::remove_dir_all(root).unwrap();
    }
    workspace::create(root).unwrap();
    let state = root.join("state");
    workspace::create(&state).unwrap();
    let intent = root.join("intent.toml");
    std::fs::write(&intent, b"[options]\nstop-service='N'\n").unwrap();
    for name in ["agent.json", "service-recovery-enabled"] {
        std::fs::write(state.join(name), b"protected data").unwrap();
    }
    let installed="O:BAG:SYD:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;BA)(A;OICIIOID;GA;;;CO)(A;OICIID;0x1200a9;;;BU)(A;CIID;0x116;;;BU)";
    set_acl(&state, installed);
    let before = snapshot(&[root.to_path_buf()]);
    let initial_sd = sd_bytes(&state);
    local_repair_plan::total_loss::verified_state_read_fixture(&state, &intent)
        .expect("safe installed state inheritance must be readable");
    assert_eq!(sd_bytes(&state), initial_sd);
    for (path, sddl) in [
        (
            state.clone(),
            "O:BAG:SYD:AI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;CI;0x156;;;BU)",
        ),
        (
            state.clone(),
            "O:BUG:SYD:AI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;CI;0x116;;;BU)",
        ),
        (
            state.join("agent.json"),
            "O:BUG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;BU)",
        ),
        (
            intent.clone(),
            "O:BAG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FW;;;BU)",
        ),
    ] {
        set_acl(&path, sddl);
        let denied_sd = sd_bytes(&path);
        assert!(
            local_repair_plan::total_loss::verified_state_read_fixture(&state, &intent).is_err()
        );
        assert_eq!(sd_bytes(&path), denied_sd);
        if path == state {
            set_acl(&state, installed);
        } else {
            set_acl(&path, "O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)");
        }
    }
    std::fs::write(state.join("maintenance.json"), b"placeholder").unwrap();
    set_acl(
        &state.join("maintenance.json"),
        "O:BUG:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;BU)",
    );
    assert!(local_repair_plan::total_loss::verified_state_read_fixture(&state, &intent).is_err());
    std::fs::remove_file(state.join("maintenance.json")).unwrap();
    std::fs::rename(state.join("agent.json"), state.join("saved.json")).unwrap();
    std::os::windows::fs::symlink_file(state.join("saved.json"), state.join("agent.json")).unwrap();
    assert!(local_repair_plan::total_loss::verified_state_read_fixture(&state, &intent).is_err());
    std::fs::remove_file(state.join("agent.json")).unwrap();
    std::fs::rename(state.join("saved.json"), state.join("agent.json")).unwrap();
    assert_eq!(snapshot(&[root.to_path_buf()]), before);
    std::fs::remove_dir_all(root).unwrap();
}
