use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Extracted {
    pub client: PathBuf,
    pub agent: PathBuf,
}

#[derive(Debug)]
pub struct PreparedTarget {
    pub client_manifest: super::install_plan::Artifact,
    pub agent: super::install_plan::Artifact,
}

pub fn seal_target(root: &Path, extracted: &Extracted) -> Result<PreparedTarget, String> {
    use sha2::{Digest, Sha256};
    use std::{io::{Read, Write}, os::windows::fs::OpenOptionsExt};
    super::workspace::verify(root)?;
    let image = root.join("image");
    if !extracted.client.starts_with(&image) || !extracted.agent.starts_with(&image)
        || extracted.client.components().chain(extracted.agent.components())
            .any(|part| matches!(part, std::path::Component::ParentDir)) {
        return Err("MSI target outside verified image".into());
    }
    let destination = root.join("target-assets");
    super::workspace::create(&destination)?;
    let inventory = super::directory_backup::capture(&extracted.client, &destination.join("client"))?;
    inventory.verify(&destination.join("client"))?;
    super::workspace::verify_file(&extracted.agent)?;
    let mut source = std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
        .open(&extracted.agent).map_err(|error| error.to_string())?;
    let size = source.metadata().map_err(|error| error.to_string())?.len();
    if size == 0 || size > 512 * 1024 * 1024 { return Err("MSI target agent size invalid".into()); }
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true)
        .open(destination.join("agent.exe")).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = source.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 { break; }
        copied += count as u64;
        if copied > size { return Err("MSI target agent changed".into()); }
        output.write_all(&buffer[..count]).map_err(|error| error.to_string())?;
        hash.update(&buffer[..count]);
    }
    output.sync_all().map_err(|error| error.to_string())?;
    drop(output);
    if copied != size { return Err("MSI target agent changed".into()); }
    let manifest_bytes = std::fs::read(destination.join("client/manifest.json")).map_err(|error| error.to_string())?;
    let prepared = PreparedTarget {
        client_manifest: super::install_plan::Artifact {
            relative_path: "target-assets/client/manifest.json".into(), size: manifest_bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(&manifest_bytes)),
        },
        agent: super::install_plan::Artifact { relative_path: "target-assets/agent.exe".into(), size, sha256: hex::encode(hash.finalize()) },
    };
    for artifact in [&prepared.client_manifest, &prepared.agent] {
        super::workspace::verify_file(&root.join(&artifact.relative_path))?;
        artifact.verify(root)?;
    }
    Ok(prepared)
}

pub fn extract(root: &Path, package: &Path, manifest: &hbb_common::UpdateManifest, key: &str, minimum_build: u64) -> Result<Extracted, String> {
    let target = super::package_gate::verify(manifest, key, minimum_build)?;
    extract_verified(root, package, target)
}

#[cfg(test)]
pub fn extract_fixture(root: &Path, package: &Path, manifest: &hbb_common::UpdateManifest, key: &str, public: &[u8]) -> Result<Extracted, String> {
    let target = super::package_gate::verify_with_key(manifest, key, 2026100502, "fixture-key", public)?;
    extract_verified(root, package, target)
}

fn extract_verified(root: &Path, package: &Path, target: &hbb_common::UpdateTarget) -> Result<Extracted, String> {
    use std::{os::windows::fs::MetadataExt, process::Command, time::Duration};
    super::workspace::verify(root)?;
    let _locked = super::package_gate::lock_verified_file(package, target)?;
    if super::msi_metadata::inspect(package)?.product_name != "RustDesk" {
        return Err("MSI extraction product mismatch".into());
    }
    let image = root.join("image");
    super::workspace::create(&image)?;
    let destination = image.to_str().ok_or("MSI extraction path encoding invalid")?;
    if destination.contains('"') || destination.chars().any(char::is_control) {
        return Err("MSI extraction destination invalid".into());
    }
    let executable = super::system_dirs::get().system32().join("msiexec.exe");
    let mut command = Command::new(executable);
    command.arg("/a").arg(package).args(["/qn", "/norestart", "REBOOT=ReallySuppress"])
        .arg(format!("TARGETDIR={destination}"))
        .arg("/L*v").arg(root.join("extract.log"));
    match super::installer_process::run(&mut command, Duration::from_secs(120))? {
        super::install_execution::InstallOutcome::Installed => {}
        outcome => return Err(format!("MSI extraction incomplete; preserve workspace: {outcome:?}")),
    }
    let mut pending = vec![image.clone()];
    let mut client = None;
    let mut agent = None;
    let mut entries = 0usize;
    let mut total = 0u64;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            entries += 1;
            if entries > 20000 || metadata.file_attributes() & 0x400 != 0 {
                return Err("MSI extraction capacity or reparse path refused".into());
            }
            if metadata.is_dir() { pending.push(path); continue; }
            if !metadata.is_file() { return Err("MSI extraction special file refused".into()); }
            total = total.checked_add(metadata.len()).ok_or("MSI extraction size overflow")?;
            if total > 8 * 1024 * 1024 * 1024 { return Err("MSI extraction capacity exceeded".into()); }
            super::workspace::verify_file(&path)?;
            let name = path.file_name().and_then(|value| value.to_str()).ok_or("MSI extraction file encoding invalid")?;
            if name.eq_ignore_ascii_case("RustDesk.exe") {
                if client.is_some() { return Err("MSI extraction ambiguous client".into()); }
                client = path.parent().map(Path::to_path_buf);
            } else if name.eq_ignore_ascii_case("rustdesk-management-agent.exe") {
                // Some packages also place a copy beside the main program.
                // Only the dedicated ManagementAgent component is selected.
                if path.parent().and_then(Path::file_name).is_some_and(|name| name == "ManagementAgent") {
                    if agent.is_some() { return Err("MSI extraction ambiguous agent".into()); }
                    agent = Some(path);
                }
            }
        }
    }
    let client = client.ok_or("MSI extraction client absent")?;
    let agent = agent.ok_or("MSI extraction agent absent")?;
    if !client.join("librustdesk.dll").is_file() || !client.join("data").join("flutter_assets").is_dir() {
        return Err("MSI extraction client directory incomplete".into());
    }
    Ok(Extracted { client, agent })
}
