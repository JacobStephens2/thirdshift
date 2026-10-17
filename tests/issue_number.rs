//! Issue numbers resolve against the Launch directory's GitHub origin.

mod support;

use support::Scenario;

const AGENT_COMMITS_AND_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn a_number_and_a_full_url_deliver_the_same_issue() {
    for input in ["7", "https://github.com/acme/widgets/issues/7"] {
        let scenario = Scenario::new();
        scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

        let result = scenario.run(&[input]);

        assert_eq!(result.code, Some(0), "{input}: {}", result.stderr);
        assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
        assert!(
            scenario.claude_calls()[0]["prompt"]
                .as_str()
                .unwrap()
                .contains("https://github.com/acme/widgets/issues/7")
        );
        assert_eq!(
            scenario.origin_file("issue-7", "feature.txt").as_deref(),
            Some("feature\n")
        );
    }
}

#[test]
fn merge_and_run_options_work_with_a_number_in_either_order() {
    for args in [
        vec![
            "merge",
            "7",
            "harness",
            "claude",
            "no-email",
            "no-base-fix",
            "no-security-review",
            "no-security-fix",
        ],
        vec![
            "--harness",
            "claude",
            "--no-email",
            "--no-base-fix",
            "--no-security-review",
            "--no-security-fix",
            "7",
            "--merge",
        ],
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(0), "{args:?}: {}", result.stderr);
        assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
        assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
        assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
        scenario.assert_cleaned_up("issue-7");
    }
}

#[test]
fn a_number_resolves_supported_origins_from_nested_directories() {
    for (origin, url) in [
        (
            "https://github.com/acme/widgets.git",
            "https://github.com/acme/widgets/issues/7",
        ),
        (
            "https://github.com/acme/widgets",
            "https://github.com/acme/widgets/issues/7",
        ),
        (
            "git@github.com:acme/widgets.git",
            "https://github.com/acme/widgets/issues/7",
        ),
        (
            "git@github.com:acme/widgets",
            "https://github.com/acme/widgets/issues/7",
        ),
        (
            "ssh://git@github.com/acme/widgets.git",
            "https://github.com/acme/widgets/issues/7",
        ),
        (
            "ssh://git@github.com/acme/widgets",
            "https://github.com/acme/widgets/issues/7",
        ),
        (
            "HTTPS://GitHub.com/ACME/Widgets.GIT/",
            "https://github.com/ACME/Widgets/issues/7",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.set_origin_url(origin);
        scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);
        let nested = scenario.launch_dir().join("some/nested/directory");
        std::fs::create_dir_all(&nested).unwrap();

        let result = scenario.run_from(&nested, &["0007"]);

        assert_eq!(result.code, Some(0), "{origin}: {}", result.stderr);
        assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
        assert!(
            scenario.claude_calls()[0]["prompt"]
                .as_str()
                .unwrap()
                .contains(url),
            "{origin}"
        );
    }
}

#[test]
fn invalid_numbers_stop_before_tracker_or_session_activity() {
    for input in [
        "0",
        "000",
        "18446744073709551616",
        "7x",
        "7.0",
        "+7",
        "-7",
        "７",
        "#7",
        "acme/widgets#7",
        " 7",
        "7 ",
    ] {
        let scenario = Scenario::new();

        let result = scenario.run(&[input]);

        assert_eq!(result.code, Some(2), "{input}: {}", result.stderr);
        scenario.assert_rejected_before_any_work(&result, "usage: thirdshift");
        assert!(
            scenario.gh_calls().is_empty(),
            "{input}: tracker was contacted"
        );
        if input.bytes().all(|byte| byte.is_ascii_digit()) {
            assert!(
                result
                    .stderr
                    .contains("issue number must be a positive decimal integer"),
                "{}",
                result.stderr
            );
        }
    }
}

#[test]
fn unresolved_numbers_stop_before_tracker_or_session_activity() {
    for origin in [
        None,
        Some("https://gitlab.com/acme/widgets.git"),
        Some("../origin.git"),
    ] {
        let scenario = Scenario::new();
        match origin {
            Some(origin) => {
                scenario.set_origin_url(origin);
            }
            None => {
                scenario.launch_git(&["remote", "remove", "origin"]);
            }
        }

        let result = scenario.run(&["7"]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        scenario
            .assert_rejected_before_any_work(&result, "cannot resolve issue number 7 from origin");
        assert!(
            result
                .stderr
                .contains("GitHub repository clone with an origin remote"),
            "{}",
            result.stderr
        );
        assert!(scenario.gh_calls().is_empty(), "tracker was contacted");
    }

    let scenario = Scenario::new();
    let result = scenario.run_from(&scenario.path(""), &["7"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    scenario.assert_rejected_before_any_work(&result, "cannot resolve issue number 7 from origin");
    assert!(
        result.stderr.contains("not a git repository"),
        "{}",
        result.stderr
    );
    assert!(scenario.gh_calls().is_empty(), "tracker was contacted");
}
