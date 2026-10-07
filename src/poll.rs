//! Shared grace periods and interruptible pauses for GitHub observations.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::interrupt;

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
fn sleep(duration: Duration) -> Result<()> {
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
