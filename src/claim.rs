//! The Claim: the mark that the factory has taken an issue, the label
//! `in-progress` in place of `ready-for-agent`, so the issue list shows what a
//! Run or a Spec run is working on. It is made before the run's work, and
//! ended once with how the run ended: removed once its Self-merge has left the
//! issue closed, kept while its pull request waits for review, and released
//! when the run failed with nothing on origin to take over.
//!
//! What it reads and changes of GitHub and origin, whether the run is
//! interrupted, and its progress lines all go through [`Outside`]:
//! [`OnGitHub`] does each through `gh`, `git` and the progress lines;
//! `InMemory`, in tests, from memory, recording each call.

use anyhow::{Context, Result};

use crate::branch;
use crate::git::Git;
use crate::github;
use crate::interrupt;
use crate::issue::IssueUrl;
use crate::labels::{self, Edit, Label, Labels, READY_FOR_AGENT};
use crate::progress;
use crate::run::Goal;

/// The label of a Claimed issue.
pub const IN_PROGRESS: Label =
    Label::new("in-progress", "A Claim: thirdshift has taken this issue");

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
}

/// Make the Claim on `issue`, from its Launch directory `launch`: label it
/// `in-progress`, in place of `ready-for-agent` if it has that, in one
/// request that keeps its other labels, having added `in-progress` to the
/// repository if it lacks it. An issue already Claimed, `in-progress` and not
/// `ready-for-agent`, is left as it is, with no request made. A failure names
/// the Claim as what could not be made.
pub fn make<'a>(issue: &'a IssueUrl, launch: &'a Git) -> Result<Claim<'a>> {
    let claimed = Claimed::make(&mut OnGitHub { launch }, issue)?;
    Ok(Claim { launch, claimed })
}

impl Claim<'_> {
    /// End the Claim once the Run or the Spec run that made it has ended,
    /// having reached `reached`, or none in a Failed run:
    ///
    /// - Merged: the Claim is removed if the Self-merge left the issue
    ///   closed.
    /// - Ready for review: the Claim stays while the pull request waits.
    /// - Failed: the Claim is released if nothing of the run is on origin to
    ///   take over.
    ///
    /// The run has ended as it has, so this never fails: a failure is a
    /// warning naming what to run by hand, and an interrupt doesn't stop it.
    pub fn end(self, reached: Option<Goal>) {
        let launch = self.launch.completion();
        self.claimed.end(&mut OnGitHub { launch: &launch }, reached);
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
    /// Make `edit`.
    fn apply(&mut self, edit: &Edit) -> Result<()>;
    /// Whether the run was interrupted.
    fn interrupted(&mut self) -> bool;
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
}

impl Outside for OnGitHub<'_> {
    fn labels(&mut self, issue: &IssueUrl) -> Result<Labels> {
        github::issue_labels(issue)
    }

    fn state(&mut self, issue: &IssueUrl) -> Result<State> {
        let issue = github::issue(issue)?;
        Ok(State {
            is_open: issue.is_open,
            labels: issue.labels,
        })
    }

    fn started(&mut self, issue: &IssueUrl) -> Result<bool> {
        Ok(branch::started(self.launch, issue)?.is_some())
    }

    fn apply(&mut self, edit: &Edit) -> Result<()> {
        edit.apply()
    }

    fn interrupted(&mut self) -> bool {
        interrupt::requested()
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
            &[READY_FOR_AGENT],
            &[IN_PROGRESS],
        );
        let claimed = Claimed {
            issue,
            added_in_progress: edit.puts_on(IN_PROGRESS),
            removed_ready_for_agent: edit.takes_off(READY_FOR_AGENT),
        };
        if !claimed.changed_any() {
            return Ok(claimed);
        }
        if claimed.removed_ready_for_agent {
            outside.step(format!(
                "labelling #{} {IN_PROGRESS}, in place of {READY_FOR_AGENT}",
                issue.number
            ));
        } else {
            outside.step(format!("labelling #{} {IN_PROGRESS}", issue.number));
        }
        outside.apply(&edit)?;
        Ok(claimed)
    }

    /// [`Claim::end`], through `outside`.
    fn end(&self, outside: &mut impl Outside, reached: Option<Goal>) {
        match reached {
            Some(Goal::Merged) => self.remove_if_closed(outside),
            Some(Goal::ReadyForReview) => {}
            None => self.release_if_nothing_on_origin(outside),
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
    /// request.
    fn release_if_nothing_on_origin(&self, outside: &mut impl Outside) {
        if !self.changed_any() {
            return;
        }
        if let Err(error) = retry_if_interrupted(outside, |outside| {
            self.put_labels_back_unless_on_origin(outside)
        }) {
            outside.warn(
                &error,
                format!(
                    "could not release the Claim on #{}, so if nothing of the run is on origin, \
                     release it by hand: {}",
                    self.issue.number,
                    self.release_by_hand()
                ),
            );
        }
    }

    /// [`Claimed::release_if_nothing_on_origin`], its failure as `git` or `gh`
    /// gave it.
    fn put_labels_back_unless_on_origin(&self, outside: &mut impl Outside) -> Result<()> {
        let issue = self.issue;
        if outside.started(issue)? {
            return Ok(());
        }
        let (off, on) = self.release_labels();
        let edit = Edit::of(issue, outside.labels(issue)?, off, on);
        if !is_on(edit.labels()) {
            return Ok(());
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
        outside.apply(&edit)
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
fn retry_if_interrupted<O: Outside>(
    outside: &mut O,
    step: impl Fn(&mut O) -> Result<()>,
) -> Result<()> {
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
    #[derive(Debug, PartialEq, Eq)]
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

        claimed.end(&mut issue, Some(Goal::Merged));

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

        claimed.end(&mut issue, Some(Goal::Merged));

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

        claimed.end(&mut issue, Some(Goal::Merged));

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

            claimed.end(&mut issue, Some(Goal::Merged));

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

        claimed.end(&mut issue, Some(Goal::ReadyForReview));

        assert_eq!(issue.calls, []);
        assert_eq!(labels(&issue), ["in-progress"]);
    }

    #[test]
    fn a_failed_run_started_on_origin_keeps_the_claim() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.started = true;

        claimed.end(&mut issue, None);

        assert_eq!(issue.calls, [Call::Started]);
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

            claimed.end(&mut issue, None);

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

        claimed.end(&mut issue, None);

        assert_eq!(labels(&issue), ["needs-info", "ready-for-agent"]);
    }

    #[test]
    fn an_issue_whose_claim_someone_took_off_meanwhile_is_left_as_it_is() {
        let url = seven();
        let mut issue = InMemory::labelled(&["ready-for-agent"]);
        let claimed = claimed(&mut issue, &url);
        issue.labels = vec!["ready-for-human".to_string()];

        claimed.end(&mut issue, None);

        assert_eq!(issue.calls, [Call::Started, Call::Labels]);
        assert_eq!(labels(&issue), ["ready-for-human"]);
    }

    #[test]
    fn a_claim_that_changed_no_label_makes_no_request_when_the_run_fails() {
        let url = seven();
        let mut issue = InMemory::labelled(&["in-progress"]);
        let claimed = claimed(&mut issue, &url);

        claimed.end(&mut issue, None);

        assert_eq!(issue.calls, []);
    }

    #[test]
    fn a_claim_that_cannot_be_released_is_a_warning_naming_the_commands_to_run_by_hand() {
        for failing in [Fails::Started, Fails::Labels, Fails::Edit] {
            let url = seven();
            let mut issue = InMemory::labelled(&["ready-for-agent"]);
            let claimed = claimed(&mut issue, &url);
            issue = issue.failing(failing);

            claimed.end(&mut issue, None);

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

        claimed.end(&mut issue, None);

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

        claimed.end(&mut issue, Some(Goal::Merged));

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

        claimed.end(&mut issue, None);

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
