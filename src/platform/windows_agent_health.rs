#[path = "../management_agent/handoff.rs"]
#[allow(dead_code)]
mod handoff;
#[path = "../management_agent/health.rs"]
mod health;
#[path = "../management_agent/health_windows.rs"]
mod health_windows;
#[path = "windows_agent_registration.rs"]
mod registration;

pub fn start() {
    let metadata: serde_json::Value = match serde_json::from_str(include_str!("../../version.json"))
    {
        Ok(value) => value,
        Err(error) => {
            hbb_common::log::warn!("local agent health build metadata invalid: {error}");
            return;
        }
    };
    let version = metadata["version"].as_str().unwrap_or("").to_owned();
    let build = metadata["build_seq"].as_u64().unwrap_or(0);
    registration::start(build);
    match health_windows::bind(r"\\.\pipe\RustDesk\management-health-v2") {
        Ok(listener) => {
            let activity_version = version.clone();
            hbb_common::tokio::spawn(async move {
                if let Err(error) = health_windows::serve_activity(listener, activity_version, build,
                    || hbb_common::config::get_online_state() > 0 && hbb_common::config::Config::get_key_confirmed(),
                    crate::updater::has_no_active_conns).await {
                    hbb_common::log::warn!("local agent activity listener ended: {error}");
                }
            });
        }
        Err(error) => hbb_common::log::warn!("local agent activity listener failed: {error}"),
    }
    let pipe = r"\\.\pipe\RustDesk\management-health-v1";
    let listener = match health_windows::bind(pipe) {
        Ok(listener) => listener,
        Err(error) => {
            hbb_common::log::warn!("local agent health listener failed: {error}");
            return;
        }
    };
    hbb_common::tokio::spawn(async move {
        if let Err(error) = health_windows::serve(listener, version, build, || {
            hbb_common::config::get_online_state() > 0
                && hbb_common::config::Config::get_key_confirmed()
        })
        .await
        {
            hbb_common::log::warn!("local agent health listener ended: {error}");
        }
    });
}
