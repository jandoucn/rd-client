use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Component, Path},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryBackup {
    pub schema: u32,
    pub directories: Vec<String>,
    pub files: Vec<BackupFile>,
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':'])
        && !value.chars().any(char::is_control)
        && !value.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' '])
        })
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn local(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("backup path not absolute".into());
    }
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|e| format!("backup path unavailable: {e}"))?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("backup reparse path refused".into());
            }
        }
        if metadata.file_type().is_symlink() {
            return Err("backup symlink refused".into());
        }
    }
    Ok(())
}

fn inventory(root: &Path) -> Result<DirectoryBackup, String> {
    local(root)?;
    if !root.is_dir() {
        return Err("backup source not directory".into());
    }
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0u64;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            local(&path)?;
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("backup path encoding invalid")?
                .replace('\\', "/");
            if !safe_relative(&relative) {
                return Err("backup relative path unsafe".into());
            }
            let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
            if metadata.is_dir() {
                directories.push(relative);
                pending.push(path);
            } else if metadata.is_file() {
                total = total
                    .checked_add(metadata.len())
                    .ok_or("backup size overflow")?;
                if total > 8 * 1024 * 1024 * 1024 || files.len() >= 20000 {
                    return Err("backup capacity exceeded".into());
                }
                let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                let mut hash = Sha256::new();
                let mut buffer = [0u8; 65536];
                let mut count = 0u64;
                loop {
                    let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
                    if read == 0 {
                        break;
                    }
                    count = count.saturating_add(read as u64);
                    if count > metadata.len() {
                        return Err("backup source size changed".into());
                    }
                    hash.update(&buffer[..read]);
                }
                if count != metadata.len() {
                    return Err("backup source size changed".into());
                }
                files.push(BackupFile {
                    path: relative,
                    size: count,
                    sha256: hex::encode(hash.finalize()),
                });
            } else {
                return Err("backup special object refused".into());
            }
            if directories.len() + files.len() > 20000 {
                return Err("backup entry capacity exceeded".into());
            }
        }
    }
    directories.sort();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let snapshot = DirectoryBackup {
        schema: 1,
        directories,
        files,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

impl DirectoryBackup {
    pub fn matches_directory(&self, root: &Path) -> Result<bool, String> {
        self.validate()?;
        Ok(inventory(root)? == *self)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.directories.len() + self.files.len() > 20000 {
            return Err("backup schema/capacity invalid".into());
        }
        let mut seen = BTreeSet::new();
        for path in self
            .directories
            .iter()
            .map(String::as_str)
            .chain(self.files.iter().map(|file| file.path.as_str()))
        {
            if !safe_relative(path) || !seen.insert(path.to_ascii_lowercase()) {
                return Err("backup manifest path unsafe or duplicate".into());
            }
        }
        for file in &self.files {
            if file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("backup hash invalid".into());
            }
        }
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("backup manifest too large".into());
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| "backup manifest malformed".to_owned())?;
        value.validate()?;
        Ok(value)
    }

    pub fn verify(&self, backup: &Path) -> Result<(), String> {
        self.validate()?;
        let actual = inventory(&backup.join("files"))?;
        if actual != *self {
            return Err("backup file set/size/hash mismatch".into());
        }
        Ok(())
    }

    fn copy_to(&self, source: &Path, destination: &Path) -> Result<(), String> {
        std::fs::create_dir(destination).map_err(|e| e.to_string())?;
        for relative in &self.directories {
            std::fs::create_dir_all(destination.join(relative)).map_err(|e| e.to_string())?;
        }
        for file in &self.files {
            let target = destination.join(&file.path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::copy(source.join(&file.path), &target).map_err(|e| e.to_string())?;
            std::fs::OpenOptions::new()
                .write(true)
                .open(&target)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
        }
        if inventory(destination)? != *self {
            return Err("backup copied file set/hash mismatch".into());
        }
        Ok(())
    }

    pub fn restore(&self, backup: &Path, destination: &Path) -> Result<(), String> {
        self.verify(backup)?;
        local(destination)?;
        if !destination.is_dir() {
            return Err("backup restore destination not directory".into());
        }
        let parent = destination
            .parent()
            .ok_or("backup restore parent unavailable")?;
        let name = destination
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or("backup destination name invalid")?;
        let staging = parent.join(format!("{name}.rollback-staging"));
        let displaced = parent.join(format!("{name}.rollback-displaced"));
        if staging.exists() || displaced.exists() {
            return Err("backup restore staging occupied; reconciliation required".into());
        }
        self.copy_to(&backup.join("files"), &staging)?;
        std::fs::rename(destination, &displaced)
            .map_err(|e| format!("backup restore old directory move: {e}"))?;
        if let Err(error) = std::fs::rename(&staging, destination) {
            if let Err(restore) = std::fs::rename(&displaced, destination) {
                return Err(format!("{error}; restore old directory failed: {restore}"));
            }
            return Err(format!("backup restore publication failed: {error}"));
        }
        if inventory(destination)? != *self {
            return Err("backup restored directory verification failed".into());
        }
        std::fs::remove_dir_all(&displaced)
            .map_err(|e| format!("backup displaced directory cleanup: {e}"))?;
        Ok(())
    }
}

pub fn capture(source: &Path, backup: &Path) -> Result<DirectoryBackup, String> {
    local(source)?;
    let source = std::fs::canonicalize(source).map_err(|e| e.to_string())?;
    local(&source)?;
    if !backup.is_absolute() || backup.exists() {
        return Err("backup destination exists or is relative".into());
    }
    let parent = backup.parent().ok_or("backup parent missing")?;
    local(parent)?;
    let canonical_parent = std::fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if canonical_parent.starts_with(&source) {
        return Err("backup destination inside source refused".into());
    }
    let snapshot = inventory(&source)?;
    std::fs::create_dir(backup).map_err(|e| e.to_string())?;
    snapshot.copy_to(&source, &backup.join("files"))?;
    if inventory(&source)? != snapshot {
        return Err("backup source changed during capture".into());
    }
    let bytes = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(backup.join("manifest.json"))
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    snapshot.verify(backup)?;
    Ok(snapshot)
}
