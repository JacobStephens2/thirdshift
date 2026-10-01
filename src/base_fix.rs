//! The Base fix (ADR-0008): a Merge run into a Run's Base branch, on an issue
//! thirdshift writes, that a Run asked to starts when its only red checks are
//! Inherited failures, and waits for. One broken Base branch gets one Base
//! fix: a Run that finds another's open for the same checks waits on that one
//! instead. Runs from one Launch directory, as a Spec run's Tickets are, look
//! for it and write it one at a time, so those that meet the same Inherited
//! failures at once share one Base fix; across Launch directories the look is
//! a best-effort lock.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::child_run::{self, Ended, Kind};
use crate::ci;
use crate::git::Git;
use crate::github::{self, Check, CheckState};
use crate::issue::IssueUrl;
use crate::poll;
use crate::progress;

/// The label that marks a Base fix issue, which a Run finds an open one by.
const BASE_FIX_LABEL: &str = "base-fix";

/// The labels of a Base fix issue, each with the description it is added to
/// the repository with if the repository lacks it.
const LABELS: [(&str, &str); 2] = [
    (BASE_FIX_LABEL, "A Base fix: CI is red on a Base branch"),
    ("ready-for-agent", "Ready for an agent to take on"),
];

/// What a Run does when its only red checks are Inherited failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnInheritedFailures {
    /// Fail, saying to fix the Base branch first.
    FailTheRun,
    /// Start a Base fix, once, and go on when it has merged.
    StartBaseFix,
    /// The Run is itself a Base fix: every red check is its own to fix, so it
    /// sees no Inherited failures and starts no Base fix.
    IsBaseFix,
}

/// A Run's one Base fix: whether it may start one, and the one it took.
pub struct BaseFix {
    on_inherited_failures: OnInheritedFailures,
    taken: Option<Taken>,
}

/// The Base fix a Run took as its one: one it started, or one it found open,
/// started by another Run, and waited on.
struct Taken {
    issue: IssueUrl,
    /// Started by this Run, rather than found open.
    started: bool,
    /// It ended as the Run waited for it to: see [`Taken::end`].
    ended: bool,
}

impl Taken {
    /// How the Base fix ends when the Run can go on: `merged`, or `closed`
    /// for one found open, of which the Run sees only the issue.
    fn end(&self) -> &'static str {
        if self.started { "merged" } else { "closed" }
    }
}

impl BaseFix {
    /// The Base fix of a Run that is the child Run `child`, if it is one,
    /// and that, with `asked`, was given `base-fix`. A Base fix starts none
    /// of its own, whatever it was given.
    pub fn new(child: Option<&Kind>, asked: bool) -> Self {
        let on_inherited_failures = match (child, asked) {
            (Some(Kind::BaseFix { .. }), _) => OnInheritedFailures::IsBaseFix,
            (_, true) => OnInheritedFailures::StartBaseFix,
            (_, false) => OnInheritedFailures::FailTheRun,
        };
        BaseFix {
            on_inherited_failures,
            taken: None,
        }
    }

    /// Whether the Run compares its red checks with the Base branch's, so
    /// that some may be Inherited failures.
    pub fn sees_inherited_failures(&self) -> bool {
        self.on_inherited_failures != OnInheritedFailures::IsBaseFix
    }

    /// Whether the Run may start a Base fix, as may then each Ticket's Run
    /// of a Spec run.
    pub fn may_start(&self) -> bool {
        self.on_inherited_failures == OnInheritedFailures::StartBaseFix
    }

    /// Fix the checks `inherited`, the only red ones on the PR `pr_url` of
    /// the Run on `issue` and all Inherited failures from `base` at
    /// `base_commit`, with a Base fix: write its issue, start it as a child
    /// `thirdshift` and wait for it to merge, after which the Run is to merge
    /// `base` in and watch CI again. If an open Base fix issue other than
    /// `issue` already names `base` and those checks, the Run waits for that
    /// one to close instead, as its one Base fix: see [`BaseFix::wait_on`].
    /// Fails, with the Run's cause, if the Run was not asked to start one, if
    /// the Base fix fails, in which case the cause names its issue, or if the
    /// Run has had its one Base fix, in which case the cause names that one's
    /// issue.
    pub fn fix(
        &mut self,
        issue: &IssueUrl,
        pr_url: &str,
        base: &str,
        base_commit: &str,
        inherited: &[Check],
    ) -> Result<()> {
        let checks = ci::check_names(inherited);
        let base_at = ci::short(base_commit);
        if let Some(taken) = &self.taken {
            bail!(
                "CI red on {checks}, which also fails on {base} at {base_at}, \
                 even after Base fix {} {}; fix {base} first",
                taken.issue.url,
                taken.end()
            );
        }
        if !self.may_start() {
            bail!("CI red on {checks}, which also fails on {base} at {base_at}; fix {base} first");
        }
        let launch = Git::new(std::env::current_dir().context("no current directory")?);
        let common_dir = launch.common_dir()?;
        // Held from the look for an open Base fix issue until this Run's own
        // is written and marked as running.
        let looking = lock(&common_dir.join("thirdshift-base-fix.lock"))?;
        if let Some(open) = open_fix_covering(issue, base, inherited)? {
            let running = Running::watch(&common_dir, open.number);
            drop(looking);
            return self.wait_on(open, running, base);
        }
        // The issue links the Base branch's own failed checks, not the PR's.
        let on_base: Vec<Check> = github::checks_on(issue, base_commit)?
            .into_iter()
            .filter(|check| {
                check.state == CheckState::Failed
                    && inherited.iter().any(|failed| failed.name == check.name)
            })
            .collect();
        let fix = github::create_issue(
            issue,
            &issue_title(base, &checks),
            &issue_body(issue, pr_url, base, base_at, &on_base),
            &LABELS,
        )?;
        let number = fix.number;
        let _running = Running::mark(&common_dir, number)?;
        drop(looking);
        progress::step(format_args!(
            "starting Base fix #{number} into {base}: {}",
            fix.url
        ));
        let taken = self.taken.insert(Taken {
            issue: fix,
            started: true,
            ended: false,
        });
        let kind = Kind::BaseFix {
            base: base.to_string(),
        };
        let child = child_run::start(&taken.issue, &kind, false)?;
        progress::step(format_args!("waiting on Base fix #{number}"));
        match child_run::wait(number, child)? {
            Ended::Reached(_) => {
                taken.ended = true;
                progress::step(format_args!(
                    "Base fix #{number} merged; merging {base} in again"
                ));
                Ok(())
            }
            Ended::Interrupted => bail!("interrupted"),
            Ended::Failed { cause, log } => {
                if let Some(log) = log {
                    progress::step(format_args!("Base fix #{number} session log: {log}"));
                }
                bail!("Base fix {} failed: {cause}", taken.issue.url)
            }
        }
    }

    /// Take the Base fix on the issue `open`, which another Run started, as
    /// the Run's one, and wait for the issue to close, as its Self-merge
    /// leaves it, after which the Run is to merge `base` in and watch CI
    /// again. With `running`, the lock the Run that started it holds while
    /// it runs, this fails, naming the issue, if the Base fix ends with the
    /// issue still open; without it, the issue is all there is to wait on.
    fn wait_on(&mut self, open: IssueUrl, running: Option<File>, base: &str) -> Result<()> {
        let number = open.number;
        progress::step(format_args!(
            "waiting on Base fix #{number}, already open: {}",
            open.url
        ));
        let taken = self.taken.insert(Taken {
            issue: open,
            started: false,
            ended: false,
        });
        let closed = poll::until(|| {
            // Before the issue is read: a Base fix closes it before it ends.
            let ended = running
                .as_ref()
                .is_some_and(|running| running.try_lock_shared().is_ok());
            let closed = !github::issue_is_open(&taken.issue)?;
            Ok((closed || ended).then_some(closed))
        })?;
        if !closed {
            bail!(
                "Base fix {} ended with its issue still open",
                taken.issue.url
            );
        }
        taken.ended = true;
        progress::step(format_args!(
            "Base fix #{number} closed; merging {base} in again"
        ));
        Ok(())
    }

    /// What became of the Base fix the Run took, if it took one, for its Run
    /// notification: the issue's URL, then `merged` or `not merged` for one
    /// it started, and `closed` or `not closed` for one it waited on.
    pub fn report(&self) -> Option<String> {
        let taken = self.taken.as_ref()?;
        let not = if taken.ended { "" } else { "not " };
        Some(format!("{} {not}{}", taken.issue.url, taken.end()))
    }
}

/// Wait for, then hold until the file is dropped, a lock on the file `path`.
fn lock(path: &Path) -> Result<File> {
    let file = File::create(path).with_context(|| format!("can't open {}", path.display()))?;
    file.lock()
        .with_context(|| format!("can't lock {}", path.display()))?;
    Ok(file)
}

/// The mark that a Run from this Launch directory is running the Base fix it
/// started: a lock on a file named for the Base fix issue, held, and the file
/// removed, when this is dropped.
struct Running {
    path: PathBuf,
    _lock: File,
}

impl Running {
    /// The file for Base fix `number`, in the repository's `common_dir`.
    fn path(common_dir: &Path, number: u64) -> PathBuf {
        common_dir.join(format!("thirdshift-base-fix-{number}.lock"))
    }

    /// Mark Base fix `number` as running.
    fn mark(common_dir: &Path, number: u64) -> Result<Self> {
        let path = Self::path(common_dir, number);
        let lock = lock(&path)?;
        Ok(Running { path, _lock: lock })
    }

    /// The file a Run from this Launch directory holds locked while it runs
    /// Base fix `number`, if one is running it now: a shared lock on it can
    /// be had once that Base fix has ended.
    fn watch(common_dir: &Path, number: u64) -> Option<File> {
        let file = File::open(Self::path(common_dir, number)).ok()?;
        file.try_lock_shared().is_err().then_some(file)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The title of the Base fix issue for `checks`, their names as in
/// "test, lint", red on the Base branch `base`.
fn issue_title(base: &str, checks: &str) -> String {
    format!("CI red on {base}: {checks}")
}

/// The oldest open Base fix issue, other than `issue` itself, whose title
/// names the Base branch `base` and every one of the checks `inherited`.
fn open_fix_covering(
    issue: &IssueUrl,
    base: &str,
    inherited: &[Check],
) -> Result<Option<IssueUrl>> {
    let before_checks = issue_title(base, "");
    let covering = github::open_issues_labelled(issue, BASE_FIX_LABEL)?
        .into_iter()
        .filter(|(open, title)| {
            open.number != issue.number
                && title.strip_prefix(&before_checks).is_some_and(|checks| {
                    let named: Vec<&str> = checks.split(", ").collect();
                    inherited
                        .iter()
                        .all(|check| named.contains(&check.name.as_str()))
                })
        })
        .map(|(open, _)| open)
        .min_by_key(|open| open.number);
    Ok(covering)
}

/// The body of the Base fix issue, from a fixed template: the checks
/// `failed` on the Base branch `base` at its commit `base_at`, shortened, with
/// their URLs, and the Run that found them, on `issue` with the PR `pr_url`.
fn issue_body(
    issue: &IssueUrl,
    pr_url: &str,
    base: &str,
    base_at: &str,
    failed: &[Check],
) -> String {
    let mut checks = String::new();
    for check in failed {
        checks += &match &check.url {
            Some(url) => format!("- {}: {url}\n", check.name),
            None => format!("- {}\n", check.name),
        };
    }
    format!(
        "CI is red on `{base}` at {base_at}: these checks fail there, so every pull request into `{base}` inherits them.\n\
         \n\
         {checks}\
         \n\
         Found by the thirdshift Run on #{}, whose pull request is {pr_url}.\n\
         \n\
         Fix the checks on `{base}`. Do not skip, disable, or weaken tests or checks to make them pass.\n",
        issue.number
    )
}
