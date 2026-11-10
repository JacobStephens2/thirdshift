use super::*;

#[test]
fn architecture_describes_the_exact_arm_history_used_for_estimates() {
    let text = include_str!("../../site/architecture/index.html");
    assert!(
        !text.contains("completed Trials per suite and Harness"),
        "The architecture page still documents the replaced suite/Harness estimate key"
    );
}

#[test]
fn a_hard_killed_process_session_keeps_its_starting_step_role() {
    let fixture = process::fixture();
    fixture.scenario.agent_does(
        r#"
python3 - <<'PY'
import os
from pathlib import Path
import signal

# Walk only this fixture's ancestry and kill the Arm, whose exact argv
# contains --trial. This prevents the active Step's completion publication.
pid = os.getppid()
for _ in range(32):
    if pid <= 1:
        break
    args = Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
    if b'--trial' in args:
        os.kill(pid, signal.SIGKILL)
        break
    status = Path(f'/proc/{pid}/status').read_text().splitlines()
    pid = int(next(line.split()[1] for line in status if line.startswith('PPid:')))
else:
    raise AssertionError('Trial ancestor search exceeded its bound')
if pid <= 1:
    raise AssertionError('No fixture Trial ancestor found')
PY
"#,
    );
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    let trials = view["accounting"]["trials"].as_array().unwrap();
    assert_eq!(trials.len(), 2, "{view}");
    for trial in trials {
        let sessions = trial["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1, "{trial}");
        assert_eq!(sessions[0]["role"], "implement", "{trial}");
    }
}

#[test]
fn legacy_trials_with_unknown_prices_still_report_paired_time() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view = fixture.experiment();
    let root = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        view["id"].as_str().unwrap()
    ));
    let metadata = root.join("experiment.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    saved.as_object_mut().unwrap().remove("pricing");
    fs::write(metadata, saved.to_string()).unwrap();
    for entry in fs::read_dir(root.join("results")).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let time_ms = if result["arm"] == "baseline" {
            1000
        } else {
            2000
        };
        assert_ne!(result["outcome"], "void", "{result}");
        for attempt in result["attempts"].as_array_mut().unwrap() {
            attempt.as_object_mut().unwrap().remove("accounting");
            attempt["timings"]["total_ms"] = json!(time_ms);
        }
        fs::write(path, result.to_string()).unwrap();
    }
    let shown = fixture.experiment();
    assert!(shown["pricing"].is_null());
    assert!(shown["accounting"]["paired_median_ratios"]["cost"].is_null());
    assert_eq!(shown["accounting"]["paired_median_ratios"]["time"], 2.0);
}

#[test]
fn estimates_use_each_arms_own_history_and_include_frozen_costs() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    assert_eq!(fixture.run(&[]).code, Some(0));
    let view = fixture.experiment();
    assert_eq!(view["results"][0]["outcome"], "succeeded", "{view}");
    let root = fixture.scenario.path("home/.thirdshift/evals/experiments");
    let old = root.join(view["id"].as_str().unwrap());
    let history = root.join("b".repeat(64));
    fs::rename(old, &history).unwrap();
    let metadata = history.join("experiment.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    saved["id"] = json!("b".repeat(64));
    fs::write(metadata, saved.to_string()).unwrap();
    for entry in fs::read_dir(history.join("results")).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let (time, cost) = if result["arm"] == "baseline" {
            (120_000, 2.0)
        } else {
            (540_000, 9.0)
        };
        result["attempts"][0]["timings"]["total_ms"] = json!(time);
        result["attempts"][0]["accounting"]["sessions"][0]["cost_usd"] = json!(cost);
        fs::write(path, result.to_string()).unwrap();
    }
    let run = fixture.run_on_terminal(&[], &[("Run these Trials? [y/N] ", "n")]);
    assert!(
        run.stderr
            .contains("Baseline arm (claude): 1 Trial, estimate 2 session-minutes, $2.000000"),
        "{}",
        run.stderr
    );
    assert!(
        run.stderr
            .contains("Candidate arm (claude): 1 Trial, estimate 9 session-minutes, $9.000000"),
        "{}",
        run.stderr
    );
}

#[test]
fn frozen_prices_report_every_process_session_by_role_trial_and_arm() {
    let fixture = process::fixture();
    fixture.scenario.agent_does(r#"
echo '{"type":"assistant","message":{"id":"priced","model":"fixture-model","usage":{"input_tokens":1000000,"output_tokens":100000,"cache_read_input_tokens":200000,"cache_creation_input_tokens":100000},"content":[]}}'
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'Private report file:'; then
  report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
  printf '[]\n' > "$report"
  printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
fi
"#);
    let prices = fixture.scenario.path("prices.json");
    fs::write(
        &prices,
        json!({"date":"2026-11-09", "source":"fixture", "models": {
            "fixture-model": {"input":2.0,"output":10.0,"cache_read":0.2,"cache_write":2.5}
        }})
        .to_string(),
    )
    .unwrap();
    let run = fixture.run(&["--prices", prices.to_str().unwrap()]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(view["pricing"]["date"], "2026-11-09");
    for arm in ["baseline", "candidate"] {
        let measured = &view["accounting"][arm];
        assert_eq!(measured["sessions"], 2);
        assert_eq!(measured["cost_usd"], 6.58);
        assert_eq!(measured["roles"]["implement"]["cost_usd"], 3.29);
        assert_eq!(measured["roles"]["security-review"]["cost_usd"], 3.29);
    }
    for trial in view["accounting"]["trials"].as_array().unwrap() {
        assert_eq!(trial["cost_usd"], 6.58);
        assert_eq!(trial["sessions"].as_array().unwrap().len(), 2);
    }
    assert_eq!(view["accounting"]["paired_median_ratios"]["cost"], 1.0);
    // A later read uses the frozen table even if its input file changes.
    fs::write(prices, "{}").unwrap();
    assert_eq!(fixture.experiment()["accounting"], view["accounting"]);
}

#[test]
fn a_session_without_usage_voids_its_trial_with_an_accounting_gap() {
    let fixture = process::fixture();
    // An explicit empty usage packet suppresses the fixture Harness's default.
    fixture.scenario.agent_does("echo '{\"type\":\"assistant\",\"message\":{\"model\":\"fixture-model\",\"usage\":{},\"content\":[]}}'");
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view: Value = serde_json::from_str(&run.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "void", "{result}");
        assert_eq!(result["attempts"][0]["void_cause"], "accounting_gap");
        assert!(
            result["attempts"][0]["cause"]
                .as_str()
                .unwrap()
                .contains("missing usage")
        );
    }
}

#[test]
fn paired_ratios_are_medians_of_matched_trials_and_exclude_void_pairs() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    assert_eq!(fixture.run(&["--trials-per-task", "3"]).code, Some(0));
    let view = fixture.experiment();
    let results = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}/results",
        view["id"].as_str().unwrap()
    ));
    for entry in fs::read_dir(&results).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let index = result["trial"].as_u64().unwrap() as usize;
        let value = if result["arm"] == "baseline" {
            [1, 2, 100][index]
        } else {
            [2, 4, 1][index]
        };
        result["attempts"][0]["timings"]["total_ms"] = json!(value * 1000);
        result["attempts"][0]["accounting"]["sessions"][0]["cost_usd"] = json!(value);
        fs::write(path, result.to_string()).unwrap();
    }
    let shown = fixture.experiment();
    assert_eq!(
        shown["accounting"]["paired_median_ratios"],
        json!({"cost":2.0,"time":2.0,"pairs":3})
    );
    for entry in fs::read_dir(&results).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if result["arm"] == "candidate" && result["trial"] == 2 {
            result["outcome"] = json!("void");
            fs::write(path, result.to_string()).unwrap();
        }
    }
    assert_eq!(
        fixture.experiment()["accounting"]["paired_median_ratios"],
        json!({"cost":2.0,"time":2.0,"pairs":2})
    );
}

#[test]
fn time_keeps_attempt_work_and_excludes_a_usage_limit_pause_between_invocations() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
echo '{"type":"assistant","message":{"id":"priced","model":"fixture-model","usage":{"input_tokens":10,"output_tokens":2},"content":[]}}'
echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["quota exceeded"]}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#);
    assert_eq!(fixture.run(&[]).code, Some(1));
    let paused = fixture.experiment();
    let metadata = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}/experiment.json",
        paused["id"].as_str().unwrap()
    ));
    // Fake a long wait at the clock boundary without sleeping in the test.
    let mut saved: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    saved["created_at"] = json!("2000-01-01T00:00:00Z");
    fs::write(metadata, saved.to_string()).unwrap();
    fixture.scenario.agent_does("true");
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let view: Value = serde_json::from_str(&resumed.stdout).unwrap();
    for result in view["results"].as_array().unwrap() {
        let time: u64 = result["attempts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|attempt| attempt["timings"]["total_ms"].as_u64().unwrap())
            .sum();
        let trial = view["accounting"]["trials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|trial| trial["arm"] == result["arm"])
            .unwrap();
        assert_eq!(trial["time_ms"], time);
        assert!(time < 60_000, "the fake pause must not enter active time");
        assert_ne!(result["outcome"], "void", "{result}");
    }
}
