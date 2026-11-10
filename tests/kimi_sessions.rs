//! Commands on Kimi Code through the scenario harness.

mod support;

use serde_json::{Value, json};
use support::Scenario;

const AGENT_OPENS_PR: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

fn kimi_calls(scenario: &Scenario) -> Vec<Value> {
    std::fs::read_to_string(scenario.path("kimi-calls.json"))
        .map(|text| serde_json::from_str(&text).unwrap())
        .unwrap_or_default()
}

#[test]
fn kimi_sessions_print_stream_json_with_null_stdin_a_set_home_and_the_factory_skill() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, &format!(r#"
echo '{{"role":"assistant","tool_calls":[{{"type":"function","id":"c1","function":{{"name":"Bash","arguments":"{{\"command\":\"cargo test\"}}"}}}}]}}'
{AGENT_OPENS_PR}
"#));
    let url = scenario.issue_url(7);
    let home = scenario.path("kimi-home");
    let result = scenario.run_with_env(
        &["harness", "kimi", "model", "kimi-k3.1-pro", &url],
        &[
            ("KIMI_CODE_HOME", home.to_str().unwrap()),
            ("KIMI_CODE_NO_AUTO_UPDATE", "0"),
            ("KIMI_CODE_TRUST_WORKSPACE", "0"),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    for line in [
        "implement: $ cargo test",
        "sessions run on kimi · kimi-k3.1-pro · default effort",
    ] {
        assert!(
            result.stderr.contains(line),
            "missing {line:?}: {}",
            result.stderr
        );
    }
    let calls = kimi_calls(&scenario);
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0]["argv"], json!(["provider", "list", "--json"]));
    let session = &calls[1];
    let prompt = session["prompt"].as_str().unwrap();
    assert_eq!(
        prompt.lines().next().unwrap(),
        format!("Load thirdshift-implement with your skill tool. {url}")
    );
    assert_eq!(
        session["argv"],
        json!([
            "--output-format",
            "stream-json",
            "-m",
            "kimi-k3.1-pro",
            "-p",
            prompt
        ])
    );
    for call in &calls {
        assert_eq!(call["stdin_null"], true);
        // The User's own Kimi home is kept, and the session's record read
        // back from it.
        assert_eq!(call["KIMI_CODE_HOME"], home.to_str().unwrap());
        assert_eq!(call["KIMI_CODE_TRUST_WORKSPACE"], "1");
        assert_eq!(call["KIMI_CODE_NO_AUTO_UPDATE"], "1");
        assert_eq!(call["KIMI_DISABLE_TELEMETRY"], "1");
    }
    assert!(
        !result
            .stderr
            .contains("warning: the session never loaded thirdshift-implement"),
        "{}",
        result.stderr
    );
    let skills = session["skill_files"].as_object().unwrap();
    assert!(
        skills
            .keys()
            .any(|path| path.starts_with("thirdshift-implement/")),
        "{skills:?}"
    );
    assert!(
        scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Built with kimi 9.8.7 · kimi-k3.1-pro · default effort")
    );
}

#[test]
fn usage_is_the_total_of_the_sessions_step_end_events_with_cache_kept_apart() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[
        "harness",
        "kimi",
        "model",
        "kimi-k3.1-pro",
        &scenario.issue_url(7),
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    // Three steps: uncached input 1200 + 300 + 45, cache reads 4096 + 5120 +
    // 5376, cache writes 512 + 0 + 64, output 150 + 80 + 20.
    assert!(
        result
            .stderr
            .contains("16713 input tokens (14592 cache read, 576 cache write), 250 output tokens"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("Model: kimi-k3.1-pro"),
        "{}",
        result.stderr
    );
    // Without KIMI_CODE_HOME, the session is pointed at Kimi's own default.
    let home = scenario.path("home/.kimi-code");
    for call in kimi_calls(&scenario) {
        assert_eq!(call["KIMI_CODE_HOME"], home.to_str().unwrap());
    }
    assert!(
        home.join("sessions/wd-fake/fake-kimi-1/agents/main/wire.jsonl")
            .is_file()
    );
    let logs = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/issue", "jsonl");
    let trace = support::session_trace::events(scenario.path(&format!(
        "home/.thirdshift/logs/acme/widgets/commands/issue/{}",
        logs[0]
    )));
    assert!(trace.contains("step.end"), "{trace}");
    assert!(trace.contains("fake-kimi-1"), "{trace}");
}

#[test]
fn a_session_without_step_end_usage_reports_usage_as_missing_not_zero() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run_with_env(
        &["harness", "kimi", &scenario.issue_url(7)],
        &[("FAKE_KIMI_NO_USAGE", "1")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(!result.stderr.contains("input tokens"), "{}", result.stderr);
    assert!(
        !result.stderr.contains("0 output tokens"),
        "{}",
        result.stderr
    );
}

#[test]
fn a_bad_model_or_any_effort_fails_before_any_work_and_names_the_valid_choices() {
    for (args, error, checked) in [
        (
            vec!["model", "kimi-k9"],
            "the Model kimi-k9 is not in Kimi Code's config: choose one of kimi-code/kimi-for-coding, kimi-k3.1-pro",
            1,
        ),
        (
            vec!["model", "kimi-k3.1-pro", "effort", "high"],
            "Kimi Code takes no Effort from the command line",
            0,
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_for(7, AGENT_OPENS_PR);
        let url = scenario.issue_url(7);
        let mut command = vec!["harness", "kimi"];
        command.extend(args);
        command.push(&url);
        let result = scenario.run(&command);
        assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
        assert!(result.stderr.contains(error), "{}", result.stderr);
        assert_eq!(scenario.entries("work"), ["widgets"]);
        assert!(scenario.issue_labels(7).is_empty());
        assert!(
            !scenario
                .path("home/.thirdshift/logs/acme/widgets/commands")
                .exists()
        );
        let calls = kimi_calls(&scenario);
        assert_eq!(calls.len(), checked, "{calls:?}");
        assert!(calls.iter().all(|call| call["prompt"].is_null()));
    }
}

#[test]
fn a_failed_turn_fails_with_kimis_exit_and_leaves_no_session_to_resume() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, "exit 1");
    let result = scenario.run(&["harness", "kimi", &scenario.issue_url(7)]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("kimi exited 1"), "{}", result.stderr);
    assert_eq!(kimi_calls(&scenario).len(), 2);
}

#[test]
fn the_user_config_can_choose_kimi_and_its_model() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[harness]\ndefault = \"kimi\"\n[harness.kimi]\nmodel = \"kimi-code/kimi-for-coding\"\n",
    );
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("sessions run on kimi · kimi-code/kimi-for-coding · default effort"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
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
    let result = scenario.run(&["architect", "harness", "kimi"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        scenario
            .issue_labels(8)
            .contains(&"architect-idea".to_string())
    );
    let calls = kimi_calls(&scenario);
    assert_eq!(
        calls[1]["prompt"].as_str().unwrap().lines().next().unwrap(),
        "Load thirdshift-improve-codebase-architecture with your skill tool."
    );
}
