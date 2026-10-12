#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;

use hbb_common::{
    base64::{engine::general_purpose::STANDARD, Engine as _},
    sodiumoxide::crypto::sign,
    UpdateManifest, UpdateTarget,
};
use sha2::{Digest, Sha256};

fn fixture(key: &str, bytes: &[u8]) -> (UpdateManifest, sign::PublicKey, sign::SecretKey) {
    hbb_common::sodiumoxide::init().unwrap();
    let (public, secret) = sign::gen_keypair();
    let mut target = UpdateTarget {
        version: "1.5.3".into(),
        build_seq: 2026101101,
        source_commit: "fixture-commit".into(),
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/package.msi".into(),
        size: bytes.len() as u64,
        sha256: hex::encode(Sha256::digest(bytes)),
        signature_key_id: "fixture-key".into(),
        ..Default::default()
    };
    let mut manifest = UpdateManifest {
        schema: 2,
        product: "rustdesk-yan".into(),
        edition: "multi".into(),
        channel: "stable".into(),
        catalog_revision: 2026101101,
        version: "1.5.3".into(),
        build_seq: 2026101101,
        source_commit: "fixture-commit".into(),
        ..Default::default()
    };
    let payload = base::update::update_signature_payload(&manifest, key, &target).unwrap();
    target.signature = STANDARD.encode(sign::sign_detached(&payload, &secret));
    manifest.targets.insert(key.into(), target);
    (manifest, public, secret)
}

#[test]
fn valid_signature_binds_exact_product_target_and_approved_package_bytes() {
    let key = "windows-x86_64-msi-standard";
    let (manifest, public, _) = fixture(key, b"package");
    let target =
        package_gate::verify_with_key(&manifest, key, 2026100502, "fixture-key", public.as_ref())
            .unwrap();
    assert_eq!(target.sha256, hex::encode(Sha256::digest(b"package")));
    assert!(
        package_gate::verify(&manifest, key, 2026100502).is_err(),
        "fixture key cannot become production authority"
    );
    assert!(package_gate::verify_with_key(
        &manifest,
        "windows-x86_64-msi-sos",
        2026100502,
        "fixture-key",
        public.as_ref()
    )
    .is_err());
}

#[test]
fn resolved_package_must_match_command_reference_target_and_build() {
    let key = "windows-x86_64-msi-standard";
    let (manifest, _, _) = fixture(key, b"package");
    let raw = serde_json::to_value(&manifest).unwrap();
    let reference = package_gate::package_reference(&raw, key).unwrap();
    let reply = serde_json::json!({"package_id":reference,"target_key":key,"manifest":raw});
    let parsed = package_gate::bind_resolved(&reply, &reference, key, 2026101101).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), serde_json::to_value(manifest).unwrap());
    for field in ["package_id", "target_key", "build", "metadata"] {
        let mut changed = reply.clone();
        match field {
            "package_id" => changed["package_id"] = "0".repeat(64).into(),
            "target_key" => changed["target_key"] = "windows-x86_64-msi-sos".into(),
            "build" => changed["manifest"]["targets"][key]["build_seq"] = 2026101102u64.into(),
            _ => changed["manifest"]["targets"][key]["primary"] = "https://download.yan.life/rustdesk/stable/other.msi".into(),
        }
        assert!(package_gate::bind_resolved(&changed, &reference, key, 2026101101).is_err(), "{field}");
    }
    assert!(package_gate::bind_resolved(&reply, &reference, key, 2026101102).is_err());
    assert!(package_gate::bind_resolved(&reply, "bad", key, 2026101101).is_err());
}

#[test]
fn package_cache_name_preserves_msi_source_name_and_refuses_windows_special_names() {
    assert_eq!(package_gate::cache_name("https://download.yan.life/rustdesk/stable/1.5.3/agent-health-v25.msi", "msi").unwrap(), "agent-health-v25.msi");
    for name in ["CON.msi", "nul.msi", "LPT1.msi", "COM9.msi", "COM1.extra.msi", "bad:name.msi", "bad?.msi", "bad%20name.msi", "bad\\name.msi", "file.exe"] {
        let url = format!("https://download.yan.life/rustdesk/stable/{name}");
        assert!(package_gate::cache_name(&url, "msi").is_err(), "{name}");
    }
    for url in ["https://other.invalid/valid.msi", "https://download.yan.life/rustdesk/stable/../a.msi", "https://download.yan.life/rustdesk/stable/a.msi?secret=1"] {
        assert!(package_gate::cache_name(url, "msi").is_err());
    }
}

#[test]
fn altered_signed_fields_unknown_key_and_downgrade_never_authorize_install() {
    let key = "windows-x86_64-msi-standard";
    let (manifest, public, _) = fixture(key, b"package");
    for index in 0..9 {
        let mut changed = manifest.clone();
        match index {
            0 => changed.product = "other".into(),
            1 => changed.channel = "nightly".into(),
            2 => changed.edition = "sos".into(),
            3 => changed.schema = 3,
            4 => changed.targets.get_mut(key).unwrap().sha256 = "a".repeat(64),
            5 => changed.targets.get_mut(key).unwrap().size += 1,
            6 => changed.targets.get_mut(key).unwrap().version = "1.5.4".into(),
            7 => changed.targets.get_mut(key).unwrap().build_seq += 1,
            _ => changed.targets.get_mut(key).unwrap().signature_key_id = "unknown".into(),
        }
        assert!(
            package_gate::verify_with_key(
                &changed,
                key,
                2026100502,
                "fixture-key",
                public.as_ref()
            )
            .is_err(),
            "index={index}"
        );
    }
    assert!(package_gate::verify_with_key(
        &manifest,
        key,
        2026101102,
        "fixture-key",
        public.as_ref()
    )
    .is_err());
    let (other, _) = sign::gen_keypair();
    assert!(package_gate::verify_with_key(
        &manifest,
        key,
        2026100502,
        "fixture-key",
        other.as_ref()
    )
    .is_err());
}

#[test]
fn unsigned_url_changes_do_not_escape_fixed_download_policy() {
    let key = "windows-x86_64-msi-standard";
    let (manifest, public, _) = fixture(key, b"package");
    for url in [
        "http://download.yan.life/rustdesk/stable/a.msi",
        "https://download.yan.life.evil.example/rustdesk/stable/a.msi",
        "https://user@download.yan.life/rustdesk/stable/a.msi",
        "https://download.yan.life:444/rustdesk/stable/a.msi",
        "https://download.yan.life/rustdesk/stable/%2e%2e/a.msi",
        "https://download.yan.life/rustdesk/stable/../unapproved/a.msi",
        "https://download.yan.life/rustdesk/stable/releases/../a.msi",
        "https://download.yan.life/rustdesk/stable/a.exe",
        "https://download.yan.life/rustdesk/stable/a.msi?token=x",
    ] {
        let mut changed = manifest.clone();
        changed.targets.get_mut(key).unwrap().primary = url.into();
        assert!(
            package_gate::verify_with_key(
                &changed,
                key,
                2026100502,
                "fixture-key",
                public.as_ref()
            )
            .is_err(),
            "{url}"
        );
    }
}

#[cfg(windows)]
#[test]
fn verified_file_remains_locked_against_mutation_through_execution_boundary() {
    let root = std::env::temp_dir().join(format!("rustdesk-package-lock-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("package.msi");
    std::fs::write(&path, b"package").unwrap();
    let (manifest, _, _) = fixture("windows-x86_64-msi-standard", b"package");
    let target = manifest.targets.values().next().unwrap();
    let locked = package_gate::lock_verified_file(&path, target).unwrap();
    assert!(std::fs::write(&path, b"changed").is_err());
    assert!(std::fs::remove_file(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"package");
    drop(locked);
    std::fs::write(&path, b"changed").unwrap();
    assert!(package_gate::lock_verified_file(&path, target).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"changed");
    std::fs::remove_dir_all(root).unwrap();
}
