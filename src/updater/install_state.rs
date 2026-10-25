//! 自动安装的尝试计数与生效确认。
//!
//! 一次拉起的安装器只要进程起得来就被当成成功，是一个装不上的版本可以永久重放的
//! 原因：新进程崩溃、安装器没把自己换掉，都没有任何一层能看见。这里把「已尝试」
//! 持久化，并给出「安装是否真的生效」的判据，供启动时和限定等待窗口共用。

use base::{config::keys, update::PendingUpdateEvent};
use hbb_common::{config, log};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 同一目标版本连续自动尝试的上限。达到后不再自动安装，避免一个一启动就崩溃的
/// 版本被每次检查重新拉起。
pub const MAX_AUTO_INSTALL_ATTEMPTS: u32 = 3;

/// 达到上限后的冷却期。仅重启进程不能清零计数，否则重启即恢复循环；冷却期过后
/// 认为机器状态和可用版本都可能已经变化，允许重新尝试。
pub const AUTO_INSTALL_COOLDOWN_SECS: i64 = 6 * 60 * 60;

/// 安装器启动后等待生效确认的时限。EXE 升级会终止本进程，因此这个窗口只在安装器
/// 没有真正换掉当前进程时才会走完。
pub const INSTALL_CONFIRM_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize,
)]
pub struct AutoInstallState {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub build_seq: u64,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub last_attempt_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize)]
pub struct PendingInstall {
    pub event: PendingUpdateEvent,
    pub deadline_unix: i64,
}

#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde_derive::Deserialize, serde_derive::Serialize,
)]
pub struct InstallState {
    #[serde(default)]
    pub auto_install: AutoInstallState,
    #[serde(default)]
    pub pending: Option<PendingInstall>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallConfirmation {
    /// 当前运行的构建已经达到目标版本，安装生效。
    Confirmed,
    /// 还没到确认时限，也还没看到生效。
    Pending,
    /// 确认时限已过，运行的仍是旧构建，本次安装失败。
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoInstallGate {
    /// 可以再拉起一次安装器，并记一次尝试。
    Attempt,
    /// 已经用尽尝试次数，等冷却期或显式重试。
    Suppressed,
}

/// 自动安装的闸门。用户手动点「立即安装」和后台命令都不受限：它们本身就是一次
/// 明确的请求，只受 `mode` 约束。
pub fn auto_install_gate(
    automatic: bool,
    state: &AutoInstallState,
    version: &str,
    build_seq: u64,
    now: i64,
) -> AutoInstallGate {
    if !automatic || state.allows_attempt(version, build_seq, now) {
        AutoInstallGate::Attempt
    } else {
        AutoInstallGate::Suppressed
    }
}

impl AutoInstallState {
    /// 目标版本换了就不再沿用旧计数；同一目标达到上限后只等冷却期过去。
    pub fn allows_attempt(&self, version: &str, build_seq: u64, now: i64) -> bool {
        if self.version != version || self.build_seq != build_seq {
            return true;
        }
        self.attempts < MAX_AUTO_INSTALL_ATTEMPTS
            || now.saturating_sub(self.last_attempt_at) >= AUTO_INSTALL_COOLDOWN_SECS
    }

    /// 每次拉起安装器都记一次。成功由启动时的生效确认来清零，所以计数只会因为
    /// 目标变化或显式重试而回落。
    pub fn record_attempt(&self, version: &str, build_seq: u64, now: i64) -> Self {
        let attempts = if self.version == version && self.build_seq == build_seq {
            self.attempts.saturating_add(1)
        } else {
            1
        };
        Self {
            version: version.to_owned(),
            build_seq,
            attempts,
            last_attempt_at: now,
        }
    }
}

/// 安装是否生效。判据与 `install_command_satisfied` 相同：运行的构建不低于目标，
/// 就说明这次安装（以及它要求的重启）已经完成。
pub fn install_confirmation(
    target_version: &str,
    target_build_seq: u64,
    running_version: &str,
    running_build_seq: u64,
    now: i64,
    deadline_unix: i64,
) -> InstallConfirmation {
    if !hbb_common::is_newer_version(
        target_version,
        target_build_seq,
        running_version,
        running_build_seq,
    ) {
        InstallConfirmation::Confirmed
    } else if now >= deadline_unix {
        InstallConfirmation::Failed
    } else {
        InstallConfirmation::Pending
    }
}

pub fn load() -> InstallState {
    let value = config::Config::get_option(keys::OPTION_UPDATE_INSTALL_STATE);
    if value.is_empty() {
        return InstallState::default();
    }
    serde_json::from_str(&value).unwrap_or_else(|err| {
        log::warn!("Failed to load update install state: {err}");
        InstallState::default()
    })
}

pub fn store(state: &InstallState) {
    match serde_json::to_string(state) {
        Ok(value) => {
            config::Config::set_option(keys::OPTION_UPDATE_INSTALL_STATE.to_owned(), value)
        }
        Err(err) => log::warn!("Failed to serialize update install state: {err}"),
    }
}

pub fn auto_install_allowed(version: &str, build_seq: u64, now: i64) -> bool {
    load().auto_install.allows_attempt(version, build_seq, now)
}

pub fn record_auto_install_attempt(version: &str, build_seq: u64, now: i64) {
    let mut state = load();
    state.auto_install = state.auto_install.record_attempt(version, build_seq, now);
    store(&state);
}

/// 手动请求或新的安装命令到达时清零，让用户和后台都能重新发起一次安装。
pub fn clear_auto_install_attempts() {
    let mut state = load();
    if state.auto_install == AutoInstallState::default() {
        return;
    }
    state.auto_install = AutoInstallState::default();
    store(&state);
}

pub fn store_pending(event: &PendingUpdateEvent, timeout: Duration, now: i64) {
    let mut state = load();
    state.pending = Some(PendingInstall {
        event: event.clone(),
        deadline_unix: now.saturating_add(timeout.as_secs() as i64),
    });
    store(&state);
}

pub fn clear_pending(transaction_id: &str) {
    let mut state = load();
    let matches = state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.event.transaction_id == transaction_id);
    if !matches {
        return;
    }
    state.pending = None;
    store(&state);
}

static INSTALL_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// 这条命令是不是本机发起、并且已经被安装器接管的那个。
///
/// 安装命令在安装过程中会终止本进程，所以重启后重新执行同一条命令时，「运行的
/// 已经是目标版本」既可能是安装成功（本机有它留下的 pending 记录），也可能是后台
/// 下发了一条与本机签名清单不符的命令（本机什么都没做过）。只有前者可以判成完成，
/// 否则会跳过目标和签名校验，把一条错命令记成安装成功。
pub fn pending_install_owns_command(command_id: &str) -> bool {
    let Some(pending) = load().pending else {
        return false;
    };
    pending.event.command_id.as_deref() == Some(command_id)
}

/// 记下即将交给安装器的这次安装，作为重启后判断「是本机装的」的依据。
pub fn store_pending_install_command(event: &PendingUpdateEvent, now: i64) {
    store_pending(event, INSTALL_CONFIRM_TIMEOUT, now);
}

/// 同一进程内同一时刻只允许一路安装。安装器一旦拉起就会接管，第二路再拉起一个
/// 只会让两套替换脚本互相踩。
pub struct InstallLease(());

impl Drop for InstallLease {
    fn drop(&mut self) {
        INSTALL_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

pub fn try_acquire_install_lease() -> Option<InstallLease> {
    INSTALL_IN_PROGRESS
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| InstallLease(()))
}

#[cfg(test)]
mod tests {
    use super::{
        install_confirmation, try_acquire_install_lease, AutoInstallState, InstallConfirmation,
        InstallState, PendingInstall, AUTO_INSTALL_COOLDOWN_SECS, INSTALL_CONFIRM_TIMEOUT,
        MAX_AUTO_INSTALL_ATTEMPTS,
    };
    use base::update::{PendingUpdateEvent, UpdateSource};
    use std::sync::{Arc, Barrier};

    // 安装租约是进程级状态，两个租约测试在同一次测试运行里并行执行会互相抢锁。
    // 用一个测试内互斥把这两个用例串起来，不为此改动生产代码。
    static LEASE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lease_test_guard() -> std::sync::MutexGuard<'static, ()> {
        LEASE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    const NOW: i64 = 1_800_000_000;

    fn event() -> PendingUpdateEvent {
        PendingUpdateEvent {
            transaction_id: "a".repeat(32),
            from_version: "1.5.4".to_owned(),
            from_build_seq: 2026102404,
            version: "1.5.4".to_owned(),
            build_seq: 2026102405,
            source: UpdateSource::Primary,
            command_id: None,
        }
    }

    fn exhausted() -> AutoInstallState {
        let mut state = AutoInstallState::default();
        for _ in 0..MAX_AUTO_INSTALL_ATTEMPTS {
            state = state.record_attempt("1.5.4", 2026102405, NOW);
        }
        state
    }

    #[test]
    fn auto_install_stops_after_the_attempt_limit() {
        let mut state = AutoInstallState::default();
        assert!(state.allows_attempt("1.5.4", 2026102405, NOW));
        for attempt in 1..=MAX_AUTO_INSTALL_ATTEMPTS {
            state = state.record_attempt("1.5.4", 2026102405, NOW);
            assert_eq!(state.attempts, attempt);
            assert_eq!(state.version, "1.5.4");
            assert_eq!(state.build_seq, 2026102405);
            assert_eq!(state.last_attempt_at, NOW);
        }
        assert!(!state.allows_attempt("1.5.4", 2026102405, NOW));
        assert!(!state.allows_attempt("1.5.4", 2026102405, NOW + 1));
        // 被拦住时记录不被改写，仍保留最后一次失败的目标和次数。
        assert_eq!(state, exhausted());
    }

    #[test]
    fn a_new_target_resets_the_attempt_count() {
        let state = exhausted();
        assert!(!state.allows_attempt("1.5.4", 2026102405, NOW));
        assert!(state.allows_attempt("1.5.5", 2026102406, NOW));
        let restarted = state.record_attempt("1.5.5", 2026102406, NOW);
        assert_eq!(restarted.attempts, 1);
        assert_eq!(restarted.version, "1.5.5");
        assert_eq!(restarted.build_seq, 2026102406);
        // 只有版本变化也算新目标，构建号变化同样算。
        assert!(state.allows_attempt("1.5.4", 2026102406, NOW));
        assert!(state.allows_attempt("1.5.5", 2026102405, NOW));
    }

    #[test]
    fn the_cooldown_lifts_the_limit_without_losing_the_record() {
        let state = exhausted();
        let last = NOW + AUTO_INSTALL_COOLDOWN_SECS;
        assert!(!state.allows_attempt("1.5.4", 2026102405, last - 1));
        assert!(state.allows_attempt("1.5.4", 2026102405, last));
        // 冷却期只是放行，不会把旧计数悄悄清空。
        assert_eq!(state.attempts, MAX_AUTO_INSTALL_ATTEMPTS);
        assert_eq!(state.version, "1.5.4");
    }

    #[test]
    fn clearing_the_record_lets_a_manual_request_try_again() {
        let cleared = AutoInstallState::default();
        assert!(cleared.allows_attempt("1.5.4", 2026102405, NOW));
        let restarted = cleared.record_attempt("1.5.4", 2026102405, NOW);
        assert_eq!(restarted.attempts, 1);
        assert_eq!(
            restarted,
            AutoInstallState {
                version: "1.5.4".to_owned(),
                build_seq: 2026102405,
                attempts: 1,
                last_attempt_at: NOW,
            }
        );
    }

    #[test]
    fn install_state_survives_a_restart_through_json() {
        let state = InstallState {
            auto_install: exhausted(),
            pending: Some(PendingInstall {
                event: event(),
                deadline_unix: NOW + INSTALL_CONFIRM_TIMEOUT.as_secs() as i64,
            }),
        };
        let encoded = serde_json::to_string(&state).expect("state should serialize");
        let decoded: InstallState = serde_json::from_str(&encoded).expect("state should parse");
        assert_eq!(decoded, state);
        // 旧版本写下的空配置也要能读回默认值，否则升级后第一次启动就会丢弃计数。
        assert_eq!(
            serde_json::from_str::<InstallState>("{}").unwrap(),
            InstallState::default()
        );
        assert_eq!(
            serde_json::from_str::<InstallState>(r#"{"auto_install":{"version":"1.5.4"}}"#)
                .unwrap()
                .auto_install
                .attempts,
            0
        );
    }

    #[test]
    fn an_installer_that_did_not_replace_the_build_is_a_failure() {
        let deadline = NOW + INSTALL_CONFIRM_TIMEOUT.as_secs() as i64;
        assert_eq!(
            install_confirmation("1.5.4", 2026102405, "1.5.4", 2026102404, deadline, deadline),
            InstallConfirmation::Failed
        );
        assert_eq!(
            install_confirmation("1.5.5", 1, "1.5.4", 2026102404, deadline + 60, deadline),
            InstallConfirmation::Failed
        );
    }

    #[test]
    fn reaching_the_target_build_confirms_the_install() {
        let deadline = NOW + INSTALL_CONFIRM_TIMEOUT.as_secs() as i64;
        for (version, build) in [("1.5.4", 2026102405), ("1.5.4", 2026102406), ("1.5.5", 1)] {
            assert_eq!(
                install_confirmation("1.5.4", 2026102405, version, build, NOW, deadline),
                InstallConfirmation::Confirmed,
                "{version}+{build}"
            );
        }
    }

    #[test]
    fn confirmation_waits_until_the_deadline() {
        let deadline = NOW + INSTALL_CONFIRM_TIMEOUT.as_secs() as i64;
        assert_eq!(
            install_confirmation("1.5.4", 2026102405, "1.5.4", 2026102404, NOW, deadline),
            InstallConfirmation::Pending
        );
        assert_eq!(
            install_confirmation(
                "1.5.4",
                2026102405,
                "1.5.4",
                2026102404,
                deadline - 1,
                deadline
            ),
            InstallConfirmation::Pending
        );
    }

    #[test]
    fn only_one_install_lease_is_granted_at_a_time() {
        let _serial = lease_test_guard();
        let first = try_acquire_install_lease().expect("first install should start");
        assert!(try_acquire_install_lease().is_none());
        drop(first);
        assert!(try_acquire_install_lease().is_some());
    }

    #[test]
    fn concurrent_installs_leave_exactly_one_winner() {
        let _serial = lease_test_guard();
        const THREADS: usize = 8;
        let barrier = Arc::new(Barrier::new(THREADS));
        let winners = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let barrier = barrier.clone();
                let winners = winners.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    if let Some(lease) = try_acquire_install_lease() {
                        winners.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        drop(lease);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("install racer should finish");
        }
        assert_eq!(winners.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
