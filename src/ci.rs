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
    Failed(FailedChecks),
}

/// The checks that failed on a commit, at least one, split by whether the
/// branch is the one to fix them.
pub struct FailedChecks {
    /// The branch's own failures, for a CI-fix Repair.
    pub own: Vec<Check>,
    /// Inherited failures: checks that also failed on the Base branch commit,
    /// as did every check there of the same name.
    pub inherited: Vec<Check>,
}

/// Wait up to the grace period for any check or status on `sha`, then watch
/// them until they have all finished. Checks that failed are compared, by
/// name, with the checks on `base_commit` as they stand: the Base branch's CI
/// is never waited for or triggered, and no log text is read. With no
/// `base_commit`, as in a Base fix, every check that failed is the branch's
/// own.
pub fn watch(issue: &IssueUrl, sha: &str, base_commit: Option<&str>) -> Result<Ci> {
    let grace = poll::grace_period();
    let short = short(sha);
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
    let failed: Vec<Check> = checks.into_iter().filter(is_failed).collect();
    if failed.is_empty() {
        progress::step(format_args!("CI passed on {short}"));
        return Ok(Ci::Passed);
    }
    progress::step(format_args!(
        "CI failed on {short}: {}",
        check_names(&failed)
    ));
    let Some(base_commit) = base_commit else {
        return Ok(Ci::Failed(FailedChecks {
            own: failed,
            inherited: Vec::new(),
        }));
    };
    let on_base = github::checks_on(issue, base_commit)?;
    let (inherited, own) = failed.into_iter().partition(|check| {
        let mut same_name = on_base.iter().filter(|base| base.name == check.name);
        // Checks sharing the name with only some of them failed there can't
        // be told apart, so the failure stays the branch's own.
        same_name.next().is_some_and(is_failed) && same_name.all(is_failed)
    });
    Ok(Ci::Failed(FailedChecks { own, inherited }))
}

/// The names of `checks`, as in "test, lint".
pub fn check_names(checks: &[Check]) -> String {
    let names: Vec<&str> = checks.iter().map(|check| check.name.as_str()).collect();
    names.join(", ")
}

/// `sha` shortened to 7 characters, as in progress messages.
pub fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn is_failed(check: &Check) -> bool {
    check.state == CheckState::Failed
}

fn count(checks: &[Check], state: CheckState) -> usize {
    checks.iter().filter(|check| check.state == state).count()
}
