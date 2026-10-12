use super::{package_gate, workspace};
use hbb_common::UpdateTarget;
use std::{io::Read, path::{Path, PathBuf}, time::Duration};

pub fn download(directory: &Path, target: &UpdateTarget) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    download_from(directory, target, &target.primary)
}

#[cfg(test)]
pub fn download_fixture(directory: &Path, target: &UpdateTarget, address: std::net::SocketAddr) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("package fixture must use a loopback listener".into());
    }
    let name = package_gate::cache_name(&target.primary, "msi")?;
    download_from(directory, target, &format!("http://{address}/{name}"))
}

fn download_from(directory: &Path, target: &UpdateTarget, url: &str) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    if target.size > 2 * 1024 * 1024 * 1024 {
        return Err("approved package exceeds download limit".into());
    }
    let cache = directory.join(package_gate::cache_name(&target.primary, "msi")?);
    if directory.exists() {
        workspace::verify(directory)?;
        let locked = package_gate::lock_verified_file(&cache, target)?;
        return Ok((cache, locked));
    }
    workspace::create(directory)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build().map_err(|error| error.to_string())?;
    let response = client.get(url).send().map_err(|error| error.to_string())?;
    receive(directory, target, response)
}

pub(super) fn receive(directory: &Path, target: &UpdateTarget, response: reqwest::blocking::Response) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    let cache = directory.join(package_gate::cache_name(&target.primary, "msi")?);
    if !response.status().is_success() {
        return Err(format!("approved package HTTP {}", response.status()));
    }
    if response.content_length().is_some_and(|length| length != target.size) {
        return Err("approved package content length mismatch".into());
    }
    let partial = directory.join("package.partial");
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
        .open(&partial).map_err(|error| error.to_string())?;
    let copied = std::io::copy(&mut response.take(target.size + 1), &mut file)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    if copied != target.size { return Err("approved package size mismatch".into()); }
    let verified = package_gate::lock_verified_file(&partial, target)?;
    drop(verified);
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let source = partial.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let destination = cache.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 8) } == 0 {
        return Err(format!("approved package publication refused: {}", std::io::Error::last_os_error()));
    }
    let locked = package_gate::lock_verified_file(&cache, target)?;
    Ok((cache, locked))
}
