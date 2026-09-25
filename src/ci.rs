//! Watching CI on the Issue branch's head commit.

use anyhow::Result;

use crate::github::{self, Check, CheckState};
use crate::issue::IssueUrl;
use crate::poll;
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
    let grace = poll::grace_period();
    let short = &sha[..sha.len().min(7)];
    progress::step(format_args!(
        "waiting up to {}s for CI on {short}",
        grace.as_secs()
    ));
    let appeared = poll::within(grace, || {
        Ok((!github::checks_on(issue, sha)?.is_empty()).then_some(()))
    })?;
    if appeared.is_none() {
        progress::step(format_args!(
            "no CI checks appeared on {short}; counting that as passing"
        ));
        return Ok(Ci::Absent);
    }

    let mut reported_pending = None;
    let checks = poll::until(|| {
        let checks = github::checks_on(issue, sha)?;
        let pending = count(&checks, CheckState::Pending);
        if pending == 0 {
            return Ok(Some(checks));
        }
        if reported_pending != Some(pending) {
            progress::step(format_args!(
                "CI on {short}: {pending} of {} checks still running",
                checks.len()
            ));
            reported_pending = Some(pending);
        }
        Ok(None)
    })?;
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
    Ok(Ci::Failed(failed))
}

fn count(checks: &[Check], state: CheckState) -> usize {
    checks.iter().filter(|check| check.state == state).count()
}
