//! Host 仅编译；通过 TEMP 构建器绑定当前 updater 分流和 runtime identity 解析。
#![cfg(t02_schema_route)]
use hbb_common::ResultType;
use std::{cell::RefCell, path::Path};

const EDITION: &str = "standard";
#[derive(Default)]
struct Observation {
    directory: std::path::PathBuf,
    state: Vec<u8>,
    markers: Vec<String>,
    old_calls: usize,
    ordinary_calls: usize,
    ordinary_error: bool,
}
thread_local! { static OBS: RefCell<Observation> = RefCell::new(Observation::default()); }
#[derive(Debug, PartialEq)]
enum UpdateCommandRunOutcome {
    AwaitingInstallResult,
    Legacy,
}
struct Command {
    command_id: String,
}
struct Target {
    sha256: String,
    primary: String,
}
fn report_update_event(_: &str, _: &str, _: u64, _: &str) {}
fn report_command_update_event(_: Option<&Command>, _: &str, _: &str) {}
fn update_new_version(
    _: bool,
    _: &str,
    _: u64,
    _: &str,
    _: &Path,
    _: Option<&str>,
) -> ResultType<()> {
    OBS.with(|o| o.borrow_mut().old_calls += 1);
    Ok(())
}

include!(concat!(
    env!("OUT_DIR"),
    "/ordinary_msi_route_projection.rs"
));

fn old_state() -> serde_json::Value {
    // 与 legacy_unknown_fields.rs / f79ed6 字段一致，仅选择 MSI target。
    serde_json::json!({
        "schema_version": 1, "edition": "standard", "package_kind": "msi",
        "target_key": "windows-x86_64-msi-standard",
        "manifest_url": "https://download.yan.life/rustdesk/stable/manifest.json",
        "normal_install_path": "C:\\Program Files\\RustDesk",
        "disabled": false, "management_agent_enabled": true,
        "management_agent_policy_revision": 17,
        "legacy_unknown_object": {"keep": [null, true, 7, "opaque"]}
    })
}

fn run(
    state: serde_json::Value,
    markers: &[&str],
    ordinary_error: bool,
) -> (Result<UpdateCommandRunOutcome, String>, usize, usize) {
    let bytes = serde_json::to_vec(&state).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "route-gate-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    workspace::create(&directory).unwrap();
    std::fs::write(directory.join("agent.json"), &bytes).unwrap();
    for marker in markers {
        std::fs::write(directory.join(marker), b"preserve").unwrap();
    }
    OBS.with(|o| {
        *o.borrow_mut() = Observation {
            directory: directory.clone(),
            state: bytes.clone(),
            markers: markers.iter().map(|v| (*v).into()).collect(),
            ordinary_error,
            ..Default::default()
        }
    });
    let result = projected_route().map_err(|e| e.to_string());
    assert_eq!(std::fs::read(directory.join("agent.json")).unwrap(), bytes);
    for marker in markers {
        assert_eq!(std::fs::read(directory.join(marker)).unwrap(), b"preserve");
    }
    std::fs::remove_dir_all(&directory).unwrap();
    OBS.with(|o| {
        let o = o.borrow();
        assert_eq!(o.state, bytes, "路由不得生成 ID 或改写旧 state");
        (result, o.old_calls, o.ordinary_calls)
    })
}

#[test]
fn valid_legacy_missing_id_uses_old_route_without_identity_creation() {
    assert_eq!(
        run(old_state(), &[], false),
        (Ok(UpdateCommandRunOutcome::Legacy), 1, 0)
    );
}

#[test]
fn legacy_default_schema_one_uses_old_route() {
    let mut state = old_state();
    state.as_object_mut().unwrap().remove("schema_version");
    assert_eq!(
        run(state, &[], false),
        (Ok(UpdateCommandRunOutcome::Legacy), 1, 0)
    );
}

#[test]
fn explicit_invalid_identity_refuses_both_routes() {
    for id in [
        serde_json::json!(""),
        serde_json::Value::Null,
        serde_json::json!(7),
        serde_json::json!(false),
        serde_json::json!({}),
        serde_json::json!("bad"),
    ] {
        let mut state = old_state();
        state["agent_installation_id"] = id;
        let (result, old, ordinary) = run(state, &[], false);
        assert!(result.is_err(), "显式坏 ID 必须拒绝");
        assert_eq!((old, ordinary), (0, 0), "资格拒绝后两路径均不得调用");
    }
}

#[test]
fn disabled_state_and_local_markers_refuse_both_routes() {
    for id in [None, Some("a".repeat(32))] {
        let mut original = old_state();
        if let Some(id) = id {
            original["agent_installation_id"] = id.into();
        }
        for (field, value) in [("disabled", true), ("management_agent_enabled", false)] {
            let mut state = original.clone();
            state[field] = value.into();
            let (result, old, ordinary) = run(state, &[], false);
            assert!(result.is_err());
            assert_eq!((old, ordinary), (0, 0));
        }
        for marker in ["disable-agent", "disable-recovery", "maintenance.json"] {
            let (result, old, ordinary) = run(original.clone(), &[marker], false);
            assert!(result.is_err());
            assert_eq!((old, ordinary), (0, 0));
        }
    }
}

#[test]
fn valid_identity_uses_ordinary_only() {
    let mut state = old_state();
    state["agent_installation_id"] = "a".repeat(32).into();
    assert_eq!(
        run(state, &[], false),
        (Ok(UpdateCommandRunOutcome::AwaitingInstallResult), 0, 1)
    );
}

#[test]
fn ordinary_error_never_falls_back_to_old_route() {
    let mut state = old_state();
    state["agent_installation_id"] = "a".repeat(32).into();
    assert_eq!(
        run(state, &[], true),
        (Err("fixture ordinary failed".into()), 0, 1)
    );
}

#[test]
fn malformed_old_fields_refuse_both_routes() {
    for (field, value) in [
        ("schema_version", serde_json::json!(2)),
        ("edition", serde_json::json!(17)),
        ("target_key", serde_json::json!(null)),
        ("manifest_url", serde_json::json!(false)),
        ("disabled", serde_json::json!("false")),
    ] {
        let mut state = old_state();
        state[field] = value;
        let (result, old, ordinary) = run(state, &[], false);
        assert!(result.is_err());
        assert_eq!((old, ordinary), (0, 0));
    }
}

#[test]
fn explicit_null_or_wrong_schema_refuses_both_routes() {
    for schema in [
        serde_json::Value::Null,
        serde_json::json!("1"),
        serde_json::json!(false),
    ] {
        let mut state = old_state();
        state["schema_version"] = schema;
        let (result, old, ordinary) = run(state, &[], false);
        assert_eq!(result, Err("ordinary installation state malformed".into()));
        assert_eq!((old, ordinary), (0, 0));
    }
}
