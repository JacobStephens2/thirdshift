//! Watching CI on the Issue branch's head commit.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::github::{self, Check, CheckState};
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::progress;

/// How CI on a commit ended.
pub enum Ci {
    /// No check or status appeared within the grace period: no CI, which
    /// counts as passing.
    Absent,
    Passed,
    /// Red, with the checks that failed.
    Failed(Vec<Check>),
}

/// Wait up to the grace period for any check or status on `sha`, then watch
/// them until they have all finished.
pub fn watch(issue: &IssueUrl, sha: &str) -> Result<Ci> {
    let grace = grace_period();
    let short = &sha[..sha.len().min(7)];
    progress::step(format_args!(
        "waiting up to {}s for CI on {short}",
        grace.as_secs()
    ));
    let deadline = Instant::now() + grace;
    let mut reported_pending = None;
    loop {
        let checks = github::checks_on(issue, sha)?;
        if checks.is_empty() {
            if Instant::now() >= deadline {
                progress::step(format_args!(
                    "no CI checks appeared on {short}; counting that as passing"
                ));
                return Ok(Ci::Absent);
            }
        } else {
            let pending = count(&checks, CheckState::Pending);
            if pending == 0 {
                let failed: Vec<Check> = checks
                    .into_iter()
                    .filter(|check| check.state == CheckState::Failed)
                    .collect();
                if failed.is_empty() {
                    progress::step(format_args!("CI passed on {short}"));
                    return Ok(Ci::Passed);
                }
                let names: Vec<&str> = failed.iter().map(|check| check.name.as_str()).collect();
                progress::step(format_args!("CI failed on {short}: {}", names.join(", ")));
                return Ok(Ci::Failed(failed));
            }
            if reported_pending != Some(pending) {
                progress::step(format_args!(
                    "CI on {short}: {pending} of {} checks still running",
                    checks.len()
                ));
                reported_pending = Some(pending);
            }
        }
        sleep(poll_interval())?;
    }
}

fn count(checks: &[Check], state: CheckState) -> usize {
    checks.iter().filter(|check| check.state == state).count()
}

/// How long to wait for CI to appear after a push, and for GitHub to work
/// out whether a PR is mergeable. `THIRDSHIFT_CI_GRACE_MS` overrides it, for
/// tests.
pub fn grace_period() -> Duration {
    millis_from_env("THIRDSHIFT_CI_GRACE_MS").unwrap_or(Duration::from_secs(60))
}

/// How often to ask GitHub again. `THIRDSHIFT_POLL_MS` overrides it, for
/// tests.
pub fn poll_interval() -> Duration {
    millis_from_env("THIRDSHIFT_POLL_MS").unwrap_or(Duration::from_secs(10))
}

fn millis_from_env(name: &str) -> Option<Duration> {
    let millis = std::env::var(name).ok()?.parse().ok()?;
    Some(Duration::from_millis(millis))
}

/// Sleep for `duration`, failing with `interrupted` as soon as the Run is
/// interrupted.
pub fn sleep(duration: Duration) -> Result<()> {
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
