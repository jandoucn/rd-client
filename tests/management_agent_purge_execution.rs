#![cfg(windows)]
#[path = "../src/management_agent/purge_policy.rs"] mod purge_policy;
#[path = "../src/management_agent/purge_execution.rs"] mod execution;
use purge_policy::{Probe, Outcome, InstallerResult};
use std::{collections::VecDeque, path::PathBuf, os::windows::fs::OpenOptionsExt};

struct Fixture {
    root: PathBuf,
    held: Option<std::fs::File>,
    probes: VecDeque<Probe>,
    result: InstallerResult,
    calls: Vec<&'static str>,
}
impl Fixture {
    fn new(result: InstallerResult, after: Probe) -> Self {
        assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN11");
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("purge-execution-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        for name in ["identity", "source", "state"] { std::fs::write(root.join(name), name.as_bytes()).unwrap(); }
        let held = Some(std::fs::OpenOptions::new().read(true).share_mode(1).open(root.join("identity")).unwrap());
        Self { root, held, probes: VecDeque::from([Probe::RegisteredVerified, after]), result, calls: vec![] }
    }
    fn assert_preserved(&self) {
        for name in ["identity", "source", "state"] { assert_eq!(std::fs::read(self.root.join(name)).unwrap(), name.as_bytes()); }
        assert_eq!(std::fs::read(self.root.join("disable-recovery")).unwrap(), b"disabled");
        assert!(!self.calls.contains(&"cleanup"));
    }
}
impl execution::Backend for Fixture {
    fn recheck(&mut self) -> Result<(), String> { self.calls.push("recheck"); Ok(()) }
    fn probe(&mut self) -> Result<Probe, String> { self.calls.push("probe"); Ok(self.probes.pop_front().unwrap()) }
    fn disable_recovery(&mut self) -> Result<(), String> {
        self.calls.push("marker"); std::fs::write(self.root.join("disable-recovery"), b"disabled").map_err(|e| e.to_string())
    }
    fn uninstall(&mut self) -> Result<InstallerResult, String> {
        assert!(self.root.join("disable-recovery").exists()); self.calls.push("installer"); Ok(self.result)
    }
    fn release_approval(&mut self) { self.calls.push("release"); drop(self.held.take()); }
    fn cleanup(&mut self) -> Result<Outcome, String> {
        self.calls.push("cleanup");
        std::fs::remove_file(self.root.join("identity")).map_err(|e| e.to_string())?;
        Ok(Outcome::Complete)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { drop(self.held.take()); std::fs::remove_dir_all(&self.root).unwrap(); }
}

#[test]
fn complete_waits_for_absence_and_releases_real_file_before_cleanup() {
    let mut f = Fixture::new(InstallerResult::Exited(0), Probe::AbsentVerified);
    assert_eq!(execution::execute(&mut f), Ok(Outcome::Complete));
    assert_eq!(f.calls, ["recheck","probe","recheck","marker","recheck","installer","recheck","probe","recheck","release","cleanup"]);
    assert!(!f.root.join("identity").exists());
}
#[test]
fn all_non_success_results_keep_marker_identity_source_state() {
    for (result, expected) in [(InstallerResult::Exited(1618),Outcome::Busy), (InstallerResult::Exited(1612),Outcome::SourceUnavailable),
        (InstallerResult::Exited(1603),Outcome::Failed(1603)),(InstallerResult::Pending,Outcome::Pending),
        (InstallerResult::Exited(3010),Outcome::RebootRequired),(InstallerResult::Exited(1641),Outcome::RebootRequired)] {
        let mut f = Fixture::new(result, Probe::AbsentVerified);
        assert_eq!(execution::execute(&mut f), Ok(expected)); f.assert_preserved();
        assert_eq!(f.calls.iter().filter(|&&v| v=="installer").count(),1);
    }
}
#[test]
fn success_or_1605_is_not_absence() {
    for code in [0,1605] { for (probe,outcome) in [(Probe::RegisteredVerified,Outcome::StillRegistered),(Probe::Unknown,Outcome::QueryUnknown)] {
        let mut f=Fixture::new(InstallerResult::Exited(code),probe);
        assert_eq!(execution::execute(&mut f),Ok(outcome));f.assert_preserved();
    }}
}
