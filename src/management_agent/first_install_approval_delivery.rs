use super::{install_guard, msi_metadata, package_gate, workspace};
mod installed_state_acl {
    pub(super) use super::workspace;
    #[cfg(test)]
    use std::path::PathBuf;
    use std::{fs::File, path::Path};
    #[path = "../preservation_state_read.rs"]
    mod reader;
    pub(super) fn verify_installed_state(state: &Path, intent: &Path) -> Result<Vec<File>, String> {
        reader::verify(state, intent)
    }
    pub(super) fn hold_installed_root(root: &Path) -> Result<Vec<File>, String> {
        reader::hold_installed_root(root)
    }
    #[cfg(test)]
    pub(super) fn with_root_fixture<T>(root: &Path, run: impl FnOnce() -> T) -> T {
        reader::with_root_fixture(root, run)
    }
    #[cfg(test)]
    pub(super) fn with_delivery_fixture<T>(
        root: &Path,
        intent: &Path,
        run: impl FnOnce() -> T,
    ) -> T {
        let _scope = TEST_DELIVERY_ROOT.with(|v| {
            struct Reset(Option<PathBuf>);
            impl Drop for Reset {
                fn drop(&mut self) {
                    TEST_DELIVERY_ROOT.with(|v| *v.borrow_mut() = self.0.take());
                }
            }
            Reset(v.replace(Some(root.to_path_buf())))
        });
        reader::with_root_fixture(root, || reader::with_delivery_fixture(root, intent, run))
    }
    #[cfg(test)]
    std::thread_local! { static TEST_DELIVERY_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) }; }
}
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

const AGENT: &str = r"C:\ProgramData\RustDesk\ManagementAgent";
const LIMIT: usize = 262144;

pub struct CurrentInstallation<'a> {
    pub installation_id: &'a str,
    pub target_key: &'a str,
    pub current_build: u64,
    pub product_code: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinationOutcome {
    Delivered,
    WaitingSource,
    WaitingSignature,
    WaitingExactBuild,
}

pub fn coordinate_approval(
    current: CurrentInstallation<'_>,
    fetch_fixed_source: impl FnOnce() -> Result<Vec<u8>, String>,
    deliver: impl FnOnce(Request<'_>) -> Result<(), DeliveryError>,
) -> Result<CoordinationOutcome, DeliveryError> {
    if !matches!(
        current.target_key,
        "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"
    ) {
        return Err(DeliveryError::TargetMismatch);
    }
    let bytes = match fetch_fixed_source() {
        Ok(bytes) if bytes.len() <= LIMIT => bytes,
        _ => return Ok(CoordinationOutcome::WaitingSource),
    };
    let manifest: hbb_common::UpdateManifest = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(CoordinationOutcome::WaitingSignature),
    };
    let target = match package_gate::verify(&manifest, current.target_key, 1) {
        Ok(value) => value,
        Err(_) => return Ok(CoordinationOutcome::WaitingSignature),
    };
    let (_, build, _) = base::update::update_target_release(&manifest, target)
        .map_err(|_| DeliveryError::BuildMismatch)?;
    if current.current_build == 0 || build != current.current_build {
        return Ok(CoordinationOutcome::WaitingExactBuild);
    }
    deliver(Request {
        manifest: &bytes,
        installation_id: current.installation_id,
        target_key: current.target_key,
        current_build: current.current_build,
        product_code: current.product_code,
    })?;
    Ok(CoordinationOutcome::Delivered)
}

#[cfg(not(test))]
fn fetch_fixed_manifest() -> Result<Vec<u8>, String> {
    fetch_manifest_http(ManifestSource::Fixed)
}

enum ManifestSource {
    Fixed,
    #[cfg(test)]
    Loopback(std::net::SocketAddr, std::time::Duration),
}

fn fetch_manifest_http(source: ManifestSource) -> Result<Vec<u8>, String> {
    let (endpoint, budget) = match source {
        ManifestSource::Fixed => (
            "https://rdapi.yan.life/rd/update/v1/manifest/stable.json".to_owned(),
            std::time::Duration::from_secs(15),
        ),
        #[cfg(test)]
        ManifestSource::Loopback(address, budget) => {
            if !address.ip().is_loopback() || budget.is_zero() {
                return Err("fixture endpoint refused".into());
            }
            (
                format!("http://{address}/rd/update/v1/manifest/stable.json"),
                budget,
            )
        }
    };
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(budget)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "approval client unavailable")?;
    let response = client
        .get(endpoint)
        // Request timeout retains one async deadline through headers and body reads.
        .timeout(budget)
        .send()
        .map_err(|_| "approval source unavailable")?;
    if response.status() != reqwest::StatusCode::OK
        || response.content_length().is_some_and(|n| n > LIMIT as u64)
    {
        return Err("approval source refused".into());
    }
    let mut bytes = Vec::new();
    response
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "approval source read failed")?;
    if bytes.len() > LIMIT {
        return Err("approval source too large".into());
    }
    Ok(bytes)
}

#[cfg(test)]
pub fn fetch_loopback_fixture(
    address: std::net::SocketAddr,
    budget: std::time::Duration,
) -> Result<Vec<u8>, String> {
    fetch_manifest_http(ManifestSource::Loopback(address, budget))
}

#[cfg(not(test))]
fn current_product_code() -> Result<String, DeliveryError> {
    #[link(name = "msi")]
    extern "system" {
        fn MsiEnumProductsExW(
            product: *const u16,
            sid: *const u16,
            context: u32,
            index: u32,
            code: *mut u16,
            actual_context: *mut u32,
            actual_sid: *mut u16,
            sid_length: *mut u32,
        ) -> u32;
        fn MsiGetProductInfoExW(
            code: *const u16,
            sid: *const u16,
            context: u32,
            property: *const u16,
            value: *mut u16,
            count: *mut u32,
        ) -> u32;
    }
    let mut found = None;
    for index in 0..4096 {
        let mut code = [0u16; 39];
        let mut context = 0;
        let status = unsafe {
            MsiEnumProductsExW(
                std::ptr::null(),
                std::ptr::null(),
                4,
                index,
                code.as_mut_ptr(),
                &mut context,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status == 259 {
            return found.ok_or(DeliveryError::RegisteredSourceMismatch);
        }
        if status != 0 || context != 4 {
            return Err(DeliveryError::RegisteredSourceMismatch);
        }
        let query = |property: &str| -> Result<String, DeliveryError> {
            let property: Vec<u16> = property.encode_utf16().chain(Some(0)).collect();
            let mut buffer = vec![0u16; 32768];
            let mut count = buffer.len() as u32;
            let result = unsafe {
                MsiGetProductInfoExW(
                    code.as_ptr(),
                    std::ptr::null(),
                    4,
                    property.as_ptr(),
                    buffer.as_mut_ptr(),
                    &mut count,
                )
            };
            if result != 0 || count as usize >= buffer.len() {
                return Err(DeliveryError::RegisteredSourceMismatch);
            }
            String::from_utf16(&buffer[..count as usize])
                .map_err(|_| DeliveryError::RegisteredSourceMismatch)
        };
        if query("InstalledProductName")? != "RustDesk" {
            continue;
        }
        if !query("InstallLocation")?
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(r"C:\Program Files\RustDesk")
            || found.is_some()
        {
            return Err(DeliveryError::RegisteredSourceMismatch);
        }
        found = Some(
            String::from_utf16(&code[..38]).map_err(|_| DeliveryError::RegisteredSourceMismatch)?,
        );
    }
    Err(DeliveryError::RegisteredSourceMismatch)
}

#[cfg(not(test))]
pub fn coordinate_current_installation() -> Result<CoordinationOutcome, DeliveryError> {
    let agent = Path::new(AGENT);
    let _root_reader = installed_state_acl::hold_installed_root(agent)
        .map_err(|_| DeliveryError::UntrustedPath)?;
    let _installed_read = installed_state_acl::verify_installed_state(
        &agent.join("state"),
        &hbb_common::config::Config2::file(),
    )
    .map_err(|_| DeliveryError::UntrustedPath)?;
    let (_, bytes) = read_locked(&agent.join("state/agent.json"), 65536, false)?;
    let state: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| DeliveryError::IdentityMismatch)?;
    let installation = state["agent_installation_id"]
        .as_str()
        .ok_or(DeliveryError::IdentityMismatch)?;
    let key = state["target_key"]
        .as_str()
        .ok_or(DeliveryError::TargetMismatch)?;
    let metadata: serde_json::Value = serde_json::from_str(include_str!("../../version.json"))
        .map_err(|_| DeliveryError::BuildMismatch)?;
    let build = metadata["build_seq"]
        .as_u64()
        .ok_or(DeliveryError::BuildMismatch)?;
    let identity = hbb_common::config::Config::file();
    if ![
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
        r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RustDesk\config\RustDesk.toml",
    ]
    .iter()
    .any(|p| identity.to_string_lossy().eq_ignore_ascii_case(p))
    {
        return Err(DeliveryError::IdentityMismatch);
    }
    let (_, config) = read_locked(&identity.with_file_name("RustDesk2.toml"), 65536, true)?;
    let code = current_product_code()?;
    let request = Request {
        manifest: &[],
        installation_id: installation,
        target_key: key,
        current_build: build,
        product_code: &code,
    };
    gates(agent, &bytes, &config, &request)?;
    let health = super::health_windows::probe_blocking(
        r"\\.\pipe\RustDesk\management-health-v2",
        Path::new(r"C:\Program Files\RustDesk\RustDesk.exe"),
        build,
        std::time::Duration::from_secs(3),
    )
    .map_err(|_| DeliveryError::BuildMismatch)?;
    if health.no_active_sessions != Some(true) {
        return Err(DeliveryError::LocallyBlocked);
    }
    drop(_installed_read);
    coordinate_approval(
        CurrentInstallation {
            installation_id: installation,
            target_key: key,
            current_build: build,
            product_code: &code,
        },
        fetch_fixed_manifest,
        deliver_installed_msi_approval,
    )
}

pub struct Request<'a> {
    pub manifest: &'a [u8],
    pub installation_id: &'a str,
    pub target_key: &'a str,
    pub current_build: u64,
    pub product_code: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryError {
    BuildMismatch,
    TargetMismatch,
    IdentityMismatch,
    SignatureInvalid,
    RegisteredSourceMismatch,
    LocallyBlocked,
    UntrustedPath,
    InvalidInput,
    ExistingApproval,
    Busy,
    PublishFailed,
}

pub fn deliver_installed_msi_approval(request: Request<'_>) -> Result<(), DeliveryError> {
    let identity = hbb_common::config::Config::file();
    if ![
        r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk.toml",
        r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RustDesk\config\RustDesk.toml",
    ]
    .iter()
    .any(|p| identity.to_string_lossy().eq_ignore_ascii_case(p))
    {
        return Err(DeliveryError::IdentityMismatch);
    }
    let agent = Path::new(AGENT);
    let config = identity.with_file_name("RustDesk2.toml");
    let _root_reader = installed_state_acl::hold_installed_root(agent)
        .map_err(|_| DeliveryError::UntrustedPath)?;
    deliver_bound(
        request,
        agent,
        &identity,
        &config,
        |code| msi_metadata::installed_source(code).map(|s| (s.package_name, s.source)),
        || {},
    )
}

fn read_locked(
    path: &Path,
    limit: usize,
    identity: bool,
) -> Result<(File, Vec<u8>), DeliveryError> {
    workspace::verify_restore_target(path, identity).map_err(|_| DeliveryError::UntrustedPath)?;
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|_| DeliveryError::UntrustedPath)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DeliveryError::InvalidInput)?;
    if bytes.len() > limit {
        return Err(DeliveryError::InvalidInput);
    }
    Ok((file, bytes))
}

fn hold_directory(path: &Path) -> Result<File, DeliveryError> {
    if path == Path::new(AGENT) {
        let _validated = installed_state_acl::hold_installed_root(path)
            .map_err(|_| DeliveryError::UntrustedPath)?;
    }
    workspace::verify(path).map_err(|_| DeliveryError::UntrustedPath)?;
    OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x02000000)
        .open(path)
        .map_err(|_| DeliveryError::UntrustedPath)
}

fn gates(
    agent: &Path,
    state: &[u8],
    config: &[u8],
    request: &Request<'_>,
) -> Result<(), DeliveryError> {
    for name in ["disable-agent", "disable-recovery", "maintenance.json"] {
        match std::fs::symlink_metadata(agent.join("state").join(name)) {
            Ok(_) => return Err(DeliveryError::LocallyBlocked),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(DeliveryError::LocallyBlocked),
        }
    }
    workspace::verify_restore_target(&agent.join("state/service-recovery-enabled"), false)
        .map_err(|_| DeliveryError::LocallyBlocked)?;
    let value: serde_json::Value =
        serde_json::from_slice(state).map_err(|_| DeliveryError::IdentityMismatch)?;
    if request.installation_id.len() != 32
        || !request
            .installation_id
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
        || value["agent_installation_id"] != request.installation_id
        || value["target_key"] != request.target_key
    {
        return Err(DeliveryError::IdentityMismatch);
    }
    if value["management_agent_enabled"] != true || value["disabled"] != false {
        return Err(DeliveryError::LocallyBlocked);
    }
    let config: hbb_common::toml::Value = hbb_common::toml::from_str(
        std::str::from_utf8(config).map_err(|_| DeliveryError::InvalidInput)?,
    )
    .map_err(|_| DeliveryError::InvalidInput)?;
    match config.get("options").and_then(|v| v.get("stop-service")) {
        None => {}
        Some(v) if matches!(v.as_str(), Some("" | "N")) => {}
        _ => return Err(DeliveryError::LocallyBlocked),
    }
    // Delivery does not interpret or mutate another installation transaction.
    for path in [
        agent.join("transactions"),
        agent.join("rescue/current"),
        agent.join("rescue/staging"),
        agent.join("rescue/first-install-staging"),
    ] {
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) if path == agent.join("transactions") => {
                let _held = hold_directory(&path)?;
                if std::fs::read_dir(&path)
                    .map_err(|_| DeliveryError::Busy)?
                    .next()
                    .is_some()
                {
                    return Err(DeliveryError::Busy);
                }
            }
            _ => return Err(DeliveryError::Busy),
        }
    }
    Ok(())
}

fn valid_identity(bytes: &[u8]) -> Result<(), DeliveryError> {
    #[derive(serde::Deserialize)]
    struct Identity {
        #[serde(default)]
        id: String,
        #[serde(default)]
        enc_id: String,
        key_pair: (Vec<u8>, Vec<u8>),
    }
    let value: Identity = hbb_common::toml::from_str(
        std::str::from_utf8(bytes).map_err(|_| DeliveryError::IdentityMismatch)?,
    )
    .map_err(|_| DeliveryError::IdentityMismatch)?;
    use hbb_common::sodiumoxide::crypto::sign;
    hbb_common::sodiumoxide::init().map_err(|_| DeliveryError::IdentityMismatch)?;
    let secret =
        sign::SecretKey::from_slice(&value.key_pair.0).ok_or(DeliveryError::IdentityMismatch)?;
    let public =
        sign::PublicKey::from_slice(&value.key_pair.1).ok_or(DeliveryError::IdentityMismatch)?;
    let proof = b"rustdesk-rescue-identity-v1";
    if (value.id.is_empty() && value.enc_id.is_empty())
        || !sign::verify_detached(&sign::sign_detached(proof, &secret), proof, &public)
    {
        return Err(DeliveryError::IdentityMismatch);
    }
    Ok(())
}

fn existing(
    path: &Path,
    request: &Request<'_>,
    manifest: &serde_json::Value,
) -> Result<bool, DeliveryError> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(DeliveryError::ExistingApproval),
        Ok(_) => {}
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Approval {
        schema: u32,
        product_code: String,
        manifest: serde_json::Value,
    }
    workspace::verify_file(path).map_err(|_| DeliveryError::UntrustedPath)?;
    let (_held, bytes) = read_locked(path, LIMIT, false)?;
    let old: Approval =
        serde_json::from_slice(&bytes).map_err(|_| DeliveryError::ExistingApproval)?;
    if old.schema != 1
        || !old.product_code.eq_ignore_ascii_case(request.product_code)
        || package_gate::package_reference(&old.manifest, request.target_key)
            .map_err(|_| DeliveryError::ExistingApproval)?
            != package_gate::package_reference(manifest, request.target_key)
                .map_err(|_| DeliveryError::InvalidInput)?
    {
        return Err(DeliveryError::ExistingApproval);
    }
    Ok(true)
}

fn deliver_bound(
    request: Request<'_>,
    agent: &Path,
    identity: &Path,
    config: &Path,
    registered: impl Fn(&str) -> Result<(String, PathBuf), String>,
    before_publish: impl FnOnce(),
) -> Result<(), DeliveryError> {
    if !matches!(
        request.target_key,
        "windows-x86_64-msi-sos" | "windows-x86_64-msi-standard"
    ) {
        return Err(DeliveryError::TargetMismatch);
    }
    if request.manifest.len() > LIMIT {
        return Err(DeliveryError::InvalidInput);
    }
    let _root_reader = installed_state_acl::hold_installed_root(agent)
        .map_err(|_| DeliveryError::UntrustedPath)?;
    let state_dir = agent.join("state");
    let _state_reader = installed_state_acl::verify_installed_state(&state_dir, config)
        .map_err(|_| DeliveryError::UntrustedPath)?;
    let _guard = install_guard::InstallGuard::try_acquire()
        .map_err(|_| DeliveryError::Busy)?
        .ok_or(DeliveryError::Busy)?;
    let (_state_lock, state) = read_locked(&agent.join("state/agent.json"), 65536, false)?;
    let (_identity_lock, identity_bytes) = read_locked(identity, 65536, true)?;
    let (_config_lock, config_bytes) = read_locked(config, 65536, true)?;
    gates(agent, &state, &config_bytes, &request)?;
    valid_identity(&identity_bytes)?;
    let manifest_value: serde_json::Value =
        serde_json::from_slice(request.manifest).map_err(|_| DeliveryError::InvalidInput)?;
    let manifest: hbb_common::UpdateManifest =
        serde_json::from_value(manifest_value.clone()).map_err(|_| DeliveryError::InvalidInput)?;
    let target = package_gate::verify(&manifest, request.target_key, 1)
        .map_err(|_| DeliveryError::SignatureInvalid)?;
    let (_, build, _) = base::update::update_target_release(&manifest, target)
        .map_err(|_| DeliveryError::BuildMismatch)?;
    if build == 0 || build != request.current_build {
        return Err(DeliveryError::BuildMismatch);
    }
    let name = package_gate::cache_name(&target.primary, "msi")
        .map_err(|_| DeliveryError::RegisteredSourceMismatch)?;
    let (registered_name, source) =
        registered(request.product_code).map_err(|_| DeliveryError::RegisteredSourceMismatch)?;
    if registered_name != name || source.file_name().and_then(|v| v.to_str()) != Some(name.as_str())
    {
        return Err(DeliveryError::RegisteredSourceMismatch);
    }
    let _package = package_gate::lock_verified_file(&source, target)
        .map_err(|_| DeliveryError::RegisteredSourceMismatch)?;
    let msi =
        msi_metadata::inspect(&source).map_err(|_| DeliveryError::RegisteredSourceMismatch)?;
    if msi.product_name != "RustDesk"
        || !msi.product_code.eq_ignore_ascii_case(request.product_code)
    {
        return Err(DeliveryError::RegisteredSourceMismatch);
    }
    let cache = agent.join("release-cache");
    if !cache
        .try_exists()
        .map_err(|_| DeliveryError::UntrustedPath)?
    {
        workspace::create(&cache).map_err(|_| DeliveryError::UntrustedPath)?;
    }
    let _cache = hold_directory(&cache)?;
    let destination = cache.join("first-install.json");
    before_publish();
    gates(agent, &state, &config_bytes, &request)?;
    let (final_name, final_source) =
        registered(request.product_code).map_err(|_| DeliveryError::RegisteredSourceMismatch)?;
    if final_name != registered_name || final_source != source {
        return Err(DeliveryError::RegisteredSourceMismatch);
    }
    if existing(&destination, &request, &manifest_value)? {
        return Ok(());
    }
    let bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"product_code":request.product_code,"manifest":manifest_value})).map_err(|_| DeliveryError::InvalidInput)?;
    if bytes.len() > LIMIT {
        return Err(DeliveryError::InvalidInput);
    }
    let temporary = cache.join(format!(
        "first-install-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&temporary)
            .map_err(|_| DeliveryError::PublishFailed)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| DeliveryError::PublishFailed)?;
        drop(file);
        workspace::verify_file(&temporary).map_err(|_| DeliveryError::UntrustedPath)?;
        gates(agent, &state, &config_bytes, &request)?;
        use std::os::windows::ffi::OsStrExt;
        let (final_name, final_source) = registered(request.product_code)
            .map_err(|_| DeliveryError::RegisteredSourceMismatch)?;
        if final_name != registered_name || final_source != source {
            return Err(DeliveryError::RegisteredSourceMismatch);
        }
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        #[link(name = "kernel32")]
        extern "system" {
            fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        }
        // WRITE_THROUGH without REPLACE_EXISTING: a competing approval always wins.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 8) } == 0 {
            return Err(DeliveryError::PublishFailed);
        }
        Ok(())
    })();
    if result.is_err() {
        match std::fs::remove_file(&temporary) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(DeliveryError::PublishFailed),
        }
    }
    result
}

#[cfg(test)]
pub struct TestLayout<'a> {
    pub agent: &'a std::path::Path,
    pub identity: std::path::PathBuf,
    pub config: std::path::PathBuf,
    pub registered_source: &'a std::path::Path,
}

#[cfg(test)]
pub fn deliver_at(request: Request<'_>, layout: TestLayout<'_>) -> Result<(), DeliveryError> {
    let root = layout.agent;
    installed_state_acl::with_root_fixture(root, || {
        let _root = installed_state_acl::hold_installed_root(root)
            .map_err(|_| DeliveryError::UntrustedPath)?;
        deliver_before_publish(request, layout, || {})
    })
}

#[cfg(test)]
pub fn deliver_registered_at(
    request: Request<'_>,
    layout: TestLayout<'_>,
) -> Result<(), DeliveryError> {
    deliver_bound(
        request,
        layout.agent,
        &layout.identity,
        &layout.config,
        |code| msi_metadata::installed_source(code).map(|s| (s.package_name, s.source)),
        || {},
    )
}

#[cfg(test)]
pub fn deliver_before_publish(
    request: Request<'_>,
    layout: TestLayout<'_>,
    hook: impl FnOnce(),
) -> Result<(), DeliveryError> {
    installed_state_acl::with_delivery_fixture(layout.agent, &layout.config, || {
        deliver_bound(
            request,
            layout.agent,
            &layout.identity,
            &layout.config,
            |_| {
                Ok((
                    layout
                        .registered_source
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    layout.registered_source.to_owned(),
                ))
            },
            hook,
        )
    })
}
