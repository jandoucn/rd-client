use std::{
    path::{Path, PathBuf},
    ptr,
};
#[path = "../../management_agent/msi_metadata.rs"]
mod msi_metadata;

const UNAVAILABLE: &str = "admission_metadata_unavailable";
#[link(name = "msi")]
extern "system" {
    fn MsiOpenDatabaseW(path: *const u16, mode: *const u16, db: *mut u32) -> u32;
    fn MsiDatabaseOpenViewW(db: u32, query: *const u16, view: *mut u32) -> u32;
    fn MsiViewExecute(view: u32, record: u32) -> u32;
    fn MsiViewFetch(view: u32, record: *mut u32) -> u32;
    fn MsiCloseHandle(handle: u32) -> u32;
    fn MsiEnumRelatedProductsW(
        upgrade: *const u16,
        reserved: u32,
        index: u32,
        product: *mut u16,
    ) -> u32;
    fn MsiGetProductInfoExW(
        product: *const u16,
        sid: *const u16,
        context: u32,
        property: *const u16,
        value: *mut u16,
        count: *mut u32,
    ) -> u32;
}
struct Handle(u32);
impl Drop for Handle {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { MsiCloseHandle(self.0) };
        }
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn row(db: &Handle, query: &str) -> Result<bool, String> {
    let mut view = Handle(0);
    if unsafe { MsiDatabaseOpenViewW(db.0, wide(query).as_ptr(), &mut view.0) } != 0
        || unsafe { MsiViewExecute(view.0, 0) } != 0
    {
        return Err(UNAVAILABLE.into());
    }
    let mut record = Handle(0);
    match unsafe { MsiViewFetch(view.0, &mut record.0) } {
        0 => Ok(true),
        259 => Ok(false),
        _ => Err(UNAVAILABLE.into()),
    }
}
pub(crate) fn inspect_capability(path: &Path) -> Result<(), String> {
    use std::os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    };
    if !path.is_absolute()
        || !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("msi"))
    {
        return Err(UNAVAILABLE.into());
    }
    for ancestor in path.ancestors() {
        if std::fs::symlink_metadata(ancestor)
            .map_err(|_| UNAVAILABLE)?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err(UNAVAILABLE.into());
        }
    }
    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|_| UNAVAILABLE)?;
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut db = Handle(0);
    if unsafe { MsiOpenDatabaseW(name.as_ptr(), ptr::null(), &mut db.0) } != 0 {
        return Err(UNAVAILABLE.into());
    }
    // Query every required table even when another table advertises admission.
    let marker = row(
        &db,
        "SELECT `Value` FROM `Property` WHERE `Property`='RUSTDESK_MSI_ADMISSION_SCHEMA'",
    )?;
    let action=row(&db,"SELECT `Action` FROM `CustomAction` WHERE `Action`='CheckAgentInstallAdmission' OR `Target`='CheckAgentInstallAdmission'")?;
    let sequence = row(
        &db,
        "SELECT `Action` FROM `InstallExecuteSequence` WHERE `Action`='CheckAgentInstallAdmission'",
    )?;
    if marker || action || sequence {
        return Err("unsupported_admission".into());
    }
    Ok(())
}

pub(crate) fn check_pair(target: &Path, cached: &Path) -> Result<(), String> {
    inspect_capability(target)?;
    inspect_capability(cached)
}

fn product_info(product: &str, key: &str) -> Result<String, String> {
    let mut buffer = [0u16; 32768];
    let mut count = 32767;
    if unsafe {
        MsiGetProductInfoExW(
            wide(product).as_ptr(),
            ptr::null(),
            4,
            wide(key).as_ptr(),
            buffer.as_mut_ptr(),
            &mut count,
        )
    } != 0
        || count == 0
        || count >= 32767
    {
        return Err(UNAVAILABLE.into());
    }
    String::from_utf16(&buffer[..count as usize]).map_err(|_| UNAVAILABLE.into())
}

fn installed_cache(target: &Path) -> Result<PathBuf, String> {
    let identity = msi_metadata::inspect(target).map_err(|_| UNAVAILABLE)?;
    if identity.product_name != "RustDesk" {
        return Err(UNAVAILABLE.into());
    }
    let upgrade = wide(&identity.upgrade_code);
    let mut selected: Option<(String, PathBuf)> = None;
    for index in 0..1024 {
        let mut code = [0u16; 39];
        let result =
            unsafe { MsiEnumRelatedProductsW(upgrade.as_ptr(), 0, index, code.as_mut_ptr()) };
        if result == 259 {
            let (product, cache) = selected.ok_or(UNAVAILABLE)?;
            let cached = msi_metadata::inspect(&cache).map_err(|_| UNAVAILABLE)?;
            if cached.product_code != product
                || cached.product_name != "RustDesk"
                || !cached
                    .upgrade_code
                    .eq_ignore_ascii_case(&identity.upgrade_code)
            {
                return Err(UNAVAILABLE.into());
            }
            return Ok(cache);
        }
        if result != 0 {
            return Err(UNAVAILABLE.into());
        }
        let product = String::from_utf16(&code[..38]).map_err(|_| UNAVAILABLE)?;
        // Never silently ignore an unreadable/other-context related registration.
        if product_info(&product, "InstalledProductName")? != "RustDesk" {
            return Err(UNAVAILABLE.into());
        }
        let location = product_info(&product, "InstallLocation")?;
        if !location
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(r"C:\Program Files\RustDesk")
        {
            return Err(UNAVAILABLE.into());
        }
        let cache = PathBuf::from(product_info(&product, "LocalPackage")?);
        if !cache.parent().is_some_and(|p| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(r"C:\Windows\Installer")
        }) || !cache
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("msi"))
            || selected.is_some()
        {
            return Err(UNAVAILABLE.into());
        }
        selected = Some((product, cache));
    }
    Err(UNAVAILABLE.into())
}

pub(crate) fn check_installed(target: &Path) -> Result<(), String> {
    inspect_capability(target)?;
    let cache = installed_cache(target)?;
    check_pair(target, &cache)
}
