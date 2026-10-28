//! The Claim: the mark that the factory has taken an issue, the label
//! `in-progress` in place of `ready-for-agent`, so the issue list shows what a
//! Run or a Spec run is working on. It is made before the run's work, and
//! ended once with how the run ended: removed once its Self-merge has left the
//! issue closed, kept while its pull request waits for review, and released
//! when the run failed with nothing on origin to take over, except a failed
//! Security fix, which stays with the Day shift. A failed run that keeps its
//! Claim labels the issue `agent-failed`, which the Failure limit counts and
//! the next Claim takes off. A Stop instead leaves the issue for triage, or
//! explicitly releases it when asked to requeue. A Custom process's Delivery
//! removes it when the pull request is ready for review, without putting
//! `ready-for-agent` back.
//!
//! What it reads and changes of GitHub and origin, whether the run is
//! interrupted, and its progress lines all go through [`Outside`]:
//! [`OnGitHub`] does each through `gh`, `git` and the progress lines;
//! `InMemory`, in tests, from memory, recording each call.

use anyhow::{Context, Result};

use crate::branch;
use crate::git::Git;
use crate::github::GitHub;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::labels::{self, Edit, Label, Labels, READY_FOR_AGENT};
use crate::progress;
use crate::run::Goal;

/// The label of a Claimed issue.
pub const IN_PROGRESS: Label =
    Label::new("in-progress", "A Claim: thirdshift has taken this issue");

/// The label of an issue the factory failed on: a Failed ticket run or a
/// Failed spec run kept its Claim, and it waits for the Day shift. The
/// Failure limit counts the open issues that carry it.
pub const AGENT_FAILED: Label = Label::new(
    "agent-failed",
    "thirdshift failed on this issue; it waits for the Day shift",
);

/// How the Run or the Spec run that made a Claim ended, as ending the Claim
/// needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// It reached this goal.
    Reached(Goal),
    /// A bug Ticket run could not reproduce the bug, and handed it back to
    /// the reporter.
    NotReproduced,
    /// It failed with its pull request left ready for review: a policy,
    /// security or merge hold held its Self-merge.
    Held,
    /// It failed otherwise.
    Failed,
}

/// Whether an issue with `labels` carries a Claim, whatever else it is
/// labelled.
pub fn is_on(labels: &Labels) -> bool {
    labels.has(IN_PROGRESS)
}

/// The Claim a Run or a Spec run made on its issue from its Launch directory,
/// which it asks origin through when the Claim is released.
pub struct Claim<'a> {
    launch: &'a Git,
    claimed: Claimed<'a>,
    branch: String,
}

/// Make the Claim on `issue`, from its Launch directory `launch`: label it
/// `in-progress`, in place of `ready-for-agent` and `agent-failed` if it has
/// them, in one request that keeps its other labels, having added
/// `in-progress` to the repository if it lacks it. An issue already Claimed,
/// `in-progress` and not `ready-for-agent`, only has `agent-failed` taken
/// off, if it has that, a failure to do so a warning; with neither, no
/// request is made. A failure names the Claim as what could not be made.
pub fn make<'a>(issue: &'a IssueUrl, launch: &'a Git, branch: &str) -> Result<Claim<'a>> {
    let claimed = Claimed::make(
        &mut OnGitHub {
            launch,
            github: GitHub::new(),
            branch,
        },
        issue,
    )?;
    Ok(Claim {
        launch,
        claimed,
        branch: branch.into(),
    })
}

impl Claim<'_> {
    /// A bug Ticket run handed reproduction back to the reporter. End its Claim
    /// without making the issue Ready again, including in a Continuation.
    pub fn needs_info(self) {
        let launch = self.launch.completion();
        self.claimed.needs_info(&mut OnGitHub {
            launch: &launch,
            github: GitHub::new().completion(),
            branch: &self.branch,
        });
    }

    /// End the Claim once the Run or the Spec run that made it has ended as
    /// `ending` says:
    ///
    /// - Merged: the Claim is removed if the Self-merge left the issue
    ///   closed.
    /// - Ready for review, or held with its pull request ready: the Claim
    ///   stays while the pull request waits.
    /// - Not reproduced: as [`Claim::needs_info`].
    /// - Failed: the Claim is released if nothing of the run is on origin to
    ///   take over, except a Security fix, whose Claim stays. A Claim that
    ///   stays labels the issue `agent-failed` too.
    /// - Stopped: the issue goes to triage, keeping its Claim only while work
    ///   remains on origin, or is released even with work when asked to requeue.
    ///
    /// The run has ended as it has, so this never fails: a failure is a
    /// warning naming what to run by hand, and an interrupt doesn't stop it.
    pub fn end(self, ending: Ending) {
        let launch = self.launch.completion();
        self.claimed.end(
            &mut OnGitHub {
                launch: &launch,
                github: GitHub::new().completion(),
                branch: &self.branch,
            },
            ending,
        );
    }

    /// Take `in-progress` off the issue, whether or not it is open, and put
    /// nothing back. A Delivery that left a pull request ready for review
    /// ends this way: the issue is not Ready, so Pickup does not take it
    /// again while that pull request is open. A failure is a warning naming
    /// what to run by hand.
    pub fn remove(self) {
        let launch = self.launch.completion();
        self.claimed.remove(&mut OnGitHub {
            launch: &launch,
            github: GitHub::new().completion(),
            branch: &self.branch,
        });
    }
}

/// What the Claim reads and changes of its issue on GitHub and of origin,
/// whether the run is interrupted, and where its progress lines go.
trait Outside {
    /// The issue's labels.
    fn labels(&mut self, issue: &IssueUrl) -> Result<Labels>;
    /// Whether the issue is open, and its labels, read together.
    fn state(&mut self, issue: &IssueUrl) -> Result<State>;
    /// Whether the issue was started on origin: an Issue branch for it,
    /// which for a Spec is its Spec branch, or a pull request from one,
    /// open, merged or closed.
    fn started(&mut self, issue: &IssueUrl) -> Result<bool>;
    /// Whether this Run's selected Issue branch or an open PR from it remains.
    fn remaining(&mut self, issue: &IssueUrl) -> Result<bool>;
    /// Make `edit`.
    fn apply(&mut self, edit: &Edit) -> Result<()>;
    /// Whether the run was interrupted.
    fn interrupted(&mut self) -> bool;
    /// The requested Stop ending, if this Command is handling a Stop.
    fn stop(&mut self) -> Option<crate::logs::stop::Ending>;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
    /// Hand on `error`, then the warning `warning` saying what to do about it
    /// by hand.
    fn warn(&mut self, error: &anyhow::Error, warning: String);
}

/// Whether an issue is open, and its labels, as read together.
struct State {
    is_open: bool,
    labels: Labels,
}

/// The issue on GitHub, and origin as the Launch directory `launch` sees it.
struct OnGitHub<'a> {
    launch: &'a Git,
    github: GitHub,
    branch: &'a str,
}

impl Outside for OnGitHub<'_> {
    fn labels(&mut self, issue: &IssueUrl) -> Result<Labels> {
        self.github.issue_labels(issue)
    }

    fn state(&mut self, issue: &IssueUrl) -> Result<State> {
        let issue = self.github.issue(issue)?;
        Ok(State {
            is_open: issue.is_open,
            labels: issue.labels,
        })
    }

    fn started(&mut self, issue: &IssueUrl) -> Result<bool> {
        Ok(branch::started(self.launch, &self.github, issue)?.is_some())
    }

    fn remaining(&mut self, issue: &IssueUrl) -> Result<bool> {
        if self.launch.on_origin(self.branch)? {
            return Ok(true);
        }
        Ok(self
            .github
            .pull_requests_with_head_prefix(issue, self.branch)?
            .iter()
            .any(|pr| pr.head == self.branch && pr.state == crate::github::PrState::Open))
    }

    fn apply(&mut self, edit: &Edit) -> Result<()> {
        edit.apply(&self.github)
    }

    fn interrupted(&mut self) -> bool {
        interrupt::requested()
    }

    fn stop(&mut self) -> Option<crate::logs::stop::Ending> {
        crate::logs::stop_requested().map(|request| request.ending())
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }

    fn warn(&mut self, error: &anyhow::Error, warning: String) {
        progress::warn(error, format_args!("{warning}"));
    }
}

/// The Claim on an issue, with what making it changed, which is what
/// releasing it puts back.
struct Claimed<'a> {
    issue: &'a IssueUrl,
    /// Whether making it added `in-progress`: the issue was not Claimed
    /// already.
    added_in_progress: bool,
    /// Whether making it took `ready-for-agent` off the issue.
    removed_ready_for_agent: bool,
    /// A failed Security fix stays with the Day shift even without a push.
    security_fix: bool,
}

impl<'a> Claimed<'a> {
    /// [`make`], through `outside`.
    fn make(outside: &mut impl Outside, issue: &'a IssueUrl) -> Result<Claimed<'a>> {
        Claimed::label_in_progress(outside, issue)
            .with_context(|| format!("could not make the Claim on #{}", issue.number))
    }

    /// [`Claimed::make`], its failure as `gh` gave it.
    fn label_in_progress(outside: &mut impl Outside, issue: &'a IssueUrl) -> Result<Claimed<'a>> {
        let edit = Edit::of(
            issue,
            outside.labels(issue)?,
            &[READY_FOR_AGENT, AGENT_FAILED],
            &[IN_PROGRESS],
        );
        let claimed = Claimed {
            issue,
            added_in_progress: edit.puts_on(IN_PROGRESS),
            removed_ready_for_agent: edit.takes_off(READY_FOR_AGENT),
            security_fix: edit.labels().has(crate::security::fixing::SECURITY_FIX),
        };
        let removed_agent_failed = edit.takes_off(AGENT_FAILED);
        if !claimed.changed_any() {
            if removed_agent_failed {
                outside.step(format!(
                    "taking {AGENT_FAILED} off #{}, which is Claimed again",
                    issue.number
                ));
                if let Err(error) = outside.apply(&edit) {
                    outside.warn(
                        &error,
                        format!(
                            "could not take {AGENT_FAILED} off #{}, so take it off by hand: {}",
                            issue.number,
                            labels::by_hand(issue, &[AGENT_FAILED], &[])
                        ),
                    );
                }
            }
            return Ok(claimed);
        }
        let replaced: Vec<&str> = [
            (claimed.removed_ready_for_agent, READY_FOR_AGENT),
            (removed_agent_failed, AGENT_FAILED),
        ]
        .into_iter()
        .filter_map(|(removed, label)| removed.then_some(label.name()))
        .collect();
        if replaced.is_empty() {
            outside.step(format!("labelling #{} {IN_PROGRESS}", issue.number));
        } else {
            outside.step(format!(
                "labelling #{} {IN_PROGRESS}, in place of {}",
                issue.number,
                replaced.join(" and ")
            ));
        }
        outside.apply(&edit)?;
        Ok(claimed)
    }

    /// [`Claim::needs_info`], through `outside`.
    fn needs_info(&self, outside: &mut impl Outside) {
        let issue = self.issue;
        let result = (|| {
            let edit = Edit::of(
                issue,
                outside.labels(issue)?,
                &[IN_PROGRESS, READY_FOR_AGENT],
                &[labels::NEEDS_INFO],
            );
            outside.apply(&edit)
        })();
        if let Err(error) = result {
            outside.warn(
                &error,
                format!(
                    "could not end the Claim on #{} after diagnosis; run: {}",
                    issue.number,
                    labels::by_hand(
                        issue,
                        &[IN_PROGRESS, READY_FOR_AGENT],
                        &[labels::NEEDS_INFO]
                    ),
                ),
            );
        }
    }

    /// [`Claim::end`], through `outside`.
    fn end(&self, outside: &mut impl Outside, ending: Ending) {
        if let Some(ending) = outside.stop() {
            self.stopped(outside, ending);
            return;
        }
        match ending {
            Ending::Reached(Goal::Merged) => self.remove_if_closed(outside),
            Ending::Reached(Goal::ReadyForReview) | Ending::Held => {}
            Ending::NotReproduced => self.needs_info(outside),
            Ending::Failed if self.security_fix => self.label_failed(outside),
            Ending::Failed => {
                if self.release_if_nothing_on_origin(outside) {
                    self.label_failed(outside);
                }
            }
        }
    }

    /// Label the issue `agent-failed`, in one request that keeps its other
    /// labels, having added the label to the repository if it lacks it, if
    /// the issue still carries the Claim: the failure waits for the Day
    /// shift. One already labelled, or no longer `in-progress`, is left as
    /// it is. It finishes after an interrupt, and a failure is a warning
    /// naming what to run by hand.
    fn label_failed(&self, outside: &mut impl Outside) {
        let issue = self.issue;
        if let Err(error) = retry_if_interrupted(outside, |outside| {
            let edit = Edit::of(issue, outside.labels(issue)?, &[], &[AGENT_FAILED]);
            if !is_on(edit.labels()) || !edit.puts_on(AGENT_FAILED) {
                return Ok(());
            }
            outside.step(format!(
                "labelling #{} {AGENT_FAILED}, so it waits for the Day shift",
                issue.number
            ));
            outside.apply(&edit)
        }) {
            outside.warn(
                &error,
                format!(
                    "could not label #{} {AGENT_FAILED}, so label it by hand: {}",
                    issue.number,
                    labels::by_hand(issue, &[], &[AGENT_FAILED])
                ),
            );
        }
    }

    fn stopped(&self, outside: &mut impl Outside, ending: crate::logs::stop::Ending) {
        use crate::logs::stop::Ending;
        if let Err(error) = retry_if_interrupted(outside, |outside| {
            let state = outside.state(self.issue)?;
            if !state.is_open {
                return self.unlabel_if_closed(outside);
            }
            let (off, on): (&[Label], &[Label]) = match ending {
                Ending::Requeue => (&[IN_PROGRESS], &[READY_FOR_AGENT]),
                Ending::Triage if outside.remaining(self.issue)? => {
                    (&[READY_FOR_AGENT], &[labels::NEEDS_TRIAGE])
                }
                Ending::Triage => (&[IN_PROGRESS, READY_FOR_AGENT], &[labels::NEEDS_TRIAGE]),
            };
            outside.apply(&Edit::of(self.issue, state.labels, off, on))
        }) {
            outside.warn(
                &error,
                format!(
                    "could not end the Stop's Claim on #{}; check its labels by hand",
                    self.issue.number
                ),
            );
        }
    }

    /// Whether making the Claim changed any label.
    fn changed_any(&self) -> bool {
        self.added_in_progress || self.removed_ready_for_agent
    }

    /// Release the Claim if the issue was never started on origin, as a
    /// Ready issue never was. The issue's labels then go back as they were
    /// before the Claim: `in-progress` comes off if the Claim added it, and
    /// `ready-for-agent` goes back if the Claim took it off, in one request
    /// that keeps the issue's other labels, those added since included when
    /// only `in-progress` comes off. An issue that is no longer
    /// `in-progress` is left as it is: someone took the Claim off meanwhile.
    /// A Claim that changed no label has nothing to put back, and makes no
    /// request. Returns whether the Claim stays: the issue was started on
    /// origin, or was Claimed before this Claim. One released, left as it
    /// is, or whose release failed does not.
    fn release_if_nothing_on_origin(&self, outside: &mut impl Outside) -> bool {
        if !self.changed_any() {
            return true;
        }
        match retry_if_interrupted(outside, |outside| {
            self.put_labels_back_unless_on_origin(outside)
        }) {
            Ok(stays) => stays,
            Err(error) => {
                outside.warn(
                    &error,
                    format!(
                        "could not release the Claim on #{}, so if nothing of the run is on \
                         origin, release it by hand: {}",
                        self.issue.number,
                        self.release_by_hand()
                    ),
                );
                false
            }
        }
    }

    /// [`Claimed::release_if_nothing_on_origin`], its failure as `git` or `gh`
    /// gave it.
    fn put_labels_back_unless_on_origin(&self, outside: &mut impl Outside) -> Result<bool> {
        let issue = self.issue;
        if outside.started(issue)? {
            return Ok(true);
        }
        let (off, on) = self.release_labels();
        let edit = Edit::of(issue, outside.labels(issue)?, off, on);
        if !is_on(edit.labels()) {
            return Ok(false);
        }
        let number = issue.number;
        outside.step(if !self.removed_ready_for_agent {
            format!("releasing the Claim on #{number}: removing {IN_PROGRESS}")
        } else if self.added_in_progress {
            format!(
                "releasing the Claim on #{number}: labelling it {READY_FOR_AGENT}, \
                 in place of {IN_PROGRESS}"
            )
        } else {
            format!("releasing the Claim on #{number}: labelling it {READY_FOR_AGENT} again")
        });
        outside.apply(&edit).map(|()| false)
    }

    /// The labels releasing the Claim takes off and puts on, in that order:
    /// `in-progress` off if making it added that, and `ready-for-agent` on if
    /// making it took that off.
    fn release_labels(&self) -> (&'static [Label], &'static [Label]) {
        let off: &[Label] = if self.added_in_progress {
            &[IN_PROGRESS]
        } else {
            &[]
        };
        let on: &[Label] = if self.removed_ready_for_agent {
            &[READY_FOR_AGENT]
        } else {
            &[]
        };
        (off, on)
    }

    /// Take `in-progress` off, open issue or not, and put nothing on. An issue
    /// that is no longer `in-progress` is left as it is.
    fn remove(&self, outside: &mut impl Outside) {
        if let Err(error) = retry_if_interrupted(outside, |outside| self.take_off(outside)) {
            outside.warn(
                &error,
                format!(
                    "could not remove {IN_PROGRESS} from issue #{}, so remove it by hand: {}",
                    self.issue.number,
                    labels::by_hand(self.issue, &[IN_PROGRESS], &[])
                ),
            );
        }
    }

    /// [`Claimed::remove`], its failure as `gh` gave it.
    fn take_off(&self, outside: &mut impl Outside) -> Result<()> {
        let edit = Edit::of(self.issue, outside.labels(self.issue)?, &[IN_PROGRESS], &[]);
        if !edit.takes_off(IN_PROGRESS) {
            return Ok(());
        }
        outside.step(format!(
            "removing {IN_PROGRESS} from issue #{}",
            self.issue.number
        ));
        outside.apply(&edit)
    }

    /// Remove the Claim if the Self-merge has left its issue closed:
    /// `in-progress` comes off, in one request that keeps the issue's other
    /// labels. An issue still open, as when closing it failed, keeps it, and
    /// one no longer `in-progress` is left as it is.
    fn remove_if_closed(&self, outside: &mut impl Outside) {
        if let Err(error) = retry_if_interrupted(outside, |outside| self.unlabel_if_closed(outside))
        {
            outside.warn(
                &error,
                format!(
                    "could not remove {IN_PROGRESS} from issue #{}, so remove it by hand: {}",
                    self.issue.number,
                    labels::by_hand(self.issue, &[IN_PROGRESS], &[])
                ),
            );
        }
    }

    /// [`Claimed::remove_if_closed`], its failure as `gh` gave it.
    fn unlabel_if_closed(&self, outside: &mut impl Outside) -> Result<()> {
        let state = outside.state(self.issue)?;
        if state.is_open {
            return Ok(());
        }
        let edit = Edit::of(self.issue, state.labels, &[IN_PROGRESS], &[]);
        if !edit.takes_off(IN_PROGRESS) {
            return Ok(());
        }
        outside.step(format!(
            "removing {IN_PROGRESS} from issue #{}",
            self.issue.number
        ));
        outside.apply(&edit)
    }

    /// The commands that release the Claim by hand.
    fn release_by_hand(&self) -> String {
        let (off, on) = self.release_labels();
        labels::by_hand(self.issue, off, on)
    }
}

/// Run `step` through `outside`, and once more if it failed with the run
/// interrupted: Ctrl-C in a terminal also kills the git or gh the step was
/// running.
fn retry_if_interrupted<O: Outside, T>(
    outside: &mut O,
    step: impl Fn(&mut O) -> Result<T>,
) -> Result<T> {
    match step(outside) {
        Err(_) if outside.interrupted() => step(outside),
        done => done,
    }
}

#[cfg(test)]
mod in_memory {
    use anyhow::{Result, bail};

    use super::{Outside, State};
    use crate::issue::IssueUrl;
    use crate::labels::{Edit, Labels};

    /// A read or a change of [`Outside`] that can fail.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fails {
        Labels,
        State,
        Started,
        Edit,
    }

    /// A call the Claim made, in the order it made it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Call {
        /// It read the issue's labels.
        Labels,
        /// It read whether the issue is open, and its labels.
        State,
        /// It asked whether the issue was started on origin.
        Started,
        /// It applied an Edit, taking `off` off the issue and putting `on`
        /// on it, which leaves it with `labels`.
        Edit {
            off: Vec<&'static str>,
            on: Vec<&'static str>,
            labels: Vec<String>,
        },
        /// It asked whether the run was interrupted.
        Interrupted,
        /// It handed on this progress line.
        Step(String),
        /// It handed on this error, then this warning.
        Warn { error: String, warning: String },
    }

    /// An issue in memory: its labels, whether it is open and was started
    /// on origin, whether the run was interrupted, and which reads or edits
    /// fail, and how many times more.
    #[derive(Default)]
    pub struct InMemory {
        /// The issue's labels, as spelled, which an Edit made changes.
        pub labels: Vec<String>,
        pub open: bool,
        pub started: bool,
        pub interrupted: bool,
        pub stop: Option<crate::logs::stop::Ending>,
        /// Each read or edit that fails, with how many times more it does.
        failing: Vec<(Fails, usize)>,
        /// Every call made, in order.
        pub calls: Vec<Call>,
    }

    impl InMemory {
        /// An open issue labelled `labels`, not started on origin.
        pub fn labelled(labels: &[&str]) -> Self {
            InMemory {
                labels: labels.iter().map(|label| label.to_string()).collect(),
                open: true,
                ..InMemory::default()
            }
        }

        /// Make `what` fail every time.
        pub fn failing(self, what: Fails) -> Self {
            self.failing_times(what, usize::MAX)
        }

        /// Make `what` fail `times` times, then succeed.
        pub fn failing_times(mut self, what: Fails, times: usize) -> Self {
            self.failing.push((what, times));
            self
        }

        /// Fail if `what` is to fail, once fewer times from now on.
        fn fail_if(&mut self, what: Fails) -> Result<()> {
            match self
                .failing
                .iter_mut()
                .find(|(failing, _)| *failing == what)
            {
                Some((_, times)) if *times > 0 => {
                    *times -= 1;
                    bail!("gh: {what:?} failed: HTTP 502")
                }
                _ => Ok(()),
            }
        }

        fn read_labels(&self) -> Labels {
            self.labels.iter().map(String::as_str).collect()
        }
    }

    impl Outside for InMemory {
        fn labels(&mut self, _: &IssueUrl) -> Result<Labels> {
            self.calls.push(Call::Labels);
            self.fail_if(Fails::Labels)?;
            Ok(self.read_labels())
        }

        fn state(&mut self, _: &IssueUrl) -> Result<State> {
            self.calls.push(Call::State);
            self.fail_if(Fails::State)?;
            Ok(State {
                is_open: self.open,
                labels: self.read_labels(),
            })
        }

        fn started(&mut self, _: &IssueUrl) -> Result<bool> {
            self.calls.push(Call::Started);
            self.fail_if(Fails::Started)?;
            Ok(self.started)
        }

        fn remaining(&mut self, issue: &IssueUrl) -> Result<bool> {
            self.started(issue)
        }

        fn apply(&mut self, edit: &Edit) -> Result<()> {
            let (off, on) = edit.changes();
            let after = edit.labels_after();
            self.calls.push(Call::Edit {
                off: off.iter().map(|label| label.name()).collect(),
                on: on.iter().map(|label| label.name()).collect(),
                labels: after.names().map(String::from).collect(),
            });
            self.fail_if(Fails::Edit)?;
            self.labels = after.names().map(String::from).collect();
            Ok(())
        }

        fn interrupted(&mut self) -> bool {
            self.calls.push(Call::Interrupted);
            self.interrupted
        }

        fn stop(&mut self) -> Option<crate::logs::stop::Ending> {
            self.stop
        }

        fn step(&mut self, line: String) {
            self.calls.push(Call::Step(line));
        }

        fn warn(&mut self, error: &anyhow::Error, warning: String) {
            self.calls.push(Call::Warn {
                error: format!("{error:#}"),
                warning,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::in_memory::{Call, Fails, InMemory};
    use super::*;

    fn seven() -> IssueUrl {
        IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap()
    }

    fn step(line: &str) -> Call {
        Call::Step(line.to_string())
    }

    fn edit(off: &[&'static str], on: &[&'static str], labels: &[&str]) -> Call {
        Call::Edit {
            off: off.to_vec(),
            on: on.to_vec(),
            labels: labels.iter().map(|label| label.to_string()).collect(),
        }
    }

    fn labels(issue: &InMemory) -> Vec<&str> {
        issue.labels.iter().map(String::as_str).collect()
    }

    /// The Claim made on `issue`, with the calls making it made forgotten.
    fn claimed<'a>(issue: &mut InMemory, url: &'a IssueUrl) -> Claimed<'a> {
        let claimed = Claimed::make(issue, url).unwrap();
        issue.calls.clear();
        claimed
    }

    const RELEASE_BY_HAND: &str = "could not release the Claim on #7, so if nothing of the run \
         is on origin, release it by hand: \
         gh api --method DELETE repos/acme/widgets/issues/7/labels/in-progress && \
         gh api --method POST repos/acme/widgets/issues/7/labels -f 'labels[]=ready-for-agent'";

    #[test]
    fn a_stop_leaves_the_issue_for_triage_keeping_the_claim_only_with_work_on_origin() {
        for started in [false, true] {
            for before in [&["bug", "ready-for-agent"][..], &["bug", "in-progress"][..]] {
                let url = seven();
                let mut issue = InMemory::labelled(before);
                let claimed = claimed(&mut issue, &url);
                issue.started = started;
                issue.stop = Some(crate::logs::stop::Ending::Triage);
                issue.interrupted = true;
                claimed.end(&mut issue, Ending::Failed);
                assert_eq!(
                    labels(&issue),
                    if started {
                        vec!["bug", "in-progress", "needs-triage"]
                    } else {
                        vec!["bug", "needs-triage"]
                    }
                );
            }
        }
    }

    #[test]
    fn a_requeue_stop_releases_even_an_existing_claim_with_work_on_origin() {
        for started in [false, true] {
            let url = seven();
            let mut issue = InMemory::labelled(&["bug", "in-progress"]);
            let claimed = claimed(&mut issue, &url);
            issue.started = started;
            issue.stop = Some(crate::logs::stop::Ending::Requeue);
            claimed.end(&mut issue, Ending::Failed);
            assert_eq!(labels(&issue), ["bug", "ready-for-agent"]);
        }
    }

    #[test]
    fn a_stop_does_not_requeue_an_issue_already_closed_by_self_merge() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.open = false;
        issue.stop = Some(crate::logs::stop::Ending::Requeue);
        claimed.end(&mut issue, Ending::Reached(Goal::Merged));
        assert_eq!(labels(&issue), ["bug"]);
    }

    #[test]
    fn the_claim_is_made_in_place_of_ready_for_agent_keeping_the_other_labels() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent", "architecture"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("labelling #7 in-progress, in place of ready-for-agent"),
                edit(
                    &["ready-for-agent"],
                    &["in-progress"],
                    &["bug", "architecture", "in-progress"]
                ),
            ]
        );
    }

    #[test]
    fn the_claim_takes_agent_failed_off_with_ready_for_agent() {
        let url = seven();
        let mut issue = InMemory::labelled(&["agent-failed", "bug", "ready-for-agent"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("labelling #7 in-progress, in place of ready-for-agent and agent-failed"),
                edit(
                    &["ready-for-agent", "agent-failed"],
                    &["in-progress"],
                    &["bug", "in-progress"]
                ),
            ]
        );
    }

    #[test]
    fn a_claim_on_an_issue_already_claimed_takes_agent_failed_off() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress", "Agent-Failed", "bug"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("taking agent-failed off #7, which is Claimed again"),
                edit(&["agent-failed"], &[], &["bug", "in-progress"]),
            ]
        );
    }

    #[test]
    fn a_claim_that_cannot_take_agent_failed_off_warns_and_is_still_made() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress", "agent-failed"]).failing(Fails::Edit);

        let made = Claimed::make(&mut issue, &url);

        assert!(made.is_ok());
        assert_eq!(
            issue.calls.last(),
            Some(&Call::Warn {
                error: "gh: Edit failed: HTTP 502".to_string(),
                warning: "could not take agent-failed off #7, so take it off by hand: \
                     gh api --method DELETE repos/acme/widgets/issues/7/labels/agent-failed"
                    .to_string(),
            })
        );
    }

    #[test]
    fn an_issue_with_no_ready_for_agent_is_still_claimed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("labelling #7 in-progress"),
                edit(&[], &["in-progress"], &["bug", "in-progress"]),
            ]
        );
    }

    #[test]
    fn an_issue_already_claimed_is_left_as_it_is_with_no_request() {
        let url = seven();
        let mut issue = InMemory::labelled(&["In-Progress", "bug"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(issue.calls, [Call::Labels]);
    }

    #[test]
    fn a_claimed_issue_marked_ready_again_loses_ready_for_agent_and_stays_claimed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress", "bug", "ready-for-agent"]);

        Claimed::make(&mut issue, &url).unwrap();

        assert_eq!(
            issue.calls,
            [
                Call::Labels,
                step("labelling #7 in-progress, in place of ready-for-agent"),
                edit(&["ready-for-agent"], &[], &["bug", "in-progress"]),
            ]
        );
    }

    #[test]
    fn a_claim_that_cannot_be_made_names_the_claim_and_the_cause() {
        for failing in [Fails::Labels, Fails::Edit] {
            let url = seven();
            let mut issue = InMemory::labelled(&["ready-for-agent"]).failing(failing);

            let error = Claimed::make(&mut issue, &url).err().unwrap();

            assert_eq!(
                format!("{error:#}"),
                format!("could not make the Claim on #7: gh: {failing:?} failed: HTTP 502")
            );
            assert_eq!(labels(&issue), ["ready-for-agent"]);
        }
    }

    #[test]
    fn a_merged_run_removes_the_claim_from_its_closed_issue() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.open = false;

        claimed.end(&mut issue, Ending::Reached(Goal::Merged));

        assert_eq!(
            issue.calls,
            [
                Call::State,
                step("removing in-progress from issue #7"),
                edit(&["in-progress"], &[], &["bug"]),
            ]
        );
        assert_eq!(labels(&issue), ["bug"]);
    }

    #[test]
    fn a_merged_run_whose_issue_is_still_open_keeps_the_claim() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Reached(Goal::Merged));

        assert_eq!(issue.calls, [Call::State]);
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    #[test]
    fn a_merged_run_leaves_an_issue_no_longer_in_progress_as_it_is() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.open = false;
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, Ending::Reached(Goal::Merged));

        assert_eq!(issue.calls, [Call::State]);
    }

    #[test]
    fn a_claim_that_cannot_be_removed_is_a_warning_naming_the_command_to_run_by_hand() {
        for failing in [Fails::State, Fails::Edit] {
            let url = seven();
            let mut issue = InMemory::labelled(&["ready-for-agent"]);
            let claimed = claimed(&mut issue, &url);
            issue.open = false;
            issue = issue.failing(failing);

            claimed.end(&mut issue, Ending::Reached(Goal::Merged));

            assert_eq!(
                issue.calls.last(),
                Some(&Call::Warn {
                    error: format!("gh: {failing:?} failed: HTTP 502"),
                    warning: "could not remove in-progress from issue #7, so remove it by hand: \
                         gh api --method DELETE repos/acme/widgets/issues/7/labels/in-progress"
                        .to_string(),
                }),
                "{failing:?}"
            );
            assert_eq!(labels(&issue), ["in-progress"]);
        }
    }

    #[test]
    fn a_run_ready_for_review_keeps_the_claim_reading_nothing() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Reached(Goal::ReadyForReview));

        assert_eq!(issue.calls, []);
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    /// The lines and the Edit that label #7 `agent-failed`, leaving it with
    /// `labels`.
    fn labelling_failed(labels: &[&str]) -> [Call; 3] {
        [
            Call::Labels,
            step("labelling #7 agent-failed, so it waits for the Day shift"),
            edit(&[], &["agent-failed"], labels),
        ]
    }

    #[test]
    fn a_failed_run_started_on_origin_keeps_the_claim_and_labels_the_issue_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        claimed.end(&mut issue, Ending::Failed);

        let mut expected = vec![Call::Started];
        expected.extend(labelling_failed(&["bug", "in-progress", "agent-failed"]));
        assert_eq!(issue.calls, expected);
        assert_eq!(labels(&issue), ["bug", "in-progress", "agent-failed"]);
    }

    #[test]
    fn a_failed_security_fix_keeps_its_claim_with_nothing_on_origin_and_is_labelled_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["security-fix", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(
            issue.calls,
            labelling_failed(&["security-fix", "in-progress", "agent-failed"])
        );
    }

    #[test]
    fn a_run_held_with_its_pull_request_ready_keeps_the_claim_and_adds_no_label() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        claimed.end(&mut issue, Ending::Held);

        assert_eq!(issue.calls, []);
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    #[test]
    fn a_bug_not_reproduced_ends_the_claim_with_needs_info_and_no_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["bug", "ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        claimed.end(&mut issue, Ending::NotReproduced);

        assert_eq!(labels(&issue), ["bug", "needs-info"]);
    }

    #[test]
    fn a_stopped_run_with_work_on_origin_is_not_labelled_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;
        issue.stop = Some(crate::logs::stop::Ending::Triage);

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(labels(&issue), ["in-progress", "needs-triage"]);
    }

    #[test]
    fn a_failed_run_on_an_issue_already_labelled_agent_failed_makes_no_request() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels.push("Agent-Failed".to_string());

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(issue.calls, [Call::Labels]);
    }

    #[test]
    fn a_failed_run_whose_issue_lost_its_claim_meanwhile_is_not_labelled_agent_failed() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(issue.calls, [Call::Labels]);
        assert_eq!(labels(&issue), ["ready-for-human"]);
    }

    #[test]
    fn labelling_agent_failed_is_tried_once_more_after_an_interrupt_then_warned_of() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;
        issue.interrupted = true;
        issue = issue.failing(Fails::Edit);

        claimed.end(&mut issue, Ending::Failed);

        let labelling = labelling_failed(&["in-progress", "agent-failed"]);
        let mut expected = vec![Call::Started];
        expected.extend(labelling.clone());
        expected.push(Call::Interrupted);
        expected.extend(labelling);
        expected.push(Call::Warn {
            error: "gh: Edit failed: HTTP 502".to_string(),
            warning: "could not label #7 agent-failed, so label it by hand: \
                 gh api --method POST repos/acme/widgets/issues/7/labels -f 'labels[]=agent-failed'"
                .to_string(),
        });
        assert_eq!(issue.calls, expected);
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    #[test]
    fn a_failed_run_with_nothing_on_origin_puts_the_labels_back_as_they_were() {
        for (before, line, off, on) in [
            (
                &["bug", "ready-for-agent", "architecture"][..],
                "labelling it ready-for-agent, in place of in-progress",
                &["in-progress"][..],
                &["ready-for-agent"][..],
            ),
            (
                &["bug"][..],
                "removing in-progress",
                &["in-progress"][..],
                &[][..],
            ),
            (
                &["in-progress", "ready-for-agent"][..],
                "labelling it ready-for-agent again",
                &[][..],
                &["ready-for-agent"][..],
            ),
        ] {
            let url = seven();
            let mut issue = InMemory::labelled(before);
            let claimed = claimed(&mut issue, &url);

            claimed.end(&mut issue, Ending::Failed);

            let after: Vec<&str> = before
                .iter()
                .copied()
                .filter(|label| *label != "ready-for-agent")
                .chain(on.iter().copied())
                .collect();
            assert_eq!(
                issue.calls,
                [
                    Call::Started,
                    Call::Labels,
                    step(&format!("releasing the Claim on #7: {line}")),
                    edit(off, on, &after),
                ],
                "{before:?}"
            );
            assert_eq!(labels(&issue), after, "{before:?}");
        }
    }

    #[test]
    fn a_released_claim_keeps_a_label_the_issue_was_given_during_the_run() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["in-progress".to_string(), "needs-info".to_string()];

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(labels(&issue), ["needs-info", "ready-for-agent"]);
    }

    #[test]
    fn an_issue_whose_claim_someone_took_off_meanwhile_is_left_as_it_is() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(issue.calls, [Call::Started, Call::Labels]);
        assert_eq!(labels(&issue), ["ready-for-human"]);
    }

    #[test]
    fn a_claim_that_changed_no_label_keeps_it_and_labels_the_issue_agent_failed_when_the_run_fails()
    {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(
            issue.calls,
            labelling_failed(&["in-progress", "agent-failed"])
        );
    }

    #[test]
    fn a_claim_that_cannot_be_released_is_a_warning_naming_the_commands_to_run_by_hand() {
        for failing in [Fails::Started, Fails::Labels, Fails::Edit] {
            let url = seven();
            let mut issue = InMemory::labelled(&["ready-for-agent"]);
            let claimed = claimed(&mut issue, &url);
            issue = issue.failing(failing);

            claimed.end(&mut issue, Ending::Failed);

            assert_eq!(
                issue.calls.last(),
                Some(&Call::Warn {
                    error: format!("gh: {failing:?} failed: HTTP 502"),
                    warning: RELEASE_BY_HAND.to_string(),
                }),
                "{failing:?}"
            );
            assert_eq!(labels(&issue), ["in-progress"], "{failing:?}");
        }
    }

    #[test]
    fn a_release_that_fails_while_interrupted_is_tried_once_more() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.interrupted = true;
        issue = issue.failing_times(Fails::Edit, 1);

        claimed.end(&mut issue, Ending::Failed);

        let released = || {
            [
                Call::Started,
                Call::Labels,
                step(
                    "releasing the Claim on #7: labelling it ready-for-agent, in place of in-progress",
                ),
                edit(&["in-progress"], &["ready-for-agent"], &["ready-for-agent"]),
            ]
        };
        let mut expected: Vec<Call> = released().into();
        expected.push(Call::Interrupted);
        expected.extend(released());
        assert_eq!(issue.calls, expected);
        assert_eq!(labels(&issue), ["ready-for-agent"]);
    }

    #[test]
    fn a_removal_that_fails_while_interrupted_is_tried_once_more_then_warned_of() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.open = false;
        issue.interrupted = true;
        issue = issue.failing(Fails::State);

        claimed.end(&mut issue, Ending::Reached(Goal::Merged));

        assert_eq!(
            issue.calls[..3],
            [Call::State, Call::Interrupted, Call::State]
        );
        assert!(matches!(issue.calls[3], Call::Warn { .. }));
        assert_eq!(issue.calls.len(), 4);
    }

    #[test]
    fn a_failure_with_no_interrupt_is_not_tried_again() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue = issue.failing_times(Fails::Started, 1);

        claimed.end(&mut issue, Ending::Failed);

        assert_eq!(
            issue.calls,
            [
                Call::Started,
                Call::Interrupted,
                Call::Warn {
                    error: "gh: Started failed: HTTP 502".to_string(),
                    warning: RELEASE_BY_HAND.to_string(),
                },
            ]
        );
    }
}
