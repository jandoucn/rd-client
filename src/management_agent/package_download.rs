use super::{package_gate, workspace};
use hbb_common::UpdateTarget;
use std::{io::Read, path::{Path, PathBuf}, time::Duration};

pub fn download(directory: &Path, target: &UpdateTarget) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    if !target.mirrors.is_empty() {
        return download_with_mirrors(directory, target, |source| Ok(source.to_owned()));
    }
    download_from(directory, target, &target.primary)
}

#[cfg(test)]
pub fn download_fixture(directory: &Path, target: &UpdateTarget, address: std::net::SocketAddr) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("package fixture must use a loopback listener".into());
    }
    if !target.mirrors.is_empty() {
        return download_with_mirrors(directory, target, |source| {
            let name = package_gate::cache_name(source, "msi")?;
            Ok(format!("http://{address}/{name}"))
        });
    }
    let name = package_gate::cache_name(&target.primary, "msi")?;
    download_from(directory, target, &format!("http://{address}/{name}"))
}

fn download_with_mirrors(
    directory: &Path,
    target: &UpdateTarget,
    request_url: impl Fn(&str) -> Result<String, String>,
) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    download_bounded(directory, target, request_url, Duration::from_secs(60))
}

fn download_bounded(
    directory: &Path,
    target: &UpdateTarget,
    request_url: impl Fn(&str) -> Result<String, String>,
    timeout: Duration,
) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    let deadline = std::time::Instant::now() + timeout;
    if target.mirrors.len() > 8 {
        return Err("approved package mirror count exceeds limit".into());
    }
    if target.size > 2 * 1024 * 1024 * 1024 {
        return Err("approved package exceeds download limit".into());
    }
    let name = package_gate::cache_name(&target.primary, "msi")?;
    if directory.exists() {
        return download_from(directory, target, &request_url(&target.primary)?);
    }
    let mut remaining = target.size * 2 + 1;
    let primary_error = match download_attempt(
        directory,
        target,
        &request_url(&target.primary)?,
        deadline,
        &mut remaining,
    ) {
        Ok(package) => return Ok(package),
        Err(error) => error,
    };
    workspace::verify(directory)?;
    let cache = directory.join(&name);
    let mut errors = vec![primary_error];
    let mut seen = std::collections::HashSet::from([target.primary.as_str()]);
    for source in &target.mirrors {
        if package_gate::cache_name(source, "msi").is_err() || !seen.insert(source.as_str()) {
            continue;
        }
        if let Err(error) = attempt_budget(deadline, remaining, target.size) {
            errors.push(error);
            return Err(errors.join("; "));
        }
        // Each source keeps its own partial evidence; the primary MSI basename stays authoritative.
        let attempt = directory.join(format!("mirror-{}", errors.len()));
        let (path, locked) = match download_attempt(
            &attempt,
            target,
            &request_url(source)?,
            deadline,
            &mut remaining,
        ) {
            Ok(package) => package,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        attempt_budget(deadline, u64::MAX, 0)?;
        drop(locked);
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
        }
        let source = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let destination = cache
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 8) } == 0 {
            return Err(format!(
                "approved package publication refused: {}",
                std::io::Error::last_os_error()
            ));
        }
        let locked = package_gate::lock_verified_file(&cache, target)?;
        attempt_budget(deadline, u64::MAX, 0)?;
        return Ok((cache, locked));
    }
    Err(errors.join("; "))
}

#[cfg(test)]
pub fn download_budget_fixture(
    directory: &Path,
    target: &UpdateTarget,
    address: std::net::SocketAddr,
    timeout: Duration,
) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("package fixture must use a loopback listener".into());
    }
    download_bounded(
        directory,
        target,
        |source| {
            Ok(format!(
                "http://{address}/{}",
                package_gate::cache_name(source, "msi")?
            ))
        },
        timeout,
    )
}

fn attempt_budget(
    deadline: std::time::Instant,
    remaining: u64,
    size: u64,
) -> Result<Duration, String> {
    let time = deadline
        .checked_duration_since(std::time::Instant::now())
        .filter(|time| !time.is_zero())
        .ok_or("approved package time budget exhausted")?;
    if remaining < size {
        return Err("approved package byte budget exhausted".into());
    }
    Ok(time)
}

fn download_attempt(
    directory: &Path,
    target: &UpdateTarget,
    url: &str,
    deadline: std::time::Instant,
    remaining: &mut u64,
) -> Result<(PathBuf, package_gate::LockedPackage), String> {
    use std::io::Write;
    let time = attempt_budget(deadline, *remaining, target.size)?;
    workspace::create(directory)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(time.min(Duration::from_secs(5)))
        .timeout(time)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client
        .get(url)
        .timeout(attempt_budget(deadline, *remaining, target.size)?)
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                "approved package time budget exhausted".into()
            } else {
                error.to_string()
            }
        })?;
    if !response.status().is_success() {
        return Err(format!("approved package HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length != target.size)
    {
        return Err("approved package content length mismatch".into());
    }
    let partial = directory.join("package.partial");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|error| error.to_string())?;
    let mut copied = 0u64;
    let mut complete = response.content_length() == Some(target.size);
    let mut buffer = [0u8; 65536];
    while copied < target.size + 1 && *remaining > 0 {
        attempt_budget(deadline, u64::MAX, 0)?;
        let count = buffer
            .len()
            .min((target.size + 1 - copied).min(*remaining) as usize);
        let count = response.read(&mut buffer[..count]).map_err(|error| {
            if std::time::Instant::now() >= deadline {
                "approved package time budget exhausted".into()
            } else {
                error.to_string()
            }
        })?;
        if count == 0 {
            complete = true;
            break;
        }
        *remaining -= count as u64;
        copied += count as u64;
        file.write_all(&buffer[..count])
            .map_err(|error| error.to_string())?;
    }
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);
    attempt_budget(deadline, u64::MAX, 0)?;
    if copied != target.size {
        return Err("approved package size mismatch".into());
    }
    if !complete {
        return Err("approved package byte budget exhausted before body boundary".into());
    }
    let verified = package_gate::lock_verified_file(&partial, target)?;
    drop(verified);
    attempt_budget(deadline, u64::MAX, 0)?;
    let cache = directory.join(package_gate::cache_name(&target.primary, "msi")?);
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let source = partial
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = cache
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 8) } == 0 {
        return Err(format!(
            "approved package publication refused: {}",
            std::io::Error::last_os_error()
        ));
    }
    let locked = package_gate::lock_verified_file(&cache, target)?;
    attempt_budget(deadline, u64::MAX, 0)?;
    Ok((cache, locked))
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
