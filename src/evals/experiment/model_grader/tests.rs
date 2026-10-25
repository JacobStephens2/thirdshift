use super::*;
use crate::scope::testing::{Fixture, REMOVE_OBSERVED_EVENTS};
use crate::session::private::record_session;
use crate::session::recording::Input;
use std::process::Command;
use std::time::Instant;

#[test]
fn recorded_model_grader_reads_native_codex_quota_from_its_launch_home() {
    let fixture = Fixture::new();
    let mut command = crate::test_support::native_codex_command(
        fixture.directory.path(),
        "s1",
        fixture.directory.path(),
        "current",
    );
    command.current_dir(fixture.directory.path());
    let mut launched = fixture.launch(command);
    let error = record_session(
        "model Grader",
        &mut launched,
        &Choice {
            harness: crate::harness::Harness::Codex,
            ..Choice::default()
        },
        fixture.directory.path(),
        Input {
            private: true,
            deadline: Some(Instant::now() + Duration::from_secs(5)),
            ..Input::default()
        },
    )
    .unwrap_err();
    assert!(error.is::<crate::session::limits::Reached>(), "{error:#}");
    assert!(!fixture.path("continued").exists());
    let trace = fs::read_to_string(fixture.path("stdout.logs.jsonl")).unwrap();
    assert!(trace.contains("used_percent"), "{trace}");
}

#[test]
fn recorded_model_grader_retains_live_kill_and_its_private_diagnostic() {
    let fixture = Fixture::new();
    let mut command = Command::new("/bin/sh");
    command.current_dir(fixture.directory.path());
    command.args([
        "-c",
        &format!(
            "echo '{{\"type\":\"assistant\",\"message\":{{\"model\":\"fixture-model\",\"content\":[]}}}}'; \
             echo 'oom_group_kill 1' > \"$EVENTS\"; {REMOVE_OBSERVED_EVENTS}"
        ),
    ]);
    let mut launched = fixture.launch(command);
    let result = record_session(
        "model Grader",
        &mut launched,
        &Choice::default(),
        fixture.directory.path(),
        Input {
            private: true,
            ..Input::default()
        },
    );
    let message = result.unwrap_err().to_string();
    assert!(
        message.contains("model Grader hit its memory cap"),
        "{message}"
    );
    assert!(message.contains(crate::scope::OOM_KILL_CAUSE), "{message}");
    assert!(!fixture.path("events").exists());
    assert_eq!(fixture.journal_reads(), 0);
    for extension in ["logs.jsonl", "spans.jsonl"] {
        assert!(
            !fs::read(fixture.path("stdout.jsonl").with_extension(extension))
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn recorded_model_grader_uses_only_kernel_evidence_and_keeps_journal_lazy() {
    for (events, journal, success, expected) in [
        (
            Some("oom_kill 0\noom_group_kill 0"),
            false,
            false,
            "session failed",
        ),
        (None, false, false, "session failed"),
        (Some("oom_kill unreadable"), false, false, "session failed"),
        (None, true, false, "hit its memory cap"),
        (Some("oom_kill 1"), true, true, ""),
    ] {
        let fixture = Fixture::new();
        fixture.journal_kill.set(journal);
        if let Some(events) = events {
            fs::write(fixture.path("events"), events).unwrap();
        }
        let mut command = Command::new("/bin/sh");
        command.current_dir(fixture.directory.path());
        command.args([
            "-c",
            if success {
                r#"echo '{"type":"result","subtype":"success","result":"graded"}'"#
            } else {
                "echo 'agent claims out of memory'; exit 137"
            },
        ]);
        let mut launched = fixture.launch(command);
        let result = record_session(
            "model Grader",
            &mut launched,
            &Choice::default(),
            fixture.directory.path(),
            Input {
                private: true,
                ..Input::default()
            },
        );
        if success {
            result.unwrap();
            assert_eq!(fixture.journal_reads(), 0);
        } else {
            let message = result.unwrap_err().to_string();
            assert!(message.contains(expected), "{message}");
            assert!(!message.contains("agent claims"));
            assert!(fixture.journal_reads() > 0);
        }
    }
}
