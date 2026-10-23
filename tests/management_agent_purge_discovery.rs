#![cfg(windows)]
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/purge_policy.rs"]
mod purge_policy;
#[path = "../src/management_agent/purge_discovery.rs"]
mod purge_discovery;
use msi_metadata::PackageIdentity;
use purge_discovery::{Approved, Candidate, classify};
use purge_policy::Probe;

fn identity() -> PackageIdentity {
    PackageIdentity { product_code: "{71A6889B-6003-42DB-989F-B7AE68260DDB}".into(),
        product_name: "RustDesk".into(), product_version: "1.5.3.20261005".into(),
        upgrade_code: "fixture-family".into() }
}
fn candidate() -> Candidate { Candidate { registered: identity(), cache: identity(), source: identity(),
    package_code: "fixture-package".into(), cache_package_code: "fixture-package".into(),
    source_package_code: "fixture-package".into(), source_path: Default::default(), cache_path: Default::default(), _locks: vec![] } }

#[test]
fn source_or_cache_identity_mismatch_refuses_same_named_product() {
    for field in ["cache", "source", "version", "package-code"] {
        let mut c = candidate();
        match field { "cache" => c.cache.product_code = "foreign".into(),
            "source" => c.source.upgrade_code = "foreign".into(),
            "package-code" => c.cache_package_code = "foreign".into(),
            _ => c.registered.product_version = "foreign".into() }
        assert_eq!(classify(&Approved::fixture(identity()), Ok(vec![c])), Probe::IdentityMismatch, "{field}");
    }
}

#[test]
fn registered_absent_foreign_ambiguous_unknown_are_distinct() {
    let a = Approved::fixture(identity());
    assert_eq!(classify(&a, Ok(vec![candidate()])), Probe::RegisteredVerified);
    assert_eq!(classify(&a, Ok(vec![])), Probe::AbsentVerified);
    let mut foreign = candidate(); foreign.registered.product_code = "foreign".into();
    assert_eq!(classify(&a, Ok(vec![foreign])), Probe::IdentityMismatch);
    assert_eq!(classify(&a, Ok(vec![candidate(), candidate()])), Probe::Ambiguous);
    assert_eq!(classify(&a, Err("access denied".into())), Probe::Unknown);
}

#[test]
fn adapter_combines_discovery_and_policy_without_external_effects() {
    use std::{cell::Cell, collections::VecDeque};
    use purge_policy::{InstallerResult, Outcome};
    for mode in ["registered", "absent", "foreign", "ambiguous", "unknown"] {
        let a = Approved::fixture(identity());
        let first = match mode {
            "registered" => Ok(vec![candidate()]), "absent" => Ok(vec![]),
            "foreign" => { let mut c = candidate(); c.source.product_code = "foreign".into(); Ok(vec![c]) },
            "ambiguous" => Ok(vec![candidate(), candidate()]), _ => Err("query failed".into()) };
        let mut queries = VecDeque::from([first]);
        if mode == "registered" { queries.push_back(Ok(vec![candidate()])); }
        else if mode == "absent" { queries.push_back(Ok(vec![])); }
        queries.push_back(Ok(vec![]));
        queries.push_back(Ok(vec![]));
        let installs = Cell::new(0); let cleanups = Cell::new(0);
        let old = b"identity/source/state".to_vec(); let mut data = old.clone();
        let result = purge_discovery::coordinate(a,
            || queries.pop_front().unwrap(),
            || { installs.set(installs.get()+1); InstallerResult::Exited(0) },
            || { cleanups.set(cleanups.get()+1); data.clear(); Ok(()) });
        assert_eq!(result, match mode { "registered" | "absent" => Outcome::Complete,
            "foreign" => Outcome::IdentityMismatch, "ambiguous" => Outcome::Ambiguous, _ => Outcome::QueryUnknown });
        assert_eq!(installs.get(), u32::from(mode == "registered"));
        assert_eq!(cleanups.get(), u32::from(matches!(mode, "registered" | "absent")));
        if cleanups.get() == 0 { assert_eq!(data, old); }
    }
}

#[test]
fn native_readonly_enumeration_reports_observation_without_authorizing_purge() {
    base::lab_guard::require_lab_os("win11");
    let candidates = purge_discovery::native_candidates().expect("read-only native query must succeed");
    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.registered.product_code, candidate.cache.product_code);
    assert_eq!(candidate.registered.product_code, candidate.source.product_code);
    println!("READ_ONLY product={} version={} cache/source metadata matched; not authorization",
        candidate.registered.product_code, candidate.registered.product_version);
}

#[test]
fn registration_change_before_installer_preserves_data() {
    use std::{cell::Cell, collections::VecDeque};
    let a = Approved::fixture(identity());
    let mut reads = VecDeque::from([Ok(vec![candidate()]), Err("registration changed".into())]);
    let calls = Cell::new(0);
    let data = b"identity/source/state".to_vec();
    let before = data.clone();
    let outcome = purge_discovery::coordinate(a, || reads.pop_front().unwrap(),
        || { calls.set(calls.get() + 1); purge_policy::InstallerResult::Pending },
        || { panic!("cleanup must not run") });
    assert_eq!(outcome, purge_policy::Outcome::QueryUnknown);
    assert_eq!(calls.get(), 0);
    assert_eq!(data, before);
}

#[test]
fn held_fixture_source_denies_writer_and_preserves_bytes() {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("purge-held-fixture-{}.bin", std::process::id()));
    let mut created = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    created.write_all(b"original approved source fixture").unwrap();
    drop(created);
    let held = purge_discovery::hold(&path).unwrap();
    let denied = std::fs::OpenOptions::new().write(true).open(&path).unwrap_err();
    assert_eq!(denied.raw_os_error(), Some(32));
    assert_eq!(std::fs::read(&path).unwrap(), b"original approved source fixture");
    drop(held);
    // Retain this uniquely named fixture as evidence; no directory/product cleanup.
}

#[test]
fn authorized_cleanup_releases_own_handles_before_synchronous_delete() {
    let path = std::env::temp_dir().join(format!("purge-cleanup-fixture-{}.bin", std::process::id()));
    std::fs::write(&path, b"preserved identity").unwrap();
    let a = Approved::fixture_locked(identity(), vec![purge_discovery::hold(&path).unwrap()]);
    assert_eq!(std::fs::remove_file(&path).unwrap_err().raw_os_error(), Some(32));
    let result = purge_discovery::coordinate(a, || Ok(vec![]),
        || panic!("already absent"), || std::fs::remove_file(&path).map_err(|_| ()));
    assert_eq!(result, purge_policy::Outcome::Complete);
    assert!(!path.exists());
}

#[test]
fn cleanup_error_preserves_remaining_fixture_bytes_and_never_reports_complete() {
    let path = std::env::temp_dir().join(format!("purge-error-fixture-{}.bin", std::process::id()));
    std::fs::write(&path, b"remaining source and identity").unwrap();
    let external = purge_discovery::hold(&path).unwrap();
    let approved = Approved::fixture_locked(identity(), vec![purge_discovery::hold(&path).unwrap()]);
    let mut calls = 0;
    let result = purge_discovery::coordinate(approved, || Ok(vec![]),
        || panic!("already absent"), || {
            calls += 1;
            let error = std::fs::remove_file(&path).unwrap_err();
            assert_eq!(error.raw_os_error(), Some(32));
            Err(())
        });
    assert_eq!(result, purge_policy::Outcome::CleanupFailed);
    assert_eq!(calls, 1);
    assert_eq!(std::fs::read(&path).unwrap(), b"remaining source and identity");
    drop(external);
    std::fs::remove_file(path).unwrap();
}
