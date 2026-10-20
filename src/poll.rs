//! Shared grace periods and interruptible pauses for GitHub observations.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::interrupt;

// Each process runs one Command; child Runs load their own User config.
static WAITING_LINE_INTERVAL: OnceLock<Duration> = OnceLock::new();

/// Capture the Command's waiting interval before any work starts.
/// `THIRDSHIFT_WAITING_MINUTE_MS` shortens a minute for binary tests,
/// preserving the User config's choice of how many minutes to wait.
pub fn configure_waiting_lines(interval: Duration) {
    let interval = millis_from_env("THIRDSHIFT_WAITING_MINUTE_MS")
        .and_then(|minute| minute.checked_mul(u32::try_from(interval.as_secs()).ok()?))
        .map(|interval| interval / 60)
        .unwrap_or(interval);
    let _ = WAITING_LINE_INTERVAL.set(interval);
}

/// Report a waiting state immediately, whenever it changes, and again
/// after the configured interval. Each emitted line starts a fresh interval.
pub struct WaitingLine<T> {
    reported: Option<T>,
    last_line: Instant,
    interval: Duration,
}

impl<T: PartialEq> WaitingLine<T> {
    pub fn new() -> Self {
        Self {
            reported: None,
            last_line: Instant::now(),
            interval: WAITING_LINE_INTERVAL
                .get()
                .copied()
                .unwrap_or(Duration::from_secs(5 * 60)),
        }
    }

    pub fn should_report(&mut self, state: T) -> bool {
        if self.reported.as_ref() != Some(&state) || self.last_line.elapsed() >= self.interval {
            self.reported = Some(state);
            self.last_line = Instant::now();
            true
        } else {
            false
        }
    }
}

/// How long to wait for something GitHub should soon report: CI appearing
/// after a push, or whether a PR is mergeable. `THIRDSHIFT_CI_GRACE_MS`
/// overrides it, for tests.
pub fn grace_period() -> Duration {
    millis_from_env("THIRDSHIFT_CI_GRACE_MS").unwrap_or(Duration::from_secs(60))
}

/// Wait one poll interval. Fails with `interrupted` as soon as the Run is
/// interrupted.
pub fn pause() -> Result<()> {
    sleep(poll_interval())
}

/// How long to wait between two asks. `THIRDSHIFT_POLL_MS` overrides it, for
/// tests.
fn poll_interval() -> Duration {
    millis_from_env("THIRDSHIFT_POLL_MS").unwrap_or(Duration::from_secs(10))
}

fn millis_from_env(name: &str) -> Option<Duration> {
    let millis = std::env::var(name).ok()?.parse().ok()?;
    Some(Duration::from_millis(millis))
}

/// Sleep for `duration`, failing with `interrupted` as soon as the Run is
/// interrupted.
pub(crate) fn sleep(duration: Duration) -> Result<()> {
    let deadline = Instant::now() + duration;
    loop {
        if interrupt::requested() {
            bail!("interrupted");
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        std::thread::sleep(left.min(Duration::from_millis(100)));
    }
}
