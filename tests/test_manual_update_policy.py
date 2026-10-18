import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class ManualUpdatePolicyTest(unittest.TestCase):
    def test_production_check_and_install_policy(self):
        common = (ROOT / "src/common.rs").read_text()
        updater = (ROOT / "src/updater.rs").read_text()
        base = (ROOT / "libs/base/src/update.rs").read_text()
        check = common[common.index("fn update_check_should_stop("):common.index("pub fn do_check_software_update_with_context(")]
        start = updater.index("    if !response.update_available", updater.index("fn check_update_request("))
        gate = updater[start:updater.index("    let Some(ref manifest)", start)]
        start = updater.index("    let mut action = decide_update_action", start)
        action = updater[start:updater.index("    if let Some(command) = command {", start)]
        decision = base[base.index("pub fn decide_update_action("):base.index("pub fn update_target_key(")]
        source = r'''
#[derive(Debug, PartialEq)]
pub enum UpdateAction { Ignore, Notify, Download, AutoInstall }
#[derive(Debug, PartialEq)]
enum UpdateCommandRunOutcome { NoUpdate, CheckCompleted }
pub struct VersionCheckResponse { update_available: bool, mode: String, auto_install: bool }
mod keys { pub const OPTION_ALLOW_AUTO_UPDATE: &str = "auto"; }
mod config { pub struct Config; impl Config { pub fn get_option(_: &str) -> String { "N".into() } } }
fn update_option_enabled(value: &str) -> bool { value == "Y" }
''' + check + decision + '''
fn admitted(response: &VersionCheckResponse, manually: bool, command: Option<()>) -> Result<UpdateCommandRunOutcome, ()> {
''' + gate + '''
    Ok(UpdateCommandRunOutcome::CheckCompleted)
}
fn install_action(response: &VersionCheckResponse, manually: bool) -> UpdateAction {
''' + action + '''
    action
}
#[test]
fn manual_check_works_with_background_updates_disabled() {
    for origin in ["manual", "command", "command_install"] {
        assert!(!update_check_should_stop(true, "disabled", origin), "{origin}");
    }
}
#[test]
fn background_checks_remain_disabled_and_absent_updates_still_stop() {
    for origin in ["system", "startup", "scheduled"] {
        assert!(update_check_should_stop(true, "disabled", origin), "{origin}");
    }
    for origin in ["manual", "command", "system", "startup"] {
        assert!(update_check_should_stop(false, "disabled", origin), "{origin}");
    }
}
#[test]
fn confirmed_manual_install_is_admitted_for_every_mode() {
    for mode in ["disabled", "notify", "download", "auto_install"] {
        let response = VersionCheckResponse { update_available: true, mode: mode.into(), auto_install: false };
        assert_eq!(admitted(&response, true, None), Ok(UpdateCommandRunOutcome::CheckCompleted), "{mode}");
    }
}
#[test]
fn confirmed_manual_install_overrides_background_action() {
    let response = VersionCheckResponse { update_available: true, mode: "disabled".into(), auto_install: false };
    assert_eq!(install_action(&response, true), UpdateAction::AutoInstall);
}
#[test]
fn disabled_background_install_and_missing_update_stay_inert() {
    for auto_install in [false, true] {
        let response = VersionCheckResponse { update_available: true, mode: "disabled".into(), auto_install };
        assert_eq!(admitted(&response, false, None), Ok(UpdateCommandRunOutcome::NoUpdate));
        assert_eq!(install_action(&response, false), UpdateAction::Ignore);
        assert_eq!(decide_update_action(&response, auto_install), UpdateAction::Ignore);
        assert_eq!(admitted(&response, false, Some(())), Ok(UpdateCommandRunOutcome::CheckCompleted));
    }
    let response = VersionCheckResponse { update_available: false, mode: "notify".into(), auto_install: false };
    assert_eq!(admitted(&response, true, None), Ok(UpdateCommandRunOutcome::NoUpdate));
    assert_eq!(install_action(&response, true), UpdateAction::Ignore);
}
'''
        with tempfile.TemporaryDirectory(prefix="manual-update-policy-") as directory:
            path = Path(directory)
            (path / "policy.rs").write_text(source)
            subprocess.run(["rustc", "--edition=2021", "--test", str(path / "policy.rs"), "-o", str(path / "policy")], check=True)
            subprocess.run([str(path / "policy"), "--nocapture"], check=True)


if __name__ == "__main__":
    unittest.main()
