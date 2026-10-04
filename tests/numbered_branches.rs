//! Numbered Issue branches: once the highest-numbered Issue branch's PR is
//! merged or closed, a Run starts fresh on the next number,
//! `issue-<n>-branch-<k+1>`, from the Base branch.

mod support;

use support::Scenario;

/// The agent commits its work and opens a PR for `branch` against `main`.
fn agent_commits_and_opens_pr_for(branch: &str) -> String {
    format!(
        r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head {branch} --title "Add feature" --body "Closes #7"
"#
    )
}

#[test]
fn the_next_numbered_branch_gets_the_fresh_prompt() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.github_has_pr("issue-7", "main", "MERGED");
    scenario.agent_does(&agent_commits_and_opens_pr_for("issue-7-branch-2"));

    scenario.run(&[&scenario.issue_url(7)]);

    let prompt = scenario.first_prompt();
    assert!(
        prompt.contains(
            "\nPush branch issue-7-branch-2 and create a pull request against main using /thirdshift:pr, marked ready for review.\n"
        ) && !prompt.contains("You are continuing work"),
        "prompt: {prompt}"
    );
}

#[test]
fn a_merged_issue_branch_deleted_from_origin_is_not_reused() {
    let scenario = Scenario::new();
    scenario.github_has_pr("issue-7", "main", "MERGED");
    scenario.agent_does(&agent_commits_and_opens_pr_for("issue-7-branch-2"));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls()[0]["branch"], "issue-7-branch-2");
    assert_eq!(scenario.origin_log("issue-7"), None);
    assert_eq!(
        scenario.origin_log("issue-7-branch-2"),
        Some(vec![
            "Add feature".to_string(),
            "Initial commit".to_string()
        ])
    );
}

#[test]
fn the_pr_comes_from_the_numbered_branch_and_closes_the_issue() {
    let scenario = Scenario::new();
    scenario.github_has_pr("issue-7", "main", "MERGED");
    scenario.agent_does(&agent_commits_and_opens_pr_for("issue-7-branch-2"));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let prs = scenario.gh_state()["prs"].as_array().unwrap().clone();
    let pr = prs.last().unwrap();
    assert_eq!(pr["head"], "issue-7-branch-2");
    assert_eq!(pr["base"], "main");
    assert_eq!(pr["state"], "OPEN");
    assert!(
        pr["body"].as_str().unwrap().contains("Closes #7"),
        "pr: {pr}"
    );
}
