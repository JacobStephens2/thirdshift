//! A fresh run on the happy path: no Issue branch exists yet, the agent
//! implements the issue and opens a PR, and thirdshift pushes, checks the PR,
//! prints its URL and cleans up.

mod support;

use support::Scenario;

/// The agent commits its work and opens a PR, but leaves the pushing to
/// thirdshift.
const AGENT_COMMITS_AND_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn prints_only_the_pr_url_and_exits_zero() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
}

#[test]
fn runs_claude_headless_in_auto_mode_in_a_sibling_worktree_on_the_issue_branch() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    let argv: Vec<&str> = call["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert!(argv.contains(&"-p"), "argv: {argv:?}");
    assert!(
        argv.windows(2).any(|w| w == ["--permission-mode", "auto"]),
        "argv: {argv:?}"
    );
    assert!(
        argv.windows(2)
            .any(|w| w == ["--output-format", "stream-json"]),
        "argv: {argv:?}"
    );
    assert!(argv.contains(&"--verbose"), "argv: {argv:?}");
    assert!(argv.contains(&"--plugin-dir"), "argv: {argv:?}");
    assert_eq!(
        call["cwd"],
        scenario.path("work/widgets-issue-7").to_str().unwrap()
    );
    assert_eq!(call["branch"], "issue-7");
}

#[test]
fn gives_the_agent_the_fresh_prompt() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(
        scenario.claude_calls()[0]["prompt"],
        "/thirdshift:implement https://github.com/acme/widgets/issues/7\n\
         The base branch is main. Review with /thirdshift:code-review using main as the fixed point.\n\
         Address the Standards and Spec findings you agree with.\n\
         Push branch issue-7 and create a pull request against main using /thirdshift:pr, marked ready for review.\n\
         In the PR body, add an \"Unaddressed findings\" section listing each skipped finding under Standards or Spec, with at least a one-line reason.\n\
         Include \"Closes #7\" in the PR body.\n"
    );
}

#[test]
fn loads_every_factory_skill_as_the_thirdshift_plugin() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    let files = &scenario.claude_calls()[0]["plugin_files"];
    let manifest: serde_json::Value = serde_json::from_str(
        files[".claude-plugin/plugin.json"]
            .as_str()
            .expect("no plugin manifest"),
    )
    .unwrap();
    assert_eq!(manifest["name"], "thirdshift");
    for skill in [
        "implement",
        "code-review",
        "pr",
        "tdd",
        "resolving-merge-conflicts",
    ] {
        let skill_md = files[format!("skills/{skill}/SKILL.md")].as_str();
        assert!(
            skill_md.is_some_and(|text| text.contains(&format!("name: {skill}"))),
            "skill {skill} missing from the plugin"
        );
    }
    assert!(
        files["skills/tdd/tests.md"].is_string(),
        "a skill's supporting files are missing"
    );
}

#[test]
fn ships_the_mattpocock_skills_mit_notice_with_the_factory_skills() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    let calls = scenario.claude_calls();
    let license = calls[0]["plugin_files"]["skills/LICENSE"].as_str();
    assert!(
        license
            .is_some_and(|text| text.contains("MIT License")
                && text.contains("Copyright (c) 2026 Matt Pocock")),
        "license: {license:?}"
    );
}

#[test]
fn logs_the_session_stream_under_the_home_directory() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    let logs = scenario.entries("home/.thirdshift/logs");
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    let name = &logs[0];
    assert!(
        name.starts_with("acme-widgets-issue-7-") && name.ends_with("-implement.jsonl"),
        "log: {name}"
    );
    let log = std::fs::read_to_string(scenario.path("home/.thirdshift/logs").join(name)).unwrap();
    assert!(
        log.contains(r#""type": "system""#) && log.contains(r#""type": "result""#),
        "log: {log}"
    );
}

#[test]
fn pushes_the_issue_branch_when_the_agent_did_not() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Add feature".to_string(),
            "Initial commit".to_string()
        ])
    );
}

#[test]
fn succeeds_when_the_agent_pushed_the_branch_itself() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_COMMITS_AND_OPENS_PR}\ngit push -q origin issue-7\n"
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.origin_log("issue-7").map(|log| log.len()), Some(2));
}

#[test]
fn leaves_no_worktree_local_issue_branch_or_temp_directory_behind() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn marks_a_draft_pr_ready_and_succeeds() {
    let scenario = Scenario::new();
    scenario.agent_does(
        "echo feature > feature.txt\ngit add feature.txt\ngit commit -q -m 'Add feature'\n\
         gh pr create --draft --base main --head issue-7 --title 'Add feature' --body 'Closes #7'\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
}
