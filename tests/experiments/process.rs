use super::*;

fn fixture() -> Fixture {
    let fixture = Fixture::with_session("process", 1);
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
            &format!("change = \"{}\"", fixture.candidate),
        ),
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
    assert!(
        view["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|trial| trial["outcome"] == "delivered"),
        "{view}"
    );
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
        manifest.replace("session = \"process\"", "session = \"spec-review\""),
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
