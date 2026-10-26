use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub enum Failure {
    Transport,
    Http(u16),
    Local,
}

#[derive(Default)]
pub struct NetworkBudget {
    failures: u8,
    next_at: Option<Instant>,
}

impl NetworkBudget {
    pub fn due(&self, now: Instant) -> bool {
        self.next_at.map(|next| now >= next).unwrap_or(true)
    }

    pub fn succeeded(&mut self) {
        *self = Self::default();
    }

    pub fn failed(&mut self, failure: Failure, retry_after: Option<u64>, now: Instant) {
        let backoff = match self.failures {
            0 => 30,
            1 => 120,
            2 => 300,
            3 => 900,
            _ => 3600,
        };
        let delay = match failure {
            // 403 means the agent waits for an administrator to adopt it or enable
            // remote management, and 401 that the client has not registered its key
            // yet; a growing backoff delayed recovery after either by up to an hour.
            Failure::Http(403) => 60,
            Failure::Http(401) => 300,
            Failure::Local => 30,
            _ => backoff,
        };
        let delay = delay.max(retry_after.unwrap_or(0).min(3600));
        if !matches!(failure, Failure::Http(403)) {
            self.failures = self.failures.saturating_add(1);
        }
        self.next_at = now.checked_add(Duration::from_secs(delay));
    }
}

pub fn retry_after(value: Option<&str>, now: u64) -> Option<u64> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let seconds = if value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse::<u64>().ok()?
    } else {
        let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
        if date.offset().local_minus_utc() != 0 {
            return None;
        }
        let timestamp = u64::try_from(date.timestamp()).ok()?;
        timestamp.saturating_sub(now)
    };
    Some(seconds.clamp(1, 3600))
}
