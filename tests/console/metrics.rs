use super::*;

#[test]
fn peek_shows_retained_run_metrics_and_marks_missing_signals() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut value = record(home.path(), 7, Some("merged"), 0);
    value["kind"] = json!("Run");
    value["run_metrics"] = json!({
        "outcome": "merged", "repairs": 0, "ci_rounds": 2,
        "unaddressed_findings": {"standards": 1, "spec": 0, "security": null},
        "total_minutes": 12.5,
        "session_signals": null,
        "stages": [{
            "name": "implement", "seconds": 750.0, "minutes": 12.5,
            "usual_range": null, "position": "too_few_runs",
            "review_findings": {"standards": 3, "spec": null},
            "session_signals": null
        }]
    });
    write_record(home.path(), 7, &value);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let output = Command::new(binary)
        .args(["show", "widgets#7", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let shown: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(shown["command"]["run_metrics"], value["run_metrics"]);

    let response = console.get("/peek?command=widgets%237");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    let metrics = html.split_once("<dt>run metrics</dt>").unwrap().1;
    for expected in [
        "<dt>outcome</dt><dd>merged",
        "<dt>repairs</dt><dd>0",
        "<dt>ci rounds</dt><dd>2",
        "<dt>unaddressed findings</dt>",
        "<dt>standards</dt><dd>1",
        "<dt>spec</dt><dd>0",
        "<dt>security</dt><dd>Missing",
        "<dt>total minutes</dt><dd>12.5",
        "<dt>name</dt><dd>implement",
        "<dt>seconds</dt><dd>750.0",
        "<dt>minutes</dt><dd>12.5",
        "<dt>usual range</dt><dd>Missing",
        "<dt>position</dt><dd>too_few_runs",
        "<dt>review findings</dt>",
        "<dt>standards</dt><dd>3",
        "<dt>spec</dt><dd>Missing",
    ] {
        assert!(metrics.contains(expected), "missing {expected}: {metrics}");
    }
}
