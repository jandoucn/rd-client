#![cfg(windows)]

#[path = "../src/platform/windows_installer_busy.rs"]
mod installer;

use installer::{classify, Api, Observation, ProbeError, Status, UnknownReason};

fn status(state: u32, controls: u32) -> Status {
    Status {
        service_type: 0x10,
        current_state: state,
        controls_accepted: controls,
        win32_exit_code: 0,
        service_specific_exit_code: 0,
        check_point: 0,
        wait_hint: 0,
        process_id: 4321,
        service_flags: 0,
    }
}

fn check(input: Result<Status, ProbeError>, expected: Observation) {
    let original = input;
    let actual = classify(input);
    assert_eq!(input, original, "classification changed the observation");
    assert_eq!(actual, expected);
    assert_eq!(actual.permits_attempt(), actual == Observation::Idle);
}

#[test]
fn stopped_without_controls_is_observed_idle_pid_is_not_authoritative() {
    check(Ok(status(1, 0)), Observation::Idle);
}

#[test]
fn running_with_shutdown_bit_is_busy_including_combined_flags() {
    for controls in [4, 5, 0x104] {
        check(Ok(status(4, controls)), Observation::Busy);
    }
}

#[test]
fn running_without_shutdown_bit_is_observed_idle_not_mutex_ownership() {
    for controls in [0, 1, 0x100] {
        check(Ok(status(4, controls)), Observation::Idle);
    }
}

#[test]
fn start_and_stop_pending_are_unknown_even_with_shutdown_bit() {
    for state in [2, 3] {
        for controls in [0, 4] {
            let mut value = status(state, controls);
            value.process_id = 0;
            value.check_point = 7;
            value.wait_hint = 30000;
            check(
                Ok(value),
                Observation::Unknown(UnknownReason::UnstableState(state)),
            );
        }
    }
}

#[test]
fn paused_and_unknown_states_fail_closed() {
    for state in [0, 5, 6, 7, u32::MAX] {
        check(
            Ok(status(state, 0)),
            Observation::Unknown(UnknownReason::UnstableState(state)),
        );
    }
}

#[test]
fn stopped_accepting_controls_is_inconsistent_not_idle() {
    for controls in [1, 4] {
        check(
            Ok(status(1, controls)),
            Observation::Unknown(UnknownReason::InconsistentStatus),
        );
    }
}

#[test]
fn wrong_service_type_and_invalid_running_pid_fail_closed() {
    for service_type in [0, 1, 0x110] {
        let mut value = status(4, 0);
        value.service_type = service_type;
        check(
            Ok(value),
            Observation::Unknown(UnknownReason::InconsistentStatus),
        );
    }
    let mut value = status(4, 0);
    value.process_id = 0;
    check(
        Ok(value),
        Observation::Unknown(UnknownReason::InconsistentStatus),
    );
    let mut shared = status(4, 4);
    shared.service_type = 0x20;
    check(Ok(shared), Observation::Busy);
}

#[test]
fn service_absent_1060_is_unknown_not_idle() {
    let error = ProbeError {
        api: Api::OpenService,
        code: 1060,
    };
    check(Err(error), Observation::Unknown(UnknownReason::Api(error)));
}

#[test]
fn access_denied_at_each_query_stage_preserves_error_and_refuses() {
    for api in [Api::OpenManager, Api::OpenService, Api::QueryStatus] {
        let error = ProbeError { api, code: 5 };
        check(Err(error), Observation::Unknown(UnknownReason::Api(error)));
    }
}

#[test]
fn invalid_handle_buffer_shutdown_and_close_errors_are_not_idle() {
    for (api, code) in [
        (Api::QueryStatus, 6),
        (Api::QueryStatus, 122),
        (Api::QueryStatus, 1115),
        (Api::CloseService, 6),
        (Api::CloseManager, 6),
    ] {
        let error = ProbeError { api, code };
        check(Err(error), Observation::Unknown(UnknownReason::Api(error)));
    }
}

#[test]
fn only_idle_allows_an_attempt_and_native_status_layout_has_all_nine_dwords() {
    assert_eq!(std::mem::size_of::<Status>(), 36);
    assert_eq!(std::mem::align_of::<Status>(), 4);
    assert!(Observation::Idle.permits_attempt());
    assert!(!Observation::Busy.permits_attempt());
    assert!(!Observation::Unknown(UnknownReason::InconsistentStatus).permits_attempt());
}

#[test]
#[ignore = "VM-only readonly SCM probe; owner must first confirm Windows Installer is idle"]
fn vm_actual_msiserver_idle_readonly() {
    base::lab_guard::require_lab();
    let result = installer::probe();
    println!("native msiserver observation: {result:?}");
    assert_eq!(
        result,
        Observation::Idle,
        "busy/unknown is not an idle-probe pass"
    );
}
