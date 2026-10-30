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
    /// The installation is not an MSI, so first-install approval does not apply.
    /// Returning an error here made every EXE install log a refusal every five
    /// minutes and left the repair worker observing without ever acting.
    NotApplicable,
}

/// First-install approval exists for MSI installs: it publishes the approved
/// package source a later MSI repair can fall back to. An EXE install has no MSI
/// product code to report, so the question does not arise.
fn approval_applies_to(target_key: &str) -> bool {
    matches!(
        target_key,
        "windows-x86_64-msi-standard" | "windows-x86_64-msi-sos"
    )
}

pub fn coordinate_approval(
    current: CurrentInstallation<'_>,
    fetch_fixed_source: impl FnOnce() -> Result<Vec<u8>, String>,
    deliver: impl FnOnce(Request<'_>) -> Result<(), DeliveryError>,
) -> Result<CoordinationOutcome, DeliveryError> {
    if !approval_applies_to(current.target_key) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredProduct {
    code: String,
    name: String,
    location: String,
}

/// Failed upgrades and reinstalls can leave several RustDesk MSI registrations behind.
/// Only those installed at the fixed directory can belong to the running client; every
/// other registration is ignored instead of failing the whole check. Which candidate
/// is the running build is settled by delivery, which accepts only the product whose
/// registered source is the current build's signed package.
fn select_install_candidates(
    products: &[RegisteredProduct],
    normal_install: &Path,
) -> Result<Vec<String>, DeliveryError> {
    let normal = normal_install.to_string_lossy();
    let normal = normal.trim_end_matches(['\\', '/']);
    let rustdesk: Vec<_> = products.iter().filter(|p| p.name == "RustDesk").collect();
    if rustdesk.is_empty() {
        return Err(DeliveryError::MsiProductAbsent);
    }
    let candidates: Vec<String> = rustdesk
        .into_iter()
        .filter(|p| {
            p.location
                .trim_end_matches(['\\', '/'])
                .eq_ignore_ascii_case(normal)
        })
        .map(|p| p.code.clone())
        .collect();
    if candidates.is_empty() {
        return Err(DeliveryError::MsiLocationMismatch);
    }
    Ok(candidates)
}

/// Tries each candidate until one is delivered. Only a registered-source mismatch is
/// specific to a candidate; anything else (source, signature, build, gates) would be
/// the same for the next one, so it ends the search.
fn coordinate_candidates(
    candidates: &[String],
    mut attempt: impl FnMut(&str) -> Result<CoordinationOutcome, DeliveryError>,
) -> Result<CoordinationOutcome, DeliveryError> {
    let mut last = Err(DeliveryError::MsiProductAbsent);
    for code in candidates {
        last = attempt(code);
        if last != Err(DeliveryError::RegisteredSourceMismatch) {
            return last;
        }
    }
    last
}

#[cfg(not(test))]
fn current_product_codes() -> Result<Vec<String>, DeliveryError> {
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
    let mut products = Vec::new();
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
            return select_install_candidates(
                &products,
                &super::system_dirs::get().normal_install(),
            );
        }
        // ERROR_BAD_CONFIGURATION: one damaged registration (often left by a failed
        // upgrade) must not hide the others.
        if status == 1610 {
            continue;
        }
        if status != 0 || context != 4 {
            return Err(DeliveryError::MsiEnumerationFailed);
        }
        let query = |property: &str| -> Option<String> {
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
                return None;
            }
            String::from_utf16(&buffer[..count as usize]).ok()
        };
        // A registration whose properties cannot be read is a leftover, not the running
        // client: an unreadable name skips it, an unreadable location keeps it out of
        // the candidates exactly like a registration outside the fixed directory.
        let (Some(name), Ok(code)) = (
            query("InstalledProductName"),
            String::from_utf16(&code[..38]),
        ) else {
            continue;
        };
        let location = query("InstallLocation").unwrap_or_default();
        products.push(RegisteredProduct {
            code,
            name,
            location,
        });
    }
    Err(DeliveryError::MsiEnumerationFailed)
}

/// An MSI major upgrade installs a new product code, so an approval kept from the
/// build it replaced can never match the registered source again (MSI answers 1605),
/// and because a present approval is never replaced, first install stayed refused.
/// Parsed as strictly as `existing()`, so a file it would call malformed is kept.
fn approval_superseded(bytes: &[u8], installed_code: &str) -> bool {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Approval {
        schema: u32,
        product_code: String,
        #[allow(dead_code)]
        manifest: serde_json::Value,
    }
    serde_json::from_slice::<Approval>(bytes)
        .is_ok_and(|old| old.schema == 1 && !old.product_code.eq_ignore_ascii_case(installed_code))
}

/// The build an approval was issued for, read from its manifest's entry for `key`.
/// `None` when the approval or that entry cannot be read; callers then do not guess.
fn approval_build(bytes: &[u8], key: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Approval {
        schema: u32,
        #[allow(dead_code)]
        product_code: String,
        manifest: serde_json::Value,
    }
    let approval = serde_json::from_slice::<Approval>(bytes)
        .ok()
        .filter(|approval| approval.schema == 1)?;
    let manifest: hbb_common::UpdateManifest = serde_json::from_value(approval.manifest).ok()?;
    let target = manifest.targets.get(key)?;
    base::update::update_target_release(&manifest, target)
        .ok()
        .map(|(_, build, _)| build)
}

/// An approval issued for another build is superseded: after an upgrade a failed MSI
/// transaction can leave the previous product registered at the fixed directory, so
/// its product code stays a candidate while the approval still names the old build.
/// Otherwise, with several candidates, an approval stays valid while it is bound to
/// any of them and is superseded only once its product is no longer a candidate.
fn approval_superseded_by(
    bytes: &[u8],
    candidates: &[String],
    running: Option<(&str, u64)>,
) -> bool {
    let older_build = running.is_some_and(|(key, build)| {
        approval_build(bytes, key).is_some_and(|approved| approved != build)
    });
    older_build
        || (!candidates.is_empty()
            && candidates
                .iter()
                .all(|code| approval_superseded(bytes, code)))
}

const DISCARD_GUARD_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
const DISCARD_GUARD_RETRY: std::time::Duration = std::time::Duration::from_millis(200);

fn locally_gated(agent: &Path) -> bool {
    ["disable-agent", "disable-recovery", "maintenance.json"]
        .iter()
        .any(|name| {
            !matches!(
                std::fs::symlink_metadata(agent.join("state").join(name)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound
            )
        })
}

fn discard_superseded_at<G>(
    agent: &Path,
    mut guard: impl FnMut() -> std::io::Result<Option<G>>,
    installed_codes: impl FnOnce() -> Result<Vec<String>, DeliveryError>,
    read: impl FnOnce(&Path) -> Result<Vec<u8>, DeliveryError>,
    running: Option<(&str, u64)>,
    wait: std::time::Duration,
    retry: std::time::Duration,
) -> Result<bool, DeliveryError> {
    let path = agent.join("release-cache/first-install.json");
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(DeliveryError::UntrustedPath),
        Ok(_) => {}
    }
    if locally_gated(agent) {
        return Ok(false);
    }
    // The product code is not guarded by the install lock (msiexec does not take it),
    // so it is read before waiting for the lock rather than while holding it.
    let codes = installed_codes()?;
    let deadline = std::time::Instant::now() + wait;
    let _guard = loop {
        match guard() {
            Ok(Some(held)) => break held,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(retry),
            _ => return Err(DeliveryError::Busy),
        }
    };
    if locally_gated(agent) || !approval_superseded_by(&read(&path)?, &codes, running) {
        return Ok(false);
    }
    std::fs::remove_file(&path).map_err(|_| DeliveryError::Busy)?;
    Ok(true)
}

#[cfg(not(test))]
pub fn discard_superseded_approval() -> Result<bool, DeliveryError> {
    let agent = super::system_dirs::get().agent();
    // Without a readable target or build the build check is skipped, not guessed.
    let key = read_locked(&agent.join("state/agent.json"), 65536, false)
        .ok()
        .and_then(|(_, bytes)| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|state| state["target_key"].as_str().map(str::to_owned));
    let build = serde_json::from_str::<serde_json::Value>(include_str!("../../version.json"))
        .ok()
        .and_then(|metadata| metadata["build_seq"].as_u64());
    let running = key.as_deref().zip(build);
    discard_superseded_at(
        &agent,
        install_guard::InstallGuard::try_acquire,
        current_product_codes,
        |path| {
            workspace::verify_file(path).map_err(|_| DeliveryError::UntrustedPath)?;
            let (_held, bytes) = read_locked(path, LIMIT, false)?;
            Ok(bytes)
        },
        running,
        DISCARD_GUARD_WAIT,
        DISCARD_GUARD_RETRY,
    )
}

#[cfg(not(test))]
pub fn coordinate_current_installation() -> Result<CoordinationOutcome, DeliveryError> {
    let agent = super::system_dirs::get().agent();
    let agent = agent.as_path();
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
    // Check the target before enumerating MSI products: `current_product_codes`
    // reports `RegisteredSourceMismatch` on any machine whose EXE install left an
    // old MSI registration behind, which is exactly the EXE case this guard covers.
    if !approval_applies_to(key) {
        return Ok(CoordinationOutcome::NotApplicable);
    }
    let metadata: serde_json::Value = serde_json::from_str(include_str!("../../version.json"))
        .map_err(|_| DeliveryError::BuildMismatch)?;
    let build = metadata["build_seq"]
        .as_u64()
        .ok_or(DeliveryError::BuildMismatch)?;
    let identity = hbb_common::config::Config::file();
    if ![
        super::system_dirs::get()
            .local_service_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
        super::system_dirs::get()
            .system_profile_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
    ]
    .iter()
    .any(|p| {
        identity
            .to_string_lossy()
            .eq_ignore_ascii_case(&p.to_string_lossy())
    }) {
        return Err(DeliveryError::IdentityMismatch);
    }
    let (_, config) = read_locked(&identity.with_file_name("RustDesk2.toml"), 65536, true)?;
    let codes = current_product_codes()?;
    let request = Request {
        manifest: &[],
        installation_id: installation,
        target_key: key,
        current_build: build,
        product_code: &codes[0],
    };
    gates(agent, &bytes, &config, &request)?;
    let health = super::health_windows::probe_blocking(
        r"\\.\pipe\RustDesk\management-health-v2",
        &super::system_dirs::get()
            .normal_install()
            .join("RustDesk.exe"),
        build,
        std::time::Duration::from_secs(3),
    )
    .map_err(|_| DeliveryError::BuildMismatch)?;
    if health.no_active_sessions != Some(true) {
        return Err(DeliveryError::SessionsActive);
    }
    drop(_installed_read);
    // The manifest does not depend on the candidate, so it is fetched once.
    let manifest = fetch_fixed_manifest();
    coordinate_candidates(&codes, |code| {
        coordinate_approval(
            CurrentInstallation {
                installation_id: installation,
                target_key: key,
                current_build: build,
                product_code: code,
            },
            || manifest.clone(),
            deliver_installed_msi_approval,
        )
    })
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
    /// No RustDesk MSI product is registered on this machine.
    MsiProductAbsent,
    /// RustDesk MSI products are registered, but none at the fixed install directory.
    MsiLocationMismatch,
    /// Windows Installer could not enumerate the registered products.
    MsiEnumerationFailed,
    /// A remote session is active or the client could not confirm there is none;
    /// approval waits rather than reinstalling under a connected user.
    SessionsActive,
    LocallyBlocked,
    UntrustedPath,
    InvalidInput,
    ExistingApproval,
    Busy,
    PublishFailed,
}

impl DeliveryError {
    /// Stable snake_case code reported in `first-install-status` and status `rescue`.
    pub fn code(self) -> &'static str {
        match self {
            Self::BuildMismatch => "build_mismatch",
            Self::TargetMismatch => "target_mismatch",
            Self::IdentityMismatch => "identity_mismatch",
            Self::SignatureInvalid => "signature_invalid",
            Self::RegisteredSourceMismatch => "registered_source_mismatch",
            Self::MsiProductAbsent => "msi_product_absent",
            Self::MsiLocationMismatch => "msi_location_mismatch",
            Self::MsiEnumerationFailed => "msi_enumeration_failed",
            Self::LocallyBlocked => "locally_blocked",
            Self::SessionsActive => "sessions_active",
            Self::UntrustedPath => "untrusted_path",
            Self::InvalidInput => "invalid_input",
            Self::ExistingApproval => "existing_approval",
            Self::Busy => "busy",
            Self::PublishFailed => "publish_failed",
        }
    }
}

impl CoordinationOutcome {
    /// A still-waiting outcome, as a report code; delivered or not applicable is `None`.
    pub fn pending_code(self) -> Option<&'static str> {
        match self {
            Self::Delivered | Self::NotApplicable => None,
            Self::WaitingSource => Some("waiting_source"),
            Self::WaitingSignature => Some("waiting_signature"),
            Self::WaitingExactBuild => Some("waiting_exact_build"),
        }
    }
}

pub fn deliver_installed_msi_approval(request: Request<'_>) -> Result<(), DeliveryError> {
    let identity = hbb_common::config::Config::file();
    if ![
        super::system_dirs::get()
            .local_service_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
        super::system_dirs::get()
            .system_profile_roaming()
            .join(r"RustDesk\config\RustDesk.toml"),
    ]
    .iter()
    .any(|p| {
        identity
            .to_string_lossy()
            .eq_ignore_ascii_case(&p.to_string_lossy())
    }) {
        return Err(DeliveryError::IdentityMismatch);
    }
    let agent = super::system_dirs::get().agent();
    let agent = agent.as_path();
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
    if path == super::system_dirs::get().agent() {
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
    // Delivery does not interpret or mutate another installation transaction. An
    // existing rescue/current is allowed: after a client updater upgrade it is rotated
    // by first install, which needs an approval for the new build.
    for path in [
        agent.join("transactions"),
        agent.join("rescue/staging"),
        agent.join("rescue/first-install-staging"),
    ] {
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) if path == agent.join("transactions") => {
                let _held = hold_directory(&path)?;
                if !transactions_finished(&path)? {
                    return Err(DeliveryError::Busy);
                }
            }
            _ => return Err(DeliveryError::Busy),
        }
    }
    Ok(())
}

/// Finished agent transactions stay on disk; only an unfinished transaction or an
/// unpublished rescue rotation means another installation is in progress. Matches the
/// first-install worker's own transaction gate.
fn transaction_finished(transaction: &[u8], rotation: Option<&[u8]>) -> bool {
    let phase = |bytes: &[u8]| {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|value| value["phase"].as_str().map(str::to_owned))
    };
    matches!(
        phase(transaction).as_deref(),
        Some("committed" | "rolled_back")
    ) && rotation.map_or(true, |bytes| phase(bytes).as_deref() == Some("published"))
}

fn transactions_finished(directory: &Path) -> Result<bool, DeliveryError> {
    let read = |path: &Path, limit: u64| -> Result<Vec<u8>, DeliveryError> {
        workspace::verify_file(path).map_err(|_| DeliveryError::Busy)?;
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| DeliveryError::Busy)?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| DeliveryError::Busy)?;
        if bytes.len() as u64 > limit {
            return Err(DeliveryError::Busy);
        }
        Ok(bytes)
    };
    for (count, entry) in std::fs::read_dir(directory)
        .map_err(|_| DeliveryError::Busy)?
        .enumerate()
    {
        if count >= 64 {
            return Ok(false);
        }
        let path = entry.map_err(|_| DeliveryError::Busy)?.path();
        workspace::verify(&path).map_err(|_| DeliveryError::Busy)?;
        let transaction = read(&path.join("transaction.json"), 65536)?;
        let rotation = match std::fs::symlink_metadata(path.join("rotation.json")) {
            Ok(_) => Some(read(&path.join("rotation.json"), 4096)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(DeliveryError::Busy),
        };
        if !transaction_finished(&transaction, rotation.as_deref()) {
            return Ok(false);
        }
    }
    Ok(true)
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

#[cfg(test)]
mod approval_scope_tests {
    use super::*;

    // `coordinate_current_installation` enumerated MSI products before checking
    // whether the install was an MSI at all. On an EXE install that has an old MSI
    // registration left behind, that scan fails and the refusal is logged every
    // five minutes for a check which never applied in the first place.
    #[test]
    fn approval_applies_only_to_msi_targets() {
        assert!(approval_applies_to("windows-x86_64-msi-standard"));
        assert!(approval_applies_to("windows-x86_64-msi-sos"));
        assert!(!approval_applies_to("windows-x86_64-exe-standard"));
        assert!(!approval_applies_to("windows-x86_64-exe-sos"));
    }

    // `coordinate_approval` must keep rejecting a non-MSI target rather than
    // silently pretending approval succeeded.
    #[test]
    fn a_non_msi_target_is_rejected_before_any_source_is_fetched() {
        let outcome = coordinate_approval(
            CurrentInstallation {
                installation_id: "a",
                target_key: "windows-x86_64-exe-standard",
                current_build: 2026102401,
                product_code: "code",
            },
            || panic!("the source must not be fetched for a non-MSI target"),
            |_| panic!("nothing is delivered for a non-MSI target"),
        );
        assert_eq!(outcome, Err(DeliveryError::TargetMismatch));
    }

    // MSI 大版本升级会换新的 ProductCode。升级前留下的审批绑定旧码，
    // 查询注册源只会得到 1605，又因已有审批从不替换，首装救援就永远停在这里。
    #[test]
    fn an_approval_for_a_replaced_product_code_is_superseded() {
        let old = br#"{"schema":1,"product_code":"{11111111-1111-1111-1111-111111111111}","manifest":{}}"#;
        assert!(approval_superseded(
            old,
            "{22222222-2222-2222-2222-222222222222}"
        ));
    }

    #[test]
    fn an_approval_for_the_installed_product_code_is_kept() {
        let current = br#"{"schema":1,"product_code":"{ABCDEFAB-1111-1111-1111-111111111111}","manifest":{}}"#;
        assert!(!approval_superseded(
            current,
            "{abcdefab-1111-1111-1111-111111111111}"
        ));
    }

    const OLD_CODE: &str = "{11111111-1111-1111-1111-111111111111}";
    const NEW_CODE: &str = "{22222222-2222-2222-2222-222222222222}";

    fn discard_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rd-discard-approval-{name}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(root.join("release-cache")).unwrap();
        std::fs::create_dir_all(root.join("state")).unwrap();
        std::fs::write(
            root.join("release-cache/first-install.json"),
            format!(r#"{{"schema":1,"product_code":"{OLD_CODE}","manifest":{{}}}}"#),
        )
        .unwrap();
        root
    }

    fn discard(
        root: &std::path::Path,
        guard: impl FnMut() -> std::io::Result<Option<u8>>,
        code: impl FnOnce() -> Result<String, DeliveryError>,
    ) -> Result<bool, DeliveryError> {
        discard_superseded_at(
            root,
            guard,
            || code().map(|code| vec![code]),
            |path| std::fs::read(path).map_err(|_| DeliveryError::UntrustedPath),
            None,
            std::time::Duration::from_millis(30),
            std::time::Duration::from_millis(5),
        )
    }

    fn approval_kept(root: &std::path::Path) -> bool {
        root.join("release-cache/first-install.json").exists()
    }

    #[test]
    fn a_superseded_approval_is_removed() {
        let root = discard_root("removed");
        assert_eq!(
            discard(&root, || Ok(Some(1)), || Ok(NEW_CODE.into())),
            Ok(true)
        );
        assert!(!approval_kept(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_approval_for_the_installed_product_is_not_removed() {
        let root = discard_root("current");
        assert_eq!(
            discard(&root, || Ok(Some(1)), || Ok(OLD_CODE.into())),
            Ok(false)
        );
        assert!(approval_kept(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn nothing_is_done_without_an_approval() {
        let root = discard_root("absent");
        std::fs::remove_file(root.join("release-cache/first-install.json")).unwrap();
        assert_eq!(
            discard(
                &root,
                || panic!("no lock is needed"),
                || panic!("no query is needed")
            ),
            Ok(false)
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    // 本地禁用、停用恢复和维护状态优先于任何清理。
    #[test]
    fn a_local_gate_keeps_the_approval() {
        for gate in ["disable-agent", "disable-recovery", "maintenance.json"] {
            let root = discard_root("gate");
            std::fs::write(root.join("state").join(gate), b"").unwrap();
            assert_eq!(
                discard(&root, || Ok(Some(1)), || Ok(NEW_CODE.into())),
                Ok(false)
            );
            assert!(approval_kept(&root), "{gate}");
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    // 和首装准备一样会撞上同一轮周期任务里其他线程的毫秒级持锁，必须等一会儿再判忙。
    #[test]
    fn a_briefly_held_guard_is_waited_for() {
        let root = discard_root("brief");
        let calls = std::cell::Cell::new(0);
        let result = discard(
            &root,
            || {
                calls.set(calls.get() + 1);
                Ok(if calls.get() < 3 { None } else { Some(1) })
            },
            || Ok(NEW_CODE.into()),
        );
        assert_eq!(result, Ok(true));
        assert_eq!(calls.get(), 3);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_guard_held_past_the_window_keeps_the_approval() {
        let root = discard_root("busy");
        assert_eq!(
            discard(&root, || Ok(None), || Ok(NEW_CODE.into())),
            Err(DeliveryError::Busy)
        );
        assert!(approval_kept(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_failing_guard_keeps_the_approval() {
        let root = discard_root("guard-error");
        let calls = std::cell::Cell::new(0);
        let result = discard(
            &root,
            || {
                calls.set(calls.get() + 1);
                Err(std::io::Error::other("denied"))
            },
            || Ok(NEW_CODE.into()),
        );
        assert_eq!(result, Err(DeliveryError::Busy));
        assert_eq!(calls.get(), 1);
        assert!(approval_kept(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    // MSI 事务进行中查不到唯一的已安装产品时，不能据此删审批。
    #[test]
    fn an_unknown_installed_product_keeps_the_approval() {
        let root = discard_root("no-code");
        assert_eq!(
            discard(
                &root,
                || Ok(Some(1)),
                || Err(DeliveryError::RegisteredSourceMismatch)
            ),
            Err(DeliveryError::RegisteredSourceMismatch)
        );
        assert!(approval_kept(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_unreadable_approval_is_kept() {
        let root = discard_root("unreadable");
        let result = discard_superseded_at(
            &root,
            || Ok(Some(1u8)),
            || Ok(vec![NEW_CODE.to_owned()]),
            |_| Err(DeliveryError::UntrustedPath),
            None,
            std::time::Duration::from_millis(30),
            std::time::Duration::from_millis(5),
        );
        assert_eq!(result, Err(DeliveryError::UntrustedPath));
        assert!(approval_kept(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    // 跑过 Agent 事务或 R4 修复的设备，transactions 下会留着已结束的事务目录。
    // 「非空即忙」让这些设备永远拿不到新审批；只有未结束的事务或未发布的轮换才算忙。
    #[test]
    fn only_an_unfinished_transaction_blocks_delivery() {
        assert!(transaction_finished(br#"{"phase":"committed"}"#, None));
        assert!(transaction_finished(br#"{"phase":"rolled_back"}"#, None));
        assert!(transaction_finished(
            br#"{"phase":"committed"}"#,
            Some(br#"{"phase":"published"}"#)
        ));
        assert!(!transaction_finished(
            br#"{"phase":"committed"}"#,
            Some(br#"{"phase":"old_moved"}"#)
        ));
        for phase in ["planned", "applying", "rolling_back", "rollback_failed"] {
            let tx = format!(r#"{{"phase":"{phase}"}}"#);
            assert!(!transaction_finished(tx.as_bytes(), None), "{phase}");
        }
        assert!(!transaction_finished(b"not json", None));
        assert!(!transaction_finished(
            br#"{"phase":"committed"}"#,
            Some(b"bad")
        ));
    }

    // 读不懂的审批不能当作过期删掉，交给原有流程照旧报 malformed。
    #[test]
    fn a_malformed_approval_is_never_treated_as_superseded() {
        assert!(!approval_superseded(
            b"not json",
            "{22222222-2222-2222-2222-222222222222}"
        ));
        assert!(!approval_superseded(
            br#"{"schema":1}"#,
            "{22222222-2222-2222-2222-222222222222}"
        ));
        assert!(!approval_superseded(
            br#"{"schema":2,"product_code":"{11111111-1111-1111-1111-111111111111}","manifest":{}}"#,
            "{22222222-2222-2222-2222-222222222222}"
        ));
        assert!(!approval_superseded(
            br#"{"schema":1,"product_code":"{11111111-1111-1111-1111-111111111111}","manifest":{},"extra":1}"#,
            "{22222222-2222-2222-2222-222222222222}"
        ));
    }

    fn product(code: &str, name: &str, location: &str) -> RegisteredProduct {
        RegisteredProduct {
            code: code.into(),
            name: name.into(),
            location: location.into(),
        }
    }

    const NORMAL: &str = r"C:\Program Files\RustDesk";

    // 多次失败升级、重装后，本机可能登记了多个 RustDesk MSI。只认装在固定安装目录的那些，
    // 其他登记（装在别处、非 RustDesk）一律忽略，不再整体判为登记源不符。
    #[test]
    fn candidates_keep_every_rustdesk_product_at_the_normal_install() {
        let products = [
            product(OLD_CODE, "RustDesk", r"C:\Program Files\RustDesk\"),
            product("{33333333-3333-3333-3333-333333333333}", "Other", NORMAL),
            product(
                "{44444444-4444-4444-4444-444444444444}",
                "RustDesk",
                r"D:\Old\RustDesk",
            ),
            product(NEW_CODE, "RustDesk", r"c:\program files\rustdesk"),
        ];
        assert_eq!(
            select_install_candidates(&products, std::path::Path::new(NORMAL)),
            Ok(vec![OLD_CODE.to_owned(), NEW_CODE.to_owned()])
        );
    }

    #[test]
    fn a_single_registration_is_the_only_candidate() {
        let products = [product(NEW_CODE, "RustDesk", NORMAL)];
        assert_eq!(
            select_install_candidates(&products, std::path::Path::new(NORMAL)),
            Ok(vec![NEW_CODE.to_owned()])
        );
    }

    #[test]
    fn no_rustdesk_registration_is_reported_as_absent() {
        let products = [product(OLD_CODE, "Other", NORMAL)];
        assert_eq!(
            select_install_candidates(&products, std::path::Path::new(NORMAL)),
            Err(DeliveryError::MsiProductAbsent)
        );
        assert_eq!(
            select_install_candidates(&[], std::path::Path::new(NORMAL)),
            Err(DeliveryError::MsiProductAbsent)
        );
    }

    #[test]
    fn rustdesk_registered_only_elsewhere_is_a_location_mismatch() {
        let products = [product(OLD_CODE, "RustDesk", r"D:\RustDesk")];
        assert_eq!(
            select_install_candidates(&products, std::path::Path::new(NORMAL)),
            Err(DeliveryError::MsiLocationMismatch)
        );
    }

    // 多个候选时逐个尝试：只有登记源是当前构建签名包的那一个能通过交付校验，
    // 它就是当前运行主程序对应的 MSI。前面的候选不符不能挡住后面的。
    #[test]
    fn coordination_tries_each_candidate_until_one_is_delivered() {
        let tried = std::cell::RefCell::new(Vec::new());
        let outcome = coordinate_candidates(&[OLD_CODE.to_owned(), NEW_CODE.to_owned()], |code| {
            tried.borrow_mut().push(code.to_owned());
            if code == NEW_CODE {
                Ok(CoordinationOutcome::Delivered)
            } else {
                Err(DeliveryError::RegisteredSourceMismatch)
            }
        });
        assert_eq!(outcome, Ok(CoordinationOutcome::Delivered));
        assert_eq!(
            *tried.borrow(),
            vec![OLD_CODE.to_owned(), NEW_CODE.to_owned()]
        );
    }

    #[test]
    fn coordination_reports_mismatch_when_no_candidate_matches() {
        let outcome = coordinate_candidates(&[OLD_CODE.to_owned(), NEW_CODE.to_owned()], |_| {
            Err(DeliveryError::RegisteredSourceMismatch)
        });
        assert_eq!(outcome, Err(DeliveryError::RegisteredSourceMismatch));
    }

    // 拉清单、签名、构建号和本地门禁与选哪个产品无关，第一次就该停下，不再拿下一个候选重试。
    #[test]
    fn a_candidate_independent_result_stops_the_search() {
        for result in [
            Ok(CoordinationOutcome::WaitingSource),
            Ok(CoordinationOutcome::WaitingExactBuild),
            Err(DeliveryError::LocallyBlocked),
            Err(DeliveryError::SessionsActive),
            Err(DeliveryError::Busy),
        ] {
            let calls = std::cell::Cell::new(0);
            let outcome =
                coordinate_candidates(&[OLD_CODE.to_owned(), NEW_CODE.to_owned()], |_| {
                    calls.set(calls.get() + 1);
                    result
                });
            assert_eq!(outcome, result);
            assert_eq!(calls.get(), 1, "{result:?}");
        }
    }

    // 已有审批只要绑定任一候选就保留；绑定的产品已不在候选里才算作废。
    #[test]
    fn an_approval_is_superseded_only_when_no_candidate_matches() {
        let old = format!(r#"{{"schema":1,"product_code":"{OLD_CODE}","manifest":{{}}}}"#);
        assert!(!approval_superseded_by(
            old.as_bytes(),
            &[NEW_CODE.to_owned(), OLD_CODE.to_ascii_lowercase()],
            None
        ));
        assert!(approval_superseded_by(
            old.as_bytes(),
            &[NEW_CODE.to_owned()],
            None
        ));
        assert!(!approval_superseded_by(
            b"not json",
            &[NEW_CODE.to_owned()],
            None
        ));
        assert!(!approval_superseded_by(old.as_bytes(), &[], None));
    }

    #[test]
    fn a_superseded_approval_with_several_registrations_is_removed() {
        let root = discard_root("several");
        let result = discard_superseded_at(
            &root,
            || Ok(Some(1u8)),
            || {
                Ok(vec![
                    NEW_CODE.to_owned(),
                    "{55555555-5555-5555-5555-555555555555}".to_owned(),
                ])
            },
            |path| std::fs::read(path).map_err(|_| DeliveryError::UntrustedPath),
            None,
            std::time::Duration::from_millis(30),
            std::time::Duration::from_millis(5),
        );
        assert_eq!(result, Ok(true));
        assert!(!approval_kept(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }

    // 上报给后台的原因码固定为 snake_case，后台按它给出处理建议。
    #[test]
    fn delivery_errors_have_stable_report_codes() {
        let cases = [
            (DeliveryError::BuildMismatch, "build_mismatch"),
            (DeliveryError::TargetMismatch, "target_mismatch"),
            (DeliveryError::IdentityMismatch, "identity_mismatch"),
            (DeliveryError::SignatureInvalid, "signature_invalid"),
            (
                DeliveryError::RegisteredSourceMismatch,
                "registered_source_mismatch",
            ),
            (DeliveryError::MsiProductAbsent, "msi_product_absent"),
            (DeliveryError::MsiLocationMismatch, "msi_location_mismatch"),
            (
                DeliveryError::MsiEnumerationFailed,
                "msi_enumeration_failed",
            ),
            (DeliveryError::LocallyBlocked, "locally_blocked"),
            (DeliveryError::SessionsActive, "sessions_active"),
            (DeliveryError::UntrustedPath, "untrusted_path"),
            (DeliveryError::InvalidInput, "invalid_input"),
            (DeliveryError::ExistingApproval, "existing_approval"),
            (DeliveryError::Busy, "busy"),
            (DeliveryError::PublishFailed, "publish_failed"),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code);
        }
        for (outcome, code) in [
            (CoordinationOutcome::Delivered, None),
            (CoordinationOutcome::NotApplicable, None),
            (CoordinationOutcome::WaitingSource, Some("waiting_source")),
            (
                CoordinationOutcome::WaitingSignature,
                Some("waiting_signature"),
            ),
            (
                CoordinationOutcome::WaitingExactBuild,
                Some("waiting_exact_build"),
            ),
        ] {
            assert_eq!(outcome.pending_code(), code);
        }
    }

    const KEY: &str = "windows-x86_64-msi-standard";

    fn approval_for_build(code: &str, build: u64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "schema": 1,
            "product_code": code,
            "manifest": {"schema": 1, "build_seq": build, "targets": {KEY: {}}}
        }))
        .unwrap()
    }

    // 升级失败残留了旧构建的登记：批准绑定的旧产品仍在候选里，但批准的是上一构建。
    // 只看产品码会永远保留它，救援包就永远换不成新构建；批准构建不是当前构建就作废。
    #[test]
    fn an_approval_for_an_older_build_is_superseded_even_if_its_product_remains() {
        let approval = approval_for_build(OLD_CODE, 2026102901);
        assert!(approval_superseded_by(
            &approval,
            &[OLD_CODE.to_owned(), NEW_CODE.to_owned()],
            Some((KEY, 2026103001))
        ));
    }

    #[test]
    fn an_approval_for_the_running_build_is_kept() {
        let approval = approval_for_build(NEW_CODE, 2026103001);
        assert!(!approval_superseded_by(
            &approval,
            &[OLD_CODE.to_owned(), NEW_CODE.to_owned()],
            Some((KEY, 2026103001))
        ));
    }

    // 读不出本目标构建号时不臆断，退回只按产品码判断。
    #[test]
    fn an_approval_without_a_readable_build_falls_back_to_product_codes() {
        let other_target = serde_json::to_vec(&serde_json::json!({
            "schema": 1,
            "product_code": OLD_CODE,
            "manifest": {"schema": 1, "build_seq": 1, "targets": {"windows-x86_64-msi-sos": {}}}
        }))
        .unwrap();
        assert!(!approval_superseded_by(
            &other_target,
            &[OLD_CODE.to_owned()],
            Some((KEY, 2026103001))
        ));
        assert_eq!(approval_build(&other_target, KEY), None);
        assert_eq!(
            approval_build(&approval_for_build(OLD_CODE, 7), KEY),
            Some(7)
        );
    }

    #[test]
    fn an_older_approval_is_removed_through_the_discard_path() {
        let root = discard_root("older-build");
        std::fs::write(
            root.join("release-cache/first-install.json"),
            approval_for_build(OLD_CODE, 2026102901),
        )
        .unwrap();
        let result = discard_superseded_at(
            &root,
            || Ok(Some(1u8)),
            || Ok(vec![OLD_CODE.to_owned(), NEW_CODE.to_owned()]),
            |path| std::fs::read(path).map_err(|_| DeliveryError::UntrustedPath),
            Some((KEY, 2026103001)),
            std::time::Duration::from_millis(30),
            std::time::Duration::from_millis(5),
        );
        assert_eq!(result, Ok(true));
        assert!(!approval_kept(&root));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
