#[path = "../src/platform/defender_policy.rs"]
mod policy;

#[test]
fn exact_consent_tokens_default_to_inherit_and_reject_conflicts() {
    assert_eq!(policy::choice("desktopicon startmenu").unwrap(), "inherit");
    assert_eq!(policy::choice("desktopicon defender=1").unwrap(), "1");
    assert_eq!(policy::choice("defender=0").unwrap(), "0");
    assert!(policy::choice("defender=1 defender=0").is_err());
    assert!(policy::choice("defender=invalid").is_err());
    assert_eq!(policy::choice("not-defender=1").unwrap(), "inherit");
}

#[test]
fn silent_argument_parser_only_forwards_explicit_valid_values() {
    assert_eq!(
        policy::silent_option(["--silent-install", "printer=1"]).unwrap(),
        ""
    );
    assert_eq!(
        policy::silent_option(["--silent-install", "defender=1"]).unwrap(),
        " defender=1"
    );
    assert_eq!(
        policy::silent_option(["--silent-install", "defender=0"]).unwrap(),
        " defender=0"
    );
    assert!(policy::silent_option(["--silent-install", "defender=2"]).is_err());
    assert!(policy::silent_option(["--silent-install", "defender=0", "defender=1"]).is_err());
}
