//! The Pass seam: what an Architect run's or a Pickup run's gates, which
//! decide before any work whether it is skipped, read and change of GitHub,
//! and what an Architect run's conclusion after its Architecture review
//! does, viewing and labelling the issue the review ended on.
//! [`OnGitHub`] does each through `gh`, the Ready issue search, the
//! interrupt flag and the progress lines; [`InMemory`], in tests, from
//! memory, recording each call.

use anyhow::Result;

use crate::git::Git;
use crate::github::{self, Issue, ListedIssue};
use crate::interrupt;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Edit, Label};
use crate::progress;
use crate::ready::{self, ReadyIssue};

/// What a pass's gates, and an Architect run's conclusion, read and change of
/// its repository on GitHub, whether the run is interrupted, and where their
/// progress lines go.
pub trait Outside {
    /// Every open issue in the repository labelled `label`.
    fn open_issues(&mut self, label: Label) -> Result<Vec<ListedIssue>>;
    /// Every closed issue in the repository labelled `label`.
    fn closed_issues(&mut self, label: Label) -> Result<Vec<ListedIssue>>;
    /// `issue`'s state, labels and when it was created.
    fn issue(&mut self, issue: &IssueUrl) -> Result<Issue>;
    /// Make `edit`.
    fn apply(&mut self, edit: &Edit) -> Result<()>;
    /// Whether the run was interrupted.
    fn interrupted(&mut self) -> bool;
    /// The repository's lowest-numbered Ready issue, if it has one, by the
    /// Ready issue search, with its lines on the issues passed over.
    fn ready_issue(&mut self) -> Result<Option<ReadyIssue>>;
    /// Hand on the progress line `line`.
    fn step(&mut self, line: String);
}

/// The repository `repo` on GitHub, from its Launch directory `launch`.
pub struct OnGitHub<'a> {
    pub launch: &'a Git,
    pub repo: &'a Repo,
}

impl Outside for OnGitHub<'_> {
    fn open_issues(&mut self, label: Label) -> Result<Vec<ListedIssue>> {
        github::open_issues_labelled(&self.repo.slug(), label)
    }

    fn closed_issues(&mut self, label: Label) -> Result<Vec<ListedIssue>> {
        github::closed_issues_labelled(&self.repo.slug(), label)
    }

    fn issue(&mut self, issue: &IssueUrl) -> Result<Issue> {
        github::issue(issue)
    }

    fn apply(&mut self, edit: &Edit) -> Result<()> {
        edit.apply()
    }

    fn interrupted(&mut self) -> bool {
        interrupt::requested()
    }

    fn ready_issue(&mut self) -> Result<Option<ReadyIssue>> {
        ready::first(self.launch, self.repo)
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }
}

#[cfg(test)]
pub use in_memory::{Call, InMemory};

#[cfg(test)]
mod in_memory {
    use anyhow::{Result, bail};

    use super::Outside;
    use crate::github::{Issue, ListedIssue};
    use crate::issue::IssueUrl;
    use crate::labels::{Edit, Label, Labels};
    use crate::ready::ReadyIssue;

    /// A call a pass's gates made, in the order they made it.
    #[derive(Debug, PartialEq, Eq)]
    pub enum Call {
        /// They listed the open issues with this label.
        Open(&'static str),
        /// They listed the closed issues with this label.
        Closed(&'static str),
        /// They viewed this issue.
        View(u64),
        /// They applied an Edit to this issue, taking `off` off it and
        /// putting `on` on it, which leaves it with `labels`.
        Edit {
            issue: u64,
            off: Vec<&'static str>,
            on: Vec<&'static str>,
            labels: Vec<String>,
        },
        /// They asked for the Ready issue.
        ReadySearch,
        /// They handed on this progress line.
        Step(String),
    }

    /// A repository in memory: its issues, filed by label, open or
    /// closed, the issues it can view, the Ready issue search's answer, the
    /// issues whose Edits fail, and whether the run was interrupted.
    #[derive(Default)]
    pub struct InMemory {
        /// Each issue, filed under the label a listing finds it by.
        filed: Vec<Filed>,
        /// Each issue it can view, by number. Viewing any other fails.
        viewable: Vec<(u64, Issue)>,
        /// Whether the run was interrupted.
        interrupted: bool,
        /// Whether listing the closed issues fails.
        closed_listing_fails: bool,
        /// The number of each issue an Edit to fails.
        failing_edits: Vec<u64>,
        /// The Ready issue search's answer: the issue, and whether it is a
        /// Spec.
        ready: Option<(ListedIssue, bool)>,
        /// Every call made, in order.
        pub calls: Vec<Call>,
    }

    /// An issue filed under a label.
    struct Filed {
        /// The label, as spelled.
        under: String,
        open: bool,
        listed: ListedIssue,
    }

    impl InMemory {
        /// Add issue `number`, open or not, labelled `labels` and filed
        /// under each.
        pub fn issue(mut self, number: u64, open: bool, labels: &[&str]) -> Self {
            for label in labels {
                self = self.filed(label, number, open, labels);
            }
            self
        }

        /// Add issue `number`, open or not, labelled `labels`, filed only
        /// under `under`, whether it has that label or not: as a listing
        /// gives an issue whose labels changed since it was listed.
        pub fn filed(mut self, under: &str, number: u64, open: bool, labels: &[&str]) -> Self {
            self.filed.push(Filed {
                under: under.to_string(),
                open,
                listed: listed(number, labels),
            });
            self
        }

        /// Let issue `number` be viewed, as `issue`.
        pub fn viewable(mut self, number: u64, issue: Issue) -> Self {
            self.viewable.push((number, issue));
            self
        }

        /// Have the run interrupted.
        pub fn interrupted(mut self) -> Self {
            self.interrupted = true;
            self
        }

        /// Make listing the closed issues fail.
        pub fn closed_listing_failing(mut self) -> Self {
            self.closed_listing_fails = true;
            self
        }

        /// Make an Edit to issue `number` fail.
        pub fn edit_failing(mut self, number: u64) -> Self {
            self.failing_edits.push(number);
            self
        }

        /// Make the Ready issue search find issue `number`, a Spec or not.
        pub fn ready(mut self, number: u64, is_spec: bool) -> Self {
            self.ready = Some((listed(number, &[]), is_spec));
            self
        }

        /// The issues, open or not as `open` says, filed under `label`.
        fn labelled(&self, label: Label, open: bool) -> Vec<ListedIssue> {
            self.filed
                .iter()
                .filter(|filed| filed.open == open && label.is_named(&filed.under))
                .map(|filed| filed.listed.clone())
                .collect()
        }
    }

    impl Outside for InMemory {
        fn open_issues(&mut self, label: Label) -> Result<Vec<ListedIssue>> {
            self.calls.push(Call::Open(label.name()));
            Ok(self.labelled(label, true))
        }

        fn closed_issues(&mut self, label: Label) -> Result<Vec<ListedIssue>> {
            self.calls.push(Call::Closed(label.name()));
            if self.closed_listing_fails {
                bail!("gh: could not list the closed issues");
            }
            Ok(self.labelled(label, false))
        }

        fn issue(&mut self, issue: &IssueUrl) -> Result<Issue> {
            self.calls.push(Call::View(issue.number));
            match self
                .viewable
                .iter()
                .find(|(number, _)| *number == issue.number)
            {
                Some((_, viewed)) => Ok(viewed.clone()),
                None => bail!(
                    "gh: Could not resolve to an issue with the number of {}",
                    issue.number
                ),
            }
        }

        fn apply(&mut self, edit: &Edit) -> Result<()> {
            let (off, on) = edit.changes();
            self.calls.push(Call::Edit {
                issue: edit.issue().number,
                off: off.iter().map(|label| label.name()).collect(),
                on: on.iter().map(|label| label.name()).collect(),
                labels: edit.labels_after().names().map(String::from).collect(),
            });
            if self.failing_edits.contains(&edit.issue().number) {
                bail!("gh: could not edit #{}", edit.issue().number);
            }
            Ok(())
        }

        fn interrupted(&mut self) -> bool {
            self.interrupted
        }

        fn ready_issue(&mut self) -> Result<Option<ReadyIssue>> {
            self.calls.push(Call::ReadySearch);
            Ok(self
                .ready
                .clone()
                .map(|(listed, is_spec)| ReadyIssue { listed, is_spec }))
        }

        fn step(&mut self, line: String) {
            self.calls.push(Call::Step(line));
        }
    }

    /// Issue `number` in `acme/widgets` as a listing gives it, labelled
    /// `labels`.
    fn listed(number: u64, labels: &[&str]) -> ListedIssue {
        ListedIssue {
            issue: IssueUrl::parse(&format!("https://github.com/acme/widgets/issues/{number}"))
                .unwrap(),
            title: format!("Issue {number}"),
            labels: labels.iter().copied().collect::<Labels>(),
        }
    }
}
