mod support;

use std::fs;

use serde_json::{Value, json};
use support::Scenario;

fn past_run(scenario: &Scenario) {
    let folder = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("7-fixture.command.json"),
        serde_json::to_vec(&json!({
            "version":1,"kind":"Run","repository":"acme/widgets","name":"acme/widgets#7",
            "issue":{"number":7,"url":"https://github.com/acme/widgets/issues/7"},
            "hostname":"factory","process_id":0,"process_start":"gone",
            "harness":"codex","model":"sol","effort":"high",
            "started_at":"2026-10-19T03:30:00Z","ended_at":"2026-10-19T04:00:00Z",
            "stages":[{"name":"implement","began_at":"2026-10-19T03:30:00Z"}],
            "outcome":"ready for review","session_log":null,"cause":null,
            "issue_branch":null,"pull_request":"https://github.com/acme/widgets/pull/23"
        }))
        .unwrap(),
    )
    .unwrap();
}

fn report(scenario: &Scenario, command: &str) -> Value {
    let output = scenario.run(&[command, "widgets#7", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    serde_json::from_str(&output.stdout).unwrap()
}

#[test]
fn show_includes_metrics_reconstructed_for_a_past_run() {
    let scenario = Scenario::new();
    past_run(&scenario);
    let metrics = report(&scenario, "metrics");
    assert_eq!(metrics["command"]["metrics"]["total_minutes"], 30.0);
    let shown = report(&scenario, "show");
    assert_eq!(
        shown["command"]["run_metrics"]["total_minutes"], 30.0,
        "show must expose the same available historical Run metrics as metrics"
    );
}

#[test]
fn show_keeps_legacy_usual_ranges_without_inventing_historical_minutes() {
    let scenario = Scenario::new();
    past_run(&scenario);
    let folder = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    let mut legacy: Value =
        serde_json::from_slice(&fs::read(folder.join("7-fixture.command.json")).unwrap()).unwrap();
    legacy.as_object_mut().unwrap().remove("ended_at");
    let ended = chrono::DateTime::parse_from_rfc3339("2026-10-19T04:00:00Z").unwrap();
    let modified = std::time::UNIX_EPOCH
        + std::time::Duration::from_secs(ended.timestamp().try_into().unwrap());
    for number in 1..=3 {
        legacy["name"] = json!(format!("acme/widgets#{number}"));
        legacy["issue"]["number"] = json!(number);
        let path = folder.join(format!("{number}-legacy.command.json"));
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        fs::File::open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }
    let shown = report(&scenario, "show");
    assert_eq!(
        shown["command"]["usual_ranges"]["implement"],
        json!({"lower_seconds": 1800.0, "upper_seconds": 1800.0})
    );
    let output = scenario.run(&["metrics", "widgets#1", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let metrics: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(metrics["command"]["metrics"]["total_minutes"], Value::Null);
    assert_eq!(
        metrics["command"]["metrics"]["stages"][0]["minutes"],
        Value::Null
    );
}

#[test]
fn show_includes_confirmed_bug_origins_from_read_time_scoring_data() {
    let scenario = Scenario::new();
    scenario.agent_does("echo work > work.txt\ngit add work.txt\ngit commit -qm 'Make widgets'\ngh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'\n");
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let folder = scenario.path("home/.thirdshift/evals/scoring");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("week.json"), serde_json::to_vec(&json!({
        "window_end":"2026-10-26T00:00:00Z",
        "bug_origins":[{
            "issue":"https://github.com/acme/widgets/issues/99","state":"confirmed",
            "settlement":{"pull_request":{"url":"https://github.com/acme/widgets/pull/1","run":null}}
        }]
    })).unwrap()).unwrap();
    let metrics = report(&scenario, "metrics");
    assert_eq!(metrics["command"]["metrics"]["confirmed_bug_origins"], 1);
    let shown = report(&scenario, "show");
    assert_eq!(
        shown["command"]["run_metrics"]["confirmed_bug_origins"], 1,
        "show must include confirmed Bug origins added after Run completion"
    );
}
