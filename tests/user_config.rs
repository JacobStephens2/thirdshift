//! The User config, `~/.thirdshift/config.toml`: `merge.always` makes every
//! Run a Merge run unless the command says `no-merge`, `base.fix` lets every
//! Run start a Base fix unless the command says `no-base-fix`, `logs.dir`
//! moves the session logs, and a config thirdshift can't use stops the Run
//! before any work.

mod support;

use support::{RunResult, Scenario};

/// The agent commits its work and opens a PR that closes issue #7, into
/// `main`.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

/// Assert the Run ended with PR #1 merged, as a Merge run does.
fn assert_merged(scenario: &Scenario, result: &RunResult) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is merged").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
}

/// Assert the Run ended with PR #1 open and ready for review, never merged.
fn assert_ready_for_review(scenario: &Scenario, result: &RunResult) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is ready for review").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "OPEN");
    assert!(
        !scenario
            .gh_calls()
            .iter()
            .any(|call| call.starts_with(&["pr".to_string(), "merge".to_string()])),
        "thirdshift ran gh pr merge"
    );
}

#[test]
fn merge_always_makes_a_run_without_the_merge_word_a_merge_run() {
    let scenario = Scenario::new();
    scenario.user_config_is("[merge]\nalways = true\n");
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_merged(&scenario, &result);
}

#[test]
fn no_merge_overrides_merge_always_for_one_run() {
    for flag in ["no-merge", "--no-merge"] {
        for flag_first in [true, false] {
            let scenario = Scenario::new();
            scenario.user_config_is("[merge]\nalways = true\n");
            scenario.agent_does(AGENT_OPENS_PR);
            let url = scenario.issue_url(7);
            let args = if flag_first {
                [flag, url.as_str()]
            } else {
                [url.as_str(), flag]
            };

            let result = scenario.run(&args);

            assert_ready_for_review(&scenario, &result);
        }
    }
}

#[test]
fn without_a_config_file_a_run_is_not_a_merge_run() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ready_for_review(&scenario, &result);
}

#[test]
fn a_config_that_says_nothing_about_merging_leaves_a_run_not_a_merge_run() {
    for config in [
        "",
        "# nothing set yet\n",
        "[merge]\n",
        "[merge]\nalways = false\n",
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(config);
        scenario.agent_does(AGENT_OPENS_PR);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_ready_for_review(&scenario, &result);
    }
}

#[test]
fn the_merge_word_goes_before_or_after_the_issue_url() {
    for merge in ["merge", "--merge"] {
        let scenario = Scenario::new();
        scenario.agent_does(AGENT_OPENS_PR);

        let result = scenario.run(&[&scenario.issue_url(7), merge]);

        assert_merged(&scenario, &result);
    }
}

#[test]
fn an_unparsable_config_stops_the_run_naming_the_file() {
    let scenario = Scenario::new();
    let path = scenario.user_config_is("[merge\nalways = true\n");
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    scenario.assert_rejected_before_any_work(&result, &path.display().to_string());
    assert!(scenario.gh_calls().is_empty(), "thirdshift called gh");
}

#[test]
fn an_unknown_key_or_section_stops_the_run_naming_it_and_the_file() {
    for (config, named) in [
        ("[merge]\nalway = true\n", "merge.alway"),
        ("[merges]\nalways = true\n", "[merges]"),
        ("colour = true\n", "colour"),
        ("[spec]\nparalel = 2\n", "spec.paralel"),
    ] {
        let scenario = Scenario::new();
        let path = scenario.user_config_is(config);
        scenario.agent_does(AGENT_OPENS_PR);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{config}: {}", result.stderr);
        scenario.assert_rejected_before_any_work(&result, &path.display().to_string());
        assert!(
            result.stderr.contains(named),
            "expected {named:?} in stderr for {config:?}: {}",
            result.stderr
        );
    }
}

#[test]
fn a_spec_parallel_that_is_not_a_whole_number_from_1_up_stops_the_run() {
    for config in [
        "[spec]\nparallel = 0\n",
        "[spec]\nparallel = -2\n",
        "[spec]\nparallel = 1.5\n",
        "[spec]\nparallel = \"3\"\n",
    ] {
        let scenario = Scenario::new();
        let path = scenario.user_config_is(config);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{config}: {}", result.stderr);
        scenario.assert_rejected_before_any_work(&result, &path.display().to_string());
        assert!(
            result
                .stderr
                .contains("spec.parallel must be a whole number from 1 up"),
            "stderr: {}",
            result.stderr
        );
    }
}

#[test]
fn a_value_of_the_wrong_type_stops_the_run_naming_the_key() {
    for config in ["merge = true\n", "[merge]\nalways = \"yes\"\n"] {
        let scenario = Scenario::new();
        let path = scenario.user_config_is(config);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{config}: {}", result.stderr);
        scenario.assert_rejected_before_any_work(&result, &path.display().to_string());
        assert!(result.stderr.contains("merge"), "stderr: {}", result.stderr);
    }
}

#[test]
fn argument_errors_come_before_the_config_is_read() {
    let scenario = Scenario::new();
    scenario.user_config_is("[merge\n");

    let result = scenario.run(&["merge", "no-merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(2), "stderr: {}", result.stderr);
}

#[test]
fn help_and_version_succeed_with_a_broken_config() {
    for config in ["[merge\n", "[merge]\nalway = true\n"] {
        let scenario = Scenario::new();
        scenario.user_config_is(config);

        for command in ["help", "version"] {
            let result = scenario.run(&[command]);

            assert_eq!(result.code, Some(0), "{command}: {}", result.stderr);
            assert_eq!(result.stderr, "", "{command}");
        }
    }
}

/// The agent commits nothing and exits 3, so the Run fails after one session.
const AGENT_EXITS_3: &str = "exit 3\n";

/// Assert the Run failed after logging its one session in `dir`, a directory
/// under the scenario, and that the "session log:" line names that log.
fn assert_logged_in(scenario: &Scenario, result: &RunResult, dir: &str) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let logs = scenario.entries(dir);
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    let log = scenario.path(dir).join(&logs[0]);
    assert!(
        result
            .stderr
            .contains(&format!("session log: {}", log.display())),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn without_logs_dir_session_logs_go_to_the_default_directory() {
    for config in [None, Some("[merge]\nalways = false\n"), Some("[logs]\n")] {
        let scenario = Scenario::new();
        if let Some(config) = config {
            scenario.user_config_is(config);
        }
        scenario.agent_does(AGENT_EXITS_3);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_logged_in(&scenario, &result, "home/.thirdshift/logs");
    }
}

#[test]
fn a_logs_dir_under_tilde_is_under_home_and_created_if_missing() {
    let scenario = Scenario::new();
    scenario.user_config_is("[logs]\ndir = \"~/elsewhere/logs\"\n");
    scenario.agent_does(AGENT_EXITS_3);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_logged_in(&scenario, &result, "home/elsewhere/logs");
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn an_absolute_logs_dir_is_used_as_is() {
    let scenario = Scenario::new();
    let dir = scenario.path("somewhere/logs");
    scenario.user_config_is(&format!("[logs]\ndir = \"{}\"\n", dir.display()));
    scenario.agent_does(AGENT_EXITS_3);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_logged_in(&scenario, &result, "somewhere/logs");
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn a_relative_or_mistyped_logs_dir_stops_the_run_naming_the_setting() {
    for (config, named) in [
        (
            "[logs]\ndir = \"logs\"\n",
            "logs.dir must be an absolute path",
        ),
        (
            "[logs]\ndir = \"./logs\"\n",
            "logs.dir must be an absolute path",
        ),
        (
            "[logs]\ndir = \"~other/logs\"\n",
            "logs.dir must be an absolute path",
        ),
        ("[logs]\ndir = \"\"\n", "logs.dir must be an absolute path"),
        ("[logs]\ndir = 3\n", "logs.dir must be a string"),
        ("logs = \"/tmp/logs\"\n", "logs must be the section [logs]"),
    ] {
        let scenario = Scenario::new();
        let path = scenario.user_config_is(config);
        scenario.agent_does(AGENT_EXITS_3);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{config}: {}", result.stderr);
        scenario.assert_rejected_before_any_work(&result, &path.display().to_string());
        assert!(
            result.stderr.contains(named),
            "expected {named:?} in stderr for {config:?}: {}",
            result.stderr
        );
        assert!(scenario.gh_calls().is_empty(), "thirdshift called gh");
    }
}

/// The agent adds feature.txt and opens the PR for #7, with `test` red on its
/// head and on `main`'s tip: an Inherited failure.
const RUN_OPENS_PR_WITH_INHERITED_FAILURE: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
gh fake checks "$(git rev-parse HEAD)" '[{"name": "test", "conclusion": "failure"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name": "test", "conclusion": "failure"}]'
"#;

/// The agent, on the Base fix issue #8, commits a fix and opens its PR into
/// `main`, with `test` green on its head.
const BASE_FIX_OPENS_PR: &str = r#"
echo "fixed" > ci-fix.txt
git add ci-fix.txt
git commit -q -m "Fix CI on main"
gh pr create --base main --head issue-8 --title "Fix CI on main" --body "Closes #8"
gh fake checks "$(git rev-parse HEAD)" '[{"name": "test", "conclusion": "success"}]'
"#;

/// Assert the Run on #7 started a Base fix on #8 that merged, and ended with
/// its own PR ready for review.
fn assert_base_fix_merged(scenario: &Scenario, result: &RunResult) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let gh = scenario.gh_state();
    assert_eq!(gh["titles"]["8"], "CI red on main: test");
    assert_eq!(gh["prs"][1]["base"], "main");
    assert_eq!(gh["prs"][1]["state"], "MERGED");
    assert_eq!(gh["prs"][0]["state"], "OPEN");
    assert_eq!(gh["prs"][0]["isDraft"], false);
}

/// Assert the Run on #7 failed on its Inherited failure, with no Base fix
/// issue written.
fn assert_failed_with_no_base_fix(scenario: &Scenario, result: &RunResult) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result
            .stderr
            .contains("CI red on test, which also fails on main at ")
            && result.stderr.contains("; fix main first"),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.gh_calls_of("issue", "create").is_empty());
    assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
}

#[test]
fn base_fix_lets_a_run_without_the_base_fix_word_start_a_base_fix() {
    let scenario = Scenario::new();
    scenario.user_config_is("[base]\nfix = true\n");
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for(8, BASE_FIX_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_base_fix_merged(&scenario, &result);
}

#[test]
fn no_base_fix_overrides_base_fix_for_one_run() {
    for flag in ["no-base-fix", "--no-base-fix"] {
        for flag_first in [true, false] {
            let scenario = Scenario::new();
            scenario.user_config_is("[base]\nfix = true\n");
            scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
            let url = scenario.issue_url(7);
            let args = if flag_first {
                [flag, url.as_str()]
            } else {
                [url.as_str(), flag]
            };

            let result = scenario.run(&args);

            assert_failed_with_no_base_fix(&scenario, &result);
        }
    }
}

#[test]
fn a_base_fix_that_is_not_true_or_false_stops_the_run_naming_the_file() {
    for (config, named) in [
        ("[base]\nfix = \"yes\"\n", "base.fix must be true or false"),
        ("[base]\nfix = 1\n", "base.fix must be true or false"),
        ("base = true\n", "base must be the section [base]"),
        ("[base]\nfixes = true\n", "unknown key base.fixes"),
    ] {
        let scenario = Scenario::new();
        let path = scenario.user_config_is(config);
        scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{config}: {}", result.stderr);
        scenario.assert_rejected_before_any_work(&result, &path.display().to_string());
        assert!(
            result.stderr.contains(named),
            "expected {named:?} in stderr for {config:?}: {}",
            result.stderr
        );
    }
}

#[test]
fn a_config_that_says_nothing_about_base_fixes_leaves_an_inherited_failure_to_fail_the_run() {
    for config in ["", "[base]\n", "[base]\nfix = false\n"] {
        let scenario = Scenario::new();
        scenario.user_config_is(config);
        scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_failed_with_no_base_fix(&scenario, &result);
    }
}

#[test]
fn base_fix_and_no_base_fix_together_are_an_argument_error_before_the_config_is_read() {
    let scenario = Scenario::new();
    scenario.user_config_is("[base\n");

    let result = scenario.run(&["base-fix", "no-base-fix", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(2), "stderr: {}", result.stderr);
    assert!(scenario.claude_calls().is_empty(), "the Run started");
}
