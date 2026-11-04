//! Security reviews of existing pull requests through the CLI and real Git.

mod support;

use serde_json::json;
use support::Scenario;

fn pull_request(scenario: &Scenario) -> String {
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    scenario.github_has_pr("colleague", "main", "OPEN")
}

const CLEAN_REVIEW: &str = r#"
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
test -n "$report"
printf '[]' > "$report"
printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

const NO_FINDINGS: &str =
    "printf 'No Standards or Spec findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"";

fn review_sessions(ordinary: &str, security: &str) -> String {
    format!(
        "if printf '%s' \"$FAKE_CLAUDE_PROMPT\" | grep -q '^Validate the final Review run'; then\n\
         printf 'Final validation passed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n\
         elif printf '%s' \"$FAKE_CLAUDE_PROMPT\" | grep -q 'guidance mode'; then\n{security}\nelse\n{ordinary}\nfi"
    )
}

#[test]
fn security_fixes_follow_addressed_findings_and_reach_the_pr_branch() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.agent_does(&review_sessions(
        r#"
    echo 'addressed findings' > addressed.txt
    git add addressed.txt
    git commit -qm 'Address Standards and Spec findings'
    printf 'Fixed Standards and Spec findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        &format!(
            r#"
    test -f addressed.txt
    echo 'bounded input' > security.txt
    git add security.txt
    git commit -qm 'Fix introduced input bound'
    {CLEAN_REVIEW}
"#
        ),
    ));

    let result = scenario.run(&["review", &url, "--security-review"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario
            .origin_file("colleague", "addressed.txt")
            .as_deref(),
        Some("addressed findings\n")
    );
    assert_eq!(
        scenario.origin_file("colleague", "security.txt").as_deref(),
        Some("bounded input\n")
    );
    let state = scenario.gh_state();
    let comments = state["prs"][0]["postedComments"].as_array().unwrap();
    assert_eq!(comments.len(), 1);
    let comment = comments[0].as_str().unwrap();
    assert!(
        comment.contains("Fixed Standards and Spec findings."),
        "{comment}"
    );
    assert!(
        comment.contains("No unaddressed introduced findings."),
        "{comment}"
    );
}

#[test]
fn security_review_setting_and_flags_select_the_session() {
    for (configured, flag, enabled) in [
        (true, None, true),
        (false, Some("security-review"), true),
        (true, Some("no-security-review"), false),
        (true, Some("--no-security-review"), false),
        (false, None, false),
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        scenario.user_config_is(&format!(
            "[security]\nreview = {configured}\n{}",
            if enabled { "" } else { "harness = 'codex'\n" }
        ));
        scenario.agent_does(&review_sessions(NO_FINDINGS, CLEAN_REVIEW));
        let mut args = vec!["review", &url];
        args.extend(flag);

        let result = scenario.run_with_env(&args, &[("FAKE_CODEX_SIGNED_OUT", "1")]);

        assert_eq!(
            result.code,
            Some(0),
            "{configured}, {flag:?}: {}",
            result.stderr
        );
        assert_eq!(scenario.claude_calls().len(), if enabled { 2 } else { 1 });
        assert!(scenario.codex_calls().is_empty());
    }
}

#[test]
fn security_findings_and_incomplete_reviews_appear_in_the_single_comment() {
    for (final_line, expected) in [
        (
            r#"Security review: {"unaddressed_count":1,"findings":["Introduced unchecked input"],"pre_existing_count":0}"#,
            "Introduced unchecked input",
        ),
        (
            "Security review: incomplete: prerequisite unavailable",
            "Security review incomplete: prerequisite unavailable",
        ),
        ("not a valid report", "Security review incomplete"),
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        let body = scenario.gh_state()["prs"][0]["body"].clone();
        scenario.agent_does(&review_sessions(
            "printf 'Standards and Spec findings addressed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
            &format!(
                r#"
    {CLEAN_REVIEW}
    printf '%s\n' '{final_line}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
            ),
        ));

        let result = scenario.run(&["review", &url, "security-review"]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let state = scenario.gh_state();
        assert_eq!(state["prs"][0]["body"], body);
        let comments = state["prs"][0]["postedComments"].as_array().unwrap();
        assert_eq!(comments.len(), 1);
        let comment = comments[0].as_str().unwrap();
        assert!(
            comment.contains("Standards and Spec findings addressed."),
            "{comment}"
        );
        assert!(comment.contains("### Security"), "{comment}");
        assert!(comment.contains(expected), "{comment}");
    }
}

#[test]
fn security_review_uses_its_configured_harness_and_preflight_fallback() {
    for signed_out in [false, true] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        scenario.user_config_is(
            "[harness]\ndefault = 'codex'\n[harness.codex]\nmodel = 'gpt-6.1-sol'\neffort = 'high'\n[harness.claude]\nmodel = 'claude-opus-5-5'\neffort = 'high'\n[security]\nreview = true\nharness = 'claude'\n"
        );
        scenario.agent_does(&format!(
            "if test -z \"$FAKE_CLAUDE_FINAL_MESSAGE\"; then\necho OK\nexit 0\nfi\n{}",
            review_sessions(NO_FINDINGS, CLEAN_REVIEW)
        ));

        let result = scenario.run_with_env(
            &["review", &url, "model", "gpt-5.5", "effort", "medium"],
            if signed_out {
                &[("FAKE_CLAUDE_SIGNED_OUT", "1")]
            } else {
                &[]
            },
        );

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let calls = if signed_out {
            scenario.codex_calls()
        } else {
            scenario.claude_calls()
        };
        let review = calls
            .iter()
            .find(|call| {
                call["prompt"]
                    .as_str()
                    .unwrap_or("")
                    .contains("guidance mode")
            })
            .unwrap_or_else(|| {
                panic!(
                    "signed_out={signed_out}; expected calls: {calls:?}; stderr: {}",
                    result.stderr
                )
            });
        let argv = review["argv"].to_string();
        assert!(
            argv.contains(if signed_out {
                "gpt-6.1-sol"
            } else {
                "claude-opus-5-5"
            }),
            "{argv}"
        );
        assert!(argv.contains("high"), "{argv}");
        let prompt = review["prompt"].as_str().unwrap();
        assert!(prompt.contains(&url), "{prompt}");
        assert!(prompt.contains(".thirdshift-review-input.md"), "{prompt}");
        assert!(!prompt.contains(".thirdshift-issue.md"), "{prompt}");
        assert!(
            prompt.contains("Do not push, merge, edit the pull request or post comments."),
            "{prompt}"
        );
        if signed_out {
            assert!(
                result
                    .stderr
                    .contains("reviewing on codex, the default Harness"),
                "{}",
                result.stderr
            );
        }
    }
}

#[test]
fn recommend_only_skips_security_review_even_when_requested() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let before = scenario.origin_log("colleague");
    scenario.user_config_is("[security]\nreview = true\nharness = 'codex'\n");
    scenario.agent_does("printf 'Recommendations only.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run_with_env(
        &["review", &url, "--recommend-only", "--security-review"],
        &[("FAKE_CODEX_SIGNED_OUT", "1")],
    );

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.origin_log("colleague"), before);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(scenario.codex_calls().is_empty());
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("Recommendations only."), "{comment}");
    assert!(!comment.contains("### Security"), "{comment}");
}

#[test]
fn pre_existing_security_findings_stay_in_private_records() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let old = json!([{
        "fingerprint": "old-input-bound",
        "title": "Old unchecked bound",
        "description": "Private old vulnerability evidence.",
        "proof_of_concept": {
            "test": "assert_bounded_input();",
            "command": "cargo test bounded_input",
            "head_exit_code": 101,
            "merge_base_exit_code": 101,
            "notes": "Fails on both commits. Low likelihood and impact; one session can add a bound.",
            "severity": "low",
            "fix_size": "single"
        }
    }]);
    scenario.agent_does(&review_sessions(
        NO_FINDINGS,
        &format!(r#"
    {CLEAN_REVIEW}
    printf '%s' '{old}' > "$report"
    printf '%s\n' 'Security review: {{"unaddressed_count":0,"findings":[],"pre_existing_count":1}}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#),
    ));

    let result = scenario.run(&["review", &url, "security-review"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let records = state["advisories"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["state"], "draft");
    assert_eq!(records[0]["severity"], "low");
    let description = records[0]["description"].as_str().unwrap();
    assert!(description.contains("Private old vulnerability evidence."));
    assert!(description.contains(scenario.launch_git(&["rev-parse", "main"]).trim()));
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    for private in [
        "Old unchecked bound",
        "old-input-bound",
        "Private old vulnerability evidence.",
        "assert_bounded_input();",
    ] {
        assert!(!comment.contains(private), "{comment}");
        assert!(!result.stdout.contains(private), "{}", result.stdout);
        assert!(!result.stderr.contains(private), "{}", result.stderr);
    }
}

#[test]
fn a_copied_review_process_can_run_the_security_review_step() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let shown = scenario.run(&["process", "show", "review-run"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let directory = scenario.path("home/.thirdshift/processes");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("secure-review.toml"), shown.stdout).unwrap();
    scenario.agent_does(&review_sessions(NO_FINDINGS, CLEAN_REVIEW));

    let result = scenario.run(&[
        "process",
        "secure-review",
        "pull-request",
        &url,
        "security-review",
    ]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(comment.contains("### Security"), "{comment}");
    assert!(
        comment.contains("No unaddressed introduced findings."),
        "{comment}"
    );
    assert_eq!(scenario.claude_calls().len(), 2);
}
