//! Scoring runs through the whole binary, with GitHub at the executable seam.
mod support;

use chrono::{DateTime, Utc};
use serde_json::Value;
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/JacobStephens2/thirdshift/commands/scoring";

fn scenario() -> Scenario {
    let scenario = Scenario::new();
    scenario.user_config_is("[scoring]\nowners = [\"acme\"]\nsample_size = 3\n");
    scenario
}

fn weeks(scenario: &Scenario) -> Vec<Value> {
    let result = scenario.run(&["scores", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    serde_json::from_str(&result.stdout).unwrap()
}

fn pr(number: u64, repo: &str, harness: &str, model: &str, security: bool) -> Value {
    let merged = Utc::now() - chrono::TimeDelta::days(1);
    serde_json::json!({
        "number": number,
        "url": format!("https://github.com/{repo}/pull/{number}"),
        "body": format!("Closes #7\n\nBuilt with {harness} · {model} · high <!-- thirdshift:built-with -->\n"),
        "mergedAt": merged.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "headRefName": "feature",
        "repository": {"nameWithOwner": repo},
        "closingIssuesReferences": {
            "pageInfo": {"hasNextPage": false},
            "nodes": [{"url": format!("https://github.com/{repo}/issues/7"),
                "labels": {"nodes": if security {serde_json::json!([{"name": "security-fix"}])} else {serde_json::json!([])},
                    "pageInfo": {"hasNextPage": false}}}]
        }
    })
}

#[test]
fn sample_spreads_across_harness_and_model_pairs_and_excludes_security_fixes() {
    let scenario = scenario();
    scenario
        .user_config_is("[scoring]\nowners = [\"acme\", \"other\", \"acme\"]\nsample_size = 3\n");
    let mut state = scenario.gh_state();
    let mut prs: Vec<Value> = (1..=12)
        .map(|n| pr(n, "acme/widgets", "claude", "opus", false))
        .collect();
    prs.push(pr(20, "acme/widgets", "claude", "sonnet", false));
    prs.push(pr(21, "other/repository", "codex 1.2.3", "gpt", false));
    prs.push(pr(22, "acme/widgets", "grok", "grok", true));
    prs.push(pr(23, "outsider/repository", "agy", "gemini", false));
    state["scoring_prs"] = prs.into();
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let weeks = weeks(&scenario);
    let sample = weeks[0]["sample"].as_array().unwrap();
    assert_eq!(sample.len(), 3, "{sample:?}");
    let pairs: std::collections::BTreeSet<_> = sample
        .iter()
        .map(|pr| {
            (
                pr["harness"].as_str().unwrap(),
                pr["model"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [("claude", "opus"), ("claude", "sonnet"), ("codex", "gpt")].into()
    );
    assert_eq!(
        sample.iter().find(|pr| pr["harness"] == "codex").unwrap()["harness_version"],
        "1.2.3"
    );
    assert!(
        sample
            .iter()
            .all(|pr| pr["number"] != 22 && pr["number"] != 23)
    );
    assert!(weeks[0]["seed"].as_u64().is_some());
    assert_eq!(weeks[0]["week"].as_str().unwrap().len(), 8);
}

#[test]
fn security_fix_on_an_issue_branch_is_excluded_without_a_closing_issue_link() {
    let scenario = scenario();
    scenario.issue_labelled(7, &["security-fix"]);
    let mut state = scenario.gh_state();
    let mut candidate = pr(10, "acme/widgets", "claude", "opus", false);
    candidate["headRefName"] = "issue-7".into();
    candidate["closingIssuesReferences"]["nodes"] = serde_json::json!([]);
    state["scoring_prs"] = serde_json::json!([candidate]);
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(weeks(&scenario)[0]["sample"], serde_json::json!([]));
}

#[test]
fn a_second_window_starts_at_the_previous_end_and_keeps_the_first_sample() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "claude", "opus", false)]);
    scenario.write_gh_state(&state);
    let first_run = scenario.run(&["score"]);
    assert_eq!(first_run.code, Some(0), "{}", first_run.stderr);
    let first = weeks(&scenario).remove(0);
    assert_eq!(first["sample"].as_array().unwrap().len(), 1);
    let second_run = scenario.run(&["score"]);
    assert_eq!(second_run.code, Some(0), "{}", second_run.stderr);
    let retained = weeks(&scenario);
    assert_eq!(retained.len(), 2);
    assert!(retained.contains(&first), "first sample was overwritten");
    let second = retained.iter().find(|week| **week != first).unwrap();
    assert_eq!(second["window_start"], first["window_end"]);
    assert_eq!(second["sample"], serde_json::json!([]));
    let text = scenario.run(&["scores"]);
    assert!(
        text.stdout
            .contains("https://github.com/acme/widgets/pull/1")
    );
    assert!(text.stdout.contains(first["week"].as_str().unwrap()));
}

#[test]
fn a_live_scoring_command_is_visible_stoppable_and_excludes_a_second_command() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["on_scoring_search"] = format!(
        "touch '{}'\nwhile true; do sleep 0.05; done",
        scenario.path("search-started").display()
    )
    .into();
    scenario.write_gh_state(&state);
    let mut held = scenario.run_until(&["score"], &[], "search-started");
    let status = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let command = &status["commands"][0];
    assert_eq!(command["kind"], "Scoring run");
    assert_eq!(command["state"], "live");
    let before = scenario.gh_calls().len();
    let skipped = scenario.run_from(&scenario.path("home"), &["score"]);
    assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    assert!(skipped.stderr.contains("another Scoring run"));
    assert_eq!(scenario.gh_calls().len(), before);
    assert_eq!(scenario.log_files(LOGS, "log").len(), 1);
    let stopped = scenario.run(&["stop", command["name"].as_str().unwrap(), "--json"]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    let stop: Value = serde_json::from_str(&stopped.stdout).unwrap();
    assert_eq!(stop["outcome"], "interrupted");
    assert!(weeks(&scenario).is_empty());
    assert_eq!(scenario.command_record(LOGS)["outcome"], "interrupted");
    let activity = std::fs::read_to_string(
        scenario.path("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(activity.contains("stopped from the CLI by"), "{activity}");
}

#[test]
fn a_failed_scoring_search_lands_in_needs_you_and_advances_no_window() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["failing"] = serde_json::json!(["api graphql"]);
    state["failing_stderr"] = "HTTP 401: Bad credentials".into();
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(weeks(&scenario).is_empty());
    assert_eq!(scenario.command_record(LOGS)["outcome"], "failed");
    let status = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "needs you");
    assert_eq!(status["commands"][0]["needs_you_kind"], "failed");
}

#[test]
fn search_reads_every_page_before_drawing_the_sample() {
    let scenario = scenario();
    scenario.user_config_is("[scoring]\nowners = [\"acme\"]\nsample_size = 200\n");
    let mut state = scenario.gh_state();
    state["scoring_prs"] = (1..=105)
        .map(|number| pr(number, "acme/widgets", "claude", "opus", false))
        .collect();
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let retained = weeks(&scenario);
    let sample = retained[0]["sample"].as_array().unwrap();
    assert_eq!(sample.len(), 105);
    assert!(sample.iter().any(|pr| pr["number"] == 105));
}

#[test]
fn incomplete_issue_labels_fail_without_advancing_the_window() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    let mut candidate = pr(10, "acme/widgets", "claude", "opus", false);
    candidate["closingIssuesReferences"]["nodes"][0]["labels"]["pageInfo"]["hasNextPage"] =
        true.into();
    state["scoring_prs"] = serde_json::json!([candidate]);
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("cannot verify all linked issue labels"),
        "{}",
        result.stderr
    );
    assert!(weeks(&scenario).is_empty());
}

#[test]
fn setup_completes_scoring_defaults_and_preserves_configured_choices() {
    let scenario = Scenario::new();
    scenario.user_config_is("[scoring]\nowners = [\"acme\"] # retain my owners\nsample_size = 4\nharness = 'codex'\nmodel = 'gpt'\nfallback_harness = 'claude'\nfallback_model = 'opus'\n");
    let result = scenario.run(&["setup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let text = std::fs::read_to_string(scenario.path("home/.thirdshift/config.toml")).unwrap();
    assert!(text.contains("# retain my owners"));
    let parsed: toml::Table = text.parse().unwrap();
    assert_eq!(parsed["scoring"]["sample_size"].as_integer(), Some(4));
    assert_eq!(parsed["scoring"]["harness"].as_str(), Some("codex"));
    assert_eq!(parsed["scoring"]["model"].as_str(), Some("gpt"));
    assert_eq!(
        parsed["scoring"]["fallback_harness"].as_str(),
        Some("claude")
    );
    assert_eq!(parsed["scoring"]["fallback_model"].as_str(), Some("opus"));
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let record = scenario.command_record(LOGS);
    assert_eq!(record["harness"], "codex");
    assert_eq!(record["model"], "gpt");
}

#[test]
fn first_window_looks_back_seven_days_and_is_a_recorded_command() {
    let scenario = scenario();
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let weeks = weeks(&scenario);
    assert_eq!(weeks.len(), 1);
    let start = DateTime::parse_from_rfc3339(weeks[0]["window_start"].as_str().unwrap()).unwrap();
    let end = DateTime::parse_from_rfc3339(weeks[0]["window_end"].as_str().unwrap()).unwrap();
    assert_eq!((end - start).num_seconds(), 7 * 24 * 60 * 60);
    assert!((Utc::now() - end.with_timezone(&Utc)).num_seconds() < 60);
    assert!(weeks[0]["seed"].as_u64().is_some());
    assert_eq!(weeks[0]["sample"], serde_json::json!([]));
    let record = scenario.command_record(LOGS);
    assert_eq!(record["kind"], "Scoring run");
    assert_eq!(record["outcome"], "sampled");
    assert_eq!(scenario.log_files(LOGS, "log").len(), 1);
    use std::os::unix::fs::PermissionsExt;
    let sample_dir = scenario.path("home/.thirdshift/evals/scoring");
    assert_eq!(
        std::fs::metadata(&sample_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for entry in std::fs::read_dir(sample_dir).unwrap() {
        assert_eq!(
            entry.unwrap().metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let activity = std::fs::read_to_string(
        scenario.path("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(activity.contains("Scoring run started:"), "{activity}");
    assert!(
        activity.contains("Scoring run ended: sampled 0 pull requests"),
        "{activity}"
    );
}
