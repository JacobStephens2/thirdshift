//! The Pass seam: what an Architect run's or a Pickup run's gates, which
//! decide before any work whether it is skipped, read and change of GitHub,
//! what an Architect run's conclusion after its Architecture review does,
//! viewing and labelling the issue the review ended on, and what a pass
//! does outside itself around them: recording its skip or its start,
//! checking its Harness, and dispatching the issue it ends on.
//! [`LaunchAndGitHub`] does each through `gh`, the Ready issue search, the
//! interrupt flag, the progress lines, the logs, the Harness check and
//! [`run::run_to_end`]; [`InMemory`], in tests, from memory, recording each
//! call.

use std::fmt::Display;

use anyhow::Result;

use crate::asks::{Asks, Flags};
use crate::config::UserConfig;
use crate::github::{self, Issue, ListedIssue};
use crate::harness::Choice;
use crate::interrupt;
use crate::issue::{IssueUrl, Repo};
use crate::labels::{Edit, Label};
use crate::launch::LaunchDirectory;
use crate::logs::{self, Pass, Work};
use crate::progress;
use crate::ready::{self, ReadyIssue};
use crate::run::{self, Ended, StartedBy};

/// What a pass dispatches, as `thirdshift <Issue URL>` would run it, but on
/// the pass's Base branch `base`, whatever the Launch directory has checked
/// out.
pub enum Dispatch<'a> {
    /// An Architect run's Architect plan.
    #[expect(
        dead_code,
        reason = "the Architect run dispatches its plan through the Pass seam from #415"
    )]
    ArchitectPlan { plan: &'a IssueUrl, base: &'a str },
    /// The Ready issue a Pickup run took, a Spec or not, as `is_spec` says.
    ReadyIssue {
        issue: &'a IssueUrl,
        is_spec: bool,
        base: &'a str,
    },
}

/// What a pass's gates, and an Architect run's conclusion, read and change of
/// its repository on GitHub, whether the run is interrupted, and where their
/// progress lines go; and what the pass records in the logs, the check of
/// its Harness, and the run it dispatches.
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
    /// Record that `pass` was skipped for `reason`.
    fn skipped(&mut self, pass: Pass, reason: &dyn Display);
    /// Check the Harness, Model and Effort the pass's sessions, and the run
    /// it dispatches, run on, failing if they can't run.
    fn check_harness(&mut self) -> Result<()>;
    /// Record that the pass started `work`, on the Harness it checked.
    fn started(&mut self, work: Work);
    /// Run `dispatch` to its end, on the Harness the pass checked.
    fn dispatch(&mut self, dispatch: Dispatch) -> Ended;
}

/// The repository `repo` on GitHub, from its Launch directory `launch`, with
/// the logs, the pass's Harness choice `harness`, and the command's `flags`
/// and the User config `config`, which ask the run it dispatches.
pub struct LaunchAndGitHub<'a> {
    pub launch: &'a LaunchDirectory,
    pub repo: &'a Repo,
    /// Settled on the Harness's names for its Model and Effort once
    /// checked.
    pub harness: &'a mut Choice,
    pub flags: &'a Flags,
    pub config: &'a UserConfig,
}

impl Outside for LaunchAndGitHub<'_> {
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
        ready::first(self.launch.git(), self.repo)
    }

    fn step(&mut self, line: String) {
        progress::step(line);
    }

    fn skipped(&mut self, pass: Pass, reason: &dyn Display) {
        logs::skipped(pass, self.repo, reason);
    }

    fn check_harness(&mut self) -> Result<()> {
        self.harness.check()
    }

    fn started(&mut self, work: Work) {
        logs::started(work, self.harness);
    }

    /// The run's asks are the Architect plan's or the Ready issue's, from
    /// the command's flags and the User config, on the checked Harness.
    fn dispatch(&mut self, dispatch: Dispatch) -> Ended {
        let (issue, asks, base) = match dispatch {
            Dispatch::ArchitectPlan { plan, base } => (
                plan,
                Asks::of_architect_plan(plan, self.flags, self.config),
                base,
            ),
            Dispatch::ReadyIssue {
                issue,
                is_spec,
                base,
            } => (
                issue,
                Asks::of_ready_issue(issue, is_spec, self.flags, self.config),
                base,
            ),
        };
        let mut asks = Asks {
            harness: self.harness.clone(),
            ..asks
        };
        run::run_to_end(issue, &mut asks, StartedBy::Dispatch { base })
    }
}

#[cfg(test)]
pub use in_memory::{Call, InMemory};

#[cfg(test)]
mod in_memory {
    use std::fmt::Display;

    use anyhow::{Result, bail};

    use super::{Dispatch, Outside};
    use crate::github::{Issue, ListedIssue};
    use crate::issue::IssueUrl;
    use crate::labels::{Edit, Label, Labels};
    use crate::logs::{Pass, Work};
    use crate::ready::ReadyIssue;
    use crate::run::Ended;

    /// A call a pass made outside itself, in the order it made it.
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
        /// It recorded that it was skipped, for this reason.
        Skipped(String),
        /// It checked its Harness, Model and Effort.
        HarnessCheck,
        /// It recorded that it started work: on this issue, for a Pickup
        /// run, or on its repository, for an Architect run.
        Started(Option<u64>),
        /// It dispatched this issue, a Spec or not, on this Base branch.
        Dispatch {
            issue: u64,
            is_spec: bool,
            base: String,
        },
    }

    /// A repository in memory: its issues, filed by label, open or
    /// closed, the issues it can view, the Ready issue search's answer, the
    /// issues whose Edits fail, and whether the run was interrupted; with
    /// whether the Harness check fails and how a dispatched run ends.
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
        /// Whether the Harness check fails.
        harness_failing: bool,
        /// How the run dispatched ends.
        ending: Option<Ended>,
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

        /// Make the Harness check fail.
        pub fn harness_failing(mut self) -> Self {
            self.harness_failing = true;
            self
        }

        /// Have the run dispatched end as `ending`.
        pub fn dispatched_ending(mut self, ending: Ended) -> Self {
            self.ending = Some(ending);
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

        fn skipped(&mut self, _: Pass, reason: &dyn Display) {
            self.calls.push(Call::Skipped(reason.to_string()));
        }

        fn check_harness(&mut self) -> Result<()> {
            self.calls.push(Call::HarnessCheck);
            if self.harness_failing {
                bail!("claude is not on PATH");
            }
            Ok(())
        }

        fn started(&mut self, work: Work) {
            let issue = match work {
                Work::Run(issue) | Work::SpecRun(issue) | Work::PickupRun(issue) => {
                    Some(issue.number)
                }
                Work::ArchitectRun(_) => None,
            };
            self.calls.push(Call::Started(issue));
        }

        fn dispatch(&mut self, dispatch: Dispatch) -> Ended {
            let Dispatch::ReadyIssue {
                issue,
                is_spec,
                base,
            } = dispatch
            else {
                unimplemented!("an Architect plan's dispatch, recorded from #415")
            };
            self.calls.push(Call::Dispatch {
                issue: issue.number,
                is_spec,
                base: base.to_string(),
            });
            self.ending
                .take()
                .expect("a dispatch, with no ending scripted for it")
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
