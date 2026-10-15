use std::{ffi::OsString, path::Path};

pub fn is_transaction(args: &[String]) -> bool {
    cfg!(windows)
        && args.first().map(String::as_str) == Some("--management-agent-exe-update")
}

pub fn forwarded(args: &[String], outer: &Path) -> Vec<OsString> {
    let mut result: Vec<OsString> = args.iter().map(OsString::from).collect();
    if is_transaction(args) {
        result[0] = "--update".into();
        result.insert(1, "--management-agent-outer-package".into());
        result.insert(2, outer.as_os_str().to_owned());
    }
    result
}
