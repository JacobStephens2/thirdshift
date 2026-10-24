use super::*;
use support::TerminalStep;

#[test]
fn historical_draw_handles_more_than_a_thousand_past_pull_requests() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = (1..=1002)
        .map(|n| {
            let mut candidate = pr(n, "acme/widgets", "codex", "gpt", false);
            candidate["mergedAt"] = (Utc::now()
                - chrono::TimeDelta::days(10 + i64::try_from(n).unwrap()))
            .to_rfc3339()
            .into();
            candidate
        })
        .collect();
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(weeks(&scenario)[0]["sample"].as_array().unwrap().len(), 20);
}

#[test]
fn first_run_marks_twenty_historical_prs_across_models_then_two_per_week() {
    let scenario = scenario();
    let old = (Utc::now() - chrono::TimeDelta::days(90)).to_rfc3339();
    let mut candidates: Vec<_> = (1..=24)
        .map(|n| {
            let (harness, model) = if n <= 12 {
                ("claude", "opus")
            } else {
                ("codex", "gpt")
            };
            let mut candidate = pr(n, "acme/widgets", harness, model, false);
            candidate["mergedAt"] = old.clone().into();
            candidate
        })
        .collect();
    candidates.push(pr(25, "acme/widgets", "codex", "gpt", true));
    let mut state = scenario.gh_state();
    state["scoring_prs"] = candidates.into();
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let first = weeks(&scenario).remove(0);
    let marked: Vec<_> = first["sample"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pr| pr["for_you"] == true)
        .collect();
    assert_eq!(marked.len(), 20, "{first}");
    assert_eq!(
        marked.iter().filter(|pr| pr["harness"] == "claude").count(),
        10
    );
    assert_eq!(
        marked.iter().filter(|pr| pr["harness"] == "codex").count(),
        10
    );
    assert!(
        marked
            .iter()
            .all(|pr| pr["assessment"]["scores"].as_array().unwrap().len() == 5)
    );
    assert!(
        !first["sample"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pr| pr["number"] == 25)
    );
    // Force a week to elapse at the retained-clock seam, then add new work.
    let dir = scenario.path("home/.thirdshift/evals/scoring");
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let mut saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    saved["window_end"] = (Utc::now() - chrono::TimeDelta::days(7))
        .to_rfc3339()
        .into();
    saved["week"] = (Utc::now() - chrono::TimeDelta::days(7))
        .format("%G-W%V")
        .to_string()
        .into();
    std::fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
    state["scoring_prs"] = (30..=34)
        .map(|n| pr(n, "acme/widgets", "claude", "opus", false))
        .collect();
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let retained = weeks(&scenario);
    assert_eq!(retained.len(), 2);
    assert_eq!(retained[1]["sample"].as_array().unwrap().len(), 3);
    assert_eq!(
        retained[1]["sample"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|pr| pr["for_you"] == true)
            .count(),
        2
    );
}

#[test]
fn mine_requires_all_five_scores_before_revealing_the_model() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "codex", "gpt", false)]);
    scenario.write_gh_state(&state);
    assert_eq!(scenario.run(&["score"]).code, Some(0));
    let url = "https://github.com/acme/widgets/pull/1";
    let partial = scenario.run_terminal(
        &["scores", url, "--mine"],
        &[],
        &[
            TerminalStep::line("Correct (pass/fail):", "pass"),
            TerminalStep::line("Correct reason:", "My assessment"),
            TerminalStep::bytes("Scoped (pass/fail):", b"\x04"),
        ],
    );
    assert_eq!(partial.code, Some(1), "{}", partial.stderr);
    assert!(!partial.stdout.contains("Implements the requested feature."));
    assert!(weeks(&scenario)[0]["sample"][0]["mine"].is_null());
    let complete = scenario.run_on_terminal(
        &["scores", url, "--mine"],
        &[],
        &[
            ("Correct (pass/fail):", "maybe\npass"),
            ("Correct reason:", "\nMy assessment"),
            ("Scoped (pass/fail):", "pass"),
            ("Scoped reason:", "Within scope"),
            ("Tested (pass/fail):", "fail"),
            ("Tested reason:", "Missing coverage"),
            ("Honest (pass/fail):", "pass"),
            ("Honest reason:", "Accurate claims"),
            ("Standards (pass/fail):", "pass"),
            ("Standards reason:", "Clear code"),
        ],
    );
    assert_eq!(complete.code, Some(0), "{}", complete.stderr);
    assert!(
        complete
            .stdout
            .contains("Implements the requested feature.")
    );
    assert!(complete.stdout.contains("My assessment"));
    let shown = scenario.run(&["scores", url, "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(shown[0]["sample"].as_array().unwrap().len(), 1);
    assert_eq!(
        shown[0]["sample"][0]["mine"]["scores"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn calibration_requires_twenty_pairs_tracks_eighteen_matches_and_rolls_independently() {
    let scenario = scenario();
    scenario.user_config_is("[scoring]\nowners = ['acme']\nsample_size = 30\nmodel = 'grader'\n");
    let mut state = scenario.gh_state();
    state["scoring_prs"] = (1..=21)
        .map(|n| pr(n, "acme/widgets", "codex", "gpt", false))
        .collect();
    scenario.write_gh_state(&state);
    assert_eq!(scenario.run(&["score"]).code, Some(0));
    for n in 1..=21 {
        let correct = if [2, 3, 21].contains(&n) {
            "fail"
        } else {
            "pass"
        };
        let answers = format!(
            "{correct}\nMy Correct reason\nfail\nMy Scoped reason\nfail\nMy Tested reason\npass\nMy Honest reason\npass\nMy Standards reason"
        );
        let run = scenario.run_on_terminal(
            &["scores", &n.to_string(), "--mine"],
            &[],
            &[("Correct (pass/fail):", &answers)],
        );
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        if n >= 19 {
            let shown = scenario.run(&["scores", &n.to_string(), "--json"]);
            let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
            let scores = shown[0]["sample"][0]["assessment"]["scores"]
                .as_array()
                .unwrap();
            let correct = &scores
                .iter()
                .find(|score| score["criterion"] == "Correct")
                .unwrap()["calibration"];
            assert_eq!(correct["compared"], n.min(20));
            assert_eq!(correct["matches"], if n == 20 { 18 } else { 17 });
            assert_eq!(correct["calibrated"], n == 20);
            for criterion in ["Tested", "Honest", "Standards"] {
                let state = &scores
                    .iter()
                    .find(|score| score["criterion"] == criterion)
                    .unwrap()["calibration"];
                assert_eq!(state["calibrated"], n >= 20, "{criterion}: {state}");
            }
            let scoped = &scores
                .iter()
                .find(|score| score["criterion"] == "Scoped")
                .unwrap()["calibration"];
            assert_eq!(scoped["matches"], 0);
            assert_eq!(scoped["calibrated"], false);
        }
    }
    let text = scenario.run(&["scores"]);
    assert!(text.stdout.contains("uncalibrated: 17/20"));
    assert!(text.stdout.contains("calibrated: 20/20"));
}
