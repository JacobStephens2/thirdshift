//! Whole-Spec integration regressions identified during the #1784 review.
mod support;

use serde_json::json;
use support::Scenario;

#[test]
fn failed_opening_base_repair_is_not_repeated_by_the_next_pass() {
    let scenario = Scenario::new();
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    let mut state = scenario.gh_state();
    state["checks"][tip.trim()] = json!([{
        "name":"external", "conclusion":"failure", "url":"https://ci.example/test"
    }]);
    scenario.write_gh_state(&state);
    scenario.agent_does("echo unfinished > repair.txt\nexit 1\n");

    let first = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(first.code, Some(1), "{}", first.stderr);
    assert_eq!(scenario.claude_calls().len(), 1, "{}", first.stderr);
    let branch = format!("fix-main-{}", &tip[..7]);
    assert_eq!(
        scenario.origin_file(&branch, "repair.txt").as_deref(),
        Some("unfinished\n")
    );

    let second = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "the next Pass repeated the failed Base repair; first:\n{}\nsecond:\n{}",
        first.stderr,
        second.stderr
    );
    assert_eq!(second.code, Some(0), "{}", second.stderr);
    let state = scenario.gh_state();
    assert_eq!(
        state["prs"].as_array().unwrap().len(),
        1,
        "{}",
        first.stderr
    );
    assert_eq!(state["prs"][0]["isDraft"], true);
    let url = state["prs"][0]["url"].as_str().unwrap();
    assert!(
        first
            .stderr
            .contains(&format!("Recovery: thirdshift fix {url}")),
        "{}",
        first.stderr
    );
}

#[test]
fn pending_base_checks_hold_an_unscoped_pass_before_pr_selection() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("waiting", "main", &["Feature"]);
    scenario.github_has_pr("waiting", "main", "OPEN");
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    let head = scenario.origin_git(&["rev-parse", "waiting"]);
    let mut state = scenario.gh_state();
    state["checks"][tip.trim()] = json!([{
        "name":"base-external", "conclusion":"success", "pending_polls":1000
    }]);
    state["checks"][head.trim()] = json!([{"name":"test", "conclusion":"failure"}]);
    scenario.write_gh_state(&state);
    scenario.agent_does("exit 1\n");

    let result = scenario.run(&["fix", "no-base-fix", "no-email"]);
    assert!(
        scenario.claude_calls().is_empty(),
        "unscoped Pass selected PR work before Base CI became green:\n{}",
        result.stderr
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
}

#[test]
fn fix_can_self_merge_a_factory_authored_pr_after_repair_without_review_markers() {
    let scenario = Scenario::new();
    scenario.agent_does(
        "echo feature > feature.txt\ngit add feature.txt\ngit commit -qm 'Implement feature'\ngh pr create --base main --head issue-7 --title 'Feature' --body 'Closes #7'\n",
    );
    let opened = scenario.run(&[&scenario.issue_url(7), "no-merge", "no-email"]);
    assert_eq!(opened.code, Some(0), "{}", opened.stderr);
    let url = scenario.gh_state()["prs"][0]["url"]
        .as_str()
        .unwrap()
        .to_string();
    let head = scenario.origin_git(&["rev-parse", "issue-7"]);
    let mut state = scenario.gh_state();
    state["checks"][head.trim()] = json!([{"name":"test", "conclusion":"failure"}]);
    scenario.write_gh_state(&state);
    scenario.agent_does(
        "echo repair >> feature.txt\ngit add feature.txt\ngit commit -qm 'Repair feature'\n",
    );

    let result = scenario.run(&["fix", &url, "merge", "no-email"]);
    assert_eq!(
        result.code,
        Some(0),
        "factory-authored and repaired PR should honor the requested Self-merge:\n{}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
}

#[test]
fn completed_base_delivery_does_not_skip_the_first_review_pass() {
    let scenario = Scenario::new();
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    let mut state = scenario.gh_state();
    state["checks"][tip.trim()] = json!([{
        "name":"external", "conclusion":"failure", "url":"https://ci.example/test"
    }]);
    scenario.write_gh_state(&state);
    scenario.agent_does(
        "echo repaired > ci-fix.txt\ngit add ci-fix.txt\ngit commit -qm 'Repair Base CI'\n",
    );
    let fixed = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(fixed.code, Some(0), "{}", fixed.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    let mut state = scenario.gh_state();
    state["prs"][0]["author"] = json!("colleague");
    scenario.write_gh_state(&state);
    scenario.agent_does("printf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let reviewed = scenario.run(&[
        "review",
        "--author",
        "colleague",
        "--recommend-only",
        "no-email",
    ]);
    assert_eq!(reviewed.code, Some(0), "{}", reviewed.stderr);
    assert_eq!(
        scenario.claude_calls().len(),
        2,
        "an agent-authored Base CI head still needs its first Review run: {}",
        reviewed.stderr
    );
}
