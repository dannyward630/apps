// SPDX-License-Identifier: Apache-2.0
//! Refresh cadence, exponential backoff, `Retry-After` and the manual refresh interval.
use std::time::Duration;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const LOCAL_SYNC_INTERVAL: Duration = Duration::from_mins(1);
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_mins(1);
pub const EXTERNAL_INTERVAL: Duration = Duration::from_mins(5);
pub const MANUAL_REFRESH_INTERVAL: Duration = Duration::from_mins(1);
const BACKOFF_BASE: Duration = Duration::from_mins(1);
const BACKOFF_CAP: Duration = Duration::from_mins(30);
const RETRY_AFTER_CAP: u64 = 86_400;
/// The wait after the n-th consecutive failure: `min(30 min, 1 min × 2^(n−1))` plus uniform
/// jitter of up to 20% of that value (`jitter` in `[0, 1)`), capped at 30 minutes.
#[must_use]
pub fn backoff(failures: u32, jitter: f64) -> Duration {
    let exponent = failures.saturating_sub(1).min(16);
    let base = BACKOFF_BASE.saturating_mul(1 << exponent).min(BACKOFF_CAP);
    base.saturating_add(base.mul_f64(0.2 * jitter.clamp(0.0, 1.0)))
        .min(BACKOFF_CAP)
}
#[must_use]
pub fn jitter() -> f64 {
    rand::random::<f64>()
}
/// Parse `Retry-After` as delta seconds (capped at one day) or an HTTP date relative to `now`.
#[must_use]
pub fn retry_after(value: &str, now: i64) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds.min(RETRY_AFTER_CAP)));
    }
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Some(Duration::from_secs(RETRY_AFTER_CAP));
    }
    let at = chrono::DateTime::parse_from_rfc2822(value)
        .ok()?
        .timestamp();
    Some(Duration::from_secs(
        u64::try_from(at.saturating_sub(now)).unwrap_or(0),
    ))
}
/// Add a retry wait without wrapping a stored Unix timestamp near its numeric limit.
#[must_use]
pub fn wait_until(now: i64, wait: Duration) -> i64 {
    now.saturating_add(seconds(wait))
}
/// The remaining wait before another manual refresh is allowed, or `None` when allowed.
#[must_use]
pub fn manual_wait(last: i64, now: i64) -> Option<Duration> {
    let interval = i64::try_from(MANUAL_REFRESH_INTERVAL.as_secs()).unwrap_or(60);
    let remaining = last + interval - now;
    (last > 0 && remaining > 0).then(|| Duration::from_secs(u64::try_from(remaining).unwrap_or(0)))
}
#[must_use]
pub fn seconds(d: Duration) -> i64 {
    i64::try_from(d.as_secs()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{retry_after, wait_until};
    use std::time::Duration;

    #[test]
    fn numeric_retry_after_is_bounded_but_short_waits_are_exact() {
        let now = 1_700_000_000;
        for (value, expected) in [
            ("9223372036854775807", 86_400),
            ("184467440737095516160", 86_400),
            ("86400", 86_400),
            ("60", 60),
        ] {
            assert_eq!(retry_after(value, now), Some(Duration::from_secs(expected)));
        }
        assert_eq!(retry_after("", now), None);
    }

    #[test]
    fn http_date_wait_and_timestamp_addition_remain_safe() {
        assert_eq!(
            retry_after("Wed, 21 Oct 2015 07:28:00 GMT", 1_445_412_420),
            Some(Duration::from_secs(60))
        );
        assert_eq!(wait_until(i64::MAX - 1, Duration::from_secs(60)), i64::MAX);
        assert_eq!(wait_until(100, Duration::from_secs(60)), 160);
    }
}
