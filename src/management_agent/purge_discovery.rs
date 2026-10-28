use super::{msi_metadata::{self, PackageIdentity}, purge_policy::Probe};

pub struct Approved {
    identity: PackageIdentity,
    #[cfg(not(test))]
    installation_id: String,
    locks: Vec<std::fs::File>,
    #[cfg(not(test))]
    package_code: String,
    #[cfg(not(test))]
    target: hbb_common::UpdateTarget,
    #[cfg(not(test))]
    _package: super::package_gate::LockedPackage,
}

impl Approved {
    pub(super) fn product_code(&self) -> &str { &self.identity.product_code }
    #[cfg(not(test))]
    pub(super) fn installation_id(&self) -> &str { &self.installation_id }
}

#[cfg(test)]
impl Approved {
    pub fn fixture(identity: PackageIdentity) -> Self { Self { identity, locks: vec![] } }
    pub fn fixture_locked(identity: PackageIdentity, locks: Vec<std::fs::File>) -> Self { Self { identity, locks } }
}

// This entry accepts neither a package path nor a GUID. The existing verifier
// binds the fixed rescue bundle to installed state and approved signed assets.
#[cfg(not(test))]
pub fn approved_installed() -> Result<Approved, String> {
    let evidence = super::purge_approval::installed()?;
    let identity = msi_metadata::inspect(&evidence.package)?;
    if identity.product_name != "RustDesk" { return Err("purge approved product mismatch".into()); }
    let package_code = package_code(&evidence.package)?;
    Ok(Approved { identity, installation_id: evidence.installation_id, package_code, target: evidence.target,
        _package: evidence.package_lock, locks: evidence.locks })
}

pub struct Candidate {
    pub registered: PackageIdentity,
    pub cache: PackageIdentity,
    pub source: PackageIdentity,
    pub package_code: String,
    pub cache_package_code: String,
    pub source_package_code: String,
    pub source_path: std::path::PathBuf,
    pub cache_path: std::path::PathBuf,
    pub _locks: Vec<std::fs::File>,
}

pub fn classify(approved: &Approved, candidates: Result<Vec<Candidate>, String>) -> Probe {
    let candidates = match candidates { Ok(v) => v, Err(_) => return Probe::Unknown };
    if candidates.is_empty() { return Probe::AbsentVerified; }
    if candidates.len() != 1 { return Probe::Ambiguous; }
    let candidate = &candidates[0];
    if [&candidate.registered, &candidate.cache, &candidate.source].iter().all(|identity|
        msi_metadata::verify_pair(&approved.identity, identity,
            &approved.identity.product_code, &approved.identity.product_code).is_ok()
        && identity.product_version == approved.identity.product_version)
        && candidate.package_code.eq_ignore_ascii_case(&candidate.cache_package_code)
        && candidate.package_code.eq_ignore_ascii_case(&candidate.source_package_code) {
        Probe::RegisteredVerified
    } else { Probe::IdentityMismatch }
}

// An observation is not an authorization token. All candidates are retained,
// including foreign same-name products; missing/unreadable data is an error.
pub fn native_candidates() -> Result<Vec<Candidate>, String> {
    native_candidates_for(None)
}

fn native_candidates_for(expected: Option<&str>) -> Result<Vec<Candidate>, String> {
    use std::{path::Path, ptr};
    let mut candidates = Vec::new();
    for index in 0..4096 {
        let mut code = [0u16; 39];
        let mut context = 0;
        let status = unsafe { MsiEnumProductsExW(ptr::null(), ptr::null(), 4, index,
            code.as_mut_ptr(), &mut context, ptr::null_mut(), ptr::null_mut()) };
        if status == 259 { return Ok(candidates); }
        if status != 0 || context != 4 { return Err(format!("MSI enumeration failed: {status}")); }
        let query = |name: &str| -> Result<String, String> {
            let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let mut buffer = vec![0u16; 32768];
            let mut size = buffer.len() as u32;
            let result = unsafe { MsiGetProductInfoExW(code.as_ptr(), ptr::null(), 4,
                name.as_ptr(), buffer.as_mut_ptr(), &mut size) };
            if result != 0 || size as usize >= buffer.len() { return Err(format!("MSI property query failed: {result}")); }
            String::from_utf16(&buffer[..size as usize]).map_err(|_| "MSI property encoding".into())
        };
        let product_code = String::from_utf16(&code[..38]).map_err(|_| "MSI code encoding")?;
        let name = query("InstalledProductName")?;
        if name != "RustDesk" {
            if expected.is_some_and(|v| v.eq_ignore_ascii_case(&product_code)) {
                return Err("approved product renamed or foreign".into());
            }
            continue;
        }
        let code = product_code;
        let location = query("InstallLocation")?.trim_end_matches(['\\', '/']).to_owned();
        if !location.eq_ignore_ascii_case(&super::system_dirs::get().normal_install().to_string_lossy()) {
            return Err("RustDesk installation outside fixed location".into());
        }
        let cache_path = query("LocalPackage")?;
        let cache_lock = hold(Path::new(&cache_path))?;
        let cache_package_code = package_code(Path::new(&cache_path))?;
        let cache = msi_metadata::inspect(Path::new(&cache_path))
            .map_err(|error| format!("registered cache {cache_path}: {error}"))?;
        let package_property = "PackageName".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let wide_code = code.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut name = [0u16; 256];
        let mut count = name.len() as u32;
        let status = unsafe { MsiSourceListGetInfoW(wide_code.as_ptr(), ptr::null(), 4, 0,
            package_property.as_ptr(), name.as_mut_ptr(), &mut count) };
        if status != 0 || count as usize >= name.len() { return Err(format!("package name query: {status}")); }
        let name = String::from_utf16(&name[..count as usize]).map_err(|_| "package name encoding")?;
        if name.is_empty() || !name.ends_with(".msi") || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b)) {
            return Err("registered package name invalid".into());
        }
        let directory = std::path::PathBuf::from(query("InstallSource")?);
        if !directory.is_absolute() || directory.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err("registered source directory invalid".into());
        }
        let source_path = directory.join(name);
        let source_lock = hold(&source_path)?;
        let source_package_code = package_code(&source_path)?;
        let source = msi_metadata::inspect(&source_path)?;
        let registered = PackageIdentity { product_code: code, product_name: "RustDesk".into(),
            product_version: query("VersionString")?, upgrade_code: cache.upgrade_code.clone() };
        candidates.push(Candidate { registered, cache, source, package_code: query("PackageCode")?,
            cache_package_code, source_package_code, source_path, cache_path: cache_path.into(), _locks: vec![cache_lock, source_lock] });
    }
    Err("MSI enumeration limit exceeded".into())
}

pub(super) fn hold(path: &std::path::Path) -> Result<std::fs::File, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    if !path.is_absolute() { return Err("purge file must be absolute".into()); }
    for parent in path.ancestors() {
        if std::fs::symlink_metadata(parent).map_err(|e| e.to_string())?.file_attributes() & 0x400 != 0 {
            return Err("purge reparse path refused".into());
        }
    }
    std::fs::OpenOptions::new().read(true).share_mode(1).open(path).map_err(|e| e.to_string())
}

fn package_code(path: &std::path::Path) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    let path = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let mut handle = 0;
    let status = unsafe { MsiGetSummaryInformationW(0, path.as_ptr(), 0, &mut handle) };
    if status != 0 { return Err(format!("MSI summary open: {status}")); }
    let mut kind = 0; let mut number = 0; let mut time = 0u64;
    let mut value = [0u16; 128]; let mut count = value.len() as u32;
    let status = unsafe { MsiSummaryInfoGetPropertyW(handle, 9, &mut kind, &mut number,
        &mut time, value.as_mut_ptr(), &mut count) };
    let closed = unsafe { MsiCloseHandle(handle) };
    if status != 0 || closed != 0 || kind != 30 || count != 38 { return Err("MSI PackageCode invalid".into()); }
    String::from_utf16(&value[..count as usize]).map_err(|_| "MSI PackageCode encoding".into())
}

#[cfg(not(test))]
pub fn verified_native(approved: &Approved) -> Result<Probe, String> {
    let registered_before = exact_registration(&approved.identity.product_code)?;
    let first = native_candidates_for(Some(&approved.identity.product_code))?;
    if !registered_before { verify_arp_absent(&approved.identity.product_code)?; }
    for candidate in &first {
        if !candidate.package_code.eq_ignore_ascii_case(&approved.package_code) { return Ok(Probe::IdentityMismatch); }
        let _source = super::package_gate::lock_verified_file(&candidate.source_path, &approved.target)?;
        let _cache = super::package_gate::lock_verified_file(&candidate.cache_path, &approved.target)?;
    }
    let second = native_candidates_for(Some(&approved.identity.product_code))?;
    let registered_after = exact_registration(&approved.identity.product_code)?;
    if !registered_after { verify_arp_absent(&approved.identity.product_code)?; }
    if registered_before != registered_after || registered_after != !second.is_empty() {
        return Err("purge exact registration changed".into());
    }
    for candidate in &second {
        if !candidate.package_code.eq_ignore_ascii_case(&approved.package_code) { return Ok(Probe::IdentityMismatch); }
        let _source = super::package_gate::lock_verified_file(&candidate.source_path, &approved.target)?;
        let _cache = super::package_gate::lock_verified_file(&candidate.cache_path, &approved.target)?;
    }
    if first.iter().map(|v| (&v.registered, &v.package_code, &v.source_path)).collect::<Vec<_>>()
        != second.iter().map(|v| (&v.registered, &v.package_code, &v.source_path)).collect::<Vec<_>>() {
        return Err("purge registration changed".into());
    }
    let before = classify(approved, Ok(first));
    let after = classify(approved, Ok(second));
    if before != after { return Err("purge registration changed".into()); }
    Ok(after)
}

#[cfg(not(test))]
fn verify_arp_absent(product: &str) -> Result<(), String> {
    use winreg::{enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY}, RegKey};
    let path = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{product}");
    for view in [KEY_WOW64_32KEY, KEY_WOW64_64KEY] {
        match RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(&path, KEY_READ | view) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(format!("purge ARP query: {e}")),
            Ok(_) => return Err("purge ARP registration remains".into()),
        }
    }
    Ok(())
}

fn exact_registration(product: &str) -> Result<bool, String> {
    let product = product.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut code = [0u16; 39]; let mut context = 0;
    let status = unsafe { MsiEnumProductsExW(product.as_ptr(), std::ptr::null(), 4, 0,
        code.as_mut_ptr(), &mut context, std::ptr::null_mut(), std::ptr::null_mut()) };
    match status {
        0 if context == 4 => Ok(true),
        259 => Ok(false),
        other => Err(format!("exact MSI registration query failed: {other}")),
    }
}

pub struct Adapter<'a, Q, U, C> {
    pub approved: &'a Approved,
    pub query: Q,
    pub installer: U,
    pub cleanup: C,
}

pub fn coordinate<Q, U, C>(approved: Approved, query: Q, installer: U, mut cleanup: C) -> super::purge_policy::Outcome
where Q: FnMut() -> Result<Vec<Candidate>, String>,
      U: FnMut() -> super::purge_policy::InstallerResult,
      C: FnMut() -> Result<(), ()> {
    let product = super::purge_policy::Product { code: &approved.identity.product_code };
    let outcome = super::purge_policy::coordinate(&product, &mut Adapter {
        approved: &approved, query, installer, cleanup: || Ok(()) });
    if outcome != super::purge_policy::Outcome::Complete { return outcome; }
    let _verified_identity = approved.identity;
    drop(approved.locks);
    #[cfg(not(test))]
    drop(approved._package);
    match cleanup() {
        Ok(()) => super::purge_policy::Outcome::Complete,
        Err(()) => super::purge_policy::Outcome::CleanupFailed,
    }
}

impl<Q, U, C> super::purge_policy::Backend for Adapter<'_, Q, U, C>
where Q: FnMut() -> Result<Vec<Candidate>, String>,
      U: FnMut() -> super::purge_policy::InstallerResult,
      C: FnMut() -> Result<(), ()> {
    fn probe(&mut self, product: &super::purge_policy::Product) -> Probe {
        if product.code != self.approved.identity.product_code { return Probe::IdentityMismatch; }
        let first = classify(self.approved, (self.query)());
        if !matches!(first, Probe::RegisteredVerified | Probe::AbsentVerified) { return first; }
        let second = classify(self.approved, (self.query)());
        if first == second { second } else { Probe::Unknown }
    }
    fn uninstall(&mut self, _: &super::purge_policy::Product) -> super::purge_policy::InstallerResult { (self.installer)() }
    fn cleanup(&mut self, _: &super::purge_policy::Product) -> Result<(), ()> { (self.cleanup)() }
}

#[link(name = "msi")]
extern "system" {
    fn MsiGetSummaryInformationW(database: u32, path: *const u16, count: u32, summary: *mut u32) -> u32;
    fn MsiSummaryInfoGetPropertyW(summary: u32, property: u32, kind: *mut u32, number: *mut i32,
        time: *mut u64, value: *mut u16, count: *mut u32) -> u32;
    fn MsiCloseHandle(handle: u32) -> u32;
    fn MsiSourceListGetInfoW(product: *const u16, sid: *const u16, context: u32, options: u32,
        property: *const u16, value: *mut u16, size: *mut u32) -> u32;
    fn MsiEnumProductsExW(product: *const u16, sid: *const u16, context: u32, index: u32,
        installed: *mut u16, installed_context: *mut u32, installed_sid: *mut u16, size: *mut u32) -> u32;
    fn MsiGetProductInfoExW(product: *const u16, sid: *const u16, context: u32,
        property: *const u16, value: *mut u16, size: *mut u32) -> u32;
}
