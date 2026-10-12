use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{io::Read, path::{Component, Path}};

#[derive(Debug, Serialize)]
pub struct Report {
    pub status: String,
    pub size: Option<u64>,
    pub sha256: Option<String>,
}

pub fn inspect(path: &Path, expected: Option<&str>, size: Option<u64>) -> Report {
    let mut report = Report { status: "unverified".into(), size: None, sha256: None };
    if (expected.is_none() && size.is_some()) || expected.is_some_and(|hash| {
        hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        report.status = "invalid_reference".into();
        return report;
    }
    match hash_file(path) {
        Ok((actual_size, hash)) => {
            report.size = Some(actual_size);
            report.status = if size.is_some_and(|expected| actual_size != expected) {
                "size_mismatch"
            } else if expected.is_some_and(|expected| hash != expected) {
                "hash_mismatch"
            } else if expected.is_some() {
                "verified"
            } else {
                "unverified"
            }.into();
            report.sha256 = Some(hash);
        }
        Err(status) => report.status = status.into(),
    }
    report
}

fn hash_file(path: &Path) -> Result<(u64, String), &'static str> {
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err("unsafe_path");
    }
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_))) {
        return Err("unsafe_path");
    }
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound { "missing" } else { "read_failed" }
        })?;
        if metadata.file_type().is_symlink() { return Err("unsafe_path"); }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 { return Err("unsafe_path"); }
        }
    }
    if !path.is_file() { return Err("not_regular_file"); }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let mut file = options.open(path).map_err(|_| "read_failed")?;
    let metadata = file.metadata().map_err(|_| "read_failed")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 { return Err("unsafe_path"); }
    }
    if !metadata.is_file() { return Err("not_regular_file"); }
    if metadata.len() > 512 * 1024 * 1024 { return Err("size_limit"); }
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "read_failed")?;
        if count == 0 { break; }
        total += count as u64;
        if total > metadata.len() { return Err("changed_during_read"); }
        hash.update(&buffer[..count]);
    }
    if total != metadata.len() { return Err("changed_during_read"); }
    Ok((total, hex::encode(hash.finalize())))
}
