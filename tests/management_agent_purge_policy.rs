#[path = "../src/management_agent/purge_policy.rs"]
mod policy;

use policy::{coordinate, Backend, InstallerResult, Outcome, Probe, Product};
use std::collections::VecDeque;

const PRODUCT: Product = Product { code: "{71A6889B-6003-42DB-989F-B7AE68260DDB}" };

struct Fixture {
    probes: VecDeque<Probe>,
    installer: InstallerResult,
    calls: Vec<&'static str>,
    data: [Vec<u8>; 3],
    cleanup_fails: bool,
}

impl Fixture {
    fn new(probes: Vec<Probe>, installer: InstallerResult) -> Self {
        Self { probes: probes.into(), installer, calls: vec![], cleanup_fails: false,
            data: [b"original identity".to_vec(), b"original source".to_vec(), b"original state".to_vec()] }
    }
}

impl Backend for Fixture {
    fn probe(&mut self, product: &Product) -> Probe {
        assert_eq!(product, &PRODUCT);
        self.calls.push("probe");
        self.probes.pop_front().expect("unexpected registry probe")
    }
    fn uninstall(&mut self, product: &Product) -> InstallerResult {
        assert_eq!(product, &PRODUCT);
        self.calls.push("installer");
        self.installer
    }
    fn cleanup(&mut self, product: &Product) -> Result<(), ()> {
        assert_eq!(product, &PRODUCT);
        self.calls.push("cleanup");
        if self.cleanup_fails { return Err(()); }
        self.data = Default::default();
        Ok(())
    }
}

#[test]
fn verified_registered_uninstalls_once_then_absent_allows_cleanup() {
    let mut f = Fixture::new(vec![Probe::RegisteredVerified, Probe::AbsentVerified], InstallerResult::Exited(0));
    assert_eq!(coordinate(&PRODUCT, &mut f), Outcome::Complete);
    assert_eq!(f.calls, ["probe", "installer", "probe", "cleanup"]);
    assert!(f.data.iter().all(Vec::is_empty));
}

#[test]
fn initial_refusal_has_no_installer_or_cleanup_and_preserves_all_bytes() {
    for (probe, outcome) in [(Probe::IdentityMismatch, Outcome::IdentityMismatch),
        (Probe::Ambiguous, Outcome::Ambiguous), (Probe::Unknown, Outcome::QueryUnknown)] {
        let mut f = Fixture::new(vec![probe], InstallerResult::Exited(0));
        let before = f.data.clone();
        assert_eq!(coordinate(&PRODUCT, &mut f), outcome);
        assert_eq!(f.calls, ["probe"]);
        assert_eq!(f.data, before);
    }
}

#[test]
fn installer_nonterminal_and_failures_preserve_all_bytes_without_cleanup() {
    for (result, outcome) in [(InstallerResult::Exited(1618), Outcome::Busy),
        (InstallerResult::Exited(1612), Outcome::SourceUnavailable),
        (InstallerResult::Exited(1603), Outcome::Failed(1603)),
        (InstallerResult::Exited(87), Outcome::Failed(87)),
        (InstallerResult::Pending, Outcome::Pending),
        (InstallerResult::Exited(3010), Outcome::RebootRequired),
        (InstallerResult::Exited(1641), Outcome::RebootRequired)] {
        let mut f = Fixture::new(vec![Probe::RegisteredVerified], result);
        let before = f.data.clone();
        assert_eq!(coordinate(&PRODUCT, &mut f), outcome);
        assert_eq!(f.calls, ["probe", "installer"]);
        assert_eq!(f.data, before);
    }
}

#[test]
fn success_and_unknown_product_require_independent_absence_confirmation() {
    for code in [0, 1605] {
        for (probe, outcome) in [(Probe::RegisteredVerified, Outcome::StillRegistered),
            (Probe::Unknown, Outcome::QueryUnknown), (Probe::IdentityMismatch, Outcome::IdentityMismatch),
            (Probe::Ambiguous, Outcome::Ambiguous)] {
            let mut f = Fixture::new(vec![Probe::RegisteredVerified, probe], InstallerResult::Exited(code));
            let before = f.data.clone();
            assert_eq!(coordinate(&PRODUCT, &mut f), outcome);
            assert_eq!(f.calls, ["probe", "installer", "probe"]);
            assert_eq!(f.data, before);
        }
    }
}

#[test]
fn unknown_product_with_confirmed_absence_is_idempotent() {
    let mut f = Fixture::new(vec![Probe::RegisteredVerified, Probe::AbsentVerified], InstallerResult::Exited(1605));
    assert_eq!(coordinate(&PRODUCT, &mut f), Outcome::Complete);
    assert_eq!(f.calls, ["probe", "installer", "probe", "cleanup"]);
}

#[test]
fn already_verified_absent_does_not_launch_installer() {
    let mut f = Fixture::new(vec![Probe::AbsentVerified], InstallerResult::Pending);
    assert_eq!(coordinate(&PRODUCT, &mut f), Outcome::Complete);
    assert_eq!(f.calls, ["probe", "cleanup"]);
}

#[test]
fn cleanup_failure_is_not_complete() {
    let mut f = Fixture::new(vec![Probe::AbsentVerified], InstallerResult::Pending);
    f.cleanup_fails = true;
    let before = f.data.clone();
    assert_eq!(coordinate(&PRODUCT, &mut f), Outcome::CleanupFailed);
    assert_eq!(f.calls, ["probe", "cleanup"]);
    assert_eq!(f.data, before);
}
