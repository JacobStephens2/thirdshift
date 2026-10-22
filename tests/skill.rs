//! Standalone skill sessions through the CLI, real Git and fake Harnesses.

mod support;

use std::fs;
use std::path::Path;

use support::{Scenario, leaves_running};

#[test]
fn factory_shorthand_runs_one_session_on_the_issue_and_prints_its_final_message() {
    let scenario = Scenario::new();
    scenario.issue_is(42, "OPEN");
    scenario.agent_does(
        "test -s .thirdshift-issue.md\nprintf 'Spec published.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["skill", "to-spec", "issue", "42"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "Spec published.\n");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1);
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(prompt.contains("thirdshift-to-spec"), "{prompt}");
    assert!(prompt.contains(&scenario.issue_url(42)), "{prompt}");
    scenario.assert_every_session_found_the_factory_skills();
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[test]
fn repository_skills_resolve_by_directory_name_before_factory_shorthand() {
    for folder in [".agents/skills", ".claude/skills"] {
        let scenario = Scenario::new();
        let relative = format!("{folder}/to-spec/SKILL.md");
        fs::create_dir_all(scenario.launch_dir().join(format!("{folder}/to-spec"))).unwrap();
        scenario.commit_locally(
            &relative,
            "# Repository-specific planning\n",
            "Add repository skill",
        );
        fs::write(
            scenario.launch_dir().join(&relative),
            "uncommitted skill changes",
        )
        .unwrap();
        fs::write(
            scenario.launch_dir().join("README.md"),
            "uncommitted checkout changes",
        )
        .unwrap();
        fs::create_dir_all(scenario.launch_dir().join("nested")).unwrap();
        fs::write(scenario.launch_dir().join("nested/local-only"), "local").unwrap();
        let before = scenario.launch_git(&["status", "--porcelain", "--untracked-files=all"]);
        let head = scenario.launch_git(&["rev-parse", "HEAD"]);
        scenario.agent_does(&format!(
            "test \"$(cat {relative})\" = '# Repository-specific planning'\n\
             test \"$(cat README.md)\" = widgets\n\
             test ! -e nested/local-only\n\
             printf 'Local skill completed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\""
        ));

        let result =
            scenario.run_from(&scenario.launch_dir().join("nested"), &["skill", "to-spec"]);

        assert_eq!(result.code, Some(0), "{folder}: {}", result.stderr);
        assert_eq!(result.stdout, "Local skill completed.\n");
        let calls = scenario.claude_calls();
        assert_eq!(calls.len(), 1);
        let prompt = calls[0]["prompt"].as_str().unwrap();
        assert!(prompt.contains(&relative), "{prompt}");
        assert!(!prompt.contains("thirdshift-to-spec"), "{prompt}");
        assert_eq!(
            scenario.launch_git(&["status", "--porcelain", "--untracked-files=all"]),
            before
        );
        assert_eq!(scenario.launch_git(&["rev-parse", "HEAD"]), head);
        assert!(
            !scenario
                .launch_dir()
                .join(".claude/skills/thirdshift-implement")
                .exists()
        );
    }
}

#[test]
fn worktrees_with_uncommitted_changes_or_unpushed_commits_are_retained() {
    for commit in [false, true] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "echo 'skill work' > result.txt\n{}\n\
             printf 'Done.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
            if commit {
                "git add result.txt\ngit commit -qm 'Skill work'"
            } else {
                "true"
            }
        ));

        let result = scenario.run(&["skill", "thirdshift-to-spec"]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(result.stdout, "Done.\n");
        let calls = scenario.claude_calls();
        let path = Path::new(calls[0]["cwd"].as_str().unwrap());
        assert!(path.exists(), "worktree was deleted: {}", result.stderr);
        assert_eq!(
            fs::read_to_string(path.join("result.txt")).unwrap(),
            "skill work\n"
        );
        assert!(
            result.stderr.contains(path.to_str().unwrap()),
            "{}",
            result.stderr
        );
        assert!(
            result.stderr.contains("retaining checkout"),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
    }
}

#[test]
fn an_unknown_skill_lists_factory_and_repository_skills_without_starting_a_session() {
    let scenario = Scenario::new();
    fs::create_dir_all(scenario.launch_dir().join(".agents/skills/local-skill")).unwrap();
    scenario.commit_locally(
        ".agents/skills/local-skill/SKILL.md",
        "Local skill",
        "Add skill",
    );
    scenario.launch_git(&["push", "origin", "main"]);

    let result = scenario.run(&["skill", "unknown-skill"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stdout.is_empty());
    for expected in [
        "unknown skill",
        "available skills:",
        "local-skill",
        "thirdshift-to-spec",
    ] {
        assert!(result.stderr.contains(expected), "{}", result.stderr);
    }
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[test]
fn clean_worktrees_with_published_commits_are_removed() {
    let scenario = Scenario::new();
    scenario.agent_does(
        "echo published > result.txt\n\
         git add result.txt\n\
         git commit -qm 'Skill work'\n\
         git push origin HEAD:refs/heads/skill-result\n\
         printf 'Published.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );

    let result = scenario.run(&["skill", "to-spec"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "Published.\n");
    assert_eq!(
        scenario
            .origin_file("skill-result", "result.txt")
            .as_deref(),
        Some("published\n")
    );
    let calls = scenario.claude_calls();
    assert!(!Path::new(calls[0]["cwd"].as_str().unwrap()).exists());
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[test]
fn failing_and_incomplete_sessions_report_the_cause_and_keep_local_work() {
    for (ending, cause) in [
        ("exit 3".to_string(), "claude exited 3"),
        ("echo '{\"type\":\"result\",\"subtype\":\"error_max_turns\",\"is_error\":true}' > \"$FAKE_CLAUDE_AFTER_RESULT\"".to_string(), "claude's turn failed"),
        ("true".to_string(), "without a final message"),
        (format!("printf 'Still waiting.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n{}", leaves_running("cargo test")), "unfinished background work: cargo test"),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!("echo unfinished > result.txt\n{ending}"));

        let result = scenario.run(&["skill", "to-spec"]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stdout.is_empty(), "{}", result.stdout);
        assert!(result.stderr.contains(cause), "{}", result.stderr);
        let calls = scenario.claude_calls();
        assert_eq!(calls.len(), 1);
        let path = Path::new(calls[0]["cwd"].as_str().unwrap());
        assert_eq!(fs::read_to_string(path.join("result.txt")).unwrap(), "unfinished\n");
    }
}

#[test]
fn the_default_harness_model_and_effort_run_at_the_checked_out_branch_head() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[harness]\ndefault = 'codex'\n[harness.codex]\nmodel = 'gpt-6.1-sol'\neffort = 'high'\n",
    );
    scenario.launch_git(&["checkout", "-b", "local-branch"]);
    scenario.commit_locally("local.txt", "committed locally", "Local work");
    scenario.agent_does("test \"$(cat local.txt)\" = 'committed locally'\nprintf 'Reviewed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run(&["skill", "code-review", "issue", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "Reviewed.\n");
    assert!(scenario.claude_calls().is_empty());
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 1);
    let args: Vec<_> = calls[0]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    assert!(
        args.windows(2).any(|pair| pair == ["-m", "gpt-6.1-sol"]),
        "{args:?}"
    );
    assert!(
        args.contains(&"model_reasoning_effort=\"high\""),
        "{args:?}"
    );
    assert!(
        args.last().unwrap().starts_with("$thirdshift-code-review"),
        "{args:?}"
    );
    scenario.assert_every_codex_session_found_the_factory_skills();
    assert_eq!(
        scenario.launch_git(&["branch", "--show-current"]).trim(),
        "local-branch"
    );
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn malformed_skill_commands_are_rejected_before_starting_a_session() {
    let scenario = Scenario::new();
    for args in [
        vec!["skill"],
        vec!["skill", "to-spec", "issue"],
        vec!["skill", "to-spec", "extra"],
    ] {
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(2), "{}", result.stderr);
        assert!(
            result
                .stderr
                .contains("usage: thirdshift skill <skill> [issue <issue>]")
        );
    }
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn standalone_skill_uses_exactly_one_session_with_a_configured_claude_model() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[harness]\n\
         default = 'claude'\n\
         [harness.claude]\n\
         model = 'claude-opus-5-5'\n\
         effort = 'high'\n",
    );
    scenario.agent_does(
        "if [ -n \"${FAKE_CLAUDE_FINAL_MESSAGE:-}\" ]; then\n\
             printf 'Done.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n\
         else\n\
             printf 'OK'\n\
         fi",
    );

    let result = scenario.run(&["skill", "to-spec"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "Done.\n");

    let calls: Vec<(String, String)> = scenario
        .claude_calls()
        .into_iter()
        .map(|call| {
            (
                call["prompt"].as_str().unwrap().to_owned(),
                call["cwd"].as_str().unwrap().to_owned(),
            )
        })
        .collect();

    assert_eq!(
        calls.len(),
        1,
        "expected one skill session; observed (prompt, cwd): {calls:?}"
    );
}

#[test]
fn muse_and_opencode_run_the_skill_without_model_probe_sessions() {
    for (harness, model) in [
        ("muse", "muse-spark-1.3"),
        ("opencode", "anthropic/claude-opus-5-5"),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(&format!(
            "[harness]\ndefault = '{harness}'\n[harness.{harness}]\nmodel = '{model}'\neffort = 'high'\n"
        ));
        scenario.agent_does("printf 'Done.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

        let result = scenario.run(&["skill", "to-spec"]);

        assert_eq!(result.code, Some(0), "{harness}: {}", result.stderr);
        assert_eq!(result.stdout, "Done.\n");
        let calls: Vec<serde_json::Value> = serde_json::from_str(
            &fs::read_to_string(scenario.path(&format!("{harness}-calls.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(calls.len(), 1, "{harness}: {calls:?}");
        assert!(
            calls[0]["prompt"]
                .as_str()
                .unwrap()
                .contains("thirdshift-to-spec")
        );
        assert!(
            !scenario
                .path(&format!("{harness}-calls.checks.json"))
                .exists()
        );
    }
}
