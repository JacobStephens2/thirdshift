use super::*;

const SPEC_DELIVERY: &str = r#"version = 1
description = "Review and deliver the Spec PR"
[worktrees.spec]
kind = "spec-branch"
[[steps]]
name = "land"
catalogue = "land-tickets"
worktree = "spec"
[[steps]]
name = "review"
catalogue = "spec-review"
worktree = "spec"
[[steps]]
name = "security"
catalogue = "security-review"
worktree = "spec"
[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "spec"
[[steps]]
name = "merge"
catalogue = "self-merge"
worktree = "spec"
"#;

const REVIEW_SPEC: &str = r#"
test -f issue-8.txt
echo reviewed > reviewed.txt
git add reviewed.txt
git commit -qm 'Review the Spec'
gh fake pr issue-7 body '"Reviewed the whole Spec. Closes #7"'
"#;

fn prepare_spec(source: &str) -> Scenario {
    let scenario = Scenario::new();
    write_process(&scenario, "complete-spec", source);
    scenario.spec_has_tickets(7, &[(8, &[])]);
    scenario.agent_does_for(8, &format!(
        "if printf '%s' \"$FAKE_CLAUDE_PROMPT\" | grep -q 'guidance mode'; then\n{CLEAN_REVIEW}\nelse\n{}\nfi",
        agent_lands_ticket(8, "issue-7")
    ));
    scenario.agent_does_for_in_session(7, 1, REVIEW_SPEC);
    scenario
}

#[test]
fn spec_self_merge_obeys_command_and_user_config_gates() {
    for (always, flag, merged) in [
        (false, Some("merge"), true),
        (true, None, true),
        (true, Some("no-merge"), false),
    ] {
        let scenario = prepare_spec(SPEC_DELIVERY);
        scenario.user_config_is(&format!("[merge]\nalways = {always}\n"));
        let mut args = vec!["process", "complete-spec", "issue", "7"];
        args.extend(flag);

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let pr = pr_from(&scenario, "issue-7");
        assert_eq!(pr["state"], if merged { "MERGED" } else { "OPEN" });
        assert_eq!(pr["isDraft"], false);
        if merged {
            assert_eq!(
                scenario.origin_git(&["show", "main:reviewed.txt"]),
                "reviewed\n"
            );
            assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
        }
    }
}

#[test]
fn spec_security_review_obeys_flags_and_user_config() {
    for (config, flag, enabled) in [
        (true, None, true),
        (false, Some("security-review"), true),
        (true, Some("no-security-review"), false),
    ] {
        let scenario = prepare_spec(SPEC_DELIVERY);
        scenario.user_config_is(&format!("[security]\nreview = {config}\n"));
        scenario.agent_does_for_in_session(7, 2, CLEAN_REVIEW);
        let mut args = vec!["process", "complete-spec", "issue", "7"];
        args.extend(flag);

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let calls: Vec<_> = scenario
            .claude_calls()
            .into_iter()
            .filter(|call| call["branch"] == "issue-7")
            .collect();
        assert_eq!(calls.len(), if enabled { 2 } else { 1 });
        let pr = pr_from(&scenario, "issue-7");
        assert_eq!(pr["isDraft"], false);
        if enabled {
            assert!(
                calls[1]["prompt"]
                    .as_str()
                    .unwrap()
                    .contains("guidance mode")
            );
            assert!(
                pr["body"]
                    .as_str()
                    .unwrap()
                    .contains("No unaddressed introduced findings.")
            );
        }
    }
}

#[test]
fn a_refused_or_early_ended_spec_security_review_holds_self_merge() {
    for (script, reason) in [
        (
            r#"printf '%s' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
            "Security review refused",
        ),
        ("exit 1", "Security review incomplete"),
    ] {
        let scenario = prepare_spec(SPEC_DELIVERY);
        scenario.agent_does_for_in_session(7, 2, script);

        let result = scenario.run(&[
            "process",
            "complete-spec",
            "issue",
            "7",
            "security-review",
            "merge",
        ]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(reason), "{}", result.stderr);
        let pr = pr_from(&scenario, "issue-7");
        assert_eq!(pr["state"], "OPEN");
        assert_eq!(pr["isDraft"], false);
        let body = pr["body"].as_str().unwrap();
        assert!(body.contains(reason), "{body}");
        assert!(body.contains("- [x] #8"), "{body}");
        assert!(!body.contains("Private refusal evidence"), "{body}");
        assert!(
            !scenario
                .issue_labels(7)
                .contains(&"in-progress".to_string())
        );
        let (_, log) = command_log(&scenario);
        assert!(step_line(&log, "deliver").contains("succeeded"), "{log}");
        assert!(step_line(&log, "merge").contains("failed"), "{log}");
    }
}

#[test]
fn a_failed_spec_review_preserves_work_and_the_draft_checklist() {
    let scenario = prepare_spec(SPEC_DELIVERY);
    scenario.agent_does_for_in_session(7, 1, "echo unfinished > unfinished.txt\nexit 9");

    let result = scenario.run(&["process", "complete-spec", "issue", "7", "merge"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let pr = pr_from(&scenario, "issue-7");
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], true);
    assert!(pr["body"].as_str().unwrap().contains("- [x] #8"));
    assert_eq!(
        scenario.origin_git(&["show", "issue-7:unfinished.txt"]),
        "unfinished\n"
    );
    for expected in [
        "Stopped at: Spec review",
        "Preserved: remotely on issue-7",
        "Recovery: thirdshift process complete-spec issue",
        "step deliver: 0s skipped",
    ] {
        assert!(
            result.stderr.contains(expected),
            "missing {expected}: {}",
            result.stderr
        );
    }
    assert!(
        scenario
            .issue_labels(7)
            .contains(&"in-progress".to_string())
    );
}

#[test]
fn spec_steps_keep_their_own_choices_and_delivery_repairs_the_spec_pr() {
    let source = SPEC_DELIVERY
        .replace("catalogue = \"spec-review\"", "catalogue = \"spec-review\"\nharness = \"codex\"\nmodel = \"gpt-6.1-sol\"\neffort = \"max\"")
        .replace("catalogue = \"security-review\"", "catalogue = \"security-review\"\nharness = \"claude\"\nmodel = \"sonnet\"\neffort = \"medium\"")
        .replace("catalogue = \"delivery\"", "catalogue = \"delivery\"\nharness = \"claude\"\nmodel = \"opus\"\neffort = \"high\"")
        .replace("catalogue = \"self-merge\"", "catalogue = \"self-merge\"\nharness = \"codex\"\nmodel = \"gpt-5.5\"\neffort = \"high\"");
    let scenario = prepare_spec(&source);
    // The fake Harnesses count their sessions independently: both reviews
    // are their Harness's first session on the Spec.
    scenario.agent_does_for_in_session(7, 1, &format!(r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
{CLEAN_REVIEW}
else
{REVIEW_SPEC}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure","url":"https://ci.example/test"}}]'
fi
"#));
    scenario.agent_does_for_in_session(
        7,
        2,
        "echo repaired > repaired.txt\ngit add repaired.txt\ngit commit -qm 'Repair Spec CI'",
    );

    let result = scenario.run(&["process", "complete-spec", "issue", "7", "security-review"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("starting Repair 1 of 5"),
        "{}",
        result.stderr
    );
    assert_eq!(pr_from(&scenario, "issue-7")["isDraft"], false);
    assert_eq!(
        scenario.origin_git(&["show", "issue-7:repaired.txt"]),
        "repaired\n"
    );
    let codex = scenario.codex_calls();
    assert_eq!(codex.len(), 1);
    assert_flag(&codex[0], "-m", "gpt-6.1-sol");
    assert!(argv_of(&codex[0]).contains(&"model_reasoning_effort=\"max\"".to_string()));
    let claude: Vec<_> = scenario
        .claude_calls()
        .into_iter()
        .filter(|call| call["branch"] == "issue-7")
        .collect();
    assert_eq!(claude.len(), 2);
    assert_flag(&claude[0], "--model", "sonnet");
    assert_flag(&claude[0], "--effort", "medium");
    assert_flag(&claude[1], "--model", "opus");
    assert_flag(&claude[1], "--effort", "high");
    let (_, log) = command_log(&scenario);
    let merge = step_line(&log, "merge");
    for choice in ["codex", "gpt-5.5", "high"] {
        assert!(merge.contains(choice), "{merge}");
    }
}

#[test]
fn spec_review_refuses_invalid_placement_before_any_session() {
    for (source, reason) in [
        (
            REVIEW.replace("skill = \"code-review\"", "catalogue = \"spec-review\""),
            "spec-review needs a Spec-branch worktree",
        ),
        (
            SPEC_DELIVERY.replace(
                "catalogue = \"spec-review\"",
                "catalogue = \"spec-review\"\noptional = true",
            ),
            "spec-review must be required",
        ),
        (
            SPEC_DELIVERY.replace(
                "catalogue = \"spec-review\"",
                "catalogue = \"spec-review\"\nparallel = \"reviews\"",
            ),
            "spec-review must be required",
        ),
        (
            SPEC_DELIVERY.replace("catalogue = \"land-tickets\"", "command = \"true\""),
            "spec-review needs land-tickets first",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.spec_has_tickets(7, &[(8, &[])]);
        write_process(&scenario, "complete-spec", &source);

        let result = scenario.run(&["process", "check", "complete-spec", "issue", "7"]);

        assert_no_session(&scenario, &result);
        assert!(result.stderr.contains(reason), "{}", result.stderr);
        assert!(scenario.issue_labels(7).is_empty());
    }
}

#[test]
fn spec_review_and_delivery_leave_the_spec_pr_ready_with_its_checklist() {
    let scenario = prepare_spec(SPEC_DELIVERY);

    let result = scenario.run(&["process", "complete-spec", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let pr = pr_from(&scenario, "issue-7");
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], false);
    let body = pr["body"].as_str().unwrap();
    assert!(body.contains("Reviewed the whole Spec"), "{body}");
    assert!(body.contains("- [x] #8"), "{body}");
    assert_eq!(
        scenario.origin_git(&["show", "issue-7:reviewed.txt"]),
        "reviewed\n"
    );
    let calls = scenario.claude_calls();
    let review = calls
        .iter()
        .find(|call| call["branch"] == "issue-7")
        .unwrap();
    let prompt = review["prompt"].as_str().unwrap();
    for expected in [
        "/thirdshift-code-review main",
        "Every Ticket",
        "Closes #7",
        "summary.json",
        "Leave the PR a draft",
    ] {
        assert!(prompt.contains(expected), "missing {expected}: {prompt}");
    }
    assert_eq!(calls.len(), 2, "Security review must be opt-in");
    assert!(
        !scenario
            .issue_labels(7)
            .contains(&"in-progress".to_string())
    );
    scenario.assert_cleaned_up("issue-7");
}

fn replacement_spec_review(failing_review: bool) -> (Scenario, RunResult) {
    let scenario = prepare_spec(SPEC_DELIVERY);
    scenario.agent_does_for_in_session(
        7,
        1,
        &format!(
            r#"
gh fake pr issue-7 state '"CLOSED"'
gh pr create --base main --head issue-7 --title Replacement --body 'Closes #7'
{}
"#,
            if failing_review { "exit 9" } else { "true" }
        ),
    );

    let result = scenario.run(&["process", "complete-spec", "issue", "7"]);
    (scenario, result)
}

#[test]
fn spec_review_can_replace_the_draft_before_delivery_captures_it() {
    let (scenario, result) = replacement_spec_review(false);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let prs: Vec<_> = state["prs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pr| pr["head"] == "issue-7")
        .collect();
    assert_eq!(prs.len(), 2);
    assert_eq!(prs[0]["state"], "CLOSED");
    assert_eq!(prs[1]["state"], "OPEN");
    assert_eq!(prs[1]["isDraft"], false);
    assert_eq!(result.stdout.trim(), prs[1]["url"].as_str().unwrap());
    let body = prs[1]["body"].as_str().unwrap();
    assert!(body.contains("- [x] #8"), "{body}");
    assert!(body.contains("Built with claude"), "{body}");
}

#[test]
fn spec_catalogue_review_accepts_a_replacement_pr_as_the_builtin_does() {
    // A replacement also receives the final checklist when the review fails.
    for failing_review in [false, true] {
        let (scenario, result) = replacement_spec_review(failing_review);
        assert_eq!(
            result.code,
            Some(i32::from(failing_review)),
            "{}",
            result.stderr
        );
        let pr = pr_from(&scenario, "issue-7");
        assert_eq!(pr["state"], "OPEN");
        assert_eq!(pr["isDraft"], failing_review);
        assert!(pr["body"].as_str().unwrap().contains("- [x] #8"), "{pr}");
    }
}

#[test]
fn failed_spec_catalogue_review_keeps_progress_and_notification_recovery() {
    let scenario = prepare_spec(SPEC_DELIVERY);
    scenario.agent_does_for_in_session(7, 1, "echo unfinished > unfinished.txt\nexit 9");

    let result = scenario.run(&["process", "complete-spec", "issue", "7", "merge"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let (_, notification) = saved_notification(&scenario, "process");
    let expected = [
        "Stopped at: Spec review",
        "Progress: 1/1 Tickets landed; final review incomplete",
        "Preserved: remotely on issue-7",
        "Recovery: thirdshift process complete-spec issue",
    ];
    let missing_terminal: Vec<_> = expected
        .iter()
        .filter(|line| !result.stderr.contains(**line))
        .collect();
    let missing_notification: Vec<_> = expected
        .iter()
        .filter(|line| !notification.contains(**line))
        .collect();
    assert!(
        missing_terminal.is_empty()
            && missing_notification.is_empty()
            && notification.contains("#8 landed"),
        "missing from terminal: {missing_terminal:?}; missing from notification: \
         {missing_notification:?}; ticket outcome present: {}\nterminal:\n{}\nnotification:\n{}",
        notification.contains("#8 landed"),
        result.stderr,
        notification,
    );
}
