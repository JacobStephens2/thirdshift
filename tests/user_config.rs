//! The User config, `~/.thirdshift/config.toml`: `merge.always` makes every
//! Run a Merge run unless the command says `no-merge`, `logs.dir` moves the
//! session logs, and a config thirdshift can't use stops the Run before any
//! work.

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
    for config in [
        "[logs]\ndir = \"logs\"\n",
        "[logs]\ndir = \"./logs\"\n",
        "[logs]\ndir = \"~other/logs\"\n",
        "[logs]\ndir = \"\"\n",
        "[logs]\ndir = 3\n",
        "logs = \"/tmp/logs\"\n",
    ] {
        let scenario = Scenario::new();
        let path = scenario.user_config_is(config);
        scenario.agent_does(AGENT_EXITS_3);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{config}: {}", result.stderr);
        scenario.assert_rejected_before_any_work(&result, &path.display().to_string());
        assert!(result.stderr.contains("logs"), "stderr: {}", result.stderr);
        assert!(scenario.gh_calls().is_empty(), "thirdshift called gh");
    }
}
