//! Wait planning only; later Tickets connect this to running Commands.
use std::time::Duration;

use chrono::{DateTime, Utc};

const COMMAND_CAP: Duration = Duration::from_secs(2 * 60 * 60);

#[derive(Default)]
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
        let until = reset + chrono::Duration::seconds(60 + jitter as i64);
        let wait = (until - clock()).to_std().unwrap_or(Duration::ZERO);
        let total = self.total.checked_add(wait).ok_or(WaitCapExceeded)?;
        if total > COMMAND_CAP {
            return Err(WaitCapExceeded);
        }
        self.total = total;
        Ok(wait)
    }
}
