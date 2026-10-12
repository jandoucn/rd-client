#![cfg(windows)]
#[path = "../src/management_agent/msi_metadata.rs"]
mod msi_metadata;

#[test]
fn invalid_database_is_refused_without_changing_bytes() {
    let path = std::env::temp_dir().join(format!("agent-invalid-msi-{}.msi", std::process::id()));
    std::fs::write(&path, b"not an MSI").unwrap();
    assert!(msi_metadata::inspect(&path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"not an MSI");
    std::fs::remove_file(&path).unwrap();
    assert!(msi_metadata::inspect(&path).is_err());
    assert!(msi_metadata::inspect(std::path::Path::new("relative.msi")).is_err());
}

#[test]
#[ignore = "Requires verified v25 MSI in lab VM"]
fn actual_package_metadata_is_read_only_and_matches_registered_identity() {
    use sha2::{Digest, Sha256};
    assert!(matches!(std::env::var("COMPUTERNAME").unwrap().as_str(), "RDLAB-WIN10" | "RDLAB-WIN11"));
    let path = std::path::Path::new(r"C:\RustDeskAgentLab\agent-health-v25.msi");
    let before = hex::encode(Sha256::digest(std::fs::read(path).unwrap()));
    assert_eq!(before, "d158bbb84ff98b7b1ac84cd373181555f9b97a9a2782c6690aaaa8859f4956d3");
    let identity = msi_metadata::inspect(path).unwrap();
    let source = msi_metadata::installed_source(&identity.product_code).unwrap();
    assert_eq!(source.package_name, "agent-health-v25.msi");
    assert!(source.source.is_file());
    assert_eq!(hex::encode(Sha256::digest(std::fs::read(&source.source).unwrap())), before);
    assert!(msi_metadata::installed_source("not-a-product").is_err());
    assert_eq!(identity.product_name, "RustDesk");
    assert!(identity.product_version.starts_with("1.5.3."));
    let expected = std::fs::read_to_string(r"C:\RustDeskAgentLab\native-v28-product.txt").unwrap();
    assert_eq!(identity.product_code, expected.trim().trim_start_matches('\u{feff}'));
    assert_eq!(identity.upgrade_code.len(), 38);
    msi_metadata::verify_pair(&identity, &identity, &identity.product_code, &identity.product_code).unwrap();
    let mut foreign = msi_metadata::inspect(path).unwrap();
    foreign.product_name = "Other product".into();
    assert_eq!(msi_metadata::verify_pair(&identity, &foreign, &identity.product_code, &identity.product_code).unwrap_err(), "MSI product family mismatch");
    foreign.product_name = identity.product_name.clone();
    foreign.upgrade_code = "{00000000-0000-0000-0000-000000000000}".into();
    assert_eq!(msi_metadata::verify_pair(&identity, &foreign, &identity.product_code, &identity.product_code).unwrap_err(), "MSI product family mismatch");
    assert_eq!(msi_metadata::verify_pair(&identity, &identity, "{00000000-0000-0000-0000-000000000000}", &identity.product_code).unwrap_err(), "MSI product code differs from prepared assets");
    assert_eq!(hex::encode(Sha256::digest(std::fs::read(path).unwrap())), before);
}
