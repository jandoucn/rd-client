use super::{identity, local_events, operations};
use local_events::{Binding, LocalEvents, Outcome, PathKey, Phase, Record};

#[path = "local_event_logic.rs"]
mod logic;
use logic::event_error;
pub use logic::{
    enrolled, format_recovery_error, r1, send_signed, upload, Observation, RecoveryResult,
};

// The caller owns the existing install mutex; the nested Windows start keeps its gates.
pub fn start_existing(state: &super::AgentState, paths: &super::AgentPaths) -> RecoveryResult {
    let run = || -> Result<RecoveryResult, String> {
        let Some(_guard) =
            super::install_guard::InstallGuard::try_acquire().map_err(|e| e.to_string())?
        else {
            return Ok(RecoveryResult {
                action: Ok(false),
                reporting: "idle",
            });
        };
        let path = paths.state.join("local-events").join("events.json");
        if !path.exists() {
            return Ok(RecoveryResult {
                action: super::recovery_windows::start_existing(state, paths),
                reporting: "unreported",
            });
        }
        let log = LocalEvents::open(&path).map_err(event_error)?;
        if let Some((_, old)) = log.active_r1() {
            let identity = super::identity::load_existing()?;
            if old.agent_installation_id != state.agent_installation_id
                || old.client_id != identity.id
                || old.client_uuid != identity.uuid
            {
                return Err("local R1 original installation requires reconciliation".into());
            }
        }
        let binding = log.verified_binding().cloned();
        let binding = binding.filter(|binding| {
            binding.agent_installation_id == state.agent_installation_id
                && super::identity::load_existing().is_ok_and(|identity| {
                    identity.id == binding.client_id && identity.uuid == binding.client_uuid
                })
        });
        Ok(r1(
            &path,
            binding.as_ref(),
            super::unix_timestamp(),
            || {
                let observed = super::recovery_windows::inspect(state, paths)?;
                Ok(if !observed.trusted_path {
                    Observation::Unknown
                } else {
                    match observed.state {
                        super::recovery::ServiceState::Running => Observation::Running,
                        super::recovery::ServiceState::Stopped => Observation::Stopped,
                        _ => Observation::Unknown,
                    }
                })
            },
            || super::recovery_windows::start_existing(state, paths),
        ))
    };
    run().unwrap_or_else(|error| RecoveryResult {
        action: Err(error),
        reporting: "degraded",
    })
}

pub fn poll(
    identity: &super::identity::DeviceIdentity,
    base: &str,
    binding: &serde_json::Value,
    paths: &super::AgentPaths,
) {
    let binding = serde_json::from_value::<Binding>(serde_json::json!({
        "client_id":binding["client_id"],"client_uuid":binding["client_uuid"],
        "agent_installation_id":binding["agent_installation_id"],"enrollment_generation":binding["enrollment_generation"]}));
    let result = binding
        .map_err(|_| "local event binding malformed".to_owned())
        .and_then(|binding| {
            let root = paths.state.join("local-events");
            if !root.exists() {
                super::workspace::create(&root)?;
            } else {
                super::workspace::verify(&root)?;
            }
            super::local_recovery_events::reconcile_repairs(
                identity,
                &binding,
                &paths.state,
                &paths.agent,
            )?;
            enrolled(
                &root.join("events.json"),
                binding,
                super::unix_timestamp(),
                |body| send_signed(identity, base, body),
            )
        });
    if let Err(error) = result {
        hbb_common::throttled_log!(
            std::time::Duration::from_secs(60),
            warn,
            "local event upload: {error}"
        );
    }
}

fn reconciliation_candidate(
    path: &std::path::Path,
) -> Result<Option<(local_events::Operation, Binding)>, String> {
    LocalEvents::open(path)
        .map(|log| log.active_r1())
        .map_err(event_error)
}

pub fn reconcile_running(state: &super::AgentState, paths: &super::AgentPaths) {
    let path = paths.state.join("local-events").join("events.json");
    if !path.exists() {
        return;
    }
    match reconciliation_candidate(&path) {
        Ok(Some(_)) => {}
        Ok(None) => return,
        Err(error) => {
            hbb_common::throttled_log!(
                std::time::Duration::from_secs(60),
                warn,
                "local R1 reconciliation: {error}"
            );
            return;
        }
    }
    if super::recovery_windows::inspect(state, paths).is_ok_and(|observed| {
        observed.trusted_path && observed.state == super::recovery::ServiceState::Running
    }) {
        // Only a read-only SCM observation may close an interrupted successful start here.
        let result = (|| -> Result<(), String> {
            let Some(_guard) =
                super::install_guard::InstallGuard::try_acquire().map_err(|e| e.to_string())?
            else {
                return Ok(());
            };
            let mut log = LocalEvents::open(&path).map_err(event_error)?;
            let Some((operation, binding)) = log.active_r1() else {
                return Ok(());
            };
            let identity = super::identity::load_existing()?;
            if binding.agent_installation_id != state.agent_installation_id
                || binding.client_id != identity.id
                || binding.client_uuid != identity.uuid
            {
                return Err("local R1 original installation requires reconciliation".into());
            }
            if super::recovery_windows::inspect(state, paths)?.state
                != super::recovery::ServiceState::Running
            {
                return Ok(());
            }
            log.record_bound_at(
                &operation,
                Record {
                    phase: Phase::Terminal,
                    attempt_seq: 1,
                    outcome: Outcome::Succeeded,
                    path_key: PathKey::RustDeskService,
                    error_code: None,
                },
                &binding,
                super::unix_timestamp(),
            )
            .map_err(event_error)?;
            Ok(())
        })();
        if let Err(error) = result {
            hbb_common::throttled_log!(
                std::time::Duration::from_secs(60),
                warn,
                "local R1 reconciliation: {error}"
            );
        }
    }
}

#[cfg(test)]
#[path = "../../tests/management_agent_local_events/reconcile.rs"]
mod reconciliation_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_r1_stopped_service_records_before_ensure_start_without_installation_changes() {
        use std::time::{Duration, Instant};
        use windows_service::{
            service::{ServiceAccess, ServiceState},
            service_manager::{ServiceManager, ServiceManagerAccess},
        };
        base::lab_guard::require_lab_os("win10");
        assert_eq!(
            std::env::var("LOCAL_EVENTS_NATIVE_R1").unwrap(),
            "authorized"
        );
        let root =
            std::path::PathBuf::from(std::env::var_os("RUSTDESK_LOCAL_EVENTS_ROOT").unwrap())
                .join("native-r1");
        super::super::workspace::create(&root).unwrap();
        let paths = super::super::AgentPaths::from_program_data(
            std::path::Path::new(r"C:\ProgramData"),
            super::super::Edition::Sos,
        );
        let before = std::fs::read(paths.state.join("agent.json")).unwrap();
        let state: super::super::AgentState = serde_json::from_slice(&before).unwrap();
        assert_eq!(
            state.agent_installation_id,
            "743e15c593b9c60038f42e5e73f7faa2"
        );
        let identity_before = super::super::identity::load_existing().unwrap();
        let manager =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).unwrap();
        let service = manager
            .open_service(
                "RustDesk",
                ServiceAccess::STOP | ServiceAccess::START | ServiceAccess::QUERY_STATUS,
            )
            .unwrap();
        assert_eq!(
            service.query_status().unwrap().current_state,
            ServiceState::Running
        );
        let _guard = super::super::install_guard::InstallGuard::try_acquire()
            .unwrap()
            .unwrap();
        service.stop().unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while service.query_status().unwrap().current_state != ServiceState::Stopped {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(50));
        }
        let binding = Binding {
            client_id: "test-device".into(),
            client_uuid: "test-uuid".into(),
            agent_installation_id: "a".repeat(32),
            enrollment_generation: 1,
        };
        let path = root.join("events.json");
        let result = r1(
            &path,
            Some(&binding),
            super::super::unix_timestamp(),
            || {
                let observed = super::super::recovery_windows::inspect(&state, &paths)?;
                Ok(match observed.state {
                    super::super::recovery::ServiceState::Stopped => Observation::Stopped,
                    super::super::recovery::ServiceState::Running => Observation::Running,
                    _ => Observation::Unknown,
                })
            },
            || {
                let log = LocalEvents::open(&path).map_err(event_error)?;
                assert_eq!(log.pending_events()[0].payload["status"], "running");
                super::super::recovery_windows::start_existing(&state, &paths)
            },
        );
        // Restore the service even when the assertion below exposes a regression.
        if service.query_status().unwrap().current_state == ServiceState::Stopped {
            service.start::<&str>(&[]).unwrap();
        }
        assert_eq!(result.action, Ok(true));
        assert_eq!(
            service.query_status().unwrap().current_state,
            ServiceState::Running
        );
        let log = LocalEvents::open(&path).unwrap();
        assert_eq!(log.pending_events().len(), 2);
        assert_eq!(log.pending_events()[1].payload["status"], "succeeded");
        let again = r1(
            &path,
            Some(&binding),
            super::super::unix_timestamp(),
            || Ok(Observation::Running),
            || panic!("no repeated start"),
        );
        assert_eq!(again.action, Ok(false));
        let after: super::super::AgentState =
            serde_json::from_slice(&std::fs::read(paths.state.join("agent.json")).unwrap())
                .unwrap();
        let identity_after = super::super::identity::load_existing().unwrap();
        assert_eq!(after.agent_installation_id, state.agent_installation_id);
        assert_eq!(identity_before.id, identity_after.id);
        assert_eq!(identity_before.public, identity_after.public);
        std::fs::write(root.join("native-proof.json"),serde_json::to_vec(&serde_json::json!({"events":log.pending_events(),"state":"running","identity_preserved":true})).unwrap()).unwrap();
    }
}
