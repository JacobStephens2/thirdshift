//! Ticket sequencing assertions exercise the compiled Process; dispatch and
//! Spec PR assertions stay with their owners. Shared adapters only script I/O.

use crate::delivery::test_support::*;
use crate::failed_run::PolicyRefusal;
use crate::recovery;
use crate::run::Goal;
use crate::security::review::Hold as SecurityHold;
use std::path::PathBuf;

#[test]
fn only_confirmed_self_merge_deletes_the_branch_and_closes_the_issue() {
    let mut outside = Scripted::default();
    assert!(ticket_run(&mut outside, Goal::Merged).is_ok());
    assert!(outside.calls.contains(&Call::Delete));
    assert!(outside.calls.contains(&Call::Close(
        "Closed by #12, merged into main by a thirdshift Ticket run with Self-merge.".to_string()
    )));
}

#[test]
fn interruption_preserves_work_and_keeps_the_recorded_interruption() {
    let mut outside = Scripted {
        failing: Some(Fails::Session),
        interrupted: true,
        ..Default::default()
    };
    let failed = ticket_run(&mut outside, Goal::ReadyForReview).unwrap_err();
    assert_eq!(failed.error.to_string(), "interrupted");
    assert!(failed.interrupted);
    assert!(
        outside
            .calls
            .contains(&Call::Preserve("interrupted".to_string()))
    );
    assert!(outside.calls.contains(&Call::Finish(false)));
}

#[test]
fn policy_refusal_keeps_the_pr_ready_without_a_preservation_push() {
    let mut outside = Scripted {
        refused: true,
        ..Default::default()
    };
    let failed = ticket_run(&mut outside, Goal::Merged).unwrap_err();
    assert!(failed.error.is::<PolicyRefusal>());
    assert_eq!(failed.pr_url.as_deref(), Some(PR_URL));
    assert!(outside.calls.contains(&Call::Finish(true)));
    assert!(
        !outside
            .calls
            .iter()
            .any(|call| matches!(call, Call::Preserve(_)))
    );
}

#[test]
fn post_merge_errors_are_warnings_and_an_already_closed_issue_stays_closed() {
    for failing in [Fails::Delete, Fails::Close] {
        let mut outside = Scripted {
            failing: Some(failing),
            ..Default::default()
        };
        assert!(ticket_run(&mut outside, Goal::Merged).is_ok());
        assert!(
            outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::Warning(_)))
        );
    }
    let mut outside = Scripted {
        issue_closed: true,
        ..Default::default()
    };
    assert!(ticket_run(&mut outside, Goal::Merged).is_ok());
    assert!(
        !outside
            .calls
            .iter()
            .any(|call| matches!(call, Call::Close(_)))
    );
}

#[test]
fn a_delivery_enters_each_stage_before_its_step() {
    let mut outside = Scripted {
        reviews_security: true,
        ..Default::default()
    };
    assert!(ticket_run(&mut outside, Goal::ReadyForReview).is_ok());
    assert_eq!(
        outside.stages,
        [
            Stage::Implementation,
            Stage::SecurityReview,
            Stage::Push,
            Stage::PullRequestReadiness
        ]
    );
}

#[test]
fn each_failed_step_stops_at_its_stage() {
    for (failing, stage) in [
        (Fails::Session, recovery::Stage::Implementation),
        (Fails::Push, recovery::Stage::Push),
        (Fails::Ready, recovery::Stage::PullRequest),
    ] {
        let mut outside = Scripted {
            failing: Some(failing),
            ..Default::default()
        };
        let failed = ticket_run(&mut outside, Goal::ReadyForReview).unwrap_err();
        assert_eq!(stopped(&failed), stage, "{failing:?}");
    }
}

#[test]
fn a_security_hold_stops_at_the_security_review_without_entering_it_again() {
    let mut outside = Scripted {
        reviews_security: true,
        security_held: true,
        ..Default::default()
    };
    let failed = ticket_run(&mut outside, Goal::Merged).unwrap_err();
    assert!(failed.error.is::<SecurityHold>(), "{:#}", failed.error);
    assert_eq!(stopped(&failed), recovery::Stage::SecurityReview);
    assert_eq!(
        outside.stages,
        [
            Stage::Implementation,
            Stage::SecurityReview,
            Stage::Push,
            Stage::PullRequestReadiness
        ]
    );
}

#[test]
fn a_security_hold_after_a_repair_reports_the_reviews_session_log() {
    let mut outside = Scripted {
        reviews_security: true,
        security_held: true,
        repairs: true,
        ..Default::default()
    };
    let failed = ticket_run(&mut outside, Goal::Merged).unwrap_err();
    assert!(failed.error.is::<SecurityHold>(), "{:#}", failed.error);
    assert_eq!(outside.log, Some(PathBuf::from("/logs/repair-1.jsonl")));
    assert_eq!(
        failed.log,
        Some(PathBuf::from("/logs/security-review.jsonl"))
    );
}

#[test]
fn a_failure_after_a_repair_reports_the_most_recent_session_log() {
    // A failing Repair session, or a cause no session gave, as with
    // Repairs exhausted, is reported with the most recent Session trace,
    // whether or not a Security review ran before it.
    for reviews_security in [false, true] {
        let mut outside = Scripted {
            reviews_security,
            repairs: true,
            failing: Some(Fails::Goal),
            ..Default::default()
        };
        let failed = ticket_run(&mut outside, Goal::Merged).unwrap_err();
        assert_eq!(failed.log, Some(PathBuf::from("/logs/repair-1.jsonl")));
    }
}
