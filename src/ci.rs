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

impl Ci {
    /// Whether it is red on a check that is the branch's own to fix.
    pub fn has_own_failures(&self) -> bool {
        matches!(self, Ci::Failed(failed) if !failed.own.is_empty())
    }
}

/// The checks that failed on a commit, at least one, split by whether the
/// branch is the one to fix them.
pub struct FailedChecks {
    /// The branch's own failures, for a CI-fix Repair.
    pub own: Vec<Check>,
    /// Inherited failures: checks that also failed on the Base branch commit,
    /// as did every check there of the same name.
    pub inherited: Vec<Check>,
    /// Where the Inherited failures fail on the Base branch commit: its own
    /// checks of those names, with their URLs there. Every one failed there
    /// and shares its name with an Inherited failure; empty exactly when
    /// `inherited` is.
    pub on_base: Vec<Check>,
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
/// the grace period for GitHub to list none of the failed check runs of
/// those workflow runs, so what the attempt before came to is never taken
/// for the re-run's. No workflow run is re-run for an Inherited failure.
///
/// `None` if there is no re-run to watch, each for a reason told in a
/// progress line: one of those checks is no GitHub Actions job, so nothing
/// is asked for, since the commit could not go green; the request failed, as when GitHub
/// refuses it; or no
/// new attempt appeared.
pub fn rerun(
    issue: &IssueUrl,
    sha: &str,
    base_commit: Option<&str>,
    failed: &FailedChecks,
) -> Result<Option<Ci>> {
    let short = short(sha);
    let not_jobs = failed.own.iter().filter(|check| check.job.is_none());
    let not_jobs = check_names(not_jobs);
    if !not_jobs.is_empty() {
        progress::step(format_args!(
            "the failed checks on {short} can't be re-run: not GitHub Actions jobs: {not_jobs}"
        ));
        return Ok(None);
    }
    progress::step(format_args!(
        "re-running the failed checks on {short}: {}",
        check_names(&failed.own)
    ));
    let mut workflow_runs: Vec<u64> = jobs_of(&failed.own).map(|job| job.workflow_run).collect();
    workflow_runs.sort_unstable();
    workflow_runs.dedup();
    for workflow_run in &workflow_runs {
        if let Err(error) = github::rerun_failed_jobs(issue, *workflow_run) {
            progress::step(format_args!(
                "the failed checks on {short} were not re-run: {error:#}"
            ));
            return Ok(None);
        }
    }

    // GitHub re-runs every failed job of a workflow run, so an Inherited
    // failure that shares one with the branch's own gets a new attempt too.
    let previous_attempt: Vec<u64> = jobs_of(&failed.own)
        .chain(jobs_of(&failed.inherited))
        .filter(|job| workflow_runs.contains(&job.workflow_run))
        .map(|job| job.check_run)
        .collect();
    let grace = poll::grace_period();
    let new_attempt = poll::within(grace, || {
        let checks = github::checks_on(issue, sha)?;
        let still_listed = jobs_of(&checks).any(|job| previous_attempt.contains(&job.check_run));
        Ok((!still_listed).then_some(()))
    })?;
    if new_attempt.is_none() {
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
    let on_base = match base_commit {
        Some(base_commit) => github::checks_on(issue, base_commit)?,
        None => Vec::new(),
    };
    Ok(Ci::Failed(split(failed, on_base)))
}

/// Split the checks `failed` on a commit into the branch's own and Inherited
/// failures, by name, against `on_base`, the checks on the Base branch
/// commit, none if no Base branch commit was given, and pick out the Base
/// branch's checks behind the Inherited failures. A failed check is inherited when every check of
/// its name on the Base branch commit failed, and there is at least one.
fn split(failed: Vec<Check>, on_base: Vec<Check>) -> FailedChecks {
    let (inherited, own): (Vec<Check>, Vec<Check>) = failed.into_iter().partition(|check| {
        let mut same_name = on_base.iter().filter(|base| base.name == check.name);
        // Checks sharing the name with only some of them failed there can't
        // be told apart, so the failure stays the branch's own.
        same_name.next().is_some_and(is_failed) && same_name.all(is_failed)
    });
    let on_base = on_base
        .into_iter()
        .filter(|base| inherited.iter().any(|check| check.name == base.name))
        .collect();
    FailedChecks {
        own,
        inherited,
        on_base,
    }
}

/// The names of `checks`, as in "test, lint".
pub fn check_names<'a>(checks: impl IntoIterator<Item = &'a Check>) -> String {
    let names: Vec<&str> = checks
        .into_iter()
        .map(|check| check.name.as_str())
        .collect();
    names.join(", ")
}

/// The GitHub Actions jobs among `checks`.
fn jobs_of(checks: &[Check]) -> impl Iterator<Item = ActionsJob> + '_ {
    checks.iter().filter_map(|check| check.job)
}

/// `checks` as a list, a line each: its name, and its URL if it has one.
pub fn check_list(checks: &[Check]) -> String {
    checks
        .iter()
        .map(|check| format!("- {}\n", check_with_url(check)))
        .collect()
}

/// The name of `check`, and its URL if it has one, as in `test: <url>`.
pub fn check_with_url(check: &Check) -> String {
    match &check.url {
        Some(url) => format!("{}: {url}", check.name),
        None => check.name.clone(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn check(name: &str, state: CheckState, url: &str) -> Check {
        Check {
            name: name.to_string(),
            state,
            url: Some(url.to_string()),
            job: None,
        }
    }

    fn red(name: &str) -> Check {
        check(name, CheckState::Failed, &format!("https://head/{name}"))
    }

    /// `checks` as [`check_with_url`] writes them.
    fn listed(checks: &[Check]) -> Vec<String> {
        checks.iter().map(check_with_url).collect()
    }

    #[test]
    fn a_red_check_with_none_of_its_name_on_the_base_branch_is_the_branchs_own() {
        let on_base = vec![check("lint", CheckState::Failed, "https://base/lint")];

        let failed = split(vec![red("test")], on_base);

        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
        assert!(failed.inherited.is_empty());
        assert!(failed.on_base.is_empty());
    }

    #[test]
    fn a_red_check_whose_every_same_name_check_failed_on_the_base_branch_is_inherited() {
        let on_base = vec![
            check("test", CheckState::Failed, "https://base/test/1"),
            check("test", CheckState::Failed, "https://base/test/2"),
        ];

        let failed = split(vec![red("test")], on_base);

        assert!(failed.own.is_empty());
        assert_eq!(listed(&failed.inherited), ["test: https://head/test"]);
        assert_eq!(
            listed(&failed.on_base),
            ["test: https://base/test/1", "test: https://base/test/2"]
        );
    }

    #[test]
    fn a_red_check_whose_same_name_checks_only_partly_failed_on_the_base_branch_is_the_branchs_own()
    {
        let on_base = vec![
            check("test", CheckState::Failed, "https://base/test/1"),
            check("test", CheckState::Passed, "https://base/test/2"),
        ];

        let failed = split(vec![red("test")], on_base);

        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
        assert!(failed.inherited.is_empty());
        assert!(failed.on_base.is_empty());
    }

    #[test]
    fn a_base_branch_check_named_as_no_red_check_is_not_carried() {
        let on_base = vec![
            check("test", CheckState::Failed, "https://base/test"),
            check("lint", CheckState::Failed, "https://base/lint"),
        ];

        let failed = split(vec![red("test")], on_base);

        assert!(failed.own.is_empty());
        assert_eq!(listed(&failed.inherited), ["test: https://head/test"]);
        assert_eq!(listed(&failed.on_base), ["test: https://base/test"]);
    }

    #[test]
    fn with_no_base_branch_commit_every_red_check_is_the_branchs_own() {
        let failed = split(vec![red("test"), red("lint")], Vec::new());

        assert_eq!(
            listed(&failed.own),
            ["test: https://head/test", "lint: https://head/lint"]
        );
        assert!(failed.inherited.is_empty());
        assert!(failed.on_base.is_empty());
    }
}
