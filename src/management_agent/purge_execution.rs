use super::purge_policy::{InstallerResult, Outcome, Probe};

// All operations are specific to the already-bound product. There is no path,
// command or ProductCode input at this orchestration boundary.
pub trait Backend {
    fn recheck(&mut self) -> Result<(), String>;
    fn probe(&mut self) -> Result<Probe, String>;
    fn disable_recovery(&mut self) -> Result<(), String>;
    fn uninstall(&mut self) -> Result<InstallerResult, String>;
    fn release_approval(&mut self);
    fn cleanup(&mut self) -> Result<Outcome, String>;
}

pub fn execute(backend: &mut impl Backend) -> Result<Outcome, String> {
    backend.recheck()?;
    let observation = backend.probe()?;
    match observation {
        Probe::IdentityMismatch => return Ok(Outcome::IdentityMismatch),
        Probe::Ambiguous => return Ok(Outcome::Ambiguous),
        Probe::Unknown => return Ok(Outcome::QueryUnknown),
        Probe::RegisteredVerified | Probe::AbsentVerified => {}
    }
    backend.recheck()?;
    backend.disable_recovery()?;
    if observation == Probe::RegisteredVerified {
        backend.recheck()?;
        match backend.uninstall()? {
            InstallerResult::Exited(0 | 1605) => {},
            InstallerResult::Exited(1618) => return Ok(Outcome::Busy),
            InstallerResult::Exited(1612) => return Ok(Outcome::SourceUnavailable),
            InstallerResult::Exited(3010 | 1641) => return Ok(Outcome::RebootRequired),
            InstallerResult::Exited(code) => return Ok(Outcome::Failed(code)),
            InstallerResult::Pending => return Ok(Outcome::Pending),
        }
    }
    backend.recheck()?;
    match backend.probe()? {
        Probe::RegisteredVerified => return Ok(Outcome::StillRegistered),
        Probe::IdentityMismatch => return Ok(Outcome::IdentityMismatch),
        Probe::Ambiguous => return Ok(Outcome::Ambiguous),
        Probe::Unknown => return Ok(Outcome::QueryUnknown),
        Probe::AbsentVerified => {}
    }
    backend.recheck()?;
    backend.release_approval();
    backend.cleanup()
}
