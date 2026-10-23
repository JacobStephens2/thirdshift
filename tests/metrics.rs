mod support;

use std::fs;

use serde_json::{Value, json};
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";

fn trace(
    scenario: &Scenario,
    name: &str,
    harness: &str,
    version: &str,
    events: Value,
) -> std::path::PathBuf {
    let path = scenario.path(LOGS).join(format!("{name}.logs.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_string(&json!({
        "resourceLogs": [{"resource": {"attributes": [
            {"key":"thirdshift.harness", "value":{"stringValue":harness}},
            {"key":"thirdshift.harness.directory", "value":{"stringValue":"/worktree"}},
            {"key":"thirdshift.harness.version", "value":{"stringValue":version}}
        ]}, "scopeLogs":[{"logRecords":[{"attributes":[{
            "key":"thirdshift.events", "value":{"stringValue":serde_json::to_string(&events).unwrap()}
        }]}]}]}]
    })).unwrap() + "\n").unwrap();
    path
}

#[test]
fn destructive_command_patterns_flag_fixture_commands_but_not_near_misses() {
    let scenario = Scenario::new();
    let cases: Value =
        serde_json::from_str(include_str!("fixtures/destructive-commands.json")).unwrap();
    let events: Vec<_> = cases
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, case)| {
            json!({"type":"tool_call_started", "id":i.to_string(), "parent_id":null,
            "tool":case["tool"], "input":case["input"]})
        })
        .collect();
    let log = trace(&scenario, "destructive", "claude", "1.0", json!(events));
    let mut past = record(7);
    past["stages"][0]["session_log"] = json!(log);
    write(&scenario, "7-fixture.command.json", &past);
    let shown = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    let flags = &shown["command"]["metrics"]["session_signals"]["implement"]["signals"]["destructive_commands"];
    let expected: Vec<_> = cases
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|case| {
            case["pattern"].as_str().map(|pattern| {
                json!({
                    "pattern":pattern, "command":case["input"]["command"]
                })
            })
        })
        .collect();
    let mut actual = flags.as_array().expect("destructive command flags").clone();
    actual.sort_by_key(|flag| flag["command"].to_string());
    let mut expected = expected;
    expected.sort_by_key(|flag| flag["command"].to_string());
    assert_eq!(actual, expected);
}

#[test]
fn shell_commands_from_each_harness_are_checked() {
    for (harness, tool, key) in [
        ("claude", "Bash", "command"),
        ("codex", "command_execution", "command"),
        ("agy", "run_command", "CommandLine"),
        ("grok", "run_terminal_command", "command"),
        ("muse", "bash", "command"),
        ("opencode", "bash", "command"),
    ] {
        let scenario = Scenario::new();
        let log = trace(
            &scenario,
            harness,
            harness,
            "1.0",
            json!([
                {"type":"tool_call_started", "id":"call", "parent_id":null,
                 "tool":tool, "input":{key:"git push --force origin issue-7"}}
            ]),
        );
        let mut past = record(7);
        past["stages"][0]["session_log"] = json!(log);
        write(&scenario, "7-fixture.command.json", &past);
        let shown = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
        assert_eq!(
            shown["command"]["metrics"]["session_signals"]["implement"]["signals"]["destructive_commands"],
            json!([{"pattern":"force-push", "command":"git push --force origin issue-7"}]),
            "{harness}"
        );
    }
}

#[test]
fn session_signals_sum_by_kind_and_trends_split_by_trace_harness_version() {
    let scenario = Scenario::new();
    let first = trace(
        &scenario,
        "first",
        "codex",
        "1.0",
        json!([
            {"type":"turn_ended", "id":"t", "usage":{"input":100,"output":7,"cache_read":20}, "outcome":"succeeded"},
            {"type":"tool_call_ended", "id":"a", "parent_id":null, "tool":"bash", "input":{"command":"false"}, "result":null, "outcome":"failed", "amended":false}
        ]),
    );
    let second = trace(
        &scenario,
        "second",
        "codex",
        "2.0",
        json!([
            {"type":"turn_ended", "id":"t", "usage":{"input":200,"output":9,"cache_read":40}, "outcome":"succeeded"},
            {"type":"tool_call_ended", "id":"a", "parent_id":null, "tool":"bash", "input":{"command":"false"}, "result":null, "outcome":"failed", "amended":false}
        ]),
    );
    let mut past = record(7);
    past["stages"] = json!([
        {"name":"Repair", "began_at":"2026-10-19T03:30:00Z", "session_log":first},
        {"name":"Repair", "began_at":"2026-10-19T03:40:00Z", "session_log":second},
        {"name":"pull request", "began_at":"2026-10-19T03:50:00Z", "session_log":second}
    ]);
    write(&scenario, "7-fixture.command.json", &past);
    let shown = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    let summary = &shown["command"]["metrics"]["session_signals"]["Repair"];
    assert_eq!(summary["observed_sessions"], 2);
    assert_eq!(
        summary["signals"]["tokens_by_model"][0]["input_tokens"],
        300
    );
    assert_eq!(summary["signals"]["tool_errors"], 2);
    assert_eq!(
        summary["signals"]["repeated_tool_calls"], 0,
        "repetition never crosses sessions"
    );
    let trends = metrics(&scenario, &["metrics", "--json"]);
    let repairs: Vec<_> = trends["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["session_kind"] == "Repair")
        .collect();
    assert_eq!(repairs.len(), 2, "{trends:#}");
    for (version, tokens) in [("1.0", 100.0), ("2.0", 200.0)] {
        let row = repairs
            .iter()
            .find(|row| row["harness_version"] == version)
            .unwrap();
        assert_eq!(row["runs"], 1);
        assert_eq!(
            row["session_signals"]["tool_errors"],
            json!({"observed":1,"missing":0,"sum":1.0,"mean":1.0})
        );
        assert_eq!(
            row["session_signals"]["tokens_by_model"][0]["input_tokens"]["sum"],
            tokens
        );
        assert_eq!(row["session_signals"]["cost_usd"]["missing"], 1);
    }
}

#[test]
fn converted_session_traces_supply_signals_without_rewriting_past_metrics() {
    let scenario = Scenario::new();
    let trace = trace(
        &scenario,
        "converted-implement",
        "codex",
        "1.2.3",
        json!([
            {"type":"turn_ended", "id":"turn-1", "usage":{"input":100,"output":7,"cache_read":20}, "outcome":"succeeded"}
        ]),
    );
    let mut past = record(7);
    past["stages"][0]["session_log"] = json!(trace.with_file_name("converted-implement.jsonl"));
    let path = write(&scenario, "7-fixture.command.json", &past);
    let original = fs::read(&path).unwrap();
    let result = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    let signals = &result["command"]["metrics"]["session_signals"]["implement"];
    assert_eq!(signals["observed_sessions"], 1);
    assert_eq!(signals["missing_sessions"], 0);
    assert_eq!(
        signals["signals"]["tokens_by_model"],
        json!([{
            "model": null, "input_tokens":100, "output_tokens":7,
            "cache_read_tokens":20, "cache_write_tokens":null
        }])
    );
    assert_eq!(signals["signals"]["cost_usd"], Value::Null);
    assert_eq!(signals["signals"]["tool_errors"], 0);
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(trace).unwrap();
    let absent = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    assert_eq!(
        absent["command"]["metrics"]["session_signals"]["implement"]["missing_sessions"],
        1
    );
    assert_eq!(
        absent["command"]["metrics"]["session_signals"]["implement"]["signals"],
        Value::Null
    );
}

#[test]
fn killed_tools_and_subagents_count_with_or_without_companion_work_events() {
    let scenario = Scenario::new();
    let mut events = json!([
        {"type":"tool_call_ended", "id":"command-1", "parent_id":null,
         "tool":"command_execution", "input":{"command":"sleep 100"},
         "result":null, "outcome":"killed", "amended":false},
        {"type":"subagent_ended", "id":"agent-1", "outcome":"killed", "result":null}
    ]);
    let log = trace(&scenario, "killed", "codex", "1.0", events.clone());
    let mut past = record(7);
    past["stages"][0]["session_log"] = json!(log);
    write(&scenario, "7-fixture.command.json", &past);
    let shown = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    assert_eq!(
        shown["command"]["metrics"]["session_signals"]["implement"]["signals"]["killed_work"],
        2
    );
    events.as_array_mut().unwrap().extend([
        json!({"type":"work_ended", "id":"command-1", "outcome":"killed"}),
        json!({"type":"work_ended", "id":"agent-1", "outcome":"killed"}),
    ]);
    trace(&scenario, "killed", "codex", "1.0", events);
    let shown = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    assert_eq!(
        shown["command"]["metrics"]["session_signals"]["implement"]["signals"]["killed_work"],
        2
    );
}

#[test]
fn session_usage_keeps_its_reported_model() {
    let scenario = Scenario::new();
    let log = trace(
        &scenario,
        "agy",
        "agy",
        "1.0",
        json!([
            {"type":"usage", "scope":"session", "model":"gemini-test",
             "usage":{"input":100,"output":7,"cache_read":20,"cache_write":0}, "cost_usd":0.25}
        ]),
    );
    let mut past = record(7);
    past["stages"][0]["session_log"] = json!(log);
    write(&scenario, "7-fixture.command.json", &past);
    let shown = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    assert_eq!(
        shown["command"]["metrics"]["session_signals"]["implement"]["signals"]["tokens_by_model"],
        json!([{
            "model":"gemini-test", "input_tokens":100, "output_tokens":7,
            "cache_read_tokens":20, "cache_write_tokens":0
        }])
    );
}

#[test]
#[cfg(target_os = "linux")]
fn non_regular_session_traces_are_missing_without_blocking() {
    use support::child_lifetime::FixtureChild;
    let scenario = Scenario::new();
    let pipe = scenario.path("broken.logs.jsonl");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap()
            .success()
    );
    let trace = scenario.path("broken-link.logs.jsonl");
    std::os::unix::fs::symlink(pipe, &trace).unwrap();
    let mut past = record(7);
    past["stages"][0]["session_log"] = json!(trace);
    write(&scenario, "7-fixture.command.json", &past);
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command
        .args(["metrics", "widgets#7", "--json"])
        .env("HOME", scenario.path("home"));
    let output = FixtureChild::spawn(&mut command, "").wait(std::time::Duration::from_secs(3));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["command"]["metrics"]["session_signals"]["implement"],
        json!({
            "observed_sessions":0,"missing_sessions":1,"signals":null
        })
    );
}

#[test]
fn unaddressed_findings_stay_separate_in_run_views_and_weekly_trends() {
    let scenario = Scenario::new();
    write(&scenario, "7-fixture.command.json", &record(7));
    let mut saved = record(8);
    saved["run_metrics"] = json!({
        "outcome": "ready for review", "repairs": 0, "ci_rounds": 1,
        "total_minutes": 30.0, "stages": [],
        "unaddressed_findings": {"standards": 2, "spec": 0, "security": null}
    });
    write(&scenario, "8-fixture.command.json", &saved);
    let one = metrics(&scenario, &["metrics", "widgets#8", "--json"]);
    assert_eq!(
        one["command"]["metrics"]["unaddressed_findings"],
        saved["run_metrics"]["unaddressed_findings"]
    );
    let old = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    assert_eq!(
        old["command"]["metrics"]["unaddressed_findings"],
        Value::Null
    );
    let trends = metrics(&scenario, &["metrics", "--json"]);
    let run = trends["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_kind"].is_null())
        .unwrap();
    assert_eq!(
        run["unaddressed_findings"]["standards"],
        json!({"observed": 1, "missing": 1, "sum": 2.0, "mean": 2.0})
    );
    assert_eq!(
        run["unaddressed_findings"]["spec"],
        json!({"observed": 1, "missing": 1, "sum": 0.0, "mean": 0.0})
    );
    assert_eq!(
        run["unaddressed_findings"]["security"],
        json!({"observed": 0, "missing": 2, "sum": null, "mean": null})
    );
}

#[test]
fn an_unrecorded_security_resume_setup_stays_missing() {
    let scenario = Scenario::new();
    let mut past = record(7);
    past["stages"][0]["name"] = "Security review".into();
    past["stages"][1]["name"] = "Resume".into();
    write(&scenario, "7-fixture.command.json", &past);
    let trends = metrics(&scenario, &["metrics", "--json"]);
    let resume = trends["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_kind"] == "Resume")
        .unwrap();
    assert_eq!(resume["harness"], Value::Null, "{resume:#}");
    assert_eq!(resume["model"], Value::Null, "{resume:#}");
    assert_eq!(resume["effort"], Value::Null, "{resume:#}");
}

#[test]
fn security_review_trends_use_the_sessions_own_setup() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        r#"
[harness]
default = "codex"
[harness.codex]
model = "gpt-6.1-sol"
effort = "low"
[harness.claude]
model = "claude-opus-5-5"
effort = "high"
[security]
harness = "claude"
"#,
    );
    scenario.agent_does_for(7, r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
printf '%s' "$FAKE_CLAUDE_PROMPT" > review-prompt.txt
report=$(sed -n 's/^Private report file: `\(.*\)`\.$/\1/p' review-prompt.txt)
test -n "$report"
printf '[]\n' > "$report"
printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
else
echo feature > feature.txt
git add feature.txt
git commit -qm 'Add feature'
gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'
fi
"#);
    let ended = scenario.run(&[&scenario.issue_url(7), "security-review"]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let sidecar = scenario
        .entries(LOGS)
        .into_iter()
        .find(|name| name.ends_with("-security-review.models.json"))
        .unwrap();
    let evidence: Value =
        serde_json::from_slice(&fs::read(scenario.path(LOGS).join(sidecar)).unwrap()).unwrap();
    assert_eq!(evidence["harness"], "claude");
    assert_eq!(evidence["requested"], "claude-opus-5-5");
    assert_eq!(evidence["effort"], "high");
    let result = metrics(&scenario, &["metrics", "--json"]);
    let review = result["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_kind"] == "Security review")
        .unwrap();
    assert_eq!(review["harness"], evidence["harness"], "{review:#}");
    assert_eq!(review["model"], evidence["requested"], "{review:#}");
    assert_eq!(review["effort"], evidence["effort"], "{review:#}");
}

fn record(number: u32) -> Value {
    json!({
        "version": 1, "kind": "Run", "repository": "acme/widgets",
        "name": format!("acme/widgets#{number}"),
        "issue": {"number": number, "url": format!("https://github.com/acme/widgets/issues/{number}")},
        "hostname": "factory", "process_id": 0, "process_start": "gone",
        "harness": "codex", "model": "sol", "effort": "high",
        "started_at": "2026-10-18T23:30:00-04:00", "ended_at": "2026-10-19T04:00:00Z",
        "stages": [
            {"name": "implement", "began_at": "2026-10-19T03:30:00Z", "session_log": "/missing/implement.logs.jsonl"},
            {"name": "CI watch", "began_at": "2026-10-19T03:50:00Z"}
        ],
        "outcome": "ready for review", "session_log": null, "cause": null,
        "issue_branch": null, "pull_request": null
    })
}

fn write(scenario: &Scenario, name: &str, record: &Value) -> std::path::PathBuf {
    let folder = scenario.path(LOGS);
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join(name);
    fs::write(&path, serde_json::to_vec(record).unwrap()).unwrap();
    path
}

fn metrics(scenario: &Scenario, args: &[&str]) -> Value {
    let result = scenario.run(args);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    serde_json::from_str(&result.stdout).unwrap()
}

#[test]
fn past_run_metrics_are_computed_without_writing_or_inventing_review_counts() {
    let scenario = Scenario::new();
    let path = write(&scenario, "7-fixture.command.json", &record(7));
    let original = fs::read(&path).unwrap();
    let result = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    let run = &result["command"]["metrics"];
    assert_eq!(run["outcome"], "ready for review");
    assert_eq!(run["repairs"], 0);
    assert_eq!(run["ci_rounds"], 1);
    assert_eq!(run["total_minutes"], 30.0);
    assert_eq!(run["stages"][0]["minutes"], 20.0);
    assert_eq!(run["stages"][0]["review_findings"], Value::Null);
    assert_eq!(run["stages"][0]["usual_range"], Value::Null);
    assert_eq!(fs::read(&path).unwrap(), original);
    let text = scenario.run(&["metrics", "widgets#7"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout.contains("review_findings: missing"),
        "{}",
        text.stdout
    );
    assert!(scenario.gh_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
}

#[test]
fn weekly_trends_keep_signals_and_setup_groups_separate_and_filters_narrow_weeks() {
    let scenario = Scenario::new();
    write(&scenario, "7-fixture.command.json", &record(7));
    let mut repaired = record(8);
    repaired["stages"][1]["name"] = "Repair".into();
    write(&scenario, "8-fixture.command.json", &repaired);
    for (number, field, value) in [
        (9, "repository", "acme/other"),
        (10, "harness", "claude"),
        (11, "model", "opus"),
        (12, "effort", "max"),
        (13, "kind", "Base fix"),
        (14, "started_at", "2026-10-12T12:00:00Z"),
    ] {
        let mut other = record(number);
        other[field] = value.into();
        write(&scenario, &format!("{number}-fixture.command.json"), &other);
    }
    let all = metrics(&scenario, &["metrics", "--json"]);
    let rows = all["weeks"].as_array().unwrap();
    let run_rows: Vec<_> = rows
        .iter()
        .filter(|row| row["session_kind"].is_null())
        .collect();
    assert_eq!(run_rows.len(), 7, "{all}");
    let group = run_rows.iter().find(|row| row["runs"] == 2).unwrap();
    assert_eq!(group["week"], "2026-10-19"); // Sunday locally, Monday UTC.
    assert_eq!(
        group["repairs"],
        json!({"observed": 2, "missing": 0, "sum": 1.0, "mean": 0.5})
    );
    assert_eq!(group["ci_rounds"]["sum"], 1.0);
    assert_eq!(group["total_minutes"]["sum"], 60.0);
    let session = rows
        .iter()
        .find(|row| row["session_kind"] == "implement" && row["runs"] == 2)
        .unwrap();
    assert_eq!(session["stages"]["implement"]["minutes"]["mean"], 20.0);
    assert_eq!(
        session["stages"]["implement"]["review_findings"]["standards"],
        json!({
            "observed": 0, "missing": 2, "sum": null, "mean": null
        })
    );
    assert!(
        session["repairs"]["sum"].is_null(),
        "Run totals must not be duplicated in session rows"
    );
    let narrowed = metrics(
        &scenario,
        &[
            "metrics",
            "--from",
            "2026-10-19",
            "--through",
            "2026-10-19",
            "--repo",
            "acme/widgets",
            "--json",
        ],
    );
    assert!(
        narrowed["weeks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["repository"] == "acme/widgets" && row["week"] == "2026-10-19")
    );
    let two_repos = metrics(
        &scenario,
        &[
            "metrics",
            "--repo",
            "acme/widgets",
            "--repo",
            "acme/other",
            "--json",
        ],
    );
    assert_eq!(two_repos, all);
    let empty = metrics(&scenario, &["metrics", "--from", "2026-10-26", "--json"]);
    assert_eq!(empty["weeks"], json!([]));
    let text = scenario.run(&["metrics"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(text.stdout.contains("ci_rounds:"));
    assert!(text.stdout.contains("review_findings:"));
    assert!(scenario.gh_calls().is_empty());
}

#[test]
fn a_spec_includes_ticket_runs_and_base_fixes_once_and_retains_saved_metrics() {
    let scenario = Scenario::new();
    let mut spec = record(20);
    spec["kind"] = "Spec run".into();
    let spec_path = write(&scenario, "20-fixture.command.json", &spec);
    let mut ticket = record(21);
    ticket["parent_record"] = json!(spec_path);
    ticket["run_metrics"] = json!({
        "outcome": "merged", "repairs": 3, "ci_rounds": 4, "total_minutes": 42.0,
        "unaddressed_findings": null,
        "stages": [{"name": "implement", "seconds": 120.0, "minutes": 2.0,
            "usual_range": {"lower_seconds": 60.0, "upper_seconds": 180.0},
            "position": "within", "review_findings": {"standards": 2, "spec": 0}}]
    });
    let ticket_path = write(&scenario, "20-fixture.issue-21.child.json", &ticket);
    let mut base_fix = record(22);
    base_fix["kind"] = "Base fix".into();
    base_fix["parent_record"] = json!(ticket_path);
    write(&scenario, "20-fixture.issue-22.child.json", &base_fix);
    let result = metrics(&scenario, &["metrics", "widgets#20", "--json"]);
    assert_eq!(result["command"]["children"].as_array().unwrap().len(), 1);
    let ticket_view = &result["command"]["children"][0];
    ticket["run_metrics"]["session_signals"] = json!({
        "implement": {"observed_sessions":0, "missing_sessions":1, "signals":null}
    });
    ticket["run_metrics"]["stages"][0]["session_signals"] = Value::Null;
    assert_eq!(ticket_view["metrics"], ticket["run_metrics"]);
    assert_eq!(ticket_view["children"][0]["kind"], "Base fix");
    let selected = metrics(&scenario, &["metrics", "widgets#21", "--json"]);
    assert_eq!(selected["command"], *ticket_view);
    let trends = metrics(&scenario, &["metrics", "--json"]);
    let rows = trends["weeks"].as_array().unwrap();
    let count: u64 = rows
        .iter()
        .filter(|row| row["session_kind"].is_null())
        .map(|row| row["runs"].as_u64().unwrap())
        .sum();
    assert_eq!(count, 3, "{trends}");
    let session = rows
        .iter()
        .find(|row| row["kind"] == "Run" && row["session_kind"] == "implement")
        .unwrap();
    assert_eq!(
        session["stages"]["implement"]["review_findings"]["standards"]["sum"],
        2.0
    );
    assert_eq!(
        session["stages"]["implement"]["review_findings"]["spec"]["sum"],
        0.0
    );
    assert_eq!(session["stages"]["implement"]["positions"]["within"], 1);
}

#[test]
fn absent_completion_times_and_live_metrics_stay_missing() {
    let scenario = Scenario::new();
    let mut past = record(7);
    past.as_object_mut().unwrap().remove("ended_at");
    write(&scenario, "7-fixture.command.json", &past);
    let mut live = record(8);
    live["outcome"] = Value::Null;
    live["ended_at"] = Value::Null;
    write(&scenario, "8-fixture.command.json", &live);
    let past = metrics(&scenario, &["metrics", "widgets#7", "--json"]);
    assert_eq!(past["command"]["metrics"]["total_minutes"], Value::Null);
    assert_eq!(
        past["command"]["metrics"]["stages"][1]["minutes"],
        Value::Null
    );
    let live = metrics(&scenario, &["metrics", "widgets#8", "--json"]);
    assert_eq!(live["command"]["metrics"], Value::Null);
    let trends = metrics(&scenario, &["metrics", "--json"]);
    let row = trends["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_kind"].is_null())
        .unwrap();
    assert_eq!(
        row["total_minutes"],
        json!({"observed": 0, "missing": 2, "sum": null, "mean": null})
    );
    assert_eq!(
        row["repairs"],
        json!({"observed": 1, "missing": 1, "sum": 0.0, "mean": 0.0})
    );
    assert_eq!(row["missing_outcomes"], 1);
}

#[test]
fn an_unrecorded_security_setup_stays_missing_instead_of_copying_the_commands() {
    let scenario = Scenario::new();
    let mut past = record(7);
    past["stages"][0]["name"] = "Security review".into();
    write(&scenario, "7-fixture.command.json", &past);
    let trends = metrics(&scenario, &["metrics", "--json"]);
    let review = trends["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_kind"] == "Security review")
        .unwrap();
    assert_eq!(review["harness"], Value::Null);
    assert_eq!(review["model"], Value::Null);
    assert_eq!(review["effort"], Value::Null);
}

#[test]
fn metrics_rejects_invalid_filters_and_explains_log_only_history() {
    let scenario = Scenario::new();
    for args in [
        vec!["metrics", "--from"],
        vec!["metrics", "--from", "yesterday"],
        vec!["metrics", "--from", "2026-10-20"],
        vec!["metrics", "--from", "2026-10-26", "--through", "2026-10-19"],
        vec!["metrics", "--from", "2026-10-19", "--from", "2026-10-19"],
        vec!["metrics", "--repo"],
        vec!["metrics", "--repo", "widgets"],
        vec!["metrics", "--repo", "acme/widgets/extra"],
        vec!["metrics", "widgets#7", "--repo", "acme/widgets"],
        vec!["metrics", "--json", "--json"],
        vec!["metrics", "--unknown"],
        vec!["metrics", "widgets#7", "widgets#8"],
    ] {
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(2), "{args:?}: {}", result.stderr);
    }
    fs::create_dir_all(scenario.path(LOGS)).unwrap();
    support::session_trace::write(
        scenario
            .path(LOGS)
            .join("7-20261019T120000+0000-implement.logs.jsonl"),
        "{}\n",
    );
    let result = scenario.run(&["metrics", "widgets#7", "--json"]);
    assert_ne!(result.code, Some(0));
    assert!(
        result.stderr.contains("metrics need a Command record"),
        "{}",
        result.stderr
    );
    let empty = metrics(&scenario, &["metrics", "--json"]);
    assert_eq!(empty["weeks"], json!([]));
}
