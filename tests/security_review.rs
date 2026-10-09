mod support;

use support::Scenario;

const OPENS_PR: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'
"#;

const CLEAN: &str = r#"printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[]}' > "$FAKE_CLAUDE_FINAL_MESSAGE""#;

#[test]
fn command_words_override_the_config_and_review_is_off_by_default() {
    for (config, word, enabled) in [
        ("", None, false),
        ("[security]\nreview = true\n", None, true),
        (
            "[security]\nreview = true\n",
            Some("no-security-review"),
            false,
        ),
        (
            "[security]\nreview = false\n",
            Some("security-review"),
            true,
        ),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(config);
        scenario.agent_does_in_session(1, OPENS_PR);
        scenario.agent_does_in_session(2, CLEAN);
        let url = scenario.issue_url(7);
        let mut args = vec![url.as_str(), "merge"];
        args.extend(word);
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{config} {word:?}: {}", result.stderr);
        assert_eq!(scenario.claude_calls().len(), if enabled { 2 } else { 1 });
        assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    }
}

#[test]
fn invalid_or_missing_final_lines_hold_self_merge_but_report_only_runs_continue() {
    for final_message in [
        "Review incomplete",
        "Security review: {}",
        r#"Security review: {"unaddressed_count":0,"findings":["Cross-tenant read"]}"#,
        r#"Security review: {"unaddressed_count":1,"findings":[""]}"#,
        r#"Security review: {"unaddressed_count":0,"findings":[]}\nMore text"#,
    ] {
        for merge in [true, false] {
            let scenario = Scenario::new();
            scenario.agent_does_in_session(1, OPENS_PR);
            // Write literal data as a file through the existing fixture.
            std::fs::write(scenario.path("review-final.txt"), final_message).unwrap();
            scenario.agent_does_in_session(2, r#"cat "$(dirname "$FAKE_CLAUDE_SCRIPT")/review-final.txt" > "$FAKE_CLAUDE_FINAL_MESSAGE""#);
            let result = scenario.run(&[
                &scenario.issue_url(7),
                "security-review",
                if merge { "merge" } else { "no-merge" },
            ]);
            assert_eq!(
                result.code,
                Some(if merge { 1 } else { 0 }),
                "{final_message}: {}",
                result.stderr
            );
            assert!(
                result.stderr.contains("Security review incomplete"),
                "{}",
                result.stderr
            );
            let pr = &scenario.gh_state()["prs"][0];
            assert_eq!(pr["state"], "OPEN");
            assert_eq!(pr["isDraft"], false);
            assert!(
                pr["body"]
                    .as_str()
                    .unwrap()
                    .contains("Security review incomplete")
            );
        }
    }
}

#[test]
fn safeguard_refusals_hold_self_merge_without_publishing_private_diagnostics() {
    for (harness, script, cause) in [
        (
            "claude",
            r#"printf '%s\n' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
            "Claude Code's [cyber] safeguard refusal",
        ),
        (
            "codex",
            r#"printf '%s\n' 'Cybersecurity safeguard refused: Private refusal evidence' > "$FAKE_CODEX_ERROR"
exit 1"#,
            "Codex's cybersecurity safeguard refusal",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_in_session(1, OPENS_PR);
        scenario.agent_does_in_session(2, script);
        let result = scenario.run(&[
            &scenario.issue_url(7),
            "security-review",
            "merge",
            "harness",
            harness,
        ]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(cause), "{}", result.stderr);
        let pr = &scenario.gh_state()["prs"][0];
        assert_eq!(pr["state"], "OPEN");
        assert_eq!(pr["isDraft"], false);
        let body = pr["body"].as_str().unwrap();
        assert!(body.contains(cause), "{body}");
        assert!(!body.contains("Private refusal evidence"));
    }
}

#[test]
fn base_fixes_inherit_the_security_review_choice() {
    let scenario = Scenario::new();
    scenario.agent_does_for_in_session(
        7,
        1,
        &format!(
            r#"{OPENS_PR}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure"}}]'
gh fake checks "$(git rev-parse origin/main)" '[{{"name":"test","conclusion":"failure"}}]'
"#
        ),
    );
    scenario.agent_does_for_in_session(7, 2, CLEAN);
    scenario.agent_does_for_in_session(
        8,
        1,
        r#"
echo fixed > ci-fix.txt
git add ci-fix.txt
git commit -q -m 'Fix base CI'
gh pr create --base main --head issue-8 --title 'Fix base CI' --body 'Closes #8'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );
    scenario.agent_does_for_in_session(8, 2, CLEAN);
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "base-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 4);
    assert!(
        calls[3]["prompt"]
            .as_str()
            .unwrap()
            .contains("git diff main...HEAD")
    );
    assert_eq!(scenario.gh_state()["prs"][1]["state"], "MERGED");
}

#[test]
fn an_unaddressed_introduced_finding_holds_self_merge_with_the_pr_ready() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(2, r#"printf '%s\n' 'Security review: {"unaddressed_count":1,"findings":["Cross-tenant read"]}' > "$FAKE_CLAUDE_FINAL_MESSAGE""#);
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("Cross-tenant read"),
        "{}",
        result.stderr
    );
    let pr = &scenario.gh_state()["prs"][0];
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], false);
    assert!(pr["body"].as_str().unwrap().contains("### Security"));
    assert!(pr["body"].as_str().unwrap().contains("Cross-tenant read"));
}

#[test]
fn enabled_review_fixes_reach_origin_before_delivery_finishes() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(
        2,
        &format!(
            r#"
echo checked > security-fix.txt
git add security-fix.txt
git commit -q -m 'Fix reproduced finding'
{CLEAN}
"#
        ),
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario
            .origin_file("issue-7", "security-fix.txt")
            .as_deref(),
        Some("checked\n")
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert!(prompt.contains("guidance mode"));
    assert!(prompt.contains("Do not delegate"));
    assert!(prompt.contains("git diff main...HEAD"));
}

// Regression A.
#[test]
fn review_regression_a_security_hold_still_repairs_red_ci() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{OPENS_PR}\n{}",
            r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name":"test","conclusion":"success"}]'
"#
        ),
    );
    scenario.agent_does_in_session(
        2,
        r#"printf '%s\n' 'Security review: {"unaddressed_count":1,"findings":["Cross-tenant read"]}' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
    );
    scenario.agent_does_in_session(
        3,
        r#"
echo fixed > ci-fixed.txt
git add ci-fixed.txt
git commit -q -m 'Fix branch CI'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("Cross-tenant read"),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario.claude_calls().len(),
        3,
        "The held Merge run should repair red CI before handing over its PR: {}",
        result.stderr
    );
    assert_eq!(
        scenario.origin_file("issue-7", "ci-fixed.txt").as_deref(),
        Some("fixed\n")
    );
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "OPEN");
    assert_eq!(state["prs"][0]["isDraft"], false);
}

// Regression B.
#[test]
fn review_regression_b_body_write_failure_preserves_refusal_and_readiness() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(
        2,
        r#"
gh fake fails 'api --method PATCH repos/acme/widgets/pulls/1'
printf '%s\n' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "OPEN");
    assert_eq!(
        state["prs"][0]["isDraft"], false,
        "A refused review must leave the already-ready PR ready when only its summary PATCH fails: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("Security review refused"),
        "The failure must retain the Security refusal cause: {}",
        result.stderr
    );
}

// Regression C.
#[test]
fn review_regression_c_continuation_replaces_the_previous_security_outcome() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(
        2,
        r#"printf '%s\n' 'Security review: {"unaddressed_count":1,"findings":["Cross-tenant read"]}' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
    );
    let first = scenario.run(&[&scenario.issue_url(7), "security-review", "no-merge"]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    assert!(
        scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Cross-tenant read")
    );
    scenario.agent_does_in_session(
        3,
        r#"
echo fixed > feature.txt
git add feature.txt
git commit -q -m 'Fix the previously reported finding'
"#,
    );
    scenario.agent_does_in_session(4, CLEAN);
    let second = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(second.code, Some(0), "{}", second.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "MERGED");
    let body = state["prs"][0]["body"].as_str().unwrap();
    assert!(
        !body.contains("Cross-tenant read"),
        "The fixed finding remains falsely listed as unaddressed after a clean review: {body}"
    );
    assert_eq!(
        body.matches("### Security review outcome").count(),
        1,
        "{body}"
    );
}
