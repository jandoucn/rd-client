//! 启动代号回退恢复。
//!
//! `agent.json` 被备份还原、重装或回滚后，本地 `status_session_generation` 可能小于后台
//! 记录的代号；每次启动只加 1 永远追不上，状态上报会一直 409。rd-api 在“代号过旧”的 409
//! 里带回当前代号（`code=session_generation_stale`），这里只负责把本地代号前进到它之上。

/// 后台代号的可信上限：留出足够余量，避免被异常响应推到 `i64::MAX` 后启动时溢出退出。
pub const SESSION_GENERATION_CEILING: u64 = (i64::MAX as u64) / 2;

/// 从 409 响应体解析后台当前代号；只认 rd-api 约定的字段，其他 409 返回 `None`。
pub fn stale_generation_from_conflict(body: &[u8]) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    if value.get("code")?.as_str()? != "session_generation_stale" {
        return None;
    }
    let current = value.get("current_session_generation")?.as_u64()?;
    (current > 0 && current < SESSION_GENERATION_CEILING).then_some(current)
}

/// 本地代号要前进到的新值；本地已经大于后台时返回 `None`（只前进不后退）。
pub fn advance(local: u64, server_current: u64) -> Option<u64> {
    if server_current < local || server_current >= SESSION_GENERATION_CEILING {
        return None;
    }
    Some(server_current + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regressed_generation_moves_past_server() {
        assert_eq!(advance(54, 60), Some(61));
    }

    #[test]
    fn equal_generation_moves_past_server() {
        assert_eq!(advance(60, 60), Some(61));
    }

    #[test]
    fn local_ahead_never_moves_backwards() {
        assert_eq!(advance(62, 60), None);
        assert_eq!(advance(61, 60), None);
    }

    #[test]
    fn untrusted_huge_generation_is_ignored() {
        assert_eq!(advance(1, SESSION_GENERATION_CEILING), None);
        assert_eq!(advance(1, u64::MAX), None);
    }

    #[test]
    fn only_the_stale_code_carries_a_generation() {
        let stale = br#"{"error":"x","code":"session_generation_stale","current_session_generation":60}"#;
        assert_eq!(stale_generation_from_conflict(stale), Some(60));
        for body in [
            br#"{"error":"agent sequence expired"}"#.as_slice(),
            br#"{"error":"x","code":"other","current_session_generation":60}"#,
            br#"{"error":"x","code":"session_generation_stale"}"#,
            br#"{"error":"x","code":"session_generation_stale","current_session_generation":-1}"#,
            br#"{"error":"x","code":"session_generation_stale","current_session_generation":0}"#,
            br#"{"error":"x","code":"session_generation_stale","current_session_generation":"60"}"#,
            br#"{"error":"x","code":"session_generation_stale","current_session_generation":9223372036854775807}"#,
            b"not-json",
        ] {
            assert_eq!(stale_generation_from_conflict(body), None, "{}", String::from_utf8_lossy(body));
        }
    }
}
