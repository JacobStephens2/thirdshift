mod support;

use support::Scenario;

#[test]
fn failed_fix_keeps_an_uncommitted_conflict_resolution() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    scenario.origin_has_commit("colleague", "shared.txt", "feature", "Feature content");
    scenario.origin_has_commit("main", "shared.txt", "base", "Base content");
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    scenario.agent_does("echo resolved > shared.txt\ngit add shared.txt\nexit 1\n");
    let result = scenario.run(&["fix", &url, "--no-email"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1, "{}", result.stderr);
    assert!(
        scenario
            .first_prompt()
            .contains("/thirdshift-resolving-merge-conflicts")
    );
    assert_eq!(
        scenario.origin_file("colleague", "shared.txt").as_deref(),
        Some("resolved\n"),
        "the failed Fix run discarded the Repair's uncommitted resolution: {}",
        result.stderr
    );
    assert!(
        scenario.origin_log("colleague").unwrap()[0].starts_with("thirdshift: failed fix run (")
    );
}

#[test]
fn fix_self_merge_deletes_its_head_branch_from_origin() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    let result = scenario.run(&["fix", &url, "--merge", "--no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert!(
        scenario.origin_log("colleague").is_none(),
        "the Self-merge left its head branch on origin: {}",
        result.stderr
    );
}
