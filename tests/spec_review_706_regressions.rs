mod support;

use serde_json::json;
use support::Scenario;

#[test]
fn security_requested_review_reports_findings_when_the_author_moves_the_head() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    let release = scenario.path("release");
    scenario.agent_does(&format!(
        r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
    report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
    test -n "$report"
    printf '[]' > "$report"
    printf '%s\n' 'Security review: {{"unaddressed_count":0,"findings":[],"pre_existing_count":0}}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
else
    echo 'proposed fix' > fix.txt
    git add fix.txt
    git commit -qm 'Fix export'
    touch '{}'
    for _ in $(seq 2400); do test -f '{}' && break; sleep 0.05; done
    test -f '{}'
    printf 'Fixed export; no Unaddressed findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
fi
"#,
        scenario.path("started").display(), release.display(), release.display()
    ));
    let held = scenario.run_until(&["review", &url, "--security-review"], &[], "started");
    scenario.origin_has_commit(
        "colleague",
        "later.txt",
        "Author work",
        "Later author commit",
    );
    std::fs::write(release, "").unwrap();
    let result = held.finish();
    assert!(scenario.origin_file("colleague", "fix.txt").is_none());
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap_or("");
    assert!(
        comment.contains("branch moved") && comment.contains("Fixed export"),
        "promised moved-head findings were not posted; state={state}; stderr={}",
        result.stderr
    );
}

#[test]
fn upper_stack_comment_warns_when_the_base_received_review_fixes() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("base", "main", &["Base feature"]);
    scenario.origin_has_branch("child", "base", &["Child feature"]);
    scenario.github_has_pr("child", "base", "OPEN");
    scenario.github_has_pr("base", "main", "OPEN");
    let original = scenario.launch_git(&["ls-remote", "origin", "refs/heads/base"]);
    scenario.origin_has_commit("base", "base-fix.txt", "fixed\n", "Address base finding");
    let updated = scenario.launch_git(&["ls-remote", "origin", "refs/heads/base"]);
    let mut state = scenario.gh_state();
    state["prs"][1]["author"] = "another-author".into();
    state["prs"][1]["comments"] = json!([{
        "body": format!("<!-- thirdshift-review {} -->", json!({
            "reviewed_head": original.split_whitespace().next().unwrap(),
            "pushed_head": updated.split_whitespace().next().unwrap()
        })),
        "author_association": "COLLABORATOR", "user": {"login": "runner"}
    }]);
    scenario.write_gh_state(&state);
    scenario.agent_does("printf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let result = scenario.run(&["review", "--author", "colleague", "--recommend-only"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    assert!(
        comment.contains("base") && comment.contains("updat") && comment.contains("merge"),
        "upper PR omitted the base-fixes/update-before-merge warning: {comment}"
    );
    assert!(scenario.origin_file("child", "base-fix.txt").is_none());
}

#[test]
fn author_pass_ignores_a_review_marker_from_a_different_trusted_account() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("feature", "main", &["Feature"]);
    scenario.github_has_pr("feature", "main", "OPEN");
    scenario.agent_does("printf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let remote = scenario.launch_git(&["ls-remote", "origin", "refs/heads/feature"]);
    let head = remote.split_whitespace().next().unwrap();
    let mut state = scenario.gh_state();
    state["prs"][0]["comments"] = json!([{
        "body": format!("<!-- thirdshift-review {} -->", json!({"reviewed_head": head})),
        "author_association": "COLLABORATOR", "user": {"login": "colleague"}
    }]);
    scenario.write_gh_state(&state);
    let result = scenario.run(&["review", "--author", "colleague", "--recommend-only"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "a different collaborator's marker suppressed the authenticated review account; {}",
        result.stderr
    );
}

#[test]
fn updated_upper_stack_does_not_warn_that_base_fixes_are_missing() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("base", "main", &["Base feature"]);
    let original = scenario.launch_git(&["ls-remote", "origin", "refs/heads/base"]);
    scenario.origin_has_commit("base", "base-fix.txt", "fixed\n", "Address base finding");
    let updated = scenario.launch_git(&["ls-remote", "origin", "refs/heads/base"]);
    scenario.origin_has_branch("child", "base", &["Child feature"]);
    scenario.github_has_pr("child", "base", "OPEN");
    scenario.github_has_pr("base", "main", "OPEN");
    let mut state = scenario.gh_state();
    state["prs"][1]["author"] = "another-author".into();
    state["prs"][1]["comments"] = json!([{
        "body": format!("<!-- thirdshift-review {} -->", json!({
            "reviewed_head": original.split_whitespace().next().unwrap(),
            "pushed_head": updated.split_whitespace().next().unwrap()
        })),
        "author_association": "COLLABORATOR", "user": {"login": "runner"}
    }]);
    scenario.write_gh_state(&state);
    scenario.agent_does("printf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let result = scenario.run(&["review", "--author", "colleague", "--recommend-only"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert!(
        !comment.contains("needs updating before merge"),
        "{comment}"
    );
}
