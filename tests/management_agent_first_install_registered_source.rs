#![cfg(windows)]
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;
#[path = "../src/management_agent/package_gate.rs"]
mod package_gate;

#[test]
#[ignore = "Win11 owner after actual official 1501 MSI upgrade; read-only registration observation"]
fn t01_installed_1501_real_registered_source_matches_signed_release() {
    use std::path::Path;
    assert_eq!(std::env::var("COMPUTERNAME").as_deref(), Ok("RDLAB-WIN11"));
    assert_eq!(
        std::env::var("RUSTDESK_T01_REGISTERED").as_deref(),
        Ok("observe-installed-1501")
    );
    let root = Path::new(r"C:\RustDeskAgentLab\t01-delivery-input");
    let reply: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("resolved.json")).unwrap()).unwrap();
    let key = "windows-x86_64-msi-sos";
    let manifest = package_gate::bind_resolved(
        &reply,
        reply["package_id"].as_str().unwrap(),
        key,
        2026101501,
    )
    .unwrap();
    let target = package_gate::verify(&manifest, key, 2026101501).unwrap();
    let name = package_gate::cache_name(&target.primary, "msi").unwrap();
    let official = root.join(&name);
    let _lock = package_gate::lock_verified_file(&official, target).unwrap();
    let product = msi_metadata::inspect(&official).unwrap();
    let paths = [
        r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json",
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk2.toml",
    ];
    let before =
        paths.map(|p| std::fs::read(p).expect("SETUP BLOCKED: installed identity/state required"));
    let observed = msi_metadata::installed_source(&product.product_code);
    assert!(
        paths.map(|p| std::fs::read(p).unwrap()) == before,
        "read-only source query preserves state/identity/config"
    );
    println!(
        "T01_REGISTERED product={} expected_basename={} observed={observed:?}",
        product.product_code, name
    );
    let observed = observed.expect("SOURCE UNAVAILABLE: preserve actual registration; do not copy missing source to manufacture success");
    assert_eq!(
        observed.package_name, name,
        "registered basename differs from signed release"
    );
    let verified = package_gate::lock_verified_file(&observed.source, target);
    assert!(
        paths.map(|p| std::fs::read(p).unwrap()) == before,
        "verification preserves state/identity/config"
    );
    if let Err(error) = &verified {
        println!("T01_REGISTERED package_verification_error={error}");
    }
    assert!(
        verified.is_ok(),
        "real registered source must match original signed bytes"
    );
}
