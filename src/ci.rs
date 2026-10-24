//! Watching CI on the Issue branch's head commit.
//!
//! What it reads of GitHub and asks of it, how it waits, and its progress
//! lines all go through [`Outside`]: [`OnGitHub`] does each through `gh`, the
//! poll module and the progress lines; `InMemory`, in tests, from a script,
//! recording each re-run request and progress line.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::github::{ActionsJob, Check, Conclusion, GitHub, JobRun};
use crate::issue::IssueUrl;
use crate::poll;
use crate::process::Interruption;
use crate::progress;

/// How CI on a commit ended.
pub enum Ci {
    /// No check or status appeared within the grace period: no CI, which
    /// counts as passing.
    Absent,
    Passed,
    /// Only cancelled checks were red, and origin now has a different head.
    Superseded,
    /// Red, with the checks that failed.
    Failed(FailedChecks),
}

impl Ci {
    /// Whether it is red on a check that is the branch's own to fix.
    fn has_own_failures(&self) -> bool {
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

impl FailedChecks {
    /// Whether the branch's own failures, at least one, are all Abandoned
    /// jobs, whatever the Inherited failures are.
    fn own_failures_all_abandoned(&self) -> bool {
        !self.own.is_empty()
            && self
                .own
                .iter()
                .all(|check| check.job.is_some_and(|job| job.abandoned))
    }
}

/// CI watch for one issue's complete Delivery. A new Delivery, including a
/// Continuation, starts a new watch; rounds, Base fixes and refused
/// Self-merges keep this one and its full-head re-run allowances.
pub struct Watch<'a> {
    issue: &'a IssueUrl,
    handed_to_repair: Option<(String, FailedChecks)>,
    consumed: HashSet<String>,
    abandoned_jobs_rerun: HashSet<String>,
}

impl<'a> Watch<'a> {
    pub fn new(issue: &'a IssueUrl) -> Self {
        Self {
            issue,
            handed_to_repair: None,
            consumed: HashSet::new(),
            abandoned_jobs_rerun: HashSet::new(),
        }
    }

    /// Record the full head and failed checks handed to a CI-fix Repair,
    /// only after the session and its pull request postcheck succeeded.
    /// Keep them until assessment, after the next successful Base merge,
    /// even if conflict Repairs intervene.
    pub fn after_repair(&mut self, head: String, failed: FailedChecks) {
        self.handed_to_repair = Some((head, failed));
    }

    /// Observe CI against this assessment's selected Base commit, including
    /// eligible Abandoned job re-runs and Check re-runs. With no Base commit
    /// every failure is the branch's own. An unchanged CI-fix Repair head
    /// consumes its Check re-run before requesting it, unless a moved origin
    /// superseded its cancellations; later own failures on that head fail as
    /// the same Declined CI fix. Other heads are observed afresh.
    /// `actionable` is borrowed only for this assessment and gates GitHub
    /// observations before and after each read.
    pub fn assess(
        &mut self,
        head: &str,
        base_commit: Option<&str>,
        actionable: &mut dyn FnMut() -> Result<()>,
        head_moved: &mut dyn FnMut(&str) -> Result<bool>,
    ) -> Result<Ci> {
        self.assess_through(
            &mut OnGitHub {
                issue: self.issue,
                actionable,
                head_moved,
            },
            head,
            base_commit,
        )
    }

    fn assess_through(
        &mut self,
        outside: &mut impl Outside,
        head: &str,
        base_commit: Option<&str>,
    ) -> Result<Ci> {
        let mut unchanged = self
            .handed_to_repair
            .take()
            .filter(|(handed, _)| handed == head);
        // Another writer may push after this round's reconciliation. Read
        // current CI before spending an unchanged Repair's re-run allowance.
        if unchanged.is_some() && outside.head_moved(head)? {
            match watch_to_end(outside, head, base_commit, None)? {
                Ci::Failed(failed) if !failed.own.is_empty() => {
                    unchanged = Some((head.to_string(), failed));
                }
                ci => return Ok(ci),
            }
        }
        let ci = match unchanged {
            Some((_, failed)) if self.consumed.insert(head.to_string()) => {
                rerun_through(outside, head, base_commit, &failed)?
            }
            _ => {
                let watched = watch_through(outside, head, base_commit)?;
                Some(self.rerun_abandoned_jobs_once(outside, head, base_commit, watched)?)
            }
        };
        match ci {
            Some(ci) if !(self.consumed.contains(head) && ci.has_own_failures()) => Ok(ci),
            _ => bail!(
                "CI red on {} and the Repair found nothing to fix on the branch",
                short(head)
            ),
        }
    }

    /// A refused or unseen Abandoned job re-run retains the original red
    /// assessment. Its separate allowance is spent before the request, and
    /// is never available after a Check re-run has been consumed.
    fn rerun_abandoned_jobs_once(
        &mut self,
        outside: &mut impl Outside,
        head: &str,
        base_commit: Option<&str>,
        ci: Ci,
    ) -> Result<Ci> {
        let Ci::Failed(failed) = &ci else {
            return Ok(ci);
        };
        if !failed.own_failures_all_abandoned()
            || self.consumed.contains(head)
            || !self.abandoned_jobs_rerun.insert(head.to_string())
        {
            return Ok(ci);
        }
        Ok(rerun_abandoned_through(outside, head, base_commit, failed)?.unwrap_or(ci))
    }
}

/// What the CI watch reads of GitHub and asks of it, how it waits, and
/// where its progress lines go.
trait Outside {
    /// A point the grace period ends at, from when it was started.
    type Deadline;
    /// The checks and statuses on the commit `sha`.
    fn checks_on(&mut self, sha: &str) -> Result<Vec<Check>>;
    /// Whether the Issue branch on origin no longer points at `sha`.
    fn head_moved(&mut self, sha: &str) -> Result<bool>;
    /// The GitHub Actions job whose check run is `check_run`, with its
    /// steps, or why it could not be read. Fails only if the Run is not to
    /// go on.
    fn job_run(&mut self, check_run: u64) -> Result<Result<JobRun>>;
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
    head_moved: &'a mut dyn FnMut(&str) -> Result<bool>,
}

impl Outside for OnGitHub<'_> {
    type Deadline = Instant;

    fn checks_on(&mut self, sha: &str) -> Result<Vec<Check>> {
        (self.actionable)()?;
        let checks = GitHub::new().checks_on(self.issue, sha)?;
        (self.actionable)()?;
        Ok(checks)
    }

    fn job_run(&mut self, check_run: u64) -> Result<Result<JobRun>> {
        (self.actionable)()?;
        let job_run = GitHub::new().job_run(self.issue, check_run);
        Interruption::Ordinary.check()?;
        (self.actionable)()?;
        Ok(job_run)
    }

    fn head_moved(&mut self, sha: &str) -> Result<bool> {
        (self.actionable)()?;
        let moved = (self.head_moved)(sha)?;
        (self.actionable)()?;
        Ok(moved)
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

/// Wait for checks to appear, then observe them to completion.
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

/// Request the Check re-run, observing fresh checks even after refusal.
/// Successful requests require new attempts; non-Actions checks or unseen
/// attempts return `None`, and read errors cannot establish green CI.
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
    let off_commit = failed
        .own
        .iter()
        .filter(|check| scoped_workflow_run(check).is_none());
    let off_commit = check_names(off_commit);
    if !off_commit.is_empty() {
        outside.step(format!(
            "the failed checks on {short} can't be re-run: no workflow run on {short} for: {off_commit}"
        ));
        return Ok(None);
    }
    outside.step(format!(
        "re-running the failed checks on {short}: {}",
        check_names(&failed.own)
    ));
    let rerun_checks = request_reruns(outside, sha, failed)?;
    if rerun_checks.requested.is_empty() {
        return watch_to_end(outside, sha, base_commit, Some(&rerun_checks)).map(Some);
    }
    watch_new_attempts(outside, sha, base_commit, &rerun_checks)
}

/// Request the Abandoned job re-run with the Check re-run's attempt guard,
/// returning `None` after any refusal or if no new attempt appears.
fn rerun_abandoned_through(
    outside: &mut impl Outside,
    sha: &str,
    base_commit: Option<&str>,
    failed: &FailedChecks,
) -> Result<Option<Ci>> {
    let short = short(sha);
    let off_commit = failed
        .own
        .iter()
        .filter(|check| scoped_workflow_run(check).is_none());
    let off_commit = check_names(off_commit);
    if !off_commit.is_empty() {
        outside.step(format!(
            "the failed checks on {short} can't be re-run: no workflow run on {short} for: {off_commit}"
        ));
        return Ok(None);
    }
    let (lost, them) = match failed.own.len() {
        1 => ("its runner", "it"),
        _ => ("their runners", "them"),
    };
    outside.step(format!(
        "{} lost {lost} on {short}; re-running {them}",
        check_names(&failed.own),
    ));
    let rerun_checks = request_reruns(outside, sha, failed)?;
    if !rerun_checks.all_requested {
        return Ok(None);
    }
    watch_new_attempts(outside, sha, base_commit, &rerun_checks)
}

/// Ask GitHub to re-run the failed jobs of each workflow run the branch's
/// own failures in `failed`, all GitHub Actions jobs, belong to, stopping at
/// the first refusal, which is reported. Returns the checks whose new
/// attempts the watch must see.
fn request_reruns<'a>(
    outside: &mut impl Outside,
    sha: &str,
    failed: &'a FailedChecks,
) -> Result<RerunChecks<'a>> {
    let mut workflow_runs: Vec<u64> = failed.own.iter().filter_map(scoped_workflow_run).collect();
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
                "the failed checks on {} were not re-run: {error:#}",
                short(sha)
            ));
            break;
        }
        requested.push(*workflow_run);
    }
    // GitHub re-runs every failed job of a workflow run, so an Inherited
    // failure that shares one with the branch's own gets a new attempt too.
    Ok(RerunChecks {
        checks: failed.own.iter().chain(&failed.inherited).collect(),
        previous_attempt,
        all_requested: requested.len() == workflow_runs.len()
            && failed.own.iter().all(|check| scoped_workflow_run(check).is_some()),
        requested,
    })
}

/// Wait up to the grace period for the new attempts of `rerun_checks` to be
/// listed on `sha`, then watch CI there to the end: `None`, told in a
/// progress line, if none appeared.
fn watch_new_attempts(
    outside: &mut impl Outside,
    sha: &str,
    base_commit: Option<&str>,
    rerun_checks: &RerunChecks<'_>,
) -> Result<Option<Ci>> {
    let (grace, deadline) = outside.start_grace();
    let new_attempt = within(outside, deadline, |outside| {
        let checks = read_checks(outside, sha)?;
        Ok(rerun_checks.new_attempts_listed(&checks).then_some(()))
    })?;
    if new_attempt.is_none() {
        outside.step(format!(
            "no re-run appeared on {} within {}s",
            short(sha),
            grace.as_secs()
        ));
        return Ok(None);
    }
    watch_to_end(outside, sha, base_commit, Some(rerun_checks)).map(Some)
}

/// Known failed checks that must stay listed during a Check re-run or a
/// re-run of Abandoned jobs. Those in successfully requested workflow runs
/// must also have new attempts.
struct RerunChecks<'a> {
    checks: Vec<&'a Check>,
    previous_attempt: Vec<u64>,
    requested: Vec<u64>,
    /// Whether every workflow run asked for was requested, none refused.
    all_requested: bool,
}

impl RerunChecks<'_> {
    fn is_requested(&self, check: &Check) -> bool {
        scoped_workflow_run(check).is_some_and(|run| self.requested.contains(&run))
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
        // checks_on lists only the latest run of each workflow. Its jobs
        // replace the old run's requirements, even if job names changed.
        if expected.workflow_run.is_some_and(|old| {
            checks.iter().any(|check| {
                check
                    .workflow_run
                    .is_some_and(|new| new.workflow_id == old.workflow_id && new.id != old.id)
            })
        }) {
            return true;
        }
        checks.iter().any(|check| {
            check.name == expected.name
                && (!self.is_requested(expected)
                    || check.job.is_some_and(|job| {
                        scoped_workflow_run(check) == scoped_workflow_run(expected)
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
    let mut waiting_line = poll::WaitingLine::new();
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
        if waiting_line.should_report(counts) {
            outside.step(format!(
                "CI on {short}: {pending} still running out of {} checks listed so far",
                checks.len()
            ));
        }
        Ok(None)
    })?;
    let mut failed: Vec<Check> = checks.into_iter().filter(is_failed).collect();
    if failed.is_empty() {
        outside.step(format!("CI passed on {short}"));
        return Ok(Ci::Passed);
    }
    if failed
        .iter()
        .any(|check| check.conclusion == Conclusion::Cancelled)
        && outside.head_moved(sha)?
    {
        failed.retain(|check| check.conclusion != Conclusion::Cancelled);
        if failed.is_empty() {
            outside.step(format!("CI on {short} was superseded by a newer push"));
            return Ok(Ci::Superseded);
        }
    }
    outside.step(format!("CI failed on {short}: {}", check_names(&failed)));
    let on_base = match base_commit {
        Some(base_commit) => read_checks(outside, base_commit)?,
        None => Vec::new(),
    };
    let mut failed = split(failed, on_base);
    mark_abandoned_jobs(outside, sha, &mut failed.own)?;
    Ok(Ci::Failed(failed))
}

/// Mark which of `own`, the branch's own failures on `sha`, are Abandoned
/// jobs. Only every one being abandoned changes anything, so nothing is read
/// unless each is a GitHub Actions job concluded `failure`, and reading stops
/// at the first that is not abandoned. A job whose steps cannot be read is
/// not one, told in a progress line, so that red CI is taken as it was.
fn mark_abandoned_jobs(outside: &mut impl Outside, sha: &str, own: &mut [Check]) -> Result<()> {
    let may_all_be_abandoned = own
        .iter()
        .all(|check| check.job.is_some() && check.conclusion == Conclusion::Failure);
    if !may_all_be_abandoned {
        return Ok(());
    }
    for check in own {
        let Some(job) = &mut check.job else {
            unreachable!("every own failure is a job")
        };
        match outside.job_run(job.check_run)? {
            Ok(job_run) => job.abandoned = is_abandoned(&job_run),
            Err(error) => outside.step(format!(
                "couldn't read the steps of {} on {}: {error:#}",
                check.name,
                short(sha)
            )),
        }
        if !job.abandoned {
            break;
        }
    }
    Ok(())
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

/// The workflow run a re-run of `check` may target: the run the check run
/// belongs to on the watched commit, as the commit's workflow-run listing
/// reports it. The job's run id comes from the check run's details URL,
/// which can name any run in the repository, so a run the listing does not
/// confirm for this commit is never re-run.
fn scoped_workflow_run(check: &Check) -> Option<u64> {
    let job = check.job?;
    let run = check.workflow_run?;
    (run.id == job.workflow_run).then_some(run.id)
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

/// Whether `job` is an Abandoned job: GitHub ended it as a failure while
/// none of its steps failed and one had not completed, as when its runner is
/// lost. A fact GitHub reports, so no annotation or log text is read.
fn is_abandoned(job: &JobRun) -> bool {
    job.conclusion == Conclusion::Failure
        && !job.steps.contains(&Conclusion::Failure)
        && job.steps.contains(&Conclusion::Unfinished)
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

    use anyhow::{Result, anyhow, bail};

    use super::Outside;
    use crate::github::{Check, Conclusion, JobRun};

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
        job_runs: Vec<(u64, JobRun)>,
        failing_job_reads: Vec<u64>,
        head_moved: bool,
        /// The check run of each job read, in order.
        pub job_reads: Vec<u64>,
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
                job_runs: Vec::new(),
                failing_job_reads: Vec::new(),
                head_moved: false,
                job_reads: Vec::new(),
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

        pub fn moved_head(mut self) -> Self {
            self.head_moved = true;
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

        /// Report the job whose check run is `check_run` as `job_run`. A job
        /// not scripted failed on a failed step.
        pub fn job_run(mut self, check_run: u64, job_run: JobRun) -> Self {
            self.job_runs.push((check_run, job_run));
            self
        }

        /// Fail each read of the job whose check run is `check_run`.
        pub fn job_read_failing(mut self, check_run: u64) -> Self {
            self.failing_job_reads.push(check_run);
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

        fn job_run(&mut self, check_run: u64) -> Result<Result<JobRun>> {
            self.job_reads.push(check_run);
            if self.failing_job_reads.contains(&check_run) {
                return Ok(Err(anyhow!("gh: HTTP 403: Resource not accessible")));
            }
            let scripted = self.job_runs.iter().find(|(id, _)| *id == check_run);
            Ok(Ok(scripted.map_or_else(
                || JobRun {
                    conclusion: Conclusion::Failure,
                    steps: vec![Conclusion::Success, Conclusion::Failure],
                },
                |(_, job_run)| job_run.clone(),
            )))
        }

        fn head_moved(&mut self, _: &str) -> Result<bool> {
            Ok(self.head_moved)
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
    use crate::github::WorkflowRun;

    const HEAD: &str = "abcdef0123456789";
    const BASE: &str = "0123456789abcdef";

    #[test]
    fn spec_regression_queued_replacement_workflow_outlives_check_rerun_grace() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut old = job(red("test"), 1, 900);
        old.workflow_run = Some(WorkflowRun {
            id: 900,
            workflow_id: 10,
        });
        let mut latest = job(green("renamed-test"), 2, 901);
        latest.workflow_run = Some(WorkflowRun {
            id: 901,
            workflow_id: 10,
        });
        let mut queued = check(
            "CI",
            Conclusion::Unfinished,
            "https://github.com/acme/widgets/actions/runs/901",
        );
        assert!(queued.job.is_none());
        queued.workflow_run = latest.workflow_run;
        let mut watch = Watch::new(&issue);
        watch.after_repair(
            HEAD.to_string(),
            FailedChecks {
                own: vec![old.clone()],
                inherited: Vec::new(),
                on_base: Vec::new(),
            },
        );
        // The workflow-run listing knows that 901 supersedes 900, but
        // checks_on's placeholder keeps its identity before jobs exist.
        // Waiting for a runner can exceed the Check re-run attempt grace.
        let mut github = InMemory::new().listing(
            HEAD,
            vec![
                vec![old],
                vec![queued.clone()],
                vec![queued.clone()],
                vec![queued.clone()],
                vec![queued],
                vec![latest],
            ],
        );

        let ci = watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap();

        assert!(matches!(ci, Ci::Passed));
        assert_eq!(github.reruns, [900]);
    }

    #[test]
    fn spec_regression_moved_cancelled_head_after_repair_spends_no_check_rerun() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let cancelled = job(
            check("test", Conclusion::Cancelled, "https://head/test"),
            1,
            900,
        );
        let mut watch = Watch::new(&issue);
        // This cancellation was a valid failure before origin moved. The
        // CI-fix Repair completed without changing HEAD; origin moves before
        // the next assessment starts.
        watch.after_repair(
            HEAD.to_string(),
            FailedChecks {
                own: vec![cancelled.clone()],
                inherited: Vec::new(),
                on_base: Vec::new(),
            },
        );
        let mut github = InMemory::new()
            .moved_head()
            .listing(HEAD, vec![vec![cancelled]]);

        let result = watch.assess_through(&mut github, HEAD, Some(BASE));

        assert!(
            github.reruns.is_empty(),
            "unexpected re-runs: {:?}",
            github.reruns
        );
        assert!(!watch.consumed.contains(HEAD));
        assert!(matches!(result.unwrap(), Ci::Superseded));
    }

    #[test]
    fn spec_regression_latest_workflow_run_can_supersede_an_active_check_rerun() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut old = job(red("test"), 1, 900);
        old.workflow_run = Some(WorkflowRun {
            id: 900,
            workflow_id: 10,
        });
        let mut latest = job(green("test"), 2, 901);
        latest.workflow_run = Some(WorkflowRun {
            id: 901,
            workflow_id: 10,
        });
        let mut watch = Watch::new(&issue);
        watch.after_repair(
            HEAD.to_string(),
            FailedChecks {
                own: vec![old.clone()],
                inherited: Vec::new(),
                on_base: Vec::new(),
            },
        );
        // Run 901 starts after the re-run request for 900. checks_on now
        // removes all of run 900's checks and returns only latest's success.
        let mut github = InMemory::new().listing(HEAD, vec![vec![old], vec![latest]]);

        let ci = watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap();

        assert!(matches!(ci, Ci::Passed));
        assert_eq!(github.reruns, [900]);
    }

    #[test]
    fn synthetic_latest_workflow_success_supersedes_an_active_check_rerun() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut old = job(red("test"), 1, 900);
        old.workflow_run = Some(WorkflowRun {
            id: 900,
            workflow_id: 10,
        });
        // exclude_superseded_runs emits this shape when run 901 completed
        // successfully but its jobs were absent from the preceding read.
        let mut latest = check(
            "CI",
            Conclusion::Success,
            "https://github.com/acme/widgets/actions/runs/901",
        );
        assert!(latest.job.is_none());
        latest.workflow_run = Some(WorkflowRun {
            id: 901,
            workflow_id: 10,
        });
        let mut watch = Watch::new(&issue);
        watch.after_repair(
            HEAD.to_string(),
            FailedChecks {
                own: vec![old.clone()],
                inherited: Vec::new(),
                on_base: Vec::new(),
            },
        );
        let mut github = InMemory::new().listing(HEAD, vec![vec![old], vec![latest]]);

        let ci = watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap();

        assert!(matches!(ci, Ci::Passed));
        assert_eq!(github.reruns, [900]);
    }

    #[test]
    fn a_moved_head_supersedes_only_cancelled_checks_and_spends_no_rerun() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let cancelled = job(
            check("test", Conclusion::Cancelled, "https://head/test"),
            1,
            900,
        );
        let mut watch = Watch::new(&issue);
        let mut github = InMemory::new()
            .moved_head()
            .listing(HEAD, vec![vec![cancelled.clone()]]);
        assert!(matches!(
            watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap(),
            Ci::Superseded
        ));
        assert!(github.reruns.is_empty());

        let mut github = InMemory::new()
            .moved_head()
            .listing(HEAD, vec![vec![cancelled.clone(), red("lint")]]);
        let failed = failed_of(watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap());
        assert_eq!(listed(&failed.own), ["lint: https://head/lint"]);
        assert!(github.reruns.is_empty());

        let mut github = InMemory::new().listing(HEAD, vec![vec![cancelled]]);
        let failed = failed_of(watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap());
        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
        assert!(github.reruns.is_empty());
    }

    #[test]
    fn assessment_re_runs_an_unchanged_ci_fix_head_from_a_new_attempt() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut watch = Watch::new(&issue);
        let old = job(red("test"), 1, 900);
        let new = job(green("test"), 2, 900);
        let mut github = InMemory::new().listing(
            HEAD,
            vec![vec![old.clone()], vec![old.clone()], vec![old], vec![new]],
        );

        let failed = failed_of(watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap());
        assert!(github.reruns.is_empty());
        watch.after_repair(HEAD.to_string(), failed);

        assert!(matches!(
            watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap(),
            Ci::Passed
        ));
        assert_eq!(github.reruns, [900]);
        assert_eq!(github.reads_of(HEAD), 5);
    }

    #[test]
    fn a_consumed_head_is_reassessed_against_the_current_base_after_another_head() {
        for outcome in ["own", "green", "inherited"] {
            let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
            let mut watch = Watch::new(&issue);
            let old = job(red("test"), 1, 900);
            let new = job(green("test"), 2, 900);
            let revisited = if outcome == "green" {
                new.clone()
            } else {
                job(red("test"), 3, 900)
            };
            let mut github = InMemory::new()
                .listing(
                    HEAD,
                    vec![
                        vec![old.clone()],
                        vec![old.clone()],
                        vec![old],
                        vec![new.clone()],
                        vec![new],
                        vec![revisited],
                    ],
                )
                .listing("other-head", vec![vec![green("test")]])
                .listing(BASE, vec![vec![green("test")]])
                .listing(
                    "current-base",
                    vec![if outcome == "inherited" {
                        vec![red("test")]
                    } else {
                        vec![green("test")]
                    }],
                )
                // Even an Abandoned job cannot receive a re-run after consumption.
                .job_run(3, abandoned_job_run());
            let failed = failed_of(watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap());
            watch.after_repair(HEAD.to_string(), failed);
            assert!(matches!(
                watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap(),
                Ci::Passed
            ));
            assert!(matches!(
                watch
                    .assess_through(&mut github, "other-head", None)
                    .unwrap(),
                Ci::Passed
            ));

            let result = watch.assess_through(&mut github, HEAD, Some("current-base"));

            match outcome {
                "own" => assert_eq!(
                    result.err().unwrap().to_string(),
                    "CI red on abcdef0 and the Repair found nothing to fix on the branch"
                ),
                "green" => assert!(matches!(result.unwrap(), Ci::Passed)),
                _ => {
                    let failed = failed_of(result.unwrap());
                    assert!(failed.own.is_empty());
                    assert_eq!(listed(&failed.inherited), ["test: https://head/test"]);
                    assert_eq!(listed(&failed.on_base), ["test: https://head/test"]);
                }
            }
            assert_eq!(github.reruns, [900]);
            assert_eq!(github.reads_of(HEAD), 7);
            assert_eq!(
                github.reads_of("current-base"),
                usize::from(outcome != "green")
            );
        }
    }

    #[test]
    fn distinct_full_heads_with_one_short_prefix_each_get_both_allowances_in_each_delivery() {
        let first = "0123456789abcdef0123456789abcdef00000001";
        let second = "0123456789abcdef0123456789abcdef00000002";
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        for _delivery in 0..2 {
            let mut watch = Watch::new(&issue);
            let mut github = InMemory::new();
            for (head, workflow) in [(first, 900), (second, 901)] {
                let old = job(red("wsl"), 1, workflow);
                let red_again = job(red("wsl"), 2, workflow);
                let passed = job(green("wsl"), 3, workflow);
                github = github
                    .listing(
                        head,
                        vec![
                            vec![old.clone()],
                            vec![old.clone()],
                            vec![old],
                            vec![red_again.clone()],
                            vec![red_again.clone()],
                            vec![red_again],
                            vec![passed],
                        ],
                    )
                    .job_run(1, abandoned_job_run())
                    .job_run(2, abandoned_job_run());

                let failed = failed_of(watch.assess_through(&mut github, head, None).unwrap());
                assert_eq!(failed.own[0].job.unwrap().check_run, 2);
                watch.after_repair(head.to_string(), failed);
                assert!(matches!(
                    watch.assess_through(&mut github, head, None).unwrap(),
                    Ci::Passed
                ));
            }
            assert_eq!(github.reruns, [900, 900, 901, 901]);
        }
    }

    #[test]
    fn a_changed_full_head_discards_the_pending_repair_without_consuming_either_heads_check_rerun()
    {
        let first = "0123456789abcdef0123456789abcdef00000001";
        let second = "0123456789abcdef0123456789abcdef00000002";
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut watch = Watch::new(&issue);
        let mut github = InMemory::new()
            .listing(first, vec![vec![job(red("test"), 1, 900)]])
            .listing(second, vec![vec![job(red("lint"), 2, 901)]]);
        let failed = failed_of(watch.assess_through(&mut github, first, None).unwrap());
        watch.after_repair(first.to_string(), failed);

        let failed = failed_of(watch.assess_through(&mut github, second, None).unwrap());
        assert_eq!(listed(&failed.own), ["lint: https://head/lint"]);
        // Returning to the old head must not use the now stale Repair fact.
        let failed = failed_of(watch.assess_through(&mut github, first, None).unwrap());
        assert_eq!(listed(&failed.own), ["test: https://head/test"]);
        assert!(github.reruns.is_empty());
    }

    #[test]
    fn both_allowances_are_consumed_before_request_preparation_can_fail() {
        for abandoned in [false, true] {
            let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
            let mut watch = Watch::new(&issue);
            let old = job(red("wsl"), 1, 900);
            let mut github = InMemory::new()
                .listing(HEAD, vec![vec![old.clone()]])
                .error_on(HEAD, 3, "cannot read checks before request");
            if abandoned {
                github = github.job_run(1, abandoned_job_run());
            } else {
                let failed = failed_of(watch.assess_through(&mut github, HEAD, None).unwrap());
                watch.after_repair(HEAD.to_string(), failed);
            }
            let error = watch.assess_through(&mut github, HEAD, None).err().unwrap();
            assert_eq!(error.to_string(), "cannot read checks before request");
            assert!(github.reruns.is_empty());

            let mut github = InMemory::new()
                .listing(HEAD, vec![vec![old]])
                .job_run(1, abandoned_job_run());
            let result = watch.assess_through(&mut github, HEAD, None);
            if abandoned {
                assert_eq!(
                    listed(&failed_of(result.unwrap()).own),
                    ["wsl: https://head/wsl"]
                );
            } else {
                assert_eq!(
                    result.err().unwrap().to_string(),
                    "CI red on abcdef0 and the Repair found nothing to fix on the branch"
                );
            }
            assert!(github.reruns.is_empty());
        }
    }

    #[test]
    fn check_rerun_red_or_impossible_declines_but_inherited_only_does_not() {
        for outcome in ["own", "non-actions", "missing", "inherited"] {
            let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
            let mut watch = Watch::new(&issue);
            let old = if outcome == "non-actions" {
                red("test")
            } else {
                job(red("test"), 1, 900)
            };
            let new = if outcome == "missing" {
                old.clone()
            } else {
                job(red("test"), 2, 900)
            };
            let mut github = InMemory::new()
                .listing(
                    HEAD,
                    vec![vec![old.clone()], vec![old.clone()], vec![old], vec![new]],
                )
                .listing(
                    BASE,
                    vec![
                        vec![],
                        if outcome == "inherited" {
                            vec![red("test")]
                        } else {
                            vec![]
                        },
                    ],
                );
            let failed = failed_of(watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap());
            watch.after_repair(HEAD.to_string(), failed);

            let result = watch.assess_through(&mut github, HEAD, Some(BASE));

            if outcome == "inherited" {
                let failed = failed_of(result.unwrap());
                assert!(failed.own.is_empty());
                assert_eq!(listed(&failed.inherited), ["test: https://head/test"]);
            } else {
                assert_eq!(
                    result.err().unwrap().to_string(),
                    "CI red on abcdef0 and the Repair found nothing to fix on the branch"
                );
            }
            assert_eq!(github.reruns.len(), usize::from(outcome != "non-actions"));
        }
    }

    fn check(name: &str, conclusion: Conclusion, url: &str) -> Check {
        Check {
            name: name.to_string(),
            conclusion,
            url: Some(url.to_string()),
            job: None,
            workflow_run: None,
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
    /// the workflow run `workflow_run`, as the watched commit's workflow-run
    /// listing reports it.
    fn job(check: Check, check_run: u64, workflow_run: u64) -> Check {
        Check {
            job: Some(ActionsJob {
                check_run,
                workflow_run,
                abandoned: false,
            }),
            workflow_run: Some(WorkflowRun {
                id: workflow_run,
                workflow_id: workflow_run,
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
            Ci::Superseded => panic!("CI was Superseded, not Failed"),
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
                Ci::Superseded => panic!("the Issue branch has not moved"),
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
    fn a_check_rerun_never_reruns_a_workflow_run_off_the_watched_commit() {
        let mut without_listing = job(red("test"), 11, 999);
        without_listing.workflow_run = None;
        let mut mismatched = job(red("test"), 11, 999);
        mismatched.workflow_run = Some(WorkflowRun {
            id: 900,
            workflow_id: 10,
        });
        for spoofed in [without_listing, mismatched] {
            let mut github =
                InMemory::new().listing(HEAD, vec![vec![spoofed.clone()]]);
            let failed = FailedChecks {
                own: vec![spoofed],
                inherited: Vec::new(),
                on_base: Vec::new(),
            };

            let ci = rerun_through(&mut github, HEAD, Some(BASE), &failed).unwrap();

            assert!(ci.is_none());
            assert!(
                github.reruns.is_empty(),
                "unexpected re-runs: {:?}",
                github.reruns
            );
        }
    }

    #[test]
    fn an_abandoned_job_rerun_never_reruns_a_workflow_run_off_the_watched_commit() {
        let mut spoofed = job(red("wsl"), 1, 999);
        spoofed.workflow_run = None;
        let mut github = InMemory::new().listing(HEAD, vec![vec![spoofed.clone()]]);
        let failed = FailedChecks {
            own: vec![spoofed],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_abandoned_through(&mut github, HEAD, None, &failed).unwrap();

        assert!(ci.is_none());
        assert!(
            github.reruns.is_empty(),
            "unexpected re-runs: {:?}",
            github.reruns
        );
    }

    #[test]
    fn an_abandoned_job_rerun_off_the_watched_commit_says_so_before_declining() {
        let mut spoofed = job(red("wsl"), 1, 999);
        spoofed.workflow_run = None;
        let mut github = InMemory::new().listing(HEAD, vec![vec![spoofed.clone()]]);
        let failed = FailedChecks {
            own: vec![spoofed],
            inherited: Vec::new(),
            on_base: Vec::new(),
        };

        let ci = rerun_abandoned_through(&mut github, HEAD, None, &failed).unwrap();

        assert!(ci.is_none());
        assert!(
            github.reruns.is_empty(),
            "unexpected re-runs: {:?}",
            github.reruns
        );
        assert!(
            github.steps.iter().any(|line| line.contains("no workflow run")),
            "expected an off-commit decline line, got: {:?}",
            github.steps
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

    /// An Abandoned job's run, as GitHub ends one whose runner was lost.
    fn abandoned_job_run() -> JobRun {
        JobRun {
            conclusion: Conclusion::Failure,
            steps: vec![Conclusion::Success, Conclusion::Unfinished],
        }
    }

    #[test]
    fn a_watch_reads_steps_only_while_every_own_failure_may_yet_be_an_abandoned_job() {
        let lost = job(red("wsl"), 1, 900);
        let mut github = InMemory::new()
            .listing(
                HEAD,
                vec![vec![
                    lost.clone(),
                    job(red("test"), 2, 900),
                    job(red("docs"), 3, 901),
                    job(green("build"), 4, 900),
                ]],
            )
            .job_run(1, abandoned_job_run());

        let failed = failed_of(watch_through(&mut github, HEAD, None).unwrap());

        // `test` failed on a step, so `docs` can change nothing.
        assert_eq!(github.job_reads, [1, 2]);
        assert!(!failed.own_failures_all_abandoned());

        // A failure that is no job concluded `failure` can't be re-run so.
        for other in [
            red("status"),
            job(
                check("lint", Conclusion::Cancelled, "https://head/lint"),
                3,
                901,
            ),
        ] {
            let mut github = InMemory::new()
                .listing(HEAD, vec![vec![lost.clone(), other]])
                .job_run(1, abandoned_job_run());

            let failed = failed_of(watch_through(&mut github, HEAD, None).unwrap());

            assert!(github.job_reads.is_empty());
            assert!(!failed.own_failures_all_abandoned());
        }
    }

    #[test]
    fn a_job_whose_steps_cannot_be_read_is_not_abandoned_and_ci_stays_red() {
        let mut github = InMemory::new()
            .listing(HEAD, vec![vec![job(red("wsl"), 1, 900)]])
            .job_read_failing(1);

        let failed = failed_of(watch_through(&mut github, HEAD, None).unwrap());

        assert_eq!(listed(&failed.own), ["wsl: https://head/wsl"]);
        assert!(!failed.own_failures_all_abandoned());
        assert_eq!(
            github.steps.last().map(String::as_str),
            Some(
                "couldn't read the steps of wsl on abcdef0: gh: HTTP 403: Resource not accessible"
            )
        );
    }

    #[test]
    fn own_failures_are_only_abandoned_jobs_when_each_is_one_whatever_is_inherited() {
        let mut github = InMemory::new()
            .listing(
                HEAD,
                vec![vec![job(red("wsl"), 1, 900), job(red("lint"), 2, 901)]],
            )
            .listing(BASE, vec![vec![red("lint")]])
            .job_run(1, abandoned_job_run());

        let failed = failed_of(watch_through(&mut github, HEAD, Some(BASE)).unwrap());

        assert!(failed.own_failures_all_abandoned());
        assert_eq!(github.job_reads, [1]);
        assert!(
            !failed_on_watch(vec![red("lint")], vec![red("lint")]).own_failures_all_abandoned()
        );
    }

    #[test]
    fn assessment_re_runs_abandoned_jobs_from_new_attempts_and_remembers_a_revisited_head() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut watch = Watch::new(&issue);
        let old = job(red("custom-release-smoke / wsl"), 1, 900);
        let new = job(green("custom-release-smoke / wsl"), 2, 900);
        let mut github = InMemory::new()
            .listing(
                HEAD,
                vec![
                    vec![old.clone()],
                    vec![old.clone()],
                    vec![old.clone()],
                    vec![old.clone()],
                    vec![new.clone()],
                    vec![new],
                    vec![old],
                ],
            )
            .listing("another-head", vec![vec![green("test")]])
            .job_run(1, abandoned_job_run());

        assert!(matches!(
            watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap(),
            Ci::Passed
        ));
        assert!(matches!(
            watch
                .assess_through(&mut github, "another-head", None)
                .unwrap(),
            Ci::Passed
        ));
        let failed = failed_of(watch.assess_through(&mut github, HEAD, Some(BASE)).unwrap());
        assert_eq!(
            listed(&failed.own),
            ["custom-release-smoke / wsl: https://head/custom-release-smoke / wsl"]
        );
        assert_eq!(github.reruns, [900]);
        assert!(github.steps.contains(
            &"custom-release-smoke / wsl lost its runner on abcdef0; re-running it".to_string()
        ));
    }

    #[test]
    fn a_refused_or_unseen_abandoned_rerun_retains_the_original_red_assessment() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        for refused in [false, true] {
            let mut watch = Watch::new(&issue);
            let old = vec![job(red("wsl"), 1, 900), job(red("mac"), 3, 901)];
            let new = vec![job(green("wsl"), 2, 900), job(green("mac"), 4, 901)];
            let mut github = InMemory::new()
                .listing(
                    HEAD,
                    if refused {
                        vec![old.clone(), old.clone(), old, new]
                    } else {
                        vec![old]
                    },
                )
                .job_run(1, abandoned_job_run())
                .job_run(3, abandoned_job_run());
            if refused {
                github = github.refusing(901);
            }

            let failed = failed_of(watch.assess_through(&mut github, HEAD, None).unwrap());

            assert_eq!(
                listed(&failed.own),
                ["wsl: https://head/wsl", "mac: https://head/mac"]
            );
            assert_eq!(failed.own[0].job.unwrap().check_run, 1);
            assert_eq!(github.reruns, [900, 901]);
            if refused {
                assert_eq!(
                    github.reads_of(HEAD),
                    3,
                    "refusal must retain the original assessment despite green checks now being available"
                );
                assert!(matches!(
                    watch.assess_through(&mut github, HEAD, None).unwrap(),
                    Ci::Passed
                ));
            } else {
                assert_eq!(
                    github.steps.last().map(String::as_str),
                    Some("no re-run appeared on abcdef0 within 60s")
                );
                assert_eq!(
                    listed(&failed_of(watch.assess_through(&mut github, HEAD, None).unwrap()).own),
                    ["wsl: https://head/wsl", "mac: https://head/mac"]
                );
            }
            assert_eq!(github.reruns, [900, 901]);
        }
    }

    #[test]
    fn a_refused_check_rerun_uses_fresh_observations_after_a_completed_repair() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut watch = Watch::new(&issue);
        let old = job(red("test"), 1, 900);
        let mut github = InMemory::new()
            .listing(
                HEAD,
                vec![
                    vec![old.clone()],
                    vec![old.clone()],
                    vec![old],
                    vec![job(green("test"), 2, 900)],
                ],
            )
            .refusing(900);
        let failed = failed_of(watch.assess_through(&mut github, HEAD, None).unwrap());
        watch.after_repair(HEAD.to_string(), failed);

        assert!(matches!(
            watch.assess_through(&mut github, HEAD, None).unwrap(),
            Ci::Passed
        ));
        assert_eq!(github.reruns, [900]);
        assert_eq!(github.reads_of(HEAD), 4);
    }

    #[test]
    fn mixed_own_failures_are_assessed_without_an_abandoned_job_rerun() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let mut watch = Watch::new(&issue);
        let mut github = InMemory::new()
            .listing(
                HEAD,
                vec![vec![job(red("wsl"), 1, 900), job(red("test"), 2, 901)]],
            )
            .job_run(1, abandoned_job_run());

        let failed = failed_of(watch.assess_through(&mut github, HEAD, None).unwrap());

        assert_eq!(
            listed(&failed.own),
            ["wsl: https://head/wsl", "test: https://head/test"]
        );
        assert!(github.reruns.is_empty());
        assert_eq!(github.job_reads, [1, 2]);
    }

    #[test]
    fn a_failed_job_with_no_failed_step_and_a_step_not_completed_is_abandoned() {
        let job = |conclusion: Conclusion, steps: &[Conclusion]| JobRun {
            conclusion,
            steps: steps.to_vec(),
        };
        use Conclusion::{Failure, Skipped, Success, Unfinished};

        // As run 39469875996 lost its runner: set-up done, a step still in
        // progress, the rest pending.
        assert!(is_abandoned(&job(
            Failure,
            &[Success, Unfinished, Unfinished, Unfinished]
        )));
        assert!(!is_abandoned(&job(Failure, &[Success, Failure, Skipped])));
        assert!(!is_abandoned(&job(
            Failure,
            &[Success, Failure, Unfinished]
        )));
        assert!(!is_abandoned(&job(Failure, &[Success, Success])));
        assert!(!is_abandoned(&job(Failure, &[])));
        assert!(!is_abandoned(&job(
            Conclusion::Cancelled,
            &[Success, Unfinished]
        )));
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
