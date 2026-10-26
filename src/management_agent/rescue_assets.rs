use super::{
    directory_backup::{self, DirectoryBackup},
    install_guard::InstallGuard,
    install_plan::{Artifact, InstallPlan},
    msi_assets, msi_metadata, package_gate,
    transaction::{Phase, Transaction},
    workspace,
};
use hbb_common::UpdateManifest;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[path = "rescue_first_install.rs"]
pub mod first_install;
#[path = "rescue_rotation.rs"]
mod rescue_rotation;
#[cfg(test)]
std::thread_local! {static ROTATION_FAULT:std::cell::Cell<Option<&'static str>>=const {std::cell::Cell::new(None)};}

#[cfg(test)]
pub fn refresh_fault_fixture(
    root: &Path,
    transaction: &Path,
    public: &[u8],
    fault: &'static str,
) -> Result<serde_json::Value, String> {
    ROTATION_FAULT.with(|value| value.set(Some(fault)));
    let result = refresh_fixture(root, transaction, public);
    ROTATION_FAULT.with(|value| value.set(None));
    result
}

fn rotation_registered_source(root: &Path, source: &Path) -> Result<(), String> {
    if !root.is_absolute()
        || !source.is_absolute()
        || source
            .components()
            .chain(root.components())
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("rescue registered source path invalid".into());
    }
    let source = source
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    for name in ["current", "previous", "staging"] {
        let moving = root
            .join(name)
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        if source == moving || source.starts_with(&format!("{moving}\\")) {
            return Err(
                "rescue registered MSI source is inside moving bundle; rotation refused".into(),
            );
        }
    }
    Ok(())
}
fn rotation_installed_source(root: &Path, evidence: &serde_json::Value) -> Result<(), String> {
    #[cfg(test)]
    if evidence["mode"] == "isolated_fixture" {
        return Ok(());
    }
    rotation_registered_source(
        root,
        Path::new(
            evidence["source"]
                .as_str()
                .ok_or("rescue registered MSI source evidence missing")?,
        ),
    )
}
#[cfg(test)]
pub fn rotation_registered_fixture(root: &Path, source: &Path) -> Result<(), String> {
    rotation_registered_source(root, source)
}

#[cfg(test)]
pub fn rotation_io_fixture(
    root: &Path,
    transaction: &Path,
    old: &str,
    new: &str,
    previous: Option<&str>,
    fault: Option<&str>,
) -> Result<(), String> {
    rescue_rotation::rotate(
        root,
        transaction,
        old,
        new,
        previous,
        |path| workspace::verify(path),
        fault,
    )
}

#[cfg(test)]
pub fn rotation_payload_fixture(
    root: &Path,
    transaction: &Path,
    old: &str,
    new: &str,
    previous: Option<&str>,
    fault: Option<&str>,
) -> Result<(), String> {
    rescue_rotation::rotate(
        root,
        transaction,
        old,
        new,
        previous,
        |path| {
            workspace::verify(path)?;
            if read(&path.join("payload.bin"), 65536, false)? != b"verified payload" {
                return Err("rotation payload integrity mismatch".into());
            }
            Ok(())
        },
        fault,
    )
}

struct Layout {
    agent: PathBuf,
    destination: PathBuf,
    transaction: PathBuf,
    client: PathBuf,
    agent_exe: PathBuf,
    identity: PathBuf,
    config: PathBuf,
    bootstrap: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    schema: u32,
    installation_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    source_transaction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    installation_receipt: Option<Artifact>,
    source_reference: String,
    target_key: String,
    version: String,
    build_seq: u64,
    verified_at: u64,
    package: Artifact,
    client_manifest: Artifact,
    agent: Artifact,
    bootstrap: Artifact,
    bootstrap_manifest: Artifact,
    identity: Artifact,
    config: Artifact,
    identity_sha256: String,
    config_sha256: String,
}

pub fn after_commit(transaction: &Path) -> Result<(), String> {
    after_commit_checked(transaction, None)
}

fn after_commit_checked(transaction: &Path, public: Option<&[u8]>) -> Result<(), String> {
    let agent = PathBuf::from(r"C:\ProgramData\RustDesk\ManagementAgent");
    let id = transaction
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("rescue transaction invalid")?;
    if !token(id, 32) || transaction != agent.join("transactions").join(id) {
        return Err("rescue source outside fixed transactions".into());
    }
    let identity = hbb_common::config::Config::file();
    if ![
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
        r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RustDesk\config\RustDesk.toml",
    ]
    .iter()
    .any(|path| identity.to_string_lossy().eq_ignore_ascii_case(path))
    {
        return Err("rescue identity outside fixed profiles".into());
    }
    let layout = Layout {
        agent_exe: agent.join("rustdesk-management-agent.exe"),
        destination: agent.join("rescue"),
        agent,
        transaction: transaction.into(),
        client: r"C:\Program Files\RustDesk".into(),
        config: identity.with_file_name("RustDesk2.toml"),
        identity,
        bootstrap: r"C:\ProgramData\RustDesk\RecoveryBootstrap".into(),
    };
    let report = prepare(&layout, public, check_installed_source)?;
    let result = transaction.join("rescue-result.json");
    if result.exists() {
        let previous: serde_json::Value = serde_json::from_slice(&read(&result, 65536, false)?)
            .map_err(|_| "rescue previous result malformed")?;
        if previous["source_transaction"] != report["source_transaction"]
            || previous["source_reference"] != report["source_reference"]
        {
            return Err("rescue previous result binding conflict".into());
        }
        return Ok(());
    }
    write_new(
        &result,
        &serde_json::to_vec(&report).map_err(|_| "rescue report encoding")?,
    )
}

#[cfg(test)]
pub fn after_commit_fixture(transaction: &Path, public: &[u8]) -> Result<(), String> {
    after_commit_checked(transaction, Some(public))
}

#[cfg(test)]
pub fn fixture_source_key_path(
    agent: &Path,
    bundle: &serde_json::Value,
) -> Result<PathBuf, String> {
    match (
        bundle["schema"].as_u64(),
        bundle.get("source_kind").and_then(|value| value.as_str()),
    ) {
        (Some(1), None | Some("committed_transaction")) => {
            let id = bundle["source_transaction"]
                .as_str()
                .filter(|id| token(id, 32))
                .ok_or("fixture committed source missing")?;
            Ok(agent
                .join("transactions")
                .join(id)
                .join("rescue-fixture-public.bin"))
        }
        (Some(2), Some("installed_msi"))
            if bundle.get("source_transaction").is_none()
                && bundle["installation_receipt"]["relative_path"]
                    == "installation-receipt.json" =>
        {
            Ok(agent.join("release-cache/fixture-public.bin"))
        }
        _ => Err("fixture source kind missing or invalid".into()),
    }
}

#[cfg(test)]
pub fn fixed_source_public_fixture(bundle: &serde_json::Value) -> Result<Vec<u8>, String> {
    let agent = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent");
    let public = read(&fixture_source_key_path(agent, bundle)?, 32, false)?;
    if public.len() != 32 {
        return Err("fixture public key size invalid".into());
    }
    let root = agent.join("rescue/current");
    let parsed: Bundle =
        serde_json::from_value(bundle.clone()).map_err(|_| "fixture bundle malformed")?;
    first_install::verify_source(&root, &parsed)?;
    let raw: serde_json::Value =
        serde_json::from_slice(&read(&root.join("approved-manifest.json"), 262144, false)?)
            .map_err(|_| "fixture approval malformed")?;
    if package_gate::package_reference(&raw, &parsed.target_key)? != parsed.source_reference {
        return Err("fixture approval binding mismatch".into());
    }
    let manifest: UpdateManifest =
        serde_json::from_value(raw).map_err(|_| "fixture approval malformed")?;
    signed_target(
        &manifest,
        &parsed.target_key,
        parsed.build_seq,
        Some(&public),
    )?;
    Ok(public)
}

#[cfg(test)]
pub fn verify_fixed_fixture(public: &[u8]) -> Result<serde_json::Value, String> {
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let state: serde_json::Value = serde_json::from_slice(&read(
        Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state\agent.json"),
        65536,
        true,
    )?)
    .map_err(|_| "rescue fixture installed state malformed")?;
    let installation = state["agent_installation_id"]
        .as_str()
        .ok_or("rescue installed identity absent")?;
    let key = state["target_key"]
        .as_str()
        .ok_or("rescue installed target absent")?;
    let report = verify(root, installation, key, Some(public))?;
    let identity = hbb_common::config::Config::file();
    if machine_protect(&read(&root.join("identity.dpapi"), 131072, false)?, true)?
        != read(&identity, 65536, true)?
    {
        return Err("rescue fixed credentials differ from original".into());
    }
    Ok(report)
}

#[cfg(test)]
fn fixture(root: &Path) -> Layout {
    Layout {
        agent: root.into(),
        destination: root.join("rescue"),
        transaction: root.join("transactions").join("a".repeat(32)),
        client: root.join("current/client/files"),
        agent_exe: root.join("current/agent.exe"),
        identity: root.join("current/identity.toml"),
        config: root.join("current/RustDesk2.toml"),
        bootstrap: root.join("bootstrap"),
    }
}

#[cfg(test)]
pub fn prepare_fixture(root: &Path, public: &[u8]) -> Result<serde_json::Value, String> {
    prepare(&fixture(root), Some(public), |_, _, _, _| {
        Ok(serde_json::json!({"mode":"isolated_fixture"}))
    })
}

#[cfg(test)]
pub fn recovery_intent_gate_fixture(root: &Path, installation: &str, key: &str) -> Result<(), String> {
    first_install::check_gates(&fixture(root), installation, key)
}

#[cfg(test)]
pub fn refresh_fixture(
    root: &Path,
    transaction: &Path,
    public: &[u8],
) -> Result<serde_json::Value, String> {
    let mut layout = fixture(root);
    layout.transaction = transaction.into();
    prepare(&layout, Some(public), |_, _, _, _| {
        Ok(serde_json::json!({"mode":"isolated_fixture"}))
    })
}

#[cfg(test)]
pub fn installed_source_fixture(
    package: &Path,
    manifest: &UpdateManifest,
    public: &[u8],
    build: u64,
) -> Result<serde_json::Value, String> {
    let target = signed_target(manifest, "windows-x86_64-msi-sos", 2026100502, Some(public))?;
    let _lock = package_gate::lock_verified_file(package, target)?;
    let msi = msi_metadata::inspect(package)?;
    check_installed_source(
        &msi,
        &package_gate::cache_name(&target.primary, "msi")?,
        target,
        build,
    )
}

#[cfg(test)]
pub fn installed_state_fixture() -> Result<(), String> {
    verify_existing_directory(
        Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state"),
        Path::new(r"C:\ProgramData\RustDesk"),
    )
}

#[cfg(test)]
pub fn prepare_installed_fixture(root: &Path, public: &[u8]) -> Result<serde_json::Value, String> {
    if root.parent() != Some(Path::new(r"C:\RustDeskAgentLab"))
        || !root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rescue-v200-"))
    {
        return Err("rescue fixture publication outside lab".into());
    }
    workspace::verify(root)?;
    let mut layout = fixture(root);
    layout.agent = r"C:\ProgramData\RustDesk\ManagementAgent".into();
    layout.client = r"C:\Program Files\RustDesk".into();
    layout.agent_exe = layout.agent.join("rustdesk-management-agent.exe");
    layout.identity =
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml"
            .into();
    layout.config = layout.identity.with_file_name("RustDesk2.toml");
    layout.bootstrap = r"C:\ProgramData\RustDesk\RecoveryBootstrap".into();
    let report = prepare(&layout, Some(public), check_installed_source)?;
    let identity = machine_protect(
        &read(
            &layout.destination.join("current/identity.dpapi"),
            131072,
            false,
        )?,
        true,
    )?;
    let config = machine_protect(
        &read(
            &layout.destination.join("current/config.dpapi"),
            131072,
            false,
        )?,
        true,
    )?;
    if identity != read(&layout.identity, 65536, true)?
        || config != read(&layout.config, 65536, true)?
    {
        return Err("rescue decrypted originals mismatch".into());
    }
    Ok(report)
}

fn check_installed_source(
    msi: &msi_metadata::PackageIdentity,
    name: &str,
    target: &hbb_common::UpdateTarget,
    build: u64,
) -> Result<serde_json::Value, String> {
    let installed = msi_metadata::installed_source(&msi.product_code)?;
    if installed.package_name != name {
        return Err("rescue registered MSI basename mismatch".into());
    }
    let _source = package_gate::lock_verified_file(&installed.source, target)?;
    let response = super::health_windows::probe_blocking(
        r"\\.\pipe\RustDesk\management-health-v1",
        Path::new(r"C:\Program Files\RustDesk\RustDesk.exe"),
        build,
        std::time::Duration::from_secs(3),
    )
    .map_err(|_| "rescue current client health unconfirmed")?;
    if !response.local_responsive {
        return Err("rescue current client health unconfirmed".into());
    }
    Ok(
        serde_json::json!({"mode":"installed","product_code":msi.product_code,"source":installed.source,
        "package_name":installed.package_name,"sha256":target.sha256,"build_seq":response.build_seq,
        "pid":response.pid,"ready":true}),
    )
}

// Existing service directories may inherit a safe ACL; only new rescue workspaces require a protected DACL.
fn verify_existing_directory(path: &Path, boundary: &Path) -> Result<(), String> {
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{ConvertSidToStringSidW, GetNamedSecurityInfoW, SE_FILE_OBJECT},
                GetAce, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION,
                OWNER_SECURITY_INFORMATION, PSID,
            },
        },
    };
    if !path.is_absolute() || !path.starts_with(boundary) {
        return Err("rescue directory boundary invalid".into());
    }
    for ancestor in path.ancestors() {
        let metadata =
            std::fs::symlink_metadata(ancestor).map_err(|_| "rescue directory unavailable")?;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("rescue directory reparse refused".into());
        }
        if !metadata.is_dir() {
            return Err("rescue directory type invalid".into());
        }
    }
    struct Memory(*mut std::ffi::c_void);
    impl Drop for Memory {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    if !LocalFree(Some(HLOCAL(self.0))).0.is_null() {
                        eprintln!("rescue ACL cleanup failed");
                    }
                }
            }
        }
    }
    let sid_text = |sid: PSID| -> Result<String, String> {
        let mut text = PWSTR::null();
        unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(|_| "rescue ACL SID invalid")?;
        let _memory = Memory(text.0.cast());
        unsafe { text.to_string() }.map_err(|_| "rescue ACL SID encoding invalid".into())
    };
    for directory in path
        .ancestors()
        .take_while(|directory| directory.starts_with(boundary))
    {
        let wide = directory
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut owner = PSID::default();
        let mut acl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = Default::default();
        let status = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                Some(&mut owner),
                None,
                Some(&mut acl),
                None,
                &mut descriptor,
            )
        };
        let _memory = Memory(descriptor.0);
        if status.0 != 0 || acl.is_null() || owner.0.is_null() {
            return Err("rescue directory ACL unavailable".into());
        }
        if !["S-1-5-18", "S-1-5-32-544"].contains(&sid_text(owner)?.as_str()) {
            return Err("rescue directory owner untrusted".into());
        }
        let mut required = [false, false];
        for index in 0..u32::from(unsafe { (*acl).AceCount }) {
            let mut pointer = std::ptr::null_mut();
            unsafe { GetAce(acl, index, &mut pointer) }
                .map_err(|_| "rescue directory ACE unavailable")?;
            if pointer.is_null() {
                return Err("rescue directory ACE missing".into());
            }
            let header = unsafe { &*pointer.cast::<ACE_HEADER>() };
            if header.AceType != 0
                || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            {
                return Err("rescue directory ACE unsupported".into());
            }
            let ace = unsafe { &*pointer.cast::<ACCESS_ALLOWED_ACE>() };
            let sid = sid_text(PSID((&ace.SidStart as *const u32).cast_mut().cast()))?;
            if header.AceFlags & 0x08 != 0 {
                continue;
            }
            match sid.as_str() {
                "S-1-5-18" | "S-1-5-32-544" => {
                    if ace.Mask & 0x001f01ff == 0x001f01ff {
                        required[usize::from(sid != "S-1-5-18")] = true;
                    }
                }
                // Create-child and directory attributes do not grant replacement of an existing
                // administrator-owned child. File-inheritable writes and DELETE_CHILD still fail.
                _ if ace.Mask & !(0x001200a9 | 0x116) == 0
                    && (ace.Mask & 0x116 == 0 || header.AceFlags & 0x01 == 0) => {}
                _ => return Err("rescue directory non-admin write permission refused".into()),
            }
        }
        if required != [true, true] {
            return Err("rescue directory administrator access incomplete".into());
        }
    }
    Ok(())
}

#[cfg(test)]
pub fn verify_fixture(root: &Path, public: &[u8]) -> Result<serde_json::Value, String> {
    verify(
        &root.join("rescue/current"),
        &"c".repeat(32),
        "windows-x86_64-msi-sos",
        Some(public),
    )
}

fn token(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn read(path: &Path, limit: u64, installed: bool) -> Result<Vec<u8>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    if installed {
        workspace::verify_restore_target(path, true)?;
    } else {
        workspace::verify_file(path)?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(|_| "rescue input unavailable")?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "rescue input read failed")?;
    if bytes.is_empty() || bytes.len() as u64 > limit {
        return Err("rescue input size invalid".into());
    }
    Ok(bytes)
}

pub(super) fn local_repair_source(
    public: Option<&[u8]>,
) -> Result<
    (
        hbb_common::UpdateManifest,
        PathBuf,
        DirectoryBackup,
        u64,
        PathBuf,
        String,
        String,
        Option<super::local_repair_plan::ApprovalSource>,
    ),
    String,
> {
    let (inventory, build, fallback) = r3_verified(public)?;
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bundle: Bundle = serde_json::from_slice(&read(&root.join("bundle.json"), 65536, false)?)
        .map_err(|_| "local repair bundle malformed")?;
    let approval_source = if bundle.schema == 2 {
        let receipt = bundle
            .installation_receipt
            .clone()
            .ok_or("local repair installation receipt missing")?;
        let value: serde_json::Value =
            serde_json::from_slice(&read(&root.join(&receipt.relative_path), 65536, false)?)
                .map_err(|_| "local repair receipt malformed")?;
        Some(super::local_repair_plan::ApprovalSource {
            source_kind: "installed_msi".into(),
            source_id: value["source_id"]
                .as_str()
                .ok_or("local repair source id missing")?
                .into(),
            source_reference: bundle.source_reference.clone(),
            installation_receipt: receipt,
        })
    } else {
        None
    };
    let manifest =
        serde_json::from_slice(&read(&root.join("approved-manifest.json"), 262144, false)?)
            .map_err(|_| "local repair manifest malformed")?;
    Ok((
        manifest,
        root.join(&bundle.package.relative_path),
        inventory,
        build,
        fallback,
        bundle.installation_id,
        bundle.source_transaction,
        approval_source,
    ))
}

pub(super) fn local_repair_approved_bytes(
    root: &Path,
    source: &super::local_repair_plan::ApprovalSource,
    key: &str,
) -> Result<Vec<u8>, String> {
    let bytes = read(&root.join("approved-manifest.json"), 262144, false)?;
    let raw: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "local repair approval manifest malformed")?;
    if package_gate::package_reference(&raw, key)? != source.source_reference {
        return Err("local repair approval reference mismatch".into());
    }
    Ok(bytes)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "rescue output occupied or unavailable")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "rescue output write failed")?;
    workspace::verify_file(path)
}

fn describe(root: &Path, name: &str, limit: u64) -> Result<Artifact, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let path = root.join(name);
    workspace::verify_file(&path)?;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .map_err(|_| "rescue asset unavailable")?;
    let size = file
        .metadata()
        .map_err(|_| "rescue asset metadata unavailable")?
        .len();
    if size == 0 || size > limit {
        return Err("rescue asset size invalid".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    let mut total = 0u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "rescue asset read failed")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > size {
            return Err("rescue asset size changed".into());
        }
        hash.update(&buffer[..count]);
    }
    if total != size {
        return Err("rescue asset size changed".into());
    }
    Ok(Artifact {
        relative_path: name.into(),
        size,
        sha256: hex::encode(hash.finalize()),
    })
}

fn copy(root: &Path, source: &Path, name: &str, limit: u64) -> Result<Artifact, String> {
    use std::os::windows::fs::OpenOptionsExt;
    workspace::verify_restore_target(source, false)?;
    let input = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(source)
        .map_err(|_| "rescue source unavailable")?;
    let size = input
        .metadata()
        .map_err(|_| "rescue source metadata unavailable")?
        .len();
    if size == 0 || size > limit {
        return Err("rescue source size invalid".into());
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(name))
        .map_err(|_| "rescue output occupied")?;
    let copied =
        std::io::copy(&mut input.take(limit + 1), &mut output).map_err(|_| "rescue copy failed")?;
    output.sync_all().map_err(|_| "rescue copy sync failed")?;
    drop(output);
    if copied != size {
        return Err("rescue source changed".into());
    }
    describe(root, name, limit)
}

fn signed_target<'a>(
    manifest: &'a UpdateManifest,
    key: &str,
    build: u64,
    public: Option<&[u8]>,
) -> Result<&'a hbb_common::UpdateTarget, String> {
    #[cfg(test)]
    if let Some(public) = public {
        return package_gate::verify_with_key(manifest, key, build, "fixture-key", public);
    }
    let _ = public;
    package_gate::verify(manifest, key, build)
}

fn check_local(layout: &Layout, plan: &InstallPlan) -> Result<(), String> {
    let state = layout.agent.join("state");
    let boundary = if layout.agent == Path::new(r"C:\ProgramData\RustDesk\ManagementAgent") {
        Path::new(r"C:\ProgramData\RustDesk")
    } else {
        &layout.agent
    };
    verify_existing_directory(&state, boundary)?;
    for name in ["disable-agent", "disable-recovery", "maintenance.json"] {
        match std::fs::symlink_metadata(state.join(name)) {
            Ok(_) => return Err("rescue locally blocked".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("rescue local gate unreadable".into()),
        }
    }
    workspace::verify_restore_target(&state.join("service-recovery-enabled"), false)?;
    workspace::verify_restore_target(&state.join("agent.json"), false)?;
    let value: serde_json::Value =
        serde_json::from_slice(&read(&state.join("agent.json"), 65536, true)?)
            .map_err(|_| "rescue state malformed")?;
    if value["agent_installation_id"] != plan.installation_id
        || value["target_key"] != plan.target_key
        || value["management_agent_enabled"] != true
        || value["disabled"] != false
    {
        return Err("rescue state identity or policy mismatch".into());
    }
    let bytes = read(&layout.config, 65536, true)?;
    let config: hbb_common::toml::Value = hbb_common::toml::from_str(
        std::str::from_utf8(&bytes).map_err(|_| "rescue intent encoding")?,
    )
    .map_err(|_| "rescue intent malformed")?;
    match config
        .get("options")
        .and_then(|options| options.get("stop-service"))
    {
        None => {}
        Some(value) if matches!(value.as_str(), Some("" | "N")) => {}
        _ => return Err("rescue user stopped".into()),
    }
    let transactions = layout.agent.join("transactions");
    if !transactions.exists() {
        // A lab transaction can exercise an installed layout before its first real transaction.
        #[cfg(test)]
        if layout.transaction.starts_with(r"C:\RustDeskAgentLab") {
            return Ok(());
        }
    }
    verify_existing_directory(&transactions, boundary)?;
    for (count, entry) in std::fs::read_dir(&transactions)
        .map_err(|_| "rescue transactions unreadable")?
        .enumerate()
    {
        if count >= 64 {
            return Err("rescue transaction capacity exceeded".into());
        }
        let path = entry
            .map_err(|_| "rescue transaction entry unreadable")?
            .path();
        workspace::verify(&path)?;
        if path != layout.transaction {
            match std::fs::symlink_metadata(path.join("rotation.json")) {
                Ok(_) => {
                    let rotation: serde_json::Value =
                        serde_json::from_slice(&read(&path.join("rotation.json"), 4096, false)?)
                            .map_err(|_| "rescue foreign rotation malformed")?;
                    if rotation["phase"] != "published" {
                        return Err(
                            "rescue rotation reconciliation required for another transaction"
                                .into(),
                        );
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("rescue rotation observation failed".into()),
            }
        }
        let tx = Transaction::decode(&read(&path.join("transaction.json"), 65536, false)?)?;
        if !matches!(tx.phase, Phase::Committed | Phase::RolledBack) {
            return Err("rescue active transaction".into());
        }
    }
    Ok(())
}


fn valid_identity(bytes: &[u8]) -> Result<(), String> {
    #[derive(Deserialize)]
    struct Identity {
        #[serde(default)]
        id: String,
        #[serde(default)]
        enc_id: String,
        key_pair: (Vec<u8>, Vec<u8>),
    }
    let identity: Identity = hbb_common::toml::from_str(
        std::str::from_utf8(bytes).map_err(|_| "rescue identity encoding")?,
    )
    .map_err(|_| "rescue identity malformed")?;
    use hbb_common::sodiumoxide::crypto::sign;
    hbb_common::sodiumoxide::init().map_err(|_| "rescue identity crypto unavailable")?;
    let secret = sign::SecretKey::from_slice(&identity.key_pair.0)
        .ok_or("rescue identity secret invalid")?;
    let public = sign::PublicKey::from_slice(&identity.key_pair.1)
        .ok_or("rescue identity public invalid")?;
    let proof = b"rustdesk-rescue-identity-v1";
    if (identity.id.is_empty() && identity.enc_id.is_empty())
        || !sign::verify_detached(&sign::sign_detached(proof, &secret), proof, &public)
    {
        return Err("rescue identity invalid".into());
    }
    Ok(())
}

fn prepare(
    layout: &Layout,
    public: Option<&[u8]>,
    check_installed: impl Fn(
        &msi_metadata::PackageIdentity,
        &str,
        &hbb_common::UpdateTarget,
        u64,
    ) -> Result<serde_json::Value, String>,
) -> Result<serde_json::Value, String> {
    workspace::verify(&layout.transaction)?;
    let plan = InstallPlan::decode(&read(
        &layout.transaction.join("prepared.json"),
        65536,
        false,
    )?)?;
    let transaction = Transaction::decode(&read(
        &layout.transaction.join("transaction.json"),
        65536,
        false,
    )?)?;
    plan.bind_transaction(&transaction)?;
    if transaction.phase != Phase::Committed
        || !matches!(
            plan.target_key.as_str(),
            "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"
        )
        || layout
            .transaction
            .file_name()
            .and_then(|name| name.to_str())
            != Some(&plan.transaction_id)
    {
        return Err("rescue source is not a committed MSI transaction".into());
    }
    let outcome: serde_json::Value = serde_json::from_slice(&read(
        &layout.transaction.join("execution-result.json"),
        65536,
        false,
    )?)
    .map_err(|_| "rescue execution result malformed")?;
    if outcome["schema"] != 1
        || outcome["transaction_id"] != plan.transaction_id
        || outcome["command_id"]
            != serde_json::to_value(&plan.command_id).map_err(|_| "rescue command encoding")?
        || outcome["phase"] != "committed"
        || outcome["reboot_required"] != false
        || outcome.get("error") != Some(&serde_json::Value::Null)
    {
        return Err("rescue committed result mismatch".into());
    }
    let _guard = InstallGuard::try_acquire()
        .map_err(|_| "rescue install guard unavailable")?
        .ok_or("rescue installation busy")?;
    check_local(layout, &plan)?;
    let destination = &layout.destination;
    if destination.exists() {
        workspace::verify(&destination)?;
    }
    let manifest_bytes = read(
        &layout.transaction.join("approved-manifest.json"),
        262144,
        false,
    )?;
    let manifest: UpdateManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| "rescue manifest malformed")?;
    let target = signed_target(&manifest, &plan.target_key, plan.target_build_seq, public)?;
    let (version, build, _) = base::update::update_target_release(&manifest, target)
        .map_err(|_| "rescue target release invalid")?;
    if version != plan.target_version
        || build != plan.target_build_seq
        || target.sha256 != plan.package.sha256
        || target.size != plan.package.size
    {
        return Err("rescue exact target mismatch".into());
    }
    let name = package_gate::cache_name(&target.primary, "msi")?;
    if plan.package.relative_path != name {
        return Err("rescue original package basename mismatch".into());
    }
    let package = layout.transaction.join(&name);
    let _locked = package_gate::lock_verified_file(&package, target)?;
    let msi = msi_metadata::inspect(&package)?;
    if msi.product_name != "RustDesk" {
        return Err("rescue MSI product mismatch".into());
    }
    let current = destination.join("current");
    let rotation_binding = rescue_rotation::binding(&layout.transaction)?;
    let published = if let Some((old, new, _)) = &rotation_binding {
        rescue_rotation::digest(&destination.join("staging"))?.is_none()
            && rescue_rotation::digest(&current)?.as_deref() == Some(new.as_str())
            && rescue_rotation::digest(&destination.join("previous"))?.as_deref()
                == Some(old.as_str())
    } else {
        false
    };
    let installed_evidence = if published {
        serde_json::Value::Null
    } else {
        check_installed(&msi, &name, target, build)?
    };
    let identity = read(&layout.identity, 65536, true)?;
    valid_identity(&identity)?;
    plan.identity_backup.verify(&layout.transaction)?;
    if identity
        != read(
            &layout.transaction.join(&plan.identity_backup.relative_path),
            65536,
            false,
        )?
    {
        return Err("rescue original identity mismatch".into());
    }
    let config = read(&layout.config, 65536, true)?;
    let reference = package_gate::package_reference(
        &serde_json::from_slice(&manifest_bytes).map_err(|_| "rescue manifest malformed")?,
        &plan.target_key,
    )?;
    if let Some((old, new, previous)) = rotation_binding {
        let stage_digest = rescue_rotation::digest(&destination.join("staging"))?;
        let candidate = if stage_digest.as_deref() == Some(new.as_str()) {
            destination.join("staging")
        } else {
            current.clone()
        };
        let candidate_bundle = rotation_compatible(&candidate, &plan, &identity, public)?;
        if candidate_bundle.source_transaction != plan.transaction_id
            || candidate_bundle.source_reference != reference
        {
            return Err("rescue resumed rotation source conflict".into());
        }
        // A bound postimage already published is reconciliation, not preparation of new bytes.
        // Keep current intent/identity checks above, but do not roll back a later Config2 preference.
        if !published {
            rotation_installed_source(destination, &installed_evidence)?;
            if candidate_bundle.config_sha256 != hex::encode(Sha256::digest(&config)) {
                return Err("rescue resumed rotation configuration changed".into());
            }
            let candidate_inventory = DirectoryBackup::decode(&read(
                &candidate.join("client/manifest.json"),
                8 * 1024 * 1024,
                false,
            )?)?;
            if !candidate_inventory.matches_directory(&layout.client)? {
                return Err("rescue resumed installed client mismatch".into());
            }
        }
        rescue_rotation::rotate(
            destination,
            &layout.transaction,
            &old,
            &new,
            previous.as_deref(),
            |path| {
                rotation_compatible(path, &plan, &identity, public)?;
                Ok(())
            },
            None,
        )?;
        let report = verify(&current, &plan.installation_id, &plan.target_key, public)?;
        if report["source_transaction"] != plan.transaction_id
            || report["source_reference"] != reference
        {
            return Err("rescue resumed rotation source conflict".into());
        }
        return Ok(report);
    }
    let old_digest = rescue_rotation::digest(&current)?;
    if old_digest.is_some() {
        let old = rotation_compatible(&current, &plan, &identity, public)?;
        if old.source_transaction == plan.transaction_id && old.source_reference == reference {
            if old.config_sha256 != hex::encode(Sha256::digest(&config)) {
                return Err("rescue same-source configuration changed".into());
            }
            return verify(&current, &plan.installation_id, &plan.target_key, public);
        }
    }
    if std::fs::symlink_metadata(destination.join("staging")).is_ok() {
        return Err("rescue staging occupied; preserve previous assets".into());
    }
    if old_digest.is_some() {
        rotation_installed_source(destination, &installed_evidence)?;
    }
    if !destination.exists() {
        workspace::create(&destination)?;
    }
    let staging = destination.join("staging");
    workspace::create(&staging)?;
    let package = copy(&staging, &package, &name, 2 * 1024 * 1024 * 1024)?;
    write_new(&staging.join("approved-manifest.json"), &manifest_bytes)?;
    // Extract the signed bytes again rather than treating a mutable transaction inventory as authority.
    let extracted = match public {
        #[cfg(test)]
        Some(key) => msi_assets::extract_fixture(
            &staging,
            &staging.join(&name),
            &manifest,
            &plan.target_key,
            key,
        )?,
        _ => msi_assets::extract(
            &staging,
            &staging.join(&name),
            &manifest,
            &plan.target_key,
            build,
        )?,
    };
    let client = directory_backup::capture(&extracted.client, &staging.join("client"))?;
    if !client.matches_directory(&layout.client)? {
        return Err("rescue current client mismatch".into());
    }
    let agent = copy(&staging, &extracted.agent, "agent.exe", 512 * 1024 * 1024)?;
    let reference = hbb_common::UpdateTarget {
        size: agent.size,
        sha256: agent.sha256.clone(),
        ..Default::default()
    };
    let _agent = package_gate::lock_verified_file(&layout.agent_exe, &reference)?;
    let _bootstrap = package_gate::lock_verified_file(
        &layout.bootstrap.join("rustdesk-recovery-bootstrap.exe"),
        &reference,
    )?;
    let bootstrap = copy(
        &staging,
        &layout.bootstrap.join("rustdesk-recovery-bootstrap.exe"),
        "bootstrap.exe",
        512 * 1024 * 1024,
    )?;
    let bootstrap_manifest = copy(
        &staging,
        &layout.bootstrap.join("bootstrap.json"),
        "bootstrap.json",
        65536,
    )?;
    write_new(
        &staging.join("identity.dpapi"),
        &machine_protect(&identity, false)?,
    )?;
    write_new(
        &staging.join("config.dpapi"),
        &machine_protect(&config, false)?,
    )?;
    let bundle = Bundle {
        schema: 1,
        installation_id: plan.installation_id.clone(),
        source_transaction: plan.transaction_id.clone(),
        source_kind: None,
        installation_receipt: None,
        source_reference: package_gate::package_reference(
            &serde_json::from_slice(&manifest_bytes).map_err(|_| "rescue manifest malformed")?,
            &plan.target_key,
        )?,
        target_key: plan.target_key.clone(),
        version: version.into(),
        build_seq: build,
        verified_at: now()?,
        package,
        client_manifest: describe(&staging, "client/manifest.json", 8 * 1024 * 1024)?,
        agent,
        bootstrap,
        bootstrap_manifest,
        identity: describe(&staging, "identity.dpapi", 131072)?,
        config: describe(&staging, "config.dpapi", 131072)?,
        identity_sha256: hex::encode(Sha256::digest(&identity)),
        config_sha256: hex::encode(Sha256::digest(&config)),
    };
    write_new(
        &staging.join("bundle.json"),
        &serde_json::to_vec(&bundle).map_err(|_| "rescue bundle encoding failed")?,
    )?;
    let mut report = verify(&staging, &plan.installation_id, &plan.target_key, public)?;
    report["installed_source_check"] = installed_evidence;
    check_local(layout, &plan)?;
    if read(&layout.identity, 65536, true)? != identity
        || read(&layout.config, 65536, true)? != config
    {
        return Err("rescue live identity or configuration changed".into());
    }
    if let Some(old) = old_digest {
        let installed_recheck = check_installed(&msi, &name, target, build)?;
        rotation_installed_source(destination, &installed_recheck)?;
        let previous_path = destination.join("previous");
        let previous = rescue_rotation::digest(&previous_path)?;
        if previous.is_some() {
            rotation_compatible(&previous_path, &plan, &identity, public)?;
        }
        let new = rescue_rotation::digest(&staging)?.ok_or("rescue staged metadata absent")?;
        #[cfg(not(test))]
        let fault = None;
        #[cfg(test)]
        let fault = ROTATION_FAULT.with(|value| value.get());
        rescue_rotation::rotate(
            destination,
            &layout.transaction,
            &old,
            &new,
            previous.as_deref(),
            |path| {
                rotation_compatible(path, &plan, &identity, public)?;
                Ok(())
            },
            fault,
        )?;
    } else {
        publish(&staging, &current)?;
    }
    Ok(report)
}

fn rotation_compatible(
    path: &Path,
    plan: &InstallPlan,
    identity: &[u8],
    public: Option<&[u8]>,
) -> Result<Bundle, String> {
    verify(path, &plan.installation_id, &plan.target_key, public)?;
    let old: Bundle = serde_json::from_slice(&read(&path.join("bundle.json"), 65536, false)?)
        .map_err(|_| "rescue rotation bundle malformed")?;
    if old.build_seq > plan.target_build_seq {
        return Err("rescue rotation downgrade refused".into());
    }
    if machine_protect(&read(&path.join("identity.dpapi"), 131072, false)?, true)? != identity {
        return Err("rescue rotation original identity mismatch".into());
    }
    Ok(old)
}

fn verify(
    root: &Path,
    installation: &str,
    key: &str,
    public: Option<&[u8]>,
) -> Result<serde_json::Value, String> {
    workspace::verify(root)?;
    let bundle: Bundle = serde_json::from_slice(&read(&root.join("bundle.json"), 65536, false)?)
        .map_err(|_| "rescue bundle malformed")?;
    first_install::verify_source(root, &bundle)?;
    if !token(&bundle.installation_id, 32)
        || bundle.installation_id != installation
        || bundle.target_key != key
        || bundle.build_seq == 0
        || bundle.verified_at == 0
        || !matches!(
            key,
            "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"
        )
    {
        return Err("rescue bundle identity mismatch".into());
    }
    for (artifact, name) in [
        (&bundle.client_manifest, "client/manifest.json"),
        (&bundle.agent, "agent.exe"),
        (&bundle.bootstrap, "bootstrap.exe"),
        (&bundle.bootstrap_manifest, "bootstrap.json"),
        (&bundle.identity, "identity.dpapi"),
        (&bundle.config, "config.dpapi"),
    ] {
        if artifact.relative_path != name {
            return Err("rescue asset reference mismatch".into());
        }
        workspace::verify_file(&root.join(name))?;
        artifact.verify(root)?;
    }
    let raw: serde_json::Value =
        serde_json::from_slice(&read(&root.join("approved-manifest.json"), 262144, false)?)
            .map_err(|_| "rescue manifest malformed")?;
    if package_gate::package_reference(&raw, key)? != bundle.source_reference {
        return Err("rescue approved reference mismatch".into());
    }
    let manifest: UpdateManifest =
        serde_json::from_value(raw).map_err(|_| "rescue manifest malformed")?;
    let target = signed_target(&manifest, key, bundle.build_seq, public)?;
    let (version, build, _) = base::update::update_target_release(&manifest, target)
        .map_err(|_| "rescue release invalid")?;
    if version != bundle.version
        || build != bundle.build_seq
        || bundle.package.relative_path != package_gate::cache_name(&target.primary, "msi")?
        || bundle.package.sha256 != target.sha256
        || bundle.package.size != target.size
    {
        return Err("rescue package binding mismatch".into());
    }
    workspace::verify_file(&root.join(&bundle.package.relative_path))?;
    let _package =
        package_gate::lock_verified_file(&root.join(&bundle.package.relative_path), target)?;
    let inventory = DirectoryBackup::decode(&read(
        &root.join("client/manifest.json"),
        8 * 1024 * 1024,
        false,
    )?)?;
    inventory.verify(&root.join("client"))?;
    for file in &inventory.files {
        workspace::verify_file(&root.join("client/files").join(&file.path))?;
    }
    if !inventory
        .files
        .iter()
        .any(|file| file.path == "RustDesk.exe")
        || !inventory
            .files
            .iter()
            .any(|file| file.path == "librustdesk.dll")
        || !inventory
            .directories
            .iter()
            .any(|directory| directory == "data/flutter_assets")
    {
        return Err("rescue client incomplete".into());
    }
    let bootstrap: serde_json::Value =
        serde_json::from_slice(&read(&root.join("bootstrap.json"), 65536, false)?)
            .map_err(|_| "rescue bootstrap malformed")?;
    if bootstrap["schema"] != 1
        || bootstrap["agent_sha256"] != bundle.agent.sha256
        || bundle.agent.sha256 != bundle.bootstrap.sha256
        || bootstrap["edition"]
            != if key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            }
        || bootstrap["package_kind"] != "msi"
        || bootstrap["normal_install_path"] != r"C:\Program Files\RustDesk"
    {
        return Err("rescue bootstrap binding mismatch".into());
    }
    let identity = machine_protect(&read(&root.join("identity.dpapi"), 131072, false)?, true)?;
    let config = machine_protect(&read(&root.join("config.dpapi"), 131072, false)?, true)?;
    if hex::encode(Sha256::digest(&identity)) != bundle.identity_sha256
        || hex::encode(Sha256::digest(&config)) != bundle.config_sha256
    {
        return Err("rescue credentials mismatch".into());
    }
    valid_identity(&identity)?;
    Ok(
        serde_json::json!({"schema":1,"rescue_readiness":"degraded","client":"verified","offline_package":"verified",
        "credentials":"verified","agent_backup":"verified","bootstrap_backup":"verified","recovery_entry":"not_integrated",
        "service_recovery":"unknown","online_source":"not_probed","source_kind":bundle.source_kind.as_deref().unwrap_or("committed_transaction"),"source_transaction":if bundle.source_transaction.is_empty(){None}else{Some(&bundle.source_transaction)},
        "source_reference":bundle.source_reference,"target_key":bundle.target_key,"version":bundle.version,"build_seq":bundle.build_seq,
        "verified_at":now()?,"prepared_at":bundle.verified_at}),
    )
}

fn now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(|_| "rescue clock invalid".into())
}

pub(super) fn trusted_agent_backup(
    expected_hash: &str,
    edition: &str,
    kind: &str,
    build: u64,
    public: Option<&[u8]>,
) -> Result<(PathBuf, u64), String> {
    if kind != "msi" || !matches!(edition, "sos" | "standard") {
        return Err("agent restore unsupported package identity".into());
    }
    let (inventory, bundle_build, _) = r3_verified(public)?;
    let _ = inventory;
    if bundle_build != build {
        return Err("agent restore different build refused".into());
    }
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let bundle: Bundle = serde_json::from_slice(&read(&root.join("bundle.json"), 65536, false)?)
        .map_err(|_| "agent restore bundle malformed")?;
    agent_backup_compatible(
        &bundle,
        root,
        expected_hash,
        edition,
        kind,
        build,
        &read(&hbb_common::config::Config::file(), 65536, true)?,
    )?;
    let source = root.join("agent.exe");
    package_gate::lock_verified_file(
        &source,
        &hbb_common::UpdateTarget {
            size: bundle.agent.size,
            sha256: expected_hash.into(),
            ..Default::default()
        },
    )?;
    Ok((source, bundle.agent.size))
}

fn agent_backup_compatible(
    bundle: &Bundle,
    root: &Path,
    expected_hash: &str,
    edition: &str,
    kind: &str,
    build: u64,
    identity: &[u8],
) -> Result<(), String> {
    if kind != "msi" || bundle.build_seq != build {
        return Err("agent restore different build or kind refused".into());
    }
    if bundle.target_key != format!("windows-x86_64-msi-{edition}")
        || bundle.agent.sha256 != expected_hash
        || bundle.bootstrap.sha256 != expected_hash
    {
        return Err("agent restore bootstrap approval mismatch".into());
    }
    if machine_protect(&read(&root.join("identity.dpapi"), 131072, false)?, true)? != identity {
        return Err("agent restore original identity mismatch".into());
    }
    Ok(())
}

#[cfg(test)]
pub fn agent_backup_fixture(
    root: &Path,
    installation: &str,
    key: &str,
    public: &[u8],
    hash: &str,
    build: u64,
    identity: &[u8],
) -> Result<(), String> {
    verify(root, installation, key, Some(public))?;
    let bundle: Bundle = serde_json::from_slice(&read(&root.join("bundle.json"), 65536, false)?)
        .map_err(|_| "agent restore bundle malformed")?;
    agent_backup_compatible(
        &bundle,
        root,
        hash,
        if key.ends_with("-sos") {
            "sos"
        } else {
            "standard"
        },
        "msi",
        build,
        identity,
    )
}

// Read-only verification has its own cooperative budget so a status worker never invokes
// install/repair helpers or the unbounded legacy backup scanner.
pub(super) fn verify_readonly_status(
    root: &Path,
    installation: &str,
    key: &str,
    public: Option<&[u8]>,
) -> Result<serde_json::Value, String> {
    use std::{
        collections::BTreeSet,
        os::windows::fs::{MetadataExt, OpenOptionsExt},
        time::{Duration, Instant},
    };
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut bytes_left = 12u64 * 1024 * 1024 * 1024;
    let check_time = || -> Result<(), String> {
        if Instant::now() >= deadline {
            Err("rescue verification deadline exceeded".into())
        } else {
            Ok(())
        }
    };
    workspace::verify(root)?;
    let bundle: Bundle = serde_json::from_slice(&read(&root.join("bundle.json"), 65536, false)?)
        .map_err(|_| "rescue bundle malformed")?;
    first_install::verify_source(root, &bundle)?;
    if bundle.installation_id != installation
        || !token(installation, 32)
        || bundle.target_key != key
        || bundle.build_seq == 0
        || !matches!(
            key,
            "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"
        )
    {
        return Err("rescue bundle identity mismatch".into());
    }
    let raw: serde_json::Value =
        serde_json::from_slice(&read(&root.join("approved-manifest.json"), 262144, false)?)
            .map_err(|_| "rescue manifest malformed")?;
    if package_gate::package_reference(&raw, key)? != bundle.source_reference {
        return Err("rescue approved reference mismatch".into());
    }
    let manifest: UpdateManifest =
        serde_json::from_value(raw).map_err(|_| "rescue manifest malformed")?;
    let target = signed_target(&manifest, key, bundle.build_seq, public)?;
    let (version, build, _) = base::update::update_target_release(&manifest, target)
        .map_err(|_| "rescue target malformed")?;
    if version != bundle.version
        || build != bundle.build_seq
        || target.size > 2 * 1024 * 1024 * 1024
        || bundle.package.size != target.size
        || bundle.package.sha256 != target.sha256
        || bundle.package.relative_path != package_gate::cache_name(&target.primary, "msi")?
    {
        return Err("rescue package binding mismatch".into());
    }
    let mut verify_file = |path: &Path, size: u64, hash: &str| -> Result<(), String> {
        check_time()?;
        if !token(hash, 64) || size > bytes_left {
            return Err("rescue verification byte budget exceeded".into());
        }
        workspace::verify_file(path)?;
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x00200000)
            .open(path)
            .map_err(|_| "rescue verification file unavailable")?;
        if file
            .metadata()
            .map_err(|_| "rescue verification metadata unavailable")?
            .len()
            != size
        {
            return Err("rescue verification size mismatch".into());
        }
        let mut digest = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            check_time()?;
            let count = file
                .read(&mut buffer)
                .map_err(|_| "rescue verification read failed")?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or("rescue verification size overflow")?;
            if total > size || count as u64 > bytes_left {
                return Err("rescue verification byte budget exceeded".into());
            }
            bytes_left -= count as u64;
            digest.update(&buffer[..count]);
        }
        if total != size || hex::encode(digest.finalize()) != hash {
            return Err("rescue verification hash mismatch".into());
        }
        Ok(())
    };
    verify_file(
        &root.join(&bundle.package.relative_path),
        bundle.package.size,
        &bundle.package.sha256,
    )?;
    for (artifact, name, limit) in [
        (
            &bundle.client_manifest,
            "client/manifest.json",
            8 * 1024 * 1024,
        ),
        (&bundle.agent, "agent.exe", 512 * 1024 * 1024),
        (&bundle.bootstrap, "bootstrap.exe", 512 * 1024 * 1024),
        (&bundle.bootstrap_manifest, "bootstrap.json", 65536),
        (&bundle.identity, "identity.dpapi", 131072),
        (&bundle.config, "config.dpapi", 131072),
    ] {
        if artifact.relative_path != name || artifact.size == 0 || artifact.size > limit {
            return Err("rescue artifact reference invalid".into());
        }
        verify_file(&root.join(name), artifact.size, &artifact.sha256)?;
    }
    let inventory = DirectoryBackup::decode(&read(
        &root.join("client/manifest.json"),
        8 * 1024 * 1024,
        false,
    )?)?;
    if !inventory.files.iter().any(|f| f.path == "RustDesk.exe")
        || !inventory.files.iter().any(|f| f.path == "librustdesk.dll")
        || !inventory
            .directories
            .iter()
            .any(|d| d == "data/flutter_assets")
    {
        return Err("rescue client incomplete".into());
    }
    let client = root.join("client/files");
    let mut pending = vec![client.clone()];
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut entries = 0;
    while let Some(directory) = pending.pop() {
        check_time()?;
        let metadata = std::fs::symlink_metadata(&directory)
            .map_err(|_| "rescue verification directory unavailable")?;
        if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
            return Err("rescue verification reparse directory refused".into());
        }
        verify_existing_directory(&directory, root)?;
        for entry in
            std::fs::read_dir(&directory).map_err(|_| "rescue verification directory unreadable")?
        {
            check_time()?;
            entries += 1;
            if entries > 20000 {
                return Err("rescue verification entry budget exceeded".into());
            }
            let path = entry
                .map_err(|_| "rescue verification entry unreadable")?
                .path();
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|_| "rescue verification metadata unavailable")?;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("rescue verification reparse refused".into());
            }
            let relative = path
                .strip_prefix(&client)
                .map_err(|_| "rescue verification path invalid")?
                .to_str()
                .ok_or("rescue verification path encoding")?
                .replace('\\', "/");
            if metadata.is_dir() {
                directories.insert(relative);
                pending.push(path);
            } else if metadata.is_file() {
                files.insert(relative);
            } else {
                return Err("rescue verification object invalid".into());
            }
        }
    }
    if files != inventory.files.iter().map(|f| f.path.clone()).collect()
        || directories != inventory.directories.iter().cloned().collect()
    {
        return Err("rescue verification file set mismatch".into());
    }
    for file in &inventory.files {
        verify_file(&client.join(&file.path), file.size, &file.sha256)?;
    }
    drop(verify_file);
    let bootstrap: serde_json::Value =
        serde_json::from_slice(&read(&root.join("bootstrap.json"), 65536, false)?)
            .map_err(|_| "rescue bootstrap malformed")?;
    if bootstrap["schema"] != 1
        || bootstrap["agent_sha256"] != bundle.agent.sha256
        || bundle.bootstrap.sha256 != bundle.agent.sha256
        || bootstrap["edition"]
            != if key.ends_with("-sos") {
                "sos"
            } else {
                "standard"
            }
        || bootstrap["package_kind"] != "msi"
        || bootstrap["normal_install_path"] != r"C:\Program Files\RustDesk"
    {
        return Err("rescue bootstrap binding mismatch".into());
    }
    let identity = machine_protect(&read(&root.join("identity.dpapi"), 131072, false)?, true)?;
    let config = machine_protect(&read(&root.join("config.dpapi"), 131072, false)?, true)?;
    if hex::encode(Sha256::digest(&identity)) != bundle.identity_sha256
        || hex::encode(Sha256::digest(&config)) != bundle.config_sha256
    {
        return Err("rescue credential digest mismatch".into());
    }
    valid_identity(&identity)?;
    check_time()?;
    Ok(
        serde_json::json!({"source_reference":bundle.source_reference,"source_transaction":bundle.source_transaction,"source_kind":bundle.source_kind.as_deref().unwrap_or("committed_transaction"),"version":bundle.version,"build_seq":bundle.build_seq,
        "verified_bytes":12u64*1024*1024*1024-bytes_left,"verified_entries":entries,
        "client":"verified","offline_package":"verified","credentials":"verified","agent_backup":"verified","bootstrap_backup":"verified"}),
    )
}

pub(super) fn r3_verified(
    public: Option<&[u8]>,
) -> Result<(DirectoryBackup, u64, PathBuf), String> {
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let agent = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent");
    let state: serde_json::Value =
        serde_json::from_slice(&read(&agent.join("state/agent.json"), 65536, true)?)
            .map_err(|_| "R3 state malformed")?;
    let installation = state["agent_installation_id"]
        .as_str()
        .ok_or("R3 installation absent")?;
    let key = state["target_key"].as_str().ok_or("R3 target absent")?;
    verify(root, installation, key, public)?;
    let bundle: Bundle = serde_json::from_slice(&read(&root.join("bundle.json"), 65536, false)?)
        .map_err(|_| "R3 bundle malformed")?;
    let identity = hbb_common::config::Config::file();
    let layout = Layout {
        agent: agent.into(),
        destination: root.into(),
        transaction: agent.join("transactions").join(&bundle.source_transaction),
        client: r"C:\Program Files\RustDesk".into(),
        agent_exe: agent.join("rustdesk-management-agent.exe"),
        config: identity.with_file_name("RustDesk2.toml"),
        identity,
        bootstrap: r"C:\ProgramData\RustDesk\RecoveryBootstrap".into(),
    };
    if bundle.schema == 2 {
        first_install::check_gates(&layout, installation, key)?;
    } else {
        let plan = InstallPlan::decode(&read(
            &layout.transaction.join("prepared.json"),
            65536,
            false,
        )?)?;
        if plan.installation_id != installation
            || plan.target_key != key
            || plan.target_build_seq != bundle.build_seq
        {
            return Err("R3 source binding mismatch".into());
        }
        check_local(&layout, &plan)?;
    }
    if machine_protect(&read(&root.join("identity.dpapi"), 131072, false)?, true)?
        != read(&layout.identity, 65536, true)?
    {
        return Err("R3 existing identity differs from bundle".into());
    }
    let inventory = DirectoryBackup::decode(&read(
        &root.join("client/manifest.json"),
        8 * 1024 * 1024,
        false,
    )?)?;
    let edition = if key.ends_with("-sos") {
        "sos"
    } else {
        "standard"
    };
    Ok((inventory, bundle.build_seq, fallback_path(edition)))
}

/// 保底客户端目录：标准版与 SOS 各自一份，位置由 edition 决定。
pub(super) fn fallback_path(edition: &str) -> PathBuf {
    Path::new(r"C:\ProgramData\RustDesk\Client")
        .join(edition)
        .join("RustDesk")
}

pub(super) fn r3_cycle_allowed() -> Result<bool, String> {
    let state = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state");
    verify_existing_directory(state, Path::new(r"C:\ProgramData\RustDesk"))?;
    for name in ["disable-agent", "disable-recovery", "maintenance.json"] {
        match std::fs::symlink_metadata(state.join(name)) {
            Ok(_) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("R3 cycle local gate unreadable".into()),
        }
    }
    workspace::verify_restore_target(&state.join("agent.json"), false)?;
    let value: serde_json::Value =
        serde_json::from_slice(&read(&state.join("agent.json"), 65536, true)?)
            .map_err(|_| "R3 cycle state malformed")?;
    if value["management_agent_enabled"] != true || value["disabled"] != false {
        return Ok(false);
    }
    if !state.join("service-recovery-enabled").is_file() {
        return Ok(false);
    }
    workspace::verify_restore_target(&state.join("service-recovery-enabled"), false)?;
    let bytes = read(&hbb_common::config::Config2::file(), 65536, true)?;
    let value: hbb_common::toml::Value = hbb_common::toml::from_str(
        std::str::from_utf8(&bytes).map_err(|_| "R3 cycle intent encoding")?,
    )
    .map_err(|_| "R3 cycle intent malformed")?;
    Ok(matches!(
        value
            .get("options")
            .and_then(|options| options.get("stop-service"))
            .and_then(|value| value.as_str()),
        None | Some("" | "N")
    ))
}

pub(super) fn r3_stage(public: Option<&[u8]>) -> Result<(PathBuf, u64), String> {
    let (inventory, build, destination) = r3_verified(public)?;
    if !r3_normal_requires_fallback(&inventory, Path::new(r"C:\Program Files\RustDesk"))? {
        return Err("R3 present normal client must use normal recovery".into());
    }
    if r3_existing_fallback(&inventory, &destination)? {
        return Ok((destination, build));
    }
    let root = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\current");
    let staging = Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\rescue\r3-staging");
    workspace::create(staging)?;
    let manifest: UpdateManifest =
        serde_json::from_slice(&read(&root.join("approved-manifest.json"), 262144, false)?)
            .map_err(|_| "R3 manifest malformed")?;
    let bundle: Bundle = serde_json::from_slice(&read(&root.join("bundle.json"), 65536, false)?)
        .map_err(|_| "R3 bundle malformed")?;
    let package = root.join(&bundle.package.relative_path);
    let extracted = match public {
        #[cfg(test)]
        Some(key) => {
            msi_assets::extract_fixture(staging, &package, &manifest, &bundle.target_key, key)?
        }
        _ => msi_assets::extract(staging, &package, &manifest, &bundle.target_key, build)?,
    };
    if !inventory.matches_directory(&extracted.client)? {
        return Err("R3 signed image inventory mismatch".into());
    }
    let copied = directory_backup::capture(&extracted.client, &staging.join("client"))?;
    copied.verify(&staging.join("client"))?;
    for file in &copied.files {
        workspace::verify_file(&staging.join("client/files").join(&file.path))?;
    }
    r3_verified(public)?;
    let parent = destination.parent().ok_or("R3 fallback parent absent")?;
    for path in [Path::new(r"C:\ProgramData\RustDesk\Client"), parent] {
        if path.exists() {
            verify_existing_directory(path, Path::new(r"C:\ProgramData\RustDesk"))?;
        } else {
            workspace::create(path)?;
        }
    }
    publish(&staging.join("client/files"), &destination)?;
    if !copied.matches_directory(&destination)? {
        return Err("R3 published inventory mismatch".into());
    }
    Ok((destination, build))
}

fn r3_unoccupied(destination: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(destination) {
        Ok(_) => Err("R3 fallback occupied; preserve data".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("R3 fallback location unreadable".into()),
    }
}

pub(super) fn r3_normal_requires_fallback(
    inventory: &DirectoryBackup,
    path: &Path,
) -> Result<bool, String> {
    let _ = inventory;
    r3_normal_missing(path)
}

pub(super) fn r3_normal_missing(path: &Path) -> Result<bool, String> {
    use std::os::windows::fs::MetadataExt;
    if !path.is_absolute() {
        return Err("R3 normal path not absolute".into());
    }
    let mut absent = false;
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("R3 normal reparse refused".into());
                }
                if !metadata.is_dir() {
                    return Err("R3 normal path is not directory".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => absent = true,
            Err(_) => return Err("R3 normal directory unreadable".into()),
        }
    }
    if absent {
        return Ok(true);
    }
    match std::fs::symlink_metadata(path.join("RustDesk.exe")) {
        Ok(metadata) => {
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("R3 normal executable reparse refused".into());
            }
            if !metadata.is_file() {
                return Err("R3 normal executable type invalid".into());
            }
            workspace::verify_restore_target(&path.join("RustDesk.exe"), false)?;
            Ok(false)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err("R3 normal executable unreadable".into()),
    }
}

pub(super) fn r3_existing_fallback(
    inventory: &DirectoryBackup,
    path: &Path,
) -> Result<bool, String> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("R3 fallback unreadable".into()),
        Ok(_) => {}
    }
    let parent = path.parent().ok_or("R3 fallback parent missing")?;
    let boundary = if path.starts_with(r"C:\ProgramData\RustDesk") {
        Path::new(r"C:\ProgramData\RustDesk")
    } else {
        parent
    };
    verify_existing_directory(path, boundary)?;
    if !inventory.matches_directory(path)? {
        return Err("R3 occupied fallback inventory mismatch; preserve data".into());
    }
    for file in &inventory.files {
        workspace::verify_file(&path.join(&file.path))?;
    }
    Ok(true)
}

#[cfg(test)]
pub fn r3_classify_fixture(
    inventory: &DirectoryBackup,
    path: &Path,
    fallback: bool,
) -> Result<bool, String> {
    if fallback {
        r3_existing_fallback(inventory, path)
    } else {
        r3_normal_requires_fallback(inventory, path)
    }
}

#[cfg(test)]
pub fn r3_validate_fixture(
    root: &Path,
    public: &[u8],
    identity: &Path,
    destination: &Path,
) -> Result<(), String> {
    verify(
        root,
        &"c".repeat(32),
        "windows-x86_64-msi-sos",
        Some(public),
    )?;
    if machine_protect(&read(&root.join("identity.dpapi"), 131072, false)?, true)?
        != read(identity, 65536, true)?
    {
        return Err("R3 existing identity differs from bundle".into());
    }
    r3_unoccupied(destination)
}

fn publish(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    }
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 8) } == 0 {
        return Err("rescue atomic publication refused; staging retained".into());
    }
    Ok(())
}

fn machine_protect(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>, String> {
    #[repr(C)]
    struct Blob {
        size: u32,
        data: *mut u8,
    }
    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut std::ffi::c_void,
            prompt: *mut std::ffi::c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    if bytes.is_empty() || bytes.len() > 131072 {
        return Err("rescue credentials size invalid".into());
    }
    let input = Blob {
        size: bytes.len() as u32,
        data: bytes.as_ptr().cast_mut(),
    };
    let mut output = Blob {
        size: 0,
        data: std::ptr::null_mut(),
    };
    let success = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1 | 4,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err("rescue machine credentials protection failed".into());
    }
    let result = unsafe { std::slice::from_raw_parts(output.data, output.size as usize).to_vec() };
    unsafe {
        std::ptr::write_bytes(output.data, 0, output.size as usize);
        if !LocalFree(output.data.cast()).is_null() {
            return Err("rescue credential buffer cleanup failed".into());
        }
    }
    Ok(result)
}
