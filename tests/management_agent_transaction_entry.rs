#[path = "../src/management_agent/transaction_entry.rs"]
mod transaction_entry;

#[test]
fn helper_entry_accepts_only_exact_transaction_id_and_bound_executable() {
    use std::path::Path;
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\transactions");
    let id = "a".repeat(32);
    let executable = root.join(&id).join("runtime").join("helper.exe");
    let arguments = vec!["--transaction-id".into(), id.clone()];
    assert_eq!(transaction_entry::resolve(root, &executable, &arguments).unwrap(), root.join(&id));
    for invalid in [vec![], vec!["--transaction-id".into()], vec!["--transaction-id".into(), "../escape".into()],
        vec!["--transaction-id".into(), "A".repeat(32)],
        vec!["--transaction-id".into(), id.clone(), "--force".into()],
        vec!["--root".into(), root.to_string_lossy().into_owned()],
        vec!["--transaction-id".into(), id.clone(), "--transaction-id".into(), id.clone()]] {
        assert_eq!(transaction_entry::resolve(root, &executable, &invalid).unwrap_err(), "invalid transaction helper arguments");
    }
    assert_eq!(transaction_entry::resolve(root, Path::new(r"C:\Temp\helper.exe"), &arguments).unwrap_err(), "transaction helper executable outside bound runtime");
    assert_eq!(transaction_entry::resolve(Path::new("relative"), &executable, &arguments).unwrap_err(), "transaction root must be absolute");
    let traversal = root.join(&id).join("runtime").join("..").join("runtime").join("helper.exe");
    assert!(transaction_entry::resolve(root, &traversal, &arguments).is_err());
}
