use super::*;

fn reprice(fixture: &Fixture, cost: f64, time: u64) {
    let view = fixture.experiment();
    let root = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        view["id"].as_str().unwrap()
    ));
    for entry in fs::read_dir(root.join("results")).unwrap() {
        let path = entry.unwrap().path();
        let mut result: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let candidate = result["arm"] == "candidate";
        result["attempts"][0]["accounting"]["sessions"][0]["cost_usd"] =
            json!(if candidate { cost } else { 1.0 });
        result["attempts"][0]["timings"]["total_ms"] = json!(if candidate { time } else { 1000 });
        fs::write(path, result.to_string()).unwrap();
    }
    let metadata = root.join("experiment.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    saved["state"] = json!("interrupted");
    saved["verdict"] = Value::Null;
    fs::write(metadata, saved.to_string()).unwrap();
}

#[test]
fn a_cost_ceiling_is_fixed_before_trials_and_fails_a_three_times_candidate() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let run = fixture.run(&["--cost-ceiling", "2", "--margin", "-1"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let view = fixture.experiment();
    assert_eq!(view["bar"]["cost_ceiling"], 2.0);
    // A stopped Experiment resumes from its retained Trial evidence.
    reprice(&fixture, 3.0, 1000);
    let resumed = fixture.run(&["--cost-ceiling", "2", "--margin", "-1"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let failed = fixture.experiment();
    assert_eq!(failed["verdict"]["verdict"], "fail", "{failed}");
    assert_eq!(failed["verdict"]["efficiency"]["quality_verdict"], "pass");
    assert!(
        failed["verdict"]["line"]
            .as_str()
            .unwrap()
            .contains("cost ceiling")
    );
    let changed = fixture.run(&["--cost-ceiling", "4", "--margin", "-1"]);
    assert_eq!(changed.code, Some(1));
    assert!(changed.stderr.contains("Bar"), "{}", changed.stderr);
}

#[test]
fn no_ceiling_preserves_quality_and_a_time_ceiling_uses_paired_time() {
    for (extra, expected, reason) in [
        (vec![], "pass", "costs more"),
        (vec!["--time-ceiling", "2"], "fail", "time ceiling"),
        (
            vec!["--cost-ceiling", "3", "--time-ceiling", "3"],
            "pass",
            "takes longer",
        ),
    ] {
        let fixture = Fixture::new();
        fixture.scenario.agent_does(review_reports_prelude());
        let args: Vec<_> = [vec!["--margin", "-1"], extra].concat();
        assert_eq!(fixture.run(&args).code, Some(0));
        let quality = fixture.experiment()["verdict"]["suites"].clone();
        reprice(&fixture, 3.0, 3000);
        let resumed = fixture.run(&args);
        assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
        let view = fixture.experiment();
        assert_eq!(view["verdict"]["suites"], quality);
        assert_eq!(view["verdict"]["verdict"], expected, "{view}");
        assert!(view["verdict"]["line"].as_str().unwrap().contains(reason));
    }
}

#[test]
fn a_credibly_cheaper_candidate_has_the_same_json_and_text_verdict_line() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let tasks = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks");
    fs::create_dir(tasks.join("second")).unwrap();
    for entry in fs::read_dir(tasks.join("fixture")).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), tasks.join("second").join(entry.file_name())).unwrap();
    }
    let args = ["--margin", "-1"];
    assert_eq!(fixture.run(&args).code, Some(0));
    reprice(&fixture, 0.5, 1000);
    let resumed = fixture.run(&args);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let view = fixture.experiment();
    assert_eq!(view["verdict"]["line"], "pass: no worse, and cheaper");
    let text = fixture
        .scenario
        .run(&["experiments", view["id"].as_str().unwrap()]);
    assert_eq!(text.code, Some(0));
    assert!(
        text.stdout.contains("pass: no worse, and cheaper"),
        "{}",
        text.stdout
    );
    assert_eq!(
        view["verdict"]["efficiency"]["cost"]["interval"],
        json!({"lower":0.5,"upper":0.5})
    );
}

#[test]
fn ceilings_reject_nonpositive_and_nonfinite_ratios_before_trials() {
    let fixture = Fixture::new();
    for flag in ["--cost-ceiling", "--time-ceiling"] {
        for value in ["0", "-1", "NaN", "inf"] {
            let run = fixture.run(&[flag, value]);
            assert_eq!(run.code, Some(2), "{}", run.stderr);
            assert!(run.stderr.contains("positive finite"), "{}", run.stderr);
        }
    }
    assert!(fixture.trial_calls().is_empty());
}
