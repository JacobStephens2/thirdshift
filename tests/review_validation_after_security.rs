mod support;

use support::Scenario;

const PRIVATE_SENTINEL: &str = "PRIVATE_OLD_EVIDENCE_1704";

/// A later validation session can read the private Security report. Its final
/// message is public comment text, so the sentinel must not be published.
#[test]
fn validation_does_not_publish_private_security_report_text() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    let report = serde_json::json!([{
        "fingerprint": "old-private-bound",
        "title": "Old private input bound",
        "description": PRIVATE_SENTINEL,
        "proof_of_concept": {
            "test": "assert_private_bound();",
            "command": "cargo test private_bound_marker",
            "head_exit_code": 101,
            "merge_base_exit_code": 101,
            "notes": "Harmless local fixture fails at both commits.",
            "severity": "low",
            "fix_size": "single"
        }
    }]);
    scenario.agent_does(&format!(
        r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q '^Validate the final Review run'; then
    report=$(find "$HOME/.thirdshift/logs" -name pre-existing.json -print -quit)
    test -n "$report"
    {{
        echo 'Validation included'
        cat "$report"
    }} > "$FAKE_CLAUDE_FINAL_MESSAGE"
elif printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
    report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
    test -n "$report"
    cat > "$report" << 'EOF'
{report}
EOF
    printf 'bounded input\n' > security.txt
    git add security.txt
    git commit -qm 'Fix introduced input bound'
    printf '%s\n' 'Security review: {{"unaddressed_count":0,"findings":[],"pre_existing_count":1}}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
else
    printf 'Standards and Spec findings fixed.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
fi
"#,
        report = report
    ));

    let result = scenario.run(&["review", &url, "--security-review"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(
        comment.contains("### Final validation"),
        "validation report missing:\n{comment}"
    );
    assert!(
        comment.contains(
            "Validation results withheld: the report included private Security review material."
        ),
        "private report was not withheld:\n{comment}"
    );
    assert!(
        !comment.contains(PRIVATE_SENTINEL),
        "private security report text reached the pull request comment:\n{comment}"
    );
    assert!(
        !comment.contains("old-private-bound"),
        "private security report text reached the pull request comment:\n{comment}"
    );
}

#[test]
fn extra_validation_covers_the_commit_after_security_fixes() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    scenario.user_config_is(
        r#"[review.commands_by_repo]
'acme/widgets' = ['git rev-parse HEAD >> "$VALIDATION_HEADS"']
"#,
    );
    scenario.agent_does(
        r#"
security=false
if [[ "$FAKE_CLAUDE_PROMPT" == *'guidance mode'* ]]; then
    security=true
    printf 'bounded input\n' > security.txt
    git add security.txt
    git commit -qm 'Fix introduced input bound'
elif test ! -f addressed.txt; then
    printf 'addressed findings\n' > addressed.txt
    git add addressed.txt
    git commit -qm 'Address Standards and Spec findings'
fi

# Execute the configured command whenever the session is given it.
if [[ "$FAKE_CLAUDE_PROMPT" == *'VALIDATION_HEADS'* ]]; then
    sh -c 'git rev-parse HEAD >> "$VALIDATION_HEADS"'
fi

if $security; then
    report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
    test -n "$report"
    printf '[]' > "$report"
    printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
else
    printf 'Standards and Spec findings fixed. Validation: git rev-parse HEAD passed (exit 0).' > "$FAKE_CLAUDE_FINAL_MESSAGE"
fi
"#,
    );
    let validation_heads = scenario.path("validation-heads");
    let result = scenario.run_with_env(
        &["review", &url, "--security-review"],
        &[("VALIDATION_HEADS", validation_heads.to_str().unwrap())],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.origin_file("colleague", "security.txt").as_deref(),
        Some("bounded input\n")
    );
    let checked = std::fs::read_to_string(validation_heads).unwrap();
    let pushed = scenario.launch_git(&["ls-remote", "origin", "refs/heads/colleague"]);
    assert_eq!(
        checked.lines().last(),
        pushed.split_whitespace().next(),
        "the final delivered commit did not receive configured validation after Security fixes"
    );
}
