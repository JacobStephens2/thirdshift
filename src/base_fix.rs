//! The Base fix (ADR-0008): a Merge run into a Run's Base branch, on an issue
//! thirdshift writes, that a Run asked to starts when its only red checks are
//! Inherited failures, and waits for. One broken Base branch gets one Base
//! fix: a Run that finds another's open for the same checks waits on that one
//! instead. Runs from one Launch directory, as a Spec run's Tickets are, look
//! for it and write it one at a time, so those that meet the same Inherited
//! failures at once share one Base fix; across clones and machines the look
//! is a best-effort lock.

use std::fmt;
use std::fs::File;
use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::child_run::{self, Ended, Kind};
use crate::ci;
use crate::git::Git;
use crate::github::{self, Check, CheckState};
use crate::issue::IssueUrl;
use crate::poll;
use crate::progress;
use crate::spec_run::READY_FOR_AGENT;

/// The label that marks a Base fix issue, which a Run finds an open one by.
const BASE_FIX_LABEL: &str = "base-fix";

/// The labels of a Base fix issue, each with the description it is added to
/// the repository with if the repository lacks it.
const LABELS: [(&str, &str); 2] = [
    (BASE_FIX_LABEL, "A Base fix: CI is red on a Base branch"),
    (READY_FOR_AGENT, "Ready for an agent to take on"),
];

/// What starts the line on stderr saying what became of the Base fix a Run
/// took, which a Spec run reads back from a Ticket's Run.
pub const REPORT: &str = "Base fix: ";

/// What a Run asks about a Base fix, by its command or, without `base-fix`
/// or `no-base-fix`, by the User config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseFixAsk {
    /// The Run may start one.
    Allow,
    /// The Run starts none.
    Forbid,
    /// Nobody decided, the command or the User config: the Run starts none,
    /// and if Inherited failures fail it, its [`Advice`] offers one, with
    /// `retry`, the command that starts the Run again with one allowed.
    Undecided { retry: String },
}

/// What starts the line of [`Advice`] linking a check where it fails on the
/// Base branch.
const BASE_CHECK: &str = "Base check";

/// What starts the line of [`Advice`] with the command that starts the Run
/// again with a Base fix allowed.
const RETRY_WITH: &str = "Retry with";

/// What starts the line of [`Advice`] naming the User config's `base.fix`.
const OR_SET: &str = "Or set";

/// A line of the advice a Run gives after its cause when Inherited failures
/// fail it with no Base fix taken: each check where it fails on the Base
/// branch, then, if nobody decided against a Base fix, how to allow one. It
/// reads `<label>: <value>` on stderr, and the Run notification lays it out
/// like its other lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advice {
    /// What the line starts with, saying what its value is.
    pub label: &'static str,
    /// A check and its URL, a command, or a setting.
    pub value: String,
}

impl Advice {
    /// Whether `message`, a line a Run printed on stderr, is one of these:
    /// a child Run's is relayed, but is neither its cause nor its session
    /// log.
    pub fn is_line(message: &str) -> bool {
        [BASE_CHECK, RETRY_WITH, OR_SET].iter().any(|label| {
            message
                .strip_prefix(label)
                .is_some_and(|value| value.starts_with(": "))
        })
    }
}

impl fmt::Display for Advice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.label, self.value)
    }
}

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
    /// The command that starts the Run again with a Base fix allowed, if
    /// nobody decided whether it may start one.
    retry: Option<String>,
    taken: Option<Taken>,
    /// What the Run says after its cause, if Inherited failures failed it
    /// with no Base fix taken.
    advice: Vec<Advice>,
}

/// The Base fix a Run took as its one: one it started, or one it found
/// another Run running and waited on.
struct Taken {
    issue: IssueUrl,
    /// Started by this Run, rather than waited on.
    started: bool,
    /// Whether it came to the end the Run awaited: see [`Taken::awaited_end`].
    reached: bool,
}

impl Taken {
    /// The end of the Base fix the Run goes on from: `merged`, or `closed`
    /// for one waited on, of which the Run sees only the issue.
    fn awaited_end(&self) -> &'static str {
        if self.started { "merged" } else { "closed" }
    }
}

impl BaseFix {
    /// The Base fix of a Run that is the child Run `child`, if it is one,
    /// and that asked `ask` about a Base fix. A Base fix starts none of its
    /// own, whatever it asked.
    pub fn new(child: Option<&Kind>, ask: BaseFixAsk) -> Self {
        let (on_inherited_failures, retry) = match (child, ask) {
            (Some(Kind::BaseFix { .. }), _) => (OnInheritedFailures::IsBaseFix, None),
            (_, BaseFixAsk::Allow) => (OnInheritedFailures::StartBaseFix, None),
            (_, BaseFixAsk::Forbid) => (OnInheritedFailures::FailTheRun, None),
            (_, BaseFixAsk::Undecided { retry }) => (OnInheritedFailures::FailTheRun, Some(retry)),
        };
        BaseFix {
            on_inherited_failures,
            retry,
            taken: None,
            advice: Vec::new(),
        }
    }

    /// Whether the Run compares its red checks with the Base branch's, so
    /// that some may be Inherited failures.
    pub fn sees_inherited_failures(&self) -> bool {
        self.on_inherited_failures != OnInheritedFailures::IsBaseFix
    }

    /// Whether the Run may start a Base fix.
    fn may_start(&self) -> bool {
        self.on_inherited_failures == OnInheritedFailures::StartBaseFix
    }

    /// What each Ticket's Run of a Spec run asks about a Base fix: what the
    /// Spec run may do, so may it, and where nobody decided, it offers the
    /// command that starts the Spec run again with one allowed.
    pub fn ask_of_tickets(&self) -> BaseFixAsk {
        if self.may_start() {
            BaseFixAsk::Allow
        } else if let Some(retry) = &self.retry {
            BaseFixAsk::Undecided {
                retry: retry.clone(),
            }
        } else {
            BaseFixAsk::Forbid
        }
    }

    /// Fix the checks `inherited`, the only red ones on the PR `pr_url` of
    /// the Run on `issue`, started from `launch`, and all Inherited failures
    /// from `base` at `base_commit`, with a Base fix: write its issue, start
    /// it as a child `thirdshift` and wait for it to merge, after which the
    /// Run is to merge `base` in and watch CI again. If an open Base fix
    /// issue other than `issue` already names `base` and those checks, no
    /// issue is written: the Run waits for that one to close instead, as its
    /// one Base fix (see [`BaseFix::wait_on`]), or, if a Run from `launch`
    /// started that Base fix and it ended with the issue still open, starts
    /// it again on the same issue. Fails, with the Run's cause, if the Run
    /// was not asked to start one, if the Base fix fails, in which case the
    /// cause names its issue, or if the Run has had its one Base fix, in
    /// which case the cause names that one's issue. A Run that fails with no
    /// Base fix taken has [`BaseFix::into_advice`] to give after its cause.
    pub fn fix(
        &mut self,
        launch: &Git,
        issue: &IssueUrl,
        pr_url: &str,
        base: &str,
        base_commit: &str,
        inherited: &[Check],
    ) -> Result<()> {
        let checks = ci::check_names(inherited);
        let base_at = ci::short(base_commit);
        if self.taken.is_some() || !self.may_start() {
            let even_after = match &self.taken {
                Some(taken) => format!(
                    ", even after Base fix {} {}",
                    taken.issue.url,
                    taken.awaited_end()
                ),
                None => {
                    self.advice = self.advice_on(issue, base, base_commit, inherited);
                    String::new()
                }
            };
            bail!(
                "CI red on {checks}, which also fails on {base} at {base_at}{even_after}; \
                 fix {base} first"
            );
        }
        // Held from the look for an open Base fix issue until the one this
        // Run starts is written and marked as running.
        let looking = launch.lock("thirdshift-base-fix.lock")?;
        let fix = match open_fix_covering(issue, base, inherited)? {
            Some(open) => match Mark::of(launch, open.number)? {
                Mark::Left => {
                    progress::step(format_args!(
                        "Base fix #{} is open but no longer running; \
                         starting it again into {base}: {}",
                        open.number, open.url
                    ));
                    open
                }
                mark => {
                    drop(looking);
                    return self.wait_on(open, mark, base);
                }
            },
            None => {
                let on_base = failed_on_base(issue, base_commit, inherited)?;
                let fix = github::create_issue(
                    issue,
                    &issue_title(base, &checks),
                    &issue_body(issue, pr_url, base, base_at, &on_base),
                    &LABELS,
                )?;
                progress::step(format_args!(
                    "starting Base fix #{} into {base}: {}",
                    fix.number, fix.url
                ));
                fix
            }
        };
        let number = fix.number;
        let running = Running::mark(launch, number)?;
        drop(looking);
        let taken = self.taken.insert(Taken {
            issue: fix,
            started: true,
            reached: false,
        });
        let kind = Kind::BaseFix {
            base: base.to_string(),
        };
        let child = child_run::start(&taken.issue, &kind, &BaseFixAsk::Forbid)?;
        progress::step(format_args!("waiting on Base fix #{number}"));
        match child_run::wait(number, child)? {
            Ended::Reached { .. } => {
                taken.reached = true;
                running.merged();
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

    /// Take the Base fix on the issue `open`, which another Run is running,
    /// as the Run's one, and wait for the issue to close, as its Self-merge
    /// leaves it, after which the Run is to merge `base` in and watch CI
    /// again. If `mark` says a Run from this Launch directory is running it,
    /// this fails, naming the issue, once that Base fix has ended with the
    /// issue still open; otherwise the issue is all there is to wait on.
    fn wait_on(&mut self, open: IssueUrl, mark: Mark, base: &str) -> Result<()> {
        let number = open.number;
        progress::step(format_args!(
            "waiting on Base fix #{number}, already open: {}",
            open.url
        ));
        let taken = self.taken.insert(Taken {
            issue: open,
            started: false,
            reached: false,
        });
        let closed = poll::until(|| {
            // Before the issue is read: a Base fix closes it before it ends.
            let ended = mark.has_ended();
            let closed = !github::issue_is_open(&taken.issue)?;
            Ok((closed || ended).then_some(closed))
        })?;
        if !closed {
            bail!(
                "Base fix {} ended with its issue still open",
                taken.issue.url
            );
        }
        taken.reached = true;
        progress::step(format_args!(
            "Base fix #{number} closed; merging {base} in again"
        ));
        Ok(())
    }

    /// The advice for a Run on `issue` that the checks `inherited`, Inherited
    /// failures from `base` at `base_commit`, fail with no Base fix taken:
    /// each check with its URL on `base`, then, if nobody decided against a
    /// Base fix, the command that retries the Run with one allowed and the
    /// User config's setting that allows one for every Run. If the checks on
    /// `base` can't be read, that is reported and none is linked.
    fn advice_on(
        &self,
        issue: &IssueUrl,
        base: &str,
        base_commit: &str,
        inherited: &[Check],
    ) -> Vec<Advice> {
        let on_base = failed_on_base(issue, base_commit, inherited).unwrap_or_else(|error| {
            progress::step(format_args!(
                "could not read the checks on {base}: {error:#}"
            ));
            Vec::new()
        });
        let mut advice: Vec<Advice> = on_base
            .iter()
            .map(|check| Advice {
                label: BASE_CHECK,
                value: ci::check_with_url(check),
            })
            .collect();
        if let Some(retry) = &self.retry {
            advice.push(Advice {
                label: RETRY_WITH,
                value: retry.clone(),
            });
            advice.push(Advice {
                label: OR_SET,
                value: "base.fix = true in ~/.thirdshift/config.toml, \
                        to allow a Base fix for every Run on this machine"
                    .to_string(),
            });
        }
        advice
    }

    /// What the Run says after its cause, if Inherited failures failed it
    /// with no Base fix taken; nothing otherwise.
    pub fn into_advice(self) -> Vec<Advice> {
        self.advice
    }

    /// What became of the Base fix the Run took, if it took one, for its Run
    /// notification: the issue's URL, then `merged` or `not merged` for one
    /// it started, and `closed` or `not closed` for one it waited on.
    pub fn report(&self) -> Option<String> {
        let taken = self.taken.as_ref()?;
        let not = if taken.reached { "" } else { "not " };
        Some(format!("{} {not}{}", taken.issue.url, taken.awaited_end()))
    }
}

/// The file, in the Launch directory's common git directory, that marks
/// Base fix `number` as started from this Launch directory.
fn mark_file(number: u64) -> String {
    format!("thirdshift-base-fix-{number}.lock")
}

/// The mark that this Run is running the Base fix it started: a lock on the
/// Base fix's mark file, held until this is dropped. The file outlives it
/// unless the Base fix merged, so the next Run to find the issue open can
/// tell the Base fix is no longer running.
struct Running {
    file: PathBuf,
    _lock: File,
}

impl Running {
    /// Mark Base fix `number` as running, from `launch`.
    fn mark(launch: &Git, number: u64) -> Result<Self> {
        let name = mark_file(number);
        Ok(Running {
            file: launch.common_dir()?.join(&name),
            _lock: launch.lock(&name)?,
        })
    }

    /// The Base fix merged, closing its issue: no Run will find it open, so
    /// the mark file goes.
    fn merged(self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// What this Launch directory's mark says of a Base fix found open.
enum Mark {
    /// A Run from here is running it, and holds the lock on this file until
    /// it ends.
    Running(File),
    /// A Run from here started it, and it ended with its issue still open.
    Left,
    /// No Run from here started it: one from another clone or machine did.
    None,
}

impl Mark {
    /// The mark of Base fix `number` in `launch`.
    fn of(launch: &Git, number: u64) -> Result<Self> {
        let Ok(file) = File::open(launch.common_dir()?.join(mark_file(number))) else {
            return Ok(Mark::None);
        };
        let mark = Mark::Running(file);
        Ok(if mark.has_ended() { Mark::Left } else { mark })
    }

    /// Whether the Run from here that was running the Base fix has ended,
    /// letting go of its lock.
    fn has_ended(&self) -> bool {
        let Mark::Running(file) = self else {
            return false;
        };
        let ended = file.try_lock_shared().is_ok();
        if ended {
            let _ = file.unlock();
        }
        ended
    }
}

/// The checks that failed on the Base branch at `base_commit`, of the Run on
/// `issue`, under the name of one of `inherited`: the Base branch's own
/// checks, with their URLs there, not the PR's.
fn failed_on_base(issue: &IssueUrl, base_commit: &str, inherited: &[Check]) -> Result<Vec<Check>> {
    let on_base = github::checks_on(issue, base_commit)?
        .into_iter()
        .filter(|check| {
            check.state == CheckState::Failed
                && inherited.iter().any(|failed| failed.name == check.name)
        })
        .collect();
    Ok(on_base)
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
    let covering = github::open_issues_labelled(&issue.repo_slug(), BASE_FIX_LABEL)?
        .into_iter()
        .filter(|(open, title)| {
            open.number != issue.number
                && title
                    .strip_prefix(&before_checks)
                    .is_some_and(|checks| inherited.iter().all(|check| names(checks, &check.name)))
        })
        .map(|(open, _)| open)
        .min_by_key(|open| open.number);
    Ok(covering)
}

/// Whether `checks`, check names as in "test, lint", names the check `name`,
/// which may itself hold the `, ` that sets names apart.
fn names(checks: &str, name: &str) -> bool {
    checks.match_indices(name).any(|(at, _)| {
        let (before, after) = (&checks[..at], &checks[at + name.len()..]);
        (before.is_empty() || before.ends_with(", "))
            && (after.is_empty() || after.starts_with(", "))
    })
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
    let checks = ci::check_list(failed);
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
