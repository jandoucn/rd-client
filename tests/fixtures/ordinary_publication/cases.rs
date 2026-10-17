#![cfg(windows)]

#[path = "../../../src/updater/ordinary_update_receipts.rs"]
mod ordinary_update_receipts;

use ordinary_update_receipts::{Binding, InstallerOutcome, Outbox};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn binding() -> Binding {
    Binding {
        transaction_id: "a".repeat(32),
        client_id: "local-fixture".into(),
        client_uuid: "local-fixture-uuid".into(),
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

struct Lab(PathBuf);

impl Lab {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ordinary-receipt-publication-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        Self(path)
    }

    fn open(&self) -> Outbox {
        Outbox::fixture(&self.0, binding()).unwrap()
    }

    fn intent(&self) -> Vec<u8> {
        fs::read(self.0.join("a".repeat(32)).join("intent.json")).unwrap()
    }

    fn staging_empty(&self) {
        assert!(!fs::read_dir(self.0.join("a".repeat(32)))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("staging-")));
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

struct ChildWriter {
    role: &'static str,
    child: Option<Child>,
}

fn spawn(lab: &Lab, role: &'static str, exit: u32) -> ChildWriter {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "writer_child", "--ignored", "--nocapture"])
        .env("T02_PUBLICATION_ROOT", &lab.0)
        .env("T02_PUBLICATION_ROLE", role)
        .env("T02_PUBLICATION_EXIT", exit.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    ChildWriter {
        role,
        child: Some(child),
    }
}

fn wait_ready(lab: &Lab) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !(lab.0.join("writer-a-ready").is_file() && lab.0.join("writer-b-ready").is_file()) {
        assert!(
            Instant::now() < deadline,
            "both children must reach publication barrier"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn finish(writer: &mut ChildWriter) -> Output {
    writer
        .child
        .take()
        .expect("writer child already joined")
        .wait_with_output()
        .unwrap_or_else(|error| panic!("writer {} failed to join: {error}", writer.role))
}

fn run_serialized_pair(first_exit: u32, second_exit: u32) -> (Vec<u8>, Output) {
    assert_eq!(
        std::env::var("RUSTDESK_T02_VM").as_deref(),
        Ok("ordinary-terminal"),
        "Win10 owner only"
    );
    let lab = Lab::new();
    let outbox = lab.open();
    let intent_before = lab.intent();
    let mut first = spawn(&lab, "a", first_exit);
    let mut second = spawn(&lab, "b", second_exit);

    wait_ready(&lab);
    assert!(
        !lab.0.join("a".repeat(32)).join("outcome.json").exists(),
        "barrier holds both publishers before rename"
    );
    fs::write(lab.0.join("writer-release-a"), b"release A").unwrap();
    let first_output = finish(&mut first);
    assert!(
        first_output.status.success(),
        "first writer stdout={} stderr={}",
        String::from_utf8_lossy(&first_output.stdout),
        String::from_utf8_lossy(&first_output.stderr)
    );
    let winner = fs::read(lab.0.join("a".repeat(32)).join("outcome.json")).unwrap();
    fs::write(lab.0.join("writer-release-b"), b"release B").unwrap();
    let second_output = finish(&mut second);

    assert_eq!(
        fs::read(lab.0.join("a".repeat(32)).join("outcome.json")).unwrap(),
        winner,
        "winner bytes are immutable"
    );
    assert_eq!(
        lab.intent(),
        intent_before,
        "original intent bytes remain unchanged"
    );
    lab.staging_empty();
    let terminal = outbox.reconcile(None).unwrap();
    assert_eq!(terminal.status, "failed");
    assert_eq!(terminal.error_code, format!("msi_exit_{first_exit}"));
    (winner, second_output)
}

#[test]
fn same_outcome_second_writer_is_idempotent_after_first_exit() {
    let (_, second) = run_serialized_pair(1603, 1603);
    assert!(
        second.status.success(),
        "same outcome should be idempotent: {}",
        String::from_utf8_lossy(&second.stderr)
    );
}

#[test]
fn conflicting_second_writer_fails_after_first_exit_and_preserves_winner() {
    let (_, second) = run_serialized_pair(1603, 1618);
    assert!(!second.status.success(), "conflicting outcome must fail");
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("ordinary receipt immutable conflict"),
        "conflict error missing: {}",
        String::from_utf8_lossy(&second.stderr)
    );
}

#[test]
#[ignore = "child process launched by the deterministic parent tests"]
fn writer_child() {
    let root = PathBuf::from(std::env::var_os("T02_PUBLICATION_ROOT").unwrap());
    let role = std::env::var("T02_PUBLICATION_ROLE").unwrap();
    let exit: u32 = std::env::var("T02_PUBLICATION_EXIT")
        .unwrap()
        .parse()
        .unwrap();
    let outbox = Outbox::fixture_open(&root, &"a".repeat(32)).unwrap();
    fs::write(root.join(format!("writer-{role}-ready")), b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !root.join(format!("writer-release-{role}")).exists() {
        assert!(Instant::now() < deadline, "writer {role} release timeout");
        thread::sleep(Duration::from_millis(5));
    }
    outbox
        .record_outcome(InstallerOutcome::Failed {
            exit_code: exit,
            error_code: format!("msi_exit_{exit}"),
        })
        .unwrap_or_else(|error| panic!("writer {role}: {error}"));
}
