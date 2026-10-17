#![cfg(windows)]

// Reuse the production module graph without running the asset fixture's MSI tests.
include!("management_agent_rescue_projection/modules.rs");

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const INSTALLATION: &str = "cccccccccccccccccccccccccccccccc";
const KEY: &str = "windows-x86_64-msi-standard";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        assert_eq!(std::env::var("COMPUTERNAME").unwrap(), "RDLAB-WIN10");
        let root = Path::new(r"C:\RustDeskAgentLab")
            .join(format!("rescue-projection-{}", uuid::Uuid::new_v4()));
        workspace::create(&root).unwrap();
        for name in ["rescue", "rescue-status", "first-install-status", "state"] {
            workspace::create(&root.join(name)).unwrap();
        }
        std::fs::write(root.join("state/identity.toml"), b"original identity bytes").unwrap();
        std::fs::write(root.join("state/agent.json"), b"original state bytes").unwrap();
        Self(root)
    }
    fn first(&self, bytes: &[u8]) {
        std::fs::write(self.0.join("first-install-status/last-check.json"), bytes).unwrap();
    }
    fn bundle(&self) -> String {
        let root = self.0.join("rescue/current");
        workspace::create(&root).unwrap();
        let bundle = br#"{"source_reference":"fixture-observation"}"#;
        let manifest = b"fixture bytes; not an approval";
        std::fs::write(root.join("bundle.json"), bundle).unwrap();
        std::fs::write(root.join("approved-manifest.json"), manifest).unwrap();
        let mut hash = Sha256::new();
        hash.update((bundle.len() as u64).to_le_bytes());
        hash.update(bundle);
        hash.update(manifest);
        hex::encode(hash.finalize())
    }
    fn cache(&self, digest: &str, checked_at: u64, installation: &str, key: &str, authority: &str) {
        let record = json!({"schema":1,"installation":installation,"target_key":key,
            "checked_at":checked_at,"source_digest":digest,"source_reference":"fixture-cache",
            "authority":authority,"state":"degraded","error_code":null,"assets":{}});
        std::fs::write(
            self.0.join("rescue-status/last-check.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }
    fn snapshot(&self) -> Value {
        let before = files(&self.0);
        let result = rescue_status::snapshot(&self.0, INSTALLATION, KEY);
        assert_eq!(
            files(&self.0),
            before,
            "status must preserve every file byte and file inventory"
        );
        assert_eq!(result["ready"], false);
        assert_eq!(result["full_recovery_supported"], false);
        assert_eq!(result["current_integrity"], "not_rechecked_by_status");
        for component in result["components"].as_object().unwrap().values() {
            assert_eq!(component["ready"], false);
        }
        result
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    collect(root, root, &mut out);
    out
}
fn assert_status(value: &Value, state: &str, freshness: &str, error: Value) {
    assert_eq!(value["state"], state);
    assert_eq!(value["freshness"], freshness);
    assert_eq!(value["error_code"], error);
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn t01z_zr01_missing_bundle_without_first_report_is_unavailable_readonly() {
    let f = Fixture::new();
    assert_status(
        &f.snapshot(),
        "unavailable",
        "unverified",
        json!("bundle_missing"),
    );
}
#[test]
fn t01z_zr02_missing_bundle_with_protected_waiting_report_is_waiting_not_ready() {
    let f = Fixture::new();
    f.first(br#"{"state":"unavailable","error_code":"waiting_approval","ready":false}"#);
    assert_status(
        &f.snapshot(),
        "unavailable",
        "unverified",
        json!("waiting_approval"),
    );
}
#[test]
fn t01z_zr03_malformed_first_report_preserves_missing_bundle() {
    let f = Fixture::new();
    for bytes in [
        b"{broken".as_slice(),
        b"null",
        b"[]",
        br#"{"error_code":false}"#,
    ] {
        f.first(bytes);
        assert_status(
            &f.snapshot(),
            "unavailable",
            "unverified",
            json!("bundle_missing"),
        );
    }
}
#[test]
fn t01z_zr03_oversized_first_report_preserves_missing_bundle() {
    let f = Fixture::new();
    let mut bytes = br#"{"error_code":"waiting_approval"}"#.to_vec();
    bytes.resize(32768, b' ');
    f.first(&bytes);
    assert_status(
        &f.snapshot(),
        "unavailable",
        "unverified",
        json!("waiting_approval"),
    );
    bytes.resize(32769, b' ');
    f.first(&bytes);
    assert_status(
        &f.snapshot(),
        "unavailable",
        "unverified",
        json!("bundle_missing"),
    );
}
#[test]
fn t01z_zr03_exclusively_locked_first_report_preserves_missing_bundle() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    f.first(br#"{"error_code":"waiting_approval"}"#);
    let path = f.0.join("first-install-status/last-check.json");
    let before = files(&f.0);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    let result = rescue_status::snapshot(&f.0, INSTALLATION, KEY);
    assert_eq!(result["ready"], false);
    assert_status(
        &result,
        "unavailable",
        "unverified",
        json!("bundle_missing"),
    );
    drop(lock);
    assert_eq!(files(&f.0), before);
}
#[test]
fn t01z_zr03_untrusted_first_report_preserves_missing_bundle() {
    let f = Fixture::new();
    f.first(br#"{"error_code":"waiting_approval"}"#);
    let path = f.0.join("first-install-status/last-check.json");
    let output = std::process::Command::new(
        Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32/icacls.exe"),
    )
    .arg(&path)
    .args(["/grant", "*S-1-1-0:R"])
    .output()
    .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(workspace::verify_file(&path).is_err());
    assert_status(
        &f.snapshot(),
        "unavailable",
        "unverified",
        json!("bundle_missing"),
    );
    assert!(
        workspace::verify_file(&path).is_err(),
        "query must not repair ACL"
    );
}
#[test]
fn t01z_zr04_partial_bundle_is_not_overridden_by_waiting_report() {
    for missing in ["bundle.json", "approved-manifest.json"] {
        let f = Fixture::new();
        f.bundle();
        std::fs::remove_file(f.0.join("rescue/current").join(missing)).unwrap();
        f.first(br#"{"error_code":"waiting_approval","blocked_reason":"existing_fallback_incompatible"}"#);
        assert_status(
            &f.snapshot(),
            "unavailable",
            "unverified",
            json!("bundle_incomplete"),
        );
    }
}
#[test]
fn t01z_zr05_untrusted_bundle_is_unknown_and_not_waiting() {
    let f = Fixture::new();
    // An ordinary inherited directory lacks the required protected DACL.
    std::fs::create_dir(f.0.join("rescue/current")).unwrap();
    std::fs::write(f.0.join("rescue/current/retained.bin"), b"retained source").unwrap();
    f.first(br#"{"error_code":"waiting_approval"}"#);
    assert!(workspace::verify(&f.0.join("rescue/current")).is_err());
    assert_status(
        &f.snapshot(),
        "unknown",
        "unverified",
        json!("source_untrusted"),
    );
    assert!(workspace::verify(&f.0.join("rescue/current")).is_err());
}
#[test]
fn t01z_zr06_baseline_first_report_does_not_bind_instance_target_schema_or_age() {
    let f = Fixture::new();
    for fields in [
        json!({"installation_id":"old-instance"}),
        json!({"target_key":"windows-x86_64-msi-sos"}),
        json!({"schema":999}),
        json!({"checked_at":1}),
    ] {
        let mut report = fields;
        report["error_code"] = json!("waiting_approval");
        f.first(&serde_json::to_vec(&report).unwrap());
        assert_status(
            &f.snapshot(),
            "unavailable",
            "unverified",
            json!("waiting_approval"),
        );
    }
}
#[test]
#[ignore = "待评审的实例绑定契约候选；当前实现预期失败，使用 --ignored --exact 单独执行"]
fn t01z_zr06_review_candidate_unbound_waiting_must_not_override_current_instance() {
    let f = Fixture::new();
    f.first(br#"{"state":"unavailable","error_code":"waiting_approval","ready":false}"#);
    assert_status(
        &f.snapshot(),
        "unavailable",
        "unverified",
        json!("bundle_missing"),
    );
}
#[test]
fn t01z_current_bundle_without_cache_does_not_consume_first_report() {
    let f = Fixture::new();
    f.bundle();
    f.first(br#"{"error_code":"waiting_approval"}"#);
    assert_status(
        &f.snapshot(),
        "unknown",
        "unverified",
        json!("verification_unavailable"),
    );
}
#[test]
fn t01z_current_cache_binding_and_freshness_use_production_snapshot() {
    let f = Fixture::new();
    let digest = f.bundle();
    f.first(br#"{"error_code":"waiting_approval"}"#);
    for (installation, key, authority) in [
        ("old", KEY, "yan-release-2026"),
        (INSTALLATION, "windows-x86_64-msi-sos", "yan-release-2026"),
        (INSTALLATION, KEY, "fixture"),
    ] {
        f.cache(&digest, now(), installation, key, authority);
        assert_status(
            &f.snapshot(),
            "unknown",
            "unverified",
            json!("report_binding_mismatch"),
        );
    }
    f.cache(
        &digest,
        now() - rescue_status::MAX_AGE,
        INSTALLATION,
        KEY,
        "yan-release-2026",
    );
    assert_status(&f.snapshot(), "unknown", "stale", Value::Null);
    f.cache(&digest, now() + 3600, INSTALLATION, KEY, "yan-release-2026");
    assert_status(
        &f.snapshot(),
        "unknown",
        "clock_rollback",
        json!("clock_rollback"),
    );
    f.cache("changed", now(), INSTALLATION, KEY, "yan-release-2026");
    assert_status(
        &f.snapshot(),
        "unknown",
        "source_changed",
        json!("source_changed"),
    );
    f.cache(&digest, now(), INSTALLATION, KEY, "yan-release-2026");
    assert_status(&f.snapshot(), "degraded", "fresh", Value::Null);
}

#[test]
fn t01z_current_cache_malformed_or_invalid_contract_remains_unverified() {
    let f = Fixture::new();
    let digest = f.bundle();
    f.first(br#"{"error_code":"waiting_approval"}"#);
    let path = f.0.join("rescue-status/last-check.json");
    std::fs::write(&path, b"{malformed").unwrap();
    assert_status(
        &f.snapshot(),
        "unknown",
        "unverified",
        json!("verification_unavailable"),
    );
    for (field, value) in [
        ("schema", json!(999)),
        ("state", json!("ready")),
        ("checked_at", json!(0)),
    ] {
        f.cache(&digest, now(), INSTALLATION, KEY, "yan-release-2026");
        let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record[field] = value;
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert_status(
            &f.snapshot(),
            "unknown",
            "unverified",
            json!("verification_unavailable"),
        );
    }
}

#[test]
fn t01z_zr02_fallback_incompatible_baseline_preserves_legacy_route() {
    let f = Fixture::new();
    f.first(
        br#"{"error_code":"waiting_approval","blocked_reason":"existing_fallback_incompatible"}"#,
    );
    let value = f.snapshot();
    assert_status(
        &value,
        "degraded",
        "unverified",
        json!("existing_fallback_incompatible"),
    );
    assert_eq!(value["blocked_reason"], "existing_fallback_incompatible");
    assert_eq!(value["legacy_route_preserved"], true);
}
