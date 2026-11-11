//! A running Command's primary-quota wait budget, shared by its child Runs.
use std::time::Duration;

use chrono::{DateTime, Utc};

const COMMAND_CAP: Duration = Duration::from_secs(2 * 60 * 60);

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct WaitBudget {
    total: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WaitCapExceeded;

impl WaitBudget {
    pub fn next(&mut self, reset: DateTime<Utc>) -> Result<Duration, WaitCapExceeded> {
        self.next_with(reset, Utc::now, || rand::random_range(0..=60))
    }

    /// Reserve this wait in the Command's running total. Rejected waits leave
    /// the total unchanged; the caller never sleeps beyond the two-hour cap.
    pub fn next_with(
        &mut self,
        reset: DateTime<Utc>,
        clock: impl FnOnce() -> DateTime<Utc>,
        jitter_seconds: impl FnOnce() -> u64,
    ) -> Result<Duration, WaitCapExceeded> {
        let jitter = jitter_seconds();
        assert!(jitter <= 60, "Quota wait jitter must be in 0..=60 seconds");
        // The override shortens the safety minute and jitter for binary tests;
        // the reset itself remains a real wall-clock deadline.
        let minute = minute();
        let margin = minute + minute.mul_f64(jitter as f64 / 60.0);
        let until = reset + chrono::Duration::from_std(margin).unwrap();
        let wait = (until - clock()).to_std().unwrap_or(Duration::ZERO);
        let total = self.total.checked_add(wait).ok_or(WaitCapExceeded)?;
        if total > COMMAND_CAP {
            return Err(WaitCapExceeded);
        }
        self.total = total;
        Ok(wait)
    }

    pub(super) fn refund(&mut self, unused: Duration) {
        self.total = self.total.saturating_sub(unused);
    }
}

/// Shorten a quota safety minute for binary tests, as with CI's grace period.
pub(in crate::github) fn minute() -> Duration {
    std::env::var("THIRDSHIFT_QUOTA_MINUTE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(60))
}
