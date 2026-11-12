use super::*;

#[test]
fn time_comparisons_use_the_remaining_trials_after_a_suspension() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let tasks = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks");
    for name in ["second", "third"] {
        fs::create_dir(tasks.join(name)).unwrap();
        for entry in fs::read_dir(tasks.join("fixture")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), tasks.join(name).join(entry.file_name())).unwrap();
        }
    }
    let args = ["--margin", "-1", "--time-ceiling", "2"];
    let run = fixture.run(&args);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view = fixture.experiment();
    let root = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        view["id"].as_str().unwrap()
    ));
    for entry in fs::read_dir(root.join("results")).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let candidate = result["arm"] == "candidate";
        result["attempts"][0]["timings"]["total_ms"] = json!(if candidate { 500 } else { 1000 });
        if candidate && result["task"] == "fixture" {
            result["attempts"][0]["timings"]["suspended_seconds"] = json!(120);
        }
        fs::write(path, result.to_string()).unwrap();
    }
    let metadata = root.join("experiment.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    saved["state"] = json!("interrupted");
    saved["verdict"] = Value::Null;
    fs::write(metadata, saved.to_string()).unwrap();
    let resumed = fixture.run(&args);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let shown: Value = serde_json::from_str(&resumed.stdout).unwrap();
    let time = &shown["verdict"]["efficiency"]["time"];
    assert_eq!(time["pairs"], 2);
    assert_eq!(time["ratio"], 0.5);
    assert_eq!(time["complete"], true, "{time}");
    assert_eq!(time["interval"], json!({"lower": 0.5, "upper": 0.5}));
    assert_eq!(shown["verdict"]["verdict"], "pass", "{shown}");
}

#[cfg(target_os = "linux")]
#[test]
fn commands_mark_trials_that_span_their_recorded_suspension() {
    for process in [false, true] {
        let fixture = if process {
            process::fixture()
        } else {
            Fixture::new()
        };
        if !process {
            fixture.scenario.agent_does(review_reports_prelude());
        }
        let library = fixture.scenario.path("suspension-clock.so");
        let compiled = std::process::Command::new("cc")
            .args([
                "-shared",
                "-fPIC",
                "tests/console/session-clock.c",
                "-ldl",
                "-o",
            ])
            .arg(&library)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let offset = fixture.scenario.path("clock-offset");
        let script = fs::read_to_string(fixture.scenario.path("claude-script.sh")).unwrap();
        fixture
            .scenario
            .agent_does(&format!("printf '120' > '{}'\n{script}", offset.display()));
        let run = fixture.scenario.run_with_env(
            &Fixture::experiment_args(&[]),
            &[
                NO_SANDBOX[0],
                ("LD_PRELOAD", library.to_str().unwrap()),
                ("THIRDSHIFT_REVIEW_CLOCK_OFFSET", offset.to_str().unwrap()),
            ],
        );
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        assert!(run.stderr.contains("host was suspended"), "{}", run.stderr);
        let view: Value = serde_json::from_str(&run.stdout).unwrap();
        let suspended: Vec<_> = view["accounting"]["trials"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|trial| trial["suspended"] == true)
            .collect();
        assert!(!suspended.is_empty(), "{view}");
        for trial in suspended {
            assert!(trial["cost_usd"].as_f64().unwrap() > 0.0, "{trial}");
        }
        assert!(
            view["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|result| result["outcome"] == if process { "delivered" } else { "succeeded" }),
            "{view}"
        );
        assert_eq!(fixture.trial_calls().len(), if process { 4 } else { 2 });
        let shown = fixture.experiment();
        assert_eq!(shown["accounting"], view["accounting"]);
    }
}

#[test]
fn suspended_trials_keep_outcomes_and_cost_but_leave_time_out_of_reports() {
    let fixture = Fixture::with_session("spec-review", 3);
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&["--margin", "-1"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view = fixture.experiment();
    let text = fixture
        .scenario
        .run(&["experiments", view["id"].as_str().unwrap()]);
    assert!(
        !text.stdout.contains("left out of timing"),
        "{}",
        text.stdout
    );
    let root = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        view["id"].as_str().unwrap()
    ));
    for entry in fs::read_dir(root.join("results")).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let candidate = result["arm"] == "candidate";
        let suspended = candidate && result["trial"] == 0;
        let time = if suspended {
            9_000_000
        } else if candidate && result["trial"] == 2 {
            6000
        } else if candidate {
            2000
        } else {
            1000
        };
        let attempt = &mut result["attempts"][0];
        attempt["timings"]["total_ms"] = json!(time);
        attempt["timings"]["suspended_seconds"] = json!(if suspended { 120 } else { 0 });
        attempt["accounting"]["sessions"][0]["time_ms"] = attempt["timings"]["total_ms"].clone();
        attempt["accounting"]["sessions"][0]["cost_usd"] = json!(if candidate { 3.0 } else { 1.0 });
        fs::write(&path, result.to_string()).unwrap();
        // Pending history supplies Arm minutes, terminal files supply ratios.
        let pending = root
            .join("trials")
            .join(path.file_stem().unwrap())
            .join("pending.json");
        fs::write(pending, result.to_string()).unwrap();
    }
    let shown = fixture.experiment();
    assert_eq!(shown["accounting"]["paired_median_ratios"]["time"], 4.0);
    assert_eq!(shown["accounting"]["paired_median_ratios"]["time_pairs"], 2);
    assert_eq!(shown["accounting"]["paired_median_ratios"]["cost"], 3.0);
    assert_eq!(shown["accounting"]["candidate"]["cost_usd"], 9.0);
    assert_eq!(shown["accounting"]["candidate"]["time_ms"], 8000);
    assert_eq!(
        shown["accounting"]["candidate"]["roles"]["spec-review"]["time_ms"],
        8000
    );
    assert_eq!(
        shown["accounting"]["candidate"]["timing_excluded_trials"],
        1
    );
    assert_eq!(shown["minutes"]["candidate"], 8000.0 / 60_000.0);
    let suspended = shown["accounting"]["trials"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|trial| trial["suspended"] == true)
        .count();
    assert_eq!(suspended, 1);
    assert_eq!(shown["results"].as_array().unwrap().len(), 6);
    assert!(
        shown["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["outcome"] == "succeeded")
    );
    let text = fixture
        .scenario
        .run(&["experiments", shown["id"].as_str().unwrap()]);
    assert!(
        text.stdout.contains("1 Trial left out of timing"),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains("suspended"), "{}", text.stdout);
    let calls = fixture.trial_calls().len();
    assert_eq!(fixture.run(&["--margin", "-1"]).code, Some(0));
    assert_eq!(
        fixture.trial_calls().len(),
        calls,
        "suspension must not rerun a Trial"
    );
}
