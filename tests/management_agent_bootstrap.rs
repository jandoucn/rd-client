#[path = "../src/management_agent/bootstrap_policy.rs"]
mod bootstrap;
#[path = "../src/management_agent/recovery.rs"]
#[allow(dead_code)]
mod recovery;

fn observation() -> bootstrap::BootstrapObservation {
    bootstrap::BootstrapObservation {
        agent: recovery::ServiceState::Stopped,
        trusted_agent: true,
        disabled_start: false,
        agent_locally_disabled: false,
        maintenance: false,
        retry_due: true,
    }
}

#[test]
fn dtf_both_services_stopped_requires_agent_start_then_main_recovery() {
    let o = observation();
    assert_eq!(
        bootstrap::decide(&o),
        bootstrap::BootstrapAction::StartAgent
    );
    let main = recovery::Observation {
        enabled: true,
        locally_blocked: false,
        intent: recovery::Intent::Enabled,
        maintenance: false,
        startup_grace: false,
        retry_due: true,
        service: recovery::ServiceState::Stopped,
        trusted_service_path: true,
        normal_available: true,
        fallback_available: false,
    };
    assert_eq!(recovery::decide(&main), recovery::Action::Start);
}

#[test]
fn restored_files_are_not_a_running_service() {
    assert_eq!(
        bootstrap::decide(&observation()),
        bootstrap::BootstrapAction::StartAgent
    );
    let mut o = observation();
    o.agent = recovery::ServiceState::Running;
    assert_eq!(bootstrap::decide(&o), bootstrap::BootstrapAction::Observe);
}

#[test]
fn every_agent_state_and_bootstrap_gate_combination_preserves_disabled_paths() {
    let mut cases = 0;
    for state in [
        recovery::ServiceState::Absent,
        recovery::ServiceState::Stopped,
        recovery::ServiceState::StartPending,
        recovery::ServiceState::StopPending,
        recovery::ServiceState::Running,
        recovery::ServiceState::Paused,
        recovery::ServiceState::Unknown,
    ] {
        for mask in 0u8..32 {
            let flag = |bit: u32| mask & (1u8 << bit) != 0u8;
            let o = bootstrap::BootstrapObservation {
                agent: state,
                trusted_agent: flag(0),
                disabled_start: flag(1),
                agent_locally_disabled: flag(2),
                maintenance: flag(3),
                retry_due: flag(4),
            };
            let expected_start = state == recovery::ServiceState::Stopped
                && o.trusted_agent
                && !o.disabled_start
                && !o.agent_locally_disabled
                && !o.maintenance
                && o.retry_due;
            assert_eq!(
                bootstrap::decide(&o) == bootstrap::BootstrapAction::StartAgent,
                expected_start,
                "state={state:?}, mask={mask}"
            );
            cases += 1;
        }
    }
    assert_eq!(cases, 224);
}

#[test]
fn quarantine_pending_maintenance_and_disable_prevent_bootstrap_start() {
    for index in 0..5 {
        let mut o = observation();
        match index {
            0 => o.trusted_agent = false,
            1 => o.disabled_start = true,
            2 => o.agent_locally_disabled = true,
            3 => o.maintenance = true,
            _ => o.retry_due = false,
        }
        assert_eq!(bootstrap::decide(&o), bootstrap::BootstrapAction::Observe);
    }
    for state in [
        recovery::ServiceState::Absent,
        recovery::ServiceState::Unknown,
        recovery::ServiceState::StartPending,
        recovery::ServiceState::StopPending,
        recovery::ServiceState::Paused,
    ] {
        let mut o = observation();
        o.agent = state;
        assert_eq!(bootstrap::decide(&o), bootstrap::BootstrapAction::Observe);
    }
}

#[test]
fn local_main_stop_does_not_require_bootstrap_to_stop_agent() {
    assert_eq!(
        bootstrap::decide(&observation()),
        bootstrap::BootstrapAction::StartAgent
    );
    let main = recovery::Observation {
        enabled: true,
        locally_blocked: false,
        intent: recovery::Intent::Disabled,
        maintenance: false,
        startup_grace: false,
        retry_due: true,
        service: recovery::ServiceState::Stopped,
        trusted_service_path: true,
        normal_available: true,
        fallback_available: false,
    };
    assert_eq!(recovery::decide(&main), recovery::Action::Observe);
}
