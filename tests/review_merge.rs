//! Opt-in Review Self-merge through the CLI, real Git and fake services.

mod support;

use serde_json::json;
use support::Scenario;

const NO_FINDINGS: &str =
    "printf 'No findings. Validation passed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"";

fn pull_request(scenario: &Scenario) -> String {
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    scenario.github_has_pr("colleague", "main", "OPEN")
}

#[test]
fn review_merge_requires_a_complete_clean_security_review() {
    for pass in [false, true] {
        for security in [
            "exit 1",
            "printf 'Incomplete' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
            r#"
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
printf '[]' > "$report"
printf '%s\n' 'Security review: {"unaddressed_count":1,"findings":["Introduced input bug"],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        ] {
            let scenario = Scenario::new();
            let url = pull_request(&scenario);
            scenario.user_config_is("[security]\nreview = true\n");
            scenario.agent_does(NO_FINDINGS);
            scenario.agent_does_in_session(2, security);
            let args = if pass {
                vec!["review", "--author", "colleague", "merge"]
            } else {
                vec!["review", &url, "merge"]
            };

            let result = scenario.run(&args);

            assert_eq!(
                result.code,
                Some(if pass { 0 } else { 1 }),
                "{}",
                result.stderr
            );
            let state = scenario.gh_state();
            assert_eq!(state["prs"][0]["state"], "OPEN", "{state}");
            assert_eq!(state["prs"][0]["isDraft"], false);
            assert!(scenario.gh_calls_of("pr", "merge").is_empty());
            assert!(
                result.stderr.contains("Security review"),
                "{}",
                result.stderr
            );
            let record =
                scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/review");
            assert_eq!(record["outcome"], "ready for review", "{record}");
        }
    }
}

#[test]
fn merge_holds_leave_ready_with_pass_specific_exit() {
    for pass in [false, true] {
        for reason in ["stack", "review-required", "foreign"] {
            let scenario = Scenario::new();
            let url = pull_request(&scenario);
            scenario.agent_does(NO_FINDINGS);
            let mut state = scenario.gh_state();
            match reason {
                "stack" => {
                    scenario.origin_has_branch("trunk", "main", &[]);
                    scenario.github_has_pr("main", "trunk", "OPEN");
                    state = scenario.gh_state();
                    let base_head = scenario.origin_git(&["rev-parse", "main"]);
                    state["prs"][1]["comments"] = json!([{
                        "body": format!("<!-- thirdshift-review {} -->", json!({"reviewed_head":base_head.trim()})),
                        "author_association":"COLLABORATOR", "user":{"login":"runner"}
                    }]);
                }
                "review-required" => state["prs"][0]["reviewDecision"] = json!("REVIEW_REQUIRED"),
                "foreign" => {
                    scenario.origin_has_branch("foreign", "colleague", &["Unreviewed commit"]);
                    scenario.agent_does(&format!("gh fake on-ci-read 1 'git fetch -q origin foreign; git push -q origin FETCH_HEAD:refs/heads/colleague'\n{NO_FINDINGS}"));
                }
                _ => unreachable!(),
            }
            scenario.write_gh_state(&state);
            let args = if pass {
                vec!["review", "--author", "colleague", "merge"]
            } else {
                vec!["review", &url, "merge"]
            };

            let result = scenario.run(&args);

            assert_eq!(
                result.code,
                Some(if pass { 0 } else { 1 }),
                "{reason}: {}",
                result.stderr
            );
            let state = scenario.gh_state();
            assert_eq!(state["prs"][0]["state"], "OPEN", "{reason}: {state}");
            assert_eq!(state["prs"][0]["isDraft"], false, "{reason}: {state}");
            assert!(
                scenario.gh_calls_of("pr", "merge").is_empty(),
                "{reason}: {}",
                result.stderr
            );
            let record =
                scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/review");
            assert_eq!(record["outcome"], "ready for review", "{reason}: {record}");
            let expected = match reason {
                "stack" => "open pull request",
                "review-required" => "approving review",
                _ => "head moved",
            };
            assert!(
                result.stderr.contains(expected),
                "{reason}: {}",
                result.stderr
            );
        }
    }
}

#[test]
fn recovery_freezes_the_effective_review_merge_choice() {
    for (config, word, expected) in [
        ("[review]\nmerge = true\n", None, " merge"),
        ("[review]\nmerge = true\n", Some("no-merge"), "--no-merge"),
        ("[merge]\nalways = true\n", None, "--no-merge"),
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        scenario.user_config_is(config);
        scenario.agent_does("exit 1");
        let mut args = vec!["review", &url];
        args.extend(word);

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/review");
        let command = record["resume_command"]
            .as_str()
            .expect("saved Continuation command");
        assert!(command.contains(expected), "{command}");
        assert!(
            result.stderr.contains(&format!("Recovery: {command}")),
            "{}",
            result.stderr
        );
    }
}

#[test]
fn review_merge_precedence_is_independent_of_merge_always() {
    for (config, word, pass, merged) in [
        ("", None, false, false),
        ("[merge]\nalways = true\n", None, false, false),
        ("[merge]\nalways = true\n", None, true, false),
        ("[review]\nmerge = true\n", None, true, true),
        ("[review]\nmerge = true\n", Some("no-merge"), true, false),
        ("[review]\nmerge = true\n", Some("--no-merge"), false, false),
        ("[review]\nmerge = false\n", Some("--merge"), false, true),
    ] {
        let scenario = Scenario::new();
        let url = pull_request(&scenario);
        scenario.user_config_is(config);
        scenario.agent_does(NO_FINDINGS);
        let mut args = if pass {
            vec!["review", "--author", "colleague"]
        } else {
            vec!["review", &url]
        };
        args.extend(word);

        let result = scenario.run(&args);

        assert_eq!(
            result.code,
            Some(0),
            "{config}, {word:?}: {}",
            result.stderr
        );
        assert_eq!(
            scenario.gh_state()["prs"][0]["state"],
            if merged { "MERGED" } else { "OPEN" }
        );
        assert_eq!(
            scenario.gh_calls_of("pr", "merge").len(),
            usize::from(merged)
        );
    }
}

#[test]
fn review_rejects_conflicting_repeated_and_recommend_only_merge_words() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    for words in [
        vec!["merge", "no-merge"],
        vec!["--merge", "--no-merge"],
        vec!["merge", "merge"],
        vec!["merge", "--merge"],
        vec!["no-merge", "--no-merge"],
        vec!["--recommend-only", "merge"],
        vec!["--merge", "--recommend-only"],
    ] {
        let mut args = vec!["review", &url];
        args.extend(words);
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(2), "{args:?}: {}", result.stderr);
    }
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
}

#[test]
fn review_merge_reviews_foreign_commits_that_arrive_at_the_merge_request() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    scenario.origin_has_branch("foreign", "colleague", &["Late foreign commit"]);
    scenario.agent_does(NO_FINDINGS);
    scenario.agent_does_in_session(1, &format!(
        "gh fake on-merge 1 'git fetch -q origin foreign; git push -q origin FETCH_HEAD:refs/heads/colleague'\n{NO_FINDINGS}"
    ));

    let result = scenario.run(&["review", &url, "merge"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_eq!(scenario.gh_calls_of("pr", "merge").len(), 2);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("new commits on origin/colleague to review"),
        "{}",
        result.stderr
    );
    assert!(
        scenario
            .origin_log("main")
            .unwrap()
            .iter()
            .any(|line| line == "Late foreign commit")
    );
}

#[test]
fn copied_review_process_can_opt_into_self_merge_after_clean_security_review() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let shown = scenario.run(&["process", "show", "review-run"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let directory = scenario.path("home/.thirdshift/processes");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("my-review.toml"), shown.stdout).unwrap();
    scenario.agent_does(NO_FINDINGS);
    scenario.agent_does_in_session(2, r#"
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
printf '[]' > "$report"
printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&[
        "process",
        "my-review",
        "pull-request",
        &url,
        "merge",
        "security-review",
    ]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
}

#[test]
fn review_merge_repairs_a_merge_time_conflict_and_then_red_ci() {
    let scenario = Scenario::new();
    scenario.origin_has_commit("main", "shared.txt", "original\n", "Shared file");
    let url = pull_request(&scenario);
    scenario.origin_has_commit("colleague", "shared.txt", "author\n", "Author change");
    scenario.origin_has_branch("base-update", "main", &[]);
    scenario.origin_has_commit(
        "base-update",
        "shared.txt",
        "upstream\n",
        "Conflicting base change",
    );
    scenario.agent_does(NO_FINDINGS);
    scenario.agent_does_in_session(1, &format!(
        "gh fake on-merge 1 'git fetch -q origin base-update; git push -q origin FETCH_HEAD:refs/heads/main'\n{NO_FINDINGS}"
    ));
    scenario.agent_does_in_session(
        2,
        &format!(
            r#"
git checkout --ours shared.txt
git add shared.txt
git commit -qm 'Resolve base conflict'
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure"}}]'
{NO_FINDINGS}
"#
        ),
    );
    scenario.agent_does_in_session(
        3,
        &format!(
            r#"
echo fixed > ci-fix.txt
git add ci-fix.txt
git commit -qm 'Fix merge CI'
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"success"}}]'
{NO_FINDINGS}
"#
        ),
    );

    let result = scenario.run(&["review", &url, "merge"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_eq!(scenario.gh_calls_of("pr", "merge").len(), 2);
    assert_eq!(scenario.claude_calls().len(), 3, "{}", result.stderr);
    assert_eq!(
        scenario.origin_file("main", "ci-fix.txt").as_deref(),
        Some("fixed\n")
    );
}

#[test]
fn review_merge_merges_the_reviewed_head_and_reports_merged() {
    let scenario = Scenario::new();
    let url = pull_request(&scenario);
    let head = scenario.origin_git(&["rev-parse", "colleague"]);
    scenario
        .agent_does("printf 'No findings. Validation passed.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run(&["review", &url, "merge"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    let merges = scenario.gh_calls_of("pr", "merge");
    assert_eq!(merges.len(), 1, "{merges:?}");
    assert!(
        merges[0]
            .windows(2)
            .any(|pair| pair == ["--match-head-commit", head.trim()])
    );
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/review");
    assert_eq!(record["outcome"], "merged", "{record}");
    let (_, notification) = support::resend::saved_notification(&scenario, "review");
    assert!(notification.contains("merged"), "{notification}");
    assert!(
        scenario
            .launch_git(&["ls-remote", "origin", "refs/heads/colleague"])
            .is_empty()
    );
}
