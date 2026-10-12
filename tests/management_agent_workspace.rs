#[cfg(windows)]
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

#[cfg(windows)]
mod tests {
    use super::workspace;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "rustdesk-workspace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn created_workspace_is_protected_and_does_not_reapprove_existing_directory() {
        let root = root();
        workspace::create(&root).unwrap();
        workspace::verify(&root).unwrap();
        assert!(workspace::create(&root).unwrap_err().contains("exists"));
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn ordinary_inherited_acl_and_missing_or_file_targets_are_refused() {
        let root = root();
        assert!(workspace::verify(&root).is_err());
        std::fs::create_dir(&root).unwrap();
        assert!(workspace::verify(&root).is_err());
        std::fs::remove_dir(&root).unwrap();
        std::fs::write(&root, b"original").unwrap();
        assert!(workspace::create(&root).is_err());
        assert!(workspace::verify(&root).is_err());
        assert_eq!(std::fs::read(&root).unwrap(), b"original");
        std::fs::remove_file(&root).unwrap();
    }

    #[test]
    fn child_files_inherit_only_system_and_admin_access_and_acl_drift_is_refused() {
        let root = root();
        workspace::create(&root).unwrap();
        let file = root.join("prepared.json");
        std::fs::write(&file, b"original preparation").unwrap();
        workspace::verify_file(&file).unwrap();
        let tool = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap()
            .join("System32")
            .join("icacls.exe");
        let changed = std::process::Command::new(tool)
            .arg(&root)
            .args(["/grant", "*S-1-1-0:(OI)(CI)F"])
            .output()
            .unwrap();
        assert!(changed.status.success());
        assert!(workspace::verify(&root).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"original preparation");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
