//! Continuation (ADR-0002): an Issue branch already on origin, with no PR or an
//! open one, is picked up where it stopped instead of started over.

mod support;

use support::Scenario;

/// The agent commits on top of whatever the branch already has and leaves
/// the PR alone: it already exists, or the test doesn't need one.
const AGENT_COMMITS: &str = r#"
echo "more" > more.txt
git add more.txt
git commit -q -m "Continue the work"
"#;

/// The agent commits and opens a PR for `issue-7` against `main`.
const AGENT_COMMITS_AND_OPENS_PR: &str = r#"
echo "more" > more.txt
git add more.txt
git commit -q -m "Continue the work"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn an_issue_branch_with_no_pr_is_continued() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.claude_calls()[0]["branch"], "issue-7");
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Continue the work".to_string(),
            "Earlier work".to_string(),
            "Initial commit".to_string(),
        ])
    );
}

#[test]
fn an_issue_branch_with_an_open_pr_is_continued_and_that_pr_is_the_result() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    let pr = scenario.github_has_pr("issue-7", "main", "OPEN");
    scenario.agent_does(AGENT_COMMITS);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{pr}\n"));
    assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Continue the work".to_string(),
            "Earlier work".to_string(),
            "Initial commit".to_string(),
        ])
    );
}

#[test]
fn the_open_prs_base_is_the_base_branch_whatever_is_checked_out() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("develop", "main", &["Develop work"]);
    scenario.launch_checks_out("develop");
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    let pr = scenario.github_has_pr("issue-7", "main", "OPEN");
    scenario.agent_does(AGENT_COMMITS);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{pr}\n"));
    let prompt = scenario.first_prompt();
    assert!(
        prompt.contains("(see git log main..HEAD)")
            && prompt.contains("The base branch is main. Review with the `thirdshift-code-review` skill using main as the fixed point.")
            && !prompt.contains("develop"),
        "prompt: {prompt}"
    );
    assert!(
        result
            .stderr
            .contains("the Base branch is main, not the checked-out develop"),
        "stderr: {}",
        result.stderr
    );
}

/// Nothing was created: no session, worktree, temp directory or log, and the
/// local Issue branch is still there.
fn assert_nothing_created(scenario: &Scenario, local_branch: &str) {
    assert!(scenario.claude_calls().is_empty(), "claude was run");
    assert_eq!(scenario.entries("work"), vec!["widgets"]);
    assert_eq!(scenario.entries("tmp"), Vec::<String>::new());
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/sessions")
            .exists(),
        "a session log was created"
    );
    assert_ne!(scenario.launch_git(&["branch", "--list", local_branch]), "");
}

#[test]
fn a_local_issue_branch_that_differs_from_origin_stops_the_run() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.launch_checks_out("issue-7");
    scenario.launch_git(&["commit", "-q", "--allow-empty", "-m", "Local only"]);
    scenario.launch_checks_out("main");
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert_eq!(result.stdout, "");
    assert!(
        result
            .stderr
            .contains("issue-7 differs from origin/issue-7"),
        "stderr: {}",
        result.stderr
    );
    assert_nothing_created(&scenario, "issue-7");
    assert_eq!(
        scenario.launch_git(&["log", "-1", "--format=%s", "issue-7"]),
        "Local only\n"
    );
    assert_eq!(scenario.origin_log("issue-7").map(|log| log.len()), Some(2));
}

#[test]
fn a_failed_runs_branch_is_continued_from_its_failure_commit() {
    let scenario = Scenario::new();
    scenario.origin_has_branch(
        "issue-7",
        "main",
        &["Earlier work", "thirdshift: failed run (claude exited 1)"],
    );
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Continue the work".to_string(),
            "thirdshift: failed run (claude exited 1)".to_string(),
            "Earlier work".to_string(),
            "Initial commit".to_string(),
        ])
    );
}

#[test]
fn selection_asks_github_once_for_the_pr_history() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.github_has_pr("issue-7", "main", "OPEN");
    scenario.agent_does(AGENT_COMMITS);

    scenario.run(&[&scenario.issue_url(7)]);

    // Pre-flight's `issue view` comes first, then the `api graphql` that finds
    // no sub-issues; what comes after it is selection, up to the Claim, which
    // starts at the next `issue view`.
    let subcommands: Vec<String> = scenario
        .gh_calls()
        .iter()
        .map(|argv| argv[..2].join(" "))
        .enumerate()
        .take_while(|(at, subcommand)| *at == 0 || subcommand != "issue view")
        .map(|(_, subcommand)| subcommand)
        .collect();
    assert_eq!(subcommands, vec!["issue view", "api graphql", "pr list"]);
    assert!(
        scenario.gh_calls()[2]
            .windows(2)
            .any(|w| w == ["--state", "all"]),
        "gh calls: {:?}",
        scenario.gh_calls()
    );
}

#[test]
fn an_open_pr_supplies_the_base_branch_even_on_a_detached_head() {
    let scenario = Scenario::new();
    scenario.launch_git(&["checkout", "-q", "--detach"]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    let pr = scenario.github_has_pr("issue-7", "main", "OPEN");
    scenario.agent_does(AGENT_COMMITS);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{pr}\n"));
    assert!(
        scenario.first_prompt().contains("The base branch is main."),
        "prompt: {}",
        scenario.first_prompt()
    );
}
