use super::*;

fn implementing_process() -> String {
    DELIVER.replace("skill = \"code-review\"", "catalogue = \"implement\"")
}

fn implementation_with_security_review() -> String {
    implementing_process().replace("name = \"ship\"", "name = \"security\"\ncatalogue = \"security-review\"\nworktree = \"fix\"\n\n[[steps]]\nname = \"ship\"")
}

#[test]
fn implement_and_security_review_share_the_delivery_path() {
    let scenario = prepare_ship(&implementation_with_security_review());
    scenario.agent_does_for_in_session(7, 2, r#"
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
test -n "$report"
printf '[]' > "$report"
printf '%s' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&["process", "ship", "issue", "7", "--security-review"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["isDraft"], false);
    assert!(
        state["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("No unaddressed introduced findings.")
    );
}

#[test]
fn implement_then_delivery_leaves_a_ready_pull_request() {
    let scenario = prepare_ship(&implementing_process());

    let result = scenario.run(&["process", "ship", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let prompt = scenario.first_prompt();
    assert!(prompt.contains("thirdshift-implement"), "{prompt}");
    assert!(prompt.contains("base branch is main"), "{prompt}");
    assert!(prompt.contains("Push branch issue-7"), "{prompt}");
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["isDraft"], false);
    assert_eq!(state["prs"][0]["base"], "main");
    assert_eq!(
        scenario.origin_file("issue-7", "feature.txt").as_deref(),
        Some("feature\n")
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn an_unreproduced_bug_ends_the_process_and_claim_without_delivery() {
    let scenario = Scenario::new();
    let body = implementation_with_security_review()
        + "\n[[steps]]\nname = \"after\"\ncommand = \"exit 99\"\nworktree = \"notes\"\n";
    write_process(
        &scenario,
        "ship",
        &body.replace(
            "[worktrees.fix]",
            "[worktrees.notes]\nkind = \"throwaway\"\n\n[worktrees.fix]",
        ),
    );
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.agent_does_for_in_session(7, 1, r#"
gh issue comment 7 --body '## Triage Notes: could not reproduce'
gh fake labels 7 '["bug", "needs-info", "in-progress"]'
echo 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&["process", "ship", "issue", "7", "--security-review"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        result.stdout,
        "https://github.com/acme/widgets/issues/7#issuecomment-42\n"
    );
    assert!(
        scenario
            .first_prompt()
            .contains("thirdshift-diagnosing-bugs")
    );
    assert!(
        result.stderr.contains("bug not reproduced"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("step ship: 0s skipped"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("step security: 0s skipped"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("step after: 0s skipped"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(scenario.gh_state()["prs"], serde_json::json!([]));
    assert_eq!(scenario.issue_labels(7), ["bug", "needs-info"]);
    scenario.assert_cleaned_up("issue-7");
    let (_, notification) = saved_notification(&scenario, "process");
    assert!(
        notification.contains("bug not reproduced"),
        "{notification}"
    );
    assert!(
        notification.contains("issues/7#issuecomment-42"),
        "{notification}"
    );
}

#[test]
fn implement_outside_the_issue_branch_is_refused_before_any_session() {
    for body in [
        "version = 1\ndescription = \"Misplaced implement\"\n[[steps]]\nname = \"implement\"\ncatalogue = \"implement\"\n".to_string(),
        implementing_process().replace("[worktrees.fix]", "[worktrees.notes]\nkind = \"throwaway\"\n\n[worktrees.fix]")
            .replacen("worktree = \"fix\"", "worktree = \"notes\"", 1),
    ] {
        refuse_issue_branch(&body, "implement needs an Issue-branch worktree");
    }
}

#[test]
fn implement_continues_existing_branches_with_or_without_an_open_pull_request() {
    for with_pr in [false, true] {
        let scenario = Scenario::new();
        write_process(&scenario, "ship", &implementing_process());
        let base = if with_pr { "release" } else { "main" };
        if with_pr {
            scenario.origin_has_branch(base, "main", &[]);
        }
        scenario.origin_has_branch("issue-7", base, &["Earlier work"]);
        let url = with_pr.then(|| scenario.github_has_pr("issue-7", base, "OPEN"));
        scenario.agent_does_for_in_session(7, 1, COMMIT_FEATURE);

        let result = scenario.run(&["process", "ship", "issue", "7"]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let prompt = scenario.first_prompt();
        assert!(prompt.contains("already has commits"), "{prompt}");
        assert!(
            prompt.contains(&format!("base branch is {base}")),
            "{prompt}"
        );
        if let Some(url) = url {
            assert!(prompt.contains(&format!("Update PR {url}")), "{prompt}");
            assert_eq!(result.stdout, format!("{url}\n"));
        }
        let state = scenario.gh_state();
        assert_eq!(state["prs"].as_array().unwrap().len(), 1);
        assert_eq!(state["prs"][0]["base"], base);
        assert_eq!(state["prs"][0]["head"], "issue-7");
        assert_eq!(state["prs"][0]["isDraft"], false);
        assert!(
            scenario
                .origin_log("issue-7")
                .unwrap()
                .contains(&"Earlier work".to_string())
        );
    }
}

#[test]
fn reproduced_bugs_go_on_to_delivery_including_an_already_fixed_continuation() {
    for continuing in [false, true] {
        let scenario = Scenario::new();
        write_process(&scenario, "ship", &implementing_process());
        scenario.issue_labelled(7, &["BuG", "ready-for-agent"]);
        scenario.commit_locally("behavior.txt", "broken\n", "Introduce bug");
        scenario.launch_git(&["push", "origin", "main"]);
        if continuing {
            scenario.origin_has_branch("issue-7", "main", &[]);
            scenario.origin_has_commit("issue-7", "behavior.txt", "fixed\n", "Fix bug");
        }
        scenario.agent_does_for_in_session(
            7,
            1,
            r#"
merge_base=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Merge base commit: `\(.*\)`\.$/\1/p')
if [ -n "$merge_base" ]; then
  test "$(git show "$merge_base:behavior.txt")" = broken || exit 21
  test "$(cat behavior.txt)" = fixed || exit 22
else
  test "$(cat behavior.txt)" = broken || exit 23
  echo fixed > behavior.txt
  git add behavior.txt
  git commit -q -m 'Fix bug'
fi
echo 'Bug reproduced: fixed' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        );

        let result = scenario.run(&["process", "ship", "issue", "7"]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let prompt = scenario.first_prompt();
        assert!(prompt.contains("thirdshift-diagnosing-bugs"), "{prompt}");
        assert_eq!(
            prompt.contains("Merge base commit:"),
            continuing,
            "{prompt}"
        );
        assert_eq!(
            scenario.origin_file("issue-7", "behavior.txt").as_deref(),
            Some("fixed\n")
        );
        assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    }
}

#[test]
fn invalid_bug_handoffs_fail_instead_of_delivering() {
    for (message, labels, error) in [
        (
            "Done",
            "[\"bug\",\"in-progress\"]",
            "without a reproduced or not reproduced completion line",
        ),
        (
            "Bug not reproduced: https://github.com/acme/widgets/issues/8#issuecomment-42",
            "[\"bug\",\"needs-info\",\"in-progress\"]",
            "must link a comment on",
        ),
        (
            "Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42",
            "[\"bug\",\"in-progress\"]",
            "requires needs-info",
        ),
    ] {
        let scenario = Scenario::new();
        write_process(&scenario, "ship", &implementing_process());
        scenario.issue_labelled(7, &["bug"]);
        scenario.agent_does_for_in_session(7, 1, &format!("gh fake labels 7 '{labels}'\nprintf '%s\\n' '{message}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\""));

        let result = scenario.run(&["process", "ship", "issue", "7"]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(error), "{}", result.stderr);
        assert_eq!(scenario.gh_state()["prs"], serde_json::json!([]));
        assert!(
            result.stderr.contains("step ship: 0s skipped"),
            "{}",
            result.stderr
        );
    }
}

#[test]
fn implement_uses_its_own_harness_model_and_effort() {
    let scenario = prepare_ship(&implementing_process().replace(
        "catalogue = \"implement\"",
        "catalogue = \"implement\"\nharness = \"codex\"\nmodel = \"gpt-6.1-sol\"\neffort = \"max\"",
    ));

    let result = scenario.run(&["process", "ship", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 1);
    assert_flag(&calls[0], "-m", "gpt-6.1-sol");
    assert!(argv_of(&calls[0]).contains(&"model_reasoning_effort=\"max\"".to_string()));
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
}

#[test]
fn implement_gets_one_resume_before_delivery() {
    let scenario = Scenario::new();
    write_process(&scenario, "ship", &implementing_process());
    scenario.agent_does_in_session(
        1,
        &format!(
            "printf waiting > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n{}",
            leaves_running("implementation fixture work")
        ),
    );
    scenario.agent_does_in_session(2, COMMIT_FEATURE);

    let result = scenario.run(&["process", "ship", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    assert!(
        argv_of(&calls[1])
            .windows(2)
            .any(|pair| pair == ["--resume", "fake-session-1"])
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    assert_eq!(
        scenario.origin_file("issue-7", "feature.txt").as_deref(),
        Some("feature\n")
    );
}

#[test]
fn each_implement_step_retains_its_own_session_trace() {
    let scenario = Scenario::new();
    write_process(&scenario, "ship", &implementing_process().replace("name = \"ship\"", "name = \"second\"\ncatalogue = \"implement\"\nworktree = \"fix\"\n\n[[steps]]\nname = \"ship\""));
    scenario.agent_does_in_session(1, COMMIT_FEATURE);
    scenario.agent_does_in_session(2, "printf checked > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run(&["process", "ship", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let traces = fs::read_dir(process_dir(&scenario))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".logs.jsonl"))
        .count();
    assert_eq!(traces, 2, "each implement Step must keep its Session trace");
}

#[test]
fn an_unreproduced_continuation_keeps_origin_work_and_ends_the_claim() {
    let scenario = Scenario::new();
    write_process(&scenario, "ship", &implementing_process());
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    let original = scenario.origin_log("issue-7");
    scenario.issue_labelled(7, &["bug", "in-progress"]);
    scenario.agent_does_for_in_session(7, 1, r#"
gh issue comment 7 --body '## Triage Notes: absent at the merge base'
gh fake labels 7 '["bug", "needs-info", "in-progress"]'
echo 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&["process", "ship", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.origin_log("issue-7"), original);
    assert_eq!(scenario.issue_labels(7), ["bug", "needs-info"]);
    assert_eq!(scenario.gh_state()["prs"], serde_json::json!([]));
    scenario.assert_cleaned_up("issue-7");
    let status = status_json(&scenario);
    let command = process_command(&status);
    assert_eq!(command["outcome"], "bug not reproduced");
    assert!(
        command["cause"]
            .as_str()
            .unwrap()
            .contains("issues/7#issuecomment-42")
    );
}
