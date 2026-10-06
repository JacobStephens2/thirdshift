//! A Command's sessions on Antigravity CLI, through real Runs against fake agy.
mod support;

use support::{FACTORY_SKILLS, Scenario};

const AGENT_OPENS_PR: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'
"#;

#[test]
fn a_run_uses_agy_with_unattended_flags_skills_and_settled_model_and_effort() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, &format!(r#"{AGENT_OPENS_PR}
echo '{{"event":"step_update","step_update":{{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{{"parameters":{{"CommandLine":"cargo test"}}}}}}}}'
echo '{{"event":"step_update","step_update":{{"step_index":2,"state":"DONE","step_type":"tool","tool_name":"run_command"}}}}'
"#));
    let result = scenario.run(&[
        "harness",
        "agy",
        "model",
        "Gemini-3.8-Flash",
        "effort",
        "Medium",
        &scenario.issue_url(7),
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty());
    let calls = scenario.agy_calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    let argv: Vec<&str> = call["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    assert_eq!(
        &argv[..8],
        [
            "-p",
            "--dangerously-skip-permissions",
            "--output-format",
            "stream-json",
            "--model",
            "gemini-3.8-flash",
            "--effort",
            "medium"
        ]
    );
    assert!(!argv.contains(&"--sandbox"));
    assert_eq!(call["stdin_null"], true);
    assert_eq!(call["auto_update"], "true");
    assert_eq!(call["git_status"], "");
    assert!(
        call["prompt"]
            .as_str()
            .unwrap()
            .starts_with("/thirdshift-implement ")
    );
    for skill in FACTORY_SKILLS {
        assert!(
            call["skill_files"][format!("{skill}/SKILL.md")]
                .as_str()
                .is_some(),
            "{skill}"
        );
    }
    let checks = scenario.agy_checks();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["argv"], serde_json::json!(["models"]));
    assert_eq!(checks[0]["auto_update"], "true");
    assert_eq!(checks[0]["stdin_null"], true);
    assert!(
        result
            .stderr
            .contains("sessions run on agy · gemini-3.8-flash · medium"),
        "{}",
        result.stderr
    );
    for line in [
        "implement: session started",
        "implement: $ cargo test",
        "1200 input tokens (200 cached), 300 output tokens (150 thinking), 1500 total tokens",
    ] {
        assert!(result.stderr.contains(line), "{}", result.stderr);
    }
    assert!(
        scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Built with agy · gemini-3.8-flash · medium")
    );
    let sessions = "home/.thirdshift/logs/acme/widgets/sessions";
    let logs = scenario.entries(sessions);
    let log = std::fs::read_to_string(scenario.path(&format!("{sessions}/{}", logs[0]))).unwrap();
    assert!(log.contains("\"event\": \"init\""), "{log}");
    assert!(log.contains("\"status\": \"SUCCESS\""), "{log}");
    assert!(log.contains("step_update"), "{log}");
}

#[test]
fn claude_only_instructions_are_linked_as_gemini_and_kept_out_of_git() {
    let scenario = Scenario::new();
    std::fs::write(scenario.launch_dir().join("CLAUDE.md"), "project rules\n").unwrap();
    scenario.launch_git(&["add", "CLAUDE.md"]);
    scenario.launch_git(&["commit", "-m", "Project rules"]);
    scenario.launch_git(&["push", "origin", "main"]);
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&["harness", "agy", &scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.agy_calls();
    assert_eq!(calls[0]["gemini_link"], "CLAUDE.md");
    assert_eq!(calls[0]["gemini_text"], "project rules\n");
    assert_eq!(calls[0]["git_status"], "");
    assert_eq!(scenario.origin_file("issue-7", "GEMINI.md"), None);
}

#[test]
fn a_session_with_unfinished_work_is_resumed_by_conversation_with_the_same_flags() {
    let scenario = Scenario::new();
    scenario.agent_does_for_in_session(7, 1, &format!(r#"{AGENT_OPENS_PR}
echo '{{"event":"step_update","step_update":{{"step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{{"parameters":{{"CommandLine":"cargo test"}}}}}}}}'
"#));
    let result = scenario.run(&[
        "harness",
        "agy",
        "model",
        "gemini-3.8-flash-high",
        &scenario.issue_url(7),
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.agy_calls();
    assert_eq!(calls.len(), 2, "{}", result.stderr);
    let args = calls[1]["argv"].as_array().unwrap();
    assert_eq!(args[6], "--conversation");
    assert_eq!(args[7], "fake-conversation-1");
    assert!(
        calls[1]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("Your background work (cargo test) was killed")
    );
    for call in calls {
        assert_eq!(call["auto_update"], "true");
        assert_eq!(call["stdin_null"], true);
        assert_eq!(call["argv"][1], "--dangerously-skip-permissions");
        assert_eq!(call["argv"][3], "stream-json");
        assert_eq!(call["argv"][5], "gemini-3.8-flash-high");
    }
}

#[test]
fn existing_instruction_files_win_and_no_link_is_made_without_claude() {
    for existing in [None, Some("AGENTS.md"), Some("GEMINI.md")] {
        let scenario = Scenario::new();
        if let Some(existing) = existing {
            std::fs::write(scenario.launch_dir().join("CLAUDE.md"), "claude rules\n").unwrap();
            std::fs::write(scenario.launch_dir().join(existing), "own rules\n").unwrap();
            scenario.launch_git(&["add", "."]);
            scenario.launch_git(&["commit", "-m", "Project instructions"]);
            scenario.launch_git(&["push", "origin", "main"]);
        }
        scenario.agent_does_for(7, AGENT_OPENS_PR);
        let result = scenario.run(&["harness", "agy", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let calls = scenario.agy_calls();
        assert_eq!(calls[0]["gemini_link"], serde_json::Value::Null);
        if existing == Some("GEMINI.md") {
            assert_eq!(calls[0]["gemini_text"], "own rules\n");
        }
        assert!(
            scenario.agy_checks().is_empty(),
            "no named settings need a catalog check"
        );
    }
}

#[test]
fn bad_models_efforts_and_catalog_failure_stop_before_any_work() {
    for (model, effort, error) in [
        ("gemini-99", "high", "choose one of gemini-3.8-flash-high"),
        ("Gemini-3.1-Pro", "Medium", "choose one of low, high"),
        (
            "gemini-3.8-flash",
            "",
            "requires an Effort: choose one of low, medium, high",
        ),
        (
            "gemini-3.8-flash-high",
            "Max",
            "choose one of low, medium, high",
        ),
    ] {
        let scenario = Scenario::new();
        let mut args = vec!["harness", "agy", "model", model];
        if !effort.is_empty() {
            args.extend(["effort", effort]);
        }
        let url = scenario.issue_url(7);
        args.push(&url);
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(error), "{}", result.stderr);
        assert!(scenario.agy_calls().is_empty());
        assert!(scenario.issue_labels(7).is_empty());
        assert_eq!(scenario.entries("work"), ["widgets"]);
        assert!(
            !scenario
                .path("home/.thirdshift/logs/acme/widgets/commands")
                .exists()
        );
    }
    let scenario = Scenario::new();
    let result = scenario.run_with_env(
        &[
            "harness",
            "agy",
            "model",
            "gemini-3.8-flash-high",
            &scenario.issue_url(7),
        ],
        &[("FAKE_AGY_CATALOG_ERROR", "sign in first")],
    );
    assert_eq!(result.code, Some(1));
    assert!(
        result.stderr.contains(
            "agy models failed, so Antigravity CLI's Models can't be read: sign in first"
        ),
        "{}",
        result.stderr
    );
    assert!(scenario.agy_calls().is_empty());
}

#[test]
fn an_agy_user_config_and_command_overrides_choose_its_own_settings() {
    let scenario = Scenario::new();
    scenario.user_config_is("[harness]\ndefault = \"agy\"\n[harness.agy]\nmodel = \"Gemini 3.8 Flash (High)\"\neffort = \"HIGH\"\n");
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[&scenario.issue_url(7), "effort", "Low"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("sessions run on agy · gemini-3.8-flash-high · low"),
        "{}",
        result.stderr
    );
}

#[test]
fn an_error_result_or_nonzero_exit_fails_with_agys_own_error() {
    for code in [0, 3] {
        let scenario = Scenario::new();
        scenario.agent_does_for(7, &format!(r#"
echo '{{"event":"result","result":{{"status":"ERROR","error":"backend unavailable","response":""}}}}' > "$FAKE_AGY_RESULT"
exit {code}
"#));
        let result = scenario.run(&["harness", "agy", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let ending = if code == 0 {
            "agy's turn failed"
        } else {
            "agy exited 3"
        };
        assert!(
            result
                .stderr
                .contains(&format!("{ending}: backend unavailable")),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.agy_calls().len(), 1);
    }
}

#[test]
fn an_architecture_review_uses_the_final_result_instead_of_text_deltas() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    scenario.user_config_is("[harness]\ndefault = \"agy\"\n");
    scenario.agent_does(r#"
gh issue create --title Plan --body Plan --label needs-triage > /dev/null
echo '{"event":"step_update","step_update":{"step_index":1,"step_type":"agent_response","state":"DONE","text_delta":"Architecture review plan: https://github.com/acme/widgets/issues/99"}}'
echo 'Architecture review plan: https://github.com/acme/widgets/issues/8' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    let result = scenario.run(&["architect", "--plan-only"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("published the plan https://github.com/acme/widgets/issues/8"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.agy_calls().len(), 1);
    assert!(
        scenario.agy_calls()[0]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("/thirdshift-improve-codebase-architecture")
    );
}

#[test]
fn a_repair_uses_the_commands_agy_model_effort_and_environment() {
    let scenario = Scenario::new();
    scenario.agent_does_for_in_session(7, 1, &format!(r#"{AGENT_OPENS_PR}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure","url":"https://ci.example/test"}}]'
"#));
    scenario.agent_does_for_in_session(7, 2, r#"
echo fix > fix.txt
git add fix.txt
git commit -q -m Fix
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success","url":"https://ci.example/test"}]'
"#);
    let result = scenario.run(&[
        "harness",
        "agy",
        "model",
        "gemini-3.8-flash",
        "effort",
        "high",
        &scenario.issue_url(7),
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.agy_calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[1]["prompt"].as_str().unwrap().contains("CI"));
    for call in calls {
        assert_eq!(call["argv"][5], "gemini-3.8-flash");
        assert_eq!(call["argv"][7], "high");
        assert_eq!(call["auto_update"], "true");
        assert_eq!(call["stdin_null"], true);
    }
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
}

#[test]
fn an_interrupt_stops_an_agy_session_and_saves_its_work() {
    let scenario = Scenario::new();
    scenario.agent_does_for(
        7,
        &format!(
            "echo 'half done' > wip.txt\ntouch '{}'\nsleep 900\n",
            scenario.path("agy-started").display()
        ),
    );
    let result = scenario.run_and_signal(
        &["harness", "agy", &scenario.issue_url(7)],
        "agy-started",
        "TERM",
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    assert_eq!(
        scenario.origin_file("issue-7", "wip.txt"),
        Some("half done\n".to_string())
    );
}
