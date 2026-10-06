//! Muse sessions through the scenario harness.
mod support;

use support::Scenario;

const AGENT_OPENS_PR: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn muse_runs_unattended_with_linked_skills_and_reports_usage_from_its_log() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let url = scenario.issue_url(7);
    let result = scenario.run(&["harness", "muse", &url]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.muse_calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call["argv"][0], "exec");
    assert_eq!(call["argv"][1], "--json");
    assert_eq!(call["argv"][2], "--yolo");
    assert_eq!(call["stdin_null"], true);
    assert_eq!(call["no_auto_update"], "1");
    assert_eq!(call["git_status"], "");
    assert_eq!(
        call["prompt"].as_str().unwrap().lines().next().unwrap(),
        format!("Load thirdshift-implement with your skill tool. {url}")
    );
    scenario.assert_every_muse_session_found_the_factory_skills();
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
    assert!(
        result
            .stderr
            .contains("1200 input tokens (200 cached), 300 output tokens"),
        "{}",
        result.stderr
    );
    assert!(!result.stderr.contains("warning:"), "{}", result.stderr);
}

const CATALOG: &str = r#"{"rows":[
  {"model_id":"muse-spark-1.3","display_label":"Muse Spark 1.3","visibility":"visible"},
  {"model_id":"muse-spark-1.3-contributor","display_label":"Contributor","visibility":"visible","is_default":true}
]}"#;

fn cache_catalog(scenario: &Scenario) {
    let dir = scenario.path("home/.local/share/muse/model-catalog");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("meta.json"), CATALOG).unwrap();
}

fn assert_no_work(scenario: &Scenario, result: &support::RunResult, error: &str) {
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains(error), "{}", result.stderr);
    assert!(scenario.muse_calls().is_empty());
    assert!(scenario.issue_labels(7).is_empty());
    assert_eq!(scenario.entries("work"), vec!["widgets"]);
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands")
            .exists()
    );
}

#[test]
fn an_unknown_model_in_muses_cached_catalog_fails_before_any_call_or_work() {
    let scenario = Scenario::new();
    cache_catalog(&scenario);
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[
        "harness",
        "muse",
        "model",
        "bad-model",
        &scenario.issue_url(7),
    ]);
    assert_no_work(
        &scenario,
        &result,
        "the Model bad-model is not in Muse's cached catalog",
    );
    assert!(!scenario.path("muse-calls.checks.json").exists());
}

#[test]
fn without_a_catalog_muse_checks_the_model_with_an_unattended_minimal_call() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[
        "harness",
        "muse",
        "model",
        "bad-model",
        &scenario.issue_url(7),
    ]);
    assert_no_work(
        &scenario,
        &result,
        "model does not exist or you lack access",
    );
    let checks: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(scenario.path("muse-calls.checks.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        checks[0]["argv"],
        serde_json::json!([
            "exec",
            "--json",
            "--yolo",
            "--model",
            "bad-model",
            "Reply with OK."
        ])
    );
    assert_eq!(checks[0]["stdin_null"], true);
    assert_eq!(checks[0]["no_auto_update"], "1");
}

#[test]
fn invalid_effort_fails_locally_even_without_a_catalog() {
    let scenario = Scenario::new();
    let result = scenario.run(&[
        "harness",
        "muse",
        "model",
        "bad-model",
        "effort",
        "bogus",
        &scenario.issue_url(7),
    ]);
    assert_no_work(
        &scenario,
        &result,
        "the Effort bogus is not supported by Muse",
    );
    assert!(!scenario.path("muse-calls.checks.json").exists());
}

#[test]
fn the_user_config_selects_muse_and_its_cached_model_and_effort_are_settled() {
    let scenario = Scenario::new();
    cache_catalog(&scenario);
    scenario.user_config_is("[harness]\ndefault = \"muse\"\n[harness.muse]\nmodel = \"Muse Spark 1.3\"\neffort = \"Max\"\n");
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.muse_calls();
    assert_eq!(
        calls[0]["argv"].as_array().unwrap()[..7],
        serde_json::json!([
            "exec",
            "--json",
            "--yolo",
            "--model",
            "muse-spark-1.3",
            "--reasoning-effort",
            "max"
        ])
        .as_array()
        .unwrap()[..]
    );
    assert!(!scenario.path("muse-calls.checks.json").exists());
    assert!(
        scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Built with muse · muse-spark-1.3 · max")
    );
}

#[test]
fn missing_log_uses_stream_text_without_usage_and_a_missing_skill_warns_once_in_both_logs() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run_with_env(
        &["harness", "muse", &scenario.issue_url(7)],
        &[("FAKE_MUSE_NO_LOG", "1"), ("FAKE_MUSE_SKIP_SKILL", "1")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.muse_calls().len(), 1);
    let warning = "warning: the session never loaded thirdshift-implement with its skill tool";
    assert_eq!(result.stderr.matches(warning).count(), 1);
    assert!(!result.stderr.contains("input tokens"));
    let dir = "home/.thirdshift/logs/acme/widgets/commands/issue";
    let name = scenario.entries(dir).pop().unwrap();
    let log = std::fs::read_to_string(scenario.path(&format!("{dir}/{name}"))).unwrap();
    assert_eq!(log.matches(warning).count(), 1);
}

#[test]
fn a_failed_turn_or_nonzero_exit_fails_with_muses_error() {
    for script in [
        "echo 'catalog unavailable' > \"$FAKE_MUSE_ERROR\"\nexit 1",
        r#"echo '{"stream":{"kind":"session","id":"fake-muse-1"},"payload_type":"run.terminal.failed","payload":{"reason":"catalog unavailable"}}'"#,
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_for(7, script);
        let result = scenario.run(&["harness", "muse", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("catalog unavailable"),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.muse_calls().len(), 1);
    }
}

#[test]
fn muse_resumes_the_stream_session_id_with_every_flag_and_environment_again() {
    let scenario = Scenario::new();
    cache_catalog(&scenario);
    scenario.agent_does_for_in_session(7, 1, &format!(r#"{AGENT_OPENS_PR}
echo '{{"stream":{{"kind":"session","id":"fake-muse-1"}},"payload_type":"task.lifecycle.proposed","payload":{{"event":{{"task_id":"task-1","task_kind":"tool.bash"}}}}}}'
echo '{{"stream":{{"kind":"session","id":"fake-muse-1"}},"payload_type":"task.lifecycle.cancelled","payload":{{"event":{{"task_id":"task-1","reason":"session ended"}}}}}}'
"#));
    let result = scenario.run(&[
        "harness",
        "muse",
        "model",
        "muse-spark-1.3",
        "effort",
        "high",
        &scenario.issue_url(7),
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.muse_calls();
    assert_eq!(calls.len(), 2, "{}", result.stderr);
    let args = calls[1]["argv"].as_array().unwrap();
    assert_eq!(
        args[..9],
        serde_json::json!([
            "exec",
            "--json",
            "--yolo",
            "--model",
            "muse-spark-1.3",
            "--reasoning-effort",
            "high",
            "--session-id",
            "fake-muse-1"
        ])
        .as_array()
        .unwrap()[..]
    );
    assert_eq!(calls[1]["stdin_null"], true);
    assert_eq!(calls[1]["no_auto_update"], "1");
}

#[test]
fn architecture_reviews_receive_only_the_last_reply_with_or_without_the_session_log() {
    for log_setting in [
        None,
        Some("FAKE_MUSE_NO_LOG"),
        Some("FAKE_MUSE_CORRUPT_LOG"),
    ] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["needs-triage", "ready-for-agent", "architecture"]);
        scenario.agent_does_in_session(
            1,
            r#"
url=$(gh issue create --title "An idea" --body "The idea" --label needs-triage)
printf 'Architecture review idea: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        );
        let env = log_setting
            .map(|setting| vec![(setting, "1")])
            .unwrap_or_default();
        let result = scenario.run_with_env(&["architect", "harness", "muse"], &env);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            result.stdout.trim(),
            "https://github.com/acme/widgets/issues/8"
        );
        assert_eq!(
            result.stderr.contains("1200 input tokens"),
            log_setting.is_none()
        );
    }
}

#[test]
fn setup_checks_muses_proposed_model_with_the_same_flags_and_environment() {
    let scenario = Scenario::new();
    let result = scenario.run_on_terminal(
        &["setup"],
        &[],
        &[
            ("Harness for every Run", "muse"),
            ("Model for muse", ""),
            ("Effort for muse", "low"),
            ("Every Run a Merge run", ""),
            ("Every Run first fast-forwards", ""),
            ("Run notifications", ""),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let config: toml::Table = result.user_config.unwrap().parse().unwrap();
    assert_eq!(
        config["harness"]["muse"]["model"].as_str(),
        Some("muse-spark-1.3")
    );
    let checks: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(scenario.path("muse-calls.checks.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        checks[0]["argv"],
        serde_json::json!([
            "exec",
            "--json",
            "--yolo",
            "--model",
            "muse-spark-1.3",
            "--reasoning-effort",
            "low",
            "Reply with OK."
        ])
    );
    assert_eq!(checks[0]["stdin_null"], true);
    assert_eq!(checks[0]["no_auto_update"], "1");
}
