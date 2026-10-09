//! Commands on Grok Build through the scenario harness.

mod support;

use support::Scenario;

const AGENT_OPENS_PR: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn a_run_can_choose_grok_build() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);

    let result = scenario.run(&["harness", "grok", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(result.stderr.contains("sessions run on grok"));
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
}

#[test]
fn an_invalid_model_or_effort_fails_before_any_work_and_names_the_valid_choices() {
    for (model, effort, error) in [
        (
            "grok-9.9-bogus",
            "high",
            "the Model grok-9.9-bogus is not in Grok Build's catalog: choose one of grok-4.7, grok-4.7-build-fast, grok-4.6, grok-4.5",
        ),
        (
            "grok-4.5",
            "xhigh",
            "the Effort xhigh is not one the Grok Build Model grok-4.5 supports: choose one of high, medium, low",
        ),
        (
            "grok-4.7",
            "max",
            "the Effort max is not one the Grok Build Model grok-4.7 supports: choose one of xhigh, high, medium, low",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_for(7, AGENT_OPENS_PR);
        let result = scenario.run(&[
            "harness",
            "grok",
            "model",
            model,
            "effort",
            effort,
            &scenario.issue_url(7),
        ]);
        assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
        assert!(result.stderr.contains(error), "{}", result.stderr);
        assert_eq!(scenario.entries("work"), ["widgets"]);
        assert!(scenario.issue_labels(7).is_empty());
        assert!(
            !scenario
                .path("home/.thirdshift/logs/acme/widgets/commands")
                .exists()
        );
        let calls = scenario.grok_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["argv"], serde_json::json!(["models"]));
        assert_eq!(calls[0]["stdin_null"], true);
        assert_eq!(calls[0]["GROK_DISABLE_AUTOUPDATER"], "1");
        assert_eq!(calls[0]["GROK_FOLDER_TRUST"], "0");
    }
}

#[test]
fn grok_sessions_are_unattended_load_the_factory_skill_and_report_tools_and_usage() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, &format!(r#"
echo '{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"run_terminal_command","input":{{"command":"cargo test"}}}}]}}}}'
{AGENT_OPENS_PR}
"#));
    let url = scenario.issue_url(7);
    let result = scenario.run_with_env(
        &[
            "harness", "grok", "model", "GROK-4.7", "effort", "High", &url,
        ],
        &[
            ("GROK_DISABLE_AUTOUPDATER", "0"),
            ("GROK_FOLDER_TRUST", "1"),
            ("GROK_SANDBOX", "workspace"),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    for line in [
        "implement: $ cargo test",
        "2 turns, $0.01",
        "sessions run on grok · grok-4.7 · high",
    ] {
        assert!(
            result.stderr.contains(line),
            "missing {line:?}: {}",
            result.stderr
        );
    }
    let calls = scenario.grok_calls();
    assert_eq!(calls.len(), 2);
    let session = &calls[1];
    let prompt = session["prompt"].as_str().unwrap();
    assert_eq!(
        prompt.lines().next().unwrap(),
        format!("/thirdshift-implement {url}")
    );
    assert_eq!(
        session["argv"],
        serde_json::json!([
            "--always-approve",
            "--sandbox",
            "off",
            "--output-format",
            "streaming-messages-json",
            "-m",
            "grok-4.7",
            "--reasoning-effort",
            "high",
            "-p",
            prompt
        ])
    );
    assert_eq!(session["git_status"], "");
    for call in calls {
        assert_eq!(call["stdin_null"], true);
        assert_eq!(call["GROK_DISABLE_AUTOUPDATER"], "1");
        assert_eq!(call["GROK_FOLDER_TRUST"], "0");
    }
    scenario.assert_every_grok_session_found_the_factory_skills();
    let logs = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/issue", "jsonl");
    let log = std::fs::read_to_string(scenario.path(&format!(
        "home/.thirdshift/logs/acme/widgets/commands/issue/{}",
        logs[0]
    )))
    .unwrap();
    assert!(log.contains("fake-grok-1"));
    assert!(log.contains("\"type\": \"result\""));
    assert!(
        scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Built with grok · grok-4.7 · high")
    );
}

#[test]
fn groks_error_is_in_the_failure_cause_even_when_the_cli_exits_zero() {
    for (event, exit, cause) in [
        (
            r#"{"type":"error","message":"backend unavailable"}"#,
            "exit 1",
            "grok exited 1: backend unavailable",
        ),
        (
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["quota exceeded"]}"#,
            "",
            "grok's turn failed: quota exceeded",
        ),
    ] {
        let scenario = Scenario::new();
        let script = if exit.is_empty() {
            format!("{AGENT_OPENS_PR}\necho '{event}' > \"$FAKE_GROK_RESULT\"\n")
        } else {
            format!("echo '{event}'\n{exit}")
        };
        scenario.agent_does_for(7, &script);
        let result = scenario.run(&["harness", "grok", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(cause), "{}", result.stderr);
        let logs = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/issue", "jsonl");
        let log = std::fs::read_to_string(scenario.path(&format!(
            "home/.thirdshift/logs/acme/widgets/commands/issue/{}",
            logs[0]
        )))
        .unwrap();
        let results = log
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|event| event["type"] == "result")
            .count();
        assert_eq!(results, 1, "{log}");
    }
}

#[test]
fn the_user_config_can_choose_grok_and_its_own_model_and_effort() {
    let scenario = Scenario::new();
    scenario.user_config_is("[harness]\ndefault = \"grok\"\n[harness.grok]\nmodel = \"grok-4.5\"\neffort = \"medium\"\n");
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("sessions run on grok · grok-4.5 · medium")
    );
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
}

#[test]
fn an_architecture_reviews_final_message_reaches_the_command() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent", "architecture"]);
    scenario.agent_does(
        r#"
idea=$(gh issue create --title "Deepen sessions" --body "An idea" --label needs-triage)
printf 'Architecture review idea: %s\n' "$idea" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = scenario.run(&["architect", "harness", "grok"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        scenario
            .issue_labels(8)
            .contains(&"architect-idea".to_string())
    );
    let calls = scenario.grok_calls();
    assert_eq!(
        calls[1]["prompt"].as_str().unwrap().lines().next().unwrap(),
        "/thirdshift-improve-codebase-architecture"
    );
    scenario.assert_every_grok_session_found_the_factory_skills();
}
