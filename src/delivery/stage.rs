//! A Delivery's stage: the one table from each stage its steps and the
//! Repair loop enter to the stage a Failed run's recovery lines report as
//! `Stopped at:`, and to the Command record stage `thirdshift status` and the
//! Console show, and the [`Tracker`] that holds the stage in force.

use std::cell::Cell;

use crate::logs::{self, Stage as CommandStage};
use crate::recovery;

/// A stage of a Delivery, as its steps and the Repair loop enter it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    Implementation,
    SpecReview,
    SecurityReview,
    Push,
    PullRequestReadiness,
    /// A round of the Repair loop: merging the Base branch or Foreign
    /// commits in, before any Repair session.
    RepairLoopRound,
    RepairSession,
    CiWatch,
    BaseFix,
    SelfMerge,
}

impl Stage {
    /// The stage a Failed run's recovery lines report, and the Command
    /// record stage, if the Delivery records one. Implementation and Spec
    /// review have none: their Session trace names its stage.
    fn records(self) -> (recovery::Stage, Option<CommandStage>) {
        match self {
            Self::Implementation => (recovery::Stage::Implementation, None),
            Self::SpecReview => (recovery::Stage::SpecReview, None),
            Self::SecurityReview => (
                recovery::Stage::SecurityReview,
                Some(CommandStage::SecurityReview),
            ),
            Self::Push => (recovery::Stage::Push, Some(CommandStage::PullRequest)),
            Self::PullRequestReadiness => (
                recovery::Stage::PullRequest,
                Some(CommandStage::PullRequest),
            ),
            Self::RepairLoopRound => (recovery::Stage::Repair, None),
            Self::RepairSession => (recovery::Stage::Repair, Some(CommandStage::Repair)),
            Self::CiWatch => (recovery::Stage::Ci, Some(CommandStage::CiWatch)),
            Self::BaseFix => (recovery::Stage::BaseFix, None),
            Self::SelfMerge => (recovery::Stage::SelfMerge, Some(CommandStage::SelfMerge)),
        }
    }
}

/// Where a Delivery's Command record stages go.
pub(super) trait CommandRecord {
    /// Record `stage`; the same stage recorded twice in a row is one.
    fn stage(&self, stage: CommandStage);
}

/// The command's own Command record.
pub(super) struct CommandLog;

impl CommandRecord for CommandLog {
    fn stage(&self, stage: CommandStage) {
        logs::stage(stage);
    }
}

/// The stage a Delivery is in, which its Failed run path reports.
pub(super) struct Tracker<R = CommandLog> {
    stopped_at: Cell<recovery::Stage>,
    record: R,
}

impl<R: CommandRecord> Tracker<R> {
    /// A Tracker that has entered `first`, recording to `record`.
    pub(super) fn new(first: Stage, record: R) -> Self {
        // Entering `first` replaces this placeholder at once.
        let tracker = Self {
            stopped_at: Cell::new(recovery::Stage::Implementation),
            record,
        };
        tracker.enter(first);
        tracker
    }

    /// Enter `stage`: it is what a failure from now on stopped at, and its
    /// Command record stage, if it has one, is recorded.
    pub(super) fn enter(&self, stage: Stage) {
        let (stopped_at, recorded) = stage.records();
        self.stopped_at.set(stopped_at);
        if let Some(recorded) = recorded {
            self.record.stage(recorded);
        }
    }

    /// A Security hold, found after the pull request was made ready, stopped
    /// the Delivery at its Security review. Nothing is recorded: the
    /// Security review was over before the stages since.
    pub(super) fn stop_at_security_review(&self) {
        self.stopped_at.set(recovery::Stage::SecurityReview);
    }

    /// The stage a failure now stopped at.
    pub(super) fn stopped_at(&self) -> recovery::Stage {
        self.stopped_at.get()
    }
}

/// An in-memory Command record, for tests: the names of the stages
/// recorded, in order, a repeated one collapsed as the Command record does.
#[cfg(test)]
#[derive(Default)]
pub(super) struct Recorded(std::cell::RefCell<Vec<&'static str>>);

#[cfg(test)]
impl Recorded {
    pub(super) fn names(&self) -> Vec<&'static str> {
        self.0.borrow().clone()
    }
}

#[cfg(test)]
impl CommandRecord for Recorded {
    fn stage(&self, stage: CommandStage) {
        let mut names = self.0.borrow_mut();
        if names.last() != Some(&stage.name()) {
            names.push(stage.name());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entering_each_stage_sets_its_recovery_stage_and_records_its_command_record_stage() {
        let table = [
            (Stage::Implementation, recovery::Stage::Implementation, None),
            (Stage::SpecReview, recovery::Stage::SpecReview, None),
            (
                Stage::SecurityReview,
                recovery::Stage::SecurityReview,
                Some("Security review"),
            ),
            (Stage::Push, recovery::Stage::Push, Some("pull request")),
            (
                Stage::PullRequestReadiness,
                recovery::Stage::PullRequest,
                Some("pull request"),
            ),
            (Stage::RepairLoopRound, recovery::Stage::Repair, None),
            (
                Stage::RepairSession,
                recovery::Stage::Repair,
                Some("Repair"),
            ),
            (Stage::CiWatch, recovery::Stage::Ci, Some("CI watch")),
            (Stage::BaseFix, recovery::Stage::BaseFix, None),
            (
                Stage::SelfMerge,
                recovery::Stage::SelfMerge,
                Some("Self-merge"),
            ),
        ];
        for (stage, stopped_at, recorded) in table {
            let tracker = Tracker::new(Stage::Implementation, Recorded::default());
            tracker.enter(stage);
            assert_eq!(tracker.stopped_at(), stopped_at, "{stage:?}");
            assert_eq!(
                tracker.record.names(),
                Vec::from_iter(recorded),
                "{stage:?}"
            );
        }
    }

    #[test]
    fn a_stage_entered_again_is_recorded_once() {
        let tracker = Tracker::new(Stage::Implementation, Recorded::default());
        for stage in [
            Stage::Push,
            Stage::PullRequestReadiness,
            Stage::RepairLoopRound,
            Stage::Push,
            Stage::CiWatch,
            Stage::CiWatch,
        ] {
            tracker.enter(stage);
        }
        assert_eq!(tracker.record.names(), ["pull request", "CI watch"]);
        assert_eq!(tracker.stopped_at(), recovery::Stage::Ci);
    }

    #[test]
    fn a_security_hold_stops_at_the_security_review_without_recording_it_again() {
        let tracker = Tracker::new(Stage::Implementation, Recorded::default());
        for stage in [
            Stage::SecurityReview,
            Stage::Push,
            Stage::PullRequestReadiness,
            Stage::RepairLoopRound,
            Stage::CiWatch,
        ] {
            tracker.enter(stage);
        }
        tracker.stop_at_security_review();
        assert_eq!(tracker.stopped_at(), recovery::Stage::SecurityReview);
        assert_eq!(
            tracker.record.names(),
            ["Security review", "pull request", "CI watch"]
        );
    }

    #[test]
    fn a_new_tracker_has_entered_its_first_stage() {
        let tracker = Tracker::new(Stage::SpecReview, Recorded::default());
        assert_eq!(tracker.stopped_at(), recovery::Stage::SpecReview);
        assert!(tracker.record.names().is_empty());
    }
}
