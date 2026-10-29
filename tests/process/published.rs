use super::*;

fn write_architecture_ticket_process(scenario: &Scenario) {
    write_process(
        scenario,
        "contractor",
        r#"version = 1
description = "Review architecture before editing the Ticket"
[worktrees.ticket]
kind = "issue-branch"
[worktrees.notes]
kind = "throwaway"
[[steps]]
name = "architecture"
catalogue = "architecture-review"
worktree = "notes"
[[steps]]
name = "edit"
command = "exit 99"
worktree = "ticket"
[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
"#,
    );
}

const PUBLISH_IDEA: &str = r#"url=$(gh issue create --title Idea --body 'Worth exploring' --label needs-triage)
printf 'Recommendation strength: Worth exploring\nArchitecture review idea: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

#[test]
fn a_selected_ticket_process_preserves_an_architecture_ideas_successful_early_end() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "architect-idea"]);
    write_process(
        &scenario,
        "publish",
        &format!("{PUBLISH_THEN_RUN}process = \"contractor\"\n"),
    );
    write_architecture_ticket_process(&scenario);
    scenario.agent_does_in_session(
        1,
        r#"url=$(gh issue create --title Ticket --body 'Do the thing')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_in_session(2, PUBLISH_IDEA);

    let result = scenario.run(&["process", "publish"]);

    assert!(
        result.stderr.contains("step edit: 0s skipped"),
        "{}",
        result.stderr
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.trim(), scenario.issue_url(9));
    assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
    assert!(
        !scenario
            .issue_labels(8)
            .contains(&"in-progress".to_string())
    );
    scenario.assert_cleaned_up("issue-8");
}

#[test]
fn an_architecture_ending_in_a_spec_keeps_the_ticket_and_its_dependents_undelivered() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "architect-idea"]);
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[8])]);
    write_architecture_ticket_process(&scenario);
    write_process(
        &scenario,
        "gather",
        r#"version = 1
description = "Gather Tickets"
[worktrees.spec]
kind = "spec-branch"
[[steps]]
name = "land"
catalogue = "land-tickets"
process = "contractor"
worktree = "spec"
"#,
    );
    scenario.agent_does_in_session(1, PUBLISH_IDEA);

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("#8 Architecture review ended: no Strong candidate"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("#9 blocked by #8"),
        "{}",
        result.stderr
    );
    assert!(!result.stderr.contains("#8 landed"), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
    assert_eq!(scenario.gh_state()["issues"]["8"], "OPEN");
    scenario.assert_cleaned_up("issue-8");
}

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
        record["dispatched"][0]["process_name"], "contractor",
        "{record}"
    );
    assert_eq!(
        record["dispatched"][0]["process_kind"], "custom",
        "{record}"
    );
    use sha2::{Digest, Sha256};
    let source = fs::read(scenario.path("home/.thirdshift/processes/contractor.toml")).unwrap();
    assert_eq!(
        record["dispatched"][0]["process_hash"],
        format!("{:x}", Sha256::digest(source)),
        "{record}"
    );
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
    use sha2::{Digest, Sha256};
    for entry in fs::read_dir(process_dir(&scenario)).unwrap() {
        let path = entry.unwrap().path();
        if !path.to_string_lossy().ends_with(".logs.jsonl") {
            continue;
        }
        let name = if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("8-")
        {
            "contractor"
        } else {
            "publish"
        };
        let source =
            fs::read(scenario.path(&format!("home/.thirdshift/processes/{name}.toml"))).unwrap();
        let expected_hash = format!("{:x}", Sha256::digest(source));
        for line in fs::read_to_string(&path).unwrap().lines() {
            let request: serde_json::Value = serde_json::from_str(line).unwrap();
            for batch in request["resourceLogs"].as_array().unwrap() {
                let attributes = batch["resource"]["attributes"].as_array().unwrap();
                for (key, expected) in [
                    ("thirdshift.process.name", name),
                    ("thirdshift.process.kind", "custom"),
                    ("thirdshift.process.hash", expected_hash.as_str()),
                ] {
                    let actual = attributes
                        .iter()
                        .find(|attr| attr["key"] == key)
                        .and_then(|attr| attr["value"]["stringValue"].as_str());
                    assert_eq!(actual, Some(expected), "{}: {batch}", path.display());
                }
            }
        }
    }
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

#[test]
fn custom_child_failed_before_delivery_drafts_its_pr() {
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
description = "Fail after implementation"

[worktrees.ticket]
kind = "issue-branch"

[[steps]]
name = "implement"
catalogue = "implement"
worktree = "ticket"

[[steps]]
name = "verify"
command = "exit 23"
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
    scenario.agent_does_for(8, &agent_lands_ticket(8, "main"));

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let pr = pr_from(&scenario, "issue-8");
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(
        pr["isDraft"], true,
        "a failed Ticket must return its PR to draft: {pr}"
    );
    let record = process_record(&scenario);
    assert_eq!(
        record["dispatched"][0]["pull_request"], pr["url"],
        "{record}"
    );
}

#[test]
fn a_nested_selected_child_keeps_its_delivery_identity() {
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
description = "Dispatch another Ticket before delivering this one"

[worktrees.ticket]
kind = "issue-branch"

[[steps]]
name = "publish-nested"
skill = "to-spec"
worktree = "ticket"

[[steps]]
name = "run-nested"
catalogue = "run-published-issue"
worktree = "ticket"

[[steps]]
name = "change"
worktree = "ticket"
command = """
echo contractor > contractor.txt
git add contractor.txt
git commit -qm contractor
gh pr create --base main --head issue-8 --title Contractor --body 'Closes #8'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]'
"""

[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
"#,
    );
    scenario.agent_does_in_session(
        1,
        r#"url=$(gh issue create --title Outer --body 'Outer Ticket')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_in_session(
        2,
        r#"url=$(gh issue create --title Nested --body 'Nested Ticket')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(9, &agent_lands_ticket(9, "main"));
    scenario.agent_does_in_session(
        4,
        r#"echo repaired > repaired.txt
git add repaired.txt
git commit -qm repaired
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success","url":"https://ci.example/test"}]'
printf repaired > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["process", "publish"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);

    let repair = fs::read_dir(process_dir(&scenario))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.starts_with("8-") && name.ends_with("-repair-1.logs.jsonl")
        })
        .expect("the outer Ticket must have a Delivery Repair trace");

    for line in fs::read_to_string(repair).unwrap().lines() {
        let request: serde_json::Value = serde_json::from_str(line).unwrap();
        for batch in request["resourceLogs"].as_array().unwrap() {
            let name = batch["resource"]["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|attribute| attribute["key"] == "thirdshift.process.name")
                .and_then(|attribute| attribute["value"]["stringValue"].as_str());
            assert_eq!(name, Some("contractor"), "{batch}");
        }
    }

    let record = process_record(&scenario);
    assert_eq!(
        record["dispatched"][0]["outcome"], "ready for review",
        "{record}"
    );
    assert_eq!(
        record["dispatched"][1]["pull_request"],
        pr_from(&scenario, "issue-9")["url"],
        "{record}"
    );
}

#[test]
fn a_custom_child_keeps_the_parents_factory_skills_until_the_command_ends() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "publish",
        &format!(
            r#"{PUBLISH_THEN_RUN}process = "contractor"

[[steps]]
name = "confirm"
skill = "code-review"
"#
        ),
    );
    write_process(
        &scenario,
        "contractor",
        r#"version = 1
description = "Command-only Ticket"
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
        r#"readlink .claude/skills/thirdshift-code-review > "$HOME/parent-skill-source"
url=$(gh issue create --title Ticket --body 'Do the thing')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_in_session(
        2,
        r#"test -r "$(cat "$HOME/parent-skill-source")/SKILL.md" || exit 23
printf 'parent skills retained' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.trim(), "parent skills retained");
}

#[test]
fn a_base_fix_after_nested_dispatch_keeps_its_outer_ticket_parent() {
    let scenario = Scenario::new();
    scenario.user_config_is("[base]\nfix = true\n");
    let publish = PUBLISH_THEN_RUN.replacen(
        "[[steps]]",
        "[limits]\nprotected_paths = [\"issue-10.txt\"]\n\n[[steps]]",
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
        r#"version = 1
description = "Dispatch a nested Ticket, then need a Base fix"

[worktrees.ticket]
kind = "issue-branch"

[[steps]]
name = "publish-nested"
skill = "to-spec"
worktree = "ticket"

[[steps]]
name = "run-nested"
catalogue = "run-published-issue"
worktree = "ticket"

[[steps]]
name = "change"
worktree = "ticket"
command = """
echo contractor > contractor.txt
git add contractor.txt
git commit -qm contractor
gh pr create --base main --head issue-8 --title Contractor --body 'Closes #8'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/main/test"}]'
"""

[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
"#,
    );
    scenario.agent_does_in_session(
        1,
        r#"url=$(gh issue create --title Outer --body 'Outer Ticket')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_in_session(
        2,
        r#"url=$(gh issue create --title Nested --body 'Nested Ticket')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(9, &agent_lands_ticket(9, "main"));
    scenario.agent_does_for(
        10,
        &format!(
            r#"{}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"success"}}]'
"#,
            agent_lands_ticket(10, "main")
        ),
    );

    let result = scenario.run(&["process", "publish"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("protected path issue-10.txt"),
        "{}",
        result.stderr
    );
    let base_fix = fs::read_dir(process_dir(&scenario))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".child.json"))
        .map(|path| serde_json::from_slice::<serde_json::Value>(&fs::read(path).unwrap()).unwrap())
        .find(|record| record["issue"]["number"] == 10)
        .expect("the Base fix must retain a child record");
    let outer = scenario.issue_url(8);
    assert_eq!(
        base_fix["parent_dispatch"].as_str(),
        Some(outer.as_str()),
        "{base_fix}"
    );
}
