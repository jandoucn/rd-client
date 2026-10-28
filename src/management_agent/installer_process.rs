use super::install_execution::InstallOutcome;
#[path = "msi_metadata.rs"]
mod admission_metadata;
use std::{
    ffi::OsString,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

pub fn msi_arguments(package: &Path, log: &Path) -> Result<Vec<OsString>, String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    if !package.is_absolute()
        || !log.is_absolute()
        || package.extension().and_then(|v| v.to_str()) != Some("msi")
        || package.to_string_lossy().contains('"')
        || log.to_string_lossy().contains('"')
    {
        return Err("installer package/log path invalid".into());
    }
    let native = |path: &Path| OsString::from_wide(&path.as_os_str().encode_wide()
        .map(|unit| if unit == u16::from(b'/') { u16::from(b'\\') } else { unit }).collect::<Vec<_>>());
    Ok(vec![
        "/i".into(),
        native(package),
        "/qn".into(),
        "/norestart".into(),
        "REBOOT=ReallySuppress".into(),
        "LAUNCH_TRAY_APP=N".into(),
        "RUSTDESK_AGENT_RECOVERY=1".into(),
        "/L*v".into(),
        native(log),
    ])
}

pub fn run(command: &mut Command, timeout: Duration) -> Result<InstallOutcome, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("installer launch failed: {e}"))?;
    let process = match super::handoff::ObservedProcess::open(child.id()) {
        Ok(process) => Some(process),
        Err(error) => match child.try_wait() {
            Ok(Some(status)) => return Ok(classify(status.code().unwrap_or(-1))),
            _ => {
                eprintln!("installer process identity unavailable; reconciliation required; pid={}: {error}",child.id());
                return Ok(InstallOutcome::Pending {
                    pid: child.id(),
                    created_at: 0,
                });
            }
        },
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Err(error) => {
                eprintln!("installer result wait failed; reconciliation required: {error}");
                return Ok(InstallOutcome::Pending {
                    pid: child.id(),
                    created_at: process.as_ref().map(|p| p.owner().created_at).unwrap_or(0),
                });
            }
            Ok(Some(status)) => return Ok(classify(status.code().unwrap_or(-1))),
            Ok(None) if Instant::now() >= deadline => {
                return Ok(InstallOutcome::Pending {
                    pid: child.id(),
                    created_at: process.as_ref().map(|p| p.owner().created_at).unwrap_or(0),
                })
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn classify(code: i32) -> InstallOutcome {
    match code {
        0 => InstallOutcome::Installed,
        3010 => InstallOutcome::RebootRequired,
        1618 => InstallOutcome::Busy,
        code => InstallOutcome::Failed(code),
    }
}

pub fn read_recorded(root: &Path, step: &str, transaction_id: &str, package_sha256: &str) -> Result<Option<InstallOutcome>, String> {
    use std::{io::Read, os::windows::fs::OpenOptionsExt};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        schema: u32,
        transaction_id: String,
        package_sha256: String,
        step: String,
        outcome: Option<InstallOutcome>,
    }
    let token = |value: &str, length: usize| value.len() == length
        && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !matches!(step, "install" | "rollback") || !token(transaction_id, 32) || !token(package_sha256, 64) {
        return Err("installer receipt binding invalid".into());
    }
    super::workspace::verify(root)?;
    let path = root.join(format!("{step}-result.json"));
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("installer receipt unavailable: {error}")),
        Ok(_) => {}
    }
    let mut outcome = None;
    for (path, is_result) in [(root.join(format!("{step}-started.json")), false), (path, true)] {
        super::workspace::verify_file(&path)?;
        let mut bytes = Vec::new();
        std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
            .open(path).map_err(|e| e.to_string())?.take(4097)
            .read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 4096 { return Err("installer receipt too large".into()); }
        let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| "installer receipt malformed")?;
        if receipt.schema != 1 || receipt.transaction_id != transaction_id
            || receipt.package_sha256 != package_sha256 || receipt.step != step {
            return Err("installer receipt binding mismatch".into());
        }
        if !is_result && receipt.outcome.is_some() { return Err("installer intent malformed".into()); }
        if is_result {
            outcome = Some(receipt.outcome.ok_or("installer result absent; reconciliation required")?);
        }
    }
    Ok(outcome)
}

pub fn run_recorded(
    command: &mut Command,
    timeout: Duration,
    root: &Path,
    step: &str,
    transaction_id: &str,
    package_sha256: &str,
) -> Result<InstallOutcome, String> {
    use std::{io::{Read, Write}, os::windows::fs::OpenOptionsExt};
    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Receipt {
        schema: u32,
        transaction_id: String,
        package_sha256: String,
        step: String,
        outcome: Option<InstallOutcome>,
    }
    let token = |value: &str, length: usize| value.len() == length
        && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !matches!(step, "install" | "rollback")
        || !token(transaction_id, 32) || !token(package_sha256, 64) {
        return Err("installer receipt binding invalid".into());
    }
    super::workspace::verify(root)?;
    let intent_path = root.join(format!("{step}-started.json"));
    let result_path = root.join(format!("{step}-result.json"));
    let load = |path: &Path| -> Result<Receipt, String> {
        super::workspace::verify_file(path)?;
        let mut bytes = Vec::new();
        std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x00200000)
            .open(path).map_err(|e| e.to_string())?.take(4097)
            .read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 4096 { return Err("installer receipt too large".into()); }
        let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| "installer receipt malformed")?;
        if receipt.schema != 1 || receipt.transaction_id != transaction_id
            || receipt.package_sha256 != package_sha256 || receipt.step != step {
            return Err("installer receipt binding mismatch".into());
        }
        Ok(receipt)
    };
    match std::fs::symlink_metadata(&result_path) {
        Ok(_) => {
            let intent = load(&intent_path)?;
            if intent.outcome.is_some() { return Err("installer intent malformed".into()); }
            return load(&result_path)?.outcome.ok_or_else(|| "installer result absent; reconciliation required".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("installer receipt unavailable: {error}")),
    }
    let _admission = if option_env!("RUSTDESK_MSI_ADMISSION") == Some("1") {
        prepare_msi_admission(command, root, step, transaction_id, package_sha256)?
    } else { None };
    let mut intent = match std::fs::OpenOptions::new().write(true).create_new(true).share_mode(0)
        .open(&intent_path) {
        Ok(intent) => intent,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err("installer launch already recorded; reconciliation required".into());
        }
        Err(error) => return Err(format!("installer launch record unavailable: {error}")),
    };
    let mut receipt = Receipt { schema: 1, transaction_id: transaction_id.into(), package_sha256: package_sha256.into(), step: step.into(), outcome: None };
    intent.write_all(&serde_json::to_vec(&receipt).map_err(|e| e.to_string())?)
        .and_then(|_| intent.sync_all()).map_err(|e| format!("installer intent persistence: {e}"))?;
    drop(intent);
    super::workspace::verify_file(&intent_path)?;
    // Once launch is possible, any unknown outcome must preserve the intent.
    // Never infer that a missing receipt means the package has not run.
    let outcome = if _admission.is_some() {
        run_admitted_msi(command, root, step)?
    } else {
        run(command, timeout).map_err(|e| format!("{e}; reconciliation required"))?
    };
    receipt.outcome = Some(outcome.clone());
    super::state_store::publish(&result_path, &serde_json::to_vec(&receipt).map_err(|e| e.to_string())?)
        .map_err(|e| format!("installer result persistence failed; reconciliation required: {e}"))?;
    super::workspace::verify_file(&result_path)?;
    Ok(outcome)
}

fn admission_callback_binding(
    permit: &serde_json::Value,
    top_used: bool,
    removal_used: bool,
    removal: bool,
    values: &[String],
) -> Result<(), String> {
    if values.len() != 8 {
        return Err("admission callback field count".into());
    }
    if removal {
        if !top_used || removal_used {
            return Err("removal callback out of order or repeated".into());
        }
    } else if top_used {
        return Err("admission callback repeated".into());
    }
    for (index, key) in [(0, "transaction_id"), (1, "step"), (2, "nonce")] {
        if removal {
            if !values[index].is_empty() {
                return Err("removal callback must discover parent from session".into());
            }
        } else if permit[key].as_str() != Some(values[index].as_str()) {
            return Err("admission callback wrong installation".into());
        }
    }
    let identity = if removal { &permit["remove"] } else { permit };
    if identity["product_code"].as_str() != Some(values[3].as_str())
        || identity["package_code"].as_str() != Some(values[4].as_str())
    {
        return Err("admission callback product/package mismatch".into());
    }
    if values[5].len() != 32
        || !values[5]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("admission challenge invalid".into());
    }
    Ok(())
}

fn run_admitted_msi(command: &Command, root: &Path, step: &str) -> Result<InstallOutcome, String> {
    use std::{ffi::c_void, io::Write, os::windows::ffi::OsStrExt, sync::Mutex};
    type Callback = Option<unsafe extern "system" fn(*mut c_void, u32, u32) -> i32>;
    #[link(name = "msi")]
    extern "system" {
        fn MsiSetExternalUIRecord(
            handler: Callback,
            filter: u32,
            context: *mut c_void,
            old: *mut Callback,
        ) -> u32;
        fn MsiSetInternalUI(level: u32, window: *mut *mut c_void) -> u32;
        fn MsiInstallProductW(package: *const u16, properties: *const u16) -> u32;
        fn MsiEnableLogW(mode: u32, path: *const u16, attributes: u32) -> u32;
        fn MsiRecordGetStringW(record: u32, field: u32, text: *mut u16, count: *mut u32) -> u32;
    }
    struct Session {
        root: std::path::PathBuf,
        permit: serde_json::Value,
        used: bool,
        removal_used: bool,
        removal_response: Option<std::path::PathBuf>,
        error: Option<String>,
    }
    unsafe extern "system" fn callback(context: *mut c_void, message: u32, record: u32) -> i32 {
        if context.is_null() || message & 0xff000000 != 0x04000000 {
            return 0;
        }
        let context = &*(context as *const Mutex<Session>);
        let mut session = match context.lock() {
            Ok(s) => s,
            Err(_) => return 2,
        };
        let get = |field| -> Result<String, String> {
            let mut bytes = [0u16; 512];
            let mut count = 511;
            if MsiRecordGetStringW(record, field, bytes.as_mut_ptr(), &mut count) != 0 {
                return Err("admission callback field invalid".into());
            }
            String::from_utf16(&bytes[..count as usize])
                .map_err(|_| "admission callback encoding".into())
        };
        let kind = get(0).unwrap_or_default();
        if kind != "RDMSI_ADMISSION_V2" && kind != "RDMSI_REMOVE_V3" {
            return 0;
        }
        let removal = kind == "RDMSI_REMOVE_V3";
        let result = (|| -> Result<(), String> {
            let values = (1..=8).map(get).collect::<Result<Vec<_>, _>>()?;
            admission_callback_binding(
                &session.permit,
                session.used,
                session.removal_used,
                removal,
                &values,
            )?;
            let challenge = &values[5];
            let owner = super::handoff::Owner {
                pid: values[6].parse().map_err(|_| "admission CA PID")?,
                created_at: values[7].parse().map_err(|_| "admission CA creation")?,
            };
            if !super::handoff::ObservedProcess::owner_alive(&owner)? {
                return Err("admission CA not alive".into());
            }
            let response = serde_json::json!({"schema":2,"role":if removal{"remove"}else{"install"},"parent_product_code":session.permit["product_code"],"parent_package_code":session.permit["package_code"],"transaction_id":session.permit["transaction_id"],"step":session.permit["step"],"nonce":session.permit["nonce"],
                "challenge":challenge,"product_code":values[3],"package_code":values[4],"helper":session.permit["helper"],"ca":owner});
            let path = if removal {
                super::system_dirs::get()
                    .agent()
                    .join("state")
                    .join(format!("msi-remove-session-{challenge}.json"))
            } else {
                session
                    .root
                    .join(format!("msi-{}-session-{challenge}.json", values[1]))
            };
            // The shared state directory may grant read access to other identities.
            // Use the transaction workspace to create protected bytes, then move
            // the same file into the fixed rendezvous slot without inheriting ACLs.
            let staged = if removal {
                session
                    .root
                    .join(format!("msi-remove-session-{challenge}.json"))
            } else {
                path.clone()
            };
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged)
                .map_err(|e| e.to_string())?;
            file.write_all(&serde_json::to_vec(&response).map_err(|e| e.to_string())?)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
            drop(file);
            super::workspace::verify_file(&staged)?;
            if removal {
                use std::os::windows::ffi::OsStrExt;
                #[link(name = "kernel32")]
                extern "system" {
                    fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
                }
                let a = staged
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                let b = path
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                if MoveFileExW(a.as_ptr(), b.as_ptr(), 8) == 0 {
                    return Err("removal response publication refused".into());
                }
            }
            super::workspace::verify_file(&path)?;
            if removal {
                session.removal_used = true;
                session.removal_response = Some(path);
            } else {
                session.used = true;
            }
            Ok(())
        })();
        if let Err(e) = result {
            session.error = Some(e);
            return 2;
        }
        1
    }
    // Windows Installer UI/log state is process-global. This path runs only in
    // the independent transaction helper and serializes calls in that process.
    static API_LOCK: Mutex<()> = Mutex::new(());
    static API_UNUSABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let _serial = API_LOCK.lock().map_err(|_| "admission MSI API poisoned")?;
    if API_UNUSABLE.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("admission MSI callback remains registered; reconciliation required".into());
    }
    let permit = serde_json::from_slice(
        &std::fs::read(root.join(format!("msi-{step}-permit.json"))).map_err(|e| e.to_string())?,
    )
    .map_err(|_| "admission permit malformed")?;
    let session = Box::new(Mutex::new(Session {
        root: root.into(),
        permit,
        used: false,
        removal_used: false,
        removal_response: None,
        error: None,
    }));
    let args = command.get_args().collect::<Vec<_>>();
    let mut properties = Vec::new();
    let mut log = None;
    let mut index = 2;
    while index < args.len() {
        let arg = args[index].to_string_lossy();
        if arg.eq_ignore_ascii_case("/qn") || arg.eq_ignore_ascii_case("/norestart") {
            index += 1;
            continue;
        }
        if arg.eq_ignore_ascii_case("/L*v") {
            index += 1;
            log = args.get(index).copied();
            index += 1;
            continue;
        }
        if !arg.contains('=') || arg.contains(['"', '\r', '\n']) {
            return Err("admitted MSI argument unsupported".into());
        }
        let (key, value) = arg.split_once('=').ok_or("admitted MSI property invalid")?;
        properties.push(format!("{key}=\"{value}\""));
        index += 1;
    }
    let package = args[1].encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let properties = properties
        .join(" ")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let log = log
        .ok_or("admission log missing")?
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        let old_ui = MsiSetInternalUI(2, std::ptr::null_mut());
        let mut old = None;
        let registration = MsiSetExternalUIRecord(
            Some(callback),
            0x10,
            &*session as *const _ as *mut c_void,
            &mut old,
        );
        if registration != 0 {
            MsiSetInternalUI(old_ui, std::ptr::null_mut());
            return Err(format!("admission UI registration: {registration}"));
        }
        let logging = MsiEnableLogW(0x3fff, log.as_ptr(), 2);
        let code = if logging == 0 {
            MsiInstallProductW(package.as_ptr(), properties.as_ptr())
        } else {
            logging
        };
        let (released, cleanup_error) = admission_api_cleanup(
            || MsiSetExternalUIRecord(old, 0, std::ptr::null_mut(), std::ptr::null_mut()),
            || MsiSetExternalUIRecord(None, 0, std::ptr::null_mut(), std::ptr::null_mut()),
            || MsiEnableLogW(0, std::ptr::null(), 0),
        );
        MsiSetInternalUI(old_ui, std::ptr::null_mut());
        if logging != 0 || cleanup_error.is_some() {
            let error = format!(
                "MSI returned {code}; log setup={logging}; {}; reconciliation required",
                cleanup_error.as_deref().unwrap_or("UI cleanup succeeded")
            );
            // Keep the observed MSI outcome separate from the normal receipt:
            // cleanup failure must leave the launch intent pending, not retry MSI.
            if !released {
                // The process-global handler still refers to this address. Keep
                // it alive and block subsequent API use until this helper exits.
                let _retained = Box::into_raw(session);
                API_UNUSABLE.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            let diagnostic = serde_json::json!({"schema":1,"step":step,"outcome":classify(code as i32),"msi_started":logging==0,"error":error});
            let persisted = super::state_store::publish(
                &root.join(format!("msi-{step}-reconciliation.json")),
                &serde_json::to_vec(&diagnostic).map_err(|e| e.to_string())?,
            );
            return Err(match persisted {
                Ok(()) => error,
                Err(e) => format!("{error}; diagnostic persistence failed: {e}"),
            });
        }
        let session = (*session)
            .into_inner()
            .map_err(|_| "admission callback poisoned")?;
        if let Some(path) = session.removal_response {
            super::workspace::verify_file(&path)?;
            let name = path.file_name().ok_or("removal response name absent")?;
            let destination = root.join(name);
            let a = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let b = destination
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            #[link(name = "kernel32")]
            extern "system" {
                fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> i32;
            }
            if MoveFileExW(a.as_ptr(), b.as_ptr(), 8) == 0 {
                return Err("removal response archival failed; preserve evidence".into());
            }
        }
        if let Some(error) = session.error {
            return Err(format!(
                "{error}; MSI returned {code}; reconciliation required"
            ));
        }
        if code == 0 && !session.used {
            return Err("MSI completed without admission callback; reconciliation required".into());
        }
        Ok(classify(code as i32))
    }
}

fn admission_cached_product_available(
    step: &str,
    status: u32,
    confirm_absent: impl FnOnce() -> u32,
) -> Result<bool, String> {
    if step == "rollback" && status == 1605 {
        let confirmed = confirm_absent();
        return if confirmed == 1605 {
            Ok(false)
        } else {
            Err(format!(
                "MSI removal target absence unconfirmed: {confirmed}"
            ))
        };
    }
    if status == 0 {
        Ok(true)
    } else {
        Err(format!("MSI removal cached package unavailable: {status}"))
    }
}

fn admission_api_cleanup(
    restore: impl FnOnce() -> u32,
    detach: impl FnOnce() -> u32,
    stop_log: impl FnOnce() -> u32,
) -> (bool, Option<String>) {
    let restored = restore();
    let detached = if restored != 0 { detach() } else { 0 };
    let logging = stop_log();
    let error = if restored != 0 || logging != 0 {
        Some(format!("MSI UI cleanup restore={restored} detach={detached} log={logging}; reconciliation required"))
    } else {
        None
    };
    (restored == 0 || detached == 0, error)
}

fn prepare_msi_removal(
    root: &Path,
    step: &str,
    plan: &serde_json::Value,
    read: &impl Fn(&Path, u64) -> Result<(serde_json::Value, String), String>,
) -> Result<Option<serde_json::Value>, String> {
    use sha2::{Digest, Sha256};
    use std::{io::Read, os::windows::fs::OpenOptionsExt};
    // A missing NativeAssets is valid only for a same-product fixture/repair:
    // it authorizes no nested uninstall. Other observation failures are fatal.
    match std::fs::symlink_metadata(root.join("native-assets.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    let (assets, assets_hash) = read(&root.join("native-assets.json"), 65536)?;
    if assets["transaction_id"] != plan["transaction_id"]
        || !matches!(assets["schema"].as_u64(), Some(1 | 2))
    {
        return Err("MSI removal native assets binding mismatch".into());
    }
    let (artifact, product) = if step == "install" {
        (&assets["original_msi"], &assets["original_product_code"])
    } else {
        (&plan["package"], &assets["target_product_code"])
    };
    if assets["original_product_code"] == assets["target_product_code"] {
        return Ok(None);
    }
    let relative = artifact["relative_path"]
        .as_str()
        .ok_or("MSI removal artifact missing")?;
    let basename = relative.strip_prefix("original/").unwrap_or(relative);
    if !basename.ends_with(".msi")
        || !basename
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
    {
        return Err("MSI removal path invalid".into());
    }
    let path = root.join(relative);
    super::workspace::verify_file(&path)?;
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&b[..n]);
    }
    if artifact["sha256"].as_str() != Some(hex::encode(hash.finalize()).as_str())
        || artifact["size"].as_u64() != Some(f.metadata().map_err(|e| e.to_string())?.len())
    {
        return Err("MSI removal original digest mismatch".into());
    }
    let identity = admission_metadata::inspect(&path)?;
    if product.as_str() != Some(identity.product_code.as_str())
        || identity.product_name != "RustDesk"
    {
        return Err("MSI removal original product mismatch".into());
    }
    // LocalPackage is the Installer-managed cached database used for removal,
    // not the full signed source MSI. Bind both identities/digests independently.
    #[link(name = "msi")]
    extern "system" {
        fn MsiGetProductInfoW(
            product: *const u16,
            property: *const u16,
            value: *mut u16,
            size: *mut u32,
        ) -> u32;
        fn MsiGetProductInfoExW(
            product: *const u16,
            sid: *const u16,
            context: u32,
            property: *const u16,
            value: *mut u16,
            size: *mut u32,
        ) -> u32;
        fn MsiGetSummaryInformationW(
            db: u32,
            path: *const u16,
            count: u32,
            summary: *mut u32,
        ) -> u32;
        fn MsiSummaryInfoGetPropertyW(
            summary: u32,
            property: u32,
            kind: *mut u32,
            number: *mut i32,
            time: *mut u64,
            value: *mut u16,
            size: *mut u32,
        ) -> u32;
        fn MsiCloseHandle(handle: u32) -> u32;
    }
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let code = wide(&identity.product_code);
    let property = wide("LocalPackage");
    let mut buffer = [0u16; 32768];
    let mut n = 32767;
    let status = unsafe {
        MsiGetProductInfoW(
            code.as_ptr(),
            property.as_ptr(),
            buffer.as_mut_ptr(),
            &mut n,
        )
    };
    if !admission_cached_product_available(step, status, || {
        let property = wide("State");
        let mut state = [0u16; 16];
        let mut size = 15;
        unsafe {
            MsiGetProductInfoExW(
                code.as_ptr(),
                std::ptr::null(),
                4,
                property.as_ptr(),
                state.as_mut_ptr(),
                &mut size,
            )
        }
    })? {
        return Ok(None);
    }
    let cache =
        String::from_utf16(&buffer[..n as usize]).map_err(|_| "MSI removal cache encoding")?;
    let cache_path = Path::new(&cache);
    if !cache_path
        .parent()
        .map(|p| {
            p.to_string_lossy().eq_ignore_ascii_case(
                &super::system_dirs::get()
                    .installer_cache()
                    .to_string_lossy(),
            )
        })
        .unwrap_or(false)
        || !cache_path
            .extension()
            .map(|e| e.to_string_lossy().eq_ignore_ascii_case("msi"))
            .unwrap_or(false)
    {
        return Err("MSI removal cache outside Installer".into());
    }
    for p in cache_path.ancestors() {
        use std::os::windows::fs::MetadataExt;
        if std::fs::symlink_metadata(p)
            .map_err(|e| e.to_string())?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err("MSI removal cache reparse".into());
        }
    }
    let cached_identity = admission_metadata::inspect(cache_path)?;
    if cached_identity != identity {
        return Err("MSI removal cache product identity mismatch".into());
    }
    let summary_code = |path: &Path| -> Result<String, String> {
        use std::os::windows::ffi::OsStrExt;
        let path = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut summary = 0;
        if unsafe { MsiGetSummaryInformationW(0, path.as_ptr(), 0, &mut summary) } != 0 {
            return Err("MSI removal summary unavailable".into());
        }
        let (mut kind, mut number, mut time, mut len) = (0, 0, 0u64, 127);
        let mut text = [0u16; 128];
        let result = unsafe {
            MsiSummaryInfoGetPropertyW(
                summary,
                9,
                &mut kind,
                &mut number,
                &mut time,
                text.as_mut_ptr(),
                &mut len,
            )
        };
        let close = unsafe { MsiCloseHandle(summary) };
        if result != 0 || close != 0 {
            return Err("MSI removal summary read failed".into());
        }
        String::from_utf16(&text[..len as usize])
            .map_err(|_| "MSI removal package code encoding".into())
    };
    let package_code = summary_code(&path)?;
    if summary_code(cache_path)? != package_code {
        return Err("MSI removal cached PackageCode mismatch".into());
    }
    let cache_hash = hex::encode(Sha256::digest(
        std::fs::read(cache_path).map_err(|e| e.to_string())?,
    ));
    Ok(Some(
        serde_json::json!({"product_code":identity.product_code,"package_code":package_code,
        "source":artifact,"native_assets_sha256":assets_hash,"cached_path":cache,"cached_sha256":cache_hash}),
    ))
}

// Acceptance-only: both the caller build and package must opt in. Old admitted
// package removal and legacy callers still require compatibility verification.
fn prepare_msi_admission(
    command: &mut Command,
    root: &Path,
    step: &str,
    id: &str,
    package_hash: &str,
) -> Result<Option<std::fs::File>, String> {
    use sha2::{Digest, Sha256};
    use std::{
        io::Read,
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    };
    #[link(name = "msi")]
    extern "system" {
        fn MsiOpenDatabaseW(path: *const u16, mode: *const u16, db: *mut u32) -> u32;
        fn MsiDatabaseOpenViewW(db: u32, query: *const u16, view: *mut u32) -> u32;
        fn MsiViewExecute(view: u32, record: u32) -> u32;
        fn MsiViewFetch(view: u32, record: *mut u32) -> u32;
        fn MsiRecordGetStringW(record: u32, field: u32, text: *mut u16, size: *mut u32) -> u32;
        fn MsiGetSummaryInformationW(
            db: u32,
            path: *const u16,
            count: u32,
            summary: *mut u32,
        ) -> u32;
        fn MsiSummaryInfoGetPropertyW(
            summary: u32,
            property: u32,
            kind: *mut u32,
            value: *mut i32,
            time: *mut u64,
            text: *mut u16,
            size: *mut u32,
        ) -> u32;
        fn MsiCloseHandle(handle: u32) -> u32;
    }
    struct Msi(u32);
    impl Drop for Msi {
        fn drop(&mut self) {
            if self.0 != 0 {
                let status = unsafe { MsiCloseHandle(self.0) };
                if status != 0 {
                    eprintln!("MSI admission metadata close failed: {status}");
                }
            }
        }
    }
    let args = command.get_args().collect::<Vec<_>>();
    if !command
        .get_program()
        .to_string_lossy()
        .replace('/', "\\")
        .eq_ignore_ascii_case(
            &super::system_dirs::get()
                .system32()
                .join("msiexec.exe")
                .to_string_lossy(),
        )
    {
        return Ok(None);
    }
    if args.len() < 2 || args[0] != "/i" {
        return Err("MSI admission arguments invalid".into());
    }
    let package = Path::new(args[1]);
    let wide = package
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut db = 0;
    let status = unsafe { MsiOpenDatabaseW(wide.as_ptr(), std::ptr::null(), &mut db) };
    if status != 0 {
        return Err(format!("MSI admission metadata open: {status}"));
    }
    let db = Msi(db);
    let property = |name: &str| -> Result<Option<String>, String> {
        let query = format!("SELECT `Value` FROM `Property` WHERE `Property`='{name}'")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut view = 0;
        if unsafe { MsiDatabaseOpenViewW(db.0, query.as_ptr(), &mut view) } != 0 {
            return Err("MSI admission property view".into());
        }
        let view = Msi(view);
        if unsafe { MsiViewExecute(view.0, 0) } != 0 {
            return Err("MSI admission property execute".into());
        }
        let mut row = 0;
        let status = unsafe { MsiViewFetch(view.0, &mut row) };
        if status == 259 {
            return Ok(None);
        }
        if status != 0 {
            return Err("MSI admission property fetch".into());
        }
        let row = Msi(row);
        let mut buf = [0u16; 256];
        let mut n = 255;
        if unsafe { MsiRecordGetStringW(row.0, 1, buf.as_mut_ptr(), &mut n) } != 0 {
            return Err("MSI admission property read".into());
        }
        String::from_utf16(&buf[..n as usize])
            .map(Some)
            .map_err(|_| "MSI admission encoding".into())
    };
    let schema = property("RUSTDESK_MSI_ADMISSION_SCHEMA")?;
    if schema.is_none() {
        return Ok(None);
    }
    if schema.as_deref() != Some("1") {
        return Err("MSI admission schema unsupported".into());
    }
    if args.iter().any(|arg| {
        let arg = arg.to_string_lossy().to_ascii_uppercase();
        arg.starts_with("RUSTDESK_ADMISSION_")
            || arg.starts_with("TRANSFORMS=")
            || arg.starts_with("PATCH=")
    }) {
        return Err("MSI admission unexpected caller override".into());
    }
    if root
        != super::system_dirs::get()
            .agent()
            .join("transactions")
            .join(id)
    {
        return Err("MSI admission requires fixed transaction directory".into());
    }
    let read = |path: &Path, limit: u64| -> Result<(serde_json::Value, String), String> {
        super::workspace::verify_file(path)?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("MSI admission record too large".into());
        }
        let value = serde_json::from_slice(&bytes).map_err(|_| "MSI admission record malformed")?;
        Ok((value, hex::encode(Sha256::digest(&bytes))))
    };
    // Profiles are selected from exclusive fixed records, never a caller property.
    let present = |name: &str| -> Result<bool, String> {
        match std::fs::symlink_metadata(root.join(name)) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("MSI admission profile unavailable: {e}")),
        }
    };
    let local = match (
        present("prepared.json")?,
        present("local-repair-plan.json")?,
    ) {
        (true, false) => false,
        (false, true) => true,
        _ => return Err("MSI admission missing or mixed plan profiles".into()),
    };
    if local && step != "install" {
        return Err("local repair does not authorize rollback MSI".into());
    }
    let (plan, plan_hash) = read(
        &root.join(if local {
            "local-repair-plan.json"
        } else {
            "prepared.json"
        }),
        65536,
    )?;
    let (handoff, handoff_hash) = read(&root.join("handoff.json"), 4096)?;
    let (marker, marker_hash) = read(
        &super::system_dirs::get()
            .agent()
            .join(r"state\maintenance.json"),
        4096,
    )?;
    let (transaction, transaction_hash) = read(&root.join("transaction.json"), 65536)?;
    if plan["schema"] != 1
        || !matches!(
            plan["target_key"].as_str(),
            Some("windows-x86_64-msi-standard" | "windows-x86_64-msi-sos")
        )
        || marker["schema"] != 1
        || transaction["schema"] != 1
        || transaction["package_sha256"] != plan["package"]["sha256"]
    {
        return Err("MSI admission source schema mismatch".into());
    }
    let historical: super::handoff::Handoff =
        serde_json::from_value(handoff.clone()).map_err(|_| "MSI admission handoff malformed")?;
    let ready: super::handoff::ReadyReceipt = serde_json::from_value(
        read(
            &root.join(if local {
                "local-ready.json"
            } else {
                "helper-ready.json"
            }),
            4096,
        )?
        .0,
    )
    .map_err(|_| "MSI admission ready malformed")?;
    let release: super::handoff::ReleaseReceipt = serde_json::from_value(
        read(
            &root.join(if local {
                "local-release.json"
            } else {
                "helper-release.json"
            }),
            4096,
        )?
        .0,
    )
    .map_err(|_| "MSI admission release malformed")?;
    historical.verify_release(&ready, &release, &historical.helper, release.released_at)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "MSI admission clock invalid")?
        .as_secs();
    if release.released_at > now {
        return Err("MSI admission future handoff".into());
    }
    let current = super::handoff::ObservedProcess::open(std::process::id())?;
    let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if current_exe != root.join("runtime/helper.exe")
        || plan["helper"]["relative_path"] != "runtime/helper.exe"
        || plan["transaction_id"] != id
        || handoff["transaction_id"] != id
        || marker["transaction_id"] != id
        || transaction["id"] != id
        || handoff["plan_sha256"] != plan_hash
        || marker["plan_sha256"] != plan_hash
        || marker["nonce"] != handoff["nonce"]
        || marker["installation_id"] != plan["installation_id"]
        || !matches!(
            (step, transaction["phase"].as_str()),
            ("install", Some("applying")) | ("rollback", Some("rolling_back"))
        )
    {
        return Err("MSI admission transaction binding mismatch".into());
    }
    if current.owner() != &historical.helper
        && super::handoff::ObservedProcess::owner_alive(&historical.helper)?
    {
        return Err("MSI admission previous helper still alive".into());
    }
    for gate in ["disable-agent", "disable-recovery"] {
        match std::fs::symlink_metadata(super::system_dirs::get().agent().join("state").join(gate))
        {
            Ok(_) => return Err("MSI admission locally blocked".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("MSI admission local gate unavailable: {e}")),
        }
    }
    super::workspace::verify_file(&current_exe)?;
    let helper_hash = hex::encode(Sha256::digest(
        std::fs::read(&current_exe).map_err(|e| e.to_string())?,
    ));
    if plan["helper"]["sha256"] != helper_hash {
        return Err("MSI admission helper digest mismatch".into());
    }
    let artifact = if step == "install" {
        plan["package"].clone()
    } else {
        read(&root.join("native-assets.json"), 65536)?.0["original_msi"].clone()
    };
    let relative = artifact["relative_path"]
        .as_str()
        .ok_or("MSI admission artifact absent")?;
    let basename = relative.strip_prefix("original/").unwrap_or(relative);
    if basename.is_empty()
        || !basename.ends_with(".msi")
        || !basename
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
    {
        return Err("MSI admission package basename invalid".into());
    }
    if package != root.join(relative) || artifact["sha256"] != package_hash {
        return Err("MSI admission package differs from plan".into());
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(package)
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if hex::encode(hash.finalize()) != package_hash
        || artifact["size"].as_u64() != Some(file.metadata().map_err(|e| e.to_string())?.len())
    {
        return Err("MSI admission package hash/size mismatch".into());
    }
    if local
        && (plan["trigger"] != "local_recovery"
            || plan["original_normal"] != "missing"
            || plan["product_code"].as_str() != property("ProductCode")?.as_deref())
    {
        return Err("local repair MSI source contract mismatch".into());
    }
    let mut summary = 0;
    if unsafe { MsiGetSummaryInformationW(db.0, std::ptr::null(), 0, &mut summary) } != 0 {
        return Err("MSI admission summary unavailable".into());
    }
    let summary = Msi(summary);
    let (mut kind, mut value, mut time, mut count) = (0, 0, 0u64, 127);
    let mut code = [0u16; 128];
    if unsafe {
        MsiSummaryInfoGetPropertyW(
            summary.0,
            9,
            &mut kind,
            &mut value,
            &mut time,
            code.as_mut_ptr(),
            &mut count,
        )
    } != 0
    {
        return Err("MSI admission package code unavailable".into());
    }
    let code = String::from_utf16(&code[..count as usize])
        .map_err(|_| "MSI admission package code encoding")?;
    let nonce = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let removal = if local {
        None
    } else {
        prepare_msi_removal(root, step, &plan, &read)?
    };
    let permit = serde_json::json!({"schema":2,"source_kind":if local{"local_repair"}else{"transaction"},"remove":removal,"transaction_id":id,"step":step,"nonce":nonce,"plan_sha256":plan_hash,
        "handoff_sha256":handoff_hash,"marker_sha256":marker_hash,"transaction_sha256":transaction_hash,
        "package_relative_path":relative,"package_sha256":package_hash,"package_size":artifact["size"],
        "product_code":property("ProductCode")?.ok_or("MSI admission product absent")?,"package_code":code,
        "helper":{"pid":current.owner().pid,"created_at":current.owner().created_at},"helper_sha256":plan["helper"]["sha256"]});
    let path = root.join(format!("msi-{step}-permit.json"));
    use std::io::Write;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(&path)
        .map_err(|e| format!("MSI admission permit creation: {e}"))?;
    output
        .write_all(&serde_json::to_vec(&permit).map_err(|e| e.to_string())?)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())?;
    drop(output);
    super::workspace::verify_file(&path)?;
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .map_err(|e| e.to_string())?;
    command
        .arg(format!("RUSTDESK_ADMISSION_TX={id}"))
        .arg(format!("RUSTDESK_ADMISSION_STEP={step}"))
        .arg(format!("RUSTDESK_ADMISSION_NONCE={nonce}"));
    Ok(Some(locked))
}

#[cfg(test)]
mod admission_binding_tests {
    use super::*;
    fn fixture() -> (serde_json::Value, Vec<String>) {
        let permit = serde_json::json!({"transaction_id":"a".repeat(32),"step":"install","nonce":"b".repeat(64),
            "product_code":"new-product","package_code":"new-package",
            "remove":{"product_code":"old-product","package_code":"old-package"}});
        let fields = vec![
            "".into(),
            "".into(),
            "".into(),
            "old-product".into(),
            "old-package".into(),
            "c".repeat(32),
            "42".into(),
            "100".into(),
        ];
        (permit, fields)
    }
    #[test]
    fn removal_requires_top_session_and_exact_frozen_old_product_package() {
        let (permit, fields) = fixture();
        let before = permit.clone();
        assert!(admission_callback_binding(&permit, true, false, true, &fields).is_ok());
        assert!(
            admission_callback_binding(&permit, false, false, true, &fields)
                .unwrap_err()
                .contains("order")
        );
        for index in [3, 4] {
            let mut wrong = fields.clone();
            wrong[index] = "foreign".into();
            assert!(
                admission_callback_binding(&permit, true, false, true, &wrong)
                    .unwrap_err()
                    .contains("product/package")
            );
        }
        assert_eq!(permit, before);
    }
    #[test]
    fn removal_replay_properties_or_missing_frozen_authority_refused() {
        let (permit, fields) = fixture();
        assert!(
            admission_callback_binding(&permit, true, true, true, &fields)
                .unwrap_err()
                .contains("repeated")
        );
        for index in 0..3 {
            let mut forged = fields.clone();
            forged[index] = "public-property".into();
            assert!(
                admission_callback_binding(&permit, true, false, true, &forged)
                    .unwrap_err()
                    .contains("discover parent")
            );
        }
        let mut no_original = permit.clone();
        no_original["remove"] = serde_json::Value::Null;
        assert!(
            admission_callback_binding(&no_original, true, false, true, &fields)
                .unwrap_err()
                .contains("product/package")
        );
        assert!(no_original["remove"].is_null());
    }
    #[test]
    fn top_request_does_not_gain_nested_or_replay_authority() {
        let (permit, mut fields) = fixture();
        for (index, key) in [
            (0, "transaction_id"),
            (1, "step"),
            (2, "nonce"),
            (3, "product_code"),
            (4, "package_code"),
        ] {
            fields[index] = permit[key].as_str().unwrap().into();
        }
        assert!(admission_callback_binding(&permit, false, false, false, &fields).is_ok());
        assert!(
            admission_callback_binding(&permit, true, false, false, &fields)
                .unwrap_err()
                .contains("repeated")
        );
        assert!(admission_callback_binding(&permit, true, false, true, &fields).is_err());
        fields[5] = "../path".into();
        assert!(
            admission_callback_binding(&permit, false, false, false, &fields)
                .unwrap_err()
                .contains("challenge")
        );
    }
}

#[cfg(test)]
mod admission_program_tests {
    use super::*;

    fn inspect_program(
        program: impl AsRef<std::ffi::OsStr>,
        step: &str,
    ) -> Result<Option<std::fs::File>, String> {
        let mut command = Command::new(program);
        // Empty arguments deliberately stop at the argument gate: no MSI is
        // opened, no process is spawned and no transaction files are touched.
        prepare_msi_admission(
            &mut command,
            Path::new(r"C:\RustDeskAgentLab\unused-admission234"),
            step,
            &"a".repeat(32),
            &"b".repeat(64),
        )
    }

    #[test]
    fn native_installer_join_enters_admission_for_install_and_rollback() {
        for step in ["install", "rollback"] {
            let program = std::path::PathBuf::from(r"C:\Windows").join("System32/msiexec.exe");
            let result = inspect_program(&program, step);
            assert!(
                matches!(result, Err(ref error) if error == "MSI admission arguments invalid"),
                "native installer join silently skipped admission for {step}: {result:?}"
            );
            assert_eq!(
                program,
                std::path::PathBuf::from(r"C:\Windows").join("System32/msiexec.exe")
            );
        }
    }

    #[test]
    fn fixed_windows_case_and_separator_spellings_enter_same_gate() {
        for program in [
            r"C:\Windows\System32\msiexec.exe",
            "c:/WINDOWS/System32/msiexec.exe",
            r"C:\Windows/System32\msiexec.exe",
        ] {
            assert!(
                matches!(inspect_program(program, "install"), Err(ref error) if error == "MSI admission arguments invalid"),
                "{program}"
            );
        }
    }

    #[test]
    fn foreign_executable_or_alias_is_not_normalized_into_fixed_program() {
        for program in [
            r"D:\Windows\System32\msiexec.exe",
            r"C:\Windows\System32\other.exe",
            "msiexec.exe",
            r"C:Windows\System32\msiexec.exe",
            r"C:\Windows\System32\..\System32\msiexec.exe",
            r"\\?\C:\Windows\System32\msiexec.exe",
            r"\\server\share\msiexec.exe",
            r"C:\RustDeskAgentLab\reparse-alias\msiexec.exe",
            r"C:\Windows\System32\msiexec.exe:stream",
            r"C:\Windows\System32\msiexec.exe.",
        ] {
            assert!(
                matches!(inspect_program(program, "install"), Ok(None)),
                "foreign program admitted: {program}"
            );
        }
    }

    #[test]
    #[ignore = "VM-only: frozen admission MSI required; validates pre-launch rejection, never installs"]
    fn native_join_recorded_caller_reads_package_then_refuses_foreign_workspace() {
        use sha2::{Digest, Sha256};
        base::lab_guard::require_lab_os("win10");
        assert_eq!(option_env!("RUSTDESK_MSI_ADMISSION"), Some("1"));
        let package = Path::new(r"C:\RustDeskAgentLab\admission234-package.msi");
        let bytes = std::fs::read(package).unwrap();
        let hash = hex::encode(Sha256::digest(&bytes));
        assert_eq!(
            hash,
            "b323a297aefd7f7382943f225c4e68f502504739602bbf3d6aa328f7e104456d"
        );
        let root = std::env::temp_dir().join(format!("admission234-{}", uuid::Uuid::new_v4()));
        super::super::workspace::create(&root).unwrap();
        let sentinel = root.join("original.txt");
        std::fs::write(&sentinel, b"preserve original fixture").unwrap();
        for step in ["install", "rollback"] {
            let program = std::path::PathBuf::from(r"C:\Windows").join("System32/msiexec.exe");
            let mut command = Command::new(program);
            command.args(msi_arguments(package, &root.join("installer.log")).unwrap());
            let error = run_recorded(
                &mut command,
                Duration::from_secs(1),
                &root,
                step,
                &"a".repeat(32),
                &hash,
            )
            .unwrap_err();
            assert_eq!(error, "MSI admission requires fixed transaction directory");
            assert!(!root.join(format!("{step}-started.json")).exists());
            assert!(!root.join(format!("{step}-result.json")).exists());
            assert!(!root.join("installer.log").exists());
            assert_eq!(
                std::fs::read(&sentinel).unwrap(),
                b"preserve original fixture"
            );
        }
        assert_eq!(std::fs::read(package).unwrap(), bytes);
        std::fs::remove_file(sentinel).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

#[cfg(test)]
mod admission_recovery_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn rollback_unknown_product_requires_independent_machine_absence() {
        let calls = Cell::new(0);
        assert_eq!(
            admission_cached_product_available("rollback", 1605, || {
                calls.set(calls.get() + 1);
                1605
            }),
            Ok(false)
        );
        assert_eq!(calls.get(), 1);
        for status in [0, 5, 234, 1608, 1610, 1627] {
            let error =
                admission_cached_product_available("rollback", 1605, || status).unwrap_err();
            assert!(error.contains("unconfirmed"), "{status}: {error}");
        }
    }

    #[test]
    fn install_absence_and_all_other_query_errors_remain_fatal() {
        for step in ["install", "rollback"] {
            assert_eq!(
                admission_cached_product_available(step, 0, || panic!("unexpected confirmation")),
                Ok(true)
            );
            for status in [5, 234, 1608, 1610, 1627, u32::MAX] {
                assert_eq!(
                    admission_cached_product_available(step, status, || panic!(
                        "unexpected confirmation"
                    ))
                    .unwrap_err(),
                    format!("MSI removal cached package unavailable: {status}")
                );
            }
        }
        assert!(
            admission_cached_product_available("install", 1605, || panic!(
                "install cannot omit removal"
            ))
            .is_err()
        );
    }

    #[test]
    fn ui_restore_failure_detaches_callback_and_reports_reconciliation() {
        let calls = Cell::new(0);
        let (released, error) = admission_api_cleanup(
            || 120,
            || {
                calls.set(calls.get() + 1);
                0
            },
            || 0,
        );
        assert!(released);
        assert_eq!(calls.get(), 1);
        let error = error.unwrap();
        assert!(error.contains("restore=120") && error.contains("reconciliation"));
    }

    #[test]
    fn ui_double_failure_retains_callback_context_and_logging_failure_is_reported() {
        let (released, error) = admission_api_cleanup(|| 120, || 5, || 1627);
        assert!(!released, "registered callback context must stay allocated");
        let error = error.unwrap();
        assert!(error.contains("detach=5") && error.contains("log=1627"));
        let (released, error) = admission_api_cleanup(|| 0, || panic!("already restored"), || 5);
        assert!(released);
        assert!(error.unwrap().contains("log=5"));
    }

    #[test]
    fn successful_ui_cleanup_does_not_detach_restored_handler() {
        assert_eq!(
            admission_api_cleanup(|| 0, || panic!("old handler must stay restored"), || 0),
            (true, None)
        );
    }

    #[test]
    #[ignore = "Win11 SYSTEM only: frozen unregistered v235-new MSI; no installation"]
    fn actual_unregistered_target_omits_only_rollback_removal_and_preserves_assets() {
        use sha2::{Digest, Sha256};
        base::lab_guard::require_lab_os("win11");
        let package = Path::new(r"C:\RustDeskAgentLab\agent-sos-admission-v235-new.msi");
        let bytes = std::fs::read(package).unwrap();
        let hash = hex::encode(Sha256::digest(&bytes));
        assert_eq!(
            hash,
            "bb989af9d4ffad5a7a1c85518584f7a6fcf203c223a2f546f7a9bd6c436292db"
        );
        let root =
            std::env::temp_dir().join(format!("admission236-absent-{}", uuid::Uuid::new_v4()));
        super::super::workspace::create(&root).unwrap();
        std::fs::write(root.join("target.msi"), &bytes).unwrap();
        let plan = serde_json::json!({"transaction_id":"e2360000000000000000000000000002","package":{"relative_path":"target.msi","sha256":hash,"size":bytes.len()}});
        let assets = serde_json::json!({"schema":2,"transaction_id":plan["transaction_id"],"original_product_code":"{E2350000-0000-0000-0000-000000000001}","target_product_code":"{E2350000-0000-0000-0000-000000000002}","original_msi":plan["package"]});
        let assets_bytes = serde_json::to_vec(&assets).unwrap();
        std::fs::write(root.join("native-assets.json"), &assets_bytes).unwrap();
        let read = |path: &Path, _limit: u64| {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            Ok((
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
                hex::encode(Sha256::digest(bytes)),
            ))
        };
        assert!(prepare_msi_removal(&root, "rollback", &plan, &read)
            .unwrap()
            .is_none());
        let mut wrong = plan.clone();
        wrong["package"]["sha256"] = "0".repeat(64).into();
        assert_eq!(
            prepare_msi_removal(&root, "rollback", &wrong, &read).unwrap_err(),
            "MSI removal original digest mismatch"
        );
        let mut absent_install = assets.clone();
        absent_install["original_product_code"] = assets["target_product_code"].clone();
        absent_install["target_product_code"] = assets["original_product_code"].clone();
        std::fs::write(
            root.join("native-assets.json"),
            serde_json::to_vec(&absent_install).unwrap(),
        )
        .unwrap();
        assert_eq!(
            prepare_msi_removal(&root, "install", &plan, &read).unwrap_err(),
            "MSI removal cached package unavailable: 1605"
        );
        assert_eq!(std::fs::read(root.join("target.msi")).unwrap(), bytes);
        assert_eq!(std::fs::read(package).unwrap(), bytes);
        std::fs::remove_dir_all(root).unwrap();
    }
}
