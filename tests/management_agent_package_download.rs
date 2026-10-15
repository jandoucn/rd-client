#![cfg(windows)]
#[path = "../src/management_agent/package_download.rs"]
mod package_download;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;
#[path = "../src/management_agent/workspace.rs"]
mod workspace;

use hbb_common::UpdateTarget;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

struct Server {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(first_status: u16, first_body: &'static [u8], declared: usize) -> Self {
        Self::with_occupied_cache(first_status, first_body, declared, None)
    }

    fn with_occupied_cache(
        first_status: u16,
        first_body: &'static [u8],
        declared: usize,
        cache: Option<PathBuf>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&stop);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                    assert!(header.len() < 8192);
                }
                let request = String::from_utf8(header).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                observed.lock().unwrap().push(path.clone());
                if path == "/disconnect.msi" {
                    continue;
                }
                if path.starts_with("/wait-") {
                    thread::sleep(Duration::from_millis(65));
                    let status = if path == "/wait-redirect.msi" {
                        302
                    } else {
                        503
                    };
                    let _=write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Length: 0\r\nLocation: http://{address}/mirror.msi\r\nConnection: close\r\n\r\n");
                    continue;
                }
                if path.starts_with("/chunk-") {
                    let body = if path == "/chunk-over.msi" {
                        "5\r\nevil!\r\n0\r\n\r\n"
                    } else {
                        "3\r\nbad\r\n"
                    };
                    write!(stream,"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{body}").unwrap();
                    continue;
                }
                if path == "/slow.msi" {
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nb"
                    )
                    .unwrap();
                    stream.flush().unwrap();
                    thread::sleep(Duration::from_millis(350));
                    let _ = stream.write_all(b"ad!");
                    continue;
                }
                if path == "/mirror.msi" {
                    if let Some(cache) = &cache {
                        std::fs::write(cache, b"previous package").unwrap();
                    }
                }
                let (status, body, length) = if path == "/mirror.msi" {
                    (200, b"good".as_slice(), 4)
                } else {
                    (first_status, first_body, declared)
                };
                write!(stream, "HTTP/1.1 {status} Fixture\r\nContent-Length: {length}\r\nLocation: http://{address}/mirror.msi\r\nConnection: close\r\n\r\n").unwrap();
                stream.write_all(body).unwrap();
            }
        });
        Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    fn paths(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn root(label: &str) -> PathBuf {
    assert!(matches!(
        std::env::var("COMPUTERNAME").unwrap().as_str(),
        "RDLAB-WIN10" | "RDLAB-WIN11"
    ));
    std::env::temp_dir().join(format!(
        "agent-mirror-{label}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

fn target(mirrors: &[&str]) -> UpdateTarget {
    UpdateTarget {
        primary: "https://download.yan.life/rustdesk/stable/1.5.3/original.msi".into(),
        mirrors: mirrors.iter().map(|value| (*value).to_owned()).collect(),
        size: 4,
        sha256: hex::encode(Sha256::digest(b"good")),
        ..Default::default()
    }
}

const MIRROR: &str = "https://download.yan.life/rustdesk/stable/mirror/mirror.msi";

#[test]
fn approved_mirror_recovers_http_integrity_truncation_and_redirect_failures() {
    for (label, status, body, declared) in [
        ("http", 503, b"fail".as_slice(), 4),
        ("hash", 200, b"evil".as_slice(), 4),
        ("short", 200, b"bad".as_slice(), 4),
        ("size", 200, b"bad".as_slice(), 3),
        ("redirect", 302, b"fail".as_slice(), 4),
    ] {
        let directory = root(label);
        let server = Server::new(status, body, declared);
        let result =
            package_download::download_fixture(&directory, &target(&[MIRROR]), server.address);
        assert!(result.is_ok(), "{label}: {:?}", result.as_ref().err());
        let (path, locked) = result.unwrap();
        assert_eq!(
            path.file_name().unwrap(),
            "original.msi",
            "MSI registered source basename must survive mirror selection"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"good");
        assert!(std::fs::write(&path, b"evil").is_err());
        assert_eq!(server.paths(), ["/original.msi", "/mirror.msi"]);
        drop(locked);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn primary_success_and_verified_cache_never_contact_mirrors() {
    let directory = root("primary");
    let server = Server::new(200, b"good", 4);
    let expected = target(&[MIRROR]);
    let (path, locked) =
        package_download::download_fixture(&directory, &expected, server.address).unwrap();
    drop(locked);
    let (cached, locked) =
        package_download::download_fixture(&directory, &expected, server.address).unwrap();
    assert_eq!(path, cached);
    assert_eq!(server.paths(), ["/original.msi"]);
    drop(locked);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unapproved_mirrors_and_duplicates_are_never_requested() {
    let invalid = [
        "https://other.invalid/rustdesk/stable/foreign.msi",
        "http://download.yan.life/rustdesk/stable/insecure.msi",
        "https://download.yan.life/rustdesk/stable/../escape.msi",
        "https://download.yan.life/rustdesk/stable/%2e%2e/escape.msi",
        "https://download.yan.life/rustdesk/stable/mirror.msi?redirect=1",
        "https://download.yan.life/rustdesk/stable/mirror.msi#fragment",
        "https://download.yan.life:443/rustdesk/stable/port.msi",
        "https://user@download.yan.life/rustdesk/stable/user.msi",
        "https://download.yan.life.evil.invalid/rustdesk/stable/foreign.msi",
        "https://download.yan.life/rustdesk/stable//empty.msi",
        "https://download.yan.life/rustdesk/stable/back\\slash.msi",
        "https://download.yan.life/rustdesk/stable/wrong.exe",
        "https://download.yan.life/rustdesk/stable/CON.msi",
        "https://download.yan.life/rustdesk/stable/line\nbreak.msi",
    ];
    for batch in invalid.chunks(5) {
        let directory = root("sources");
        let server = Server::new(503, b"fail", 4);
        let mut sources = batch.to_vec();
        sources.extend([
            "https://download.yan.life/rustdesk/stable/1.5.3/original.msi",
            MIRROR,
            MIRROR,
        ]);
        let expected = target(&sources);
        let (path, locked) =
            package_download::download_fixture(&directory, &expected, server.address).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"good");
        assert_eq!(server.paths(), ["/original.msi", "/mirror.msi"]);
        drop(locked);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn all_failed_sources_preserve_prior_cache_and_do_not_publish_invalid_bytes() {
    let directory = root("all-fail");
    let server = Server::new(200, b"evil", 4);
    let expected = target(&["https://download.yan.life/rustdesk/stable/second.msi"]);
    let prior = root("prior");
    std::fs::write(&prior, b"previous usable package").unwrap();
    let error = package_download::download_fixture(&directory, &expected, server.address)
        .err()
        .unwrap();
    assert!(error.contains("integrity mismatch"), "{error}");
    assert_eq!(server.paths(), ["/original.msi", "/second.msi"]);
    assert!(!directory.join("original.msi").exists());
    assert_eq!(std::fs::read(&prior).unwrap(), b"previous usable package");
    let preserved = std::fs::read(directory.join("package.partial")).unwrap();
    assert_eq!(preserved, b"evil");
    assert_eq!(
        std::fs::read(directory.join("mirror-1/package.partial")).unwrap(),
        b"evil"
    );
    std::fs::remove_dir_all(directory).unwrap();
    std::fs::remove_file(prior).unwrap();
}

#[test]
fn failed_mirrors_are_deduplicated_and_preserve_each_attempt() {
    let directory = root("attempts");
    let server = Server::new(200, b"evil", 4);
    let bad = "https://download.yan.life/rustdesk/stable/second.msi";
    let expected = target(&[bad, bad, MIRROR]);
    let error = package_download::download_fixture(&directory, &expected, server.address)
        .err()
        .unwrap();
    assert!(error.contains("byte budget"), "{error}");
    assert_eq!(
        std::fs::read(directory.join("package.partial")).unwrap(),
        b"evil"
    );
    assert_eq!(
        std::fs::read(directory.join("mirror-1/package.partial")).unwrap(),
        b"evil"
    );
    assert_eq!(server.paths(), ["/original.msi", "/second.msi"]);
    assert!(!directory.join("original.msi").exists());
    let retry = package_download::download_fixture(&directory, &expected, server.address)
        .err()
        .unwrap();
    assert!(retry.contains("path unavailable"), "{retry}");
    assert_eq!(server.paths().len(), 2);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn disconnected_primary_can_fall_back() {
    let directory = root("disconnect");
    let server = Server::new(200, b"good", 4);
    let mut expected = target(&[MIRROR]);
    expected.primary = "https://download.yan.life/rustdesk/stable/disconnect.msi".into();
    let (path, locked) =
        package_download::download_fixture(&directory, &expected, server.address).unwrap();
    assert_eq!(path.file_name().unwrap(), "disconnect.msi");
    assert_eq!(std::fs::read(path).unwrap(), b"good");
    assert_eq!(server.paths(), ["/disconnect.msi", "/mirror.msi"]);
    drop(locked);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn single_source_redirect_and_untrusted_only_mirrors_fail_closed() {
    for mirrors in [
        vec![],
        vec!["https://other.invalid/rustdesk/stable/mirror.msi"],
    ] {
        let directory = root("redirect-closed");
        let server = Server::new(302, b"fail", 4);
        let error =
            package_download::download_fixture(&directory, &target(&mirrors), server.address)
                .err()
                .unwrap();
        assert!(error.contains("HTTP 302"), "{error}");
        assert_eq!(server.paths(), ["/original.msi"]);
        assert!(!directory.join("original.msi").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn existing_partial_evidence_is_not_retried_or_overwritten() {
    let directory = root("existing-evidence");
    workspace::create(&directory).unwrap();
    std::fs::write(directory.join("package.partial"), b"prior evidence").unwrap();
    let server = Server::new(200, b"good", 4);
    let error = package_download::download_fixture(&directory, &target(&[MIRROR]), server.address)
        .err()
        .unwrap();
    assert!(error.contains("path unavailable"), "{error}");
    assert_eq!(
        std::fs::read(directory.join("package.partial")).unwrap(),
        b"prior evidence"
    );
    assert!(!directory.join("original.msi").exists());
    assert!(server.paths().is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn single_source_hash_failure_preserves_error_and_partial() {
    let directory = root("single-hash");
    let server = Server::new(200, b"evil", 4);
    let error = package_download::download_fixture(&directory, &target(&[]), server.address)
        .err()
        .unwrap();
    assert_eq!(error, "install package integrity mismatch");
    assert_eq!(server.paths(), ["/original.msi"]);
    assert_eq!(
        std::fs::read(directory.join("package.partial")).unwrap(),
        b"evil"
    );
    assert!(!directory.join("original.msi").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn invalid_primary_and_download_limit_never_contact_a_mirror() {
    let server = Server::new(200, b"good", 4);
    for (primary, size, message) in [
        ("https://other.invalid/original.msi", 4, "source invalid"),
        (
            "https://download.yan.life/rustdesk/stable/original.msi",
            2 * 1024 * 1024 * 1024 + 1,
            "download limit",
        ),
    ] {
        let directory = root("rejected");
        let mut expected = target(&[MIRROR]);
        expected.primary = primary.into();
        expected.size = size;
        let error = package_download::download_fixture(&directory, &expected, server.address)
            .err()
            .unwrap();
        assert!(error.contains(message), "{error}");
        assert!(!directory.exists());
        assert!(server.paths().is_empty());
    }
}

#[test]
fn mirrors_keep_content_signature_contract_and_manifest_reference_binding() {
    use hbb_common::{
        base64::{engine::general_purpose::STANDARD, Engine as _},
        sodiumoxide::crypto::sign,
        UpdateManifest,
    };
    hbb_common::sodiumoxide::init().unwrap();
    let (public, secret) = sign::gen_keypair();
    let key = "windows-x86_64-msi-standard";
    let mut expected = target(&[]);
    expected.version = "1.5.3".into();
    expected.build_seq = 2026100502;
    expected.source_commit = "fixture".into();
    expected.signature_key_id = "fixture-key".into();
    let mut manifest = UpdateManifest {
        schema: 2,
        product: "rustdesk-yan".into(),
        edition: "multi".into(),
        channel: "stable".into(),
        ..Default::default()
    };
    let payload = base::update::update_signature_payload(&manifest, key, &expected).unwrap();
    expected.signature = STANDARD.encode(sign::sign_detached(&payload, &secret));
    manifest.targets.insert(key.into(), expected.clone());
    let original_reference =
        package_gate::package_reference(&serde_json::to_value(&manifest).unwrap(), key).unwrap();
    expected.mirrors.push(MIRROR.into());
    assert_eq!(
        base::update::update_signature_payload(&manifest, key, &expected).unwrap(),
        payload
    );
    manifest.targets.insert(key.into(), expected);
    assert_ne!(
        package_gate::package_reference(&serde_json::to_value(&manifest).unwrap(), key).unwrap(),
        original_reference
    );
    let verified =
        package_gate::verify_with_key(&manifest, key, 2026100502, "fixture-key", public.as_ref())
            .unwrap();
    let directory = root("signed-mirror");
    let server = Server::new(503, b"fail", 4);
    let (path, locked) =
        package_download::download_fixture(&directory, verified, server.address).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"good");
    drop(locked);
    let prior = std::fs::read(directory.join("original.msi")).unwrap();
    manifest.targets.get_mut(key).unwrap().sha256 = hex::encode(Sha256::digest(b"evil"));
    let error =
        package_gate::verify_with_key(&manifest, key, 2026100502, "fixture-key", public.as_ref())
            .err()
            .unwrap();
    assert!(error.contains("signature verification failed"), "{error}");
    assert_eq!(
        std::fs::read(directory.join("original.msi")).unwrap(),
        prior
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn occupied_final_cache_is_not_replaced_even_if_a_mirror_is_healthy() {
    let directory = root("occupied");
    workspace::create(&directory).unwrap();
    std::fs::write(directory.join("original.msi"), b"previous package").unwrap();
    let server = Server::new(503, b"fail", 4);
    let error = package_download::download_fixture(&directory, &target(&[MIRROR]), server.address)
        .err()
        .unwrap();
    assert!(error.contains("size mismatch"), "{error}");
    assert_eq!(
        std::fs::read(directory.join("original.msi")).unwrap(),
        b"previous package"
    );
    assert!(server.paths().is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mirror_publication_refuses_a_newly_occupied_destination() {
    let directory = root("publication-race");
    let server = Server::with_occupied_cache(503, b"fail", 4, Some(directory.join("original.msi")));
    let error = package_download::download_fixture(&directory, &target(&[MIRROR]), server.address)
        .err()
        .unwrap();
    assert!(error.contains("publication refused"), "{error}");
    assert_eq!(
        std::fs::read(directory.join("original.msi")).unwrap(),
        b"previous package"
    );
    assert_eq!(
        std::fs::read(directory.join("mirror-1/original.msi")).unwrap(),
        b"good"
    );
    assert_eq!(server.paths(), ["/original.msi", "/mirror.msi"]);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn every_source_http_length_truncation_and_redirect_failure_keeps_old_data() {
    for (label, status, body, declared, message) in [
        ("http-all", 503, b"fail".as_slice(), 4, "HTTP 503"),
        ("short-all", 200, b"bad".as_slice(), 4, ""),
        (
            "size-all",
            200,
            b"bad".as_slice(),
            3,
            "content length mismatch",
        ),
        ("redirect-all", 302, b"fail".as_slice(), 4, "HTTP 302"),
    ] {
        let directory = root(label);
        let prior = root("old-package");
        std::fs::write(&prior, b"old usable package").unwrap();
        let server = Server::new(status, body, declared);
        let error = package_download::download_fixture(
            &directory,
            &target(&["https://download.yan.life/rustdesk/stable/second.msi"]),
            server.address,
        )
        .err()
        .unwrap();
        if label == "short-all" {
            assert!(
                error.contains("body") || error.contains("end of file"),
                "{error}"
            );
            assert_eq!(
                std::fs::read(directory.join("package.partial")).unwrap(),
                b"bad"
            );
            assert_eq!(
                std::fs::read(directory.join("mirror-1/package.partial")).unwrap(),
                b"bad"
            );
        } else {
            assert_eq!(error.matches(message).count(), 2, "{error}");
        }
        assert_eq!(server.paths(), ["/original.msi", "/second.msi"]);
        assert!(!directory.join("original.msi").exists());
        assert_eq!(std::fs::read(&prior).unwrap(), b"old usable package");
        std::fs::remove_dir_all(directory).unwrap();
        std::fs::remove_file(prior).unwrap();
    }
}

#[test]
fn corrupt_same_size_cache_is_preserved_without_network() {
    let directory = root("cache-hash");
    workspace::create(&directory).unwrap();
    std::fs::write(directory.join("original.msi"), b"evil").unwrap();
    let server = Server::new(200, b"good", 4);
    let error = package_download::download_fixture(&directory, &target(&[MIRROR]), server.address)
        .err()
        .unwrap();
    assert_eq!(error, "install package integrity mismatch");
    assert_eq!(
        std::fs::read(directory.join("original.msi")).unwrap(),
        b"evil"
    );
    assert!(server.paths().is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn fixture_rejects_non_loopback_and_zero_port_before_creating_workspace() {
    for address in ["192.0.2.1:12345", "127.0.0.1:0"] {
        let directory = root("fixture-address");
        let error = package_download::download_fixture(
            &directory,
            &target(&[MIRROR]),
            address.parse().unwrap(),
        )
        .err()
        .unwrap();
        assert_eq!(error, "package fixture must use a loopback listener");
        assert!(!directory.exists());
    }
}

#[test]
fn budget_rejects_more_than_eight_mirrors_before_network() {
    let directory = root("budget-sources");
    let server = Server::new(200, b"good", 4);
    let error =
        package_download::download_fixture(&directory, &target(&[MIRROR; 9]), server.address)
            .err()
            .expect("source limit");
    assert!(error.contains("mirror count"), "{error}");
    assert!(server.paths().is_empty());
    assert!(!directory.exists());
}

#[test]
fn budget_stops_before_third_full_body_and_keeps_both_failures() {
    let directory = root("budget-bytes");
    let server = Server::new(200, b"evil", 4);
    let error = package_download::download_fixture(
        &directory,
        &target(&[
            "https://download.yan.life/rustdesk/stable/second.msi",
            MIRROR,
        ]),
        server.address,
    )
    .err()
    .expect("byte limit");
    assert!(error.contains("byte budget"), "{error}");
    assert!(error.contains("integrity mismatch"), "{error}");
    assert_eq!(server.paths(), ["/original.msi", "/second.msi"]);
    for path in ["package.partial", "mirror-1/package.partial"] {
        assert_eq!(std::fs::read(directory.join(path)).unwrap(), b"evil");
    }
    assert!(!directory.join("original.msi").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn budget_deadline_covers_body_and_does_not_restart_for_mirror() {
    let directory = root("budget-time");
    let server = Server::new(200, b"good", 4);
    let mut expected = target(&[MIRROR]);
    expected.primary = "https://download.yan.life/rustdesk/stable/slow.msi".into();
    let start = std::time::Instant::now();
    let error = package_download::download_budget_fixture(
        &directory,
        &expected,
        server.address,
        Duration::from_millis(100),
    )
    .err()
    .expect("deadline");
    assert!(error.contains("time budget"), "{error}");
    assert!(start.elapsed() < Duration::from_millis(300));
    assert_eq!(server.paths(), ["/slow.msi"]);
    assert_eq!(
        std::fs::read(directory.join("package.partial")).unwrap(),
        b"b"
    );
    assert!(!directory.join("slow.msi").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn budget_deadline_is_shared_across_redirect_and_transport_attempts() {
    let directory = root("budget-shared");
    let server = Server::new(200, b"good", 4);
    let mut expected = target(&[
        "https://download.yan.life/rustdesk/stable/disconnect.msi",
        "https://download.yan.life/rustdesk/stable/wait-error.msi",
        MIRROR,
    ]);
    expected.primary = "https://download.yan.life/rustdesk/stable/wait-redirect.msi".into();
    let start = std::time::Instant::now();
    let error = package_download::download_budget_fixture(
        &directory,
        &expected,
        server.address,
        Duration::from_millis(100),
    )
    .err()
    .expect("shared deadline");
    assert!(error.contains("time budget"), "{error}");
    assert!(start.elapsed() < Duration::from_millis(250));
    assert!(!server.paths().contains(&"/mirror.msi".into()));
    assert!(!directory.join("wait-redirect.msi").exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn budget_counts_overflow_and_partial_transport_bytes_and_preserves_evidence() {
    for name in ["chunk-over.msi", "chunk-short.msi"] {
        let directory = root("budget-partial");
        let server = Server::new(200, b"evil", 4);
        let mut expected = target(&[
            "https://download.yan.life/rustdesk/stable/second.msi",
            MIRROR,
        ]);
        expected.primary = format!("https://download.yan.life/rustdesk/stable/{name}");
        let error = package_download::download_fixture(&directory, &expected, server.address)
            .err()
            .expect("byte exhaustion");
        assert!(error.contains("byte budget"), "{error}");
        assert_eq!(server.paths().len(), 2);
        let partial = std::fs::read(directory.join("package.partial")).unwrap();
        assert_eq!(
            partial,
            if name == "chunk-over.msi" {
                b"evil!".as_slice()
            } else {
                b"bad".as_slice()
            }
        );
        assert_eq!(
            std::fs::read(directory.join("mirror-1/package.partial")).unwrap(),
            b"evil"
        );
        assert!(!directory.join(name).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn budget_accepts_eight_mirrors_but_requests_only_primary_plus_eight() {
    let directory = root("budget-nine-sources");
    let server = Server::new(503, b"fail", 4);
    let mirrors = (0..8)
        .map(|n| format!("https://download.yan.life/rustdesk/stable/{n}.msi"))
        .collect::<Vec<_>>();
    let mut expected = target(&[]);
    expected.mirrors = mirrors;
    let error = package_download::download_fixture(&directory, &expected, server.address)
        .err()
        .unwrap();
    assert_eq!(error.matches("HTTP 503").count(), 9);
    assert_eq!(server.paths().len(), 9);
    assert!(!directory.join("original.msi").exists());
    std::fs::remove_dir_all(directory).unwrap();
}
