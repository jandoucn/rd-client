#![cfg(windows)]
#[path = "../src/platform/windows_system_dirs.rs"]
#[allow(dead_code)]
mod system_dirs;
// 代理源码里有 `crate::management_agent::system_dirs` 的写法，测试入口补同名路径。
#[allow(unused_imports)]
mod management_agent {
    pub(crate) use super::system_dirs;
}
#[path="../src/management_agent/purge_marker.rs"] mod purge_marker;
use std::{path::{Path,PathBuf},process::Command};

fn ps(script:&str) {
    let output=Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        .args(["-NoProfile","-NonInteractive","-Command",script]).output().unwrap();
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
}
fn setup()->PathBuf {
    base::lab_guard::require_lab_os("win11");
    let root=Path::new(r"C:\ProgramData").join(format!("RustDeskPurgeMarkerFixture-{}",uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    ps(&format!("$ErrorActionPreference='Stop';$a=New-Object Security.AccessControl.DirectorySecurity;$a.SetSecurityDescriptorSddlForm('O:BAG:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)(A;CI;0x116;;;BU)');Set-Acl -LiteralPath '{}' -AclObject $a",root.display()));
    std::fs::create_dir(root.join("state")).unwrap();
    std::fs::write(root.join("state/agent.json"),b"original state").unwrap();root
}
fn sddl(path:&Path)->String {
    let output=Command::new("powershell.exe").args(["-NoProfile","-Command",&format!("(Get-Acl -LiteralPath '{}').Sddl",path.display())]).output().unwrap();
    assert!(output.status.success());String::from_utf8_lossy(&output.stdout).trim().into()
}

#[test]
fn inherited_state_creates_protected_marker_and_pins_namespace() {
    let root=setup();let state=root.join("state");let before=sddl(&state);
    let held=purge_marker::fixture(&state,||Ok(())).unwrap();
    assert_eq!(std::fs::read(state.join("disable-recovery")).unwrap(),b"disabled\n");
    assert_eq!(sddl(&state),before);
    assert!(sddl(&state.join("disable-recovery")).contains("D:P"));
    assert!(std::fs::rename(&state,root.join("swapped")).is_err());
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(),b"original state");
    drop(held);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn untrusted_occupied_marker_is_never_overwritten() {
    let root=setup();let state=root.join("state");let path=state.join("disable-recovery");
    std::fs::write(&path,b"foreign occupied bytes").unwrap();
    ps(&format!("$a=Get-Acl '{}';$a.SetOwner([Security.Principal.SecurityIdentifier]'S-1-5-32-545');Set-Acl '{}' $a",path.display(),path.display()));
    let before=sddl(&path);
    assert!(purge_marker::fixture(&state,||Ok(())).err().unwrap().contains("owner/DACL untrusted"));
    assert_eq!(std::fs::read(&path).unwrap(),b"foreign occupied bytes");assert_eq!(sddl(&path),before);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn delete_child_and_junction_are_refused_without_writes() {
    let root=setup();let state=root.join("state");
    ps(&format!("$ErrorActionPreference='Stop';$a=Get-Acl '{}';$a.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule([Security.Principal.SecurityIdentifier]'S-1-5-32-545',[Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles,[Security.AccessControl.AccessControlType]::Allow)));Set-Acl -LiteralPath '{}' -AclObject $a",state.display(),state.display()));
    let before=sddl(&state);
    println!("DELETE_CHILD_SDDL={before};root={}",root.display());
    assert!(purge_marker::fixture(&state,||Ok(())).err().unwrap().contains("untrusted rights"));
    assert!(!state.join("disable-recovery").exists());assert_eq!(sddl(&state),before);
    let link=root.join("link");
    assert!(Command::new("cmd.exe").args(["/c","mklink","/J"]).arg(&link).arg(&state).status().unwrap().success());
    assert!(purge_marker::fixture(&link,||Ok(())).err().unwrap().contains("reparse"));
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(),b"original state");
    std::fs::remove_dir(link).unwrap();std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn create_new_race_preserves_winner_and_never_reaches_installer() {
    let root=setup();let state=root.join("state");let path=state.join("disable-recovery");
    let calls=std::cell::Cell::new(0);let before=std::cell::RefCell::new(String::new());
    let result=purge_marker::fixture(&state,|| {
        calls.set(calls.get()+1);
        if calls.get()==2 {
            std::fs::write(&path,b"race winner").unwrap();
            *before.borrow_mut()=sddl(&path);
        }
        Ok(())
    });
    assert!(result.err().unwrap().contains("create_new refused"));
    assert_eq!(std::fs::read(&path).unwrap(),b"race winner");
    assert_eq!(sddl(&path),*before.borrow());
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(),b"original state");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn context_rejection_and_ancestor_swap_preserve_namespace() {
    let root=setup();let state=root.join("state");let before=sddl(&state);
    let result=purge_marker::fixture(&state,|| {
        assert!(std::fs::rename(&root,root.with_extension("swapped")).is_err());
        Err("fixture maintenance present".into())
    });
    assert_eq!(result.err().unwrap(),"fixture maintenance present");
    assert!(!state.join("disable-recovery").exists());assert_eq!(sddl(&state),before);
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(),b"original state");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn trusted_existing_marker_keeps_bytes_and_acl() {
    let root=setup();let state=root.join("state");
    drop(purge_marker::fixture(&state,||Ok(())).unwrap());
    let path=state.join("disable-recovery");std::fs::write(&path,b"existing trusted bytes").unwrap();
    let before=sddl(&path);
    let held=purge_marker::fixture(&state,||Ok(())).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(),b"existing trusted bytes");assert_eq!(sddl(&path),before);
    drop(held);std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ordinary_user_preoccupation_is_preserved() {
    let root=setup();let state=root.join("state");
    let rendezvous=std::env::var("PURGE_MARKER_RENDEZVOUS").expect("guest driver rendezvous required");
    let signal=Path::new(&rendezvous);
    std::fs::write(signal.join("state-path.txt"),state.to_string_lossy().as_bytes()).unwrap();
    let deadline=std::time::Instant::now()+std::time::Duration::from_secs(40);
    while !signal.join("user-done.txt").exists() {
        assert!(std::time::Instant::now()<deadline,"ordinary user did not finish");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let path=state.join("disable-recovery");let before=sddl(&path);
    assert!(before.contains("S-1-5-21-"),"must be owned by real local user: {before}");
    assert!(purge_marker::fixture(&state,||Ok(())).err().unwrap().contains("owner/DACL untrusted"));
    assert_eq!(std::fs::read(&path).unwrap(),b"rdlab occupied bytes");assert_eq!(sddl(&path),before);
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(),b"original state");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn ordinary_user_concurrent_ancestor_junction_is_blocked() {
    let root=setup();let state=root.join("state");
    let signal=PathBuf::from(std::env::var("PURGE_MARKER_RENDEZVOUS").unwrap());
    let target=signal.join("replacement");
    assert_eq!(std::fs::read(target.join("sentinel")).unwrap(),b"untouched target");
    let before=sddl(&root);let state_before=sddl(&state);
    let target_before=sddl(&target);
    let once=std::cell::Cell::new(false);
    let held=purge_marker::fixture(&state,|| {
        if !once.replace(true) {
            std::fs::write(signal.join("ancestor-path.txt"),root.to_string_lossy().as_bytes()).unwrap();
            let deadline=std::time::Instant::now()+std::time::Duration::from_secs(90);
            while !signal.join("race-result.json").exists() {
                assert!(std::time::Instant::now()<deadline,"rdlab race timeout");
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let r:serde_json::Value=serde_json::from_slice(&std::fs::read(signal.join("race-result.json")).unwrap()).unwrap();
            assert_eq!(r["elevated"],false);assert_eq!(r["user"],"rdlab");
            assert_eq!(r["renameBlocked"],true);assert_eq!(r["junctionBlocked"],true);
            assert!(r["renameWin32"].as_i64().is_some_and(|v|v==5||v==32));
            assert!(!root.with_extension("swapped").exists());
        }
        Ok(())
    }).unwrap();
    // A privileged rename removes the ACL denial explanation: the held handle
    // must independently reject replacement with ERROR_SHARING_VIOLATION.
    assert_eq!(std::fs::rename(&root,root.with_extension("swapped")).unwrap_err().raw_os_error(),Some(32));
    assert_eq!(sddl(&root),before);assert_eq!(sddl(&state),state_before);
    assert_eq!(sddl(&target),target_before);
    assert_eq!(std::fs::read(state.join("disable-recovery")).unwrap(),b"disabled\n");
    assert_eq!(std::fs::read(state.join("agent.json")).unwrap(),b"original state");
    assert!(!target.join("disable-recovery").exists());
    assert!(!target.join("state/disable-recovery").exists());
    assert_eq!(std::fs::read(target.join("sentinel")).unwrap(),b"untouched target");
    drop(held);
    let moved=root.with_extension("swapped");std::fs::rename(&root,&moved).unwrap();
    std::fs::rename(&moved,&root).unwrap();std::fs::remove_dir_all(root).unwrap();
}
