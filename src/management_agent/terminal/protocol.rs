//! 终端 WS 帧的解析与渲染（与 socket 无关的纯编解码）。
//!
//! broker→agent：`open`/`input`/`resize`/`close`/`ping`；
//! agent→broker：`opened`/`output`/`exited`/`closed`/`error`/`pong`。
//! 字段、base64、seq 规则以 `docs/terminal-protocol.zh.md`（含 §10 定稿）为准。

use super::{
    manager::{validate_open, OpenRequest},
    CloseReason, ShellKind, TerminalError, TerminalSize,
};
use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};

/// broker→agent 的客户端帧。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientFrame {
    Open(OpenRequest),
    Input {
        session_id: String,
        data: Vec<u8>,
    },
    Resize {
        session_id: String,
        size: TerminalSize,
    },
    Close {
        session_id: String,
    },
    Ping,
}

fn session_id_of(object: &serde_json::Map<String, Value>) -> Result<String, TerminalError> {
    let value = object
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or(TerminalError::InvalidArgs("frame_session_id_missing"))?;
    if super::manager::is_session_id(value) {
        Ok(value.to_owned())
    } else {
        Err(TerminalError::InvalidArgs("frame_session_id_invalid"))
    }
}

fn dimension(
    object: &serde_json::Map<String, Value>,
    key: &'static str,
) -> Result<u16, TerminalError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or(TerminalError::InvalidArgs("frame_dimension_invalid"))
}

pub fn parse_client_frame(value: &Value) -> Result<ClientFrame, TerminalError> {
    let object = value
        .as_object()
        .ok_or(TerminalError::InvalidArgs("frame_not_object"))?;
    let frame_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or(TerminalError::InvalidArgs("frame_type_missing"))?;
    match frame_type {
        "open" => {
            let mut args = object.clone();
            args.remove("type");
            Ok(ClientFrame::Open(validate_open(&Value::Object(args))?))
        }
        "input" => {
            let session_id = session_id_of(object)?;
            let data = object
                .get("data")
                .and_then(Value::as_str)
                .ok_or(TerminalError::InvalidArgs("input_data_missing"))?;
            let data = STANDARD
                .decode(data)
                .map_err(|_| TerminalError::InvalidArgs("input_data_not_base64"))?;
            Ok(ClientFrame::Input { session_id, data })
        }
        "resize" => {
            let session_id = session_id_of(object)?;
            let size = TerminalSize::new(dimension(object, "cols")?, dimension(object, "rows")?)?;
            Ok(ClientFrame::Resize { session_id, size })
        }
        "close" => Ok(ClientFrame::Close {
            session_id: session_id_of(object)?,
        }),
        "ping" => Ok(ClientFrame::Ping),
        _ => Err(TerminalError::InvalidArgs("frame_type_unknown")),
    }
}

pub fn opened(
    seq: u64,
    session_id: &str,
    shell: ShellKind,
    size: TerminalSize,
    started_at_ms: u64,
) -> Value {
    json!({
        "type": "opened",
        "seq": seq,
        "session_id": session_id,
        "shell": shell.name(),
        "cols": size.cols,
        "rows": size.rows,
        "started_at": started_at_ms,
    })
}

pub fn output(seq: u64, session_id: &str, data: &[u8]) -> Value {
    json!({
        "type": "output",
        "seq": seq,
        "session_id": session_id,
        "data": STANDARD.encode(data),
        "stream": "stdout",
    })
}

pub fn exited(seq: u64, session_id: &str, exit_code: u32, duration_ms: u64) -> Value {
    json!({
        "type": "exited",
        "seq": seq,
        "session_id": session_id,
        "exit_code": exit_code,
        "duration_ms": duration_ms,
    })
}

pub fn closed(seq: u64, session_id: &str, reason: CloseReason) -> Value {
    json!({
        "type": "closed",
        "seq": seq,
        "session_id": session_id,
        "reason": reason.as_str(),
    })
}

pub fn error(seq: u64, session_id: Option<&str>, err: &TerminalError) -> Value {
    json!({
        "type": "error",
        "seq": seq,
        "session_id": session_id,
        "code": err.code(),
        "message": err.to_string(),
    })
}

pub fn pong() -> Value {
    json!({ "type": "pong", "seq": 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap()
    }

    const SID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn parses_open_frame_with_type() {
        let frame = parse_client_frame(&json(&format!(
            r#"{{"type":"open","session_id":"{SID}","shell":"pwsh","cols":120,"rows":30}}"#
        )))
        .unwrap();
        match frame {
            ClientFrame::Open(req) => {
                assert_eq!(req.session_id, SID);
                assert_eq!(req.shell, ShellKind::Pwsh);
                assert_eq!(req.size, TerminalSize::new(120, 30).unwrap());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn open_frame_still_rejects_unknown_keys_besides_type() {
        let err = parse_client_frame(&json(&format!(
            r#"{{"type":"open","session_id":"{SID}","shell":"cmd","cols":80,"rows":24,"env":"X=1"}}"#
        )))
        .unwrap_err();
        assert_eq!(err, TerminalError::InvalidArgs("open_unknown_key"));
    }

    #[test]
    fn parses_input_base64_to_raw_bytes() {
        let data = STANDARD.encode([0xe4, 0xb8, 0xad, 0x0d]); // 中\r
        let frame = parse_client_frame(&json(&format!(
            r#"{{"type":"input","session_id":"{SID}","data":"{data}"}}"#
        )))
        .unwrap();
        assert_eq!(
            frame,
            ClientFrame::Input {
                session_id: SID.to_owned(),
                data: vec![0xe4, 0xb8, 0xad, 0x0d],
            }
        );
    }

    #[test]
    fn rejects_bad_input_base64_and_missing_fields() {
        assert!(parse_client_frame(&json(&format!(
            r#"{{"type":"input","session_id":"{SID}","data":"!!!notb64"}}"#
        )))
        .is_err());
        assert!(parse_client_frame(&json(r#"{"type":"input","data":"AA=="}"#)).is_err());
        assert!(parse_client_frame(&json(r#"{"session_id":"x"}"#)).is_err());
        assert!(parse_client_frame(&json(r#"{"type":"bogus"}"#)).is_err());
        assert!(parse_client_frame(&json("42")).is_err());
    }

    #[test]
    fn parses_resize_close_ping() {
        assert_eq!(
            parse_client_frame(&json(&format!(
                r#"{{"type":"resize","session_id":"{SID}","cols":100,"rows":40}}"#
            )))
            .unwrap(),
            ClientFrame::Resize {
                session_id: SID.to_owned(),
                size: TerminalSize::new(100, 40).unwrap(),
            }
        );
        assert_eq!(
            parse_client_frame(&json(&format!(
                r#"{{"type":"close","session_id":"{SID}"}}"#
            )))
            .unwrap(),
            ClientFrame::Close {
                session_id: SID.to_owned()
            }
        );
        assert_eq!(
            parse_client_frame(&json(r#"{"type":"ping"}"#)).unwrap(),
            ClientFrame::Ping
        );
    }

    #[test]
    fn resize_rejects_bad_session_and_dims() {
        assert!(parse_client_frame(&json(
            r#"{"type":"resize","session_id":"SHORT","cols":80,"rows":24}"#
        ))
        .is_err());
        assert!(parse_client_frame(&json(&format!(
            r#"{{"type":"resize","session_id":"{SID}","cols":0,"rows":24}}"#
        )))
        .is_err());
    }

    #[test]
    fn renders_server_frames_with_contract_shapes() {
        let size = TerminalSize::new(80, 24).unwrap();
        let o = opened(1, SID, ShellKind::PowerShell, size, 1700000000000);
        assert_eq!(o["type"], "opened");
        assert_eq!(o["seq"], 1);
        assert_eq!(o["shell"], "powershell");
        assert_eq!(o["cols"], 80);
        assert_eq!(o["started_at"], 1700000000000u64);

        let out = output(2, SID, &[0xe4, 0xb8, 0xad]);
        assert_eq!(out["type"], "output");
        assert_eq!(out["seq"], 2);
        assert_eq!(out["stream"], "stdout");
        assert_eq!(out["data"], STANDARD.encode([0xe4, 0xb8, 0xad]));

        let e = exited(9, SID, 3, 1234);
        assert_eq!(e["exit_code"], 3);
        assert_eq!(e["duration_ms"], 1234);

        let c = closed(0, SID, CloseReason::Idle);
        assert_eq!(c["reason"], "idle");
        assert_eq!(c["seq"], 0);

        let err = error(0, Some(SID), &TerminalError::SessionLimit);
        assert_eq!(err["code"], "terminal_session_limit");
        assert_eq!(err["session_id"], SID);
        let err_no_session = error(
            0,
            None,
            &TerminalError::LocalGate("local_gate_disable_agent"),
        );
        assert_eq!(err_no_session["session_id"], Value::Null);
        assert_eq!(err_no_session["code"], "terminal_local_gate");

        assert_eq!(pong(), json!({"type":"pong","seq":0}));
    }
}
