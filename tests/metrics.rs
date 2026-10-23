mod support;

use std::fs;

use serde_json::{Value, json};
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";

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
