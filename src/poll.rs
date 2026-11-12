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

    /// A waiting line for a state whose first line was already written:
    /// it repeats only after the interval.
    pub fn already_reported(state: T) -> Self {
        Self {
            reported: Some(state),
            ..Self::new()
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

/// A GitHub wait's cadence, reset whenever its observed state changes.
/// Ten seconds for the first minute, thirty through minute four, then sixty.
/// The existing test override scales both the intervals and their age thresholds.
pub struct Backoff<T> {
    observed: Option<(T, Instant)>,
}

impl<T: PartialEq> Backoff<T> {
    pub fn new() -> Self {
        Self { observed: None }
    }

    /// Time is supplied at the observation seam so schedules need no real sleeps
    /// in tests. Equal observations keep the age of the last change.
    pub fn observe(&mut self, state: T, now: Instant) -> Duration {
        if self
            .observed
            .as_ref()
            .is_none_or(|(last, _)| *last != state)
        {
            self.observed = Some((state, now));
        }
        let unchanged = now.duration_since(self.observed.as_ref().unwrap().1);
        let shortest = poll_interval();
        if unchanged < shortest.saturating_mul(6) {
            shortest
        } else if unchanged < shortest.saturating_mul(24) {
            shortest.saturating_mul(3)
        } else {
            shortest.saturating_mul(6)
        }
    }
}

/// Fixed cadence for local host-resource observations, interruptibly.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{Check, Conclusion};

    #[test]
    fn unchanged_github_waits_grow_to_a_sixty_second_ceiling() {
        let start = Instant::now();
        let mut backoff = Backoff::new();
        for (elapsed, expected) in [
            (0, 10),
            (59, 10),
            (60, 30),
            (239, 30),
            (240, 60),
            (1800, 60),
        ] {
            assert_eq!(
                backoff.observe((), start + Duration::from_secs(elapsed)),
                Duration::from_secs(expected)
            );
        }
    }

    #[test]
    fn a_changed_check_or_new_check_resets_the_wait_to_ten_seconds() {
        let pending = Check {
            name: "build".into(),
            conclusion: Conclusion::Unfinished,
            url: None,
            job: None,
            workflow_run: None,
        };
        let mut completed = pending.clone();
        completed.conclusion = Conclusion::Success;
        let mut test = pending.clone();
        test.name = "test".into();
        let mut added = pending.clone();
        added.name = "lint".into();
        for changed in [
            vec![completed, test.clone()],
            vec![pending.clone(), test.clone(), added],
        ] {
            let start = Instant::now();
            let initial = vec![pending.clone(), test.clone()];
            let mut backoff = Backoff::new();
            backoff.observe(initial.clone(), start);
            assert_eq!(
                backoff.observe(initial, start + Duration::from_secs(600)),
                Duration::from_secs(60)
            );
            assert_eq!(
                backoff.observe(changed.clone(), start + Duration::from_secs(610)),
                Duration::from_secs(10)
            );
            assert_eq!(
                backoff.observe(changed.clone(), start + Duration::from_secs(669)),
                Duration::from_secs(10)
            );
            assert_eq!(
                backoff.observe(changed, start + Duration::from_secs(670)),
                Duration::from_secs(30)
            );
        }
    }
}
