//! T02 frozen-source fixture. Host builds only; execute on the coordinator's VM.
//! Build with rustc --test --cfg t02_projection and T02_UPDATE_SOURCE pointing to
//! an unchanged extraction of updater::update_new_version. The fixture replaces
//! only installer/reporting boundaries; it does not execute MSI or contact APIs.
//! A missing receipt is a contract failure, never evidence that installation failed.

#[cfg(not(any(t02_projection, t02_receipts, t02_wiring, t02_preparation)))]
#[test]
#[ignore = "requires the frozen T02 projection EXE; do not run client tests on host"]
fn t02_requires_frozen_source_projection() {
    panic!("build the dedicated T02 projection; a zero-test run is not acceptance");
}

#[cfg(all(t02_projection, not(t02_integration)))]
mod fixture {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::atomic::{AtomicUsize, Ordering},
    };

    type ResultType<T> = Result<T, Box<dyn std::error::Error>>;
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    macro_rules! bail {
        ($message:expr) => {
            return Err(($message).into())
        };
    }
    #[macro_export]
    macro_rules! fixture_log {
        ($($arg:tt)*) => { { let _ = format_args!($($arg)*); } };
    }
    mod log {
        pub(crate) use crate::fixture_log as debug;
        pub(crate) use crate::fixture_log as error;
    }
    mod hbb_common {
        #[macro_export]
        macro_rules! fixture_allow_err {
            ($value:expr) => {{
                let _ = $value;
            }};
        }
        pub(crate) use crate::fixture_allow_err as allow_err;
    }

    // Compile the complete production function unchanged, including the EXE arm.
    include!(env!("T02_UPDATE_SOURCE"));

    #[derive(Clone)]
    enum UpdateSource {
        Primary,
        Mirror,
    }
    impl UpdateSource {
        fn as_str(&self) -> &'static str {
            match self {
                Self::Primary => "primary",
                Self::Mirror => "mirror",
            }
        }
    }
    struct PendingUpdateEvent {
        transaction_id: String,
        from_version: String,
        from_build_seq: u64,
        version: String,
        build_seq: u64,
        source: UpdateSource,
        command_id: Option<String>,
    }
    impl PendingUpdateEvent {
        fn cli_args(&self) -> Vec<String> {
            panic!("MSI fixture entered EXE arm")
        }
    }
    fn new_update_transaction_id() -> String {
        "fixture-attempt".into()
    }
    fn root() -> PathBuf {
        std::env::var_os("T02_ROOT").unwrap().into()
    }
    fn mode() -> String {
        std::env::var("T02_MODE").unwrap()
    }
    fn durable_receipts(directory: &Path) -> Vec<u8> {
        let mut records = Vec::new();
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.path().is_dir() {
                records.extend(durable_receipts(&entry.path()));
            } else if ["intent.json", "outcome.json", "terminal.json"]
                .iter()
                .any(|name| entry.file_name() == *name)
            {
                let bytes = fs::read(entry.path()).unwrap();
                let receipt: serde_json::Value =
                    serde_json::from_slice(&bytes).expect("receipt must be valid JSON");
                assert_eq!(receipt["schema"], 1);
                assert_eq!(receipt["binding"]["to_build_seq"], 2026101501u64);
                assert!(receipt["binding"]["transaction_id"]
                    .as_str()
                    .is_some_and(|id| id.len() == 32));
                records.extend(bytes);
            }
        }
        records
    }
    fn append(name: &str, value: &str) {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root().join(name))
            .unwrap();
        writeln!(file, "{value}").unwrap();
        file.sync_all().unwrap();
    }
    fn report_update_event(status: &str, _: &str, _: u64, _: &str) -> bool {
        append("requests", status);
        if durable_receipts(&root()).is_empty() {
            append("violations", "terminal_not_durable_before_send");
        }
        !matches!(mode().as_str(), "lost_ack" | "install_failed")
    }
    fn remove_download_artifact(path: &Path) {
        fs::remove_file(path).unwrap();
    }

    pub(crate) mod platform {
        use super::*;
        pub fn get_current_process_session_id() -> Option<u32> {
            Some(0)
        }
        pub fn update_me_msi(_: &str, _: bool) -> ResultType<()> {
            if durable_receipts(&root()).is_empty() {
                append("violations", "pending_not_durable_before_installer");
            }
            append("installer", "entered");
            match mode().as_str() {
                // The independent installer survives; the old reporting worker does not.
                "worker_exit" => std::process::exit(0),
                "install_failed" => Err("fixture_installer_failed".into()),
                _ => Ok(()),
            }
        }
        pub fn get_custom_client_staging_dir() -> PathBuf {
            panic!("EXE arm")
        }
        pub fn handle_custom_client_staging_dir_before_update(_: &Path) -> ResultType<()> {
            panic!("EXE arm")
        }
        pub fn remove_custom_client_staging_dir(_: &Path) -> ResultType<()> {
            panic!("EXE arm")
        }
        pub fn launch_privileged_process(_: u32, _: &str) -> ResultType<*mut u8> {
            panic!("EXE arm")
        }
    }

    struct Lab(PathBuf);
    impl Lab {
        fn new() -> Self {
            assert_eq!(
                std::env::var("RUSTDESK_T02_VM").as_deref(),
                Ok("ordinary-terminal"),
                "SETUP BLOCKED: VM coordinator must explicitly enable the fixture"
            );
            let path = std::env::temp_dir().join(format!(
                "rustdesk-t02-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn run(&self, mode: &str) {
            fs::write(self.0.join("package.msi"), b"local fixture, never executed").unwrap();
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "fixture::t02_child_worker",
                    "--ignored",
                    "--nocapture",
                ])
                .env("T02_ROOT", &self.0)
                .env("T02_MODE", mode)
                .stdin(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "fixture child setup failed: {status}");
        }
        fn read(&self, name: &str) -> String {
            fs::read_to_string(self.0.join(name)).unwrap_or_default()
        }
    }
    impl Drop for Lab {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    #[ignore = "spawned only by the VM fixture parent"]
    fn t02_child_worker() {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let result = update_new_version(
            true,
            "1.5.3",
            2026101501,
            "primary",
            &root().join("package.msi"),
            None,
        );
        if mode() == "install_failed" {
            assert_eq!(result.unwrap_err().to_string(), "fixture_installer_failed");
        } else {
            result.unwrap();
        }
    }

    #[test]
    fn t02_pending_must_be_durable_before_installer_can_stop_worker() {
        let lab = Lab::new();
        lab.run("worker_exit");
        assert_eq!(lab.read("installer"), "entered\n");
        assert!(
            !lab.read("violations")
                .contains("pending_not_durable_before_installer"),
            "ordinary MSI entered installer without restart-recoverable intent"
        );
    }

    #[test]
    fn t02_worker_exit_must_leave_original_attempt_for_new_process() {
        let lab = Lab::new();
        lab.run("worker_exit");
        assert_eq!(
            lab.read("requests"),
            "",
            "old worker cannot report after exit"
        );
        assert!(!durable_receipts(&lab.0).is_empty(),
            "new process has no ordinary MSI pending receipt to reconcile; new build/status is not installed");
    }

    #[test]
    fn t02_terminal_must_be_durable_before_send_and_lost_ack_restart() {
        let lab = Lab::new();
        lab.run("lost_ack");
        assert_eq!(lab.read("requests"), "installed\n");
        let receipt = durable_receipts(&lab.0);
        assert!(
            !receipt.is_empty(),
            "lost ACK discarded ordinary installed event; restart cannot retry it"
        );
        assert!(!lab
            .read("violations")
            .contains("terminal_not_durable_before_send"));
    }

    #[test]
    fn t02_failed_attempt_must_survive_restart_before_successful_retry() {
        let lab = Lab::new();
        lab.run("install_failed");
        assert_eq!(lab.read("requests"), "failed\n");
        let failure = durable_receipts(&lab.0);
        assert!(
            !failure.is_empty(),
            "ordinary failure has no durable receipt after failed terminal ACK"
        );
        lab.run("lost_ack");
        assert_eq!(lab.read("requests"), "failed\ninstalled\n");
        assert!(
            durable_receipts(&lab.0)
                .windows(failure.len())
                .any(|bytes| bytes == failure),
            "retry overwrote first failure"
        );
    }

    #[test]
    fn t02_installer_error_without_rollback_proof_reports_failed() {
        let lab = Lab::new();
        lab.run("install_failed");
        assert_eq!(
            lab.read("requests"),
            "failed\n",
            "installer error is not proof that old installation was restored"
        );
    }
}

#[cfg(t02_projection)]
pub const VERSION: &str = "1.5.3";
#[cfg(t02_projection)]
pub const BUILD_SEQ: u64 = 2026100506;
#[cfg(t02_projection)]
pub fn is_custom_client() -> bool {
    false
}
#[cfg(t02_projection)]
pub(crate) use fixture::platform;

#[cfg(all(windows, t02_receipts, not(any(t02_projection, t02_wiring))))]
#[path = "../src/updater/ordinary_update_receipts.rs"]
mod ordinary_update_receipts;

#[cfg(all(windows, t02_receipts, not(any(t02_projection, t02_wiring))))]
mod persistence {
    use super::lease_guard;
    use super::ordinary_update_receipts::{
        Binding, Channel, InstallerOutcome, Outbox, VerifiedRunningTarget,
    };
    use sha2::Digest;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn supervisor_refuses_dispatch_without_executed_writer_selfcheck() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        let before = contents(&lab, "intent.json");
        let mut launched = false;
        let result = outbox.dispatch(|_, _| {
            launched = true;
            Ok(())
        });
        assert_eq!(contents(&lab, "intent.json"), before);
        assert!(!lab.0.join("a".repeat(32)).join("outcome.json").exists());
        assert!(
            !launched,
            "BUSINESS RED: installer entered without executed fixed writer selfcheck"
        );
        assert!(
            result.is_err(),
            "missing writer readiness must be a concrete refusal"
        );
    }

    #[test]
    fn supervisor_dispatch_keeps_install_lease_until_outcome_is_durable() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        let before = contents(&lab, "intent.json");
        use super::ordinary_update_receipts::supervisor;
        supervisor::fixture_stage(&outbox).unwrap();
        let guard = supervisor::guard::InstallGuard::try_acquire()
            .unwrap()
            .unwrap();
        supervisor::launch(&outbox, guard).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !lab.0.join("child-finished").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "supervisor child failed/timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(contents(&lab, "intent.json"), before);
        assert_eq!(
            lab.open(binding()).reconcile(None).unwrap().error_code,
            "msi_exit_1603"
        );
        assert_eq!(
            fs::read(lab.0.join("child-finished")).unwrap(),
            b"lease-excluded-before-and-after-outcome"
        );
        supervisor::fixture_wait_exit(&outbox).unwrap();
        assert!(lease_guard::InstallGuard::try_acquire().unwrap().is_some());
    }

    #[test]
    #[ignore = "production launch handshake fixture child only"]
    fn supervisor_green_child() {
        use super::ordinary_update_receipts::supervisor;
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let root = PathBuf::from(std::env::var_os("T02_SUPERVISOR_ROOT").unwrap());
        let outbox = Outbox::fixture_open(&root, &"a".repeat(32)).unwrap();
        supervisor::fixture_run(
            &outbox,
            &std::env::var("T02_SUPERVISOR_DIGEST").unwrap(),
            |_, _| {
                let excluded = || {
                    std::thread::spawn(|| {
                        supervisor::guard::InstallGuard::try_acquire()
                            .unwrap()
                            .is_none()
                    })
                    .join()
                    .unwrap()
                };
                assert!(excluded(), "lease not owned before installer");
                outbox.record_outcome(InstallerOutcome::Failed {
                    exit_code: 1603,
                    error_code: "msi_exit_1603".into(),
                })?;
                assert!(excluded(), "lease released before durable outcome");
                Ok(())
            },
        )
        .unwrap();
        fs::write(
            root.join("child-finished"),
            b"lease-excluded-before-and-after-outcome",
        )
        .unwrap();
    }

    #[test]
    fn ordinary_install_dispatch_observes_bound_intent_before_installer_and_refuses_repeat() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        let bytes = contents(&lab, "intent.json");
        outbox
            .dispatch(|id, digest| {
                assert_eq!(id, binding().transaction_id);
                assert_eq!(digest, hex::encode(sha2::Sha256::digest(&bytes)));
                let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(v["schema"], 1);
                assert_eq!(v["binding"]["package_sha256"], binding().package_sha256);
                assert_eq!(v["binding"]["to_build_seq"], binding().to_build_seq);
                assert_eq!(v["binding"]["installation_id"], binding().installation_id);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            outbox
                .dispatch(|_, _| panic!("must not reinstall"))
                .unwrap_err(),
            "ordinary installer already dispatched or marker unavailable"
        );
        assert_eq!(contents(&lab, "intent.json"), bytes);
    }

    #[test]
    fn durable_installed_partial_ack_reopens_real_receipt_and_only_retries_missing_channel() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        outbox
            .record_outcome(InstallerOutcome::Succeeded { exit_code: 3010 })
            .unwrap();
        outbox
            .reconcile(Some(&VerifiedRunningTarget { binding: binding() }))
            .unwrap();
        let before = contents(&lab, "terminal.json");
        outbox
            .report_pending(|channel, b, t| {
                assert_eq!(b, &binding());
                assert_eq!(t.status, "installed");
                matches!(channel, Channel::Update)
            })
            .unwrap();
        drop(outbox);
        let reloaded = Outbox::fixture_open(&lab.0, &binding().transaction_id).unwrap();
        let mut channels = Vec::new();
        reloaded
            .report_pending(|channel, _, _| {
                channels.push(matches!(channel, Channel::Command));
                true
            })
            .unwrap();
        assert_eq!(channels, vec![true]);
        assert_eq!(contents(&lab, "terminal.json"), before);
        assert_eq!(reloaded.pending(Channel::Update).unwrap(), None);
        assert_eq!(reloaded.pending(Channel::Command).unwrap(), None);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&contents(&lab, "outcome.json")).unwrap()
                ["value"]["exit_code"],
            3010
        );
    }
    struct Lab(PathBuf);
    impl Lab {
        fn new() -> Self {
            assert_eq!(
                std::env::var("RUSTDESK_T02_VM").as_deref(),
                Ok("ordinary-terminal"),
                "VM runner only"
            );
            let path = std::env::temp_dir().join(format!(
                "t02-outbox-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            Self(path)
        }
        fn open(&self, binding: Binding) -> Outbox {
            Outbox::fixture(&self.0, binding).unwrap()
        }
    }
    impl Drop for Lab {
        fn drop(&mut self) {
            if self.0.exists() {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }
    fn binding() -> Binding {
        Binding {
            transaction_id: "a".repeat(32),
            client_id: "fixture-id".into(),
            client_uuid: "fixture-uuid".into(),
            installation_id: "b".repeat(32),
            target_key: "windows-x86_64-msi-standard".into(),
            package_sha256: "c".repeat(64),
            from_version: "1.5.3".into(),
            from_build_seq: 2026100506,
            to_version: "1.5.3".into(),
            to_build_seq: 2026101501,
            source: "primary".into(),
            command_id: Some("fixture-command".into()),
        }
    }
    fn contents(lab: &Lab, name: &str) -> Vec<u8> {
        fs::read(lab.0.join("a".repeat(32)).join(name)).unwrap()
    }

    #[test]
    fn schema_two_intent_hash_binds_package_name_and_rejects_missing_receipt() {
        let lab = Lab::new();
        let outbox = Outbox::fixture_named(&lab.0, binding(), "approved-release.msi").unwrap();
        let intent = contents(&lab, "intent.json");
        let value: serde_json::Value = serde_json::from_slice(&intent).unwrap();
        assert_eq!(value["schema"], 2);
        assert_eq!(
            value["intent_binding"]["approved_package_name"],
            "approved-release.msi"
        );
        let digest = outbox.intent_digest().unwrap();
        assert_eq!(outbox.package_name().unwrap(), "approved-release.msi");
        let changed = serde_json::from_value::<serde_json::Value>(serde_json::json!({
            "schema": 2,
            "binding": value["binding"].clone(),
            "value": null,
            "approved_package_name": "substituted.msi"
        }))
        .unwrap();
        fs::write(
            lab.0.join("a".repeat(32)).join("intent.json"),
            serde_json::to_vec(&changed).unwrap(),
        )
        .unwrap();
        assert_ne!(
            hex::encode(sha2::Sha256::digest(contents(&lab, "intent.json"))),
            digest
        );
        assert!(outbox.intent_digest().is_err());
    }

    #[test]
    fn schema_one_pending_receipt_bytes_remain_unchanged() {
        let lab = Lab::new();
        let transaction = lab.0.join("a".repeat(32));
        std::fs::create_dir_all(&transaction).unwrap();
        let legacy_binding = binding();
        let intent = serde_json::to_vec(&serde_json::json!({
            "schema": 1,
            "binding": legacy_binding,
            "value": null
        }))
        .unwrap();
        let outcome = serde_json::to_vec(&serde_json::json!({
            "schema": 1,
            "binding": binding(),
            "value": {"status":"failed","exit_code":1603,"error_code":"msi_exit_1603"}
        }))
        .unwrap();
        fs::write(transaction.join("intent.json"), &intent).unwrap();
        fs::write(transaction.join("outcome.json"), &outcome).unwrap();
        let reopened = Outbox::fixture_open(&lab.0, &"a".repeat(32)).unwrap();
        assert_eq!(reopened.binding().transaction_id, "a".repeat(32));
        assert_eq!(fs::read(transaction.join("intent.json")).unwrap(), intent);
        assert_eq!(fs::read(transaction.join("outcome.json")).unwrap(), outcome);
    }

    #[test]
    fn invalid_package_name_is_rejected_before_intent_creation() {
        for invalid in ["NUL.msi", "COM1.extra.msi", "LPT9.msi"] {
            let lab = Lab::new();
            let result = Outbox::fixture_named(&lab.0, binding(), invalid);
            assert!(
                matches!(result, Err(error) if error == "ordinary package basename invalid"),
                "{invalid}"
            );
            assert!(!lab.0.join("a".repeat(32)).exists(), "{invalid}");
        }
    }

    #[test]
    #[ignore = "VM fixture parent invokes this exact child"]
    fn restart_child_reads_same_unacknowledged_terminal() {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let root = PathBuf::from(std::env::var_os("T02_RECEIPTS_ROOT").unwrap());
        let outbox = Outbox::fixture_open(&root, &"a".repeat(32)).unwrap();
        if std::env::var("T02_INTENT_ONLY").as_deref() == Ok("yes") {
            assert_eq!(
                outbox.reconcile(None).unwrap_err(),
                "ordinary receipt missing: outcome.json"
            );
            assert!(!root.join("a".repeat(32)).join("terminal.json").exists());
            return;
        }
        let (terminal, digest) = outbox.pending(Channel::Update).unwrap().unwrap();
        assert_eq!(terminal.status, "installed");
        assert_eq!(std::env::var("T02_RECEIPTS_DIGEST").unwrap(), digest);
        assert!(!root.join("a".repeat(32)).join("ack-update.json").exists());
    }

    #[test]
    fn lost_ack_new_process_reads_exact_original_terminal_without_reinstall() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        outbox
            .record_outcome(InstallerOutcome::Succeeded { exit_code: 0 })
            .unwrap();
        outbox
            .reconcile(Some(&VerifiedRunningTarget { binding: binding() }))
            .unwrap();
        let (_, digest) = outbox.pending(Channel::Update).unwrap().unwrap();
        let before = contents(&lab, "terminal.json");
        drop(outbox);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "persistence::restart_child_reads_same_unacknowledged_terminal",
                "--ignored",
                "--nocapture",
            ])
            .env("T02_RECEIPTS_ROOT", &lab.0)
            .env("T02_RECEIPTS_DIGEST", digest)
            .stdin(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "child fixture failed: {status}");
        assert_eq!(contents(&lab, "terminal.json"), before);
        assert!(lab
            .open(binding())
            .pending(Channel::Update)
            .unwrap()
            .is_some());
    }

    #[test]
    fn intent_only_new_process_cannot_infer_success_from_target_build() {
        let lab = Lab::new();
        drop(lab.open(binding()));
        let before = contents(&lab, "intent.json");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "persistence::restart_child_reads_same_unacknowledged_terminal",
                "--ignored",
                "--nocapture",
            ])
            .env("T02_RECEIPTS_ROOT", &lab.0)
            .env("T02_INTENT_ONLY", "yes")
            .stdin(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(
            status.success(),
            "intent-only restart child failed: {status}"
        );
        assert_eq!(contents(&lab, "intent.json"), before);
        assert!(!lab.0.join("a".repeat(32)).join("terminal.json").exists());
    }

    #[test]
    fn outcome_writer_request_rejects_paths_statuses_and_noncanonical_exit() {
        use super::ordinary_update_receipts::OutcomeWriteRequest;
        let id = "a".repeat(32);
        let digest = "b".repeat(64);
        for args in [
            vec!["../escape".into(), digest.clone(), "1603".into()],
            vec![id.clone(), r"C:\other\intent.json".into(), "1603".into()],
            vec![id.clone(), digest.clone(), "installed".into()],
            vec![id.clone(), digest.clone(), "-1".into()],
            vec![id.clone(), digest.clone(), "01603".into()],
            vec![id.clone(), digest.clone(), "4294967296".into()],
            vec![
                id.clone(),
                digest.clone(),
                "1603".into(),
                r"C:\other".into(),
            ],
        ] {
            assert!(
                OutcomeWriteRequest::parse(&args).is_err(),
                "invalid writer request admitted: {args:?}"
            );
        }
        assert!(OutcomeWriteRequest::parse(&[id, digest, "1603".into()]).is_ok());
    }

    #[test]
    fn independent_writer_checks_intent_digest_before_recording_real_exit() {
        use super::ordinary_update_receipts::OutcomeWriteRequest;
        let lab = Lab::new();
        drop(lab.open(binding()));
        let before = contents(&lab, "intent.json");
        let digest = hex::encode(sha2::Sha256::digest(&before));
        let bad =
            OutcomeWriteRequest::parse(&["a".repeat(32), "f".repeat(64), "1603".into()]).unwrap();
        assert_eq!(
            bad.fixture_write(&lab.0).unwrap_err(),
            "ordinary intent digest mismatch"
        );
        assert_eq!(contents(&lab, "intent.json"), before);
        assert!(!lab.0.join("a".repeat(32)).join("outcome.json").exists());
        let request = OutcomeWriteRequest::parse(&["a".repeat(32), digest, "1603".into()]).unwrap();
        request.fixture_write(&lab.0).unwrap();
        request.fixture_write(&lab.0).unwrap();
        let terminal = lab.open(binding()).reconcile(None).unwrap();
        assert_eq!(terminal.status, "failed");
        assert_eq!(terminal.error_code, "msi_exit_1603");
        assert_eq!(contents(&lab, "intent.json"), before);
    }

    #[test]
    fn writer_success_exit_does_not_publish_terminal_without_health() {
        use super::ordinary_update_receipts::OutcomeWriteRequest;
        for exit in ["0", "3010"] {
            let lab = Lab::new();
            drop(lab.open(binding()));
            let digest = hex::encode(sha2::Sha256::digest(contents(&lab, "intent.json")));
            OutcomeWriteRequest::parse(&["a".repeat(32), digest, exit.into()])
                .unwrap()
                .fixture_write(&lab.0)
                .unwrap();
            assert_eq!(
                lab.open(binding()).reconcile(None).unwrap_err(),
                "ordinary running target unconfirmed"
            );
            assert!(!lab.0.join("a".repeat(32)).join("terminal.json").exists());
        }
    }

    #[test]
    #[ignore = "VM parent launches preparer, which launches waiting writer"]
    fn independent_writer_child() {
        use super::ordinary_update_receipts::OutcomeWriteRequest;
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let root = PathBuf::from(std::env::var_os("T02_WRITER_ROOT").unwrap());
        if std::env::var("T02_WRITER_ROLE").as_deref() == Ok("prepare") {
            drop(Outbox::fixture(&root, binding()).unwrap());
            let bytes = fs::read(root.join("a".repeat(32)).join("intent.json")).unwrap();
            let digest = hex::encode(sha2::Sha256::digest(bytes));
            let writer = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "persistence::independent_writer_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("T02_WRITER_ROLE", "write")
                .env("T02_WRITER_DIGEST", digest)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            fs::write(root.join("writer-pid"), writer.id().to_string()).unwrap();
            // Do not wait: the caller process dies before its independent writer runs.
            std::process::exit(0);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !root.join("release-writer").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture writer release timeout"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        OutcomeWriteRequest::parse(&[
            "a".repeat(32),
            std::env::var("T02_WRITER_DIGEST").unwrap(),
            "1603".into(),
        ])
        .unwrap()
        .fixture_write(&root)
        .unwrap();
        fs::write(root.join("writer-done"), b"1603").unwrap();
    }

    #[test]
    fn writer_survives_preparer_exit_and_publishes_1603_without_install_success() {
        let lab = Lab::new();
        let mut preparer = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "persistence::independent_writer_child",
                "--ignored",
                "--nocapture",
            ])
            .env("T02_WRITER_ROOT", &lab.0)
            .env("T02_WRITER_ROLE", "prepare")
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let preparer_pid = preparer.id();
        assert!(preparer.wait().unwrap().success());
        let writer_pid: u32 = fs::read_to_string(lab.0.join("writer-pid"))
            .unwrap()
            .parse()
            .unwrap();
        assert_ne!(writer_pid, preparer_pid);
        assert!(!lab.0.join("a".repeat(32)).join("outcome.json").exists());
        fs::write(lab.0.join("release-writer"), b"caller exit observed").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !lab.0.join("writer-done").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "independent outcome writer timeout"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let terminal = lab.open(binding()).reconcile(None).unwrap();
        assert_eq!(terminal.status, "failed");
        assert_eq!(terminal.error_code, "msi_exit_1603");
        assert!(lab
            .open(binding())
            .pending(Channel::Update)
            .unwrap()
            .is_some());
    }

    #[test]
    fn occupied_publication_target_returns_error_and_keeps_intent() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        let before = contents(&lab, "intent.json");
        let occupied = lab.0.join("a".repeat(32)).join("outcome.json");
        fs::create_dir(&occupied).unwrap();
        assert_eq!(
            outbox
                .record_outcome(InstallerOutcome::Succeeded { exit_code: 0 })
                .unwrap_err(),
            "workspace asset is not file"
        );
        assert_eq!(contents(&lab, "intent.json"), before);
        assert!(occupied.is_dir());
        assert!(!lab.0.join("a".repeat(32)).join("terminal.json").exists());
    }

    #[test]
    fn invalid_transaction_cannot_escape_fixed_attempt_directory() {
        let lab = Lab::new();
        let mut invalid = binding();
        invalid.transaction_id = "../another-attempt".into();
        assert_eq!(
            Outbox::fixture(&lab.0, invalid).err().unwrap(),
            "ordinary receipt binding invalid"
        );
        assert!(!lab.0.exists());
    }

    #[test]
    fn corrupted_ack_is_not_accepted_and_terminal_bytes_are_retained() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        outbox
            .record_outcome(InstallerOutcome::Failed {
                exit_code: 1603,
                error_code: "msi_1603".into(),
            })
            .unwrap();
        outbox.reconcile(None).unwrap();
        let before = contents(&lab, "terminal.json");
        let (_, digest) = outbox.pending(Channel::Update).unwrap().unwrap();
        outbox.acknowledge(Channel::Update, &digest).unwrap();
        let path = lab.0.join("a".repeat(32)).join("ack-update.json");
        let mut ack: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        ack["value"] = "wrong-digest".into();
        fs::write(&path, serde_json::to_vec(&ack).unwrap()).unwrap();
        assert_eq!(
            lab.open(binding()).pending(Channel::Update).unwrap_err(),
            "ordinary terminal acknowledgement mismatch"
        );
        assert_eq!(contents(&lab, "terminal.json"), before);
    }

    #[test]
    fn intent_reopens_and_conflicting_identity_preserves_original_bytes() {
        let lab = Lab::new();
        drop(lab.open(binding()));
        let bytes = contents(&lab, "intent.json");
        drop(lab.open(binding()));
        for field in ["identity", "target", "package", "instance"] {
            let mut other = binding();
            match field {
                "identity" => other.client_uuid.push('x'),
                "target" => other.to_build_seq += 1,
                "package" => other.package_sha256 = "d".repeat(64),
                _ => other.installation_id.push('x'),
            }
            assert_eq!(
                Outbox::fixture(&lab.0, other).err().unwrap(),
                "ordinary receipt immutable conflict"
            );
            assert_eq!(contents(&lab, "intent.json"), bytes);
        }
    }

    #[test]
    fn missing_outcome_or_health_never_becomes_installed_and_keeps_intent() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        let before = contents(&lab, "intent.json");
        assert_eq!(
            outbox.reconcile(None).unwrap_err(),
            "ordinary receipt missing: outcome.json"
        );
        outbox
            .record_outcome(InstallerOutcome::Succeeded { exit_code: 0 })
            .unwrap();
        assert_eq!(
            outbox.reconcile(None).unwrap_err(),
            "ordinary running target unconfirmed"
        );
        for field in ["identity", "target", "instance", "package"] {
            let mut changed = binding();
            match field {
                "identity" => changed.client_uuid.push('x'),
                "target" => changed.to_build_seq += 1,
                "instance" => changed.installation_id.push('x'),
                _ => changed.package_sha256 = "d".repeat(64),
            }
            assert_eq!(
                outbox
                    .reconcile(Some(&VerifiedRunningTarget { binding: changed }))
                    .unwrap_err(),
                "ordinary running target unconfirmed"
            );
        }
        assert_eq!(contents(&lab, "intent.json"), before);
        assert!(!lab.0.join("a".repeat(32)).join("terminal.json").exists());
    }

    #[test]
    fn durable_terminal_survives_reopen_lost_ack_and_channels_ack_independently() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        outbox
            .record_outcome(InstallerOutcome::Succeeded { exit_code: 3010 })
            .unwrap();
        assert_eq!(
            outbox
                .reconcile(Some(&VerifiedRunningTarget { binding: binding() }))
                .unwrap()
                .status,
            "installed"
        );
        let terminal = contents(&lab, "terminal.json");
        let pending = outbox.pending(Channel::Update).unwrap().unwrap();
        drop(outbox);
        let outbox = lab.open(binding());
        assert_eq!(
            outbox.pending(Channel::Update).unwrap(),
            Some(pending.clone())
        );
        assert_eq!(
            outbox.acknowledge(Channel::Update, "wrong").unwrap_err(),
            "ordinary terminal acknowledgement mismatch"
        );
        assert_eq!(contents(&lab, "terminal.json"), terminal);
        outbox.acknowledge(Channel::Update, &pending.1).unwrap();
        outbox.acknowledge(Channel::Update, &pending.1).unwrap();
        assert_eq!(outbox.pending(Channel::Update).unwrap(), None);
        assert_eq!(
            outbox.pending(Channel::Command).unwrap(),
            Some(pending.clone())
        );
        outbox.acknowledge(Channel::Command, &pending.1).unwrap();
        assert_eq!(lab.open(binding()).pending(Channel::Command).unwrap(), None);
        assert_eq!(contents(&lab, "terminal.json"), terminal);
    }

    #[test]
    fn failure_and_original_error_remain_after_ack_and_separate_successful_attempt() {
        let lab = Lab::new();
        let old = lab.open(binding());
        old.record_outcome(InstallerOutcome::Failed {
            exit_code: 1603,
            error_code: "msi_1603".into(),
        })
        .unwrap();
        let terminal = old.reconcile(None).unwrap();
        assert_eq!(terminal.status, "failed");
        assert_eq!(terminal.error_code, "msi_1603");
        let before = contents(&lab, "terminal.json");
        let (_, digest) = old.pending(Channel::Update).unwrap().unwrap();
        old.acknowledge(Channel::Update, &digest).unwrap();
        assert_eq!(
            old.record_outcome(InstallerOutcome::Succeeded { exit_code: 0 })
                .unwrap_err(),
            "ordinary receipt immutable conflict"
        );
        let mut next = binding();
        next.transaction_id = "e".repeat(32);
        let retry = lab.open(next.clone());
        retry
            .record_outcome(InstallerOutcome::Succeeded { exit_code: 0 })
            .unwrap();
        retry
            .reconcile(Some(&VerifiedRunningTarget { binding: next }))
            .unwrap();
        assert_eq!(contents(&lab, "terminal.json"), before);
        assert_eq!(lab.open(binding()).reconcile(None).unwrap(), terminal);
    }

    #[test]
    fn malformed_outcome_and_invalid_exit_preserve_intent_and_never_publish_terminal() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        let before = contents(&lab, "intent.json");
        assert_eq!(
            outbox
                .record_outcome(InstallerOutcome::Succeeded { exit_code: 1603 })
                .unwrap_err(),
            "ordinary installer success exit invalid"
        );
        assert_eq!(
            outbox
                .record_outcome(InstallerOutcome::Failed {
                    exit_code: 0,
                    error_code: "bad".into()
                })
                .unwrap_err(),
            "ordinary installer failure invalid"
        );
        fs::write(
            lab.0.join("a".repeat(32)).join("outcome.json"),
            b"{bad json",
        )
        .unwrap();
        assert_eq!(
            outbox.reconcile(None).unwrap_err(),
            "ordinary receipt malformed"
        );
        assert_eq!(contents(&lab, "intent.json"), before);
        assert!(!lab.0.join("a".repeat(32)).join("terminal.json").exists());
    }

    #[test]
    fn concurrent_same_outcome_publication_is_idempotent() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        let before_intent = contents(&lab, "intent.json");
        let (first, second) = run_barrier_writer_pair(&lab, 1603, 1603);
        assert!(first.success);
        assert!(second.success);
        assert_eq!(contents(&lab, "outcome.json"), first.bytes);
        assert_eq!(contents(&lab, "intent.json"), before_intent);
        assert_no_staging(&lab);
        drop(outbox);
    }

    #[test]
    fn concurrent_conflicting_outcome_keeps_first_bytes_and_cleans_loser() {
        let lab = Lab::new();
        let outbox = lab.open(binding());
        let before_intent = contents(&lab, "intent.json");
        let (first, second) = run_barrier_writer_pair(&lab, 1603, 1618);
        assert!(first.success);
        assert!(!second.success);
        assert_eq!(
            second.error.as_deref(),
            Some("ordinary receipt immutable conflict")
        );
        assert_eq!(contents(&lab, "outcome.json"), first.bytes);
        assert_eq!(contents(&lab, "intent.json"), before_intent);
        assert_no_staging(&lab);
        drop(outbox);
    }

    #[derive(Debug)]
    struct BarrierWriterResult {
        success: bool,
        error: Option<String>,
        bytes: Vec<u8>,
    }

    fn assert_no_staging(lab: &Lab) {
        assert!(!fs::read_dir(lab.0.join("a".repeat(32)))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("staging-")));
    }

    fn run_barrier_writer_pair(
        lab: &Lab,
        first_exit: u32,
        second_exit: u32,
    ) -> (BarrierWriterResult, BarrierWriterResult) {
        let test = std::env::var("T02_BARRIER_TEST")
            .unwrap_or_else(|_| "ordinary_writer_barrier_child".into());
        let mut children = Vec::new();
        for (role, exit) in [("a", first_exit), ("b", second_exit)] {
            let test_name = format!("persistence::{test}");
            children.push(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", &test_name, "--ignored", "--nocapture"])
                    .env("T02_RECEIPTS_ROOT", &lab.0)
                    .env("T02_WRITER_ROLE", role)
                    .env("T02_WRITER_EXIT", exit.to_string())
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
        }
        let mut a = children.remove(0);
        let mut b = children.remove(0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !lab.0.join("writer-a-ready").exists() || !lab.0.join("writer-b-ready").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "writer barrier timeout"
            );
            std::thread::yield_now();
        }
        fs::write(lab.0.join("writer-release-a"), b"go").unwrap();
        let a_status = a.wait().unwrap();
        assert!(a_status.success());
        fs::write(lab.0.join("writer-release-b"), b"go").unwrap();
        let b_status = b.wait().unwrap();
        let first_bytes = contents(lab, "outcome.json");
        let a_result = BarrierWriterResult {
            success: a_status.success(),
            error: None,
            bytes: first_bytes.clone(),
        };
        let b_result = BarrierWriterResult {
            success: b_status.success(),
            error: if b_status.success() {
                None
            } else {
                Some(fs::read_to_string(lab.0.join("writer-b-error")).unwrap())
            },
            bytes: first_bytes,
        };
        (a_result, b_result)
    }

    #[test]
    #[ignore = "spawned only by the concurrent writer parent"]
    fn ordinary_writer_barrier_child() {
        let root = PathBuf::from(std::env::var_os("T02_RECEIPTS_ROOT").unwrap());
        let role = std::env::var("T02_WRITER_ROLE").unwrap();
        let exit: u32 = std::env::var("T02_WRITER_EXIT").unwrap().parse().unwrap();
        let outbox = Outbox::fixture_open(&root, &"a".repeat(32)).unwrap();
        let result = outbox.record_outcome(InstallerOutcome::Failed {
            exit_code: exit,
            error_code: format!("msi_{exit}"),
        });
        if let Err(error) = result {
            fs::write(root.join(format!("writer-{role}-error")), error).unwrap();
            std::process::exit(1);
        }
    }
}

#[cfg(all(windows, t02_receipts, not(any(t02_projection, t02_wiring))))]
#[path = "../src/platform/windows_agent_guard.rs"]
mod lease_guard;

#[cfg(t02_wiring)]
mod wiring {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::{Path, PathBuf},
        sync::Mutex,
        time::Duration,
    };
    type ResultType<T> = Result<T, Box<dyn std::error::Error>>;
    static ENDPOINT: Mutex<String> = Mutex::new(String::new());
    const CHCP_RELATIVE_PATH: &str = "chcp.com";
    const UTF8_CODE_PAGE: u32 = 65001;
    const BATCH_CODE_PAGE_FAILURE_EXIT_CODE: u32 = 10001;
    const BATCH_OUTPUT_DIRECTORY_EXISTS_EXIT_CODE: u32 = 10002;
    const BATCH_OUTPUT_DIRECTORY_CREATE_FAILURE_EXIT_CODE: u32 = 10003;

    #[macro_export]
    macro_rules! t02_wiring_warn { ($($arg:tt)*) => {{ let _ = format_args!($($arg)*); }}; }
    mod log {
        pub(crate) use crate::t02_wiring_warn as warn;
    }

    struct Identity {
        client_id: String,
        client_uuid: String,
    }
    struct Auth {
        device_id: String,
        public_key: String,
        timestamp: String,
        nonce: String,
        signature: String,
    }
    fn current_update_client_identity() -> Identity {
        Identity {
            client_id: "local-fixture".into(),
            client_uuid: "local-fixture-uuid".into(),
        }
    }
    fn update_device_auth_headers(
        _: &str,
        _: &str,
        _: &Identity,
        _: &str,
        _: &[u8],
    ) -> ResultType<Auth> {
        Ok(Auth {
            device_id: "fixture".into(),
            public_key: "fixture".into(),
            timestamp: "1".into(),
            nonce: "fixture".into(),
            signature: "fixture".into(),
        })
    }
    struct LocalClient(reqwest::blocking::Client);
    impl LocalClient {
        fn post(&self, url: &str) -> reqwest::blocking::RequestBuilder {
            assert_eq!(url, "https://rdapi.yan.life/rd/update/v1/events");
            self.0.post(ENDPOINT.lock().unwrap().clone())
        }
    }
    fn create_http_client_with_url_strict(_: &str) -> ResultType<LocalClient> {
        Ok(LocalClient(
            reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()?,
        ))
    }
    fn get_system_executable(relative: &str) -> ResultType<PathBuf> {
        Ok(PathBuf::from(r"C:\Windows\System32").join(relative))
    }
    fn path_for_cmd_environment(path: &Path) -> ResultType<&str> {
        Ok(path.to_str().unwrap())
    }
    fn trusted_install_environment() -> ResultType<String> {
        Ok(String::new())
    }

    // These are complete, unchanged production functions; only the HTTP destination
    // and OS-environment discovery are fixture boundaries.
    include!(env!("T02_WIRING_SOURCE"));

    fn report_fixture(status: u16, body: &'static str) -> bool {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal"),
            "VM only"
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        *ENDPOINT.lock().unwrap() = format!(
            "http://{}/rd/update/v1/events",
            listener.local_addr().unwrap()
        );
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let count = socket.read(&mut chunk).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let size: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + size {
                        let payload: serde_json::Value =
                            serde_json::from_slice(&request[end + 4..end + 4 + size]).unwrap();
                        assert_eq!(payload["status"], "installed");
                        assert_eq!(payload["from_build_seq"], 2026100506u64);
                        assert_eq!(payload["to_build_seq"], 2026101501u64);
                        break;
                    }
                }
            }
            write!(socket, "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let reported = report_update_event_with_origin_and_error(
            "installed",
            "1.5.3",
            2026100506,
            "1.5.3",
            2026101501,
            "primary",
            "",
        );
        server.join().unwrap();
        reported
    }

    #[test]
    fn t02_http_201_php_ok_true_acknowledges() {
        assert!(report_fixture(201, r#"{"ok":true}"#));
    }
    #[test]
    fn t02_http_200_accepted_false_must_not_acknowledge() {
        assert!(
            !report_fixture(200, r#"{"accepted":false}"#),
            "2xx alone discarded a rejected terminal ACK"
        );
    }
    #[test]
    fn t02_http_201_ok_false_must_not_acknowledge() {
        assert!(
            !report_fixture(201, r#"{"ok":false}"#),
            "PHP negative ACK treated as delivered"
        );
    }
    #[test]
    fn t02_http_200_malformed_body_must_not_acknowledge() {
        assert!(
            !report_fixture(200, "not-json"),
            "unreadable ACK treated as delivered"
        );
    }
    #[test]
    fn t02_http_503_ok_true_must_not_acknowledge() {
        assert!(!report_fixture(503, r#"{"ok":true}"#));
    }

    #[test]
    fn t02_real_runner_preserves_child_exit_1603() {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal"),
            "VM only"
        );
        let error = update_me_msi("fixture.msi", true).unwrap_err();
        assert_eq!(error.to_string(), "fixture elevated exit code 1603");
    }

    #[test]
    fn t02_real_runner_accepts_child_exit_3010() {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal"),
            "VM only"
        );
        let result = update_me_msi("fixture-reboot.msi", true);
        assert!(
            result.is_ok(),
            "MSI 3010 is success requiring reboot, not rollback: {result:?}"
        );
    }

    fn run_cmds(commands: String, show: bool, tip: &str) -> ResultType<()> {
        assert!(!show);
        assert_eq!(tip, "update-msi");
        let directory = std::env::temp_dir().join(format!("t02-runner-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        // Current-directory executable is a fixed compiled fixture, not a real MSI.
        fs::write(
            directory.join("msiexec.exe"),
            include_bytes!(env!("T02_MSI_STUB")),
        )
        .unwrap();
        let script = directory.join("runner.bat");
        fs::write(&script, prepare_install_commands(&commands).unwrap()).unwrap();
        let result = std::process::Command::new(r"C:\Windows\System32\cmd.exe")
            .args(["/D", "/C"])
            .arg(&script)
            .current_dir(&directory)
            .output()
            .unwrap();
        let code = result.status.code();
        fs::remove_dir_all(&directory).unwrap();
        #[cfg(t02_msi_reboot)]
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("T02_MSI_RAW_EXIT="),
            "fixture executable did not reach its validated exit; code={code:?}"
        );
        match code {
            Some(0) => Ok(()),
            Some(code) => Err(format!("fixture elevated exit code {code}").into()),
            None => Err("fixture runner missing exit code".into()),
        }
    }

    #[test]
    fn t02_http_201_missing_ok_must_not_acknowledge() {
        assert!(!report_fixture(201, "{}"));
    }
    #[test]
    fn t02_http_201_string_ok_must_not_acknowledge() {
        assert!(!report_fixture(201, r#"{"ok":"true"}"#));
    }
}

#[cfg(t02_wiring)]
pub const PRODUCT: &str = "rustdesk";
#[cfg(t02_wiring)]
pub const EDITION: &str = "standard";
#[cfg(t02_wiring)]
pub const BUILD_NUMBER: &str = "fixture";
#[cfg(t02_wiring)]
pub const CHANNEL: &str = "stable";
#[cfg(t02_wiring)]
pub const SOURCE_COMMIT: &str = "fixture";

#[cfg(t02_preparation)]
pub const VERSION: &str = "1.5.3";
#[cfg(t02_preparation)]
pub const BUILD_SEQ: u64 = 2026100506;

#[cfg(t02_preparation)]
mod preparation {
    use super::ordinary_update_receipts as receipts;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    const INSTALLED: &str = "writer-source.exe";
    fn fixture_current_exe() -> std::io::Result<PathBuf> {
        Ok(INSTALLED.into())
    }
    fn root() -> PathBuf {
        std::env::var_os("T02_PREPARATION_ROOT").unwrap().into()
    }
    fn mode() -> String {
        std::env::var("T02_PREPARATION_MODE").unwrap()
    }
    fn install_allowed() -> Result<(), String> {
        Ok(())
    }
    fn instance() -> Result<String, String> {
        Ok("b".repeat(32))
    }
    fn owns_command(_: &str) -> bool {
        false
    }
    mod install_guard {
        pub struct InstallGuard;
        impl InstallGuard {
            pub fn try_acquire() -> std::io::Result<Option<Self>> {
                #[cfg(t02_admission_gate)]
                if let Some(after) = std::env::var_os("T02_GATE_AFTER_LOCK") {
                    // Deterministic discovery boundary: no package or registry mutation.
                    std::env::set_var("T02_GATE_CACHED", after);
                    std::fs::write(
                        super::root().join("guard-acquired"),
                        b"cache selection changed",
                    )?;
                }
                Ok(Some(Self))
            }
        }
    }
    struct Outbox {
        actual: receipts::Outbox,
        directory: PathBuf,
        binding: receipts::Binding,
    }
    impl Outbox {
        fn preparation_failed(&self, error: receipts::PreparationError) -> Result<(), String> {
            self.actual.preparation_failed(error)
        }
        fn blocks_preparation_retry(&self) -> bool {
            self.actual.blocks_preparation_retry()
        }
        fn prepare(binding: receipts::Binding) -> Result<Self, String> {
            #[cfg(t02_admission_gate)]
            if mode().starts_with("gate-") {
                fs::create_dir_all(root()).map_err(|e| e.to_string())?;
                fs::write(
                    root().join("intent-boundary-entered"),
                    b"before intent creation",
                )
                .map_err(|e| e.to_string())?;
                return Err("fixture_intent_boundary".into());
            }
            let directory = root().join(&binding.transaction_id);
            let actual = receipts::Outbox::fixture(&root(), binding.clone())?;
            Ok(Self {
                actual,
                directory,
                binding,
            })
        }
        fn open_existing(_: &str) -> Result<Self, String> {
            Err("no existing fixture transaction".into())
        }
        fn dispatch(
            &self,
            run: impl FnOnce(&str, &str) -> Result<(), String>,
        ) -> Result<(), String> {
            self.actual.dispatch(run)
        }
    }
    mod workspace {
        use super::*;
        pub fn create(path: &Path) -> Result<(), String> {
            fs::create_dir(path).map_err(|e| e.to_string())?;
            if mode() == "occupied" {
                fs::write(
                    path.join("ordinary-msi-writer.exe"),
                    b"old protected writer",
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        pub fn verify_restore_target(_: &Path, _: bool) -> Result<(), String> {
            Ok(())
        }
        pub fn verify_file(_: &Path) -> Result<(), String> {
            Ok(())
        }
    }
    // Complete current production install/copy functions; no simulated failure handler.
    // Earlier root scan is redirected into this fixture through its ROOT constant.
    const ROOT: &str = "T02_UNUSED_NO_EXISTING_ROOT";
    use receipts::{Binding, PreparationError};
    mod supervisor {
        use super::*;
        pub fn seal_runtime(_: &Outbox) -> Result<(), String> {
            panic!("copy failure must occur first")
        }
        pub fn launch(_: &Outbox, _: install_guard::InstallGuard) -> Result<(), String> {
            panic!("must not launch")
        }
    }
    mod admission {
        use super::*;
        pub fn check_installed(target: &Path) -> Result<(), String> {
            #[cfg(t02_admission_gate)]
            {
                let cached = PathBuf::from(std::env::var_os("T02_GATE_CACHED").unwrap());
                let result = receipts::admission::check_pair(target, &cached);
                if std::env::var_os("T02_GATE_AFTER_LOCK").is_some() {
                    use std::io::Write;
                    fs::create_dir_all(root()).unwrap();
                    let mut log = fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(root().join("gate-checks"))
                        .unwrap();
                    writeln!(
                        log,
                        "{}:{}",
                        if root().join("guard-acquired").exists() {
                            "after"
                        } else {
                            "before"
                        },
                        result
                            .as_ref()
                            .err()
                            .map(String::as_str)
                            .unwrap_or("disabled")
                    )
                    .unwrap();
                }
                result
            }
            #[cfg(not(t02_admission_gate))]
            {
                let _ = target;
                Ok(())
            }
        }
    }
    mod runtime_projection {
        use super::*;
        use crate::preparation_hbb as hbb_common;
        use std::io::{Read, Write};
        include!(env!("T02_PREPARATION_SOURCE"));
    }

    #[test]
    #[ignore = "fixture parent only"]
    fn preparation_child() {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let package = root().with_extension("input");
        fs::write(&package, b"fixture package").unwrap();
        fs::write(INSTALLED, b"unchanged fixture writer bytes").unwrap();
        let _lock = if mode() == "locked" {
            use std::os::windows::fs::OpenOptionsExt;
            Some(
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(INSTALLED)
                    .unwrap(),
            )
        } else {
            None
        };
        let result = runtime_projection::install(
            &package,
            &"c".repeat(64),
            "windows-x86_64-msi-standard",
            "1.5.3",
            2026101501,
            "primary",
            None,
        );
        assert!(result.is_err(), "fixture must fail before installer");
        fs::write(root().join("observed-error.txt"), result.unwrap_err()).unwrap();
        fs::remove_file(package).unwrap();
    }

    fn exercise(mode: &str) {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let directory = std::env::temp_dir().join(format!(
            "t02-preparation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let work = directory.with_extension("work");
        fs::create_dir(&work).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "preparation::preparation_child",
                "--ignored",
                "--nocapture",
            ])
            .env("T02_PREPARATION_ROOT", &directory)
            .env("T02_PREPARATION_MODE", mode)
            .current_dir(&work)
            .status()
            .unwrap();
        assert!(status.success(), "fixture setup/child failed");
        let tx = directory.join("a".repeat(32));
        let before = fs::read(tx.join("intent.json")).unwrap();
        let intent: serde_json::Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(intent["binding"]["transaction_id"], "a".repeat(32));
        assert!(!tx.join("dispatch.json").exists());
        assert!(!tx.join("outcome.json").exists());
        assert!(!directory.join("installer-entered").exists());
        assert_eq!(
            fs::read(work.join(INSTALLED)).unwrap(),
            b"unchanged fixture writer bytes"
        );
        if mode == "occupied" {
            assert_eq!(
                fs::read(tx.join("runtime/ordinary-msi-writer.exe")).unwrap(),
                b"old protected writer"
            );
        }
        let error = fs::read_to_string(directory.join("observed-error.txt")).unwrap();
        assert!(!error.is_empty());
        let failure = fs::read(tx.join("preparation-failed.json"));
        assert_eq!(fs::read(tx.join("intent.json")).unwrap(), before);
        // Capture assertions before scoped cleanup, including the missing-record RED.
        let failure = failure
            .ok()
            .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok());
        let reopened = receipts::Outbox::fixture_open(&directory, &"a".repeat(32)).unwrap();
        assert!(reopened.preparation_retryable().unwrap());
        assert!(!reopened.blocks_preparation_retry());
        let oldfailure = fs::read(tx.join("preparation-failed.json")).unwrap();
        let mut next = reopened.binding().clone();
        next.transaction_id = "d".repeat(32);
        let retry = receipts::Outbox::fixture(&directory, next).unwrap();
        assert!(retry.blocks_preparation_retry());
        assert_eq!(
            fs::read(tx.join("preparation-failed.json")).unwrap(),
            oldfailure
        );
        assert_eq!(fs::read(tx.join("intent.json")).unwrap(), before);
        fs::remove_dir_all(&directory).unwrap();
        fs::remove_dir_all(&work).unwrap();
        let failure=failure.expect("BUSINESS RED: undispatched preparation error left immortal intent without preparation-failed receipt");
        assert_eq!(failure["value"]["phase"], "preparation_failed");
        assert_eq!(
            failure["value"]["error"]["kind"],
            if mode == "occupied" {
                "already_exists"
            } else {
                "sharing_violation"
            }
        );
        assert_eq!(failure["value"]["error"]["stage"], "copy_writer");
        assert_eq!(failure["value"]["error"]["code"], error);
        assert!(failure["value"]["error"]["os_code"].is_number());
    }
    #[test]
    fn occupied_writer_destination_records_retryable_preparation_failure() {
        exercise("occupied");
    }
    #[test]
    fn locked_writer_source_records_retryable_preparation_failure() {
        exercise("locked");
    }
    #[cfg(t02_admission_gate)]
    fn admission_rows(package: &Path, table: &str, key: &str, value: &str) -> Result<usize, u32> {
        #[link(name = "msi")]
        extern "system" {
            fn MsiOpenDatabaseW(path: *const u16, mode: *const u16, db: *mut u32) -> u32;
            fn MsiDatabaseOpenViewW(db: u32, query: *const u16, view: *mut u32) -> u32;
            fn MsiViewExecute(view: u32, record: u32) -> u32;
            fn MsiViewFetch(view: u32, record: *mut u32) -> u32;
            fn MsiCloseHandle(handle: u32) -> u32;
        }
        use std::os::windows::ffi::OsStrExt;
        let path: Vec<u16> = package.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut db = 0;
        let status = unsafe { MsiOpenDatabaseW(path.as_ptr(), std::ptr::null(), &mut db) };
        if status != 0 {
            return Err(status);
        }
        let query: Vec<u16> = format!("SELECT `{key}` FROM `{table}` WHERE `{key}`='{value}'")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut view = 0;
        let mut status = unsafe { MsiDatabaseOpenViewW(db, query.as_ptr(), &mut view) };
        let mut count = 0;
        if status == 0 {
            status = unsafe { MsiViewExecute(view, 0) };
            if status == 0 {
                loop {
                    let mut row = 0;
                    status = unsafe { MsiViewFetch(view, &mut row) };
                    if status == 259 {
                        status = 0;
                        break;
                    }
                    if status != 0 {
                        break;
                    }
                    count += 1;
                    unsafe { MsiCloseHandle(row) };
                }
            }
            unsafe { MsiCloseHandle(view) };
        }
        unsafe { MsiCloseHandle(db) };
        if status == 0 {
            Ok(count)
        } else {
            Err(status)
        }
    }

    #[cfg(t02_admission_gate)]
    fn gate_case(case: &str, target: &str, cached: &str, expected: &str) {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let inputs = PathBuf::from(std::env::var_os("T02_ADMISSION_INPUTS").unwrap());
        for name in [target, cached] {
            let path = inputs.join(name);
            if name == "missing.msi" {
                assert!(!path.exists());
                continue;
            }
            let rows = admission_rows(
                &path,
                "InstallExecuteSequence",
                "Action",
                "CheckAgentInstallAdmission",
            )
            .unwrap();
            assert_eq!(rows, usize::from(name == "enabled.msi"));
            let marker = admission_rows(
                &path,
                "Property",
                "Property",
                "RUSTDESK_MSI_ADMISSION_SCHEMA",
            )
            .unwrap();
            assert_eq!(marker, usize::from(name == "enabled.msi"));
        }
        let directory =
            std::env::temp_dir().join(format!("t02-gate-{}-{case}", std::process::id()));
        let work = directory.with_extension("work");
        fs::create_dir(&work).unwrap();
        let target_path = inputs.join(target);
        let cached_path = inputs.join(cached);
        let before: Vec<_> = [&target_path, &cached_path]
            .iter()
            .map(|p| fs::read(p).ok())
            .collect();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "preparation::gate_child",
                "--ignored",
                "--nocapture",
            ])
            .env("T02_PREPARATION_ROOT", &directory)
            .env("T02_PREPARATION_MODE", format!("gate-{case}"))
            .env("T02_GATE_TARGET", &target_path)
            .env("T02_GATE_CACHED", &cached_path)
            .current_dir(&work)
            .status()
            .unwrap();
        assert!(status.success(), "fixture setup failed");
        let result = fs::read_to_string(directory.join("gate-result")).unwrap();
        let entered = directory.join("intent-boundary-entered").exists();
        assert!(
            !directory.join("a".repeat(32)).exists(),
            "fixture must stop before durable intent"
        );
        assert_eq!(
            [&target_path, &cached_path]
                .iter()
                .map(|p| fs::read(p).ok())
                .collect::<Vec<_>>(),
            before
        );
        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(work).unwrap();
        assert_eq!(
            result, expected,
            "BUSINESS RED: ordinary install did not reject package capability before intent"
        );
        assert_eq!(entered, expected == "fixture_intent_boundary");
    }
    #[cfg(t02_admission_gate)]
    #[test]
    #[ignore = "parent only"]
    fn gate_child() {
        let target = PathBuf::from(std::env::var_os("T02_GATE_TARGET").unwrap());
        let result = runtime_projection::install(
            &target,
            &"c".repeat(64),
            "windows-x86_64-msi-standard",
            "1.5.3",
            2026101501,
            "primary",
            None,
        );
        fs::create_dir_all(root()).unwrap();
        fs::write(root().join("gate-result"), result.unwrap_err()).unwrap();
    }
    #[cfg(t02_admission_gate)]
    #[test]
    fn gate_target_enabled_refused_before_intent() {
        gate_case(
            "target",
            "enabled.msi",
            "standard1501.msi",
            "unsupported_admission",
        );
    }
    #[cfg(t02_admission_gate)]
    #[test]
    fn gate_cached_enabled_refused_before_intent() {
        gate_case(
            "cached",
            "standard1501.msi",
            "enabled.msi",
            "unsupported_admission",
        );
    }
    #[cfg(t02_admission_gate)]
    #[test]
    fn gate_missing_target_metadata_refused_before_intent() {
        gate_case(
            "missing-target",
            "missing.msi",
            "standard1501.msi",
            "admission_metadata_unavailable",
        );
    }
    #[cfg(t02_admission_gate)]
    #[test]
    fn gate_missing_cached_metadata_refused_before_intent() {
        gate_case(
            "missing-cached",
            "standard1501.msi",
            "missing.msi",
            "admission_metadata_unavailable",
        );
    }
    #[cfg(t02_admission_gate)]
    #[test]
    fn gate_formal1501_disabled_reaches_intent_boundary() {
        gate_case(
            "disabled",
            "standard1501.msi",
            "sos1501.msi",
            "fixture_intent_boundary",
        );
    }
    #[cfg(t02_admission_gate)]
    fn gate_lock_change(after: &str, expected: &str) {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let inputs = PathBuf::from(std::env::var_os("T02_ADMISSION_INPUTS").unwrap());
        let target = inputs.join("standard1501.msi");
        let cached = inputs.join("sos1501.msi");
        let changed = inputs.join(after);
        assert_eq!(
            admission_rows(
                &cached,
                "InstallExecuteSequence",
                "Action",
                "CheckAgentInstallAdmission"
            )
            .unwrap(),
            0
        );
        if after == "enabled.msi" {
            assert_eq!(
                admission_rows(
                    &changed,
                    "InstallExecuteSequence",
                    "Action",
                    "CheckAgentInstallAdmission"
                )
                .unwrap(),
                1
            );
        } else {
            assert!(!changed.exists());
        }
        let before: Vec<_> = [&target, &cached, &changed]
            .iter()
            .map(|p| fs::read(p).ok())
            .collect();
        let root =
            std::env::temp_dir().join(format!("t02-lock-gate-{}-{}", std::process::id(), after));
        let work = root.with_extension("work");
        fs::create_dir(&work).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "preparation::gate_child",
                "--ignored",
                "--nocapture",
            ])
            .env("T02_PREPARATION_ROOT", &root)
            .env("T02_PREPARATION_MODE", "gate-lock-change")
            .env("T02_GATE_TARGET", &target)
            .env("T02_GATE_CACHED", &cached)
            .env("T02_GATE_AFTER_LOCK", &changed)
            .current_dir(&work)
            .status()
            .unwrap();
        assert!(status.success(), "fixture child setup failed");
        let checks = fs::read_to_string(root.join("gate-checks")).unwrap();
        let result = fs::read_to_string(root.join("gate-result")).unwrap();
        let entered = root.join("intent-boundary-entered").exists();
        assert!(root.join("guard-acquired").exists());
        assert!(
            checks.starts_with("before:disabled\n"),
            "early check did not pass: {checks}"
        );
        assert!(!root.join("a".repeat(32)).exists());
        assert_eq!(
            [&target, &cached, &changed]
                .iter()
                .map(|p| fs::read(p).ok())
                .collect::<Vec<_>>(),
            before
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(work).unwrap();
        assert_eq!(
            result, expected,
            "BUSINESS RED: changed cache after lease acquisition escaped recheck; checks={checks}"
        );
        assert!(!entered, "intent boundary entered after cache changed");
        assert_eq!(checks, format!("before:disabled\nafter:{expected}\n"));
    }
    #[cfg(t02_admission_gate)]
    #[test]
    fn gate_lock_enabled_cache_refused_before_intent() {
        gate_lock_change("enabled.msi", "unsupported_admission");
    }
    #[cfg(t02_admission_gate)]
    #[test]
    fn gate_lock_missing_cache_refused_before_intent() {
        gate_lock_change("missing.msi", "admission_metadata_unavailable");
    }
    #[test]
    fn failed_failure_receipt_write_stays_pending_and_preserves_original_data() {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let directory =
            std::env::temp_dir().join(format!("t02-failure-write-{}", std::process::id()));
        let binding = receipts::Binding {
            transaction_id: "a".repeat(32),
            client_id: "fixture".into(),
            client_uuid: "uuid".into(),
            installation_id: "b".repeat(32),
            target_key: "windows-x86_64-msi-standard".into(),
            package_sha256: "c".repeat(64),
            from_version: "1.5.3".into(),
            from_build_seq: 1,
            to_version: "1.5.3".into(),
            to_build_seq: 2,
            source: "primary".into(),
            command_id: Some("same-command".into()),
        };
        let outbox = receipts::Outbox::fixture(&directory, binding).unwrap();
        let tx = directory.join("a".repeat(32));
        let before = fs::read(tx.join("intent.json")).unwrap();
        fs::create_dir(tx.join("preparation-failed.json")).unwrap();
        assert_eq!(
            outbox
                .preparation_failed(receipts::PreparationError::refused("copy_writer"))
                .unwrap_err(),
            "ordinary preparation failure persistence unconfirmed"
        );
        let reopened = receipts::Outbox::fixture_open(&directory, &"a".repeat(32)).unwrap();
        assert!(reopened.preparation_retryable().is_err());
        assert!(reopened.blocks_preparation_retry());
        assert_eq!(fs::read(tx.join("intent.json")).unwrap(), before);
        assert!(tx.join("preparation-failed.json").is_dir());
        assert!(!tx.join("outcome.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn dispatched_unknown_retains_intent_and_never_fabricates_preparation_failure() {
        assert_eq!(
            std::env::var("RUSTDESK_T02_VM").as_deref(),
            Ok("ordinary-terminal")
        );
        let directory = std::env::temp_dir().join(format!("t02-dispatched-{}", std::process::id()));
        let binding = receipts::Binding {
            transaction_id: "a".repeat(32),
            client_id: "fixture".into(),
            client_uuid: "uuid".into(),
            installation_id: "b".repeat(32),
            target_key: "windows-x86_64-msi-standard".into(),
            package_sha256: "c".repeat(64),
            from_version: "1.5.3".into(),
            from_build_seq: 1,
            to_version: "1.5.3".into(),
            to_build_seq: 2,
            source: "primary".into(),
            command_id: None,
        };
        let outbox = receipts::Outbox::fixture(&directory, binding).unwrap();
        let tx = directory.join("a".repeat(32));
        let before = fs::read(tx.join("intent.json")).unwrap();
        // Frozen post-dispatch crash state: no installer is run by this test.
        fs::write(tx.join("dispatch.json"), outbox.intent_digest().unwrap()).unwrap();
        let reopened = receipts::Outbox::fixture_open(&directory, &"a".repeat(32)).unwrap();
        assert_eq!(
            reopened.reconcile(None).unwrap_err(),
            "ordinary receipt missing: outcome.json"
        );
        assert_eq!(fs::read(tx.join("intent.json")).unwrap(), before);
        assert!(tx.join("dispatch.json").exists());
        for name in ["outcome.json", "terminal.json", "preparation-failed.json"] {
            assert!(!tx.join(name).exists());
        }
        assert_eq!(
            reopened
                .preparation_failed(receipts::PreparationError::refused("copy_writer"))
                .unwrap_err(),
            "ordinary preparation failure after dispatch refused"
        );
        assert!(reopened.blocks_preparation_retry());
        fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(t02_preparation)]
pub fn current_update_client_identity() -> preparation_identity::Identity {
    preparation_identity::Identity {
        client_id: "fixture-id".into(),
        client_uuid: "fixture-uuid".into(),
    }
}
#[cfg(t02_preparation)]
pub mod preparation_identity {
    pub struct Identity {
        pub client_id: String,
        pub client_uuid: String,
    }
}
#[cfg(t02_preparation)]
pub fn new_update_transaction_id() -> String {
    "a".repeat(32)
}
#[cfg(t02_preparation)]
pub fn report_update_event(_: &str, _: &str, _: u64, _: &str) {
    panic!("preparation error must precede installing event")
}
#[cfg(t02_preparation)]
mod platform {
    pub mod windows {
        pub fn update_me_msi_receipt(_: &str, _: &str) -> Result<(), String> {
            panic!("installer must not run")
        }
    }
}
#[cfg(t02_preparation)]
mod preparation_hbb {
    pub mod config {
        pub fn is_disable_installation() -> bool {
            false
        }
    }
    pub mod log {
        pub(crate) use crate::preparation_warn as warn;
    }
}
#[cfg(t02_preparation)]
#[macro_export]
macro_rules! preparation_warn {($($arg:tt)*)=>{{let _=format_args!($($arg)*);}};}
