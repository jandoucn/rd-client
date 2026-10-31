//! Retention for finished transaction directories.
//!
//! Every installation, update and local repair leaves a directory under
//! `ManagementAgent\transactions`, each holding a full copy of its MSI. They are evidence for a
//! while and afterwards only take disk space and slots: first-install approval delivery refuses to
//! run once 64 entries exist, which freezes rescue bundle rotation for good.
//!
//! Only directories whose `transaction.json` is committed or rolled back (and whose rescue
//! rotation, if any, was published) are eligible. Unfinished, unreadable and rollback-failed
//! transactions are kept because they are what reconciliation and diagnosis read. The newest
//! finished ones and the transaction a rescue bundle was built from are always kept. Local repair
//! (R4) attempts are kept too: their presence is the per-bundle attempt count, and a rolled-back
//! attempt is what stops R4 from retrying a package that already failed.
use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const KEEP_NEWEST: usize = 8;
const RETENTION: Duration = Duration::from_secs(14 * 24 * 3600);
const SCAN_LIMIT: usize = 256;
const QUARANTINE_PREFIX: &str = "tx-";
const LOCAL_REPAIR_PLAN: &str = "local-repair-plan.json";

/// Same predicate first-install approval delivery uses to call a transaction finished.
pub fn finished(transaction: &[u8], rotation: Option<&[u8]>) -> bool {
    let phase = |bytes: &[u8]| {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|value| value["phase"].as_str().map(str::to_owned))
    };
    matches!(
        phase(transaction).as_deref(),
        Some("committed" | "rolled_back")
    ) && rotation.map_or(true, |bytes| phase(bytes).as_deref() == Some("published"))
}

/// Pure selection: newest first, the first `KEEP_NEWEST` and every referenced one stay; the rest
/// go once older than `RETENTION`. A timestamp in the future counts as new.
pub fn expired(
    finished: &[(String, SystemTime)],
    referenced: &HashSet<String>,
    now: SystemTime,
) -> Vec<String> {
    let mut ordered = finished.to_vec();
    ordered.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ordered
        .into_iter()
        .skip(KEEP_NEWEST)
        .filter(|(id, time)| {
            !referenced.contains(id)
                && now
                    .duration_since(*time)
                    .is_ok_and(|age| age > RETENTION)
        })
        .map(|(id, _)| id)
        .collect()
}

fn valid_id(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Transactions a rescue bundle still points at as its source.
fn referenced(agent: &Path) -> HashSet<String> {
    ["current", "previous", "staging"]
        .into_iter()
        .filter_map(|slot| read_limited(&agent.join("rescue").join(slot).join("bundle.json"), 65536))
        .filter_map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter_map(|bundle| bundle["source_transaction"].as_str().map(str::to_owned))
        .filter(|id| !id.is_empty())
        .collect()
}

fn plain_directory(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && metadata.file_attributes() & 0x400 == 0)
}

fn finished_transactions(root: &Path) -> Result<Vec<(String, SystemTime)>, String> {
    let mut found = Vec::new();
    let entries =
        std::fs::read_dir(root).map_err(|_| "transaction retention directory unreadable")?;
    for entry in entries.take(SCAN_LIMIT).flatten() {
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let path = entry.path();
        if !valid_id(&id) || !plain_directory(&path) {
            continue;
        }
        match std::fs::symlink_metadata(path.join(LOCAL_REPAIR_PLAN)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => continue,
        }
        let Some(transaction) = read_limited(&path.join("transaction.json"), 65536) else {
            continue;
        };
        let rotation = match std::fs::symlink_metadata(path.join("rotation.json")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => continue,
            Ok(_) => match read_limited(&path.join("rotation.json"), 4096) {
                Some(bytes) => Some(bytes),
                None => continue,
            },
        };
        if !finished(&transaction, rotation.as_deref()) {
            continue;
        }
        let Ok(modified) = std::fs::metadata(path.join("transaction.json"))
            .and_then(|metadata| metadata.modified())
        else {
            continue;
        };
        found.push((id, modified));
    }
    Ok(found)
}

fn quarantine(agent: &Path) -> Result<PathBuf, String> {
    let root = agent.join("quarantine");
    match std::fs::symlink_metadata(&root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            super::workspace::create(&root)?
        }
        Err(_) => return Err("transaction retention quarantine unavailable".into()),
        Ok(_) => super::workspace::verify(&root)?,
    }
    Ok(root)
}

/// Removes expired finished transactions and returns how many went. Each directory is first
/// renamed aside, which Windows refuses while any file inside is open, so a transaction in use is
/// never half deleted; only the renamed copy is then removed.
pub fn prune(agent: &Path, now: SystemTime) -> Result<usize, String> {
    let removed = {
        let Some(_guard) = super::maintenance_orphan::install_guard_patiently()? else {
            return Err("transaction retention installation busy".into());
        };
        move_expired(agent, now, quarantine)?
    };
    // Deleting large MSI copies can take a while; nothing else needs the guard for that.
    discard_moved(agent);
    Ok(removed)
}

fn move_expired(
    agent: &Path,
    now: SystemTime,
    quarantine: impl Fn(&Path) -> Result<PathBuf, String>,
) -> Result<usize, String> {
    let root = agent.join("transactions");
    if !plain_directory(&root) {
        return Ok(0);
    }
    let expired = expired(&finished_transactions(&root)?, &referenced(agent), now);
    let stamp = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default();
    let mut removed = 0;
    if !expired.is_empty() {
        let aside = quarantine(agent)?;
        for id in expired {
            let moved = aside.join(format!("{QUARANTINE_PREFIX}{id}-{stamp}"));
            if std::fs::rename(root.join(&id), &moved).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Removes moved-aside transactions, including ones an earlier run could not finish.
fn discard_moved(agent: &Path) {
    if let Ok(entries) = std::fs::read_dir(agent.join("quarantine")) {
        for entry in entries.take(SCAN_LIMIT).flatten() {
            let ours = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(QUARANTINE_PREFIX));
            if ours && plain_directory(&entry.path()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(days: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(days * 24 * 3600)
    }

    fn named(ids: &[(&str, u64)]) -> Vec<(String, SystemTime)> {
        ids.iter().map(|(id, day)| (id.to_string(), at(*day))).collect()
    }

    #[test]
    fn only_committed_or_rolled_back_with_published_rotation_are_finished() {
        let committed = br#"{"phase":"committed"}"#;
        assert!(finished(committed, None));
        assert!(finished(br#"{"phase":"rolled_back"}"#, None));
        assert!(finished(committed, Some(br#"{"phase":"published"}"#)));
        assert!(!finished(committed, Some(br#"{"phase":"staged"}"#)));
        for phase in ["applying", "rollback_failed", "rolling_back", "validating"] {
            let bytes = format!(r#"{{"phase":"{phase}"}}"#);
            assert!(!finished(bytes.as_bytes(), None), "{phase}");
        }
        assert!(!finished(b"not json", None));
    }

    #[test]
    fn the_newest_eight_stay_whatever_their_age() {
        let all = named(&[
            ("a", 1), ("b", 2), ("c", 3), ("d", 4), ("e", 5),
            ("f", 6), ("g", 7), ("h", 8), ("i", 9), ("j", 10),
        ]);
        let mut gone = expired(&all, &HashSet::new(), at(100));
        gone.sort();
        assert_eq!(gone, ["a", "b"]);
    }

    #[test]
    fn recent_and_referenced_transactions_stay() {
        let all = named(&[
            ("new1", 95), ("new2", 95), ("new3", 95), ("new4", 95),
            ("new5", 95), ("new6", 95), ("new7", 95), ("new8", 95),
            ("recent", 90), ("bundle", 1), ("old", 1),
        ]);
        let referenced = HashSet::from(["bundle".to_owned()]);
        assert_eq!(expired(&all, &referenced, at(100)), ["old"]);
    }

    #[test]
    fn a_clock_behind_the_files_removes_nothing() {
        let all = named(&[
            ("a", 50), ("b", 50), ("c", 50), ("d", 50), ("e", 50),
            ("f", 50), ("g", 50), ("h", 50), ("i", 50),
        ]);
        assert!(expired(&all, &HashSet::new(), at(10)).is_empty());
    }

    #[test]
    fn directory_names_are_restricted() {
        assert!(valid_id("e1b6d84a0814548aa7ebf9cdcfcbca3a"));
        assert!(valid_id("repair-1_2"));
        for name in ["", "..", "a b", "a\\b", &"x".repeat(65)] {
            assert!(!valid_id(name), "{name}");
        }
    }

    fn transaction(root: &Path, id: &str, phase: &str, days_ago: u64, extra: &[&str]) {
        let dir = root.join("transactions").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("transaction.json");
        std::fs::write(&file, format!(r#"{{"phase":"{phase}"}}"#)).unwrap();
        for name in extra {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let when = SystemTime::now() - Duration::from_secs(days_ago * 24 * 3600);
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    // 真实目录：只回收过期、已完成、未被引用、不是 R4 尝试的事务；被占用的目录改名失败就留着。
    #[test]
    fn prune_moves_only_expired_finished_transactions() {
        let agent = std::env::temp_dir().join(format!(
            "rd-tx-prune-{}",
            uuid::Uuid::new_v4().simple()
        ));
        for n in 0..8 {
            transaction(&agent, &format!("new{n}"), "committed", 0, &[]);
        }
        transaction(&agent, "oldcommitted", "committed", 30, &["pkg.msi"]);
        transaction(&agent, "oldrolledback", "rolled_back", 30, &[]);
        transaction(&agent, "oldfailed", "rollback_failed", 30, &[]);
        transaction(&agent, "oldapplying", "applying", 30, &[]);
        transaction(&agent, "oldrepair", "rolled_back", 30, &[LOCAL_REPAIR_PLAN]);
        transaction(&agent, "oldsource", "committed", 30, &[]);
        transaction(&agent, "oldbusy", "committed", 30, &["held.log"]);
        let rescue = agent.join("rescue").join("current");
        std::fs::create_dir_all(&rescue).unwrap();
        std::fs::write(rescue.join("bundle.json"), r#"{"source_transaction":"oldsource"}"#).unwrap();
        // Like a running helper: the file is open without delete sharing.
        let held = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(agent.join("transactions/oldbusy/held.log"))
                .unwrap()
        };
        // A stale moved copy from an earlier run is finished off as well.
        std::fs::create_dir_all(agent.join("quarantine/tx-leftover-1")).unwrap();

        // The install guard is a machine-wide mutex other tests may hold; test the two halves.
        let plain = |agent: &Path| {
            let root = agent.join("quarantine");
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            Ok(root)
        };
        assert_eq!(move_expired(&agent, SystemTime::now(), plain), Ok(2));
        discard_moved(&agent);
        let left = |id: &str| agent.join("transactions").join(id).exists();
        assert!(!left("oldcommitted") && !left("oldrolledback"));
        for id in ["oldfailed", "oldapplying", "oldrepair", "oldsource", "oldbusy", "new0", "new7"] {
            assert!(left(id), "{id}");
        }
        let quarantined = std::fs::read_dir(agent.join("quarantine")).unwrap().count();
        assert_eq!(quarantined, 0, "moved copies are deleted");
        drop(held);
        std::fs::remove_dir_all(&agent).unwrap();
    }

    #[test]
    fn rescue_bundle_sources_are_referenced() {
        let agent = std::env::temp_dir().join(format!(
            "rd-tx-retention-{}",
            uuid::Uuid::new_v4().simple()
        ));
        for (slot, body) in [
            ("current", r#"{"source_transaction":"aaa"}"#),
            ("previous", r#"{"source_transaction":null}"#),
            ("staging", r#"{"source_transaction":""}"#),
        ] {
            let dir = agent.join("rescue").join(slot);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("bundle.json"), body).unwrap();
        }
        assert_eq!(referenced(&agent), HashSet::from(["aaa".to_owned()]));
        std::fs::remove_dir_all(&agent).unwrap();
    }
}
