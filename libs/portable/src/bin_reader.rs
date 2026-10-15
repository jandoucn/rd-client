use std::{
    collections::HashSet,
    fs::{self},
    io::{Cursor, Read},
    path::Path,
};

// The generic payload, shared by every customer and compiled in once per release.
#[cfg(windows)]
const BIN_DATA: &[u8] = include_bytes!("../data.bin");

// The per-customer payload, injected into the RCDATA resource after the template
// has been built, so that customizing a client needs no recompilation.
#[cfg(windows)]
const PACKAGE_RESOURCE_NAME: &str = "RDPKG";

// 4bytes
const LENGTH: usize = 4;
const IDENTIFIER: &[u8] = b"rustdesk";
const IDENTIFIER_LENGTH: usize = 8;
const MD5_LENGTH: usize = 32;
const BUF_SIZE: usize = 4096;

#[cfg(windows)]
#[used]
static STRICT_ASSETS_CAPABILITY: [u8; 28] = *b"RUSTDESK_STRICT_ASSETS_V1\0\0\0";

#[cfg(windows)]
pub(crate) fn strict_assets(destination: Option<&Path>) -> Result<(), String> {
    strict::run(destination)
}

// The managed branch derives its inventory from the signed image, never from output files.
#[cfg(windows)]
mod strict {
    use super::*;
    use std::{collections::BTreeSet, io::Write, os::windows::fs::MetadataExt};

    const MAX_FILES: usize = 20000;
    const MAX_FILE: u64 = 512 * 1024 * 1024;
    const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;

    struct Entry<'a> {
        path: String,
        raw: &'a [u8],
        md5_code: &'a [u8],
    }

    fn absent_resource_error(code: u32) -> bool {
        matches!(code, 1813 | 1814)
    }

    // Only TYPE/NAME_NOT_FOUND means no optional overlay. A broken resource is not absent.
    fn read_resource_strict(name: &str) -> Result<Option<Vec<u8>>, String> {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetModuleHandleW(name: *const u16) -> *mut c_void;
            fn FindResourceW(
                module: *mut c_void,
                name: *const u16,
                kind: *const u16,
            ) -> *mut c_void;
            fn SizeofResource(module: *mut c_void, resource: *mut c_void) -> u32;
            fn LoadResource(module: *mut c_void, resource: *mut c_void) -> *mut c_void;
            fn LockResource(resource: *mut c_void) -> *mut c_void;
            fn GetLastError() -> u32;
        }
        let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            if module.is_null() {
                let code = GetLastError();
                return Err(format!("strict:resource-module:{code}"));
            }
            let resource = FindResourceW(module, name.as_ptr(), 10usize as *const u16);
            if resource.is_null() {
                let code = GetLastError();
                return if absent_resource_error(code) {
                    Ok(None)
                } else {
                    Err(format!("strict:resource-find:{code}"))
                };
            }
            let size = SizeofResource(module, resource);
            if size == 0 {
                return Err("strict:resource-empty".into());
            }
            if size as u64 > 2 * 1024 * 1024 * 1024 {
                return Err("strict:resource-limit".into());
            }
            let loaded = LoadResource(module, resource);
            if loaded.is_null() {
                let code = GetLastError();
                return Err(format!("strict:resource-load:{code}"));
            }
            let data = LockResource(loaded);
            // LockResource does not set LastError; do not report a stale error code.
            if data.is_null() {
                return Err("strict:resource-lock".into());
            }
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(size as usize)
                .map_err(|_| "strict:resource-allocation")?;
            bytes.extend_from_slice(std::slice::from_raw_parts(data.cast::<u8>(), size as usize));
            Ok(Some(bytes))
        }
    }

    pub(super) fn relative(value: &str) -> Result<String, String> {
        let value = value.replace('\\', "/");
        let value = value.strip_prefix("./").unwrap_or(&value);
        if value.is_empty() || value.len() > 240 {
            return Err("strict:path".into());
        }
        for part in value.split('/') {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            if part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || part
                    .chars()
                    .any(|c| c.is_control() || !c.is_ascii() || ":*?\"<>|".contains(c))
                || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
            {
                return Err("strict:path".into());
            }
        }
        Ok(value.to_ascii_lowercase())
    }

    fn records(blob: &[u8]) -> Result<(Vec<Entry<'_>>, String), String> {
        if blob.len() > 2 * 1024 * 1024 * 1024usize || !blob.starts_with(IDENTIFIER) {
            return Err("strict:format".into());
        }
        let mut at = 8usize;
        let mut files = Vec::new();
        let mut seen = BTreeSet::new();
        let take = |at: &mut usize, n: usize| -> Result<&[u8], String> {
            let end = at.checked_add(n).ok_or("strict:format")?;
            let bytes = blob.get(*at..end).ok_or("strict:format")?;
            *at = end;
            Ok(bytes)
        };
        loop {
            if blob.get(at..at + 8) == Some(IDENTIFIER) {
                at += 8;
                break;
            }
            if files.len() >= MAX_FILES {
                return Err("strict:limit".into());
            }
            let n = read_u32(blob, at).ok_or("strict:format")? as usize;
            at += 4;
            if n > 240 {
                return Err("strict:path".into());
            }
            let path =
                relative(std::str::from_utf8(take(&mut at, n)?).map_err(|_| "strict:path")?)?;
            if !seen.insert(path.clone()) {
                return Err("strict:duplicate".into());
            }
            let n = read_u32(blob, at).ok_or("strict:format")? as usize;
            at += 4;
            if n == 0 || n as u64 > MAX_FILE {
                return Err("strict:limit".into());
            }
            let raw = take(&mut at, n)?;
            let md5_code = take(&mut at, 32)?;
            if !md5_code.iter().all(u8::is_ascii_hexdigit) {
                return Err("strict:format".into());
            }
            files.push(Entry {
                path,
                raw,
                md5_code,
            });
        }
        let exe = relative(
            std::str::from_utf8(blob.get(at..).ok_or("strict:format")?)
                .map_err(|_| "strict:format")?,
        )?;
        Ok((files, exe))
    }

    fn files(package: Option<&[u8]>) -> Result<Vec<Entry<'_>>, String> {
        let (mut files, generic_exe) = records(BIN_DATA)?;
        let (overlay, exe) = match package {
            Some(blob) => records(blob)?,
            None => (Vec::new(), generic_exe.clone()),
        };
        if generic_exe != exe {
            for file in &mut files {
                if file.path == generic_exe {
                    file.path = exe.clone();
                }
            }
        }
        let replaced = overlay
            .iter()
            .map(|file| file.path.as_str())
            .collect::<BTreeSet<_>>();
        files.retain(|file| !replaced.contains(file.path.as_str()));
        files.extend(overlay);
        if exe != "rustdesk.exe" || files.len() > MAX_FILES {
            return Err("strict:required".into());
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let names = files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<BTreeSet<_>>();
        if names.len() != files.len() {
            return Err("strict:duplicate".into());
        }
        for name in &names {
            let mut parent = Path::new(name).parent();
            while let Some(p) = parent {
                if names.contains(p.to_str().ok_or("strict:path")?) {
                    return Err("strict:collision".into());
                }
                parent = p.parent();
            }
        }
        for name in [
            "rustdesk.exe",
            "librustdesk.dll",
            "rustdesk-management-agent.exe",
            "flutter_windows.dll",
            "data/icudtl.dat",
        ] {
            if !names.contains(name) {
                return Err("strict:required".into());
            }
        }
        if !names.iter().any(|p| p.starts_with("data/flutter_assets/"))
            || !names
                .iter()
                .any(|p| !p.contains('/') && p.ends_with("_plugin.dll"))
        {
            return Err("strict:required".into());
        }
        Ok(files)
    }

    struct Hash {
        algorithm: *mut std::ffi::c_void,
        hash: *mut std::ffi::c_void,
    }
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptOpenAlgorithmProvider(
            out: *mut *mut std::ffi::c_void,
            name: *const u16,
            implementation: *const u16,
            flags: u32,
        ) -> i32;
        fn BCryptCreateHash(
            algorithm: *mut std::ffi::c_void,
            out: *mut *mut std::ffi::c_void,
            object: *mut u8,
            size: u32,
            secret: *mut u8,
            secret_size: u32,
            flags: u32,
        ) -> i32;
        fn BCryptHashData(
            hash: *mut std::ffi::c_void,
            data: *const u8,
            size: u32,
            flags: u32,
        ) -> i32;
        fn BCryptFinishHash(
            hash: *mut std::ffi::c_void,
            data: *mut u8,
            size: u32,
            flags: u32,
        ) -> i32;
        fn BCryptDestroyHash(hash: *mut std::ffi::c_void) -> i32;
        fn BCryptCloseAlgorithmProvider(algorithm: *mut std::ffi::c_void, flags: u32) -> i32;
    }
    impl Hash {
        fn new() -> Result<Self, String> {
            let mut h = Self {
                algorithm: std::ptr::null_mut(),
                hash: std::ptr::null_mut(),
            };
            let name = "SHA256\0".encode_utf16().collect::<Vec<_>>();
            if unsafe {
                BCryptOpenAlgorithmProvider(&mut h.algorithm, name.as_ptr(), std::ptr::null(), 0)
            } < 0
            {
                return Err("strict:crypto".into());
            }
            if unsafe {
                BCryptCreateHash(
                    h.algorithm,
                    &mut h.hash,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    0,
                    0,
                )
            } < 0
            {
                return Err("strict:crypto".into());
            }
            Ok(h)
        }
        fn update(&self, bytes: &[u8]) -> Result<(), String> {
            if unsafe { BCryptHashData(self.hash, bytes.as_ptr(), bytes.len() as u32, 0) } < 0 {
                Err("strict:crypto".into())
            } else {
                Ok(())
            }
        }
        fn finish(&self) -> Result<String, String> {
            let mut bytes = [0u8; 32];
            if unsafe { BCryptFinishHash(self.hash, bytes.as_mut_ptr(), 32, 0) } < 0 {
                return Err("strict:crypto".into());
            }
            Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
        }
    }
    impl Drop for Hash {
        fn drop(&mut self) {
            unsafe {
                if !self.hash.is_null() {
                    BCryptDestroyHash(self.hash);
                }
                if !self.algorithm.is_null() {
                    BCryptCloseAlgorithmProvider(self.algorithm, 0);
                }
            }
        }
    }

    fn regular_chain(path: &Path) -> Result<(), String> {
        for p in path.ancestors() {
            let m = std::fs::symlink_metadata(p).map_err(|_| "strict:directory")?;
            if m.file_attributes() & 0x400 != 0 || !m.is_dir() {
                return Err("strict:reparse".into());
            }
        }
        Ok(())
    }

    pub(super) fn run(destination: Option<&Path>) -> Result<(), String> {
        // This reference keeps the capability marker in the signed image even in release builds.
        if std::hint::black_box(&STRICT_ASSETS_CAPABILITY)[0] != b'R' {
            return Err("strict:format".into());
        }
        let resource = read_resource_strict(PACKAGE_RESOURCE_NAME)?;
        let files = files(resource.as_deref())?;
        if let Some(path) = destination {
            let exe = std::env::current_exe().map_err(|_| "strict:directory")?;
            let parent = exe.parent().ok_or("strict:directory")?;
            if path != parent.join("payload") {
                return Err("strict:destination".into());
            }
            regular_chain(parent)?;
            if std::fs::symlink_metadata(path).is_ok() {
                return Err("strict:occupied".into());
            }
            std::fs::create_dir(path).map_err(|_| "strict:occupied")?;
        }
        let mut total = 0u64;
        let mut inventory = String::from("rustdesk-strict-assets-v1\n");
        for file in files {
            let mut output = if let Some(root) = destination {
                let path = root.join(&file.path);
                let parent = path.parent().ok_or("strict:path")?;
                std::fs::create_dir_all(parent).map_err(|_| "strict:write")?;
                regular_chain(parent)?;
                Some(
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .map_err(|_| "strict:occupied")?,
                )
            } else {
                None
            };
            let mut decoder = brotli::Decompressor::new(Cursor::new(file.raw), BUF_SIZE);
            let hash = Hash::new()?;
            let mut md5 = md5::Context::new();
            let mut size = 0u64;
            let mut buffer = [0u8; 65536];
            loop {
                let n = decoder
                    .read(&mut buffer)
                    .map_err(|_| "strict:compression")?;
                if n == 0 {
                    break;
                }
                size += n as u64;
                total += n as u64;
                if size > MAX_FILE || total > MAX_TOTAL {
                    return Err("strict:limit".into());
                }
                hash.update(&buffer[..n])?;
                md5.consume(&buffer[..n]);
                if let Some(out) = output.as_mut() {
                    out.write_all(&buffer[..n]).map_err(|_| "strict:write")?;
                }
            }
            if size == 0 {
                return Err("strict:empty".into());
            }
            if format!("{:x}", md5.compute()).as_bytes() != file.md5_code {
                return Err("strict:checksum".into());
            }
            if let Some(out) = output {
                out.sync_all().map_err(|_| "strict:write")?;
            }
            inventory.push_str(&format!("{}\t{}\t{}\n", file.path, size, hash.finish()?));
            if inventory.len() > 8 * 1024 * 1024 {
                return Err("strict:limit".into());
            }
        }
        inventory.push_str("end\n");
        if destination.is_none() {
            std::io::stdout()
                .write_all(inventory.as_bytes())
                .map_err(|_| "strict:output")?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn strict_resource_absence_is_only_type_or_name_not_found() {
            for code in [0, 2, 5, 6, 8, 1812, 1813, 1814, 1815, 193] {
                assert_eq!(
                    absent_resource_error(code),
                    matches!(code, 1813 | 1814),
                    "{code}"
                );
            }
            assert!(read_resource_strict("RDPKG_STRICT_TEST_ABSENT")
                .unwrap()
                .is_none());
        }

        #[test]
        fn strict_paths_reject_windows_escape_and_aliases() {
            for path in [
                "../evil",
                "C:/evil",
                "/evil",
                "a//b",
                "./../evil",
                "a:stream",
                "NUL.exe",
                "com1.txt",
                "a./b",
                "a /b",
                "a\tb",
            ] {
                assert!(relative(path).is_err(), "{path}");
            }
            assert_eq!(relative(".\\RustDesk.exe").unwrap(), "rustdesk.exe");
        }

        #[test]
        fn strict_blob_validates_boundaries_duplicates_and_record_limits() {
            fn blob(names: &[&str]) -> Vec<u8> {
                let mut bytes = b"rustdesk".to_vec();
                for name in names {
                    bytes.extend_from_slice(&(name.len() as u32).to_be_bytes());
                    bytes.extend_from_slice(name.as_bytes());
                    bytes.extend_from_slice(&1u32.to_be_bytes());
                    bytes.push(0);
                    bytes.extend_from_slice(&[b'a'; 32]);
                }
                bytes.extend_from_slice(b"rustdeskrustdesk.exe");
                bytes
            }
            let good = blob(&["./rustdesk.exe"]);
            assert_eq!(
                records(Box::leak(good.clone().into_boxed_slice()))
                    .unwrap()
                    .0
                    .len(),
                1
            );
            for cut in [0, 7, 12, good.len() - 16] {
                assert!(records(Box::leak(good[..cut].to_vec().into_boxed_slice())).is_err());
            }
            assert!(records(Box::leak(blob(&["a", "A"]).into_boxed_slice())).is_err());
            assert!(records(Box::leak(blob(&["../escape"]).into_boxed_slice())).is_err());
            let names = (0..20001).map(|n| format!("{n}.dll")).collect::<Vec<_>>();
            assert!(records(Box::leak(
                blob(&names.iter().map(String::as_str).collect::<Vec<_>>()).into_boxed_slice()
            ))
            .is_err());
        }
    }
}

pub(crate) struct BinaryData {
    pub md5_code: &'static [u8],
    // compressed gzip data
    pub raw: &'static [u8],
    pub path: String,
}

pub(crate) struct BinaryReader {
    pub files: Vec<BinaryData>,
    pub exe: String,
    // Paths supplied by the per-customer package. Recorded so that a file dropped
    // from a later package -- a logo the customer removed, say -- can be deleted
    // from an existing extraction, which the timestamp wipe no longer covers now
    // that the packer is built once per release rather than once per customer.
    pub package_paths: Vec<String>,
}

impl BinaryReader {
    pub fn new() -> Result<Self, String> {
        let package = read_package()?;
        let package_paths = package.0.iter().map(|f| f.path.clone()).collect();
        let (files, exe) = merge(read_embedded()?, package);
        Ok(Self {
            files,
            exe,
            package_paths,
        })
    }
}

// Folds the per-customer package into the generic payload.
fn merge(
    embedded: (Vec<BinaryData>, String),
    package: (Vec<BinaryData>, String),
) -> (Vec<BinaryData>, String) {
    let (mut files, generic_exe) = embedded;
    let (package_files, package_exe) = package;

    let exe = if package_exe.is_empty() {
        generic_exe.clone()
    } else {
        package_exe
    };

    // The generic payload ships the executable under its stock name, the package
    // decides the final one. Rename on extraction so the process is always
    // `<appname>.exe`, which the app itself relies on to find its own sessions.
    if !generic_exe.is_empty() && normalize_path(&exe) != normalize_path(&generic_exe) {
        let generic_key = normalize_path(&generic_exe);
        for file in files.iter_mut() {
            if normalize_path(&file.path) == generic_key {
                file.path = exe.clone();
            }
        }
    }

    // Per-customer entries replace the generic ones they shadow.
    if !package_files.is_empty() {
        let overridden: HashSet<String> = package_files
            .iter()
            .map(|file| normalize_path(&file.path))
            .collect();
        files.retain(|file| !overridden.contains(&normalize_path(&file.path)));
        files.extend(package_files);
    }

    (files, exe)
}

pub(crate) fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .to_lowercase()
}

fn read_u32(blob: &[u8], at: usize) -> Option<u32> {
    let bytes = blob.get(at..at + LENGTH)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

// Returns the files and the executable to launch, or None if the blob is absent or malformed.
fn parse(blob: &'static [u8]) -> Option<(Vec<BinaryData>, String)> {
    let mut base = 0usize;
    let mut parsed = Vec::new();
    if blob.get(base..base + IDENTIFIER_LENGTH)? != IDENTIFIER {
        return None;
    }
    base += IDENTIFIER_LENGTH;
    loop {
        if blob.get(base..base + IDENTIFIER_LENGTH)? == IDENTIFIER {
            base += IDENTIFIER_LENGTH;
            break;
        }
        let path_length = read_u32(blob, base)? as usize;
        base += LENGTH;
        let path = std::str::from_utf8(blob.get(base..base + path_length)?)
            .ok()?
            .to_owned();
        base += path_length;
        let file_length = read_u32(blob, base)? as usize;
        base += LENGTH;
        let raw = blob.get(base..base + file_length)?;
        base += file_length;
        let md5_code = blob.get(base..base + MD5_LENGTH)?;
        base += MD5_LENGTH;
        parsed.push(BinaryData {
            md5_code,
            raw,
            path,
        });
    }
    let executable = std::str::from_utf8(blob.get(base..)?).ok()?.to_owned();
    Some((parsed, executable))
}

#[cfg(windows)]
fn read_embedded() -> Result<(Vec<BinaryData>, String), String> {
    parse(BIN_DATA).ok_or_else(|| "bin file is not valid!".to_owned())
}

#[cfg(not(windows))]
fn read_embedded() -> Result<(Vec<BinaryData>, String), String> {
    Ok(Default::default())
}

fn parse_package_blob(blob: Option<&'static [u8]>) -> Result<(Vec<BinaryData>, String), String> {
    let Some(blob) = blob else {
        return Ok(Default::default());
    };
    let package = parse(blob).ok_or_else(|| "RDPKG resource is invalid".to_owned())?;
    if package.1.trim().is_empty() {
        return Err("RDPKG resource has no executable".to_owned());
    }
    Ok(package)
}

#[cfg(windows)]
fn read_package() -> Result<(Vec<BinaryData>, String), String> {
    parse_package_blob(read_resource(PACKAGE_RESOURCE_NAME))
}

#[cfg(not(windows))]
fn read_package() -> Result<(Vec<BinaryData>, String), String> {
    Ok(Default::default())
}

// Reads an RCDATA resource out of the running image. Resources live in the mapped
// image for the lifetime of the process, so the slice is genuinely 'static and no
// copy is needed.
#[cfg(windows)]
fn read_resource(name: &str) -> Option<&'static [u8]> {
    use std::ptr::null_mut;
    use winapi::um::libloaderapi::{FindResourceW, LoadResource, LockResource, SizeofResource};

    // MAKEINTRESOURCEW(10), avoids depending on the winuser feature for RT_RCDATA.
    const RT_RCDATA: *const u16 = 10 as _;

    let name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let info = FindResourceW(null_mut(), name.as_ptr(), RT_RCDATA);
        if info.is_null() {
            return None;
        }
        let size = SizeofResource(null_mut(), info) as usize;
        if size == 0 {
            return None;
        }
        let handle = LoadResource(null_mut(), info);
        if handle.is_null() {
            return None;
        }
        let data = LockResource(handle) as *const u8;
        if data.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(data, size))
    }
}

impl BinaryData {
    fn decompress(&self) -> Vec<u8> {
        let cursor = Cursor::new(self.raw);
        let mut decoder = brotli::Decompressor::new(cursor, BUF_SIZE);
        let mut buf = Vec::new();
        decoder.read_to_end(&mut buf).ok();
        buf
    }

    pub fn write_to_file(&self, prefix: &Path) {
        let p = prefix.join(&self.path);
        if let Some(parent) = p.parent() {
            if !parent.exists() {
                let _ = fs::create_dir_all(parent);
            }
        }
        if p.exists() {
            // check md5
            let f = fs::read(p.clone()).unwrap_or_default();
            let digest = format!("{:x}", md5::compute(&f));
            let md5_record = String::from_utf8_lossy(self.md5_code);
            if digest == md5_record {
                // same, skip this file
                println!("skip {}", &self.path);
                return;
            } else {
                println!("writing {}", p.display());
                println!("{} -> {}", md5_record, digest)
            }
        }
        let _ = fs::write(p, self.decompress());
    }
}

impl BinaryReader {
    #[cfg(linux)]
    pub fn configure_permission(&self, prefix: &Path) {
        use std::os::unix::prelude::PermissionsExt;

        let exe_path = prefix.join(&self.exe);
        if exe_path.exists() {
            if let Ok(f) = File::open(exe_path) {
                if let Ok(meta) = f.metadata() {
                    let mut permissions = meta.permissions();
                    permissions.set_mode(0o755);
                    f.set_permissions(permissions).ok();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Builds a blob in the same layout generate.py writes, so these tests pin the
    // cross-language format contract as well as the merge rules.
    fn blob(files: &[(&str, &[u8])], exe: &str) -> &'static [u8] {
        let mut out = Vec::new();
        out.extend_from_slice(IDENTIFIER);
        for (path, data) in files {
            out.extend_from_slice(&(path.len() as u32).to_be_bytes());
            out.extend_from_slice(path.as_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(data);
            out.extend_from_slice(&[b'a'; MD5_LENGTH]);
        }
        out.extend_from_slice(IDENTIFIER);
        out.extend_from_slice(exe.as_bytes());
        Box::leak(out.into_boxed_slice())
    }

    fn entry<'a>(files: &'a [BinaryData], path: &str) -> Option<&'a BinaryData> {
        files
            .iter()
            .find(|file| normalize_path(&file.path) == normalize_path(path))
    }

    #[test]
    fn parses_the_generate_py_layout() {
        let (files, exe) = parse(blob(
            &[("./rustdesk.exe", b"app"), ("./custom.txt", b"cfg")],
            "./rustdesk.exe",
        ))
        .unwrap();
        assert_eq!(exe, "./rustdesk.exe");
        assert_eq!(files.len(), 2);
        assert_eq!(entry(&files, "./custom.txt").unwrap().raw, b"cfg");
    }

    #[test]
    fn rejects_malformed_blobs() {
        assert!(parse(b"".as_slice()).is_none());
        assert!(parse(b"notrustd".as_slice()).is_none());
        // Truncated mid-record rather than panicking on a slice out of range.
        assert!(parse(b"rustdesk\x00\x00\x00\x40partial".as_slice()).is_none());
    }

    #[test]
    fn distinguishes_an_absent_package_from_a_malformed_one() {
        assert!(parse_package_blob(None).unwrap().0.is_empty());
        assert!(parse_package_blob(Some(b"damaged")).is_err());
        assert!(parse_package_blob(Some(blob(&[("./custom.txt", b"cfg")], ""))).is_err());
    }

    #[test]
    fn without_a_package_the_stock_payload_is_untouched() {
        let embedded = parse(blob(&[("./rustdesk.exe", b"app")], "./rustdesk.exe")).unwrap();
        let (files, exe) = merge(embedded, Default::default());
        assert_eq!(exe, "./rustdesk.exe");
        assert!(entry(&files, "./rustdesk.exe").is_some());
    }

    #[test]
    fn renames_the_stock_executable_to_the_package_name() {
        // x86: the big executable stays in the generic payload and only gets renamed.
        let embedded = parse(blob(
            &[("./rustdesk.exe", b"app"), ("./sciter.dll", b"dll")],
            "./rustdesk.exe",
        ))
        .unwrap();
        let package = parse(blob(&[("./custom.txt", b"cfg")], "./acme.exe")).unwrap();

        let (files, exe) = merge(embedded, package);

        assert_eq!(exe, "./acme.exe");
        assert!(entry(&files, "./acme.exe").is_some());
        assert!(entry(&files, "./rustdesk.exe").is_none());
        // Untouched neighbours survive.
        assert_eq!(entry(&files, "./sciter.dll").unwrap().raw, b"dll");
        assert_eq!(entry(&files, "./custom.txt").unwrap().raw, b"cfg");
    }

    #[test]
    fn package_entries_win_over_the_generic_payload() {
        // x64: the customized executable and icons ship in the package instead.
        let embedded = parse(blob(
            &[
                ("./data/flutter_assets/assets/icon.ico", b"stock-icon"),
                ("./librustdesk.dll", b"core"),
            ],
            "./rustdesk.exe",
        ))
        .unwrap();
        let package = parse(blob(
            &[
                ("./acme.exe", b"branded"),
                ("./data/flutter_assets/assets/icon.ico", b"acme-icon"),
            ],
            "./acme.exe",
        ))
        .unwrap();

        let (files, exe) = merge(embedded, package);

        assert_eq!(exe, "./acme.exe");
        assert_eq!(
            entry(&files, "./data/flutter_assets/assets/icon.ico")
                .unwrap()
                .raw,
            b"acme-icon"
        );
        assert_eq!(
            files
                .iter()
                .filter(|f| normalize_path(&f.path) == "data/flutter_assets/assets/icon.ico")
                .count(),
            1
        );
        assert_eq!(entry(&files, "./librustdesk.dll").unwrap().raw, b"core");
    }

    #[test]
    fn package_paths_are_recorded_for_the_dropped_file_sweep() {
        let package = parse(blob(
            &[("./custom.txt", b"cfg"), ("./data/logo.png", b"img")],
            "./acme.exe",
        ))
        .unwrap();
        let mut paths: Vec<String> = package.0.iter().map(|f| f.path.clone()).collect();
        paths.sort();
        assert_eq!(paths, vec!["./custom.txt", "./data/logo.png"]);

        // Merging must not disturb them: the generic payload contributes none.
        let embedded = parse(blob(&[("./librustdesk.dll", b"core")], "./rustdesk.exe")).unwrap();
        let (files, _) = merge(embedded, package);
        assert!(entry(&files, "./data/logo.png").is_some());
    }

    #[test]
    fn matches_paths_across_separator_styles() {
        // generate.py emits backslashes when it runs on Windows.
        let embedded = parse(blob(&[(".\\rustdesk.exe", b"app")], ".\\rustdesk.exe")).unwrap();
        let package = parse(blob(&[("./custom.txt", b"cfg")], "./acme.exe")).unwrap();

        let (files, exe) = merge(embedded, package);

        assert_eq!(exe, "./acme.exe");
        assert!(entry(&files, "./acme.exe").is_some());
        assert!(entry(&files, ".\\rustdesk.exe").is_none());
    }
}
