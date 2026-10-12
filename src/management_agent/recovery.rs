#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Enabled,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    Absent,
    Stopped,
    StartPending,
    StopPending,
    Running,
    Paused,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub enabled: bool,
    pub locally_blocked: bool,
    pub intent: Intent,
    pub maintenance: bool,
    pub startup_grace: bool,
    pub retry_due: bool,
    pub service: ServiceState,
    pub trusted_service_path: bool,
    pub normal_available: bool,
    pub fallback_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Observe,
    Start,
    RepairNormal,
    RepairFallback,
    RecoverPackage,
}

pub fn decide(o: &Observation) -> Action {
    if !o.enabled
        || o.locally_blocked
        || o.intent != Intent::Enabled
        || o.maintenance
        || o.startup_grace
        || !o.retry_due
    {
        return Action::Observe;
    }
    match o.service {
        ServiceState::Stopped
            if o.trusted_service_path && (o.normal_available || o.fallback_available) =>
        {
            Action::Start
        }
        ServiceState::Absent | ServiceState::Stopped => {
            if o.normal_available {
                Action::RepairNormal
            } else if o.fallback_available {
                Action::RepairFallback
            } else {
                Action::RecoverPackage
            }
        }
        _ => Action::Observe,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetryBudget {
    pub failures: u8,
    pub next_at: u64,
    pub healthy_since: Option<u64>,
}

impl RetryBudget {
    pub fn due(&self, now: u64) -> bool {
        now >= self.next_at
    }

    pub fn record_failure(&mut self, now: u64) {
        let delay = match self.failures {
            0 => 30,
            1 => 120,
            2 => 300,
            _ => 3600,
        };
        self.failures = self.failures.saturating_add(1);
        self.next_at = now.saturating_add(delay);
        self.healthy_since = None;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn observe_health(&mut self, healthy: bool, now: u64) {
        if !healthy {
            self.healthy_since = None;
            return;
        }
        let since = self.healthy_since.get_or_insert(now);
        if now.saturating_sub(*since) >= 600 {
            self.reset();
        }
    }
}

pub fn service_path_matches(command: &str, expected: &str) -> bool {
    let command = command.trim();
    let Some(quoted) = command.strip_prefix('"') else {
        return !expected.chars().any(char::is_whitespace)
            && command
                .strip_suffix(" --service")
                .map(|path| path.eq_ignore_ascii_case(expected))
                .unwrap_or(false);
    };
    let Some((path, arguments)) = quoted.split_once('"') else {
        return false;
    };
    path.eq_ignore_ascii_case(expected) && arguments.trim() == "--service"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsiOutcome {
    Installed,
    RebootRequired,
    RebootInitiated,
    Busy,
    Failed(i32),
}

pub fn classify_msi_exit(code: i32) -> MsiOutcome {
    match code {
        0 => MsiOutcome::Installed,
        3010 => MsiOutcome::RebootRequired,
        1641 => MsiOutcome::RebootInitiated,
        1618 => MsiOutcome::Busy,
        code => MsiOutcome::Failed(code),
    }
}
