use super::test_support::*;
use super::*;
use anyhow::anyhow;

#[test]
fn spec_delivery_pushes_then_annotates_then_restores_checklist_before_readiness() {
    let mut outside = Scripted::default();
    assert_eq!(
        spec_delivery(&mut outside, Goal::ReadyForReview)
            .ok()
            .as_deref(),
        Some(PR_URL)
    );
    assert_eq!(
        outside.calls,
        [
            Call::CatchUp,
            Call::Session(Kind::SpecReview),
            Call::Push,
            Call::Annotation,
            Call::Checklist,
            Call::Ready,
            Call::Goal(Goal::ReadyForReview)
        ]
    );
}

#[test]
fn spec_delivery_catches_up_before_its_opening_session() {
    let mut outside = Scripted::default();
    assert!(spec_delivery(&mut outside, Goal::ReadyForReview).is_ok());
    assert_eq!(
        outside.calls[..2],
        [Call::CatchUp, Call::Session(Kind::SpecReview)]
    );
}

#[test]
fn annotation_failure_warns_and_still_restores_checklist_and_marks_ready() {
    let mut outside = Scripted {
        failing: Some(Fails::Annotation),
        ..Default::default()
    };
    assert!(spec_delivery(&mut outside, Goal::ReadyForReview).is_ok());
    assert!(matches!(&outside.calls[4], Call::Warning(line) if line.contains("could not write")));
    assert_eq!(
        outside.calls[5..],
        [
            Call::Checklist,
            Call::Ready,
            Call::Goal(Goal::ReadyForReview)
        ]
    );
}

#[test]
fn each_delivery_failure_preserves_work_then_finishes_the_pr_with_its_primary_cause() {
    for failing in [
        Fails::CatchUp,
        Fails::Session,
        Fails::Push,
        Fails::Checklist,
        Fails::Ready,
        Fails::Goal,
    ] {
        let mut outside = Scripted {
            failing: Some(failing),
            ..Default::default()
        };
        let failed = spec_delivery(&mut outside, Goal::Merged).unwrap_err();
        assert_eq!(failed.error.to_string(), format!("{failing:?} failed"));
        assert_eq!(failed.pr_url.as_deref(), Some(PR_URL));
        assert_eq!(
            failed.log,
            if failing == Fails::CatchUp {
                None
            } else {
                Some(PathBuf::from("/logs/spec-review.jsonl"))
            }
        );
        assert_eq!(
            outside.calls[outside.calls.len() - 2..],
            [
                Call::Preserve(format!("{failing:?} failed")),
                Call::Finish(false)
            ]
        );
        assert!(!outside.calls.contains(&Call::Delete));
    }
}

#[test]
fn failed_run_finishing_errors_warn_and_preserve_the_primary_error_and_log() {
    for failing in [Fails::Preserve, Fails::Finish] {
        let mut outside = Scripted {
            failing: Some(failing),
            ..Default::default()
        };
        let failed = fail(
            &mut outside,
            recovery::Stage::Implementation,
            PathBuf::from("/worktree"),
            Some(PathBuf::from("/logs/implement.jsonl")),
            anyhow!("original\nextra context"),
        );
        assert_eq!(failed.error.to_string(), "original\nextra context");
        assert_eq!(failed.log, Some(PathBuf::from("/logs/implement.jsonl")));
        assert_eq!(outside.calls[0], Call::Preserve("original".to_string()));
        assert!(
            outside
                .calls
                .iter()
                .any(|call| matches!(call, Call::Step(line) if line.contains("could not")))
        );
    }
}

#[test]
fn a_spec_delivery_opens_in_its_spec_review() {
    for failing in [Fails::CatchUp, Fails::Session] {
        let mut outside = Scripted {
            failing: Some(failing),
            ..Default::default()
        };
        let failed = spec_delivery(&mut outside, Goal::ReadyForReview).unwrap_err();
        assert_eq!(stopped(&failed), recovery::Stage::SpecReview, "{failing:?}");
    }
}

#[test]
fn a_security_hold_whose_review_started_no_session_reports_the_most_recent_session_log() {
    let hold = SecurityOutcome::of(Err(anyhow!("no merge base")), None)
        .hold()
        .unwrap();
    let failed = fail(
        &mut Scripted::default(),
        recovery::Stage::SecurityReview,
        PathBuf::from("/worktree"),
        Some(PathBuf::from("/logs/repair-1.jsonl")),
        hold.into(),
    );
    assert_eq!(failed.log, Some(PathBuf::from("/logs/repair-1.jsonl")));
}

#[test]
fn protected_path_globs_match_on_segment_boundaries() {
    fn matches(pattern: &str, path: &str) -> bool {
        glob_match(pattern.as_bytes(), path.as_bytes())
    }
    assert!(
        !matches("**/key.txt", "mykey.txt"),
        "**/key.txt matched inside a segment"
    );
    assert!(
        !matches("src/**/config.toml", "src/myconfig.toml"),
        "src/**/config.toml matched inside a segment"
    );
    assert!(matches("src/**/config.toml", "src/pkg/config.toml"));
    assert!(matches("src/**/config.toml", "src/config.toml"));
    assert!(matches("**/key.txt", "dir/key.txt"));
    assert!(matches("**/key.txt", "key.txt"));
    assert!(matches("secrets/**", "secrets/key.txt"));
    assert!(matches("**/*.pem", "keys/id.pem"));
    assert!(!matches("*.txt", "notes/plain.txt"));
    assert!(matches("caf?.txt", "café.txt"), "? should be one character");
    assert!(!matches("a?c", "a/c"));
}
