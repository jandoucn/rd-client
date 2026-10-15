#[path = "../libs/portable/src/update_args.rs"]
mod update_args;

use std::{ffi::OsString, path::Path};

fn forward(args: &[&str]) -> Vec<OsString> {
    update_args::forwarded(
        &args.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
        Path::new(r"C:\lab\approved package.exe"),
    )
}

#[test]
fn old_update_does_not_require_agent_or_catalog() {
    assert_eq!(forward(&["--update"]), vec![OsString::from("--update")]);
}

#[test]
fn old_event_and_mirror_source_are_forwarded_without_transaction_takeover() {
    let args = ["--update", "--update-event", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "1.5.3", "2026100504", "1.5.3", "2026100505", "mirror", "command-1"];
    assert_eq!(forward(&args), args.iter().map(OsString::from).collect::<Vec<_>>());
}

#[test]
#[cfg(windows)]
fn explicit_transaction_is_separate_from_old_update() {
    assert_eq!(forward(&["--management-agent-exe-update"]), vec![
        OsString::from("--update"),
        OsString::from("--management-agent-outer-package"),
        OsString::from(r"C:\lab\approved package.exe"),
    ]);
}

#[test]
fn transaction_policy_never_infers_support_from_old_arguments() {
    for args in [
        vec!["--update"],
        vec!["--update", "--update-event", "--management-agent-exe-update"],
        vec!["--management-agent-exe-update=1"],
        vec!["--silent-install", "--management-agent-exe-update"],
    ] {
        let strings = args.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert!(!update_args::is_transaction(&strings));
        assert_eq!(forward(&args), args.iter().map(OsString::from).collect::<Vec<_>>());
    }
}

#[test]
#[cfg(windows)]
fn explicit_transaction_retains_gate_arguments_without_legacy_retry() {
    let args = ["--management-agent-exe-update", "--management-agent-outer-package", "invalid"];
    let strings = args.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    assert!(update_args::is_transaction(&strings));
    let forwarded = forward(&args);
    // Keep conflicting input visible to the strict parser instead of repairing it.
    assert_eq!(forwarded.iter().filter(|v| *v == "--management-agent-outer-package").count(), 2);
    assert_eq!(forwarded.last().unwrap(), "invalid");
}

#[test]
fn non_update_commands_keep_their_arguments() {
    for args in [vec!["--silent-install"], vec!["--connect", "123"], vec![]] {
        assert_eq!(forward(&args), args.iter().map(OsString::from).collect::<Vec<_>>());
    }
}
