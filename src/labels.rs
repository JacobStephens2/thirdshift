//! An issue's labels and the labels thirdshift knows: every rule about them,
//! so none is applied without GitHub's case-insensitivity. The triage labels
//! live here, as no one concept owns them; the labels of one concept, as the
//! Claim's `in-progress`, live with that concept. Every change thirdshift
//! makes to an issue's labels is an [`Edit`].

use std::fmt;

use anyhow::Result;

use crate::github;
use crate::issue::IssueUrl;

/// A label thirdshift knows. It prints as its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Label {
    name: &'static str,
    description: &'static str,
}

impl Label {
    /// The label `name`, added to a repository that lacks it with
    /// `description`.
    pub const fn new(name: &'static str, description: &'static str) -> Label {
        Label { name, description }
    }

    /// Its name, as thirdshift spells it.
    pub fn name(self) -> &'static str {
        self.name
    }

    /// The description it is added to a repository with.
    pub fn description(self) -> &'static str {
        self.description
    }

    /// Whether `name` names it, whatever its case: GitHub's label names are
    /// case-insensitive.
    pub fn is_named(self, name: &str) -> bool {
        name.eq_ignore_ascii_case(self.name)
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// The triage label of an issue no one has evaluated yet, which an
/// Architecture review publishes its plan with.
pub const NEEDS_TRIAGE: Label =
    Label::new("needs-triage", "Maintainer needs to evaluate this issue");

/// The triage label of an issue an agent can take on: what thirdshift swaps
/// a plan's `needs-triage` for once the plan passes its checks, what a Base
/// fix issue is opened with, and what a Claim takes off.
pub const READY_FOR_AGENT: Label = Label::new("ready-for-agent", "Ready for an agent to take on");

/// The triage labels that make an open Ticket an Unready Ticket, in the
/// order they are looked for.
pub const UNREADY: [Label; 4] = [
    Label::new("ready-for-human", "Requires human implementation"),
    Label::new("needs-info", "Waiting on reporter for more information"),
    Label::new("wontfix", "Will not be actioned"),
    NEEDS_TRIAGE,
];

/// An issue's labels, or a repository's, each spelled as GitHub spells it.
/// It has no `==`, which would compare names with case. Every question it
/// answers ignores case, as GitHub does.
#[derive(Debug, Clone, Default)]
pub struct Labels(Vec<String>);

impl<S: Into<String>> FromIterator<S> for Labels {
    fn from_iter<I: IntoIterator<Item = S>>(names: I) -> Labels {
        Labels(names.into_iter().map(Into::into).collect())
    }
}

impl Labels {
    /// Whether `label` is one of them, whatever its case.
    pub fn has(&self, label: Label) -> bool {
        self.spelled(label).is_some()
    }

    /// `label` as these labels spell it, if it is one of them.
    pub fn spelled(&self, label: Label) -> Option<&str> {
        self.0
            .iter()
            .map(String::as_str)
            .find(|name| label.is_named(name))
    }

    /// The first of the [`UNREADY`] labels among them, if any.
    pub fn unready(&self) -> Option<Label> {
        UNREADY.into_iter().find(|label| self.has(*label))
    }

    /// These labels with each of `off` taken off and then each of `on` put
    /// on, after the rest. One of `on` already here, whatever its case, is
    /// there once, as `on` spells it.
    pub fn swapped(&self, off: &[Label], on: &[Label]) -> Labels {
        let kept = self
            .0
            .iter()
            .filter(|name| !off.iter().chain(on).any(|label| label.is_named(name)))
            .cloned();
        Labels(
            kept.chain(on.iter().map(|label| label.name.to_string()))
                .collect(),
        )
    }

    /// Their names, as spelled, in order: what the GitHub adapter writes.
    /// Only for writing them: a question about them is one of the methods
    /// above, which ignore case.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

/// A change to an issue's labels: the labels to take off it and the labels
/// to put on it, against the labels it had when read. Asked first what it
/// will change, so a caller can say so, then applied.
pub struct Edit<'a> {
    issue: &'a IssueUrl,
    labels: Labels,
    off: Vec<Label>,
    on: Vec<Label>,
}

/// The request that makes an edit.
#[derive(Debug)]
enum Request {
    /// None: no label changes.
    None,
    /// Take this label off, as the issue spells it, keeping the labels added
    /// since the read.
    Delete(String),
    /// Add each of `missing` to the repository, unless it has it already,
    /// then write `labels` as all the issue's labels.
    Put { missing: Vec<Label>, labels: Labels },
}

impl<'a> Edit<'a> {
    /// Taking each of `off` off `issue` and putting each of `on` on it,
    /// against its labels as read now.
    pub fn read(issue: &'a IssueUrl, off: &[Label], on: &[Label]) -> Result<Edit<'a>> {
        Ok(Edit::of(issue, github::issue_labels(issue)?, off, on))
    }

    /// Taking each of `off` off `issue` and putting each of `on` on it,
    /// against `labels`, which the caller has just read.
    pub fn of(issue: &'a IssueUrl, labels: Labels, off: &[Label], on: &[Label]) -> Edit<'a> {
        Edit {
            issue,
            labels,
            off: off.to_vec(),
            on: on.to_vec(),
        }
    }

    /// Whether it takes `label` off: one of the labels to take off, and not
    /// one to put on, that the issue has, whatever its case.
    pub fn takes_off(&self, label: Label) -> bool {
        self.off.contains(&label) && !self.on.contains(&label) && self.labels.has(label)
    }

    /// Whether it puts `label` on: one of the labels to put on that the
    /// issue lacks, whatever its case.
    pub fn puts_on(&self, label: Label) -> bool {
        self.on.contains(&label) && !self.labels.has(label)
    }

    /// The issue's labels, as read.
    pub fn labels(&self) -> &Labels {
        &self.labels
    }

    /// Make the change, in one request, having first added each label it
    /// puts on to the repository, with its description, if the repository
    /// lacks it. A change of no label makes no request, and taking a single
    /// label off, putting none on, keeps the labels added since the read.
    pub fn apply(&self) -> Result<()> {
        match request(&self.labels, &self.off, &self.on) {
            Request::None => Ok(()),
            Request::Delete(label) => github::remove_label(self.issue, &label),
            Request::Put { missing, labels } => {
                if !missing.is_empty() {
                    github::ensure_labels(&self.issue.repo_slug(), &missing)?;
                }
                github::set_labels(self.issue, &labels)
            }
        }
    }
}

/// The request that takes each of `off` off an issue with `labels` and puts
/// each of `on` on it. A label in both is put on.
fn request(labels: &Labels, off: &[Label], on: &[Label]) -> Request {
    let taken_off: Vec<&str> = off
        .iter()
        .filter(|label| !on.contains(label))
        .filter_map(|label| labels.spelled(*label))
        .collect();
    let missing: Vec<Label> = on
        .iter()
        .copied()
        .filter(|label| !labels.has(*label))
        .collect();
    match (taken_off.as_slice(), missing.is_empty()) {
        ([], true) => Request::None,
        ([label], true) => Request::Delete(label.to_string()),
        _ => Request::Put {
            labels: labels.swapped(off, on),
            missing,
        },
    }
}

/// The commands that take each of `off` off `issue` and put each of `on` on
/// it, to run by hand, keeping its other labels.
pub fn by_hand(issue: &IssueUrl, off: &[Label], on: &[Label]) -> String {
    let removes = off
        .iter()
        .map(|label| github::remove_label_command(issue, *label));
    let adds = on
        .iter()
        .map(|label| github::add_label_command(issue, *label));
    removes.chain(adds).collect::<Vec<_>>().join(" && ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const IN_PROGRESS: Label = Label::new("in-progress", "A Claim");

    fn labels(names: &[&str]) -> Labels {
        names.iter().copied().collect()
    }

    fn names(labels: &Labels) -> Vec<&str> {
        labels.names().collect()
    }

    const ARCHITECT_PLAN: Label = Label::new("architect-plan", "An Architect plan");

    fn put(missing: &[Label], labels: &[&str]) -> Request {
        Request::Put {
            missing: missing.to_vec(),
            labels: self::labels(labels),
        }
    }

    /// `Request` as a comparable value: `Labels` has no `==`.
    fn compared(request: Request) -> (Option<String>, Vec<Label>, Vec<String>) {
        match request {
            Request::None => (None, Vec::new(), Vec::new()),
            Request::Delete(label) => (Some(label), Vec::new(), Vec::new()),
            Request::Put { missing, labels } => {
                (None, missing, labels.names().map(str::to_string).collect())
            }
        }
    }

    fn assert_request(actual: Request, expected: Request) {
        assert_eq!(compared(actual), compared(expected));
    }

    #[test]
    fn an_edit_that_changes_no_label_whatever_its_case_makes_no_request() {
        let labels = labels(&["bug", "In-Progress"]);

        assert_request(request(&labels, &[], &[]), Request::None);
        assert_request(request(&labels, &[READY_FOR_AGENT], &[]), Request::None);
        assert_request(
            request(&labels, &[READY_FOR_AGENT], &[IN_PROGRESS]),
            Request::None,
        );
    }

    #[test]
    fn taking_one_label_off_and_putting_none_on_deletes_it_as_the_issue_spells_it() {
        let triaged = labels(&["NEEDS-TRIAGE", "In-Progress"]);
        let labels = labels(&["bug", "In-Progress"]);

        assert_request(
            request(&labels, &[IN_PROGRESS], &[]),
            Request::Delete("In-Progress".into()),
        );
        assert_request(
            request(&triaged, &[IN_PROGRESS], &[NEEDS_TRIAGE]),
            Request::Delete("In-Progress".into()),
        );
    }

    #[test]
    fn a_label_both_taken_off_and_put_on_is_put_on() {
        let labels = labels(&["In-Progress", "bug"]);

        assert_request(
            request(&labels, &[IN_PROGRESS], &[IN_PROGRESS]),
            Request::None,
        );
        assert_request(
            request(&labels, &[IN_PROGRESS, NEEDS_TRIAGE], &[IN_PROGRESS]),
            Request::None,
        );
    }

    #[test]
    fn taking_two_labels_off_writes_the_rest() {
        let labels = labels(&["Needs-Triage", "bug", "in-progress"]);

        assert_request(
            request(&labels, &[NEEDS_TRIAGE, IN_PROGRESS], &[]),
            put(&[], &["bug"]),
        );
    }

    #[test]
    fn putting_labels_on_adds_those_the_issue_lacks_to_the_repository_first() {
        let labels = labels(&["Needs-Triage", "bug", "ARCHITECT-PLAN"]);

        assert_request(
            request(&labels, &[NEEDS_TRIAGE], &[READY_FOR_AGENT, ARCHITECT_PLAN]),
            put(
                &[READY_FOR_AGENT],
                &["bug", "ready-for-agent", "architect-plan"],
            ),
        );
    }

    #[test]
    fn a_claim_swaps_ready_for_agent_for_in_progress_writing_all_the_labels() {
        let labels = labels(&["Ready-For-Agent", "bug"]);

        assert_request(
            request(&labels, &[READY_FOR_AGENT], &[IN_PROGRESS]),
            put(&[IN_PROGRESS], &["bug", "in-progress"]),
        );
    }

    #[test]
    fn an_edit_says_what_it_takes_off_and_puts_on_whatever_the_case() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();
        let edit = Edit::of(
            &issue,
            labels(&["READY-FOR-AGENT", "In-Progress"]),
            &[READY_FOR_AGENT, NEEDS_TRIAGE],
            &[IN_PROGRESS, ARCHITECT_PLAN],
        );

        assert!(edit.takes_off(READY_FOR_AGENT));
        assert!(!edit.takes_off(NEEDS_TRIAGE));
        assert!(!edit.takes_off(IN_PROGRESS));
        assert!(!edit.puts_on(IN_PROGRESS));
        assert!(edit.puts_on(ARCHITECT_PLAN));
        assert!(!edit.puts_on(READY_FOR_AGENT));
        assert_eq!(names(edit.labels()), ["READY-FOR-AGENT", "In-Progress"]);
    }

    #[test]
    fn the_commands_by_hand_take_off_then_put_on() {
        let issue = IssueUrl::parse("https://github.com/acme/widgets/issues/7").unwrap();

        assert_eq!(by_hand(&issue, &[], &[]), "");
        assert_eq!(
            by_hand(&issue, &[IN_PROGRESS], &[READY_FOR_AGENT]),
            "gh api --method DELETE repos/acme/widgets/issues/7/labels/in-progress \
             && gh api --method POST repos/acme/widgets/issues/7/labels -f 'labels[]=ready-for-agent'"
        );
    }

    #[test]
    fn a_label_is_had_and_spelled_whatever_its_case() {
        let labels = labels(&["bug", "Ready-For-Agent"]);

        assert!(labels.has(READY_FOR_AGENT));
        assert_eq!(labels.spelled(READY_FOR_AGENT), Some("Ready-For-Agent"));
        assert!(!labels.has(NEEDS_TRIAGE));
        assert_eq!(labels.spelled(NEEDS_TRIAGE), None);
    }

    #[test]
    fn a_label_prints_as_its_name() {
        assert_eq!(NEEDS_TRIAGE.to_string(), "needs-triage");
        assert_eq!(format!("{READY_FOR_AGENT}"), "ready-for-agent");
    }

    #[test]
    fn the_first_unready_label_is_found_in_any_case() {
        assert_eq!(labels(&["bug"]).unready(), None);
        for label in UNREADY {
            let upper = label.name().to_uppercase();
            assert_eq!(labels(&["bug", &upper]).unready(), Some(label), "{label}");
        }
        assert_eq!(
            labels(&["Needs-Triage", "WONTFIX"])
                .unready()
                .map(Label::name),
            Some("wontfix")
        );
    }

    #[test]
    fn taking_off_a_label_takes_it_off_whatever_its_case() {
        let labels = labels(&["Needs-Triage", "bug"]);

        assert_eq!(names(&labels.swapped(&[NEEDS_TRIAGE], &[])), ["bug"]);
    }

    #[test]
    fn a_claim_swaps_ready_for_agent_for_in_progress_in_any_case() {
        let labels = labels(&["Ready-For-Agent", "bug", "IN-PROGRESS"]);

        assert_eq!(
            names(&labels.swapped(&[READY_FOR_AGENT], &[IN_PROGRESS])),
            ["bug", "in-progress"]
        );
    }

    #[test]
    fn releasing_a_claim_swaps_in_progress_for_ready_for_agent_in_any_case() {
        let labels = labels(&["bug", "In-Progress", "later"]);

        assert_eq!(
            names(&labels.swapped(&[IN_PROGRESS], &[READY_FOR_AGENT])),
            ["bug", "later", "ready-for-agent"]
        );
    }

    #[test]
    fn a_label_put_on_that_is_already_there_is_there_once_as_put_on() {
        let labels = labels(&["READY-FOR-AGENT", "bug"]);

        assert_eq!(
            names(&labels.swapped(&[], &[READY_FOR_AGENT])),
            ["bug", "ready-for-agent"]
        );
    }

    #[test]
    fn swapping_leaves_the_labels_it_was_asked_of_as_they_were() {
        let before = labels(&["Needs-Triage", "bug"]);
        let _ = before.swapped(&[NEEDS_TRIAGE], &[READY_FOR_AGENT]);

        assert_eq!(names(&before), ["Needs-Triage", "bug"]);
    }
}
