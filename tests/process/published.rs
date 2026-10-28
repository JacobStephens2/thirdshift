use super::*;

#[test]
fn a_child_process_must_exist_and_have_an_issue_branch_before_any_session() {
    for (process, reason) in [
        ("absent", "unknown process absent"),
        (
            "notes",
            "the child Ticket Process notes needs an Issue-branch worktree",
        ),
    ] {
        for check in [true, false] {
            let scenario = Scenario::new();
            write_process(&scenario, "notes", REVIEW);
            write_process(
                &scenario,
                "publish",
                &format!("{PUBLISH_THEN_RUN}process = \"{process}\"\n"),
            );

            let result = if check {
                scenario.run(&["process", "check", "publish"])
            } else {
                scenario.run(&["process", "publish"])
            };

            assert_no_session(&scenario, &result);
            assert!(result.stderr.contains(reason), "{}", result.stderr);
        }
    }
}

#[test]
fn a_published_ticket_runs_the_named_custom_process() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "publish",
        &format!("{PUBLISH_THEN_RUN}process = \"contractor\"\n"),
    );
    write_process(
        &scenario,
        "contractor",
        r#"version = 1
description = "Contractor Ticket process"

[worktrees.ticket]
kind = "issue-branch"

[[steps]]
name = "change"
command = "echo contractor > contractor.txt; git add contractor.txt; git commit -qm contractor; gh pr create --base main --head issue-8 --title Contractor --body 'Closes #8'"
worktree = "ticket"

[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
"#,
    );
    scenario.agent_does_in_session(
        1,
        r#"url=$(gh issue create --title Ticket --body 'Do the thing')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let pr = pr_from(&scenario, "issue-8");
    assert_eq!(result.stdout.trim(), pr["url"].as_str().unwrap());
    assert_eq!(
        scenario.origin_git(&["show", "issue-8:contractor.txt"]),
        "contractor\n"
    );
    assert_eq!(scenario.claude_calls().len(), 1, "a built-in session ran");
    let record = process_record(&scenario);
    assert_eq!(record["process_name"], "publish", "{record}");
    assert_eq!(record["dispatched"][0]["kind"], "Run", "{record}");
    assert_eq!(
        record["dispatched"][0]["outcome"], "ready for review",
        "{record}"
    );
    scenario.assert_cleaned_up("issue-8");
}

#[test]
fn a_published_spec_still_dispatches_a_spec_run_with_a_custom_ticket_process_setting() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "publish",
        &format!("{PUBLISH_THEN_RUN}process = \"contractor\"\n"),
    );
    write_process(
        &scenario,
        "contractor",
        r#"version = 1
description = "This Ticket process must not replace Spec dispatch"

[worktrees.ticket]
kind = "issue-branch"

[[steps]]
name = "fail"
command = "exit 99"
worktree = "ticket"

[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
"#,
    );
    scenario.agent_does_in_session(
        1,
        r#"spec=$(gh issue create --title Spec --body 'Widgets')
ticket=$(gh issue create --title Ticket --body 'One widget')
gh api --method POST repos/acme/widgets/issues/${spec##*/}/sub_issues -F sub_issue_id=${ticket##*/}
printf '%s\n' "$spec" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(9, &agent_lands_ticket(9, "issue-8"));

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let pr = pr_from(&scenario, "issue-8");
    assert_eq!(result.stdout.trim(), pr["url"].as_str().unwrap());
    let record = process_record(&scenario);
    assert_eq!(record["dispatched"][0]["kind"], "Spec run", "{record}");
    assert_eq!(
        record["dispatched"][0]["outcome"], "ready for review",
        "{record}"
    );
    assert_eq!(scenario.origin_git(&["show", "issue-8:issue-9.txt"]), "9\n");
}

#[test]
fn a_process_setting_is_a_name_and_only_belongs_to_run_published_issue() {
    for (body, reason) in [
        (
            format!("{PUBLISH_THEN_RUN}process = 42\n"),
            "process must be a string",
        ),
        (
            format!("{PUBLISH_THEN_RUN}process = \"\"\n"),
            "process must not be empty",
        ),
        (
            format!("{PUBLISH_THEN_RUN}process = \"../ticket\"\n"),
            "process must be a single path component",
        ),
        (
            format!("{REVIEW}process = \"ticket-run\"\n"),
            "process is only for run-published-issue",
        ),
    ] {
        let scenario = Scenario::new();
        write_process(&scenario, "publish", &body);

        let result = scenario.run(&["process", "check", "publish"]);

        assert_no_session(&scenario, &result);
        assert!(result.stderr.contains(reason), "{}", result.stderr);
    }
}

#[test]
fn a_custom_child_keeps_its_results_and_session_traces_apart_from_its_parent() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "publish",
        &format!(
            r#"{PUBLISH_THEN_RUN}process = "contractor"

[[steps]]
name = "read-parent"
command = 'cat "$THIRDSHIFT_RESULTS_DIR/publish.txt"'
"#
        ),
    );
    write_process(
        &scenario,
        "contractor",
        r#"version = 1
description = "Custom skill with the same Step name as its parent"

[worktrees.ticket]
kind = "issue-branch"

[[steps]]
name = "publish"
skill = "code-review"
worktree = "ticket"

[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
"#,
    );
    scenario.agent_does_in_session(
        1,
        r#"url=$(gh issue create --title Ticket --body 'Do the thing')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}\nprintf 'child result' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
            agent_lands_ticket(8, "main")
        ),
    );

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let traces = fs::read_dir(process_dir(&scenario))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".logs.jsonl"))
        .count();
    assert_eq!(
        (result.stdout.trim(), traces),
        (scenario.issue_url(8).as_str(), 2)
    );
}

#[test]
fn a_selected_child_keeps_inherited_limits_on_its_base_fix() {
    let scenario = Scenario::new();
    scenario.user_config_is("[base]\nfix = true\n");
    let publish = PUBLISH_THEN_RUN.replacen(
        "[[steps]]",
        "[limits]\nprotected_paths = [\"issue-9.txt\"]\n\n[[steps]]",
        1,
    );
    write_process(
        &scenario,
        "publish",
        &format!("{publish}process = \"contractor\"\n"),
    );
    write_process(
        &scenario,
        "contractor",
        include_str!("../../processes/ticket-run.toml"),
    );
    scenario.agent_does_in_session(
        1,
        r#"url=$(gh issue create --title Ticket --body 'Do the thing')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(
        8,
        &format!(
            r#"{}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure","url":"https://ci.example/test"}}]'
gh fake checks "$(git rev-parse origin/main)" '[{{"name":"test","conclusion":"failure","url":"https://ci.example/main/test"}}]'
"#,
            agent_lands_ticket(8, "main")
        ),
    );
    scenario.agent_does_for(
        9,
        &format!(
            r#"{}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"success"}}]'
"#,
            agent_lands_ticket(9, "main")
        ),
    );

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("protected path issue-9.txt"),
        "{}",
        result.stderr
    );
    assert_eq!(pr_from(&scenario, "issue-9")["state"], "OPEN");
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
}

#[test]
fn a_selected_child_keeps_its_ready_pr_in_failure_accounting() {
    let scenario = Scenario::new();
    scenario.user_config_is("[merge]\nalways = true\n");
    let publish = PUBLISH_THEN_RUN.replacen(
        "[[steps]]",
        "[limits]\nprotected_paths = [\"issue-8.txt\"]\n\n[[steps]]",
        1,
    );
    write_process(
        &scenario,
        "publish",
        &format!("{publish}process = \"contractor\"\n"),
    );
    write_process(
        &scenario,
        "contractor",
        include_str!("../../processes/ticket-run.toml"),
    );
    scenario.agent_does_in_session(
        1,
        r#"url=$(gh issue create --title Ticket --body 'Do the thing')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(8, &agent_lands_ticket(8, "main"));

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("protected path issue-8.txt"),
        "{}",
        result.stderr
    );
    let pr = pr_from(&scenario, "issue-8");
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], false);
    let record = process_record(&scenario);
    assert_eq!(record["dispatched"][0]["outcome"], "failed", "{record}");
    assert_eq!(
        record["dispatched"][0]["pull_request"], pr["url"],
        "{record}"
    );
}
