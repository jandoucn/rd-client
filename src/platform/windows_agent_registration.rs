#[path = "../management_agent/handoff.rs"]
mod handoff;
#[path = "windows_agent_guard.rs"]
mod install_guard;
#[path = "../management_agent/registration_reconnect.rs"]
mod registration_reconnect;
#[path = "../management_agent/workspace.rs"]
mod workspace;

use std::{io::Read, path::Path};

fn read(path: &Path, identity: bool) -> Result<Vec<u8>, String> {
    workspace::verify_restore_target(path, identity)?;
    let mut bytes = vec![];
    std::fs::File::open(path)
        .map_err(|_| "registration gate unreadable")?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "registration gate unreadable")?;
    if bytes.len() > 65536 {
        return Err("registration gate too large".into());
    }
    Ok(bytes)
}

pub fn check_gates(
    state: &Path,
    config: &Path,
    is_server: bool,
    needs_deploy: bool,
    idle: bool,
    live_stop: &str,
) -> Result<(), String> {
    if !is_server {
        return Err("registration server role required".into());
    }
    if needs_deploy {
        return Err("registration deployment required".into());
    }
    if !idle {
        return Err("registration active sessions".into());
    }
    if !matches!(live_stop, "" | "N") {
        return Err("registration live intent refused".into());
    }
    for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
        match std::fs::symlink_metadata(state.join(marker)) {
            Ok(_) => return Err("registration locally blocked".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("registration marker unknown".into()),
        }
    }
    workspace::verify_restore_target(&state.join("service-recovery-enabled"), false)?;
    let agent: serde_json::Value = serde_json::from_slice(&read(&state.join("agent.json"), false)?)
        .map_err(|_| "registration state malformed")?;
    if agent["disabled"] != false || agent["management_agent_enabled"] != true {
        return Err("registration policy refused".into());
    }
    let bytes = read(config, true)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "registration intent encoding")?;
    let config: hbb_common::toml::Value =
        hbb_common::toml::from_str(text).map_err(|_| "registration intent malformed")?;
    match config
        .get("options")
        .and_then(|v| v.get("stop-service"))
        .and_then(|v| v.as_str())
    {
        Some("" | "N") => Ok(()),
        _ => Err("registration intent unknown or stopped".into()),
    }
}

#[cfg(not(test))]
pub fn start(build: u64) {
    if !crate::is_server() || !crate::platform::is_installed() {
        return;
    }
    let listener = match registration_reconnect::bind() {
        Ok(v) => v,
        Err(e) => {
            hbb_common::log::warn!("registration listener unavailable: {e}");
            return;
        }
    };
    hbb_common::tokio::spawn(async move {
        let result=registration_reconnect::serve(listener,build,||{
            use hbb_common::config::Config;
            use std::sync::atomic::Ordering;
            let _guard=install_guard::InstallGuard::try_acquire().map_err(|_|"registration install lock unavailable")?.ok_or("registration installation busy")?;
            let config=Config::file().with_file_name("RustDesk2.toml");
            if ![r"C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\RustDesk\config\RustDesk2.toml",r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RustDesk\config\RustDesk2.toml"].iter().any(|p|config.to_string_lossy().eq_ignore_ascii_case(p)){return Err("registration config outside fixed profiles".into());}
            check_gates(Path::new(r"C:\ProgramData\RustDesk\ManagementAgent\state"),&config,crate::is_server(),crate::rendezvous_mediator::NEEDS_DEPLOY.load(Ordering::SeqCst),crate::updater::has_no_active_conns(),&Config::get_option("stop-service"))?;
            if crate::platform::installing_service(){return Err("registration installation active".into());}
            if hbb_common::config::get_online_state()>0&&Config::get_key_confirmed(){return Err("registration already online".into());}
            if Config::get_rendezvous_servers()!=vec![String::from("rd.yan.life")]{return Err("registration server configuration mismatch".into());}
            crate::RendezvousMediator::restart();Ok(())
        }).await;
        if let Err(e) = result {
            hbb_common::log::warn!("registration listener ended: {e}");
        }
    });
}
#[cfg(test)]
pub fn start(_: u64) {}

#[cfg(test)]
pub fn with_install_gate(callback: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let _guard = install_guard::InstallGuard::try_acquire()
        .map_err(|_| "registration install lock unavailable")?
        .ok_or("registration installation busy")?;
    callback()
}
