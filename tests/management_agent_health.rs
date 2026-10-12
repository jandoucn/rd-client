#[path = "../src/management_agent/health.rs"]
mod health;

use health::{Request, Response};

#[test]
fn local_health_binds_nonce_process_build_and_reports_network_separately() {
    let request = Request::new("a".repeat(64)).unwrap();
    let response =
        Response::new(&request, 123, 1000, "1.5.3".into(), 2026100502, true, false).unwrap();
    response.verify(&request, 123, 1000, 2026100502).unwrap();
    assert!(response.local_responsive);
    assert!(!response.registered);
    let before = response.clone();
    for index in 0..6 {
        let mut changed = response.clone();
        match index {
            0 => changed.nonce = "b".repeat(64),
            1 => changed.pid += 1,
            2 => changed.process_created_at += 1,
            3 => changed.build_seq -= 1,
            4 => changed.schema = 2,
            _ => changed.local_responsive = false,
        };
        assert!(
            changed.verify(&request, 123, 1000, 2026100502).is_err(),
            "index={index}"
        );
        assert_eq!(response, before);
    }
}

#[test]
fn malformed_oversized_and_unknown_requests_do_not_execute_actions() {
    assert!(Request::new("bad".into()).is_err());
    let bytes = serde_json::to_vec(&Request::new("a".repeat(64)).unwrap()).unwrap();
    Request::decode(&bytes).unwrap();
    assert!(Request::decode(br#"{"schema":1,"nonce":"bad","command":"stop"}"#).is_err());
    assert!(Request::decode(&vec![b' '; 4097]).is_err());
    assert!(Response::decode(br#"{}"#).is_err());
    assert!(Response::decode(&vec![b' '; 4097]).is_err());
}

#[test]
fn activity_status_is_explicit_and_legacy_health_never_implies_idle() {
    let request = Request::new("a".repeat(64)).unwrap();
    let response = Response::new(&request, 123, 1000, "1.5.3".into(), 2026100502, true, false).unwrap();
    let legacy = serde_json::to_value(&response).unwrap();
    assert!(legacy.get("no_active_sessions").is_none());
    for idle in [false, true] {
        let mut wire = legacy.clone();
        wire["no_active_sessions"] = idle.into();
        let decoded = Response::decode(&serde_json::to_vec(&wire).unwrap()).unwrap();
        decoded.verify(&request, 123, 1000, 2026100502).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap()["no_active_sessions"], idle);
        assert_eq!(legacy, serde_json::to_value(&response).unwrap());
    }
    let mut invalid = legacy;
    invalid["no_active_sessions"] = "true".into();
    assert_eq!(Response::decode(&serde_json::to_vec(&invalid).unwrap()).unwrap_err(), "health response malformed");
}
