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
        scenario.set_origin_url(origin);
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
fn a_missing_git_identity_is_rejected_with_a_command_to_set_it() {
    for (key, set_command) in [
        ("user.name", r#"git config --global user.name "Your Name""#),
        (
            "user.email",
            "git config --global user.email you@example.com",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.launch_git(&["config", "--global", "--unset", key]);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        scenario.assert_rejected_before_any_work(&result, &format!("git {key} is not set"));
        scenario.assert_rejected_before_any_work(&result, set_command);
    }
}

#[test]
fn a_git_identity_set_only_in_the_launch_repository_is_enough() {
    let scenario = Scenario::new();
    for (key, value) in [
        ("user.name", "Repo Runner"),
        ("user.email", "repo@example.com"),
    ] {
        scenario.launch_git(&["config", "--global", "--unset", key]);
        scenario.launch_git(&["config", key, value]);
    }
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
}

/// Assert that the Run `args` start on #`issue`, labelled `bug` and
/// `ready-for-agent`, is stopped with `message` before any work, with the
/// issue's labels and the repository's as they were.
fn assert_stopped_with_no_label_changed(
    scenario: &Scenario,
    issue: u32,
    args: &[&str],
    message: &str,
) {
    scenario.issue_labelled(issue, &["bug", "ready-for-agent"]);

    let result = scenario.run(args);

    scenario.assert_rejected_before_any_work(&result, message);
    assert_eq!(
        scenario.issue_labels(issue),
        ["bug", "ready-for-agent"],
        "{message}"
    );
    assert!(scenario.repo_labels().is_empty(), "{message}");
}

#[test]
fn a_run_stopped_by_a_preflight_check_changes_no_label() {
    let scenario = Scenario::new();
    scenario.issue_is(7, "CLOSED");
    assert_stopped_with_no_label_changed(
        &scenario,
        7,
        &[&scenario.issue_url(7)],
        "issue #7 is closed",
    );

    let scenario = Scenario::new();
    scenario.commit_locally("unpushed.txt", "unpushed\n", "Unpushed work");
    assert_stopped_with_no_label_changed(
        &scenario,
        7,
        &[&scenario.issue_url(7)],
        "local main is 1 commit(s) ahead of origin/main",
    );

    let scenario = Scenario::new();
    assert_stopped_with_no_label_changed(
        &scenario,
        7,
        &[&scenario.issue_url(7), "parallel", "2"],
        "parallel is only for a Spec, and #7 has no sub-issues",
    );
}

#[test]
fn a_spec_run_stopped_by_a_preflight_check_changes_no_label() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[])]);
    scenario.issue_is(21, "CLOSED");

    assert_stopped_with_no_label_changed(
        &scenario,
        20,
        &[&scenario.issue_url(20)],
        "every Ticket is closed and there is no Spec branch; nothing to do",
    );
}
