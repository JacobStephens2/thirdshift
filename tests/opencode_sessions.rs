//! OpenCode Commands through the scenario harness.
mod support;
use serde_json::{Value, json};
use support::{FACTORY_SKILLS, Scenario};

const AGENT_OPENS_PR: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'
"#;

fn records(scenario: &Scenario, extension: &str) -> Vec<Value> {
    std::fs::read_to_string(scenario.path(&format!("opencode-calls.{extension}")))
        .map(|text| serde_json::from_str(&text).unwrap())
        .unwrap_or_default()
}

#[test]
fn opencode_runs_standalone_with_stdin_skills_and_usage_despite_a_dropped_last_step() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[
        "harness",
        "opencode",
        "model",
        "provider/MiMo-V2.6-Pro",
        "effort",
        "high",
        &scenario.issue_url(7),
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = records(&scenario, "json");
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]["argv"],
        json!([
            "run",
            "--standalone",
            "--format",
            "json",
            "--auto",
            "-m",
            "provider/MiMo-V2.6-Pro#high"
        ])
    );
    assert_eq!(calls[0]["no_auto_update"], "1");
    assert_eq!(calls[0]["git_status"], "");
    assert!(
        calls[0]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("Load thirdshift-implement with your skill tool. ")
    );
    for skill in FACTORY_SKILLS {
        assert!(
            calls[0]["skill_files"][format!("{skill}/SKILL.md")].is_string(),
            "{skill}"
        );
    }
    assert!(
        result.stderr.contains(
            "2800 input tokens (400 cache read, 0 cache write), 800 output tokens (200 reasoning)"
        ),
        "{}",
        result.stderr
    );
    assert!(!result.stderr.contains("warning:"), "{}", result.stderr);
    let checks = records(&scenario, "checks.json");
    assert_eq!(checks[0]["argv"], calls[0]["argv"]);
    assert_eq!(checks[0]["prompt"], "Reply with OK.");
    assert_eq!(checks[0]["no_auto_update"], "1");
    let exports = records(&scenario, "exports.json");
    let exports = exports
        .iter()
        .filter(|call| call["argv"][3] != "check-session")
        .collect::<Vec<_>>();
    assert_eq!(
        exports[0]["argv"],
        json!(["session", "export", "--standalone", "fake-opencode-1"])
    );
    assert_eq!(exports[0]["no_auto_update"], "1");
    assert!(
        scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Built with opencode · provider/MiMo-V2.6-Pro · high")
    );
}

#[test]
fn user_config_selects_opencode_and_command_effort_overrides_its_variant() {
    let scenario = Scenario::new();
    scenario.user_config_is("[harness]\ndefault = \"opencode\"\n[harness.opencode]\nmodel = \"provider/model#low\"\neffort = \"low\"\n");
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&["effort", "high", &scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        records(&scenario, "json")[0]["argv"][6],
        "provider/model#high"
    );
}

#[test]
fn bad_models_and_efforts_fail_before_the_claim_worktree_and_command_log() {
    for (model, effort) in [("provider/bad-model", "high"), ("provider/model", "bogus")] {
        let scenario = Scenario::new();
        let result = scenario.run(&[
            "harness",
            "opencode",
            "model",
            model,
            "effort",
            effort,
            &scenario.issue_url(7),
        ]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("Model or Effort"),
            "{}",
            result.stderr
        );
        assert!(
            result.stderr.contains("provider.no-route"),
            "{}",
            result.stderr
        );
        assert!(records(&scenario, "json").is_empty());
        assert_eq!(records(&scenario, "checks.json").len(), 1);
        assert!(scenario.issue_labels(7).is_empty());
        assert_eq!(scenario.entries("work"), ["widgets"]);
        assert!(
            !scenario
                .path("home/.thirdshift/logs/acme/widgets/commands")
                .exists()
        );
    }
}

#[test]
fn an_effort_without_a_model_explains_how_to_select_the_route() {
    let scenario = Scenario::new();
    let result = scenario.run(&[
        "harness",
        "opencode",
        "effort",
        "high",
        &scenario.issue_url(7),
    ]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("set model <provider>/<model> too"),
        "{}",
        result.stderr
    );
    assert!(records(&scenario, "checks.json").is_empty());
}

#[test]
fn claude_only_instructions_are_linked_as_agents_and_kept_out_of_git() {
    for existing in [false, true] {
        let scenario = Scenario::new();
        std::fs::write(scenario.launch_dir().join("CLAUDE.md"), "claude rules\n").unwrap();
        if existing {
            std::fs::write(scenario.launch_dir().join("AGENTS.md"), "own rules\n").unwrap();
        }
        scenario.launch_git(&["add", "."]);
        scenario.launch_git(&["commit", "-m", "Project instructions"]);
        scenario.launch_git(&["push", "origin", "main"]);
        scenario.agent_does_for(7, AGENT_OPENS_PR);
        let result = scenario.run(&["harness", "opencode", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let calls = records(&scenario, "json");
        assert_eq!(
            calls[0]["agents_link"],
            if existing {
                Value::Null
            } else {
                json!("CLAUDE.md")
            }
        );
        assert_eq!(
            calls[0]["agents_text"],
            if existing {
                "own rules\n"
            } else {
                "claude rules\n"
            }
        );
        assert_eq!(calls[0]["git_status"], "");
        assert_eq!(
            scenario.origin_file("issue-7", "AGENTS.md"),
            existing.then(|| "own rules\n".to_string())
        );
    }
}

#[test]
fn an_exported_failed_outcome_fails_even_when_the_cli_exits_zero() {
    let scenario = Scenario::new();
    scenario.agent_does_for(
        7,
        "echo 'route became unavailable' > \"$FAKE_OPENCODE_ERROR\"",
    );
    let result = scenario.run_with_env(
        &["harness", "opencode", &scenario.issue_url(7)],
        &[("FAKE_OPENCODE_EXPORT_FAILED", "1")],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("opencode's turn failed: route became unavailable"),
        "{}",
        result.stderr
    );
    assert_eq!(records(&scenario, "json").len(), 1);
}

#[test]
fn a_failed_turn_in_the_stream_or_nonzero_exit_fails_with_opencodes_error() {
    for script in [
        "echo 'provider unavailable' > \"$FAKE_OPENCODE_ERROR\"\nexit 1",
        r#"echo '{"type":"error","sessionID":"fake-opencode-1","error":{"message":"provider unavailable"}}'"#,
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_for(7, script);
        let result = scenario.run(&["harness", "opencode", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("provider unavailable"),
            "{}",
            result.stderr
        );
        assert_eq!(records(&scenario, "json").len(), 1);
    }
}

#[test]
fn architecture_reviews_receive_the_last_reply_with_or_without_the_export() {
    for setting in [
        None,
        Some("FAKE_OPENCODE_NO_EXPORT"),
        Some("FAKE_OPENCODE_CORRUPT_EXPORT"),
    ] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["needs-triage", "ready-for-agent", "architecture"]);
        scenario.agent_does_in_session(
            1,
            r#"
url=$(gh issue create --title 'An idea' --body 'The idea' --label needs-triage)
printf 'Architecture review idea: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        );
        let env = setting
            .map(|setting| vec![(setting, "1")])
            .unwrap_or_default();
        let result = scenario.run_with_env(&["architect", "harness", "opencode"], &env);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            result.stdout.trim(),
            "https://github.com/acme/widgets/issues/8"
        );
        assert_eq!(
            result.stderr.contains("2800 input tokens"),
            setting.is_none()
        );
    }
}

#[test]
fn a_missing_skill_warns_in_the_progress_and_command_log_without_retrying() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run_with_env(
        &["harness", "opencode", &scenario.issue_url(7)],
        &[("FAKE_OPENCODE_SKIP_SKILL", "1")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let warning = "warning: the session never loaded thirdshift-implement with its skill tool";
    assert_eq!(result.stderr.matches(warning).count(), 1);
    assert_eq!(records(&scenario, "json").len(), 1);
    let dir = "home/.thirdshift/logs/acme/widgets/commands/issue";
    let name = scenario.entries(dir).pop().unwrap();
    let log = std::fs::read_to_string(scenario.path(&format!("{dir}/{name}"))).unwrap();
    assert_eq!(log.matches(warning).count(), 1);
}

#[test]
fn setup_proposes_no_model_and_checks_the_answer_standalone() {
    let scenario = Scenario::new();
    let result = scenario.run_on_terminal(
        &["setup"],
        &[],
        &[
            ("Harness for every Run", "opencode"),
            (
                "Model for opencode [opencode's own default]",
                "provider/model",
            ),
            ("Effort for opencode", "high"),
            ("Every Run a Merge run", ""),
            ("Every Run first fast-forwards", ""),
            ("Run notifications", ""),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let config: toml::Table = result.user_config.unwrap().parse().unwrap();
    assert_eq!(config["harness"]["default"].as_str(), Some("opencode"));
    assert_eq!(
        config["harness"]["opencode"]["model"].as_str(),
        Some("provider/model")
    );
    assert_eq!(
        config["harness"]["opencode"]["effort"].as_str(),
        Some("high")
    );
    let checks = records(&scenario, "checks.json");
    assert_eq!(
        checks[0]["argv"],
        json!([
            "run",
            "--standalone",
            "--format",
            "json",
            "--auto",
            "-m",
            "provider/model#high"
        ])
    );
    assert_eq!(checks[0]["prompt"], "Reply with OK.");
    assert_eq!(checks[0]["no_auto_update"], "1");
}

#[test]
fn an_exported_failed_check_stops_before_any_work_even_with_exit_zero() {
    let scenario = Scenario::new();
    let result = scenario.run_with_env(
        &["harness", "opencode", &scenario.issue_url(7)],
        &[("FAKE_OPENCODE_CHECK_EXPORT_FAILED", "1")],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("exported outcome is failed"),
        "{}",
        result.stderr
    );
    assert!(scenario.issue_labels(7).is_empty());
    assert!(records(&scenario, "json").is_empty());
}

#[test]
fn without_a_stream_session_id_no_resume_or_export_id_is_invented() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run_with_env(
        &["harness", "opencode", &scenario.issue_url(7)],
        &[("FAKE_OPENCODE_NO_SESSION_ID", "1")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(records(&scenario, "json").len(), 1);
    assert!(records(&scenario, "exports.json").is_empty());
    assert!(!result.stderr.contains("input tokens"));
}

#[test]
fn setup_retries_a_refused_model_and_effort_with_the_same_preflight_check() {
    let scenario = Scenario::new();
    let result = scenario.run_on_terminal(
        &["setup"],
        &[],
        &[
            ("Harness for every Run", "opencode"),
            ("Model for opencode", "provider/bad-model"),
            ("Effort for opencode", "high"),
            ("Model for opencode", "provider/model"),
            ("Effort for opencode", "low"),
            ("Every Run a Merge run", ""),
            ("Every Run first fast-forwards", ""),
            ("Run notifications", ""),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let checks = records(&scenario, "checks.json");
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0]["argv"][6], "provider/bad-model#high");
    assert_eq!(checks[1]["argv"][6], "provider/model#low");
}
