use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub struct PackageIdentity {
    pub product_code: String,
    pub product_name: String,
    pub product_version: String,
    pub upgrade_code: String,
}

#[derive(Debug)]
pub struct InstalledSource {
    pub package_name: String,
    pub source: std::path::PathBuf,
}

/// How an installed product was tied to a candidate package.
#[derive(Debug)]
pub enum RegisteredPackage {
    /// The registered source file still exists; callers keep hash-verifying it.
    Source(InstalledSource),
    /// The registered source was removed (the online updater installs from a temporary directory
    /// that is deleted afterwards). Windows Installer accepts the identical package from another
    /// location, so identity is proven by the installed product's package code instead.
    PackageCode { package_name: String },
}

impl RegisteredPackage {
    pub fn package_name(&self) -> &str {
        match self {
            RegisteredPackage::Source(source) => &source.package_name,
            RegisteredPackage::PackageCode { package_name } => package_name,
        }
    }
}

/// Pure decision: the registered source file decides when present; otherwise the installed
/// package code must equal the candidate's.
pub fn removed_source_matches(
    installed_package_code: &str,
    candidate_package_code: &str,
) -> Result<(), String> {
    if !valid_guid(installed_package_code) || !valid_guid(candidate_package_code) {
        return Err("installed MSI package code invalid".into());
    }
    if !installed_package_code.eq_ignore_ascii_case(candidate_package_code) {
        return Err("installed MSI package code differs from local package".into());
    }
    Ok(())
}

fn valid_guid(value: &str) -> bool {
    value.len() == 38
        && value.bytes().enumerate().all(|(index, byte)| match index {
            0 => byte == b'{',
            37 => byte == b'}',
            9 | 14 | 19 | 24 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// Like `installed_source`, but a removed registered source is not an error when `candidate` is
/// the very package Windows Installer recorded for the product.
pub fn registered_package(
    product_code: &str,
    candidate: &Path,
) -> Result<RegisteredPackage, String> {
    let (package_name, directory) = registered_location(product_code)?;
    let source = directory.join(&package_name);
    let missing = match std::fs::symlink_metadata(&source) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => return Err("installed MSI source unreadable".into()),
        Ok(_) => false,
    };
    if !missing {
        return installed_source(product_code).map(RegisteredPackage::Source);
    }
    let candidate_identity = inspect(candidate)?;
    if candidate_identity.product_name != "RustDesk"
        || !candidate_identity
            .product_code
            .eq_ignore_ascii_case(product_code)
    {
        return Err("installed MSI source product mismatch".into());
    }
    removed_source_matches(
        &installed_package_code(product_code)?,
        &package_code(candidate)?,
    )?;
    Ok(RegisteredPackage::PackageCode { package_name })
}

fn installed_package_code(product_code: &str) -> Result<String, String> {
    let code = product_code
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let property = "PackageCode"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut buffer = vec![0u16; 64];
    let mut count = buffer.len() as u32;
    let status = unsafe {
        MsiGetProductInfoW(
            code.as_ptr(),
            property.as_ptr(),
            buffer.as_mut_ptr(),
            &mut count,
        )
    };
    if status != 0 || count as usize >= buffer.len() {
        return Err(format!("installed MSI package code query failed: {status}"));
    }
    String::from_utf16(&buffer[..count as usize])
        .map_err(|_| "installed MSI package code encoding invalid".into())
}

fn package_code(path: &Path) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut summary = Handle(0);
    let status = unsafe { MsiGetSummaryInformationW(0, wide.as_ptr(), 0, &mut summary.0) };
    if status != 0 {
        return Err(format!("MSI summary open failed: {status}"));
    }
    let (mut kind, mut integer, mut time) = (0u32, 0i32, 0u64);
    let mut buffer = vec![0u16; 64];
    let mut count = buffer.len() as u32;
    // PID_REVNUMBER (9) holds the package code.
    let status = unsafe {
        MsiSummaryInfoGetPropertyW(
            summary.0,
            9,
            &mut kind,
            &mut integer,
            &mut time,
            buffer.as_mut_ptr(),
            &mut count,
        )
    };
    if status != 0 || count as usize >= buffer.len() {
        return Err(format!("MSI package code read failed: {status}"));
    }
    String::from_utf16(&buffer[..count as usize])
        .map_err(|_| "MSI package code encoding invalid".into())
}

fn registered_location(product_code: &str) -> Result<(String, std::path::PathBuf), String> {
    if !valid_guid(product_code) {
        return Err("installed MSI product code invalid".into());
    }
    let code = product_code.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let query = |property: &str| -> Result<String, String> {
        let property = property.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut buffer = vec![0u16; 32768];
        let mut count = buffer.len() as u32;
        let status = unsafe { MsiSourceListGetInfoW(code.as_ptr(), std::ptr::null(), 4, 0,
            property.as_ptr(), buffer.as_mut_ptr(), &mut count) };
        if status != 0 || count as usize >= buffer.len() {
            return Err(format!("installed MSI source query failed: {status}"));
        }
        String::from_utf16(&buffer[..count as usize]).map_err(|_| "installed MSI source encoding invalid".into())
    };
    let name = query("PackageName")?;
    if name.is_empty() || name.len() > 180 || !name.ends_with(".msi")
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte)) {
        return Err("installed MSI source name invalid".into());
    }
    let last = query("LastUsedSource")?;
    if query("LastUsedType")? != "n" {
        return Err("installed MSI source is not a local directory".into());
    }
    let directory = std::path::PathBuf::from(last);
    use std::path::{Component, Prefix};
    if !directory.is_absolute()
        || !matches!(directory.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)))
        || directory.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err("installed MSI source is not a local directory".into());
    }
    Ok((name, directory))
}

pub fn installed_source(product_code: &str) -> Result<InstalledSource, String> {
    let (name, directory) = registered_location(product_code)?;
    let source = directory.join(&name);
    let package = inspect(&source)?;
    if package.product_name != "RustDesk" || !package.product_code.eq_ignore_ascii_case(product_code) {
        return Err("installed MSI source product mismatch".into());
    }
    Ok(InstalledSource { package_name: name, source })
}

pub fn verify_pair(original: &PackageIdentity, target: &PackageIdentity, original_code: &str, target_code: &str) -> Result<(), String> {
    if original.product_name != "RustDesk" || target.product_name != "RustDesk"
        || !original.upgrade_code.eq_ignore_ascii_case(&target.upgrade_code) {
        return Err("MSI product family mismatch".into());
    }
    if !original.product_code.eq_ignore_ascii_case(original_code)
        || !target.product_code.eq_ignore_ascii_case(target_code) {
        return Err("MSI product code differs from prepared assets".into());
    }
    Ok(())
}

#[link(name = "msi")]
extern "system" {
    fn MsiSourceListGetInfoW(product: *const u16, sid: *const u16, context: u32, options: u32,
        property: *const u16, value: *mut u16, count: *mut u32) -> u32;
    fn MsiOpenDatabaseW(path: *const u16, persist: *const u16, database: *mut u32) -> u32;
    fn MsiDatabaseOpenViewW(database: u32, query: *const u16, view: *mut u32) -> u32;
    fn MsiViewExecute(view: u32, record: u32) -> u32;
    fn MsiViewFetch(view: u32, record: *mut u32) -> u32;
    fn MsiRecordGetStringW(record: u32, field: u32, buffer: *mut u16, count: *mut u32) -> u32;
    fn MsiCloseHandle(handle: u32) -> u32;
    fn MsiGetProductInfoW(
        product: *const u16,
        property: *const u16,
        value: *mut u16,
        count: *mut u32,
    ) -> u32;
    fn MsiGetSummaryInformationW(
        database: u32,
        path: *const u16,
        updates: u32,
        summary: *mut u32,
    ) -> u32;
    fn MsiSummaryInfoGetPropertyW(
        summary: u32,
        property: u32,
        kind: *mut u32,
        integer: *mut i32,
        time: *mut u64,
        value: *mut u16,
        count: *mut u32,
    ) -> u32;
}

struct Handle(u32);
impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != 0 {
            let status = unsafe { MsiCloseHandle(self.0) };
            if status != 0 { eprintln!("MSI metadata handle close failed: {status}"); }
        }
    }
}

fn property(database: &Handle, name: &str) -> Result<String, String> {
    let query = format!("SELECT `Value` FROM `Property` WHERE `Property` = '{name}'")
        .encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut view = Handle(0);
    let status = unsafe { MsiDatabaseOpenViewW(database.0, query.as_ptr(), &mut view.0) };
    if status != 0 { return Err(format!("MSI property query failed: {status}")); }
    let status = unsafe { MsiViewExecute(view.0, 0) };
    if status != 0 { return Err(format!("MSI property read failed: {status}")); }
    let mut record = Handle(0);
    let status = unsafe { MsiViewFetch(view.0, &mut record.0) };
    if status != 0 { return Err(format!("MSI required property absent: {name}")); }
    let mut buffer = [0u16; 1025];
    let mut count = buffer.len() as u32;
    let status = unsafe { MsiRecordGetStringW(record.0, 1, buffer.as_mut_ptr(), &mut count) };
    if status != 0 || count as usize >= buffer.len() {
        return Err("MSI property too large or unreadable".into());
    }
    let value = String::from_utf16(&buffer[..count as usize]).map_err(|_| "MSI property encoding invalid")?;
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err("MSI property invalid".into());
    }
    Ok(value)
}

pub fn inspect(path: &Path) -> Result<PackageIdentity, String> {
    use std::os::windows::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt}};
    if !path.is_absolute() || path.extension().and_then(|value| value.to_str()) != Some("msi") {
        return Err("MSI metadata path invalid".into());
    }
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| "MSI metadata path unavailable")?;
        if metadata.file_attributes() & 0x400 != 0 { return Err("MSI reparse path refused".into()); }
    }
    let _lock = std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
        .open(path).map_err(|_| "MSI metadata file unavailable")?;
    let wide = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let mut database = Handle(0);
    let status = unsafe { MsiOpenDatabaseW(wide.as_ptr(), std::ptr::null(), &mut database.0) };
    if status != 0 { return Err(format!("MSI metadata database open failed: {status}")); }
    Ok(PackageIdentity {
        product_code: property(&database, "ProductCode")?,
        product_name: property(&database, "ProductName")?,
        product_version: property(&database, "ProductVersion")?,
        upgrade_code: property(&database, "UpgradeCode")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLED: &str = "{0280AA40-A3B0-42A9-B304-1779653286C8}";

    #[test]
    fn removed_source_accepts_only_the_recorded_package() {
        assert_eq!(
            removed_source_matches(INSTALLED, &INSTALLED.to_ascii_lowercase()),
            Ok(())
        );
        assert_eq!(
            removed_source_matches(INSTALLED, "{1280AA40-A3B0-42A9-B304-1779653286C8}")
                .unwrap_err(),
            "installed MSI package code differs from local package"
        );
    }

    #[test]
    fn removed_source_rejects_malformed_codes() {
        for bad in [
            "",
            "0280AA40-A3B0-42A9-B304-1779653286C8",
            "{0280AA40-A3B0-42A9-B304-1779653286CZ}",
            "{0280AA40A3B0-42A9-B304-1779653286C8-}",
        ] {
            assert_eq!(
                removed_source_matches(INSTALLED, bad).unwrap_err(),
                "installed MSI package code invalid"
            );
            assert_eq!(
                removed_source_matches(bad, INSTALLED).unwrap_err(),
                "installed MSI package code invalid"
            );
        }
    }

    #[test]
    fn registered_package_name_is_reported_for_both_kinds() {
        let removed = RegisteredPackage::PackageCode {
            package_name: "rustdesk.msi".into(),
        };
        assert_eq!(removed.package_name(), "rustdesk.msi");
        let present = RegisteredPackage::Source(InstalledSource {
            package_name: "a.msi".into(),
            source: "C:\\x\\a.msi".into(),
        });
        assert_eq!(present.package_name(), "a.msi");
    }
}
