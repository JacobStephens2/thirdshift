use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use super::*;
use crate::evals::experiment::{Arm, trial_results};

/// Environment and interruption state belong to an isolated test process.
fn isolated(env: &[(&str, &str)]) -> bool {
    if std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").is_some() {
        crate::interrupt::install().unwrap();
        return false;
    }
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(root.path().join("home")).unwrap();
    executable(&bin.join("systemctl"), "#!/bin/sh\necho running\n");
    // The test executable has no --oom-exec entry point. This external
    // manager fixture unwraps it and launches the selected executable.
    executable(
        &bin.join("systemd-run"),
        "#!/bin/sh\nwhile [ \"$1\" != -- ]; do shift; done\nshift\nshift\n[ \"$1\" = --oom-exec ] || exit 98\nshift\nexec \"$@\"\n",
    );
    executable(&bin.join("journalctl"), "#!/bin/sh\nexit 0\n");
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            std::thread::current().name().unwrap(),
            "--nocapture",
        ])
        .env("THIRDSHIFT_EXECUTION_FIXTURE", root.path())
        .env("HOME", root.path().join("home"))
        .env("PATH", path)
        .env("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")
        .envs(env.iter().copied())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    root: PathBuf,
    task: Task,
    prepared: arms::Prepared,
    setup: Setup,
    trace: logs::trace::Context,
    signal: PathBuf,
}

impl Fixture {
    fn new(body: &str) -> Self {
        let root = tempfile::tempdir_in(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap())
            .unwrap()
            .keep();
        fs::create_dir(root.join("bin")).unwrap();
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        let git = crate::git::Git::new(&source);
        git.run(&["init", "--template="]).unwrap();
        git.run(&["config", "user.name", "Eval"]).unwrap();
        git.run(&["config", "user.email", "eval@example.invalid"])
            .unwrap();
        fs::write(source.join("CLAUDE.md"), "Review the change.\n").unwrap();
        git.run(&["add", "."]).unwrap();
        git.run(&["commit", "-m", "Base"]).unwrap();
        let base = git.run(&["rev-parse", "HEAD"]).unwrap();
        fs::write(source.join("change.txt"), "Reviewed change\n").unwrap();
        git.run(&["add", "."]).unwrap();
        git.run(&["commit", "-m", "Change"]).unwrap();
        let change = git.run(&["rev-parse", "HEAD"]).unwrap();
        let task = Task {
            suite: "code-review".into(), name: "fixture".into(), directory: root.clone(),
            manifest: serde_json::from_value(json!({
                "format": 1, "state": "approved", "session": "code-review",
                "repository": "source", "base": base, "change": change,
                "criteria": "Review the change", "checks": [["sh", "-c", "test -f change.txt"]], "inputs": {},
                "trial": {"issue_url": "https://github.com/acme/widgets/issues/7", "base": "base", "branch": "issue-7", "pr_url": "https://github.com/acme/widgets/pull/8"}
            })).unwrap(),
        };
        let binary = root.join("arm");
        executable(
            &binary,
            &format!(
                "#!/usr/bin/env python3\nimport json, os, pathlib, signal, sys, time\nrequest = json.load(open(sys.argv[2]))\nrepository = pathlib.Path(request['repository'])\noutput = pathlib.Path(request['output_dir'])\noutput.mkdir()\n{body}\n"
            ),
        );
        let harness = root.join("bin/opencode");
        executable(&harness, "#!/bin/sh\nexit 99\n");
        let arm = json!({"commit": base, "binary": binary, "harness": {"name": "opencode", "path": harness, "version": "fixture"}});
        let prepared =
            serde_json::from_value(json!({"format": 1, "baseline": arm, "candidate": arm}))
                .unwrap();
        Self {
            signal: root.join("plan-limit"),
            root,
            task,
            prepared,
            setup: Setup {
                commit: base,
                harness: "opencode".into(),
                model: None,
                effort: None,
            },
            trace: logs::trace::Context {
                trace: crate::logs::trace::id(&[b"execution"]),
                command: crate::logs::trace::id(&[b"command"]),
                run: crate::logs::trace::id(&[b"run"]),
                stage: crate::logs::trace::id(&[b"trials"]),
                identity: "execution-fixture".into(),
                repository: "acme/widgets".into(),
            },
        }
    }

    fn execution(&self, timeout: u64) -> Execution<'_> {
        Execution::prepare(
            &self.root,
            &self.prepared,
            [&self.setup, &self.setup],
            Duration::from_secs(timeout),
            &self.signal,
        )
        .unwrap()
    }

    fn request(&self) -> Request<'_> {
        Request {
            task: &self.task,
            identity: trial_results::Identity {
                suite: &self.task.suite,
                task: &self.task.name,
                arm: Arm::Candidate,
                trial: 0,
            },
            attempt: 2,
            trace: &self.trace,
        }
    }
}

const SUCCESS: &str = r#"
assert (repository / 'AGENTS.md').read_text() == 'Review the change.\n'
report = output / 'standards.md'
report.write_text('Review complete.\n')
print(json.dumps(dict(version=1, outcome='succeeded', timings=dict(total_ms=12, session_ms=10), reports=dict(standards=str(report), spec=None), void_cause=None, cause=None, session_id=None, log=None, model_evidence=None)))
"#;

#[test]
fn completed_score_and_evidence_live_through_publication() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new(SUCCESS);
    let execution = fixture.execution(5);
    let mut writer =
        trial_results::Writer::resume(&fixture.root, fixture.request().identity).unwrap();
    let attempt = execution
        .run(fixture.request(), |attempt, output, score| {
            assert!(output.outcome == Outcome::Succeeded);
            assert!(score.passed);
            assert_eq!(score.checks.len(), 3);
            assert!(score.model.is_none());
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            assert!(!attempt.join("trace.spans.jsonl").exists());
            let request: Value =
                serde_json::from_slice(&fs::read(attempt.join("request.json"))?).unwrap();
            assert_eq!(request["version"], 1);
            let started: Value =
                serde_json::from_slice(&fs::read(attempt.join("started.json"))?).unwrap();
            assert_eq!(started["index"], 2);
            writer.record_attempt(&attempt, output, score)?;
            writer.finish()?;
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            Ok(attempt)
        })
        .unwrap();
    assert!(!attempt.parent().unwrap().join("active.json").exists());
    assert!(fixture.root.join("started").is_file());
    for artifact in [
        "request.json",
        "stdout",
        "stderr",
        "scope",
        "started.json",
        "trace-context.json",
        "trace.logs.jsonl",
        "trace.spans.jsonl",
        "result.json",
    ] {
        assert!(attempt.join(artifact).is_file(), "missing {artifact}");
    }
    let spans = fs::read_to_string(attempt.join("trace.spans.jsonl")).unwrap();
    assert!(spans.contains("Grader"));
    assert!(spans.contains("succeeded"));
}

#[test]
fn changed_instructions_void_an_otherwise_successful_execution() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new(&format!(
        "{SUCCESS}\n(repository / 'AGENTS.md').write_text('Edited instructions')"
    ));
    fixture
        .execution(5)
        .run(fixture.request(), |attempt, output, score| {
            assert!(output.outcome == Outcome::Void);
            assert_eq!(output.void_cause, Some(VoidCause::RepositoryChanged));
            assert!(!score.passed);
            assert_eq!(score.checks.len(), 2, "Void Trials skip code checks");
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            Ok(())
        })
        .unwrap();
}

#[test]
fn timeout_preserves_failure_after_mutation_and_stops_descendants() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new(
        r#"
import subprocess
(repository / 'changed.txt').write_text('Changed')
escaped = output.parent / 'escaped'
subprocess.Popen([sys.executable, '-c', "import pathlib,sys,time; time.sleep(2); pathlib.Path(sys.argv[1]).touch()", str(escaped)])
time.sleep(20)
"#,
    );
    let attempt = fixture
        .execution(1)
        .run(fixture.request(), |attempt, output, score| {
            assert!(output.outcome == Outcome::Failed);
            assert_eq!(output.void_cause, None);
            assert_eq!(output.cause.as_deref(), Some("Trial timed out"));
            assert_eq!(output.timings.total_ms, 1000);
            assert!(!score.passed);
            Ok(attempt)
        })
        .unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        !attempt.join("escaped").exists(),
        "a timed out descendant kept working"
    );
    assert!(attempt.join("trace.spans.jsonl").is_file());
    assert!(!attempt.parent().unwrap().join("active.json").exists());
}

#[test]
fn callback_errors_propagate_after_trace_and_active_evidence_cleanup() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new(SUCCESS);
    let mut writer =
        trial_results::Writer::resume(&fixture.root, fixture.request().identity).unwrap();
    let mut retained = None;
    let error = fixture
        .execution(5)
        .run(fixture.request(), |attempt, output, score| -> Result<()> {
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            writer.record_attempt(&attempt, output, score)?;
            retained = Some(attempt);
            Err(std::io::Error::from_raw_os_error(libc::ENOSPC).into())
        })
        .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(libc::ENOSPC)
    );
    assert_eq!(
        error.to_string(),
        std::io::Error::from_raw_os_error(libc::ENOSPC).to_string()
    );
    let attempt = retained.unwrap();
    assert!(!attempt.parent().unwrap().join("active.json").exists());
    assert!(attempt.join("trace.spans.jsonl").is_file());
    assert_eq!(writer.progress().attempts.len(), 1);
    assert!(!trial_results::finished(
        &fixture.root,
        &fixture.request().identity
    ));
}

#[test]
fn older_arms_stop_at_native_limits_in_fresh_homes_from_raw_and_trace_streams() {
    if isolated(&[("THIRDSHIFT_TRIAL_NO_SANDBOX", "")]) {
        return;
    }
    for session in ["code-review", "spec-review"] {
        for trace in [false, true] {
            let mut fixture = Fixture::new(&format!(
                r#"
import datetime
native = output / 'home/.codex/sessions/2026/10/24/rollout-fake-thread-1.jsonl'
native.parent.mkdir(parents=True)
event = json.dumps(dict(type='thread.started', thread_id='fake-thread-1'))
suffix = 'jsonl'
if {trace:?} == 'true':
    event = json.dumps(dict(resourceLogs=[dict(scopeLogs=[dict(logRecords=[dict(body=dict(stringValue=event))])])]))
    suffix = 'logs.jsonl'
    span = dict(traceId='11' * 16, spanId='22' * 8, name='invoke_agent codex', startTimeUnixNano='1', endTimeUnixNano='2')
    (output / 'trial-{session}.spans.jsonl').write_text(json.dumps(dict(resourceSpans=[dict(scopeSpans=[dict(spans=[span])])])) + '\n')
(output / ('trial-{session}.' + suffix)).write_text(event + '\n')
native.write_text(json.dumps(dict(type='session_meta', payload=dict(id='fake-thread-1', cwd=request['repository']))) + '\n')
time.sleep(0.2)
with native.open('a') as records:
    records.write(json.dumps(dict(timestamp=datetime.datetime.now(datetime.timezone.utc).isoformat(), type='event_msg', payload=dict(type='token_count', rate_limits=dict(secondary=dict(used_percent=85))))) + '\n')
time.sleep(2)
(output.parent / 'continued').touch()
sys.exit(99)
"#,
                trace = trace.to_string()
            ));
            fixture.setup.harness = "codex".into();
            fixture.prepared.baseline.harness.name = "codex".into();
            fixture.prepared.candidate.harness.name = "codex".into();
            if session == "spec-review" {
                fixture.task.manifest.session = crate::evals::format::Session::SpecReview;
            }
            let attempt = fixture
                .execution(5)
                .run(fixture.request(), |attempt, output, score| {
                    assert!(output.outcome == Outcome::Void);
                    assert_eq!(output.void_cause, Some(VoidCause::UsageLimit));
                    assert_eq!(output.session_id.as_deref(), Some("fake-thread-1"));
                    let suffix = if trace { "logs.jsonl" } else { "jsonl" };
                    assert!(
                        output
                            .log
                            .unwrap()
                            .ends_with(format!("trial-{session}.{suffix}"))
                    );
                    assert!(!score.passed);
                    assert!(fixture.signal.is_file());
                    Ok(attempt)
                })
                .unwrap();
            assert!(!attempt.join("continued").exists());
            assert!(attempt.join("trace.spans.jsonl").is_file());
            assert!(!attempt.parent().unwrap().join("active.json").exists());
            if trace {
                let retained: Value = serde_json::from_slice(
                    &fs::read(attempt.join(format!("session/trial-{session}.spans.jsonl")))
                        .unwrap(),
                )
                .unwrap();
                let span = &retained["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
                assert_eq!(
                    span["traceId"],
                    serde_json::to_value(fixture.trace.trace).unwrap()
                );
                let context: logs::trace::Context =
                    serde_json::from_slice(&fs::read(attempt.join("trace-context.json")).unwrap())
                        .unwrap();
                assert_eq!(
                    span["parentSpanId"],
                    serde_json::to_value(context.stage).unwrap()
                );
            }
        }
    }
}

#[test]
fn interruption_never_reaches_completion_or_creates_a_checkpoint() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new("os.kill(os.getppid(), signal.SIGTERM)\ntime.sleep(20)");
    let mut writer =
        trial_results::Writer::resume(&fixture.root, fixture.request().identity).unwrap();
    let mut called = false;
    let error = fixture
        .execution(5)
        .run(fixture.request(), |attempt, output, score| {
            called = true;
            writer.record_attempt(&attempt, output, score)
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "interrupted");
    assert!(!called);
    assert!(writer.progress().attempts.is_empty());
    let attempt =
        super::super::trial_sessions::experiment(&trial_results::files::Native, &fixture.root)
            .unwrap()
            .pop()
            .unwrap()
            .path;
    assert!(!attempt.parent().unwrap().join("active.json").exists());
    assert!(!attempt.parent().unwrap().join("pending.json").exists());
    assert!(!attempt.join("result.json").exists());
    assert!(attempt.join("trace.spans.jsonl").is_file());
}

#[test]
fn unavailable_code_grader_is_void_and_rescored_before_completion() {
    if isolated(&[("THIRDSHIFT_TRIAL_NO_SANDBOX", "")]) {
        return;
    }
    let bin = PathBuf::from(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap()).join("bin");
    executable(
        &bin.join("bwrap"),
        "#!/bin/sh\necho 'fixture sandbox unavailable' >&2\nexit 1\n",
    );
    let fixture = Fixture::new(SUCCESS);
    fixture
        .execution(5)
        .run(fixture.request(), |attempt, output, score| {
            assert!(output.outcome == Outcome::Void);
            assert_eq!(output.void_cause, Some(VoidCause::SandboxUnavailable));
            assert!(!score.passed);
            assert_eq!(
                score.checks.len(),
                2,
                "the rescored Void must omit code checks"
            );
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            Ok(())
        })
        .unwrap();
}

fn protocol(outcome: &str, cause: Option<&str>) -> String {
    let output = json!({
        "version": 1, "outcome": outcome, "void_cause": cause,
        "timings": {"total_ms": 12, "session_ms": 10},
        "reports": {"standards": null, "spec": null}
    });
    format!("print({:?})", output.to_string())
}

#[test]
fn scope_cap_evidence_preserves_explicit_voids_and_unexplained_kills_fail() {
    if isolated(&[]) {
        return;
    }
    for (body, cap, outcome, cause) in [
        (
            "os.kill(os.getpid(), signal.SIGKILL)".into(),
            false,
            Outcome::Failed,
            None,
        ),
        ("sys.exit(137)".into(), false, Outcome::Failed, None),
        (
            protocol("failed", None),
            true,
            Outcome::Void,
            Some(VoidCause::MemoryCap),
        ),
        (
            "sys.exit(9)".into(),
            true,
            Outcome::Void,
            Some(VoidCause::MemoryCap),
        ),
        (
            "print('malformed')".into(),
            true,
            Outcome::Void,
            Some(VoidCause::MemoryCap),
        ),
        (
            protocol("void", Some("usage_limit")),
            true,
            Outcome::Void,
            Some(VoidCause::UsageLimit),
        ),
        (protocol("succeeded", None), true, Outcome::Succeeded, None),
    ] {
        let journal = PathBuf::from(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap())
            .join("bin/journalctl");
        executable(
            &journal,
            if cap {
                "#!/bin/sh\necho \"Failed with result 'oom-kill'\"\n"
            } else {
                "#!/bin/sh\nexit 0\n"
            },
        );
        let fixture = Fixture::new(&body);
        fixture
            .execution(5)
            .run(fixture.request(), |_, output, score| {
                assert!(output.outcome == outcome, "{body}: {}", output.outcome);
                assert_eq!(output.void_cause, cause, "{body}");
                assert_eq!(score.checks[0].passed, outcome == Outcome::Succeeded);
                if cause == Some(VoidCause::MemoryCap) {
                    assert_eq!(output.cause.as_deref(), Some(crate::scope::OOM_KILL_CAUSE));
                }
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn operational_failures_never_publish_completed_values() {
    if isolated(&[]) {
        return;
    }
    for (body, message) in [
        ("sys.exit(9)", "Arm Trial protocol failed"),
        ("print('malformed')", "invalid Trial result"),
        (
            "print('{\"version\":2,\"outcome\":\"failed\",\"timings\":{\"total_ms\":1,\"session_ms\":1},\"reports\":{\"standards\":null,\"spec\":null}}')",
            "unsupported Trial result",
        ),
    ] {
        let fixture = Fixture::new(body);
        let execution = fixture.execution(5);
        let mut writer =
            trial_results::Writer::resume(&fixture.root, fixture.request().identity).unwrap();
        let mut called = false;
        let error = execution
            .run(fixture.request(), |attempt, output, score| {
                called = true;
                writer.record_attempt(&attempt, output, score)
            })
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error:#}");
        assert!(!called);
        assert!(writer.progress().attempts.is_empty());
        let trial = fixture
            .root
            .join("trials")
            .join(fixture.request().identity.key());
        assert!(!trial.join("pending.json").exists());
        assert!(!trial.join("active.json").exists());
        assert!(!trial_results::finished(
            &fixture.root,
            &fixture.request().identity
        ));
        let attempts =
            super::super::trial_sessions::experiment(&trial_results::files::Native, &fixture.root)
                .unwrap();
        assert_eq!(attempts.len(), 1);
        assert!(attempts[0].path.join("stdout").is_file());
        assert!(attempts[0].path.join("trace.spans.jsonl").is_file());
        assert!(!attempts[0].path.join("result.json").exists());
    }
}
