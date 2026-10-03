//! An issue's labels and the labels thirdshift knows: every rule about them,
//! so none is applied without GitHub's case-insensitivity. The triage labels
//! live here, as no one concept owns them; the labels of one concept, as the
//! Claim's `in-progress`, live with that concept.

use std::fmt;

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
