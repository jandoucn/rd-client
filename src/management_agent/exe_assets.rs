use super::{package_gate, workspace};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, PartialEq, Eq)]
pub enum AssetError {
    Identity,
    Signature,
    Package,
    Workspace,
    Occupied,
    Inventory,
    Extraction,
    Integrity,
    Limit,
    Timeout,
}

#[derive(Debug)]
pub struct SealedAssets {
    pub root: PathBuf,
    pub files: Vec<(String, u64, String)>,
}

pub fn check_identity(
    installed_kind: &str,
    installed_edition: &str,
    target_key: &str,
) -> Result<(), AssetError> {
    if installed_kind != "exe"
        || !matches!(installed_edition, "standard" | "sos")
        || target_key != format!("windows-x86_64-exe-{installed_edition}")
    {
        return Err(AssetError::Identity);
    }
    Ok(())
}

pub fn prepare(
    parent: &Path,
    package: &Path,
    manifest: &hbb_common::UpdateManifest,
    key: &str,
    installed_kind: &str,
    edition: &str,
    minimum_build: u64,
) -> Result<SealedAssets, AssetError> {
    if parent != Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\exe-assets") {
        return Err(AssetError::Workspace);
    }
    prepare_checked(
        parent,
        package,
        manifest,
        key,
        installed_kind,
        edition,
        minimum_build,
        None,
    )
}

#[cfg(test)]
pub fn prepare_fixture(
    parent: &Path,
    package: &Path,
    manifest: &hbb_common::UpdateManifest,
    key: &str,
    installed_kind: &str,
    edition: &str,
    minimum_build: u64,
    public: &[u8],
) -> Result<SealedAssets, AssetError> {
    prepare_checked(
        parent,
        package,
        manifest,
        key,
        installed_kind,
        edition,
        minimum_build,
        Some(public),
    )
}

const MAX_PACKAGE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_TOTAL: u64 = 8 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_FILES: usize = 20000;

fn prepare_checked(
    parent: &Path,
    package: &Path,
    manifest: &hbb_common::UpdateManifest,
    key: &str,
    kind: &str,
    edition: &str,
    minimum: u64,
    public: Option<&[u8]>,
) -> Result<SealedAssets, AssetError> {
    check_identity(kind, edition, key)?;
    let target = {
        #[cfg(test)]
        if let Some(public) = public {
            package_gate::verify_with_key(manifest, key, minimum, "fixture-key", public)
        } else {
            package_gate::verify(manifest, key, minimum)
        }
        #[cfg(not(test))]
        {
            let _ = public;
            package_gate::verify(manifest, key, minimum)
        }
    }
    .map_err(|_| AssetError::Signature)?;
    if target.size == 0 || target.size > MAX_PACKAGE {
        return Err(AssetError::Limit);
    }
    package_gate::cache_name(&target.primary, "exe").map_err(|_| AssetError::Package)?;
    let _source_lock =
        package_gate::lock_verified_file(package, target).map_err(|_| AssetError::Package)?;
    supported_packer(package)?;
    workspace::verify(parent).map_err(|_| AssetError::Workspace)?;
    let root = parent.join(format!("prepare-{}", uuid::Uuid::new_v4().simple()));
    workspace::create(&root).map_err(|_| AssetError::Workspace)?;
    let copied = root.join("package.exe");
    let mut source = std::fs::File::open(package).map_err(|_| AssetError::Package)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&copied)
        .map_err(|_| AssetError::Occupied)?;
    let count = std::io::copy(&mut (&mut source).take(target.size + 1), &mut output)
        .map_err(|_| AssetError::Package)?;
    output.sync_all().map_err(|_| AssetError::Workspace)?;
    drop(output);
    if count != target.size {
        return Err(AssetError::Package);
    }
    workspace::verify_file(&copied).map_err(|_| AssetError::Workspace)?;
    let _copy_lock =
        package_gate::lock_verified_file(&copied, target).map_err(|_| AssetError::Package)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let inventory = run(
        &copied,
        &[
            "--management-agent-strict-assets".into(),
            "--management-agent-inventory".into(),
        ],
        deadline,
    )?;
    let files = parse_inventory(&inventory)?;
    save(&root.join("expected-inventory.txt"), &inventory)?;
    let payload = root.join("payload");
    run(
        &copied,
        &[
            "--management-agent-strict-assets".into(),
            "--management-agent-extract-path".into(),
            payload.as_os_str().to_owned(),
        ],
        deadline,
    )?;
    verify_payload(&payload, &files)?;
    if Instant::now() >= deadline {
        return Err(AssetError::Timeout);
    }
    let reference = package_gate::package_reference(
        &serde_json::to_value(manifest).map_err(|_| AssetError::Inventory)?,
        key,
    )
    .map_err(|_| AssetError::Inventory)?;
    let record = serde_json::json!({"schema":1,"target_key":key,"package_sha256":target.sha256,"package_size":target.size,
        "approved_reference":reference,"inventory_sha256":hex::encode(Sha256::digest(&inventory)),"files":files});
    save(
        &root.join("approved-manifest.json"),
        &serde_json::to_vec(manifest).map_err(|_| AssetError::Inventory)?,
    )?;
    save(
        &root.join("sealed.json"),
        &serde_json::to_vec(&record).map_err(|_| AssetError::Inventory)?,
    )?;
    Ok(SealedAssets { root, files })
}

fn save(path: &Path, bytes: &[u8]) -> Result<(), AssetError> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| AssetError::Occupied)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| AssetError::Workspace)?;
    drop(file);
    workspace::verify_file(path).map_err(|_| AssetError::Workspace)
}

fn supported_packer(path: &Path) -> Result<(), AssetError> {
    let mut file = std::fs::File::open(path).map_err(|_| AssetError::Package)?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).map_err(|_| AssetError::Package)?;
    if &dos[..2] != b"MZ" {
        return Err(AssetError::Package);
    }
    let offset = u32::from_le_bytes([dos[60], dos[61], dos[62], dos[63]]) as u64;
    if !(64..=16 * 1024 * 1024).contains(&offset) {
        return Err(AssetError::Package);
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| AssetError::Package)?;
    let mut pe = [0u8; 26];
    file.read_exact(&mut pe).map_err(|_| AssetError::Package)?;
    if &pe[..6] != b"PE\0\0\x64\x86" || &pe[24..26] != b"\x0b\x02" {
        return Err(AssetError::Package);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| AssetError::Package)?;
    let marker = b"RUSTDESK_STRICT_ASSETS_V1";
    let mut carry = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| AssetError::Package)?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&buffer[..n]);
        if carry.windows(marker.len()).any(|v| v == marker) {
            return Ok(());
        }
        let keep = marker.len() - 1;
        carry.drain(..carry.len().saturating_sub(keep));
    }
    Err(AssetError::Package)
}

fn run(
    package: &Path,
    args: &[std::ffi::OsString],
    deadline: Instant,
) -> Result<Vec<u8>, AssetError> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    if Instant::now() >= deadline {
        return Err(AssetError::Timeout);
    }
    let mut child = Command::new(package)
        .args(args)
        .current_dir(package.parent().ok_or(AssetError::Workspace)?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|_| AssetError::Extraction)?;
    let stdout = child.stdout.take().ok_or(AssetError::Extraction)?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(AssetError::Extraction)
                }
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let killed = child.kill();
                let waited = child.wait();
                break if killed.is_ok() && waited.is_ok() {
                    Err(AssetError::Timeout)
                } else {
                    Err(AssetError::Extraction)
                };
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| AssetError::Extraction)?
        .map_err(|_| AssetError::Extraction)?;
    result?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(AssetError::Limit);
    }
    Ok(bytes)
}

fn safe_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value.is_ascii()
        && !value.contains('\\')
        && !value
            .chars()
            .any(|c| c.is_control() || ":*?\"<>|".contains(c))
        && value.split('/').all(|p| {
            let stem = p.split('.').next().unwrap_or("").to_ascii_uppercase();
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with(['.', ' '])
                && !["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                && !((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        })
}

fn parse_inventory(bytes: &[u8]) -> Result<Vec<(String, u64, String)>, AssetError> {
    let text = std::str::from_utf8(bytes).map_err(|_| AssetError::Inventory)?;
    let mut lines = text.lines();
    if lines.next() != Some("rustdesk-strict-assets-v1") {
        return Err(AssetError::Inventory);
    }
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut names = BTreeSet::new();
    let mut ended = false;
    for line in lines.by_ref() {
        if line == "end" {
            ended = true;
            break;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 3 || !safe_path(fields[0]) {
            return Err(AssetError::Inventory);
        }
        let size = fields[1]
            .parse::<u64>()
            .map_err(|_| AssetError::Inventory)?;
        if size == 0 || size > MAX_FILE || files.len() >= MAX_FILES {
            return Err(AssetError::Limit);
        }
        total = total.checked_add(size).ok_or(AssetError::Limit)?;
        if total > MAX_TOTAL {
            return Err(AssetError::Limit);
        }
        if fields[2].len() != 64
            || !fields[2]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !names.insert(fields[0].to_ascii_lowercase())
        {
            return Err(AssetError::Inventory);
        }
        files.push((fields[0].to_owned(), size, fields[2].to_owned()));
    }
    if !ended || lines.next().is_some() {
        return Err(AssetError::Inventory);
    }
    for name in [
        "rustdesk.exe",
        "librustdesk.dll",
        "rustdesk-management-agent.exe",
        "flutter_windows.dll",
        "data/icudtl.dat",
    ] {
        if !names.contains(name) {
            return Err(AssetError::Inventory);
        }
    }
    if !names.iter().any(|p| p.starts_with("data/flutter_assets/"))
        || !names
            .iter()
            .any(|p| !p.contains('/') && p.ends_with("_plugin.dll"))
    {
        return Err(AssetError::Inventory);
    }
    for name in &names {
        let mut parent = Path::new(name).parent();
        while let Some(p) = parent {
            if names.contains(p.to_str().ok_or(AssetError::Inventory)?) {
                return Err(AssetError::Inventory);
            }
            parent = p.parent();
        }
    }
    Ok(files)
}

fn verify_payload(root: &Path, files: &[(String, u64, String)]) -> Result<(), AssetError> {
    use std::os::windows::fs::MetadataExt;
    let mut directories = BTreeSet::new();
    for (name, _, _) in files {
        let mut parent = Path::new(name).parent();
        while let Some(p) = parent {
            if !p.as_os_str().is_empty() {
                directories.insert(p.to_str().ok_or(AssetError::Integrity)?.replace('\\', "/"));
            }
            parent = p.parent();
        }
    }
    let mut pending = vec![root.to_path_buf()];
    let mut seen = BTreeSet::new();
    let mut entries = 0usize;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|_| AssetError::Integrity)? {
            let path = entry.map_err(|_| AssetError::Integrity)?.path();
            let m = std::fs::symlink_metadata(&path).map_err(|_| AssetError::Integrity)?;
            entries += 1;
            if entries > 40000 {
                return Err(AssetError::Limit);
            }
            if m.file_attributes() & 0x400 != 0 {
                return Err(AssetError::Integrity);
            }
            if m.is_dir() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| AssetError::Integrity)?
                    .to_str()
                    .ok_or(AssetError::Integrity)?
                    .replace('\\', "/");
                if !directories.remove(&relative) {
                    return Err(AssetError::Integrity);
                }
                pending.push(path);
                continue;
            }
            if !m.is_file() {
                return Err(AssetError::Integrity);
            }
            workspace::verify_file(&path).map_err(|_| AssetError::Integrity)?;
            let relative = path
                .strip_prefix(root)
                .map_err(|_| AssetError::Integrity)?
                .to_str()
                .ok_or(AssetError::Integrity)?
                .replace('\\', "/");
            let (_, size, hash) = files
                .iter()
                .find(|f| f.0 == relative)
                .ok_or(AssetError::Integrity)?;
            if !seen.insert(relative) || m.len() != *size {
                return Err(AssetError::Integrity);
            }
            let target = hbb_common::UpdateTarget {
                size: *size,
                sha256: hash.clone(),
                ..Default::default()
            };
            let _lock = package_gate::lock_verified_file(&path, &target)
                .map_err(|_| AssetError::Integrity)?;
        }
    }
    if seen.len() != files.len() || !directories.is_empty() {
        return Err(AssetError::Integrity);
    }
    Ok(())
}

#[cfg(test)]
pub fn validate_inventory_fixture(bytes: &[u8]) -> Result<(), AssetError> {
    parse_inventory(bytes).map(|_| ())
}

#[cfg(test)]
pub fn verify_output_fixture(sealed: &SealedAssets) -> Result<(), AssetError> {
    verify_payload(&sealed.root.join("payload"), &sealed.files)
}
