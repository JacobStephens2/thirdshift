use super::*;

fn reviewed_process() -> String {
    LAND.replace(
        "name = \"ship\"",
        "name = \"security\"\ncatalogue = \"security-review\"\nworktree = \"fix\"\n\n[[steps]]\nname = \"ship\"",
    )
}

const CLEAN_REVIEW: &str = r#"
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
test -n "$report"
printf '[]' > "$report"
printf '%s' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

#[test]
fn security_review_config_runs_before_delivery_and_records_the_outcome() {
    let scenario = prepare_ship(&reviewed_process());
    scenario.user_config_is("[security]\nreview = true\n");
    scenario.agent_does_for_in_session(7, 2, CLEAN_REVIEW);
    let result = scenario.run(&["process", "ship", "issue", "7", "merge"]);
    assert_merged(&scenario, &result);
    let body = scenario.gh_state()["prs"][0]["body"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        body.contains("No unaddressed introduced findings."),
        "{body}"
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(
        calls[1]["prompt"]
            .as_str()
            .unwrap()
            .contains("guidance mode")
    );
}

#[test]
fn security_review_flags_override_the_config_and_skip_without_a_session() {
    for (config, flag, enabled) in [
        (false, Some("security-review"), true),
        (false, Some("--security-review"), true),
        (true, Some("no-security-review"), false),
        (true, Some("--no-security-review"), false),
        (false, None, false),
    ] {
        let scenario = prepare_ship(&reviewed_process());
        scenario.user_config_is(&format!(
            "[security]\nreview = {config}\n{}",
            if enabled { "" } else { "harness = 'codex'\n" }
        ));
        scenario.agent_does_for_in_session(7, 2, CLEAN_REVIEW);
        let mut args = vec!["process", "ship", "issue", "7"];
        args.extend(flag);
        let result = scenario.run_with_env(
            &args,
            if enabled {
                &[]
            } else {
                &[("FAKE_CODEX_SIGNED_OUT", "1")]
            },
        );
        assert_left_ready(&scenario, &result);
        assert_eq!(scenario.claude_calls().len(), if enabled { 2 } else { 1 });
        if !enabled {
            assert!(
                result
                    .stderr
                    .contains("Security review skipped: not requested"),
                "{}",
                result.stderr
            );
        }
    }
}

#[test]
fn security_review_refuses_invalid_placement_before_any_session() {
    let body = reviewed_process();
    for (invalid, needle) in [
        (REVIEW.replace("skill = \"code-review\"", "catalogue = \"security-review\""), "security-review needs an Issue-branch worktree"),
        (body.replace("catalogue = \"security-review\"", "catalogue = \"security-review\"\nparallel = \"checks\""), "security-review cannot be in a parallel group"),
        (body.replace("name = \"ship\"", "name = \"edit-again\"\ncommand = \"true\"\nworktree = \"fix\"\n\n[[steps]]\nname = \"ship\""), "security-review must follow the editing Steps"),
    ] {
        refuse_issue_branch(&invalid, needle);
    }
    let review =
        "[[steps]]\nname = \"security\"\ncatalogue = \"security-review\"\nworktree = \"fix\"\n";
    refuse_issue_branch(
        &format!("{LAND}\n{review}"),
        "security-review must precede delivery",
    );
    let on_throwaway = body
        .replace(
            "[worktrees.fix]",
            "[worktrees.scratch]\nkind = \"throwaway\"\n\n[worktrees.fix]",
        )
        .replace(
            "catalogue = \"security-review\"\nworktree = \"fix\"",
            "catalogue = \"security-review\"\nworktree = \"scratch\"",
        );
    refuse_issue_branch(
        &on_throwaway,
        "security-review must use the Issue-branch worktree",
    );
}

#[test]
fn security_review_uses_its_configured_harness_and_falls_back_before_work() {
    for signed_out in [false, true] {
        let scenario = prepare_ship(&reviewed_process());
        scenario.user_config_is("[harness]\ndefault = 'codex'\n[harness.codex]\nmodel = 'gpt-6.1-sol'\neffort = 'high'\n[security]\nreview = true\nharness = 'claude'\n");
        scenario.agent_does_for_in_session(7, 1, &format!(
            "if printf '%s' \"$FAKE_CLAUDE_PROMPT\" | grep -q 'guidance mode'; then\n{CLEAN_REVIEW}\nelse\n{COMMIT_FEATURE}\nfi"
        ));
        scenario.agent_does_for_in_session(7, 2, CLEAN_REVIEW);
        let result = scenario.run_with_env(
            &[
                "process", "ship", "issue", "7", "model", "gpt-5.5", "effort", "medium",
            ],
            if signed_out {
                &[("FAKE_CLAUDE_SIGNED_OUT", "1")]
            } else {
                &[]
            },
        );
        assert_left_ready(&scenario, &result);
        assert!(
            scenario.gh_state()["prs"][0]["body"]
                .as_str()
                .unwrap()
                .contains("No unaddressed introduced findings.")
        );
        let calls = if signed_out {
            scenario.codex_calls()
        } else {
            scenario.claude_calls()
        };
        let review = calls.last().unwrap();
        assert!(
            review["prompt"].as_str().unwrap().contains("guidance mode"),
            "{calls:?}"
        );
        if signed_out {
            assert!(
                result
                    .stderr
                    .contains("reviewing on codex, the default Harness"),
                "{}",
                result.stderr
            );
            let argv = review["argv"].to_string();
            assert!(argv.contains("gpt-6.1-sol"), "{argv}");
            assert!(argv.contains("high"), "{argv}");
        } else {
            assert_eq!(scenario.codex_calls().len(), 1);
        }
    }
}

#[test]
fn security_review_explicit_harness_is_refused_before_any_session() {
    for enabled in [false, true] {
        let scenario = Scenario::new();
        write_process(
            &scenario,
            "ship",
            &reviewed_process().replace(
                "catalogue = \"security-review\"",
                "catalogue = \"security-review\"\nharness = \"codex\"",
            ),
        );
        scenario.user_config_is(&format!("[security]\nreview = {enabled}\n"));
        let result = scenario.run_with_env(
            &["process", "ship", "issue", "7"],
            &[("FAKE_CODEX_SIGNED_OUT", "1")],
        );
        assert_no_session(&scenario, &result);
        assert!(
            result
                .stderr
                .contains("step security's Harness codex can't start"),
            "{}",
            result.stderr
        );
    }
}

#[test]
fn security_review_refusal_or_early_end_holds_self_merge_after_delivery() {
    for (script, reason) in [
        (
            r#"printf '%s' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
            "Security review refused",
        ),
        ("exit 1", "Security review incomplete"),
        (
            r#"printf '%s' 'Security review: not JSON' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
            "Security review incomplete",
        ),
    ] {
        let scenario = prepare_ship(&reviewed_process());
        scenario.agent_does_for_in_session(7, 2, script);
        let result = scenario.run(&["process", "ship", "issue", "7", "security-review", "merge"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(reason), "{}", result.stderr);
        assert!(scenario.gh_calls_of("pr", "merge").is_empty());
        let state = scenario.gh_state();
        let pr = &state["prs"][0];
        assert_eq!(pr["state"], "OPEN");
        assert_eq!(pr["isDraft"], false);
        let body = pr["body"].as_str().unwrap();
        assert!(body.contains(reason), "{body}");
        assert!(!body.contains("Private refusal evidence"), "{body}");
        assert_eq!(scenario.issue_labels(7), Vec::<String>::new());
        let (_, log) = command_log(&scenario);
        assert!(step_line(&log, "ship").contains("succeeded"), "{log}");
        assert!(step_line(&log, "land").contains("failed"), "{log}");
    }
}

#[test]
fn security_review_startup_failure_retries_on_the_default_harness() {
    let scenario = prepare_ship(&reviewed_process());
    scenario.user_config_is(
        "[harness]\ndefault = 'codex'\n[security]\nreview = true\nharness = 'grok'\n",
    );
    scenario.agent_does_for_in_session(7, 1, &format!(r#"
if [ -n "${{FAKE_GROK_RESULT:-}}" ]; then
    printf '%s' '{{"type":"result","subtype":"error_during_execution","num_turns":0,"usage":{{"input_tokens":0,"output_tokens":0}},"errors":["Not signed in."]}}' > "$FAKE_GROK_RESULT"
    exit 1
fi
{COMMIT_FEATURE}
"#));
    scenario.agent_does_for_in_session(7, 2, CLEAN_REVIEW);
    let result = scenario.run(&["process", "ship", "issue", "7", "merge"]);
    assert_merged(&scenario, &result);
    assert!(
        result
            .stderr
            .contains("reviewing on codex, the default Harness"),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario
            .grok_calls()
            .iter()
            .filter(|call| call["prompt"].is_string())
            .count(),
        1
    );
    assert_eq!(scenario.codex_calls().len(), 2);
}

#[test]
fn later_clean_security_review_does_not_clear_an_earlier_refusal_hold() {
    let body = reviewed_process().replace(
        "name = \"ship\"",
        "name = \"security-again\"\ncatalogue = \"security-review\"\nworktree = \"fix\"\n\n[[steps]]\nname = \"ship\"",
    );
    let scenario = prepare_ship(&body);
    scenario.agent_does_for_in_session(
        7,
        2,
        r#"printf '%s' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
    );
    scenario.agent_does_for_in_session(7, 3, CLEAN_REVIEW);
    let result = scenario.run(&["process", "ship", "issue", "7", "security-review", "merge"]);
    assert_eq!(scenario.claude_calls().len(), 3, "{}", result.stderr);
    assert!(
        scenario.gh_calls_of("pr", "merge").is_empty(),
        "{}",
        result.stderr
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "OPEN");
    assert!(
        scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Security review refused")
    );
    let review_logs: Vec<_> = result
        .stderr
        .lines()
        .filter_map(|line| {
            line.split_once("logging the session to ")
                .map(|(_, path)| path)
        })
        .filter(|path| path.ends_with("-security-review.logs.jsonl"))
        .collect();
    assert_eq!(review_logs.len(), 2, "{}", result.stderr);
    assert_ne!(
        review_logs[0], review_logs[1],
        "Each review must retain its own Session trace"
    );
}

#[test]
fn show_lists_every_custom_security_review_trace() {
    let body = reviewed_process().replace(
        "name = \"ship\"",
        "name = \"security-again\"\ncatalogue = \"security-review\"\nworktree = \"fix\"\n\n[[steps]]\nname = \"ship\"",
    );
    let scenario = prepare_ship(&body);
    scenario.agent_does_for_in_session(7, 2, CLEAN_REVIEW);
    scenario.agent_does_for_in_session(7, 3, CLEAN_REVIEW);
    let result = scenario.run(&["process", "ship", "issue", "7", "security-review"]);
    assert_left_ready(&scenario, &result);
    assert_eq!(scenario.claude_calls().len(), 3, "{}", result.stderr);
    let status = status_json(&scenario);
    let name = process_command(&status)["name"].as_str().unwrap();
    let shown = scenario.run(&["show", name, "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: serde_json::Value = serde_json::from_str(&shown.stdout).unwrap();
    let reviews: Vec<_> = shown["command"]["session_logs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .filter(|path| path.ends_with("-security-review.logs.jsonl"))
        .collect();
    assert_eq!(reviews.len(), 2, "{}", shown);
}
