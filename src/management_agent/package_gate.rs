use hbb_common::{
    base64::{engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}, Engine as _},
    sodiumoxide::crypto::sign,
    UpdateManifest, UpdateTarget,
};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

const KEY_ID: &str = "yan-release-2026";
const PUBLIC_KEY: &str = "YNphn2SGjnetwp0bb/uEGpzfQi8OavMWTHCmqPbMuxg=";

pub fn cache_name(url: &str, kind: &str) -> Result<String, String> {
    if !matches!(kind, "msi" | "exe") { return Err("package cache type invalid".into()); }
    let path = url.strip_prefix("https://download.yan.life/rustdesk/stable/")
        .ok_or("package cache source invalid")?;
    if path.contains(['%', '\\', '?', '#', ':']) || path.chars().any(char::is_control)
        || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err("package cache source invalid".into());
    }
    let name = path.rsplit('/').next().ok_or("package cache name absent")?;
    if name.len() > 180 || !name.ends_with(&format!(".{kind}"))
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte)) {
        return Err("package cache name invalid".into());
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4
            && matches!(stem.as_bytes()[3], b'1'..=b'9')) {
        return Err("package cache reserved name".into());
    }
    Ok(name.to_owned())
}

pub fn package_reference(manifest: &serde_json::Value, key: &str) -> Result<String, String> {
    if !manifest.is_object() {
        return Err("approved package manifest must be an object".into());
    }
    let bytes = serde_json::to_vec(&serde_json::json!({"manifest": manifest, "target_key": key}))
        .map_err(|error| error.to_string())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub fn bind_resolved(
    reply: &serde_json::Value,
    reference: &str,
    key: &str,
    build: u64,
) -> Result<UpdateManifest, String> {
    if reference.len() != 64
        || !reference.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || build == 0
        || reply["package_id"] != reference
        || reply["target_key"] != key
        || package_reference(&reply["manifest"], key)? != reference
    {
        return Err("approved package command binding mismatch".into());
    }
    let manifest: UpdateManifest = serde_json::from_value(reply["manifest"].clone())
        .map_err(|_| "approved package manifest malformed".to_owned())?;
    let target = manifest.targets.get(key).ok_or("approved package target absent")?;
    let (_, actual_build, _) = base::update::update_target_release(&manifest, target)
        .map_err(|error| error.to_string())?;
    if actual_build != build {
        return Err("approved package build differs from command".into());
    }
    Ok(manifest)
}

pub fn verify<'a>(
    manifest: &'a UpdateManifest,
    key: &str,
    minimum_build: u64,
) -> Result<&'a UpdateTarget, String> {
    let public = STANDARD
        .decode(PUBLIC_KEY)
        .map_err(|_| "release public key encoding invalid".to_owned())?;
    verify_with_key(manifest, key, minimum_build, KEY_ID, &public)
}

pub fn verify_with_key<'a>(
    manifest: &'a UpdateManifest,
    key: &str,
    minimum_build: u64,
    key_id: &str,
    public: &[u8],
) -> Result<&'a UpdateTarget, String> {
    if ![1, 2].contains(&manifest.schema)
        || manifest.product != "rustdesk-yan"
        || manifest.channel != "stable"
        || !["multi", "standard", "sos"].contains(&manifest.edition.as_str())
    {
        return Err("install manifest product/channel/schema invalid".into());
    }
    let (kind, edition) = match key {
        "windows-x86_64-msi-standard" => ("msi", "standard"),
        "windows-x86_64-msi-sos" => ("msi", "sos"),
        "windows-x86_64-exe-standard" => ("exe", "standard"),
        "windows-x86_64-exe-sos" => ("exe", "sos"),
        _ => return Err("install manifest target invalid".into()),
    };
    if manifest.edition != "multi" && manifest.edition != edition {
        return Err("install manifest edition mismatch".into());
    }
    let target = manifest
        .targets
        .get(key)
        .ok_or("install manifest target absent")?;
    base::update::validate_target_metadata(target, key_id).map_err(|e| e.to_string())?;
    let (_, build, _) =
        base::update::update_target_release(manifest, target).map_err(|e| e.to_string())?;
    if build < minimum_build || build == 0 {
        return Err("install package downgrade refused".into());
    }
    let url =
        url::Url::parse(&target.primary).map_err(|_| "install package URL malformed".to_owned())?;
    let original_path = target
        .primary
        .strip_prefix("https://download.yan.life/")
        .ok_or("install package URL outside approved source")?;
    if original_path.contains(['%', '\\'])
        || original_path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("install package original URL path traversal refused".into());
    }
    let path = url.path();
    if url.scheme() != "https"
        || url.host_str() != Some("download.yan.life")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
        || url.query().is_some()
        || url.fragment().is_some()
        || !path.starts_with("/rustdesk/stable/")
        || path.contains('%')
        || path.contains('\\')
        || path
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
        || !path.ends_with(&format!(".{kind}"))
    {
        return Err("install package URL outside approved source".into());
    }
    hbb_common::sodiumoxide::init()
        .map_err(|_| "install signature crypto unavailable".to_owned())?;
    let public =
        sign::PublicKey::from_slice(public).ok_or("install signature public key invalid")?;
    let signature = STANDARD
        .decode(&target.signature)
        .or_else(|_| URL_SAFE_NO_PAD.decode(&target.signature))
        .map_err(|_| "install signature encoding invalid".to_owned())?;
    let signature = sign::Signature::from_bytes(&signature)
        .map_err(|_| "install signature length invalid".to_owned())?;
    let payload =
        base::update::update_signature_payload(manifest, key, target).map_err(|e| e.to_string())?;
    if !sign::verify_detached(&signature, &payload, &public) {
        return Err("install manifest signature verification failed".into());
    }
    Ok(target)
}

pub struct LockedPackage {
    _file: std::fs::File,
}

pub fn lock_verified_file(path: &Path, target: &UpdateTarget) -> Result<LockedPackage, String> {
    if !path.is_absolute() || target.size == 0 {
        return Err("install package path/size invalid".into());
    }
    for ancestor in path.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|e| format!("install package path unavailable: {e}"))?;
        if metadata.file_type().is_symlink() {
            return Err("install package symlink refused".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("install package reparse path refused".into());
            }
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("install package lock: {e}"))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() != target.size {
        return Err("install package size mismatch".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > target.size {
            return Err("install package size changed".into());
        }
        hash.update(&buffer[..count]);
    }
    if total != target.size || hex::encode(hash.finalize()) != target.sha256.to_ascii_lowercase() {
        return Err("install package integrity mismatch".into());
    }
    Ok(LockedPackage { _file: file })
}
