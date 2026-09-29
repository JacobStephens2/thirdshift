//! `launch.pull` in the User config: a Run brings the Launch directory's
//! checkout of the Base branch up to date with origin, fast-forward only,
//! and a failure to do so is a warning, never a Failed run.

mod support;

use support::Scenario;

/// The agent commits its work and opens a PR that closes issue #7, into
/// `main`.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

const PULL_ON: &str = "[launch]\npull = true\n";

/// A scenario whose origin `main` is one commit, touching README.md, ahead of
/// the Launch directory's `main`.
fn origin_ahead() -> Scenario {
    let scenario = Scenario::new();
    scenario.origin_has_commit("main", "README.md", "widgets, updated\n", "Upstream work");
    scenario
}

fn launch_head(scenario: &Scenario, branch: &str) -> String {
    scenario.launch_git(&["rev-parse", &format!("refs/heads/{branch}")])
}

fn origin_head(scenario: &Scenario, branch: &str) -> String {
    scenario.origin_git(&["rev-parse", &format!("refs/heads/{branch}")])
}

#[test]
fn pull_brings_the_launch_directorys_base_branch_up_to_date() {
    let scenario = origin_ahead();
    scenario.user_config_is(PULL_ON);
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        launch_head(&scenario, "main"),
        origin_head(&scenario, "main")
    );
    assert_eq!(
        std::fs::read_to_string(scenario.launch_dir().join("README.md")).unwrap(),
        "widgets, updated\n"
    );
    assert!(
        result
            .stderr
            .contains("thirdshift: updating main in the Launch directory from origin/main"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("warning"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn without_pull_the_launch_directory_is_left_alone() {
    for config in [None, Some("[launch]\npull = false\n"), Some("[launch]\n")] {
        let scenario = origin_ahead();
        if let Some(config) = config {
            scenario.user_config_is(config);
        }
        scenario.agent_does(AGENT_OPENS_PR);
        let before = launch_head(&scenario, "main");

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(0), "{config:?}: {}", result.stderr);
        assert_eq!(launch_head(&scenario, "main"), before, "{config:?}");
        assert_ne!(before, origin_head(&scenario, "main"));
        assert!(
            !result.stderr.contains("Launch directory"),
            "{config:?}: {}",
            result.stderr
        );
    }
}

#[test]
fn an_up_to_date_launch_directory_is_a_quiet_no_op() {
    let scenario = Scenario::new();
    scenario.user_config_is(PULL_ON);
    scenario.agent_does(AGENT_OPENS_PR);
    let before = launch_head(&scenario, "main");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(launch_head(&scenario, "main"), before);
    assert!(
        !result.stderr.contains("Launch directory") && !result.stderr.contains("warning"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_blocked_update_is_a_warning_and_keeps_uncommitted_changes() {
    let scenario = origin_ahead();
    scenario.user_config_is(PULL_ON);
    scenario.agent_does(AGENT_OPENS_PR);
    std::fs::write(scenario.launch_dir().join("README.md"), "my edit\n").unwrap();
    let before = launch_head(&scenario, "main");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is ready for review").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains(
            "thirdshift: warning: could not update main in the Launch directory, \
             so update it by hand: git pull --ff-only origin main"
        ),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("README.md"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(launch_head(&scenario, "main"), before);
    assert_eq!(
        std::fs::read_to_string(scenario.launch_dir().join("README.md")).unwrap(),
        "my edit\n"
    );
}

#[test]
fn a_continuation_based_elsewhere_leaves_the_checked_out_branch_alone() {
    let scenario = Scenario::new();
    scenario.user_config_is(PULL_ON);
    scenario.origin_has_branch("develop", "main", &["Develop work"]);
    scenario.launch_checks_out("develop");
    scenario.origin_has_commit("develop", "develop.txt", "more\n", "More develop work");
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    let pr = scenario.github_has_pr("issue-7", "main", "OPEN");
    scenario.agent_does("git commit -q --allow-empty -m 'Continue the work'");
    let before = launch_head(&scenario, "develop");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{pr}\n"));
    assert_eq!(launch_head(&scenario, "develop"), before);
    assert_eq!(
        scenario.launch_git(&["branch", "--show-current"]),
        "develop\n"
    );
    assert!(
        !result.stderr.contains("Launch directory"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_run_stopped_by_pre_flight_leaves_the_launch_directory_alone() {
    let scenario = origin_ahead();
    scenario.user_config_is(PULL_ON);
    scenario.issue_is(7, "CLOSED");
    let before = launch_head(&scenario, "main");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    scenario.assert_rejected_before_any_work(&result, "issue #7 is closed");
    assert_eq!(launch_head(&scenario, "main"), before);
}

#[test]
fn an_origin_mismatch_leaves_the_launch_directory_alone() {
    let scenario = origin_ahead();
    scenario.user_config_is(PULL_ON);
    let before = launch_head(&scenario, "main");

    let result = scenario.run(&["https://github.com/acme/gadgets/issues/7"]);

    scenario.assert_rejected_before_any_work(&result, "origin mismatch");
    assert_eq!(launch_head(&scenario, "main"), before);
}

#[test]
fn a_launch_setting_thirdshift_cant_use_stops_the_run_naming_it_and_the_file() {
    for (config, named) in [
        ("[launch]\npul = true\n", "launch.pul"),
        (
            "[launch]\npull = \"yes\"\n",
            "launch.pull must be true or false",
        ),
        ("launch = true\n", "launch must be the section [launch]"),
    ] {
        let scenario = origin_ahead();
        let path = scenario.user_config_is(config);
        let before = launch_head(&scenario, "main");

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{config}: {}", result.stderr);
        scenario.assert_rejected_before_any_work(&result, &path.display().to_string());
        assert!(
            result.stderr.contains(named),
            "expected {named:?} in stderr for {config:?}: {}",
            result.stderr
        );
        assert!(scenario.gh_calls().is_empty(), "thirdshift called gh");
        assert_eq!(launch_head(&scenario, "main"), before);
    }
}
