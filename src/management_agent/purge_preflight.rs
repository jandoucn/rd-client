// Execution admission is separate from read-only identity recognition. The
// caller keeps this thread-bound guard through its eventual transaction.
pub struct Context {
    _guard: super::install_guard::InstallGuard,
    maintenance: std::path::PathBuf,
}

pub fn acquire() -> Result<Context, String> {
    #[cfg(not(test))]
    if !unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() } {
        return Err("purge execution requires administrator".into());
    }
    acquire_at(&super::system_dirs::get().agent().join(r"state\maintenance.json"))
}

fn acquire_at(maintenance: &std::path::Path) -> Result<Context, String> {
    let guard = super::install_guard::InstallGuard::try_acquire()
        .map_err(|error| format!("purge execution mutex unknown: {error}"))?
        .ok_or_else(|| "purge execution busy".to_owned())?;
    let context = Context { _guard: guard, maintenance: maintenance.into() };
    context.recheck()?;
    Ok(context)
}

impl Context {
    // Call immediately before each eventual side effect while retaining this guard.
    // This observes external maintenance writers; it does not claim an atomic
    // transaction with writers that do not participate in the install mutex.
    pub fn recheck(&self) -> Result<(), String> {
        match std::fs::symlink_metadata(&self.maintenance) {
            Ok(_) => Err("purge execution maintenance present".into()),
            Err(error) if matches!(error.raw_os_error(), Some(2 | 3)) => Ok(()),
            Err(error) => Err(format!("purge execution maintenance unknown: {error}")),
        }
    }
}

#[cfg(test)]
pub fn acquire_fixture(maintenance: &std::path::Path) -> Result<Context, String> {
    acquire_at(maintenance)
}
