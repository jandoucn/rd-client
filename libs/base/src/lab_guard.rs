//! Single gate for the destructive experimental / fault-injection acceptance
//! suites that were previously pinned to two fixed VirtualBox guests by
//! machine name and SMBIOS UUID.
//!
//! The suites are destructive (they stop and remove the RustDesk services and
//! wipe installation directories) and must not run on a developer desktop. The
//! gate is now opt-in instead of name-based: the operator declares the machine
//! as a lab host with `RUSTDESK_LAB_HOST=<COMPUTERNAME>`. Any host qualifies,
//! including the physical Win10 LTSC box where SYSTEM is available.

use std::env;

/// Environment variable naming the machine the operator declared as the lab.
pub const LAB_HOST_ENV: &str = "RUSTDESK_LAB_HOST";

/// Optional `win10` / `win11` selector, replacing the old name-based split.
pub const LAB_OS_ENV: &str = "RUSTDESK_LAB_OS";

/// Pure decision: does `declared` (the `RUSTDESK_LAB_HOST` value) authorise
/// running on `computer` (the current `COMPUTERNAME`)?
///
/// Empty or absent `declared` always refuses, so an unconfigured shell can
/// never run a destructive suite. Comparison is case-insensitive because Windows
/// computer names are.
pub fn is_declared_lab(declared: Option<&str>, computer: Option<&str>) -> bool {
    let declared = match declared.map(str::trim) {
        Some(value) if !value.is_empty() => value,
        _ => return false,
    };
    match computer.map(str::trim) {
        Some(computer) if !computer.is_empty() => declared.eq_ignore_ascii_case(computer),
        _ => false,
    }
}

/// Refuses unless this machine was declared as the lab host.
pub fn require_lab() {
    if !is_declared_lab(
        read_env(LAB_HOST_ENV).as_deref(),
        read_env("COMPUTERNAME").as_deref(),
    ) {
        panic!(
            "{} must name this machine: set {} to the output of `echo %COMPUTERNAME%` \
             on the lab host before running this destructive acceptance test",
            LAB_HOST_ENV, LAB_HOST_ENV
        );
    }
}

/// False when the machine was not declared, for guards that must not panic.
pub fn is_lab_host() -> bool {
    is_declared_lab(
        read_env(LAB_HOST_ENV).as_deref(),
        read_env("COMPUTERNAME").as_deref(),
    )
}

/// Optional OS split for suites that were pinned to one of the two guests.
/// `None` means the suite did not care about the build; read the selector when
/// it did.
pub fn declared_os() -> Option<String> {
    read_env(LAB_OS_ENV)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

/// Pure decision for the optional OS split: an unset selector admits every
/// suite; a set selector admits only the suite it names.
pub fn os_allows(declared_os: Option<&str>, suite: &str) -> bool {
    match declared_os.map(str::trim) {
        Some(value) if !value.is_empty() => value.eq_ignore_ascii_case(suite),
        _ => true,
    }
}

/// Like [`require_lab`], for suites that were pinned to one guest OS.
pub fn require_lab_os(suite: &str) {
    require_lab();
    if !os_allows(declared_os().as_deref(), suite) {
        panic!("{} excludes this {} acceptance test", LAB_OS_ENV, suite);
    }
}

/// Non-panicking form of [`require_lab_os`] for fixtures that report `Err`.
pub fn is_lab_os(suite: &str) -> bool {
    is_lab_host() && os_allows(declared_os().as_deref(), suite)
}

fn read_env(name: &str) -> Option<String> {
    env::var_os(name).map(|value| value.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_declaration_refuses() {
        assert!(!is_declared_lab(None, Some("YAN")));
        assert!(!is_lab_declaration("", "YAN"));
        assert!(!is_lab_declaration("   ", "YAN"));
    }

    #[test]
    fn different_machine_refuses() {
        assert!(!is_lab_declaration("LAB-GUEST", "YAN"));
        assert!(!is_lab_declaration("YAN", "OLIVIER-PC"));
        assert!(!is_lab_declaration("YANX", "YAN"));
    }

    #[test]
    fn same_machine_is_allowed() {
        assert!(is_lab_declaration("YAN", "YAN"));
    }

    #[test]
    fn case_differences_still_match() {
        assert!(is_lab_declaration("yan", "YAN"));
        assert!(is_lab_declaration("Yan", "yAn"));
    }

    #[test]
    fn missing_computer_refuses() {
        assert!(!is_declared_lab(Some("YAN"), None));
        assert!(!is_declared_lab(Some("YAN"), Some("")));
    }

    #[test]
    fn os_selector_unset_allows_any_suite() {
        assert!(os_allows(None, "win10"));
        assert!(os_allows(Some(""), "win11"));
    }

    #[test]
    fn os_selector_matching_suite_is_allowed() {
        assert!(os_allows(Some("win10"), "win10"));
        assert!(os_allows(Some(" WIN11 "), "win11"));
    }

    #[test]
    fn os_selector_other_suite_refuses() {
        assert!(!os_allows(Some("win10"), "win11"));
        assert!(!os_allows(Some("win11"), "win10"));
    }

    fn is_lab_declaration(declared: &str, computer: &str) -> bool {
        is_declared_lab(Some(declared), Some(computer))
    }
}
