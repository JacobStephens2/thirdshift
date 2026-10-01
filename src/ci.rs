//! Watching CI on the Issue branch's head commit.

use anyhow::Result;

use crate::github::{self, ActionsJob, Check, CheckState};
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
    watch_to_end(issue, sha, base_commit)
}

/// The Check re-run of `sha`: ask GitHub to re-run the branch's own checks
/// in `failed`, once for each GitHub Actions workflow run they belong to,
/// then [`watch`] CI on `sha` again, from the new attempt on: it waits up to
/// the grace period for GitHub to list none of the check runs that failed,
/// so what the attempt before came to is never taken for the re-run's.
/// Inherited failures are left as they are.
///
/// `None` if there is no re-run to watch, each for a reason told in a
/// progress line: one of those checks is no GitHub Actions job, so nothing
/// is asked for, since the commit could not go green; GitHub refused; or no
/// new attempt appeared.
pub fn rerun(
    issue: &IssueUrl,
    sha: &str,
    base_commit: Option<&str>,
    failed: &FailedChecks,
) -> Result<Option<Ci>> {
    let short = short(sha);
    let (jobs, others): (Vec<&Check>, Vec<&Check>) =
        failed.own.iter().partition(|check| check.job.is_some());
    if !others.is_empty() {
        progress::step(format_args!(
            "the failed checks on {short} can't be re-run: not GitHub Actions jobs: {}",
            names(&others)
        ));
        return Ok(None);
    }
    let jobs: Vec<ActionsJob> = jobs.iter().filter_map(|check| check.job).collect();
    progress::step(format_args!(
        "re-running the failed checks on {short}: {}",
        check_names(&failed.own)
    ));
    let mut workflow_runs: Vec<u64> = jobs.iter().map(|job| job.workflow_run).collect();
    workflow_runs.sort_unstable();
    workflow_runs.dedup();
    for workflow_run in workflow_runs {
        if let Err(error) = github::rerun_failed_jobs(issue, workflow_run) {
            progress::step(format_args!("GitHub refused the re-run: {error:#}"));
            return Ok(None);
        }
    }

    let grace = poll::grace_period();
    let appeared = poll::within(grace, || {
        let checks = github::checks_on(issue, sha)?;
        let stale = checks
            .iter()
            .filter_map(|check| check.job)
            .any(|job| jobs.iter().any(|failed| failed.check_run == job.check_run));
        Ok((!stale).then_some(()))
    })?;
    if appeared.is_none() {
        progress::step(format_args!(
            "no re-run appeared on {short} within {}s",
            grace.as_secs()
        ));
        return Ok(None);
    }
    watch_to_end(issue, sha, base_commit).map(Some)
}

/// [`watch`], once checks have appeared on `sha`.
fn watch_to_end(issue: &IssueUrl, sha: &str, base_commit: Option<&str>) -> Result<Ci> {
    let short = short(sha);
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
    names(&checks.iter().collect::<Vec<_>>())
}

fn names(checks: &[&Check]) -> String {
    let names: Vec<&str> = checks.iter().map(|check| check.name.as_str()).collect();
    names.join(", ")
}

/// `checks` as a list, a line each: its name, and its URL if it has one.
pub fn check_list(checks: &[Check]) -> String {
    checks
        .iter()
        .map(|check| match &check.url {
            Some(url) => format!("- {}: {url}\n", check.name),
            None => format!("- {}\n", check.name),
        })
        .collect()
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
