mod support;

use support::Scenario;

#[test]
fn failed_bug_handoff_never_delivers_existing_pr() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug"]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.github_has_pr("issue-7", "main", "OPEN");
    scenario.agent_does(r#"
echo 'Could not reproduce the bug; publishing the triage comment failed with HTTP 403.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&[&scenario.issue_url(7), "--merge"]);

    assert_eq!(
        scenario.gh_state()["prs"][0]["state"],
        "OPEN",
        "{}",
        result.stderr
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
}
