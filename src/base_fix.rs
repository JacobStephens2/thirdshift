//! The Base fix (ADR-0008): a Merge run into a Run's Base branch, on an issue
//! thirdshift writes, that a Run asked to starts when its only red checks are
//! Inherited failures, and waits for. One broken Base branch gets one Base
//! fix: a Run that finds another's open for the same checks waits on that one
//! instead. Runs from one Launch directory, as a Spec run's Tickets are, look
//! for it and write it one at a time, so those that meet the same Inherited
//! failures at once share one Base fix; across clones and machines the look
//! is a best-effort lock. Its rules reach the look lock, GitHub, the marks,
//! the child Run and the poll interval only through [`Outside`].

use std::fmt;
use std::fs::File;
use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::child_run::{self, Ended, Kind};
use crate::ci::{self, FailedChecks};
use crate::git::Git;
use crate::github::{self, Check, ListedIssue};
use crate::issue::IssueUrl;
use crate::labels::{Label, Labels, READY_FOR_AGENT};
use crate::poll;
use crate::progress;

/// The label that marks a Base fix issue, which a Run finds an open one by.
pub const BASE_FIX: Label = Label::new("base-fix", "A Base fix: CI is red on a Base branch");

/// Whether an issue with `labels` is a Base fix issue, whatever else it is
/// labelled.
pub fn is_issue(labels: &Labels) -> bool {
    labels.has(BASE_FIX)
}

/// The labels of a Base fix issue.
const LABELS: [Label; 2] = [BASE_FIX, READY_FOR_AGENT];

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
    /// Whether `message`, a line a Run printed on stderr, is one of these.
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

    /// Fix the Inherited failures in `failed`, the only red checks on the PR
    /// `pr_url` of the Run on `issue`, started from `launch`, which also fail
    /// on `base` at `base_commit`, where `failed` holds that commit's checks
    /// of their names, with a Base fix: write its issue, start it as a child
    /// `thirdshift` and wait for it to merge, after which the Run is to merge
    /// `base` in and watch CI again. If an open Base fix issue other than
    /// `issue` already names `base` and those checks, no issue is written:
    /// the Run waits for that one to close instead, as its one Base fix (see
    /// [`BaseFix::wait_on`]), or, if a Run from `launch` started that Base fix
    /// and it ended with the issue still open, starts it again on the same
    /// issue. Fails, with the Run's cause, if the Run was not asked to start
    /// one, if the Base fix fails, in which case the cause names its issue,
    /// or if the Run has had its one Base fix, in which case the cause names
    /// that one's issue. A Run that fails with no Base fix taken has
    /// [`BaseFix::into_advice`] to give after its cause.
    pub fn fix(
        &mut self,
        launch: &Git,
        issue: &IssueUrl,
        pr_url: &str,
        base: &str,
        base_commit: &str,
        failed: &FailedChecks,
    ) -> Result<()> {
        let mut outside = LaunchAndGitHub { launch, issue };
        self.fix_through(&mut outside, issue, pr_url, base, base_commit, failed)
    }

    /// [`BaseFix::fix`], reaching the outside world through `outside`.
    fn fix_through<O: Outside>(
        &mut self,
        outside: &mut O,
        issue: &IssueUrl,
        pr_url: &str,
        base: &str,
        base_commit: &str,
        failed: &FailedChecks,
    ) -> Result<()> {
        let inherited = &failed.inherited;
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
                    self.advice = self.advice_on(&failed.on_base);
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
        let looking = outside.look()?;
        let covering = covering(outside.open_fixes()?, issue, base, inherited);
        let fix = match covering {
            Some(open) => match outside.mark(open.number)? {
                Mark::Left => {
                    outside.step(format!(
                        "Base fix #{} is open but no longer running; \
                         starting it again into {base}: {}",
                        open.number, open.url
                    ));
                    open
                }
                Mark::Running(watch) => {
                    drop(looking);
                    return self.wait_on(outside, open, Some(watch), base);
                }
                Mark::None => {
                    drop(looking);
                    return self.wait_on(outside, open, None, base);
                }
            },
            None => {
                let fix = outside.create_issue(
                    &issue_title(base, &checks),
                    &issue_body(issue, pr_url, base, base_at, &failed.on_base),
                    &LABELS,
                )?;
                outside.step(format!(
                    "starting Base fix #{} into {base}: {}",
                    fix.number, fix.url
                ));
                fix
            }
        };
        let number = fix.number;
        let running = outside.mark_running(number)?;
        drop(looking);
        let taken = self.taken.insert(Taken {
            issue: fix,
            started: true,
            reached: false,
        });
        outside.step(format!("waiting on Base fix #{number}"));
        match outside.run_base_fix(&taken.issue, base)? {
            Ended::Reached { .. } => {
                taken.reached = true;
                outside.merged(running);
                outside.step(format!(
                    "Base fix #{number} merged; merging {base} in again"
                ));
                Ok(())
            }
            Ended::Interrupted => bail!("interrupted"),
            Ended::Failed { cause, log } => {
                if let Some(log) = log {
                    outside.step(format!("Base fix #{number} session log: {log}"));
                }
                bail!("Base fix {} failed: {cause}", taken.issue.url)
            }
        }
    }

    /// Take the Base fix on the issue `open`, which another Run is running,
    /// as the Run's one, and wait for the issue to close, as its Self-merge
    /// leaves it, after which the Run is to merge `base` in and watch CI
    /// again. If `watch` is the mark of a Run from this Launch directory
    /// running it, this fails, naming the issue, once that Base fix has ended
    /// with the issue still open; otherwise the issue is all there is to wait
    /// on.
    fn wait_on<O: Outside>(
        &mut self,
        outside: &mut O,
        open: IssueUrl,
        watch: Option<O::Watch>,
        base: &str,
    ) -> Result<()> {
        let number = open.number;
        outside.step(format!(
            "waiting on Base fix #{number}, already open: {}",
            open.url
        ));
        let taken = self.taken.insert(Taken {
            issue: open,
            started: false,
            reached: false,
        });
        let closed = loop {
            // Before the issue is read: a Base fix closes it before it ends.
            let ended = watch.as_ref().is_some_and(|watch| outside.has_ended(watch));
            let closed = !outside.issue_is_open(&taken.issue)?;
            if closed || ended {
                break closed;
            }
            outside.pause()?;
        };
        if !closed {
            bail!(
                "Base fix {} ended with its issue still open",
                taken.issue.url
            );
        }
        taken.reached = true;
        outside.step(format!(
            "Base fix #{number} closed; merging {base} in again"
        ));
        Ok(())
    }

    /// The advice for a Run that Inherited failures fail with no Base fix
    /// taken: each of `on_base`, the checks behind them on the Base branch,
    /// with its URL there, then, if nobody decided against a Base fix, the
    /// command that retries the Run with one allowed and the User config's
    /// setting that allows one for every Run.
    fn advice_on(&self, on_base: &[Check]) -> Vec<Advice> {
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

/// What a Base fix does or reads outside itself: the look lock, the open
/// Base fix issues, this Launch directory's marks, the child Run, the poll
/// interval and its progress lines.
trait Outside {
    /// The look lock, held until dropped.
    type Look;
    /// This Run's mark that it runs the Base fix it started, held until
    /// dropped or [`Outside::merged`].
    type Running;
    /// An opened mark of a Base fix found running from here.
    type Watch;

    /// Take the look lock, held from the look for an open Base fix issue
    /// until the one the Run starts is written and marked as running.
    fn look(&mut self) -> Result<Self::Look>;
    /// The open Base fix issues in the Run's repository.
    fn open_fixes(&mut self) -> Result<Vec<ListedIssue>>;
    /// What this Launch directory's mark says of the Base fix on issue
    /// `number`.
    fn mark(&mut self, number: u64) -> Result<Mark<Self::Watch>>;
    /// Whether the Run from here that was running the Base fix `watch` marks
    /// has ended.
    fn has_ended(&mut self, watch: &Self::Watch) -> bool;
    /// Whether `issue` is open.
    fn issue_is_open(&mut self, issue: &IssueUrl) -> Result<bool>;
    /// Write the Base fix issue, in the Run's repository.
    fn create_issue(&mut self, title: &str, body: &str, labels: &[Label]) -> Result<IssueUrl>;
    /// Mark the Base fix on issue `number` as running, from here.
    fn mark_running(&mut self, number: u64) -> Result<Self::Running>;
    /// The Base fix `running` marks merged: no Run will find it open, so its
    /// mark goes.
    fn merged(&mut self, running: Self::Running);
    /// Start the Base fix on `fix` as a child Run into `base`, and wait for
    /// it to end.
    fn run_base_fix(&mut self, fix: &IssueUrl, base: &str) -> Result<Ended>;
    /// Wait one poll interval. Fails with `interrupted` as soon as the Run
    /// is interrupted.
    fn pause(&mut self) -> Result<()>;
    /// Write the progress line `line`.
    fn step(&mut self, line: String);
}

/// What this Launch directory's mark says of a Base fix found open.
enum Mark<W> {
    /// A Run from here is running it, and holds its mark until it ends.
    Running(W),
    /// A Run from here started it, and it ended with its issue still open.
    Left,
    /// No Run from here started it: one from another clone or machine did.
    None,
}

/// The outside world of a Run on `issue` started from `launch`: its git
/// directory's locks and marks, GitHub, the child Run and the poll interval.
struct LaunchAndGitHub<'a> {
    launch: &'a Git,
    issue: &'a IssueUrl,
}

impl Outside for LaunchAndGitHub<'_> {
    type Look = File;
    type Running = Running;
    /// The mark file, open, its lock held by the Run running the Base fix.
    type Watch = File;

    fn look(&mut self) -> Result<File> {
        self.launch.lock("thirdshift-base-fix.lock")
    }

    /// One `gh issue list`.
    fn open_fixes(&mut self) -> Result<Vec<ListedIssue>> {
        github::open_issues_labelled(&self.issue.repo_slug(), BASE_FIX)
    }

    /// Left if the mark file is there and its lock free when first opened.
    fn mark(&mut self, number: u64) -> Result<Mark<File>> {
        let Ok(file) = File::open(self.launch.common_dir()?.join(mark_file(number))) else {
            return Ok(Mark::None);
        };
        Ok(if self.has_ended(&file) {
            Mark::Left
        } else {
            Mark::Running(file)
        })
    }

    fn has_ended(&mut self, file: &File) -> bool {
        let ended = file.try_lock_shared().is_ok();
        if ended {
            let _ = file.unlock();
        }
        ended
    }

    fn issue_is_open(&mut self, issue: &IssueUrl) -> Result<bool> {
        github::issue_is_open(issue)
    }

    fn create_issue(&mut self, title: &str, body: &str, labels: &[Label]) -> Result<IssueUrl> {
        github::create_issue(self.issue, title, body, labels)
    }

    fn mark_running(&mut self, number: u64) -> Result<Running> {
        Running::mark(self.launch, number)
    }

    fn merged(&mut self, running: Running) {
        running.merged();
    }

    fn run_base_fix(&mut self, fix: &IssueUrl, base: &str) -> Result<Ended> {
        let kind = Kind::BaseFix {
            base: base.to_string(),
        };
        child_run::start(fix, kind, BaseFixAsk::Forbid)?.wait()
    }

    fn pause(&mut self) -> Result<()> {
        poll::pause()
    }

    fn step(&mut self, line: String) {
        progress::step(line);
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

/// The title of the Base fix issue for `checks`, their names as in
/// "test, lint", red on the Base branch `base`.
fn issue_title(base: &str, checks: &str) -> String {
    format!("CI red on {base}: {checks}")
}

/// Of the open Base fix issues `open`, the oldest, other than `issue`
/// itself, whose title names the Base branch `base` and every one of the
/// checks `inherited`.
fn covering(
    open: Vec<ListedIssue>,
    issue: &IssueUrl,
    base: &str,
    inherited: &[Check],
) -> Option<IssueUrl> {
    let before_checks = issue_title(base, "");
    open.into_iter()
        .filter(|open| {
            open.issue.number != issue.number
                && open
                    .title
                    .strip_prefix(&before_checks)
                    .is_some_and(|checks| inherited.iter().all(|check| names(checks, &check.name)))
        })
        .map(|open| open.issue)
        .min_by_key(|open| open.number)
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use anyhow::anyhow;

    use super::*;
    use crate::github::CheckState;

    /// What the Base fix did outside itself, in the order it did it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Did {
        /// Took the look lock.
        Look,
        /// Let go of the look lock.
        LookReleased,
        /// Listed the open Base fix issues.
        ListFixes,
        /// Read the mark of the Base fix on this issue.
        ReadMark(u64),
        /// Asked whether the Run running the Base fix on this issue ended.
        ReadEnded(u64),
        /// Asked whether this issue is open.
        ReadIssue(u64),
        /// Wrote an issue with this title, body and labels.
        CreateIssue(String, String, Vec<&'static str>),
        /// Marked the Base fix on this issue as running.
        MarkRunning(u64),
        /// Removed the mark of the Base fix on this issue, as merged.
        Merged(u64),
        /// Let go of the mark of the Base fix on this issue.
        RunningReleased(u64),
        /// Started the Base fix on this issue into this Base branch, and
        /// waited for it.
        RunBaseFix(u64, String),
        /// Waited one poll interval.
        Pause,
        /// Wrote this progress line.
        Line(String),
    }

    type Log = Rc<RefCell<Vec<Did>>>;

    /// A held lock or mark, which records `on_drop` when let go of.
    struct Guard {
        log: Log,
        on_drop: Did,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            self.log.borrow_mut().push(self.on_drop.clone());
        }
    }

    /// What a found Base fix's mark says, as a script gives it.
    #[derive(Clone, Copy)]
    enum Marked {
        Running,
        Left,
    }

    /// What the outside world answers, call by call. No mark means none,
    /// a Run running a Base fix has not ended and the child Run reaches its
    /// goal, unless scripted otherwise.
    #[derive(Default)]
    struct Script {
        /// The open Base fix issues, by number and title.
        open_fixes: Vec<(u64, &'static str)>,
        /// The mark of each Base fix issue that has one.
        marks: Vec<(u64, Marked)>,
        /// For each ask, whether the Run running the Base fix ended.
        ended: VecDeque<bool>,
        /// For each ask, whether the issue is open.
        open: VecDeque<bool>,
        /// How each child Run ends.
        children: VecDeque<Ended>,
        /// Whether the Run is interrupted during a pause.
        interrupted: bool,
    }

    /// The outside world as `script` answers it, with what the Base fix did
    /// there recorded in `did`.
    struct Scripted {
        script: Script,
        did: Log,
    }

    impl Scripted {
        fn new(script: Script) -> Self {
            Scripted {
                script,
                did: Rc::default(),
            }
        }

        fn push(&self, did: Did) {
            self.did.borrow_mut().push(did);
        }

        fn guard(&self, on_drop: Did) -> Guard {
            Guard {
                log: Rc::clone(&self.did),
                on_drop,
            }
        }

        fn did(&self) -> Vec<Did> {
            self.did.borrow().clone()
        }
    }

    impl Outside for Scripted {
        type Look = Guard;
        type Running = Guard;
        /// The number of the issue whose mark it is.
        type Watch = u64;

        fn look(&mut self) -> Result<Guard> {
            self.push(Did::Look);
            Ok(self.guard(Did::LookReleased))
        }

        fn open_fixes(&mut self) -> Result<Vec<ListedIssue>> {
            self.push(Did::ListFixes);
            Ok(self
                .script
                .open_fixes
                .iter()
                .map(|&(number, title)| listed(number, title))
                .collect())
        }

        fn mark(&mut self, number: u64) -> Result<Mark<u64>> {
            self.push(Did::ReadMark(number));
            let marked = self.script.marks.iter().find(|(of, _)| *of == number);
            Ok(match marked {
                Some((_, Marked::Running)) => Mark::Running(number),
                Some((_, Marked::Left)) => Mark::Left,
                None => Mark::None,
            })
        }

        fn has_ended(&mut self, watch: &u64) -> bool {
            self.push(Did::ReadEnded(*watch));
            self.script.ended.pop_front().unwrap_or(false)
        }

        fn issue_is_open(&mut self, issue: &IssueUrl) -> Result<bool> {
            self.push(Did::ReadIssue(issue.number));
            Ok(self
                .script
                .open
                .pop_front()
                .expect("no issue state scripted"))
        }

        fn create_issue(&mut self, title: &str, body: &str, labels: &[Label]) -> Result<IssueUrl> {
            self.push(Did::CreateIssue(
                title.to_string(),
                body.to_string(),
                labels.iter().map(|label| label.name()).collect(),
            ));
            Ok(url(8))
        }

        fn mark_running(&mut self, number: u64) -> Result<Guard> {
            self.push(Did::MarkRunning(number));
            Ok(self.guard(Did::RunningReleased(number)))
        }

        fn merged(&mut self, running: Guard) {
            let Did::RunningReleased(number) = running.on_drop else {
                unreachable!("not a running mark");
            };
            self.push(Did::Merged(number));
        }

        fn run_base_fix(&mut self, fix: &IssueUrl, base: &str) -> Result<Ended> {
            self.push(Did::RunBaseFix(fix.number, base.to_string()));
            Ok(self.script.children.pop_front().unwrap_or(Ended::Reached {
                pr_url: None,
                base_fix: None,
            }))
        }

        fn pause(&mut self) -> Result<()> {
            self.push(Did::Pause);
            if self.script.interrupted {
                return Err(anyhow!("interrupted"));
            }
            Ok(())
        }

        fn step(&mut self, line: String) {
            self.push(Did::Line(line));
        }
    }

    /// Issue #`number` of acme/widgets.
    fn url(number: u64) -> IssueUrl {
        IssueUrl::parse(&format!("https://github.com/acme/widgets/issues/{number}")).unwrap()
    }

    /// The open Base fix issue #`number`, titled `title`.
    fn listed(number: u64, title: &str) -> ListedIssue {
        ListedIssue {
            issue: url(number),
            title: title.to_string(),
            labels: LABELS.iter().map(|label| label.name()).collect(),
        }
    }

    const FIX_URL: &str = "https://github.com/acme/widgets/issues/8";

    const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

    /// A failed check named `name`, at `url` if given.
    fn check(name: &str, url: Option<&str>) -> Check {
        Check {
            name: name.to_string(),
            state: CheckState::Failed,
            url: url.map(String::from),
            job: None,
        }
    }

    /// The checks `names`, each an Inherited failure, failing on main at
    /// `https://ci.example/main/<name>`.
    fn inherited(names: &[&str]) -> FailedChecks {
        FailedChecks {
            own: Vec::new(),
            inherited: names.iter().map(|name| check(name, None)).collect(),
            on_base: names
                .iter()
                .map(|name| check(name, Some(&format!("https://ci.example/main/{name}"))))
                .collect(),
        }
    }

    /// A Run's Base fix as asked `ask`, not itself a Base fix.
    fn asked(ask: BaseFixAsk) -> BaseFix {
        BaseFix::new(None, ask)
    }

    fn undecided() -> BaseFixAsk {
        BaseFixAsk::Undecided {
            retry: "thirdshift https://github.com/acme/widgets/issues/7 base-fix".to_string(),
        }
    }

    /// Fix the Inherited failures `failed` on main at `abcdef123` for the Run
    /// on #7, through `outside`.
    fn fix(base_fix: &mut BaseFix, outside: &mut Scripted, failed: &FailedChecks) -> Result<()> {
        base_fix.fix_through(outside, &url(7), PR_URL, "main", "abcdef123", failed)
    }

    /// The error of `result`, which must be one.
    fn error(result: Result<()>) -> String {
        format!("{:#}", result.expect_err("did not fail"))
    }

    fn line(line: &str) -> Did {
        Did::Line(line.to_string())
    }

    const CAUSE: &str = "CI red on test, which also fails on main at abcdef1; fix main first";

    #[test]
    fn a_run_not_allowed_a_base_fix_fails_linking_each_check_and_does_nothing_outside() {
        let mut base_fix = asked(BaseFixAsk::Forbid);
        let mut outside = Scripted::new(Script::default());

        let result = fix(&mut base_fix, &mut outside, &inherited(&["test"]));

        assert_eq!(error(result), CAUSE);
        assert_eq!(outside.did(), []);
        assert_eq!(base_fix.report(), None);
        assert_eq!(
            base_fix.into_advice(),
            [Advice {
                label: BASE_CHECK,
                value: "test: https://ci.example/main/test".to_string(),
            }]
        );
    }

    #[test]
    fn a_base_branch_check_with_no_url_is_named_alone() {
        let mut base_fix = asked(BaseFixAsk::Forbid);
        let mut outside = Scripted::new(Script::default());
        let mut failed = inherited(&["test"]);
        failed.on_base[0].url = None;

        let _ = fix(&mut base_fix, &mut outside, &failed);

        let advice: Vec<String> = base_fix
            .into_advice()
            .iter()
            .map(Advice::to_string)
            .collect();
        assert_eq!(advice, ["Base check: test"]);
    }

    #[test]
    fn a_run_nobody_decided_for_also_offers_to_retry_with_a_base_fix_or_set_one() {
        let mut base_fix = asked(undecided());
        let mut outside = Scripted::new(Script::default());

        let result = fix(&mut base_fix, &mut outside, &inherited(&["lint", "test"]));

        assert_eq!(
            error(result),
            "CI red on lint, test, which also fails on main at abcdef1; fix main first"
        );
        assert_eq!(outside.did(), []);
        let advice: Vec<String> = base_fix
            .into_advice()
            .iter()
            .map(Advice::to_string)
            .collect();
        assert_eq!(
            advice,
            [
                "Base check: lint: https://ci.example/main/lint",
                "Base check: test: https://ci.example/main/test",
                "Retry with: thirdshift https://github.com/acme/widgets/issues/7 base-fix",
                "Or set: base.fix = true in ~/.thirdshift/config.toml, \
                 to allow a Base fix for every Run on this machine",
            ]
        );
    }

    /// What starting a Base fix on a new issue #8 does, up to waiting on it.
    fn writes_and_starts_fix_8() -> Vec<Did> {
        vec![
            Did::Look,
            Did::ListFixes,
            Did::CreateIssue(
                "CI red on main: test".to_string(),
                "CI is red on `main` at abcdef1: these checks fail there, \
                 so every pull request into `main` inherits them.\n\
                 \n\
                 - test: https://ci.example/main/test\n\
                 \n\
                 Found by the thirdshift Run on #7, whose pull request is \
                 https://github.com/acme/widgets/pull/1.\n\
                 \n\
                 Fix the checks on `main`. Do not skip, disable, or weaken \
                 tests or checks to make them pass.\n"
                    .to_string(),
                vec!["base-fix", "ready-for-agent"],
            ),
            line(&format!("starting Base fix #8 into main: {FIX_URL}")),
            Did::MarkRunning(8),
            Did::LookReleased,
            line("waiting on Base fix #8"),
            Did::RunBaseFix(8, "main".to_string()),
        ]
    }

    #[test]
    fn an_allowed_run_with_no_covering_issue_writes_one_starts_its_base_fix_and_waits_for_it() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script {
            // Another Base branch's, and other checks'.
            open_fixes: vec![(5, "CI red on develop: test"), (6, "CI red on main: lint")],
            ..Script::default()
        });

        fix(&mut base_fix, &mut outside, &inherited(&["test"])).unwrap();

        let mut did = writes_and_starts_fix_8();
        did.extend([
            Did::Merged(8),
            Did::RunningReleased(8),
            line("Base fix #8 merged; merging main in again"),
        ]);
        assert_eq!(outside.did(), did);
        assert_eq!(base_fix.report(), Some(format!("{FIX_URL} merged")));
        assert_eq!(base_fix.into_advice(), []);
    }

    #[test]
    fn a_base_fix_that_fails_fails_the_run_naming_its_issue_and_keeps_its_mark_file() {
        for (log, log_line) in [
            (
                Some("/logs/8.log".to_string()),
                Some(line("Base fix #8 session log: /logs/8.log")),
            ),
            (None, None),
        ] {
            let mut base_fix = asked(BaseFixAsk::Allow);
            let mut outside = Scripted::new(Script {
                children: VecDeque::from([Ended::Failed {
                    cause: "CI red on test".to_string(),
                    log,
                }]),
                ..Script::default()
            });

            let result = fix(&mut base_fix, &mut outside, &inherited(&["test"]));

            assert_eq!(
                error(result),
                format!("Base fix {FIX_URL} failed: CI red on test")
            );
            let mut did = writes_and_starts_fix_8();
            did.extend(log_line);
            did.push(Did::RunningReleased(8));
            assert_eq!(outside.did(), did);
            assert_eq!(base_fix.report(), Some(format!("{FIX_URL} not merged")));
        }
    }

    #[test]
    fn an_interrupted_base_fix_fails_the_run_as_interrupted() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script {
            children: VecDeque::from([Ended::Interrupted]),
            ..Script::default()
        });

        let result = fix(&mut base_fix, &mut outside, &inherited(&["test"]));

        assert_eq!(error(result), "interrupted");
        let mut did = writes_and_starts_fix_8();
        did.push(Did::RunningReleased(8));
        assert_eq!(outside.did(), did);
        assert_eq!(base_fix.report(), Some(format!("{FIX_URL} not merged")));
    }

    #[test]
    fn inherited_failures_after_the_one_base_fix_fail_the_run_naming_it_with_no_advice() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script::default());
        fix(&mut base_fix, &mut outside, &inherited(&["test"])).unwrap();
        let before = outside.did();

        let result = fix(&mut base_fix, &mut outside, &inherited(&["test"]));

        assert_eq!(
            error(result),
            format!(
                "CI red on test, which also fails on main at abcdef1, \
                 even after Base fix {FIX_URL} merged; fix main first"
            )
        );
        assert_eq!(outside.did(), before, "nothing looked up or written");
        assert_eq!(base_fix.into_advice(), []);
    }

    #[test]
    fn inherited_failures_after_a_base_fix_waited_on_name_it_closed() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script {
            open_fixes: vec![(8, "CI red on main: test")],
            open: VecDeque::from([false]),
            ..Script::default()
        });
        fix(&mut base_fix, &mut outside, &inherited(&["test"])).unwrap();
        let before = outside.did();

        let result = fix(&mut base_fix, &mut outside, &inherited(&["test"]));

        assert_eq!(
            error(result),
            format!(
                "CI red on test, which also fails on main at abcdef1, \
                 even after Base fix {FIX_URL} closed; fix main first"
            )
        );
        assert_eq!(outside.did(), before, "nothing looked up or written");
        assert_eq!(base_fix.into_advice(), []);
    }

    /// What finding #8 open and covering the checks does, up to the wait.
    fn finds_fix_8() -> Vec<Did> {
        vec![
            Did::Look,
            Did::ListFixes,
            Did::ReadMark(8),
            Did::LookReleased,
            line(&format!("waiting on Base fix #8, already open: {FIX_URL}")),
        ]
    }

    #[test]
    fn a_covering_issue_running_from_here_is_waited_on_until_it_closes() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script {
            open_fixes: vec![(8, "CI red on main: lint, test")],
            marks: vec![(8, Marked::Running)],
            open: VecDeque::from([true, false]),
            ..Script::default()
        });

        fix(&mut base_fix, &mut outside, &inherited(&["test"])).unwrap();

        let mut did = finds_fix_8();
        did.extend([
            Did::ReadEnded(8),
            Did::ReadIssue(8),
            Did::Pause,
            Did::ReadEnded(8),
            Did::ReadIssue(8),
            line("Base fix #8 closed; merging main in again"),
        ]);
        assert_eq!(outside.did(), did);
        assert_eq!(base_fix.report(), Some(format!("{FIX_URL} closed")));
    }

    #[test]
    fn a_covering_issue_whose_run_from_here_ends_with_it_still_open_fails_the_run_naming_it() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script {
            open_fixes: vec![(8, "CI red on main: test")],
            marks: vec![(8, Marked::Running)],
            ended: VecDeque::from([false, true]),
            open: VecDeque::from([true, true]),
            ..Script::default()
        });

        let result = fix(&mut base_fix, &mut outside, &inherited(&["test"]));

        assert_eq!(
            error(result),
            format!("Base fix {FIX_URL} ended with its issue still open")
        );
        let mut did = finds_fix_8();
        did.extend([
            Did::ReadEnded(8),
            Did::ReadIssue(8),
            Did::Pause,
            Did::ReadEnded(8),
            Did::ReadIssue(8),
        ]);
        assert_eq!(outside.did(), did);
        assert_eq!(base_fix.report(), Some(format!("{FIX_URL} not closed")));
    }

    #[test]
    fn a_covering_issue_with_no_mark_here_is_waited_on_by_the_issue_alone() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script {
            open_fixes: vec![(8, "CI red on main: test")],
            open: VecDeque::from([true, true, false]),
            ..Script::default()
        });

        fix(&mut base_fix, &mut outside, &inherited(&["test"])).unwrap();

        let mut did = finds_fix_8();
        did.extend([
            Did::ReadIssue(8),
            Did::Pause,
            Did::ReadIssue(8),
            Did::Pause,
            Did::ReadIssue(8),
            line("Base fix #8 closed; merging main in again"),
        ]);
        assert_eq!(outside.did(), did);
        assert_eq!(base_fix.report(), Some(format!("{FIX_URL} closed")));
    }

    #[test]
    fn a_covering_issue_left_by_a_run_from_here_is_started_again_with_no_issue_written() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script {
            open_fixes: vec![(8, "CI red on main: test")],
            marks: vec![(8, Marked::Left)],
            ..Script::default()
        });

        fix(&mut base_fix, &mut outside, &inherited(&["test"])).unwrap();

        assert_eq!(
            outside.did(),
            [
                Did::Look,
                Did::ListFixes,
                Did::ReadMark(8),
                line(&format!(
                    "Base fix #8 is open but no longer running; \
                     starting it again into main: {FIX_URL}"
                )),
                Did::MarkRunning(8),
                Did::LookReleased,
                line("waiting on Base fix #8"),
                Did::RunBaseFix(8, "main".to_string()),
                Did::Merged(8),
                Did::RunningReleased(8),
                line("Base fix #8 merged; merging main in again"),
            ]
        );
        assert_eq!(base_fix.report(), Some(format!("{FIX_URL} merged")));
    }

    #[test]
    fn an_interrupt_while_waiting_on_a_covering_issue_fails_the_run_as_interrupted() {
        let mut base_fix = asked(BaseFixAsk::Allow);
        let mut outside = Scripted::new(Script {
            open_fixes: vec![(8, "CI red on main: test")],
            open: VecDeque::from([true]),
            interrupted: true,
            ..Script::default()
        });

        let result = fix(&mut base_fix, &mut outside, &inherited(&["test"]));

        assert_eq!(error(result), "interrupted");
        let mut did = finds_fix_8();
        did.extend([Did::ReadIssue(8), Did::Pause]);
        assert_eq!(outside.did(), did);
        assert_eq!(base_fix.report(), Some(format!("{FIX_URL} not closed")));
    }

    /// The number of the issue `covering` finds among `open` for the Run on
    /// #7 into main, with Inherited failures `checks`.
    fn covering_number(open: &[(u64, &str)], checks: &[&str]) -> Option<u64> {
        let open = open
            .iter()
            .map(|&(number, title)| listed(number, title))
            .collect();
        covering(open, &url(7), "main", &inherited(checks).inherited).map(|issue| issue.number)
    }

    #[test]
    fn the_covering_issue_is_the_oldest_naming_the_base_branch_and_every_check() {
        let open = [
            (12, "CI red on main: test"),
            (9, "CI red on main: lint, test"),
            (5, "CI red on develop: test"),
            (4, "CI red on main: lint"),
            (3, "CI red on main: tests"),
            (2, "CI red on mainline: test"),
        ];
        assert_eq!(covering_number(&open, &["test"]), Some(9));
        assert_eq!(covering_number(&open, &["lint", "test"]), Some(9));
        assert_eq!(covering_number(&open, &["build"]), None);
    }

    #[test]
    fn the_runs_own_issue_never_covers_its_checks() {
        assert_eq!(
            covering_number(&[(7, "CI red on main: test")], &["test"]),
            None
        );
    }

    #[test]
    fn a_check_whose_name_has_a_comma_is_found_among_the_checks_an_issue_names() {
        let open = [(8, "CI red on main: lint, test (ubuntu, stable)")];
        assert_eq!(covering_number(&open, &["test (ubuntu, stable)"]), Some(8));
        assert_eq!(covering_number(&open, &["ubuntu"]), None);
    }

    #[test]
    fn report_says_whether_the_base_fix_taken_reached_its_end() {
        let report = |started, reached| {
            let mut base_fix = asked(BaseFixAsk::Allow);
            base_fix.taken = Some(Taken {
                issue: url(8),
                started,
                reached,
            });
            base_fix.report().unwrap()
        };
        assert_eq!(report(true, true), format!("{FIX_URL} merged"));
        assert_eq!(report(true, false), format!("{FIX_URL} not merged"));
        assert_eq!(report(false, true), format!("{FIX_URL} closed"));
        assert_eq!(report(false, false), format!("{FIX_URL} not closed"));
        assert_eq!(asked(BaseFixAsk::Allow).report(), None);
    }

    #[test]
    fn tickets_ask_what_the_spec_run_asked() {
        for ask in [BaseFixAsk::Allow, BaseFixAsk::Forbid, undecided()] {
            assert_eq!(asked(ask.clone()).ask_of_tickets(), ask);
        }
    }

    #[test]
    fn a_run_that_is_itself_a_base_fix_sees_no_inherited_failures_and_starts_none() {
        let child = Kind::BaseFix {
            base: "main".to_string(),
        };
        for ask in [BaseFixAsk::Allow, BaseFixAsk::Forbid, undecided()] {
            let base_fix = BaseFix::new(Some(&child), ask);
            assert!(!base_fix.sees_inherited_failures());
            assert_eq!(base_fix.ask_of_tickets(), BaseFixAsk::Forbid);
        }
        let ticket = Kind::Ticket {
            spec_branch: "spec-3".to_string(),
        };
        assert!(BaseFix::new(Some(&ticket), BaseFixAsk::Allow).sees_inherited_failures());
        assert!(asked(BaseFixAsk::Forbid).sees_inherited_failures());
    }
}
