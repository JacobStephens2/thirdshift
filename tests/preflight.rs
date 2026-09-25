//! Pre-flight checks: a Run that makes no sense stops with a clear message and
//! a non-zero exit before any worktree, temp directory or session exists.

mod support;

use support::Scenario;

/// The agent commits its work and opens a PR, so a Run that passes pre-flight
/// succeeds.
const AGENT_COMMITS_AND_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn a_closed_issue_is_rejected() {
    let scenario = Scenario::new();
    scenario.issue_is(7, "CLOSED");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    scenario.assert_rejected_before_any_work(&result, "issue #7 is closed");
}

#[test]
fn a_missing_argument_is_a_usage_error() {
    let scenario = Scenario::new();

    let result = scenario.run(&[]);

    scenario.assert_rejected_before_any_work(&result, "usage: thirdshift <Issue URL>");
}

#[test]
fn an_argument_that_is_not_a_github_issue_url_is_a_usage_error() {
    for url in [
        "7",
        "https://gitlab.com/acme/widgets/issues/7",
        "https://github.example.com/acme/widgets/issues/7",
        "https://github.com/acme/widgets/pull/7",
        "https://github.com/acme/widgets",
        "https://github.com/acme/widgets/issues/seven",
    ] {
        let scenario = Scenario::new();

        let result = scenario.run(&[url]);

        scenario.assert_rejected_before_any_work(&result, "usage: thirdshift <Issue URL>");
        assert!(
            result
                .stderr
                .contains(&format!("not a GitHub issue URL: {url}")),
            "stderr for {url}: {}",
            result.stderr
        );
    }
}

#[test]
fn the_origin_match_accepts_https_and_ssh_with_or_without_git_in_any_case() {
    for origin in [
        "https://github.com/ACME/Widgets.git",
        "https://github.com/acme/widgets",
        "git@github.com:acme/widgets.git",
    ] {
        let scenario = Scenario::new();
        scenario.launch_git(&["config", "remote.origin.url", origin]);
        scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(0), "origin {origin}: {}", result.stderr);
    }
}

#[test]
fn an_issue_in_another_owners_or_repos_repository_is_an_origin_mismatch() {
    for url in [
        "https://github.com/other/widgets/issues/7",
        "https://github.com/acme/gadgets/issues/7",
    ] {
        let scenario = Scenario::new();

        let result = scenario.run(&[url]);

        scenario.assert_rejected_before_any_work(&result, "origin mismatch");
    }
}

#[test]
fn a_detached_head_is_rejected() {
    let scenario = Scenario::new();
    scenario.launch_git(&["checkout", "-q", "--detach"]);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    scenario.assert_rejected_before_any_work(&result, "HEAD is detached");
}

#[test]
fn a_base_branch_missing_on_origin_is_rejected() {
    let scenario = Scenario::new();
    scenario.launch_git(&["checkout", "-q", "-b", "local-only"]);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    scenario.assert_rejected_before_any_work(
        &result,
        "base branch local-only does not exist on origin",
    );
}

#[test]
fn a_local_base_branch_ahead_of_origin_is_rejected() {
    let scenario = Scenario::new();
    scenario.commit_locally("unpushed.txt", "unpushed\n", "Unpushed work");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    scenario
        .assert_rejected_before_any_work(&result, "local main is 1 commit(s) ahead of origin/main");
}

#[test]
fn uncommitted_changes_in_the_launch_directory_are_allowed_and_left_out() {
    let scenario = Scenario::new();
    std::fs::write(scenario.launch_dir().join("README.md"), "my edit\n").unwrap();
    std::fs::write(scenario.launch_dir().join("scratch.txt"), "notes\n").unwrap();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_file("issue-7", "README.md").as_deref(),
        Some("widgets\n")
    );
    assert_eq!(scenario.origin_file("issue-7", "scratch.txt"), None);
    assert_eq!(
        std::fs::read_to_string(scenario.launch_dir().join("README.md")).unwrap(),
        "my edit\n"
    );
}

#[test]
fn a_missing_git_identity_is_rejected_naming_the_missing_key() {
    for key in ["user.name", "user.email"] {
        let scenario = Scenario::new();
        scenario.launch_git(&["config", "--global", "--unset", key]);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        scenario.assert_rejected_before_any_work(&result, &format!("git {key} is not set"));
    }
}
