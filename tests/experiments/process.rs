use super::*;

#[test]
fn process_grader_export_obeys_the_bar_timeout() {
    let fixture = fixture();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"{script}
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'Private report file:'; then
    blob=$(git rev-parse HEAD:feature.txt)
    first=$(printf '%s' "$blob" | cut -c1-2)
    rest=$(printf '%s' "$blob" | cut -c3-)
    object=".git/objects/$first/$rest"
    rm "$object"
    mkfifo "$object"
fi
"#
    ));
    let run = fixture.run(&["--timeout", SHORT_TIMEOUT]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["score"]["passed"], false, "{result}");
        assert!(
            result["score"]["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["reason"].as_str().unwrap().contains("timed out")),
            "{result}"
        );
    }
}

pub(super) fn fixture() -> Fixture {
    let fixture = Fixture::with_session("process", 1);
    fixture
        .scenario
        .launch_git(&["checkout", "-b", "reference-fix"]);
    fixture
        .scenario
        .commit_locally("feature.txt", "implemented\n", "Reference fix");
    fixture.scenario.commit_locally(
        "test_feature.py",
        r#"
import unittest
from test_support import feature, neighbor

class FeatureTests(unittest.TestCase):
    def test_fix(self):
        self.assertEqual(feature(), 'implemented\n')

    def test_neighbor(self):
        self.assertEqual(neighbor(), 'stable')
"#,
        "Reference tests",
    );
    fixture.scenario.commit_locally(
        "test_support.py",
        r#"
from pathlib import Path

def feature():
    return Path('feature.txt').read_text() if Path('feature.txt').exists() else 'missing'

def neighbor():
    return Path('neighbor.txt').read_text() if Path('neighbor.txt').exists() else 'stable'
"#,
        "Reference shared fakes",
    );
    let fix = fixture.scenario.launch_git(&["rev-parse", "HEAD"]);
    fixture.scenario.launch_git(&["checkout", "candidate"]);
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/process/tasks/fixture/task.toml");
    let text = fs::read_to_string(&task).unwrap();
    fs::write(
        task,
        text.replace(
            "session = \"process\"",
            "session = \"process\"\nprovenance = \"real\"\nenvironment = \"host\"",
        )
        .replace(
            &format!("change = \"{}\"", fixture.baseline),
            &format!("change = \"{}\"", fix.trim()),
        ) + r#"
[process_grading]
test_files = ["test_feature.py"]
test_fakes = ["test_support.py"]
fail_to_pass = [["python3", "-m", "unittest", "test_feature.FeatureTests.test_fix"]]
pass_to_pass = [["python3", "-m", "unittest", "test_feature.FeatureTests.test_neighbor"]]
"#,
    )
    .unwrap();
    fixture.scenario.agent_does(r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'Private report file:'; then
    test -f feature.txt
    report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
    printf '[]\n' > "$report"
    printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
else
    test ! -f src/prompt.rs
    git config user.name Eval
    git config user.email eval@example.invalid
    echo implemented > feature.txt
    git add feature.txt
    git commit -q -m 'Implement local Ticket'
fi
"#);
    fixture
}

#[test]
fn a_delivered_trial_without_the_fix_fails_its_selector() {
    let fixture = fixture();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&script.replace(
        "echo implemented > feature.txt",
        "echo broken > feature.txt",
    ));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
        let check = &result["score"]["checks"][1];
        assert_eq!(
            check["command"][3], "test_feature.FeatureTests.test_fix",
            "{check}"
        );
        assert!(
            check["reason"]
                .as_str()
                .unwrap()
                .contains("test_feature.FeatureTests.test_fix"),
            "{check}"
        );
    }
}

#[test]
fn a_committed_neighbor_regression_fails_pass_to_pass() {
    let fixture = fixture();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&script.replace(
        "git add feature.txt",
        "echo broken > neighbor.txt\ngit add feature.txt neighbor.txt",
    ));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
        let checks = &result["score"]["checks"];
        assert_eq!(checks[1]["passed"], true, "{checks}");
        assert_eq!(checks[2]["passed"], false, "{checks}");
        assert!(
            checks[2]["reason"]
                .as_str()
                .unwrap()
                .contains("test_feature.FeatureTests.test_neighbor"),
            "{checks}"
        );
    }
}

#[test]
fn trial_edits_to_tests_and_shared_fakes_are_replaced() {
    let fixture = fixture();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&script.replace(
        "echo implemented > feature.txt",
        r#"echo broken > feature.txt
cat > test_feature.py <<'PY'
import unittest
class FeatureTests(unittest.TestCase):
    def test_fix(self): pass
    def test_neighbor(self): pass
PY
cat > test_support.py <<'PY'
def feature(): return 'implemented\n'
def neighbor(): return 'stable'
PY
git add test_feature.py test_support.py"#,
    ));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
        assert_eq!(result["score"]["checks"][1]["passed"], false, "{result}");
        assert_eq!(result["score"]["checks"][2]["passed"], true, "{result}");
    }
}

#[test]
fn a_shadowed_standard_library_cannot_pass_a_broken_commit() {
    let fixture = fixture();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&script.replace(
        "echo implemented > feature.txt\n    git add feature.txt",
        r#"echo broken > feature.txt
cat > pathlib.py << 'PY'
class Path:
    def __init__(self, *args, **kwargs):
        self.name = str(args[0]) if args else ""
    def exists(self):
        return True
    def read_text(self):
        if self.name == "neighbor.txt":
            return "stable"
        return "implemented\n"
PY
git add feature.txt pathlib.py"#,
    ));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
        assert_eq!(result["score"]["checks"][1]["passed"], false, "{result}");
        assert_eq!(result["score"]["checks"][2]["passed"], true, "{result}");
    }
}

#[test]
fn uncommitted_repairs_do_not_change_the_graded_commit() {
    let fixture = fixture();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&format!(
        "{}\necho implemented > feature.txt\n",
        script.replace(
            "echo implemented > feature.txt",
            "echo broken > feature.txt"
        )
    ));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
        assert_eq!(result["score"]["checks"][1]["passed"], false, "{result}");
    }
}

fn set_selectors(fixture: &Fixture, fail_to_pass: &[&str], pass_to_pass: &[&str]) {
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/process/tasks/fixture/task.toml");
    let mut manifest = fs::read_to_string(&task)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    for (name, command) in [
        ("fail_to_pass", fail_to_pass),
        ("pass_to_pass", pass_to_pass),
    ] {
        let mut commands = toml_edit::Array::new();
        commands.push(command.iter().copied().collect::<toml_edit::Array>());
        manifest["process_grading"][name] = toml_edit::value(commands);
    }
    fs::write(task, manifest.to_string()).unwrap();
}

fn sandboxed_fixture() -> Option<Fixture> {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (sandboxed process Grader): bwrap is not on PATH");
        return None;
    }
    let fixture = fixture();
    super::sandbox::sandbox_graders(&fixture);
    Some(fixture)
}

#[test]
fn process_code_graders_have_no_host_network() {
    let Some(fixture) = sandboxed_fixture() else {
        return;
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = format!(
        r#"
import os, socket
assert os.readlink('/proc/self/ns/net') != {namespace:?}
try:
    socket.create_connection(('127.0.0.1', {port}), timeout=1)
except OSError:
    print('offline confirmed')
else:
    raise AssertionError('host network is reachable')
"#,
        namespace = fs::read_link("/proc/self/ns/net")
            .unwrap()
            .to_str()
            .unwrap()
    );
    set_selectors(&fixture, &["python3", "-c", &script], &["true"]);
    let run = fixture.scenario.run(&Fixture::experiment_args(&[]));
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
        assert!(
            result["score"]["checks"][1]["reason"]
                .as_str()
                .unwrap()
                .contains("offline confirmed"),
            "{result}"
        );
    }
}

#[test]
fn a_process_selector_timeout_is_a_failed_check() {
    let Some(fixture) = sandboxed_fixture() else {
        return;
    };
    set_selectors(
        &fixture,
        &[
            "python3",
            "-c",
            "import time; print('selector started', flush=True); time.sleep(60)",
        ],
        &["true"],
    );
    let run = fixture
        .scenario
        .run(&Fixture::experiment_args(&["--timeout", SHORT_TIMEOUT]));
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "delivered", "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
        let checks = &result["score"]["checks"];
        assert_eq!(checks[1]["passed"], false, "{checks}");
        assert!(
            checks[1]["reason"].as_str().unwrap().contains("timed out"),
            "{checks}"
        );
        assert_eq!(checks[2]["passed"], true, "{checks}");
    }
}

#[test]
fn invalid_process_grading_is_refused_before_trials_start() {
    let fixture = fixture();
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/process/tasks/fixture/task.toml");
    let original = fs::read_to_string(&task).unwrap();
    for (before, after, diagnostic) in [
        ("[process_grading]", "[unknown_grading]", "unknown field"),
        (
            "test_files = [\"test_feature.py\"]",
            "test_files = []",
            "requires test_files",
        ),
        (
            "test_feature.py",
            "../test_feature.py",
            "confined relative Eval path",
        ),
        (
            "test_files = [\"test_feature.py\"]",
            "test_files = [\".git/config\"]",
            "confined relative Eval path",
        ),
        (
            "fail_to_pass = [[\"python3\", \"-m\", \"unittest\", \"test_feature.FeatureTests.test_fix\"]]",
            "fail_to_pass = []",
            "requires fail_to_pass and pass_to_pass",
        ),
        (
            "pass_to_pass = [[\"python3\", \"-m\", \"unittest\", \"test_feature.FeatureTests.test_neighbor\"]]",
            "pass_to_pass = [[]]",
            "nonempty argument vectors",
        ),
    ] {
        fs::write(&task, original.replace(before, after)).unwrap();
        let run = fixture.run(&[]);
        assert_ne!(run.code, Some(0), "{before}: {}", run.stderr);
        assert!(run.stderr.contains(diagnostic), "{before}: {}", run.stderr);
        assert!(fixture.trial_calls().is_empty());
    }
    fs::write(&task, original.split("[process_grading]").next().unwrap()).unwrap();
    let run = fixture.run(&[]);
    assert_ne!(run.code, Some(0));
    assert!(
        run.stderr.contains("process tasks require process_grading"),
        "{}",
        run.stderr
    );
    assert!(fixture.trial_calls().is_empty());
}

#[test]
fn a_final_question_stops_the_process_and_counts_as_failed() {
    let fixture = fixture();
    fixture
        .scenario
        .agent_does("printf '%s\\n' 'Which option should I use?' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "stopped_to_ask", "{result}");
        assert_eq!(result["score"]["passed"], false);
        assert_eq!(
            result["attempts"][0]["steps"][0]["outcome"],
            "stopped_to_ask"
        );
    }
    assert_eq!(view["outcomes"]["baseline"]["stopped_to_ask"], 1);
    assert_eq!(view["outcomes"]["candidate"]["stopped_to_ask"], 1);
    assert_eq!(fixture.trial_calls().len(), 2);
    assert!(fixture.grader_calls().is_empty());
}

#[test]
fn github_processes_are_refused_before_any_trial_starts() {
    for arm in ["--baseline-process", "--candidate-process"] {
        let fixture = fixture();
        let run = fixture.run(&[arm, "spec-run"]);
        assert_ne!(run.code, Some(0));
        assert!(
            run.stderr.contains("requires GitHub before"),
            "{}",
            run.stderr
        );
        assert!(fixture.trial_calls().is_empty());
    }
}

#[test]
fn built_in_ticket_trials_deliver_and_keep_all_steps_and_sessions() {
    let fixture = fixture();
    let run = fixture.run(&[
        "--candidate-process",
        "ticket-run",
        "--baseline-process",
        "ticket-run",
    ]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    result_format::assert_documented("Experiment", &view);
    let results = view["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for trial in results {
        assert_eq!(trial["outcome"], "delivered", "{trial}");
        assert_eq!(trial["score"]["passed"], true, "{trial}");
        let checks = trial["score"]["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 3, "{checks:?}");
        assert_eq!(
            checks[1]["command"][3],
            "test_feature.FeatureTests.test_fix"
        );
        assert_eq!(
            checks[2]["command"][3],
            "test_feature.FeatureTests.test_neighbor"
        );
        let steps = trial["attempts"][0]["steps"].as_array().unwrap();
        assert_eq!(
            steps
                .iter()
                .map(|s| s["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["implement", "security-review"]
        );
        assert!(steps.iter().all(|s| s["outcome"] == "succeeded"));
    }
    assert_eq!(view["sessions"].as_array().unwrap().len(), 4, "{view}");
    assert_eq!(view["outcomes"]["baseline"]["delivered"], 1);
    assert_eq!(view["outcomes"]["candidate"]["delivered"], 1);
    assert_eq!(fixture.trial_calls().len(), 4);
    assert!(fixture.grader_calls().is_empty());
    assert!(fixture.scenario.gh_calls().is_empty());
    let status = fixture.scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 1, "{status}");
    assert_eq!(commands[0]["kind"], "Experiment");
    for session in view["sessions"].as_array().unwrap() {
        let evidence = fixture.scenario.run(&[
            "experiments",
            view["id"].as_str().unwrap(),
            "--session",
            session["id"].as_str().unwrap(),
            "--json",
        ]);
        assert_eq!(evidence.code, Some(0), "{}", evidence.stderr);
    }
    let record = fixture
        .scenario
        .command_record("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment");
    let export = fixture.scenario.path("process-trace");
    let trace = fixture.scenario.run(&[
        "logs",
        record["name"].as_str().unwrap(),
        "--export",
        export.to_str().unwrap(),
    ]);
    assert_eq!(trace.code, Some(0), "{}", trace.stderr);
    let spans = trace_spans(&export.join("spans.jsonl"));
    let steps: Vec<_> = spans
        .iter()
        .filter(|s| s["name"].as_str().unwrap().starts_with("Step "))
        .collect();
    assert_eq!(steps.len(), 4, "{spans:?}");
    let sessions: Vec<_> = spans
        .iter()
        .filter(|s| s["name"].as_str().unwrap().starts_with("invoke_agent "))
        .collect();
    assert_eq!(sessions.len(), 4, "{spans:?}");
    assert!(
        sessions
            .iter()
            .all(|s| steps.iter().any(|step| step["spanId"] == s["parentSpanId"]))
    );
}

fn custom_process(fixture: &Fixture) -> PathBuf {
    let skill = fixture.scenario.path("home/.thirdshift/skills/fixture");
    fs::create_dir_all(skill.join("scripts")).unwrap();
    fs::write(skill.join("SKILL.md"), "Frozen fixture instructions\n").unwrap();
    fs::write(skill.join("scripts/helper.txt"), "Frozen helper\n").unwrap();
    let path = fixture.scenario.path("custom-ticket.toml");
    fs::write(
        &path,
        r#"version = 1
description = "Custom Trial fixture"
[worktrees.ticket]
kind = "issue-branch"
[[steps]]
name = "custom-implement"
skill = "fixture"
worktree = "ticket"
model = "opus"
effort = "high"
[[steps]]
name = "check"
command = "test -f feature.txt"
worktree = "ticket"
[[steps]]
name = "delivery"
catalogue = "delivery"
worktree = "ticket"
"#,
    )
    .unwrap();
    path
}

#[test]
fn custom_process_trials_use_frozen_home_skills_beside_the_builtin_baseline() {
    let fixture = fixture();
    let process = custom_process(&fixture);
    let original = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q custom-implement; then
    test "$(cat .claude/skills/fixture/SKILL.md)" = 'Frozen fixture instructions'
    test "$(cat .claude/skills/fixture/scripts/helper.txt)" = 'Frozen helper'
fi
echo 'changed original' > '{}'
echo 'changed original' > '{}'
{original}
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q custom-implement; then
    printf 'Custom implementation complete.\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
fi
"#,
        process.display(),
        fixture
            .scenario
            .path("home/.thirdshift/skills/fixture/SKILL.md")
            .display()
    ));
    let run = fixture.run(&[
        "--candidate-process",
        process.to_str().unwrap(),
        "--trials-per-task",
        "3",
    ]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(view["results"].as_array().unwrap().len(), 6);
    for trial in view["results"].as_array().unwrap() {
        assert_eq!(trial["outcome"], "delivered", "{trial}");
        let steps = trial["attempts"][0]["steps"].as_array().unwrap();
        let names: Vec<_> = steps
            .iter()
            .map(|step| step["name"].as_str().unwrap())
            .collect();
        if trial["arm"] == "candidate" {
            assert_eq!(names, ["custom-implement", "check"]);
        } else {
            assert_eq!(names, ["implement", "security-review"]);
        }
    }
    assert_eq!(
        view["prepared"]["candidate"]["process"]["steps"][0]["model"],
        "opus"
    );
    assert!(view["prepared"]["baseline"]["process"]["files"].is_object());
    assert!(fixture.scenario.gh_calls().is_empty());
    result_format::assert_documented("Experiment", &view);
}

#[test]
fn changing_a_frozen_file_between_trials_stops_the_experiment_with_its_name() {
    for name in [
        ".thirdshift/processes/custom-ticket.toml",
        ".thirdshift/skills/fixture/scripts/helper.txt",
    ] {
        let fixture = fixture();
        let process = custom_process(&fixture);
        let original = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
        fixture.scenario.agent_does(&format!(
            r#"
python3 - <<'PY'
from pathlib import Path
root = Path({root:?})
for path in root.glob('*/.arm-inputs-*/candidate/{name}'):
    path.write_text('tampered frozen file\n')
PY
{original}
printf 'Complete.\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
            root = fixture.scenario.path("home/.thirdshift/evals/experiments")
        ));
        let run = fixture.run(&[
            "--candidate-process",
            process.to_str().unwrap(),
            "--trials-per-task",
            "3",
        ]);
        assert_ne!(run.code, Some(0), "{}", run.stderr);
        assert!(
            run.stderr
                .contains(&format!("frozen Arm file changed: {name}")),
            "{}",
            run.stderr
        );
        assert!(
            fixture.trial_calls().len() <= 3,
            "started another Trial after tampering: {}",
            run.stderr
        );
        let saved = fixture.experiment();
        assert_eq!(saved["state"], "paused");
        assert!(saved["results"].as_array().unwrap().len() < 6);
    }
}

#[test]
fn a_custom_skill_question_stops_the_trial() {
    let fixture = fixture();
    let process = custom_process(&fixture);
    fixture.scenario.agent_does(
        "printf 'Which implementation should I use?\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );
    let run = fixture.run(&["--candidate-process", process.to_str().unwrap()]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    let trial = view["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|trial| trial["arm"] == "candidate")
        .unwrap();
    assert_eq!(trial["outcome"], "stopped_to_ask", "{trial}");
    assert_eq!(trial["attempts"][0]["steps"].as_array().unwrap().len(), 1);
}

#[test]
fn custom_step_uses_its_pinned_harness_model_and_effort() {
    let fixture = fixture();
    let process = custom_process(&fixture);
    let text = fs::read_to_string(&process).unwrap();
    fs::write(
        &process,
        text.replace(
            "model = \"opus\"",
            "harness = \"codex\"\nmodel = \"gpt-6.1-sol\"",
        )
        .replace("effort = \"high\"", "effort = \"max\""),
    )
    .unwrap();
    let original = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"
ln -sf /missing/updated-codex '{}'
{original}
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q custom-implement; then
    printf 'Complete.\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
fi
"#,
        fixture.scenario.path("bin/codex").display()
    ));
    let run = fixture.run(&["--candidate-process", process.to_str().unwrap()]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for trial in view["results"].as_array().unwrap() {
        if trial["arm"] == "baseline" {
            assert_eq!(trial["outcome"], "delivered", "{trial}");
        } else {
            assert_eq!(trial["outcome"], "void", "{trial}");
            let attempt = &trial["attempts"][0];
            assert_eq!(attempt["void_cause"], "accounting_gap", "{attempt}");
            assert!(
                attempt["cause"]
                    .as_str()
                    .unwrap()
                    .contains("missing observed Model"),
                "{attempt}"
            );
            assert_eq!(attempt["steps"][0]["name"], "custom-implement");
            assert_eq!(attempt["steps"][0]["outcome"], "succeeded");
        }
    }
    let calls = fixture.scenario.codex_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    let args = calls[0]["argv"].as_array().unwrap();
    assert!(args.iter().any(|arg| arg == "gpt-6.1-sol"));
    assert!(
        args.iter()
            .any(|arg| arg.as_str().is_some_and(|word| word.contains("max")))
    );
    assert_eq!(
        view["prepared"]["candidate"]["process"]["steps"][0]["harness"]["name"],
        "codex"
    );
}

#[test]
fn a_custom_candidate_can_compare_with_a_builtin_only_baseline_protocol() {
    let fixture = fixture();
    let process = custom_process(&fixture);
    let binary = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/builds/{}/thirdshift",
        fixture.baseline
    ));
    fs::write(
        &binary,
        format!(
            r#"#!/bin/sh
set -eu
if [ "$1" = --trial ]; then
    python3 - "$2" <<'PY'
import json, sys
assert json.load(open(sys.argv[1]))['process'] == 'ticket-run'
PY
fi
exec '{}' "$@"
"#,
            env!("CARGO_BIN_EXE_thirdshift")
        ),
    )
    .unwrap();
    let original = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&format!("{original}\nif printf '%s' \"$FAKE_CLAUDE_PROMPT\" | grep -q custom-implement; then printf 'Complete.\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"; fi\n"));
    let run = fixture.run(&["--candidate-process", process.to_str().unwrap()]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert!(
        view["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|trial| trial["outcome"] == "delivered"),
        "{view}"
    );
}

#[test]
fn a_security_review_question_keeps_its_stopped_to_ask_outcome() {
    let fixture = fixture();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"{script}
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'Private report file:'; then
    printf '%s\n' 'Should I fix this finding?' > "$FAKE_CLAUDE_FINAL_MESSAGE"
fi
"#
    ));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "stopped_to_ask", "{result}");
        assert_eq!(result["score"]["passed"], false);
        assert_eq!(
            result["attempts"][0]["steps"][1]["outcome"],
            "stopped_to_ask"
        );
    }
}

#[test]
fn spec_process_outcome_counts_exclude_failed_review_trials() {
    let fixture = fixture();
    let suites = fixture.scenario.path("home/.thirdshift/evals/suites");
    let process = suites.join("process");
    let review = suites.join("spec-review");
    fs::create_dir_all(review.join("tasks/fixture")).unwrap();
    fs::copy(process.join("suite.toml"), review.join("suite.toml")).unwrap();
    fs::copy(
        process.join("tasks/fixture/spec.md"),
        review.join("tasks/fixture/spec.md"),
    )
    .unwrap();
    let manifest = fs::read_to_string(process.join("tasks/fixture/task.toml")).unwrap();
    fs::write(
        review.join("tasks/fixture/task.toml"),
        manifest
            .split("[process_grading]")
            .next()
            .unwrap()
            .replace("session = \"process\"", "session = \"spec-review\""),
    )
    .unwrap();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'Every Ticket of the Spec'; then
    exit 17
fi
{script}
"#
    ));
    let run = fixture.run(&["--suite", "process", "--suite", "spec-review"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for arm in ["baseline", "candidate"] {
        let results: Vec<_> = view["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|result| result["arm"] == arm)
            .collect();
        assert!(
            results
                .iter()
                .any(|r| r["suite"] == "process" && r["outcome"] == "delivered"),
            "{view}"
        );
        assert!(
            results
                .iter()
                .any(|r| r["suite"] == "spec-review" && r["outcome"] == "failed"),
            "{view}"
        );
        assert_eq!(view["outcomes"][arm]["delivered"], 1, "{view}");
        assert_eq!(
            view["outcomes"][arm]["failed"], 0,
            "failed review Trial contaminated Process Trial outcomes: {view}"
        );
    }
}

#[test]
fn spec_a_markdown_final_question_is_stopped_to_ask() {
    let fixture = fixture();
    let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"{script}
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q '^/thirdshift-implement'; then
    printf '%s\n' '**Which option should I use?**' > "$FAKE_CLAUDE_FINAL_MESSAGE"
fi
"#
    ));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(
            result["outcome"], "stopped_to_ask",
            "a final Markdown question was scored as completed: {result}"
        );
        assert_eq!(result["score"]["passed"], false, "{result}");
    }
    assert_eq!(view["outcomes"]["baseline"]["stopped_to_ask"], 1);
    assert_eq!(view["outcomes"]["candidate"]["stopped_to_ask"], 1);
}
