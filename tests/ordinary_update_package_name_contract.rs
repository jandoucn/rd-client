//! 仅 portable 生产 helper 契约；host 只做 --no-run，VM owner 执行测试。

use base::update::{try_update_sources, UpdateSource};
use hbb_common::UpdateManifest;
#[path = "../src/updater/ordinary_update_receipts/package_contract.rs"]
mod package_contract;
#[path = "../src/updater/ordinary_update_receipts/package_plan.rs"]
mod package_plan;

const KEY: &str = "windows-x86_64-msi-standard";
const NAME: &str = "rustdesk-1.5.3-2026101501-standard-windows-x86_64.msi";
const ROOT: &str = "https://download.yan.life/rustdesk/stable/1.5.3/";

fn metadata() -> UpdateManifest {
    serde_json::from_value(serde_json::json!({
        "schema": 2,
        "product": "rustdesk-yan",
        "edition": "multi",
        "channel": "stable",
        "catalog_revision": 2026101501u64,
        "version": "1.5.3",
        "build_seq": 2026101501u64,
        "source_commit": "metadata-fixture",
        "targets": {
            (KEY): {
                "version": "1.5.3",
                "build_seq": 2026101501u64,
                "source_commit": "metadata-fixture",
                "primary": format!("{ROOT}{NAME}"),
                "mirrors": ["https://mirror.invalid/transport-only.msi"],
                "size": 1234,
                "sha256": "a".repeat(64),
                "signature_key_id": "yan-release-2026",
                "signature": ""
            }
        }
    }))
    .expect("正式 manifest 数据结构必须能解析本地 metadata fixture")
}

#[test]
fn primary_metadata_resolves_approved_name_independently_of_mirror_name() {
    let manifest = metadata();
    let target = &manifest.targets[KEY];
    assert_eq!(
        package_contract::primary_package_name(&target.primary, "msi").unwrap(),
        NAME
    );
    assert_ne!(
        package_contract::primary_package_name(&target.primary, "msi").unwrap(),
        target.mirrors[0].rsplit('/').next().unwrap()
    );
}

#[test]
fn cache_name_rejects_invalid_metadata_without_changing_original() {
    let manifest = metadata();
    let original = serde_json::to_vec(&manifest).unwrap();
    for (url, error) in [
        (
            format!("{ROOT}../escape.msi"),
            "package cache source invalid",
        ),
        (
            format!("{ROOT}bad%20name.msi"),
            "package cache source invalid",
        ),
        (
            format!("{ROOT}bad\\name.msi"),
            "package cache source invalid",
        ),
        (
            format!("{ROOT}{NAME}?token=1"),
            "package cache source invalid",
        ),
        (
            format!("{ROOT}{NAME}#fragment"),
            "package cache source invalid",
        ),
        (format!("{ROOT}NUL.msi"), "package cache reserved name"),
        (
            format!("{ROOT}COM1.extra.msi"),
            "package cache reserved name",
        ),
        (format!("{ROOT}wrong.exe"), "package cache name invalid"),
        (
            format!("{ROOT}{}.msi", "a".repeat(177)),
            "package cache name invalid",
        ),
        (
            "https://other.invalid/package.msi".into(),
            "package cache source invalid",
        ),
    ] {
        assert_eq!(
            package_contract::primary_package_name(&url, "msi").unwrap_err(),
            error,
            "{url}"
        );
        assert_eq!(serde_json::to_vec(&manifest).unwrap(), original);
    }
    let boundary = format!("{}.msi", "a".repeat(176));
    assert_eq!(
        package_contract::primary_package_name(&format!("{ROOT}{boundary}"), "msi").unwrap(),
        boundary
    );
}

#[test]
fn binding_preserves_release_identity_and_transport_source() {
    let manifest = metadata();
    let target = &manifest.targets[KEY];
    let binding = package_contract::binding(package_contract::BindingFields {
        transaction_id: "a".repeat(32),
        client_id: "client".into(),
        client_uuid: "uuid".into(),
        installation_id: "b".repeat(32),
        target_key: KEY,
        package_sha256: "c".repeat(64),
        from_version: "1.5.2",
        from_build_seq: 2026101500,
        to_version: &target.version,
        to_build_seq: target.build_seq,
        source: "primary",
        command_id: None,
    });
    assert_eq!(binding.target_key, KEY);
    assert_eq!(binding.source, "primary");
    assert_eq!(binding.to_version, "1.5.3");
    assert_eq!(binding.to_build_seq, 2026101501);
    assert_eq!(binding.package_sha256, "c".repeat(64));
    assert_eq!(binding.command_id, None);
}

#[test]
fn runtime_destination_and_writer_arguments_preserve_approved_name_and_digest() {
    let manifest = metadata();
    let approved =
        package_contract::primary_package_name(&manifest.targets[KEY].primary, "msi").unwrap();
    let target = package_contract::runtime_package_destination(
        std::path::Path::new(r"C:\ordinary\runtime"),
        Some(&approved),
    )
    .unwrap();
    assert_eq!(
        target,
        std::path::Path::new(r"C:\ordinary\runtime").join(NAME)
    );
    let arguments = package_contract::runner_arguments("transaction", "digest", "3010");
    assert_eq!(
        arguments,
        ["--ordinary-msi-outcome", "transaction", "digest", "3010"]
    );
}

#[test]
fn receipt_name_helper_supports_explicit_and_legacy_names() {
    let manifest = metadata();
    let approved =
        package_contract::primary_package_name(&manifest.targets[KEY].primary, "msi").unwrap();
    let old_receipt_name = package_contract::receipt_package_name(None).unwrap();
    assert_eq!(old_receipt_name, "package.msi");
    assert_eq!(
        package_contract::receipt_package_name(Some(&approved)).unwrap(),
        NAME
    );
    assert_ne!(old_receipt_name, approved);
}

// 直接验证待接线的生产纯 helper；不代表 runtime 已接入此 helper。
#[test]
fn successful_mirror_path_keeps_primary_approved_basename() {
    let manifest = metadata();
    let original = serde_json::to_vec(&manifest).unwrap();
    let target = &manifest.targets[KEY];
    let approved = package_contract::primary_package_name(&target.primary, "msi").unwrap();
    let mut attempts = Vec::new();
    let (downloaded, source) = try_update_sources(target, |url| {
        attempts.push(url.to_owned());
        if url == target.primary {
            Err("fixture primary unavailable")
        } else {
            Ok(std::path::Path::new("fixture-cache").join(url.rsplit('/').next().unwrap()))
        }
    })
    .unwrap();
    assert_eq!(
        attempts,
        [target.primary.clone(), target.mirrors[0].clone()]
    );
    assert_eq!(source, UpdateSource::Mirror);
    assert_eq!(serde_json::to_vec(&manifest).unwrap(), original);
    let plan = package_plan::plan(
        &target.primary,
        &downloaded,
        std::path::Path::new("fixture-runtime"),
    )
    .unwrap();
    assert_eq!(plan.source, downloaded);
    assert_eq!(plan.approved_package_name, approved);
    assert_eq!(
        plan.destination,
        std::path::Path::new("fixture-runtime").join(NAME)
    );
}

#[test]
fn transport_basename_never_changes_approved_destination() {
    let manifest = metadata();
    let original = serde_json::to_vec(&manifest).unwrap();
    let directory = std::path::Path::new("fixture-runtime");
    for transport_name in [
        NAME,
        "mirror-download.bin",
        "NUL.msi",
        "transport%20name.msi",
    ] {
        let downloaded = std::path::Path::new("fixture-cache").join(transport_name);
        let plan =
            package_plan::plan(&manifest.targets[KEY].primary, &downloaded, directory).unwrap();
        assert_eq!(plan.source, downloaded);
        assert_eq!(plan.destination, directory.join(NAME));
        assert_eq!(plan.approved_package_name, NAME);
    }
    assert_eq!(serde_json::to_vec(&manifest).unwrap(), original);
}

#[test]
fn invalid_manifest_primary_cannot_be_rescued_by_valid_download_name() {
    let manifest = metadata();
    let original = serde_json::to_vec(&manifest).unwrap();
    let source = std::path::Path::new("fixture-cache").join(NAME);
    for (name, error) in [
        ("NUL.msi", "package cache reserved name"),
        ("../escape.msi", "package cache source invalid"),
        ("wrong.exe", "package cache name invalid"),
    ] {
        assert_eq!(
            package_plan::plan(
                &format!("{ROOT}{name}"),
                &source,
                std::path::Path::new("fixture-runtime")
            ),
            Err(error.into()),
        );
        assert_eq!(serde_json::to_vec(&manifest).unwrap(), original);
    }
}

#[test]
fn red_runtime_destination_rejects_reserved_basename() {
    let directory = std::path::Path::new("fixture-runtime");
    assert_eq!(
        package_contract::runtime_package_destination(directory, Some("NUL.msi")),
        Err("ordinary package basename invalid".into())
    );
}

// Compile the actual receipt types in their own module so pure tests can access
// their private serialization contract. t02_receipts excludes runtime startup.
#[cfg(t02_receipts)]
mod receipt_contract {
    include!("../src/updater/ordinary_update_receipts.rs");

    fn fixture_binding() -> Binding {
        package_contract::binding(package_contract::BindingFields {
            transaction_id: "a".repeat(32),
            client_id: "fixture-client".into(),
            client_uuid: "fixture-uuid".into(),
            installation_id: "b".repeat(32),
            target_key: super::KEY,
            package_sha256: "c".repeat(64),
            from_version: "1.5.3",
            from_build_seq: 2026100506,
            to_version: "1.5.3",
            to_build_seq: 2026101501,
            source: "mirror",
            command_id: None,
        })
        .into()
    }

    // 只供 VM 执行：真实 Outbox 写入临时目录，不启动安装器或网络。
    struct FixtureRoot(std::path::PathBuf);

    impl FixtureRoot {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let root = std::env::temp_dir().join(format!(
                "ordinary-name-contract-{}-{}-{id}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
            assert!(!root.exists());
            Self(root)
        }
    }

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            if self.0.exists() {
                std::fs::remove_dir_all(&self.0).expect("清理本测试创建的临时目录");
            }
        }
    }

    #[test]
    fn red_approved_name_changes_terminal_binding_digest() {
        let first_root = FixtureRoot::new();
        let second_root = FixtureRoot::new();
        let first = Outbox::prepare_at(&first_root.0, fixture_binding(), super::NAME).unwrap();
        let second =
            Outbox::prepare_at(&second_root.0, fixture_binding(), "renamed-release.msi").unwrap();
        let terminal = Terminal {
            status: "failed".into(),
            error_code: "msi_exit_1603".into(),
        };
        assert_ne!(
            first.intent_digest().unwrap(),
            second.intent_digest().unwrap()
        );
        assert_ne!(
            first.terminal_digest(&terminal).unwrap(),
            second.terminal_digest(&terminal).unwrap(),
            "批准名称必须进入实际 Binding 摘要，不能只改变 intent 外层 JSON"
        );
    }

    #[test]
    fn red_changed_persisted_binding_refuses_intent_and_preserves_receipt() {
        let root = FixtureRoot::new();
        let outbox = Outbox::prepare_at(&root.0, fixture_binding(), super::NAME).unwrap();
        let path = outbox.directory.join("intent.json");
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record["binding"]["package_sha256"] = "d".repeat(64).into();
        let changed = serde_json::to_vec(&record).unwrap();
        fs::write(&path, &changed).unwrap();
        let result = outbox.intent();
        assert_eq!(fs::read(&path).unwrap(), changed);
        assert!(!outbox.directory.join("outcome.json").exists());
        assert_eq!(result, Err("ordinary receipt binding mismatch".into()));
    }

    #[test]
    fn intent_serialization_roundtrips_explicit_name() {
        let mut intent = IntentRecord {
            schema: 2,
            binding: fixture_binding(),
            intent_binding: IntentBinding {
                approved_package_name: super::NAME.into(),
            },
            value: (),
        };
        let binding_before = serde_json::to_vec(&intent.binding).unwrap();
        let before = serde_json::to_vec(&intent).unwrap();
        let digest = Sha256::digest(&before);
        intent.intent_binding.approved_package_name = "renamed-release.msi".into();
        let after = serde_json::to_vec(&intent).unwrap();
        assert_eq!(serde_json::to_vec(&intent.binding).unwrap(), binding_before);
        assert_ne!(Sha256::digest(&after), digest);
        let decoded: IntentRecord = serde_json::from_slice(&after).unwrap();
        assert_eq!(
            decoded.intent_binding.approved_package_name,
            "renamed-release.msi"
        );
        assert_eq!(decoded.binding, fixture_binding());
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), after);
    }

    #[test]
    fn schema_two_requires_name_binding_and_preserves_legacy_bytes() {
        let legacy = serde_json::to_vec(&Record {
            schema: 1,
            binding: fixture_binding(),
            value: (),
        })
        .unwrap();
        let before = legacy.clone();
        let decoded: Record<()> = serde_json::from_slice(&legacy).unwrap();
        assert_eq!(decoded.schema, 1);
        assert_eq!(decoded.binding, fixture_binding());
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), before);
        assert!(serde_json::from_slice::<IntentRecord>(&legacy).is_err());
        let mut missing: serde_json::Value = serde_json::from_slice(&legacy).unwrap();
        missing["schema"] = 2.into();
        assert!(serde_json::from_value::<IntentRecord>(missing).is_err());
        assert_eq!(legacy, before);
    }

    #[test]
    fn legacy_receipt_reopens_named_destination_without_rewriting_bytes() {
        let root = FixtureRoot::new();
        let binding = fixture_binding();
        let directory = root.0.join(&binding.transaction_id);
        workspace::create(&root.0).unwrap();
        workspace::create(&directory).unwrap();
        let intent = serde_json::to_vec(&Record {
            schema: 1,
            binding: binding.clone(),
            value: (),
        })
        .unwrap();
        let name = serde_json::to_vec(&Record {
            schema: 1,
            binding: binding.clone(),
            value: super::NAME,
        })
        .unwrap();
        fs::write(directory.join("intent.json"), &intent).unwrap();
        fs::write(directory.join("package-name.json"), &name).unwrap();
        let reopened = Outbox::fixture_open(&root.0, &binding.transaction_id).unwrap();
        assert_eq!(reopened.package_name().unwrap(), super::NAME);
        assert_eq!(
            package_contract::runtime_package_destination(
                &directory,
                Some(&reopened.package_name().unwrap())
            )
            .unwrap(),
            directory.join(super::NAME)
        );
        assert_eq!(
            reopened.intent_digest().unwrap(),
            hex::encode(Sha256::digest(&intent))
        );
        assert_eq!(fs::read(directory.join("intent.json")).unwrap(), intent);
        assert_eq!(fs::read(directory.join("package-name.json")).unwrap(), name);
        assert!(!directory.join("outcome.json").exists());
    }

    #[test]
    fn legacy_terminal_digest_and_existing_ack_preserve_original_bytes() {
        let root = FixtureRoot::new();
        let mut binding = fixture_binding();
        binding.command_id = Some("legacy-command".into());
        let directory = root.0.join(&binding.transaction_id);
        workspace::create(&root.0).unwrap();
        workspace::create(&directory).unwrap();
        let intent = serde_json::to_vec(&Record {
            schema: 1,
            binding: binding.clone(),
            value: (),
        })
        .unwrap();
        let terminal = Terminal {
            status: "failed".into(),
            error_code: "msi_exit_1603".into(),
        };
        let terminal_bytes = serde_json::to_vec(&Record {
            schema: 1,
            binding: binding.clone(),
            value: &terminal,
        })
        .unwrap();
        let digest = hex::encode(Sha256::digest(&terminal_bytes));
        let ack = serde_json::to_vec(&Record {
            schema: 1,
            binding: binding.clone(),
            value: &digest,
        })
        .unwrap();
        fs::write(directory.join("intent.json"), &intent).unwrap();
        fs::write(directory.join("terminal.json"), &terminal_bytes).unwrap();
        fs::write(directory.join("ack-update.json"), &ack).unwrap();
        let reopened = Outbox::fixture_open(&root.0, &binding.transaction_id).unwrap();
        assert_eq!(reopened.terminal_digest(&terminal).unwrap(), digest);
        assert_eq!(reopened.pending(Channel::Update).unwrap(), None);
        assert_eq!(
            reopened.pending(Channel::Command).unwrap(),
            Some((terminal, digest.clone()))
        );
        reopened.acknowledge(Channel::Update, &digest).unwrap();
        assert_eq!(fs::read(directory.join("intent.json")).unwrap(), intent);
        assert_eq!(
            fs::read(directory.join("terminal.json")).unwrap(),
            terminal_bytes
        );
        assert_eq!(fs::read(directory.join("ack-update.json")).unwrap(), ack);
        assert!(!directory.join("ack-command.json").exists());
        assert!(!directory.join("outcome.json").exists());
    }

    #[test]
    fn invalid_name_is_rejected_before_any_intent_directory_access() {
        let binding = fixture_binding();
        let before = serde_json::to_vec(&binding).unwrap();
        for invalid in ["NUL.msi", "COM1.extra.msi", "../escape.msi", ""] {
            let root = FixtureRoot::new();
            let result = Outbox::prepare_at(&root.0, binding.clone(), invalid);
            assert!(
                !root.0.exists(),
                "无效名称不得创建 root 或 intent: {invalid}"
            );
            assert_eq!(
                result.err().as_deref(),
                Some("ordinary package basename invalid"),
                "{invalid}"
            );
            assert_eq!(serde_json::to_vec(&binding).unwrap(), before);
        }
    }

    #[test]
    fn approved_plan_flows_through_real_outbox_digest_and_writer_request() {
        let root = FixtureRoot::new();
        let manifest = super::metadata();
        let binding = fixture_binding();
        let directory = root.0.join(&binding.transaction_id);
        let source = Path::new("fixture-cache/transport-only.msi");
        let plan =
            super::package_plan::plan(&manifest.targets[super::KEY].primary, source, &directory)
                .unwrap();
        let outbox =
            Outbox::prepare_at(&root.0, binding.clone(), &plan.approved_package_name).unwrap();
        let reopened = Outbox::fixture_open(&root.0, &binding.transaction_id).unwrap();
        let destination = package_contract::runtime_package_destination(
            &directory,
            Some(&reopened.package_name().unwrap()),
        )
        .unwrap();
        assert_eq!(destination, plan.destination);
        assert_eq!(destination, directory.join(super::NAME));
        assert_eq!(plan.source, source);
        assert_eq!(reopened.binding(), outbox.binding());
        let digest = reopened.intent_digest().unwrap();
        let arguments =
            package_contract::runner_arguments(&binding.transaction_id, &digest, "3010");
        assert_eq!(arguments[0], "--ordinary-msi-outcome");
        let request = OutcomeWriteRequest::parse(
            &arguments[1..]
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(request.transaction_id, binding.transaction_id);
        assert_eq!(request.intent_sha256, outbox.intent_digest().unwrap());
        assert_eq!(request.exit_code, 3010);
        assert!(!directory.join("outcome.json").exists());
    }
}

#[test]
fn package_basename_rejects_paths_and_accepts_legacy_receipt_name() {
    assert_eq!(
        package_contract::receipt_package_name(None).unwrap(),
        "package.msi"
    );
    for name in ["../x.msi", r"C:\x.msi", "x.msi?query", "", "name\0.msi"] {
        assert!(
            package_contract::receipt_package_name(Some(name)).is_err(),
            "{name}"
        );
    }
}
