//! The Harness, Model and Effort a Command's sessions run on: chosen by
//! `harness`, `model` and `effort` on the command, else by the User config's
//! `[harness]` section, else Claude Code with its own Model and Effort;
//! checked before any work; and recorded in the Command log, the Activity
//! log, the pull request's body and the Run notification. How they reach a
//! Spec run's Tickets, a Base fix and the run a pass dispatches is covered
//! with each of those.

mod support;

use std::fs;

use serde_json::Value;
use support::resend::ResendStandIn;
use support::{RunResult, Scenario};

/// The agent commits its work and opens a PR that closes issue #7.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

/// What Claude says, and how it exits, when it refuses the Model it is
/// asked to run on.
const CLAUDE_REFUSES_THE_MODEL: &str = r#"
echo "There's an issue with the selected model (Opus 5.5). It may not exist or you may not have access to it."
exit 1
"#;

/// The scenario's repository's logs, relative to the scenario.
const LOGS: &str = "home/.thirdshift/logs/acme/widgets";

/// A scenario whose agent opens the PR for #7. The test call that checks a
/// named Model, the first call to `claude` when there is one, runs no script
/// of the agent's: it does nothing and succeeds.
fn scenario() -> Scenario {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    scenario
}

/// The arguments of each call to `claude` that was an agent session.
fn sessions(scenario: &Scenario) -> Vec<Vec<String>> {
    calls(scenario, true)
}

/// Each call to `claude` that was the test call checking a Model: its
/// arguments and what it was given on stdin.
fn test_calls(scenario: &Scenario) -> Vec<(Vec<String>, String)> {
    scenario
        .claude_calls()
        .into_iter()
        .filter(|call| !is_session(call))
        .map(|call| (argv(&call), call["stdin"].as_str().unwrap().to_string()))
        .collect()
}

fn calls(scenario: &Scenario, sessions: bool) -> Vec<Vec<String>> {
    scenario
        .claude_calls()
        .iter()
        .filter(|call| is_session(call) == sessions)
        .map(argv)
        .collect()
}

fn argv(call: &Value) -> Vec<String> {
    call["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_string())
        .collect()
}

/// Whether `call` was an agent session, which streams JSON, rather than the
/// test call.
fn is_session(call: &Value) -> bool {
    argv(call).iter().any(|arg| arg == "--output-format")
}

/// The value after `flag` in `args`, if `flag` is there.
fn value_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let at = args.iter().position(|arg| arg == flag)?;
    args.get(at + 1).map(String::as_str)
}

/// Assert every agent session ran with `model` and `effort`, each `None`
/// where it should be left to Claude, and that there was at least one.
fn assert_sessions_on(scenario: &Scenario, model: Option<&str>, effort: Option<&str>) {
    let sessions = sessions(scenario);
    assert!(!sessions.is_empty(), "no session ran");
    for args in sessions {
        assert_eq!(value_of(&args, "--model"), model, "{args:?}");
        assert_eq!(value_of(&args, "--effort"), effort, "{args:?}");
    }
}

/// Assert the Run failed with `error` before any work: no Claim, no
/// worktree, no agent session and no Command log.
fn assert_failed_before_any_work(scenario: &Scenario, result: &RunResult, error: &str) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(error),
        "expected {error:?} in stderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout, "");
    assert!(sessions(scenario).is_empty(), "a session ran");
    assert_eq!(scenario.entries("work"), vec!["widgets"]);
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert!(scenario.issue_labels(7).is_empty(), "the Claim was made");
    assert!(
        !scenario.path(&format!("{LOGS}/commands")).exists(),
        "a Command log was kept"
    );
}

/// The PR's body once the Run has ended.
fn pr_body(scenario: &Scenario) -> String {
    scenario.gh_state()["prs"][0]["body"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn with_no_flag_and_no_user_config_sessions_run_on_claude_with_no_model_and_no_test_call() {
    let scenario = scenario();

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_sessions_on(&scenario, None, None);
    assert!(test_calls(&scenario).is_empty());
    assert!(
        pr_body(&scenario).ends_with(
            "\n\nBuilt with claude · default model · default effort \
             <!-- thirdshift:built-with -->\n"
        ),
        "{}",
        pr_body(&scenario)
    );
}

#[test]
fn model_and_effort_on_the_command_reach_every_session_after_one_test_call_with_them() {
    let scenario = scenario();

    let result = scenario.run(&[
        "model",
        "claude-opus-5-5",
        &scenario.issue_url(7),
        "--effort",
        "high",
    ]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_sessions_on(&scenario, Some("claude-opus-5-5"), Some("high"));
    let test_calls = test_calls(&scenario);
    assert_eq!(
        test_calls,
        [(
            ["-p", "--model", "claude-opus-5-5", "--effort", "high"]
                .map(String::from)
                .to_vec(),
            "Reply with OK.".to_string()
        )]
    );
    assert!(
        result.stderr.contains(
            "thirdshift: checking the Model claude-opus-5-5 with a test call to claude\n"
        ),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn an_effort_alone_makes_no_test_call() {
    let scenario = scenario();

    let result = scenario.run(&[&scenario.issue_url(7), "effort", "max"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_sessions_on(&scenario, None, Some("max"));
    assert!(test_calls(&scenario).is_empty());
}

#[test]
fn the_user_config_sets_the_model_and_effort_of_the_harness_it_chooses() {
    let scenario = scenario();
    scenario.user_config_is(
        "[harness]\ndefault = \"claude\"\n\n[harness.claude]\nmodel = \"opus\"\neffort = \"low\"\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_sessions_on(&scenario, Some("opus"), Some("low"));
    assert_eq!(test_calls(&scenario).len(), 1);
}

#[test]
fn harness_claude_on_the_command_overrides_a_codex_default_and_takes_claudes_settings() {
    let scenario = scenario();
    scenario.user_config_is(
        "[harness]\ndefault = \"codex\"\n\n\
         [harness.claude]\nmodel = \"opus\"\neffort = \"\"\n\n\
         [harness.codex]\nmodel = \"gpt-6.1-sol\"\neffort = \"max\"\n",
    );

    let result = scenario.run(&["harness", "claude", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_sessions_on(&scenario, Some("opus"), None);
}

#[test]
fn a_model_on_the_command_wins_over_the_user_configs() {
    let scenario = scenario();
    scenario.user_config_is("[harness.claude]\nmodel = \"opus\"\neffort = \"low\"\n");

    let result = scenario.run(&[&scenario.issue_url(7), "model", "sonnet"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_sessions_on(&scenario, Some("sonnet"), Some("low"));
}

#[test]
fn a_model_claude_refuses_fails_the_run_before_any_work_with_claudes_error() {
    let scenario = scenario();
    scenario.agent_does_in_session(1, CLAUDE_REFUSES_THE_MODEL);

    let result = scenario.run(&[&scenario.issue_url(7), "model", "Opus 5.5"]);

    assert_failed_before_any_work(
        &scenario,
        &result,
        "thirdshift: claude refused a test call on the Model Opus 5.5: There's an issue with \
         the selected model (Opus 5.5).",
    );
    assert_eq!(test_calls(&scenario).len(), 1);
}

#[test]
fn a_harness_missing_from_path_fails_the_run_before_any_work_naming_what_chose_it() {
    for (config, chosen_by) in [
        (None, "chosen by the default"),
        (
            Some("[harness]\ndefault = \"claude\"\n"),
            "chosen by harness.default in the User config",
        ),
    ] {
        let scenario = scenario();
        if let Some(config) = config {
            scenario.user_config_is(config);
        }
        fs::remove_file(scenario.path("bin/claude")).unwrap();
        let path = format!("{}:/usr/bin:/bin", scenario.path("bin").display());

        let result = scenario.run_with_env(&[&scenario.issue_url(7)], &[("PATH", &path)]);

        assert_failed_before_any_work(&scenario, &result, "claude is not on PATH");
        assert!(
            result.stderr.contains(chosen_by),
            "stderr: {}",
            result.stderr
        );
    }
}

#[test]
fn the_command_log_the_activity_log_and_the_pr_body_name_the_harness_model_and_effort() {
    let scenario = scenario();

    let result = scenario.run(&[&scenario.issue_url(7), "model", "opus", "effort", "high"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let commands = scenario.log_files(&format!("{LOGS}/commands/issue"), "log");
    assert_eq!(commands.len(), 1, "{commands:?}");
    let command_log =
        fs::read_to_string(scenario.path(&format!("{LOGS}/commands/issue/{}", commands[0])))
            .unwrap();
    let opening: Vec<&str> = command_log.lines().take(4).collect();
    assert!(
        opening
            .iter()
            .any(|line| line.ends_with(" sessions run on claude · opus · high")),
        "{command_log}"
    );
    let activity = fs::read_to_string(scenario.path(&format!("{LOGS}/activity.log"))).unwrap();
    let started = activity.lines().next().unwrap();
    assert!(
        started.ends_with(&format!(
            " Run #7 started: commands/issue/{}, on claude · opus · high",
            commands[0]
        )),
        "{activity}"
    );
    assert!(
        pr_body(&scenario).ends_with(
            "Closes #7\n\nBuilt with claude · opus · high <!-- thirdshift:built-with -->\n"
        ),
        "{}",
        pr_body(&scenario)
    );
}

#[test]
fn the_run_notification_names_the_harness_model_and_effort() {
    let scenario = scenario();
    let resend = ResendStandIn::replying(200, r#"{"id":"49a3999c"}"#);

    let result = scenario.run_with_env(
        &[
            &scenario.issue_url(7),
            "email",
            "me@example.com",
            "effort",
            "max",
        ],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(
        text.contains("\nBuilt with claude · default model · max\nHost:"),
        "{text}"
    );
}
