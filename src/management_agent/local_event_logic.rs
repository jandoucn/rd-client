use super::local_events::{Binding, EventError, LocalEvents, Outcome, PathKey, Phase, Record};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Running,
    Stopped,
    Unknown,
}

pub struct RecoveryResult {
    pub action: Result<bool, String>,
    pub reporting: &'static str,
}

pub fn format_recovery_error(error: &str, reporting: &str) -> String {
    const MARKERS: [&str; 3] = [
        "local_events_unreported",
        "local_events_degraded_capacity",
        "local_events_degraded",
    ];
    let marker = match reporting {
        "unreported" => MARKERS[0],
        "degraded_capacity" => MARKERS[1],
        "degraded" => MARKERS[2],
        _ => return error.to_owned(),
    };
    let mut original = error;
    // Replace only trailing status markers; retain the existing diagnostic verbatim.
    loop {
        if MARKERS.contains(&original) {
            original = "";
            break;
        }
        match original.rsplit_once("; ") {
            Some((prefix, suffix)) if MARKERS.contains(&suffix) => original = prefix,
            _ => break,
        }
    }
    if original.is_empty() {
        marker.to_owned()
    } else {
        format!("{original}; {marker}")
    }
}

pub fn r1(
    path: &Path,
    binding: Option<&Binding>,
    now: u64,
    mut observe: impl FnMut() -> Result<Observation, String>,
    start: impl FnOnce() -> Result<bool, String>,
) -> RecoveryResult {
    let mut run = || -> Result<RecoveryResult, String> {
        let mut log = LocalEvents::open(path).map_err(event_error)?;
        let active = log.active_r1();
        let Some(binding) = active.as_ref().map(|(_, binding)| binding).or(binding) else {
            return Ok(RecoveryResult {
                action: start(),
                reporting: "unreported",
            });
        };
        log.prune_confirmed(now).map_err(event_error)?;
        let state = observe()?;
        if state == Observation::Unknown {
            return Err("local R1 requires SCM reconciliation".into());
        }
        if active.is_none() && state == Observation::Running {
            return Ok(RecoveryResult {
                action: Ok(false),
                reporting: "idle",
            });
        }
        let operation = match log.begin_r1(binding, now) {
            Ok(operation) => operation,
            Err(EventError::Full) if active.is_none() => {
                return Ok(RecoveryResult {
                    action: start(),
                    reporting: "degraded_capacity",
                })
            }
            Err(error) => return Err(event_error(error)),
        };
        let action = if state == Observation::Running {
            Ok(false)
        } else {
            start()
        };
        // A refused/uncertain start retains Started; only observed Running is success.
        if state == Observation::Running || matches!(action, Ok(true)) {
            log.record_bound_at(
                &operation,
                Record {
                    phase: Phase::Terminal,
                    attempt_seq: 1,
                    outcome: Outcome::Succeeded,
                    path_key: PathKey::RustDeskService,
                    error_code: None,
                },
                binding,
                now,
            )
            .map_err(event_error)?;
        }
        Ok(RecoveryResult {
            action,
            reporting: "pending",
        })
    };
    run().unwrap_or_else(|error| RecoveryResult {
        action: Err(error),
        reporting: "degraded",
    })
}

pub fn enrolled(
    path: &Path,
    binding: Binding,
    now: u64,
    send: impl FnMut(&serde_json::Value) -> Result<serde_json::Value, String>,
) -> Result<(), String> {
    let mut log = LocalEvents::open(path).map_err(event_error)?;
    log.remember_binding(binding.clone()).map_err(event_error)?;
    log.prune_confirmed(now).map_err(event_error)?;
    upload(&mut log, &binding, send)
}

pub fn upload(
    log: &mut LocalEvents,
    binding: &Binding,
    mut send: impl FnMut(&serde_json::Value) -> Result<serde_json::Value, String>,
) -> Result<(), String> {
    let mut mismatched = false;
    let mut sent = 0;
    for event in log.pending_events().to_vec() {
        if event.payload["client_id"] != binding.client_id
            || event.payload["client_uuid"] != binding.client_uuid
            || event.payload["agent_installation_id"] != binding.agent_installation_id
            || event.payload["enrollment_generation"] != binding.enrollment_generation
        {
            mismatched = true;
            continue;
        }
        if sent >= 8 {
            continue;
        }
        let ack = send(&event.payload)?;
        if ack["ok"] != true || ack["event_id"] != event.event_id {
            return Err("local event acknowledgement mismatch".into());
        }
        log.acknowledge(&event.event_id, &event.digest)
            .map_err(event_error)?;
        sent += 1;
    }
    if mismatched {
        return Err("local event original binding retained".into());
    }
    Ok(())
}

pub(super) fn event_error(error: EventError) -> String {
    format!("local event {error:?}")
}

pub fn send_signed(
    identity: &super::identity::DeviceIdentity,
    base: &str,
    payload: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use hbb_common::{
        base64::{engine::general_purpose::STANDARD, Engine as _},
        sodiumoxide::{crypto::sign, randombytes},
    };
    use std::{
        io::Read,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    const ROUTE: &str = "/rd/update/v1/agent/events";
    if payload["client_id"] != identity.id
        || payload["client_uuid"] != identity.uuid
        || !payload["command_id"].is_null()
    {
        return Err("local event signing identity mismatch".into());
    }
    #[cfg(not(test))]
    if base != "https://rdapi.yan.life" {
        return Err("local event fixed API required".into());
    }
    let body = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
    if body.len() > super::local_events::MAX_RECORD_BYTES {
        return Err("local event wire oversized".into());
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64;
    let nonce = hex::encode(randombytes::randombytes(16));
    let canonical = super::operations::canonical_request(
        "POST",
        ROUTE,
        (&identity.id, &identity.uuid),
        timestamp,
        &nonce,
        "",
        &body,
    );
    let response = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{base}{ROUTE}"))
        .header("Content-Type", "application/json")
        .header("X-RustDesk-Device-ID", &identity.id)
        .header(
            "X-RustDesk-Device-Public-Key",
            STANDARD.encode(identity.public.as_ref()),
        )
        .header("X-RustDesk-Device-Timestamp", timestamp.to_string())
        .header("X-RustDesk-Device-Nonce", nonce)
        .header(
            "X-RustDesk-Device-Signature",
            STANDARD.encode(sign::sign_detached(canonical.as_bytes(), &identity.secret).to_bytes()),
        )
        .body(body)
        .send()
        .map_err(|_| "local event transport failed".to_owned())?;
    if !response.status().is_success() {
        return Err(format!("local event HTTP {}", response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    response
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "local event response unavailable")?;
    if bytes.len() > 65536 {
        return Err("local event response oversized".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "local event response malformed".into())
}
