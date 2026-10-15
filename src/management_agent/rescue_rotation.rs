use super::super::{state_store, workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u32,
    transaction: String,
    old: String,
    new: String,
    previous: Option<String>,
    phase: String,
}

pub(super) fn digest(path: &Path) -> Result<Option<String>, String> {
    use std::io::Read;
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("rotation object unreadable".into()),
        Ok(_) => {}
    }
    workspace::verify(path)?;
    workspace::verify_file(&path.join("bundle.json"))?;
    let mut bytes = Vec::new();
    std::fs::File::open(path.join("bundle.json"))
        .map_err(|e| e.to_string())?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65536 {
        return Err("rotation metadata too large".into());
    }
    Ok(Some(hex::encode(Sha256::digest(bytes))))
}
fn move_directory(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let from = from
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 8) } == 0 {
        return Err(format!(
            "rotation rename refused: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}
fn persist(path: &Path, record: &Receipt, preimage: Option<&[u8]>) -> Result<Vec<u8>, String> {
    match (std::fs::read(path), preimage) {
        (Err(e), None) if e.kind() == std::io::ErrorKind::NotFound => {}
        (Ok(bytes), Some(old)) if bytes == old => {}
        _ => return Err("rotation receipt CAS conflict".into()),
    }
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    state_store::publish(path, &bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}
fn verify_archive(
    path: &Path,
    expected: Option<&str>,
    validate: &impl Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if digest(path)?.as_deref() != expected {
        return Err("rotation retired archive conflict".into());
    }
    if expected.is_some() {
        validate(path)?;
    }
    Ok(())
}
pub(super) fn binding(
    transaction: &Path,
) -> Result<Option<(String, String, Option<String>)>, String> {
    let path = transaction.join("rotation.json");
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("rotation receipt unreadable".into()),
        Ok(_) => {}
    }
    workspace::verify_file(&path)?;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Err("rotation receipt too large".into());
    }
    let r: Receipt = serde_json::from_slice(&bytes).map_err(|_| "rotation receipt malformed")?;
    if r.schema != 1
        || r.transaction
            != transaction
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("rotation transaction encoding")?
    {
        return Err("rotation receipt binding mismatch".into());
    }
    Ok(Some((r.old, r.new, r.previous)))
}

pub(super) fn rotate(
    root: &Path,
    transaction: &Path,
    old: &str,
    new: &str,
    previous: Option<&str>,
    validate: impl Fn(&Path) -> Result<(), String>,
    fault: Option<&str>,
) -> Result<(), String> {
    use std::os::windows::fs::OpenOptionsExt;
    workspace::verify(root)?;
    workspace::verify(transaction)?;
    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(transaction.join("rotation.lock"))
        .map_err(|e| e.to_string())?;
    let id = transaction
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("rotation transaction invalid")?
        .to_owned();
    let receipt_path = transaction.join("rotation.json");
    for value in [Some(old), Some(new), previous].into_iter().flatten() {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("rotation digest invalid".into());
        }
    }
    let current = root.join("current");
    let stage = root.join("staging");
    let prior = root.join("previous");
    let retired = transaction.join("retired-rescue");
    let mut record = Receipt {
        schema: 1,
        transaction: id,
        old: old.into(),
        new: new.into(),
        previous: previous.map(str::to_owned),
        phase: "prepared".into(),
    };
    let mut bytes = match std::fs::read(&receipt_path) {
        Ok(bytes) => {
            workspace::verify_file(&receipt_path)?;
            let stored: Receipt =
                serde_json::from_slice(&bytes).map_err(|_| "rotation receipt malformed")?;
            if stored.schema != 1
                || stored.transaction != record.transaction
                || stored.old != old
                || stored.new != new
                || stored.previous != record.previous
                || !matches!(
                    stored.phase.as_str(),
                    "prepared" | "previous_retired" | "old_moved" | "published"
                )
            {
                return Err("rotation receipt binding conflict".into());
            }
            record = stored;
            bytes
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if digest(&current)?.as_deref() != Some(old)
                || digest(&stage)?.as_deref() != Some(new)
                || digest(&prior)?.as_deref() != previous
                || digest(&retired)?.is_some()
            {
                return Err("rotation initial preimage conflict".into());
            }
            validate(&current)?;
            validate(&stage)?;
            if previous.is_some() {
                validate(&prior)?;
            }
            persist(&receipt_path, &record, None)?
        }
        Err(e) => return Err(e.to_string()),
    };
    if record.phase != "prepared" {
        verify_archive(&retired, previous, &validate)?;
    }
    if record.phase == "published" {
        if digest(&current)?.as_deref() != Some(new)
            || digest(&prior)?.as_deref() != Some(old)
            || digest(&stage)?.is_some()
        {
            return Err("rotation published image conflict".into());
        }
        validate(&current)?;
        validate(&prior)?;
        return Ok(());
    }
    // Reconcile a rename that completed before its receipt update using exact pre/post digests.
    if digest(&current)?.as_deref() == Some(new)
        && digest(&stage)?.is_none()
        && digest(&prior)?.as_deref() == Some(old)
    {
        verify_archive(&retired, previous, &validate)?;
        validate(&current)?;
        validate(&prior)?;
        record.phase = "published".into();
        persist(&receipt_path, &record, Some(&bytes))?;
        return Ok(());
    }
    if digest(&stage)?.as_deref() != Some(new) {
        return Err("rotation staged image conflict; old backup retained".into());
    }
    validate(&stage)?;
    if record.phase == "prepared" {
        if let Some(previous) = previous {
            match (digest(&prior)?, digest(&retired)?) {
                (Some(p), None) if p == previous => {
                    validate(&prior)?;
                    move_directory(&prior, &retired)?;
                }
                (None, Some(p)) if p == previous => validate(&retired)?,
                _ => return Err("rotation previous preimage conflict".into()),
            }
        } else if digest(&prior)?.is_some() {
            return Err("rotation unexpected previous conflict".into());
        }
        verify_archive(&retired, previous, &validate)?;
        #[cfg(test)]
        if fault == Some("after_previous_move") {
            return Err("injected rotation interruption".into());
        }
        record.phase = "previous_retired".into();
        bytes = persist(&receipt_path, &record, Some(&bytes))?;
    }
    if record.phase == "previous_retired" {
        verify_archive(&retired, previous, &validate)?;
        match (digest(&current)?, digest(&prior)?) {
            (Some(c), None) if c == old => {
                validate(&current)?;
                move_directory(&current, &prior)?;
            }
            (None, Some(p)) if p == old => validate(&prior)?,
            _ => return Err("rotation current preimage conflict".into()),
        }
        #[cfg(test)]
        if fault == Some("after_current_move") {
            return Err("injected rotation interruption".into());
        }
        record.phase = "old_moved".into();
        bytes = persist(&receipt_path, &record, Some(&bytes))?;
    }
    let _ = fault;
    verify_archive(&retired, previous, &validate)?;
    if digest(&current)?.is_some() || digest(&prior)?.as_deref() != Some(old) {
        return Err("rotation publish preimage conflict".into());
    }
    validate(&prior)?;
    validate(&stage)?;
    move_directory(&stage, &current)?;
    #[cfg(test)]
    if fault == Some("after_publish_move") {
        return Err("injected rotation interruption".into());
    }
    validate(&current)?;
    verify_archive(&retired, previous, &validate)?;
    record.phase = "published".into();
    persist(&receipt_path, &record, Some(&bytes))?;
    Ok(())
}
