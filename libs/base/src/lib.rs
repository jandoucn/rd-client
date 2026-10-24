pub mod config;

/// Whether a `stop-service` option value means "the user wants the service to keep
/// running". A RustDesk2.toml written by the customised installer carries no such
/// key at all, so a missing key must read as enabled. The management agent's
/// recovery path and its registration reconnect must agree on this; they once
/// disagreed, which left every fresh install permanently refusing to reconnect.
pub fn stop_service_intent_enabled(value: Option<&str>) -> bool {
    matches!(value, None | Some("") | Some("N"))
}
pub mod fs;
pub mod keyboard;
pub mod lab_guard;
pub mod platform;
pub mod protos;
pub mod update;

pub use protos::message as message_proto;
