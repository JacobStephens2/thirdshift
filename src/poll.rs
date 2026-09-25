//! Asking GitHub the same question until it has an answer.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::interrupt;

/// How long to wait for something GitHub should soon report: CI appearing
/// after a push, or whether a PR is mergeable. `THIRDSHIFT_CI_GRACE_MS`
/// overrides it, for tests.
pub fn grace_period() -> Duration {
    millis_from_env("THIRDSHIFT_CI_GRACE_MS").unwrap_or(Duration::from_secs(60))
}

/// Ask `probe` every poll interval until it answers `Some`, and return the
/// answer. Fails with `interrupted` as soon as the Run is interrupted.
pub fn until<T>(mut probe: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    loop {
        if let Some(answer) = probe()? {
            return Ok(answer);
        }
        sleep(poll_interval())?;
    }
}

/// Like [`until`], but give up with `None` once `limit` has passed.
pub fn within<T>(
    limit: Duration,
    mut probe: impl FnMut() -> Result<Option<T>>,
) -> Result<Option<T>> {
    let deadline = Instant::now() + limit;
    until(|| match probe()? {
        Some(answer) => Ok(Some(Some(answer))),
        None if Instant::now() >= deadline => Ok(Some(None)),
        None => Ok(None),
    })
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
