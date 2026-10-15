// Pure orchestration only: not wired to purge or a Windows backend. Product and
// AbsentVerified require an independently authorized identity/probe adapter.
#[derive(Debug, PartialEq, Eq)]
pub struct Product<'a> {
    pub code: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    RegisteredVerified,
    AbsentVerified,
    IdentityMismatch,
    Ambiguous,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallerResult {
    Exited(u32),
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    Busy,
    SourceUnavailable,
    Failed(u32),
    Pending,
    RebootRequired,
    IdentityMismatch,
    Ambiguous,
    QueryUnknown,
    StillRegistered,
    CleanupFailed,
}

pub trait Backend {
    fn probe(&mut self, product: &Product) -> Probe;
    fn uninstall(&mut self, product: &Product) -> InstallerResult;
    fn cleanup(&mut self, product: &Product) -> Result<(), ()>;
}

pub fn coordinate(product: &Product, backend: &mut impl Backend) -> Outcome {
    match backend.probe(product) {
        Probe::IdentityMismatch => return Outcome::IdentityMismatch,
        Probe::Ambiguous => return Outcome::Ambiguous,
        Probe::Unknown => return Outcome::QueryUnknown,
        Probe::AbsentVerified => return cleanup(product, backend),
        Probe::RegisteredVerified => {}
    }
    match backend.uninstall(product) {
        InstallerResult::Pending => return Outcome::Pending,
        InstallerResult::Exited(1618) => return Outcome::Busy,
        InstallerResult::Exited(1612) => return Outcome::SourceUnavailable,
        InstallerResult::Exited(3010 | 1641) => return Outcome::RebootRequired,
        InstallerResult::Exited(0 | 1605) => {}
        InstallerResult::Exited(code) => return Outcome::Failed(code),
    }
    match backend.probe(product) {
        Probe::AbsentVerified => cleanup(product, backend),
        Probe::RegisteredVerified => Outcome::StillRegistered,
        Probe::IdentityMismatch => Outcome::IdentityMismatch,
        Probe::Ambiguous => Outcome::Ambiguous,
        Probe::Unknown => Outcome::QueryUnknown,
    }
}

fn cleanup(product: &Product, backend: &mut impl Backend) -> Outcome {
    match backend.cleanup(product) {
        Ok(()) => Outcome::Complete,
        Err(()) => Outcome::CleanupFailed,
    }
}
