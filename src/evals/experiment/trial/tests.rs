use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::super::{evidence, trace};
use super::*;

struct Fixture {
    root: tempfile::TempDir,
    task: Task,
    bar: Bar,
    trace: logs::trace::Context,
    signal: PathBuf,
}

impl Fixture {
    fn new(retries: usize) -> Self {
        let root = tempfile::tempdir().unwrap();
        Self {
            task: Task {
                suite: "code-review".into(),
                name: "fixture".into(),
                directory: root.path().into(),
                manifest: serde_json::from_value(json!({
                    "format": 1, "state": "approved", "session": "code-review",
                    "repository": "source", "base": "base", "change": "change",
                    "criteria": "Review the change", "checks": [], "inputs": {}
                }))
                .unwrap(),
            },
            bar: Bar {
                void_rules: super::super::VoidRules {
                    retries,
                    causes: vec![
                        VoidCause::SessionUnavailable,
                        VoidCause::UsageLimit,
                        VoidCause::RateLimit,
                        VoidCause::Interrupted,
                    ],
                },
                ..Bar::default()
            },
            trace: logs::trace::Context {
                trace: logs::trace::id(&[b"trial"]),
                command: logs::trace::id(&[b"command"]),
                run: logs::trace::id(&[b"run"]),
                stage: logs::trace::id(&[b"trials"]),
                identity: "trial-fixture".into(),
                repository: "acme/widgets".into(),
            },
            signal: root.path().join("plan-limit"),
            root,
        }
    }

    fn key(&self) -> String {
        trial_results::trial_key("code-review", "fixture", Arm::Baseline, 0)
    }

    fn trial(&self) -> PathBuf {
        self.root.path().join("trials").join(self.key())
    }

    fn terminal(&self) -> PathBuf {
        self.root
            .path()
            .join("results")
            .join(format!("{}.json", self.key()))
    }

    fn runner<'a>(&'a self, script: &'a Scripted) -> Runner<'a> {
        Runner {
            directory: self.root.path(),
            bar: &self.bar,
            signal: &self.signal,
            trace: &self.trace,
            execution: Box::new(script),
        }
    }

    fn run(&self, script: &Scripted) -> Result<()> {
        self.runner(script).run(&self.task, Arm::Baseline, 0)
    }

    fn pending(&self) -> Value {
        read(&self.trial().join("pending.json"))
    }

    fn result(&self) -> Value {
        let entries = trial_results::terminal(self.root.path()).unwrap();
        assert_eq!(entries.len(), 1);
        serde_json::to_value(entries[0].result()).unwrap()
    }
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn output(outcome: &str, cause: Option<VoidCause>) -> Output {
    serde_json::from_value(json!({
        "version": 1, "outcome": outcome, "void_cause": cause,
        "timings": {"total_ms": 60_000, "session_ms": 60_000},
        "reports": {"standards": null, "spec": null}
    }))
    .unwrap()
}

fn score(passed: bool) -> grading::Score {
    serde_json::from_value(json!({"passed": passed, "wrong_finding": false,
        "checks": [{"command": null, "passed": true, "reason": "scripted mechanical Score"}]}))
    .unwrap()
}

#[derive(Clone, Copy)]
enum Collision {
    Attempt,
    Pending,
    Terminal,
}

struct Step {
    output: Output,
    terminal: bool,
    collision: Option<Collision>,
}

fn step(outcome: &str, cause: Option<VoidCause>, terminal: bool) -> Step {
    Step {
        output: output(outcome, cause),
        terminal,
        collision: None,
    }
}

#[derive(Default)]
struct Scripted {
    steps: Mutex<VecDeque<Step>>,
    indices: Mutex<Vec<usize>>,
    attempts: Mutex<Vec<PathBuf>>,
}

impl Scripted {
    fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Mutex::new(steps.into()),
            ..Self::default()
        }
    }

    fn assert_indices(&self, expected: &[usize]) {
        assert_eq!(*self.indices.lock().unwrap(), expected);
        assert!(self.steps.lock().unwrap().is_empty());
        for attempt in self.attempts.lock().unwrap().iter() {
            assert!(!attempt.parent().unwrap().join("active.json").exists());
            assert!(attempt.join("trace.spans.jsonl").is_file());
        }
    }
}

impl Attempt for &Scripted {
    fn run(&self, request: execution::Request<'_>, complete: Completion<'_>) -> Result<()> {
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected attempt");
        self.indices.lock().unwrap().push(request.attempt);
        assert_eq!(request.task.name, "fixture");
        let root = &request.task.directory;
        let trial = root.join("trials").join(request.identity.key());
        if request.attempt > 0 {
            assert_eq!(
                read(&trial.join("pending.json"))["attempts"]
                    .as_array()
                    .unwrap()
                    .len(),
                request.attempt,
                "completed history must checkpoint before another attempt starts"
            );
        }
        let terminal = root
            .join("results")
            .join(format!("{}.json", request.identity.key()));
        let attempt = tempfile::Builder::new()
            .prefix("attempt-")
            .tempdir_in(&trial)?
            .keep();
        self.attempts.lock().unwrap().push(attempt.clone());
        let _active = evidence::Active::start(&trial, &attempt, request.attempt)?;
        let mut trace = trace::Attempt::start(&attempt, &request.identity.key(), request.trace)?;
        trace.outcome(step.output.outcome);
        let collision = step.collision.map(|collision| match collision {
            Collision::Attempt => attempt.join("result.json"),
            Collision::Pending => trial.join("pending.json"),
            Collision::Terminal => terminal.clone(),
        });
        if let Some(path) = collision {
            fs::create_dir(path)?;
        }
        let passed = step.output.outcome == Outcome::Succeeded;
        let result = complete(attempt.clone(), step.output, score(passed));
        // These observations happen before either evidence lease can drop.
        assert!(trial.join("active.json").is_file());
        assert!(!attempt.join("trace.spans.jsonl").exists());
        assert_eq!(terminal.is_file(), step.terminal);
        if step.terminal {
            assert_eq!(read(&terminal), read(&trial.join("pending.json")));
            assert!(attempt.join("result.json").is_file());
        }
        result
    }
}

#[test]
fn successful_and_failed_trials_publish_before_evidence_cleanup() {
    for outcome in ["succeeded", "failed"] {
        let fixture = Fixture::new(2);
        let script = Scripted::new(vec![step(outcome, None, true)]);
        fixture.run(&script).unwrap();
        script.assert_indices(&[0]);
        let result = fixture.result();
        assert_eq!(result, fixture.pending());
        assert_eq!(result["outcome"], outcome);
        assert_eq!(result["score"]["passed"], outcome == "succeeded");
        assert_eq!(
            result["score"]["checks"][0]["reason"],
            "scripted mechanical Score"
        );
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn retryable_void_checkpoints_then_retries_fresh() {
    let fixture = Fixture::new(2);
    let script = Scripted::new(vec![
        step("void", Some(VoidCause::SessionUnavailable), false),
        step("succeeded", None, true),
    ]);
    fixture.run(&script).unwrap();
    script.assert_indices(&[0, 1]);
    let attempts = script.attempts.lock().unwrap();
    assert_ne!(attempts[0], attempts[1]);
    assert_eq!(read(&attempts[0].join("result.json"))["outcome"], "void");
    assert_eq!(fixture.result()["attempts"].as_array().unwrap().len(), 2);
}

#[test]
fn missing_or_excluded_void_causes_are_terminal() {
    for cause in [None, Some(VoidCause::HostKilled)] {
        let fixture = Fixture::new(2);
        let script = Scripted::new(vec![step("void", cause, true)]);
        fixture.run(&script).unwrap();
        script.assert_indices(&[0]);
        assert_eq!(fixture.result()["outcome"], "void");
    }
}

#[test]
fn retry_budget_includes_the_initial_attempt_and_zero_retries() {
    for (retries, indices) in [(0, vec![0]), (2, vec![0, 1, 2])] {
        let fixture = Fixture::new(retries);
        let script = Scripted::new(
            indices
                .iter()
                .map(|index| {
                    step(
                        "void",
                        Some(VoidCause::SessionUnavailable),
                        *index == retries,
                    )
                })
                .collect(),
        );
        fixture.run(&script).unwrap();
        script.assert_indices(&indices);
        assert_eq!(
            fixture.result()["attempts"].as_array().unwrap().len(),
            indices.len()
        );
    }
}

#[test]
fn fresh_pauses_checkpoint_and_exhaustion_finishes_without_replaying_the_pause() {
    for cause in [
        VoidCause::UsageLimit,
        VoidCause::RateLimit,
        VoidCause::Interrupted,
    ] {
        for retries in [0, 2] {
            let fixture = Fixture::new(retries);
            for index in 0..=retries {
                let script = Scripted::new(vec![step("void", Some(cause), index == retries)]);
                let error = fixture.run(&script).unwrap_err();
                assert!(error.to_string().contains(&format!("paused: {cause:?}")));
                script.assert_indices(&[index]);
                assert_eq!(
                    fixture.pending()["attempts"].as_array().unwrap().len(),
                    index + 1
                );
                assert_eq!(fixture.terminal().is_file(), index == retries);
            }
            let no_attempt = Scripted::default();
            fixture.run(&no_attempt).unwrap();
            no_attempt.assert_indices(&[]);
            assert_eq!(
                fixture.result()["attempts"].as_array().unwrap().len(),
                retries + 1
            );
        }
    }
}

#[test]
fn pause_excluded_by_saved_bar_still_pauses_after_terminal_publication() {
    let mut fixture = Fixture::new(2);
    fixture.bar.void_rules.causes.clear();
    let script = Scripted::new(vec![step("void", Some(VoidCause::UsageLimit), true)]);
    assert!(
        fixture
            .run(&script)
            .unwrap_err()
            .to_string()
            .contains("paused: UsageLimit")
    );
    script.assert_indices(&[0]);
    fixture.run(&Scripted::default()).unwrap();
    assert_eq!(fixture.result()["attempts"].as_array().unwrap().len(), 1);
}

#[test]
fn retained_retryable_pause_resumes_with_the_same_budget_and_new_score() {
    for cause in [VoidCause::UsageLimit, VoidCause::RateLimit] {
        let fixture = Fixture::new(2);
        let paused = Scripted::new(vec![step("void", Some(cause), false)]);
        assert!(fixture.run(&paused).is_err());
        assert_eq!(fixture.pending()["score"]["passed"], false);
        let resumed = Scripted::new(vec![step("succeeded", None, true)]);
        fixture.run(&resumed).unwrap();
        resumed.assert_indices(&[1]);
        assert_eq!(fixture.result()["outcome"], "succeeded");
        assert_eq!(fixture.result()["score"]["passed"], true);
        assert_eq!(fixture.result()["attempts"].as_array().unwrap().len(), 2);
    }
}

#[test]
fn terminal_publication_failure_resumes_only_the_checkpointed_history() {
    for (outcome, cause, retries, count) in [
        ("succeeded", None, 2, 1),
        ("failed", None, 2, 1),
        ("void", None, 2, 1),
        ("void", Some(VoidCause::HostKilled), 2, 1),
        ("void", Some(VoidCause::SessionUnavailable), 0, 1),
        ("void", Some(VoidCause::SessionUnavailable), 2, 3),
        ("void", Some(VoidCause::UsageLimit), 0, 1),
    ] {
        let fixture = Fixture::new(retries);
        let script = Scripted::new(
            (0..count)
                .map(|index| Step {
                    collision: (index + 1 == count).then_some(Collision::Terminal),
                    ..step(outcome, cause, false)
                })
                .collect(),
        );
        assert!(fixture.run(&script).is_err());
        script.assert_indices(&(0..count).collect::<Vec<_>>());
        let saved = fixture.pending();
        assert_eq!(saved["attempts"].as_array().unwrap().len(), count);
        assert_eq!(saved["outcome"], outcome);
        fs::remove_dir(fixture.terminal()).unwrap();
        let no_attempt = Scripted::default();
        fixture.run(&no_attempt).unwrap();
        no_attempt.assert_indices(&[]);
        assert_eq!(fixture.result(), saved);
    }
}

#[test]
fn publication_errors_keep_exactly_the_durable_prefix_and_do_not_retry() {
    for collision in [Collision::Attempt, Collision::Pending] {
        let fixture = Fixture::new(2);
        let script = Scripted::new(vec![Step {
            collision: Some(collision),
            ..step("void", Some(VoidCause::SessionUnavailable), false)
        }]);
        assert!(fixture.run(&script).is_err());
        script.assert_indices(&[0]);
        assert!(!fixture.trial().join("pending.json").is_file());
        assert!(!fixture.terminal().exists());
        let attempt = script.attempts.lock().unwrap()[0].clone();
        assert_eq!(
            attempt.join("result.json").is_file(),
            matches!(collision, Collision::Pending)
        );
        if matches!(collision, Collision::Pending) {
            fs::remove_dir(fixture.trial().join("pending.json")).unwrap();
        }
        let resumed = Scripted::new(vec![step("succeeded", None, true)]);
        fixture.run(&resumed).unwrap();
        resumed.assert_indices(&[0]);
        assert_eq!(fixture.result()["attempts"].as_array().unwrap().len(), 1);
    }
    let fixture = Fixture::new(2);
    let script = Scripted::new(vec![
        step("void", Some(VoidCause::SessionUnavailable), false),
        Step {
            collision: Some(Collision::Attempt),
            ..step("succeeded", None, false)
        },
    ]);
    assert!(fixture.run(&script).is_err());
    script.assert_indices(&[0, 1]);
    assert_eq!(fixture.pending()["attempts"].as_array().unwrap().len(), 1);
    let resumed = Scripted::new(vec![step("succeeded", None, true)]);
    fixture.run(&resumed).unwrap();
    resumed.assert_indices(&[1]);
    assert_eq!(fixture.result()["attempts"].as_array().unwrap().len(), 2);
}
