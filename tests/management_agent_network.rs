#[path = "../src/management_agent/network_policy.rs"]
mod network_policy;

use network_policy::{retry_after, Failure, NetworkBudget};
use std::time::{Duration, Instant};

#[test]
fn network_failures_are_independent_monotonic_and_bounded() {
    let now = Instant::now();
    let mut budget = NetworkBudget::default();
    assert!(budget.due(now));
    budget.failed(Failure::Transport, None, now);
    assert!(!budget.due(now + Duration::from_secs(29)));
    assert!(budget.due(now + Duration::from_secs(30)));
    budget.failed(Failure::Http(401), None, now + Duration::from_secs(30));
    assert!(!budget.due(now + Duration::from_secs(329)));
    assert!(budget.due(now + Duration::from_secs(330)));
    budget.failed(
        Failure::Http(429),
        Some(3600),
        now + Duration::from_secs(330),
    );
    assert!(!budget.due(now + Duration::from_secs(3929)));
    assert!(budget.due(now + Duration::from_secs(3930)));
    budget.succeeded();
    assert!(budget.due(now));
    for _ in 0..255 {
        budget.failed(Failure::Transport, None, now);
    }
    assert!(budget.due(now + Duration::from_secs(3600)));
    budget.failed(Failure::Local, None, now);
    assert!(budget.due(now + Duration::from_secs(30)));
}

#[test]
fn retry_after_seconds_and_http_date_are_clamped_and_invalid_values_are_not_trusted() {
    assert_eq!(retry_after(Some("120"), 1_000), Some(120));
    assert_eq!(retry_after(Some("999999999999"), 1_000), Some(3600));
    assert_eq!(retry_after(Some("0"), 1_000), Some(1));
    assert_eq!(
        retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT"), 1_445_412_420),
        Some(60)
    );
    assert_eq!(
        retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT"), 1_445_412_600),
        Some(1)
    );
    for value in [
        None,
        Some("-1"),
        Some("1.5"),
        Some("payload"),
        Some("9999999999999999999999999999999"),
        Some("Wed, 99 Oct 2015 07:28:00 GMT"),
    ] {
        assert_eq!(retry_after(value, 1_000), None);
    }
}
