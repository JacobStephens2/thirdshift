//! Process inspection must not acquire a running Review's retained-work authority.

mod support;

use support::Scenario;

#[test]
fn process_check_does_not_remove_a_running_review_checkout() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("feature", "main", &["Feature"]);
    let url = scenario.github_has_pr("feature", "main", "OPEN");
    scenario.agent_waits_for_release("printf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let held = scenario.run_until(&["review", &url, "--recommend-only"], &[], "started");
    let before = scenario.launch_git(&["worktree", "list", "--porcelain"]);
    let checkout = before
        .split("\n\n")
        .find(|entry| entry.contains("branch refs/heads/feature"))
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .unwrap()
        .to_string();

    let checked = scenario.run(&["process", "check", "review-run", "pull-request", &url]);
    let exists_after_check = std::path::Path::new(&checkout).exists();
    std::fs::write(scenario.path("release"), "").unwrap();
    let result = held.finish();

    assert!(
        exists_after_check,
        "process check removed active checkout {checkout}; check: {}; active Review: {}",
        checked.stderr, result.stderr
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
}
