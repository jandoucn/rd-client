#[path = "../src/management_agent/recovery.rs"]
mod recovery;

use recovery::{decide, Action, Intent, Observation, ServiceState};

fn observation() -> Observation {
    Observation {
        enabled: true,
        locally_blocked: false,
        intent: Intent::Enabled,
        maintenance: false,
        startup_grace: false,
        retry_due: true,
        service: ServiceState::Stopped,
        trusted_service_path: true,
        normal_available: true,
        fallback_available: false,
    }
}

#[test]
fn stopped_service_is_started_even_when_executable_exists() {
    assert_eq!(decide(&observation()), Action::Start);
}

#[test]
fn every_recovery_gate_and_service_file_combination_is_safe() {
    let mut cases = 0;
    for intent in [Intent::Enabled, Intent::Disabled, Intent::Unknown] {
        for service in [
            ServiceState::Absent,
            ServiceState::Stopped,
            ServiceState::StartPending,
            ServiceState::StopPending,
            ServiceState::Running,
            ServiceState::Paused,
            ServiceState::Unknown,
        ] {
            for mask in 0u16..256 {
                let flag = |bit: u32| mask & (1u16 << bit) != 0u16;
                let o = Observation {
                    enabled: flag(0),
                    locally_blocked: flag(1),
                    maintenance: flag(2),
                    startup_grace: flag(3),
                    retry_due: flag(4),
                    trusted_service_path: flag(5),
                    normal_available: flag(6),
                    fallback_available: flag(7),
                    intent,
                    service,
                };
                let action = decide(&o);
                let permitted = o.enabled
                    && !o.locally_blocked
                    && !o.maintenance
                    && !o.startup_grace
                    && o.retry_due
                    && intent == Intent::Enabled;
                if !permitted || !matches!(service, ServiceState::Stopped | ServiceState::Absent) {
                    assert_eq!(action, Action::Observe, "{o:?}");
                } else if service == ServiceState::Stopped
                    && o.trusted_service_path
                    && (o.normal_available || o.fallback_available)
                {
                    assert_eq!(action, Action::Start, "{o:?}");
                } else {
                    match action {
                        Action::RepairNormal => assert!(o.normal_available, "{o:?}"),
                        Action::RepairFallback => {
                            assert!(!o.normal_available && o.fallback_available, "{o:?}")
                        }
                        Action::RecoverPackage => {
                            assert!(!o.normal_available && !o.fallback_available, "{o:?}")
                        }
                        _ => panic!("unsafe repair decision {action:?}: {o:?}"),
                    }
                }
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 5376);
}

#[test]
fn explicit_stop_unknown_intent_and_local_block_prevent_changes() {
    for intent in [Intent::Disabled, Intent::Unknown] {
        let mut o = observation();
        o.intent = intent;
        assert_eq!(decide(&o), Action::Observe);
    }
    let mut o = observation();
    o.locally_blocked = true;
    assert_eq!(decide(&o), Action::Observe);
    o.locally_blocked = false;
    o.enabled = false;
    assert_eq!(decide(&o), Action::Observe);
}

#[test]
fn maintenance_grace_and_retry_window_never_trigger_repair() {
    for field in 0..3 {
        let mut o = observation();
        o.service = ServiceState::Absent;
        match field {
            0 => o.maintenance = true,
            1 => o.startup_grace = true,
            _ => o.retry_due = false,
        }
        assert_eq!(decide(&o), Action::Observe);
    }
}

#[test]
fn pending_running_paused_and_unknown_states_are_not_restarted() {
    for service in [
        ServiceState::Running,
        ServiceState::StartPending,
        ServiceState::StopPending,
        ServiceState::Paused,
        ServiceState::Unknown,
    ] {
        let mut o = observation();
        o.service = service;
        assert_eq!(decide(&o), Action::Observe);
    }
}

#[test]
fn service_repair_uses_normal_then_fallback_and_never_downloads_over_live_service() {
    let mut o = observation();
    o.service = ServiceState::Absent;
    assert_eq!(decide(&o), Action::RepairNormal);
    o.normal_available = false;
    o.fallback_available = true;
    assert_eq!(decide(&o), Action::RepairFallback);
    o.fallback_available = false;
    assert_eq!(decide(&o), Action::RecoverPackage);
    o.service = ServiceState::Running;
    assert_eq!(decide(&o), Action::Observe);
}

#[test]
fn stopped_service_with_wrong_path_is_repaired_not_blindly_started() {
    let mut o = observation();
    o.trusted_service_path = false;
    assert_eq!(decide(&o), Action::RepairNormal);
    o.normal_available = false;
    assert_eq!(decide(&o), Action::RecoverPackage);
}

#[test]
fn fallback_actions_are_selected_for_each_unexpected_main_failure() {
    let mut observation = observation();
    observation.service = ServiceState::Absent;
    observation.trusted_service_path = false;
    assert_eq!(decide(&observation), Action::RepairNormal);
    observation.normal_available = false;
    observation.fallback_available = true;
    assert_eq!(decide(&observation), Action::RepairFallback);
    observation.fallback_available = false;
    assert_eq!(decide(&observation), Action::RecoverPackage);
}

#[test]
fn matching_path_without_available_file_cannot_be_started() {
    let mut o = observation();
    o.normal_available = false;
    assert_eq!(decide(&o), Action::RecoverPackage);
}

#[test]
fn restart_does_not_reset_backoff_and_clock_rollback_is_conservative() {
    let mut budget = recovery::RetryBudget::default();
    budget.record_failure(1000);
    assert!(!budget.due(1029));
    assert!(budget.due(1030));
    budget.record_failure(1030);
    assert!(!budget.due(1149));
    assert!(budget.due(1150));
    budget.record_failure(1150);
    assert!(!budget.due(1449));
    assert!(budget.due(1450));
    budget.record_failure(1450);
    assert!(!budget.due(5049));
    assert!(budget.due(5050));
    assert!(!budget.due(900));
    budget.reset();
    assert!(budget.due(0));
}

#[test]
fn retry_budget_resets_only_after_continuous_healthy_window() {
    let mut budget = recovery::RetryBudget::default();
    budget.record_failure(1000);
    budget.observe_health(true, 1050);
    budget.observe_health(true, 1649);
    assert_eq!(budget.failures, 1);
    budget.observe_health(false, 1650);
    budget.observe_health(true, 1700);
    budget.observe_health(true, 2200);
    assert_eq!(budget.failures, 1);
    budget.observe_health(true, 2300);
    assert_eq!(budget.failures, 0);
    assert!(budget.due(2300));
}

#[test]
fn service_command_line_requires_exact_executable_and_fixed_service_argument() {
    let expected = r"C:\Program Files\RustDesk\RustDesk.exe";
    assert!(recovery::service_path_matches(
        r#""C:\Program Files\RustDesk\RustDesk.exe" --service"#,
        expected
    ));
    assert!(recovery::service_path_matches(
        r#""c:\program files\rustdesk\rustdesk.exe" --service"#,
        expected
    ));
    assert!(recovery::service_path_matches(
        r"C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe --service",
        r"C:\ProgramData\RustDesk\Client\standard\RustDesk\RustDesk.exe"
    ));
    for invalid in [
        r"C:\Program Files\RustDesk\RustDesk.exe --service",
        r#""C:\Program Files\RustDesk\RustDesk.exe" --service --import-config x"#,
        r#""C:\Program Files\RustDesk\RustDesk.exe.evil" --service"#,
        r#""C:\elsewhere\RustDesk.exe" --service"#,
        r#""C:\Program Files\RustDesk\RustDesk.exe""#,
    ] {
        assert!(
            !recovery::service_path_matches(invalid, expected),
            "{invalid}"
        );
    }
}

#[test]
fn package_exit_codes_distinguish_success_reboot_and_busy() {
    assert_eq!(
        recovery::classify_msi_exit(0),
        recovery::MsiOutcome::Installed
    );
    assert_eq!(
        recovery::classify_msi_exit(3010),
        recovery::MsiOutcome::RebootRequired
    );
    assert_eq!(
        recovery::classify_msi_exit(1641),
        recovery::MsiOutcome::RebootInitiated
    );
    assert_eq!(
        recovery::classify_msi_exit(1618),
        recovery::MsiOutcome::Busy
    );
    for code in [1603, 1921, -1, 1] {
        assert_eq!(
            recovery::classify_msi_exit(code),
            recovery::MsiOutcome::Failed(code)
        );
    }
}
