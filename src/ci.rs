//! Watching CI on the Issue branch's head commit.
//!
//! What it reads of GitHub and asks of it, how it waits, and its progress
//! lines all go through [`Outside`]: [`OnGitHub`] does each through `gh`, the
//! poll module and the progress lines; `InMemory`, in tests, from a script,
//! recording each re-run request and progress line.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};

use crate::github::{ActionsJob, Check, Conclusion, GitHub};
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
/// own. `actionable` gates each GitHub observation before and after it.
pub fn watch(
    issue: &IssueUrl,
    sha: &str,
    base_commit: Option<&str>,
    actionable: &mut dyn FnMut() -> Result<()>,
) -> Result<Ci> {
    watch_through(&mut OnGitHub { issue, actionable }, sha, base_commit)
}

/// The Check re-run of `sha`: ask GitHub to re-run the branch's own checks
/// in `failed`, once for each GitHub Actions workflow run they belong to,
/// then watch CI on `sha` again, from the new attempt on: it waits up to
/// the grace period for every re-run check name to have a check run not
/// listed just before the request, including Inherited failures sharing a
/// successfully requested workflow run. Each must stay listed until it
/// finishes, so a missing check or an earlier attempt cannot establish green
/// CI. A refused request is reported, then all current checks on the same
/// head are watched: a Repair may already
/// have re-run them. No workflow run is re-run for an Inherited failure.
///
/// `None` if there is no re-run to watch, each for a reason told in a
/// progress line: one of those checks is no GitHub Actions job, so nothing
/// is asked for, since the commit could not go green; or no new attempt
/// appeared for a successful request. A checks-read error or a listing
/// that disappears while watching is an error, never green or absent CI.
pub fn rerun(
    issue: &IssueUrl,
    sha: &str,
    base_commit: Option<&str>,
    failed: &FailedChecks,
    actionable: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<Ci>> {
    rerun_through(
        &mut OnGitHub { issue, actionable },
        sha,
        base_commit,
        failed,
    )
}

/// What the CI watch reads of GitHub and asks of it, how it waits, and
/// where its progress lines go.
trait Outside {
    /// A point the grace period ends at, from when it was started.
    type Deadline;
    /// The checks and statuses on the commit `sha`.
    fn checks_on(&mut self, sha: &str) -> Result<Vec<Check>>;
    /// Ask GitHub to re-run the failed jobs of the workflow run
    /// `workflow_run`.
    fn rerun_failed_jobs(&mut self, workflow_run: u64) -> Result<()>;
    /// Wait one poll interval. Fails with `interrupted` as soon as the Run
    /// is interrupted.
    fn pause(&mut self) -> Result<()>;
    /// Wait a check-read retry delay, interruptibly, without polling CI.
    fn retry_pause(&mut self, delay: Duration) -> Result<()>;
    /// Start a grace period: how long it is, for progress lines, and the
    /// deadline it ends at.
    fn start_grace(&mut self) -> (Duration, Self::Deadline);
    /// Whether `deadline` has passed.
    fn passed(&mut self, deadline: &Self::Deadline) -> bool;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// GitHub through `gh`, for `issue`'s repository, with the poll module's
/// interval and grace period.
struct OnGitHub<'a> {
    issue: &'a IssueUrl,
    actionable: &'a mut dyn FnMut() -> Result<()>,
}

impl Outside for OnGitHub<'_> {
    type Deadline = Instant;

    fn checks_on(&mut self, sha: &str) -> Result<Vec<Check>> {
        (self.actionable)()?;
        let checks = GitHub::new().checks_on(self.issue, sha)?;
        (self.actionable)()?;
        Ok(checks)
    }

    fn rerun_failed_jobs(&mut self, workflow_run: u64) -> Result<()> {
        (self.actionable)()?;
        GitHub::new().rerun_failed_jobs(self.issue, workflow_run)
    }

    fn pause(&mut self) -> Result<()> {
        poll::pause()
    }

    fn retry_pause(&mut self, delay: Duration) -> Result<()> {
        poll::sleep(delay)
    }

    fn start_grace(&mut self) -> (Duration, Instant) {
        let grace = poll::grace_period();
        (grace, Instant::now() + grace)
    }

    fn passed(&mut self, deadline: &Instant) -> bool {
        Instant::now() >= *deadline
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }
}

/// A fresh, whole observation on the same full commit each time. Three total
/// attempts, separated by one then two seconds; only transient failures retry.
/// Successful observations alone can establish CI facts.
fn read_checks(outside: &mut impl Outside, sha: &str) -> Result<Vec<Check>> {
    let mut failures = Vec::new();
    for attempt in 1..=3 {
        match outside.checks_on(sha) {
            Ok(checks) => return Ok(checks),
            Err(error) => {
                if !crate::github::check_read_is_transient(&error) {
                    return Err(error);
                }
                if attempt == 3 {
                    return Err(error).with_context(|| {
                        format!(
                            "CI check read on {sha} exhausted 3 attempts; earlier failures: {}",
                            failures.join("; ")
                        )
                    });
                }
                let delay = Duration::from_secs(attempt);
                failures.push(format!("attempt {attempt}: {error:#}"));
                outside.step(format!(
                    "CI check read on {} failed (attempt {attempt}/3): {error:#}; retrying in {}s",
                    short(sha),
                    delay.as_secs()
                ));
                outside.retry_pause(delay)?;
            }
        }
    }
    unreachable!("the third attempt returns success or exhaustion")
}

/// [`watch`], through `outside`.
fn watch_through(outside: &mut impl Outside, sha: &str, base_commit: Option<&str>) -> Result<Ci> {
    let (grace, deadline) = outside.start_grace();
    let short = short(sha);
    outside.step(format!(
        "waiting up to {}s for CI on {short}",
        grace.as_secs()
    ));
    let appeared = within(outside, deadline, |outside| {
        Ok((!read_checks(outside, sha)?.is_empty()).then_some(()))
    })?;
    if appeared.is_none() {
        outside.step(format!(
            "no CI checks appeared on {short}; counting that as passing"
        ));
        return Ok(Ci::Absent);
    }
    watch_to_end(outside, sha, base_commit, None)
}

/// [`rerun`], through `outside`.
fn rerun_through(
    outside: &mut impl Outside,
    sha: &str,
    base_commit: Option<&str>,
    failed: &FailedChecks,
) -> Result<Option<Ci>> {
    let short = short(sha);
    let not_jobs = failed.own.iter().filter(|check| check.job.is_none());
    let not_jobs = check_names(not_jobs);
    if !not_jobs.is_empty() {
        outside.step(format!(
            "the failed checks on {short} can't be re-run: not GitHub Actions jobs: {not_jobs}"
        ));
        return Ok(None);
    }
    outside.step(format!(
        "re-running the failed checks on {short}: {}",
        check_names(&failed.own)
    ));
    let mut workflow_runs: Vec<u64> = jobs_of(&failed.own).map(|job| job.workflow_run).collect();
    workflow_runs.sort_unstable();
    workflow_runs.dedup();
    let before = read_checks(outside, sha)?;
    let previous_attempt: Vec<u64> = jobs_of(&before)
        .chain(jobs_of(&failed.own))
        .chain(jobs_of(&failed.inherited))
        .map(|job| job.check_run)
        .collect();
    let mut requested = Vec::new();
    for workflow_run in &workflow_runs {
        if let Err(error) = outside.rerun_failed_jobs(*workflow_run) {
            outside.step(format!(
                "the failed checks on {short} were not re-run: {error:#}"
            ));
            break;
        }
        requested.push(*workflow_run);
    }
    // GitHub re-runs every failed job of a workflow run, so an Inherited
    // failure that shares one with the branch's own gets a new attempt too.
    let rerun_checks = RerunChecks {
        checks: failed.own.iter().chain(&failed.inherited).collect(),
        previous_attempt,
        requested,
    };
    if rerun_checks.requested.is_empty() {
        return watch_to_end(outside, sha, base_commit, Some(&rerun_checks)).map(Some);
    }
    let (grace, deadline) = outside.start_grace();
    let new_attempt = within(outside, deadline, |outside| {
        let checks = read_checks(outside, sha)?;
        Ok(rerun_checks.new_attempts_listed(&checks).then_some(()))
    })?;
    if new_attempt.is_none() {
        outside.step(format!(
            "no re-run appeared on {short} within {}s",
            grace.as_secs()
        ));
        return Ok(None);
    }
    watch_to_end(outside, sha, base_commit, Some(&rerun_checks)).map(Some)
}

/// Known failed checks that must stay listed during a Check re-run. Those
/// in successfully requested workflow runs must also have new attempts.
struct RerunChecks<'a> {
    checks: Vec<&'a Check>,
    previous_attempt: Vec<u64>,
    requested: Vec<u64>,
}

impl RerunChecks<'_> {
    fn is_requested(&self, check: &Check) -> bool {
        check
            .job
            .is_some_and(|job| self.requested.contains(&job.workflow_run))
    }

    fn new_attempts_listed(&self, checks: &[Check]) -> bool {
        self.checks
            .iter()
            .filter(|check| self.is_requested(check))
            .all(|check| self.check_is_listed(check, checks))
    }

    fn listed_in(&self, checks: &[Check]) -> bool {
        self.checks
            .iter()
            .all(|check| self.check_is_listed(check, checks))
    }

    fn check_is_listed(&self, expected: &Check, checks: &[Check]) -> bool {
        checks.iter().any(|check| {
            check.name == expected.name
                && (!self.is_requested(expected)
                    || check.job.is_some_and(|job| {
                        Some(job.workflow_run) == expected.job.map(|job| job.workflow_run)
                            && !self.previous_attempt.contains(&job.check_run)
                    }))
        })
    }
}

/// [`watch_through`], once checks have appeared on `sha`, or a Check re-run
/// whose checks must stay listed with new attempts until they finish.
fn watch_to_end(
    outside: &mut impl Outside,
    sha: &str,
    base_commit: Option<&str>,
    rerun_checks: Option<&RerunChecks<'_>>,
) -> Result<Ci> {
    let short = short(sha);
    let mut reported_counts = None;
    let checks = until(outside, |outside| {
        let checks = read_checks(outside, sha)?;
        ensure!(
            !checks.is_empty() && rerun_checks.is_none_or(|rerun| rerun.listed_in(&checks)),
            "CI checks disappeared on {short}; cannot verify CI"
        );
        let pending = count(&checks, Reading::Pending);
        if pending == 0 {
            return Ok(Some(checks));
        }
        let counts = (pending, checks.len());
        if reported_counts != Some(counts) {
            outside.step(format!(
                "CI on {short}: {pending} still running out of {} checks listed so far",
                checks.len()
            ));
            reported_counts = Some(counts);
        }
        Ok(None)
    })?;
    let failed: Vec<Check> = checks.into_iter().filter(is_failed).collect();
    if failed.is_empty() {
        outside.step(format!("CI passed on {short}"));
        return Ok(Ci::Passed);
    }
    outside.step(format!("CI failed on {short}: {}", check_names(&failed)));
    let on_base = match base_commit {
        Some(base_commit) => read_checks(outside, base_commit)?,
        None => Vec::new(),
    };
    Ok(Ci::Failed(split(failed, on_base)))
}

/// Ask `probe` every poll interval of `outside` until it answers `Some`, and
/// return the answer. Fails with `interrupted` as soon as the Run is
/// interrupted. Unlike [`poll::until`], `probe` is lent `outside` on each
/// ask.
fn until<O: Outside, T>(
    outside: &mut O,
    mut probe: impl FnMut(&mut O) -> Result<Option<T>>,
) -> Result<T> {
    loop {
        if let Some(answer) = probe(outside)? {
            return Ok(answer);
        }
        outside.pause()?;
    }
}

/// Like [`until`], but give up with `None` once `deadline`, a grace
/// period's, has passed.
fn within<O: Outside, T>(
    outside: &mut O,
    deadline: O::Deadline,
    mut probe: impl FnMut(&mut O) -> Result<Option<T>>,
) -> Result<Option<T>> {
    until(outside, |outside| match probe(outside)? {
        Some(answer) => Ok(Some(Some(answer))),
        None if outside.passed(&deadline) => Ok(Some(None)),
        None => Ok(None),
    })
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

/// What a check's conclusion means for CI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reading {
    Pending,
    Passed,
    Failed,
}

/// The reading of `check`: CI watch is the one place a conclusion is read.
fn reading(check: &Check) -> Reading {
    match &check.conclusion {
        Conclusion::Unfinished | Conclusion::Pending => Reading::Pending,
        Conclusion::Success | Conclusion::Neutral | Conclusion::Skipped => Reading::Passed,
        Conclusion::Failure
        | Conclusion::Cancelled
        | Conclusion::TimedOut
        | Conclusion::ActionRequired
        | Conclusion::Stale
        | Conclusion::StartupFailure
        | Conclusion::Error
        | Conclusion::Unrecognised(_) => Reading::Failed,
    }
}

fn is_failed(check: &Check) -> bool {
    reading(check) == Reading::Failed
}

fn count(checks: &[Check], wanted: Reading) -> usize {
    checks
        .iter()
        .filter(|check| reading(check) == wanted)
        .count()
}

#[cfg(test)]
mod in_memory {
    use std::time::Duration;

    use anyhow::{Result, bail};

    use super::Outside;
    use crate::github::Check;

    /// GitHub in memory: for each commit, a script of the check listings
    /// its reads take in turn, the last one repeating; the workflow runs
    /// whose re-run is refused; the pause an interrupt arrives on; and how
    /// many pauses a grace period lasts. Records the commits read, the
    /// re-run requests and the progress lines.
    pub struct InMemory {
        listings: Vec<(String, Vec<Vec<Check>>)>,
        refused: Vec<u64>,
        read_failures: Vec<(String, usize)>,
        read_errors: Vec<(String, usize, &'static str)>,
        interrupted_on: Option<usize>,
        grace_pauses: usize,
        pauses: usize,
        /// The commit of each read, in order.
        pub reads: Vec<String>,
        /// The workflow run of each re-run request, refused ones included.
        pub reruns: Vec<u64>,
        /// Every progress line handed on.
        pub steps: Vec<String>,
        pub retry_delays: Vec<Duration>,
    }

    impl InMemory {
        /// No checks on any commit, with a grace period of three pauses.
        pub fn new() -> Self {
            InMemory {
                listings: Vec::new(),
                refused: Vec::new(),
                read_failures: Vec::new(),
                read_errors: Vec::new(),
                interrupted_on: None,
                grace_pauses: 3,
                pauses: 0,
                reads: Vec::new(),
                reruns: Vec::new(),
                steps: Vec::new(),
                retry_delays: Vec::new(),
            }
        }

        /// Script the reads of `sha` to take `listings` in turn.
        pub fn listing(mut self, sha: &str, listings: Vec<Vec<Check>>) -> Self {
            self.listings.push((sha.to_string(), listings));
            self
        }

        /// Return truncated JSON instead of the listing on this read (from 1).
        pub fn truncated_on(mut self, sha: &str, read: usize) -> Self {
            self.read_failures.push((sha.to_string(), read));
            self
        }

        /// Fail this read (from 1) with an unclassified error or interruption.
        pub fn error_on(mut self, sha: &str, read: usize, message: &'static str) -> Self {
            self.read_errors.push((sha.to_string(), read, message));
            self
        }

        /// Refuse a re-run of `workflow_run`.
        pub fn refusing(mut self, workflow_run: u64) -> Self {
            self.refused.push(workflow_run);
            self
        }

        /// Interrupt the Run during pause `pause`, counted from 1.
        pub fn interrupted_on_pause(mut self, pause: usize) -> Self {
            self.interrupted_on = Some(pause);
            self
        }

        /// The reads made of `sha`.
        pub fn reads_of(&self, sha: &str) -> usize {
            self.reads.iter().filter(|read| *read == sha).count()
        }
    }

    impl Outside for InMemory {
        /// The number of pauses at which the grace period ends.
        type Deadline = usize;

        fn checks_on(&mut self, sha: &str) -> Result<Vec<Check>> {
            let read = self.reads_of(sha);
            self.reads.push(sha.to_string());
            if let Some((_, _, message)) = self
                .read_errors
                .iter()
                .find(|(commit, attempt, _)| commit == sha && *attempt == read + 1)
            {
                bail!("{message}");
            }
            if self.read_failures.contains(&(sha.to_string(), read + 1)) {
                return Err(serde_json::from_str::<serde_json::Value>("{\"name\":")
                    .unwrap_err()
                    .into());
            }
            let listing = self
                .listings
                .iter()
                .find(|(commit, _)| commit == sha)
                .and_then(|(_, listings)| listings.get(read).or(listings.last()));
            Ok(listing.cloned().unwrap_or_default())
        }

        fn rerun_failed_jobs(&mut self, workflow_run: u64) -> Result<()> {
            self.reruns.push(workflow_run);
            if self.refused.contains(&workflow_run) {
                bail!("gh: HTTP 403: workflow run {workflow_run} cannot be re-run");
            }
            Ok(())
        }

        fn pause(&mut self) -> Result<()> {
            self.pauses += 1;
            if self.interrupted_on == Some(self.pauses) {
                bail!("interrupted");
            }
            Ok(())
        }

        fn retry_pause(&mut self, delay: Duration) -> Result<()> {
            self.retry_delays.push(delay);
            self.pause()
        }

        fn start_grace(&mut self) -> (Duration, usize) {
            (Duration::from_secs(60), self.pauses + self.grace_pauses)
        }

        fn passed(&mut self, deadline: &usize) -> bool {
            self.pauses >= *deadline
        }

        fn step(&mut self, line: String) {
            self.steps.push(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::in_memory::InMemory;
    use super::*;

    const HEAD: &str = "abcdef0123456789";
    const BASE: &str = "0123456789abcdef";

    fn check(name: &str, conclusion: Conclusion, url: &str) -> Check {
        Check {
            name: name.to_string(),
            conclusion,
            url: Some(url.to_string()),
            job: None,
        }
    }

    fn red(name: &str) -> Check {
        check(name, Conclusion::Failure, &format!("https://head/{name}"))
    }

    fn pending(name: &str) -> Check {
        check(
            name,
            Conclusion::Unfinished,
            &format!("https://head/{name}"),
        )
    }

    fn green(name: &str) -> Check {
        check(name, Conclusion::Success, &format!("https://head/{name}"))
    }

    /// `check` as the GitHub Actions job whose check run is `check_run`, of
    /// the workflow run `workflow_run`.
    fn job(check: Check, check_run: u64, workflow_run: u64) -> Check {
        Check {
            job: Some(ActionsJob {
                check_run,
                workflow_run,
            }),
            ..check
        }
    }

    /// `checks` as [`check_with_url`] writes them.
    fn listed(checks: &[Check]) -> Vec<String> {
        checks.iter().map(check_with_url).collect()
    }

    /// The checks that failed, of `ci`, which must be red.
    fn failed_of(ci: Ci) -> FailedChecks {
        match ci {
            Ci::Failed(failed) => failed,
            Ci::Absent => panic!("CI was Absent, not Failed"),
            Ci::Passed => panic!("CI Passed, not Failed"),
        }
    }

    /// What watching CI on [`HEAD`] came to, against [`BASE`], where CI
    /// ends as `head` and the Base branch commit's checks are `on_base`.
    fn failed_on_watch(head: Vec<Check>, on_base: Vec<Check>) -> FailedChecks {
        let mut github = InMemory::new()
            .listing(HEAD, vec![head])
            .listing(BASE, vec![on_base]);
        failed_of(watch_through(&mut github, HEAD, Some(BASE)).unwrap())
    }

    #[test]
    fn a_truncated_initial_check_read_recovers_without_losing_pending_or_red_checks() {
        let mut github = InMemory::new()
            .listing(HEAD, vec![vec![], vec![pending("test")], vec![red("test")]])
            .truncated_on(HEAD, 1);

        let failed = failed_of(watch_through(&mut github, HEAD, None).unwrap());

        assert_eq!(listed(&failed.own), listed(&[red("test")]));
        assert_eq!(github.reads_of(HEAD), 3);
        assert!(github.reruns.is_empty());
        assert!(github.steps.iter().any(|line| line.contains("retry")));
    }

    #[test]
    fn transient_check_reads_exhaust_exactly_three_attempts_without_establishing_absent_ci() {
        let mut github = InMemory::new()
            .truncated_on(HEAD, 1)
            .truncated_on(HEAD, 2)
            .truncated_on(HEAD, 3);

        let error = watch_through(&mut github, HEAD, None).err().unwrap();

        assert_eq!(github.reads, [HEAD, HEAD, HEAD]);
        assert_eq!(
            github.retry_delays,
            [Duration::from_secs(1), Duration::from_secs(2)]
        );
        let diagnostic = format!("{error:#}");
        assert!(diagnostic.contains("exhausted 3 attempts"), "{diagnostic}");
        assert!(diagnostic.contains("attempt 1: EOF"), "{diagnostic}");
        assert!(diagnostic.contains("attempt 2: EOF"), "{diagnostic}");
        assert!(
            diagnostic.contains("EOF while parsing a value"),
            "{diagnostic}"
        );
        assert!(!github.steps.iter().any(|line| line.contains("passing")));
    }

    #[test]
    fn a_third_check_read_can_recover_to_green() {
        let mut github = InMemory::new()
            .listing(HEAD, vec![vec![green("test")]])
            .truncated_on(HEAD, 1)
            .truncated_on(HEAD, 2);

        assert!(matches!(
            watch_through(&mut github, HEAD, None).unwrap(),
            Ci::Passed
        ));
        assert_eq!(
            github.retry_delays,
            [Duration::from_secs(1), Duration::from_secs(2)]
        );
        assert!(github.reruns.is_empty());
    }

    #[test]
    fn a_permanent_or_interrupted_check_read_starts_no_retry() {
        for message in [
            "HTTP 401: Bad credentials",
            "unsupported response",
            "interrupted",
        ] {
            let mut github = InMemory::new().error_on(HEAD, 1, message);

            let error = watch_through(&mut github, HEAD, None).err().unwrap();

            assert_eq!(error.to_string(), message);
            assert_eq!(github.reads, [HEAD]);
            assert!(github.retry_delays.is_empty());
        }
    }

    #[test]
    fn cancellation_during_check_read_backoff_starts_no_further_read() {
        let mut github = InMemory::new()
            .truncated_on(HEAD, 1)
            .interrupted_on_pause(1);

        let error = watch_through(&mut github, HEAD, None).err().unwrap();

        assert_eq!(error.to_string(), "interrupted");
        assert_eq!(github.reads, [HEAD]);
        assert_eq!(github.retry_delays, [Duration::from_secs(1)]);
    }

    #[test]
    fn pending_check_polling_and_base_comparison_recover_to_real_failures() {
        let mut github = InMemory::new()
            .listing(
                HEAD,
                vec![
                    vec![pending("test")],
                    vec![],
                    vec![pending("test")],
                    vec![red("test")],
                ],
            )
            .listing(BASE, vec![vec![], vec![red("test")]])
            .truncated_on(HEAD, 2)
            .truncated_on(BASE, 1);

        let failed = failed_of(watch_through(&mut github, HEAD, Some(BASE)).unwrap());

        assert!(failed.own.is_empty());
        assert_eq!(listed(&failed.inherited), listed(&[red("test")]));
        assert_eq!(github.reads, [HEAD, HEAD, HEAD, HEAD, BASE, BASE]);
        assert_eq!(
            github.retry_delays,
            [Duration::from_secs(1), Duration::from_secs(1)]
        );
        assert!(github.reruns.is_empty());
    }

    #[test]
    fn check_rerun_read_recovery_never_repeats_a_workflow_request() {
        let old = job(red("test"), 1, 900);
        let new = job(green("test"), 2, 900);
        let failed = FailedChecks {
            own: vec![old.clone()],
            inherited: vec![],
            on_base: vec![],
        };
        // Before requesting, while waiting for the new attempt, and while
        // watching it to completion all use the same read recovery policy.
        for read in 1..=3 {
            let mut listings = vec![vec![old.clone()], vec![new.clone()], vec![new.clone()]];
            listings.insert(read - 1, vec![]);
            let mut github = InMemory::new()
                .listing(HEAD, listings)
                .truncated_on(HEAD, read);

            assert!(matches!(
                rerun_through(&mut github, HEAD, None, &failed).unwrap(),
                Some(Ci::Passed)
            ));
            assert_eq!(github.reruns, [900]);
            assert_eq!(github.retry_delays, [Duration::from_secs(1)]);
            assert_eq!(github.reads, [HEAD, HEAD, HEAD, HEAD]);
        }
    }

    #[test]
    fn check_rerun_recovery_keeps_old_attempt_and_disappearance_protections() {
        let old = job(red("test"), 1, 900);
        let new = job(green("test"), 2, 900);
        let failed = FailedChecks {
            own: vec![old.clone()],
            inherited: vec![],
            on_base: vec![],
        };
        let mut github = InMemory::new()
            .listing(HEAD, vec![vec![old]])
            .truncated_on(HEAD, 2);
        assert!(
            rerun_through(&mut github, HEAD, None, &failed)
                .unwrap()
                .is_none()
        );
        assert_eq!(github.reruns, [900]);

        let mut github = InMemory::new()
            .listing(HEAD, vec![failed.own.clone(), vec![new], vec![]])
            .truncated_on(HEAD, 3);
        let error = rerun_through(&mut github, HEAD, None, &failed)
            .err()
            .unwrap();
        assert!(error.to_string().contains("checks disappeared"));
        assert_eq!(github.reruns, [900]);
    }

    #[test]
    fn unreadable_checks_after_a_refused_rerun_cannot_establish_green() {
        let failed = FailedChecks {
            own: vec![job(red("test"), 1, 900)],
            inherited: vec![],
            on_base: vec![],
        };
        let mut github = InMemory::new()
            .listing(HEAD, vec![failed.own.clone()])
            .refusing(900)
            .truncated_on(HEAD, 2)
            .truncated_on(HEAD, 3)
            .truncated_on(HEAD, 4);

        let error = rerun_through(&mut github, HEAD, None, &failed)
            .err()
            .unwrap();

        assert!(error.to_string().contains("exhausted 3 attempts"));
        assert_eq!(github.reruns, [900]);
        assert_eq!(github.reads, [HEAD, HEAD, HEAD, HEAD]);
    }

    #[test]
    fn with_no_checks_within_the_grace_period_ci_is_absent() {
        let mut github = InMemory::new();

        let ci = watch_through(&mut github, HEAD, Some(BASE)).unwrap();

        assert!(matches!(ci, Ci::Absent));
        assert_eq!(
            github.steps,
            [
                "waiting up to 60s for CI on abcdef0",
                "no CI checks appeared on abcdef0; counting that as passing",
            ]
        );
    }

    #[test]
    fn pending_checks_are_watched_until_they_pass_telling_each_new_running_count() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![pending("test"), pending("lint")],
                vec![pending("test"), pending("lint")],
                vec![green("test"), pending("lint")],
                vec![green("test"), pending("lint")],
                vec![green("test"), green("lint")],
            ],
        );

        let ci = watch_through(&mut github, HEAD, Some(BASE)).unwrap();

        assert!(matches!(ci, Ci::Passed));
        assert_eq!(
            github.steps,
            [
                "waiting up to 60s for CI on abcdef0",
                "CI on abcdef0: 2 still running out of 2 checks listed so far",
                "CI on abcdef0: 1 still running out of 2 checks listed so far",
                "CI passed on abcdef0",
            ]
        );
        assert_eq!(github.reads_of(BASE), 0);
    }

    #[test]
    fn dependent_jobs_can_raise_both_pending_and_listed_counts() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![pending("build"), pending("lint")],
                vec![pending("build"), pending("lint")],
                vec![green("build"), pending("lint")],
                vec![
                    green("build"),
                    pending("lint"),
                    pending("test/linux"),
                    pending("test/macos"),
                ],
                vec![
                    green("build"),
                    green("lint"),
                    green("test/linux"),
                    green("test/macos"),
                ],
            ],
        );

        let ci = watch_through(&mut github, HEAD, Some(BASE)).unwrap();

        assert!(matches!(ci, Ci::Passed));
        assert_eq!(
            github.steps,
            [
                "waiting up to 60s for CI on abcdef0",
                "CI on abcdef0: 2 still running out of 2 checks listed so far",
                "CI on abcdef0: 1 still running out of 2 checks listed so far",
                "CI on abcdef0: 3 still running out of 4 checks listed so far",
                "CI passed on abcdef0",
            ]
        );
    }

    #[test]
    fn a_changed_listed_count_is_reported_even_when_pending_is_unchanged() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![pending("test")],
                vec![pending("test")],
                vec![pending("test")],
                vec![pending("test"), green("lint")],
                vec![pending("test"), green("lint")],
                vec![green("test"), green("lint")],
            ],
        );

        let ci = watch_through(&mut github, HEAD, Some(BASE)).unwrap();

        assert!(matches!(ci, Ci::Passed));
        assert_eq!(
            github.steps,
            [
                "waiting up to 60s for CI on abcdef0",
                "CI on abcdef0: 1 still running out of 1 checks listed so far",
                "CI on abcdef0: 1 still running out of 2 checks listed so far",
                "CI passed on abcdef0",
            ]
        );
    }

    #[test]
    fn the_base_branch_commit_is_read_once_after_every_check_has_finished() {
        let mut github = InMemory::new()
            .listing(
                HEAD,
                vec![
                    vec![pending("test"), pending("lint")],
                    vec![red("test"), pending("lint")],
                    vec![red("test"), red("lint")],
                ],
            )
            .listing(
                BASE,
                vec![vec![check(
                    "lint",
                    Conclusion::Failure,
                    "https://base/lint",
                )]],
            );

        let failed = failed_of(watch_through(&mut github, HEAD, Some(BASE)).unwrap());

        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
        assert_eq!(listed(&failed.inherited), ["lint: https://head/lint"]);
        assert_eq!(listed(&failed.on_base), ["lint: https://base/lint"]);
        assert_eq!(github.reads_of(BASE), 1);
        assert_eq!(github.reads.last().map(String::as_str), Some(BASE));
        assert_eq!(
            github.steps.last().map(String::as_str),
            Some("CI failed on abcdef0: test, lint")
        );
    }

    #[test]
    fn a_red_check_with_none_of_its_name_on_the_base_branch_is_the_branchs_own() {
        let on_base = vec![check("lint", Conclusion::Failure, "https://base/lint")];

        let failed = failed_on_watch(vec![red("test")], on_base);

        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
        assert!(failed.inherited.is_empty());
        assert!(failed.on_base.is_empty());
    }

    #[test]
    fn a_red_check_whose_every_same_name_check_failed_on_the_base_branch_is_inherited() {
        let on_base = vec![
            check("test", Conclusion::Failure, "https://base/test/1"),
            check("test", Conclusion::Failure, "https://base/test/2"),
        ];

        let failed = failed_on_watch(vec![red("test")], on_base);

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
            check("test", Conclusion::Failure, "https://base/test/1"),
            check("test", Conclusion::Success, "https://base/test/2"),
        ];

        let failed = failed_on_watch(vec![red("test")], on_base);

        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
        assert!(failed.inherited.is_empty());
        assert!(failed.on_base.is_empty());
    }

    #[test]
    fn a_base_branch_check_named_as_no_red_check_is_not_carried() {
        let on_base = vec![
            check("test", Conclusion::Failure, "https://base/test"),
            check("lint", Conclusion::Failure, "https://base/lint"),
        ];

        let failed = failed_on_watch(vec![red("test"), green("lint")], on_base);

        assert!(failed.own.is_empty());
        assert_eq!(listed(&failed.inherited), ["test: https://head/test"]);
        assert_eq!(listed(&failed.on_base), ["test: https://base/test"]);
    }

    /// Every conclusion GitHub can report, with its reading.
    fn every_conclusion() -> Vec<(Conclusion, Reading)> {
        // No wildcard arm: a new conclusion fails to compile here until it
        // has a row below.
        match Conclusion::Unfinished {
            Conclusion::Unfinished
            | Conclusion::Pending
            | Conclusion::Success
            | Conclusion::Neutral
            | Conclusion::Skipped
            | Conclusion::Failure
            | Conclusion::Cancelled
            | Conclusion::TimedOut
            | Conclusion::ActionRequired
            | Conclusion::Stale
            | Conclusion::StartupFailure
            | Conclusion::Error
            | Conclusion::Unrecognised(_) => {}
        }
        vec![
            (Conclusion::Unfinished, Reading::Pending),
            (Conclusion::Pending, Reading::Pending),
            (Conclusion::Success, Reading::Passed),
            (Conclusion::Neutral, Reading::Passed),
            (Conclusion::Skipped, Reading::Passed),
            (Conclusion::Failure, Reading::Failed),
            (Conclusion::Cancelled, Reading::Failed),
            (Conclusion::TimedOut, Reading::Failed),
            (Conclusion::ActionRequired, Reading::Failed),
            (Conclusion::Stale, Reading::Failed),
            (Conclusion::StartupFailure, Reading::Failed),
            (Conclusion::Error, Reading::Failed),
            (Conclusion::Unrecognised("".into()), Reading::Failed),
            (Conclusion::Unrecognised("someday".into()), Reading::Failed),
        ]
    }

    #[test]
    fn each_conclusion_on_the_head_is_read_as_pending_passed_or_the_branchs_own_failure() {
        for (conclusion, reading) in every_conclusion() {
            let concluded = vec![check("test", conclusion.clone(), "https://head/test")];
            // Read as the checks appear, then as the watch's first read.
            let mut github = InMemory::new().listing(
                HEAD,
                vec![concluded.clone(), concluded, vec![green("test")]],
            );

            let ci = watch_through(&mut github, HEAD, Some(BASE)).unwrap();

            let read = match ci {
                Ci::Passed
                    if github
                        .steps
                        .iter()
                        .any(|line| line.contains("still running")) =>
                {
                    Reading::Pending
                }
                Ci::Passed => Reading::Passed,
                Ci::Failed(failed) => {
                    assert_eq!(listed(&failed.own), ["test: https://head/test"]);
                    assert!(failed.inherited.is_empty(), "{conclusion:?}");
                    Reading::Failed
                }
                Ci::Absent => panic!("CI was Absent with {conclusion:?} on the head"),
            };
            assert_eq!(read, reading, "{conclusion:?} on the head");
        }
    }

    #[test]
    fn each_conclusion_on_the_base_branch_commit_counts_as_an_inherited_failure_when_failed() {
        for (conclusion, reading) in every_conclusion() {
            let on_base = vec![check("test", conclusion.clone(), "https://base/test")];

            let failed = failed_on_watch(vec![red("test")], on_base);

            let (own, inherited) = if reading == Reading::Failed {
                (vec![], vec![red("test")])
            } else {
                (vec![red("test")], vec![])
            };
            assert_eq!(listed(&failed.own), listed(&own), "{conclusion:?}");
            assert_eq!(
                listed(&failed.inherited),
                listed(&inherited),
                "{conclusion:?} on the base branch commit"
            );
        }
    }

    #[test]
    fn with_no_base_branch_commit_the_base_is_never_read_and_every_red_check_is_the_branchs_own() {
        let mut github = InMemory::new()
            .listing(HEAD, vec![vec![red("test"), red("lint")]])
            .listing(BASE, vec![vec![red("test"), red("lint")]]);

        let failed = failed_of(watch_through(&mut github, HEAD, None).unwrap());

        assert_eq!(
            listed(&failed.own),
            ["test: https://head/test", "lint: https://head/lint"]
        );
        assert!(failed.inherited.is_empty());
        assert!(failed.on_base.is_empty());
        assert_eq!(github.reads_of(BASE), 0);
    }

    #[test]
    fn a_check_rerun_with_a_failed_check_that_is_no_actions_job_asks_for_nothing() {
        let mut github = InMemory::new();
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1), red("lint")],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        assert!(ci.is_none());
        assert!(github.reruns.is_empty());
        assert_eq!(
            github.steps,
            ["the failed checks on abcdef0 can't be re-run: not GitHub Actions jobs: lint"]
        );
    }

    #[test]
    fn a_check_rerun_excludes_known_failed_ids_missing_before_the_request() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![green("lint")],
                vec![job(red("test"), 11, 1), green("lint")],
                vec![job(red("test"), 11, 1), green("lint")],
                vec![job(green("test"), 12, 1), green("lint")],
            ],
        );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        assert!(
            matches!(ci, Some(Ci::Passed)),
            "the old failed attempt must not end the watch before the new attempt passes"
        );
    }

    #[test]
    fn a_check_rerun_waits_through_a_missing_failed_check_until_its_new_attempt_fails() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![job(red("test"), 11, 1), green("lint")],
                vec![green("lint")],
                vec![green("lint")],
                vec![job(pending("test"), 12, 1), green("lint")],
                vec![job(red("test"), 12, 1), green("lint")],
            ],
        );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        let failed = failed_of(ci.expect("a re-run to watch"));
        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
        assert!(!github.steps.contains(&"CI passed on abcdef0".to_string()));
    }

    #[test]
    fn a_check_rerun_does_not_pass_if_its_new_check_disappears_during_the_watch() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![job(red("test"), 11, 1), green("lint")],
                vec![job(pending("test"), 12, 1), green("lint")],
                vec![green("lint")],
            ],
        );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let result = rerun_through(&mut github, HEAD, Some(BASE), &failed);

        assert_eq!(
            result.err().expect("unverified CI").to_string(),
            "CI checks disappeared on abcdef0; cannot verify CI"
        );
    }

    #[test]
    fn a_check_rerun_uses_the_attempt_listed_before_the_request_when_failures_are_stale() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![job(red("test"), 21, 1), green("lint")],
                vec![job(green("test"), 21, 1), green("lint")],
                vec![job(green("test"), 21, 1), green("lint")],
                vec![job(pending("test"), 31, 1), green("lint")],
                vec![job(red("test"), 31, 1), green("lint")],
            ],
        );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        let failed = failed_of(ci.expect("a re-run newer than the request-time listing"));
        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
    }

    #[test]
    fn a_check_rerun_whose_check_never_returns_has_none_even_with_other_green_checks() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![job(red("test"), 11, 1), green("lint")],
                vec![green("lint")],
            ],
        );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        assert!(ci.is_none());
        assert_eq!(
            github.steps.last().map(String::as_str),
            Some("no re-run appeared on abcdef0 within 60s")
        );
    }

    #[test]
    fn a_partial_rerun_cannot_pass_when_a_refused_failed_check_disappears() {
        let mut github = InMemory::new().refusing(2).listing(
            HEAD,
            vec![
                vec![job(red("test"), 11, 1), job(red("lint"), 21, 2)],
                vec![job(pending("test"), 12, 1), job(red("lint"), 21, 2)],
                vec![job(pending("test"), 12, 1), job(red("lint"), 21, 2)],
                vec![job(green("test"), 12, 1)],
            ],
        );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1), job(red("lint"), 21, 2)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let result = rerun_through(&mut github, HEAD, None, &failed);

        assert!(
            !matches!(result, Ok(Some(Ci::Passed))),
            "the refused workflow's known failed lint check disappeared; CI cannot pass"
        );
    }

    #[test]
    fn a_check_rerun_asks_once_for_each_workflow_run() {
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![
                    job(red("test"), 11, 1),
                    job(red("lint"), 12, 1),
                    job(red("build"), 21, 2),
                ],
                vec![
                    job(green("test"), 31, 1),
                    job(green("lint"), 32, 1),
                    job(green("build"), 41, 2),
                ],
            ],
        );
        let failed = FailedChecks {
            own: vec![
                job(red("test"), 11, 1),
                job(red("build"), 21, 2),
                job(red("lint"), 12, 1),
            ],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        assert!(matches!(ci, Some(Ci::Passed)));
        let mut reruns = github.reruns.clone();
        reruns.sort_unstable();
        assert_eq!(reruns, [1, 2]);
        assert_eq!(
            github.steps,
            [
                "re-running the failed checks on abcdef0: test, build, lint",
                "CI passed on abcdef0",
            ]
        );
    }

    #[test]
    fn a_check_rerun_whose_first_request_is_refused_asks_no_more() {
        let mut github = InMemory::new().refusing(1).refusing(2).listing(
            HEAD,
            vec![vec![job(red("test"), 11, 1), job(red("build"), 21, 2)]],
        );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1), job(red("build"), 21, 2)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        assert!(matches!(ci, Some(Ci::Failed(_))));
        assert_eq!(github.reruns.len(), 1);
        let refused = github.reruns[0];
        assert_eq!(
            github.steps[1],
            format!(
                "the failed checks on abcdef0 were not re-run: \
                 gh: HTTP 403: workflow run {refused} cannot be re-run"
            )
        );
    }

    #[test]
    fn an_empty_listing_is_not_a_new_attempt_when_a_later_request_is_refused() {
        let mut github = InMemory::new().refusing(2).listing(
            HEAD,
            vec![
                vec![job(red("test"), 11, 1), job(red("lint"), 21, 2)],
                Vec::new(),
                vec![job(red("test"), 11, 1), job(green("lint"), 22, 2)],
                vec![job(pending("test"), 12, 1), job(green("lint"), 22, 2)],
                vec![job(green("test"), 12, 1), job(green("lint"), 22, 2)],
            ],
        );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1), job(red("lint"), 21, 2)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        assert!(matches!(ci, Some(Ci::Passed)));
        assert_eq!(github.reruns, [1, 2]);
    }

    #[test]
    fn a_refused_request_watches_pending_checks_and_classifies_inherited_failures() {
        let mut github = InMemory::new()
            .refusing(1)
            .listing(
                HEAD,
                vec![
                    vec![job(green("test"), 12, 1), job(pending("lint"), 22, 2)],
                    vec![job(green("test"), 12, 1), job(pending("lint"), 22, 2)],
                    vec![job(green("test"), 12, 1), job(red("lint"), 22, 2)],
                ],
            )
            .listing(BASE, vec![vec![red("lint")]]);
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        let failed = failed_of(ci.expect("current CI after refusal"));
        assert!(failed.own.is_empty());
        assert_eq!(listed(&failed.inherited), ["lint: https://head/lint"]);
        assert_eq!(listed(&failed.on_base), ["lint: https://head/lint"]);
        assert_eq!(github.reruns, [1]);
        assert!(
            github.steps.contains(
                &"CI on abcdef0: 1 still running out of 2 checks listed so far".to_string()
            )
        );
    }

    #[test]
    fn a_later_refusal_assesses_pending_and_red_checks_across_the_whole_head() {
        let mut github = InMemory::new().refusing(2).listing(
            HEAD,
            vec![
                vec![
                    job(red("test"), 11, 1),
                    job(red("lint"), 21, 2),
                    job(red("build"), 31, 3),
                ],
                vec![
                    job(green("test"), 12, 1),
                    job(green("lint"), 22, 2),
                    job(pending("build"), 32, 3),
                ],
                vec![
                    job(green("test"), 12, 1),
                    job(green("lint"), 22, 2),
                    job(pending("build"), 32, 3),
                ],
                vec![
                    job(green("test"), 12, 1),
                    job(green("lint"), 22, 2),
                    job(red("build"), 32, 3),
                ],
            ],
        );
        let failed = FailedChecks {
            own: vec![
                job(red("test"), 11, 1),
                job(red("lint"), 21, 2),
                job(red("build"), 31, 3),
            ],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        let failed = failed_of(ci.expect("current CI after refusal"));
        assert_eq!(listed(&failed.own), ["build: https://head/build"]);
        assert!(failed.inherited.is_empty());
        assert_eq!(github.reruns, [1, 2]);
        assert!(
            github.steps.contains(
                &"CI on abcdef0: 1 still running out of 3 checks listed so far".to_string()
            )
        );
    }

    #[test]
    fn interruption_stops_the_pending_watch_after_a_refused_request() {
        let mut github = InMemory::new()
            .refusing(1)
            .listing(HEAD, vec![vec![job(pending("test"), 12, 1)]])
            .interrupted_on_pause(1);
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let result = rerun_through(&mut github, HEAD, Some(BASE), &failed);

        assert_eq!(
            result.err().expect("interrupted watch").to_string(),
            "interrupted"
        );
        assert_eq!(github.reruns, [1]);
    }

    #[test]
    fn checks_disappearing_during_a_refusal_watch_cannot_count_as_green() {
        let mut github = InMemory::new()
            .refusing(1)
            .listing(HEAD, vec![vec![job(pending("test"), 12, 1)], Vec::new()]);
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let result = rerun_through(&mut github, HEAD, Some(BASE), &failed);

        assert_eq!(
            result.err().expect("unverified CI").to_string(),
            "CI checks disappeared on abcdef0; cannot verify CI"
        );
        assert_eq!(github.reruns, [1]);
    }

    #[test]
    fn a_check_rerun_whose_attempt_before_is_still_listed_after_the_grace_period_has_none() {
        let mut github = InMemory::new().listing(HEAD, vec![vec![job(red("test"), 11, 1)]]);
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        assert!(ci.is_none());
        assert_eq!(github.reruns, [1]);
        assert_eq!(
            github.steps.last().map(String::as_str),
            Some("no re-run appeared on abcdef0 within 60s")
        );
    }

    #[test]
    fn a_check_rerun_waits_for_an_inherited_failure_sharing_a_workflow_run_to_be_rerun_too() {
        let mut github = InMemory::new()
            .listing(
                HEAD,
                vec![
                    vec![job(red("test"), 11, 1), job(red("lint"), 12, 1)],
                    // The branch's own failure has a new attempt, but the
                    // Inherited failure's attempt before is still listed.
                    vec![job(pending("test"), 21, 1), job(red("lint"), 12, 1)],
                    vec![job(pending("test"), 21, 1), job(pending("lint"), 22, 1)],
                    vec![job(green("test"), 21, 1), job(red("lint"), 22, 1)],
                ],
            )
            .listing(
                BASE,
                vec![vec![check(
                    "lint",
                    Conclusion::Failure,
                    "https://base/lint",
                )]],
            );
        let failed = FailedChecks {
            own: vec![job(red("test"), 11, 1)],
            inherited: vec![job(red("lint"), 12, 1)],
            on_base: vec![check("lint", Conclusion::Failure, "https://base/lint")],
        };

        let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

        // Had the watch not waited for lint's old check run to go, it would
        // have watched the listing after as the new attempt, still running.
        let failed = failed_of(ci.expect("a re-run to watch"));
        assert!(failed.own.is_empty());
        assert_eq!(listed(&failed.inherited), ["lint: https://head/lint"]);
        assert_eq!(github.reruns, [1]);
        assert_eq!(
            github.steps,
            [
                "re-running the failed checks on abcdef0: test",
                "CI failed on abcdef0: lint",
            ]
        );
    }

    #[test]
    fn an_interrupt_during_a_pause_fails_the_watch() {
        let mut github = InMemory::new()
            .listing(HEAD, vec![vec![pending("test")]])
            .interrupted_on_pause(1);

        let error = watch_through(&mut github, HEAD, Some(BASE))
            .err()
            .expect("the watch to fail");

        assert_eq!(error.to_string(), "interrupted");
    }
}
