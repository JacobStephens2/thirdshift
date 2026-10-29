use super::*;

#[test]
fn architect_run_is_a_readable_valid_builtin_whose_copy_can_run() {
    let scenario = architecture_process();
    let source = assert_compiled_process(
        &scenario,
        "architect-run",
        &["architecture-review", "run-published-issue"],
        None,
    );
    write_process(&scenario, "my-architect", &source);
    scenario.agent_does_in_session(1, PUBLISH_PLAN);
    let result = scenario.run(&["process", "my-architect", "--plan-only"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.trim(), scenario.issue_url(8));
    assert_eq!(
        scenario.issue_labels(8),
        ["ready-for-agent", "architect-plan"]
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn an_architecture_review_cannot_skip_delivery_of_issue_branch_work() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    write_process(
        &scenario,
        "unsafe-review",
        r#"
version = 1
description = "Edit an issue, then review architecture before Delivery"

[worktrees.fix]
kind = "issue-branch"
[worktrees.notes]
kind = "throwaway"

[[steps]]
name = "edit"
command = "printf pending > pending.txt"
worktree = "fix"

[[steps]]
name = "architecture"
catalogue = "architecture-review"
worktree = "notes"

[[steps]]
name = "ship"
catalogue = "delivery"
worktree = "fix"
"#,
    );
    scenario.agent_does_in_session(
        1,
        &PUBLISH_PLAN
            .replace("Strong", "Worth exploring")
            .replace("review plan:", "review idea:"),
    );

    let result = scenario.run(&["process", "unsafe-review", "issue", "7"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("architecture-review cannot skip delivery"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty(), "{}", result.stderr);
}

#[test]
fn an_early_architecture_end_preserves_earlier_issue_branch_work() {
    let scenario = architecture_process();
    write_process(
        &scenario,
        "architecture",
        r#"
version = 1
description = "Edit, review architecture, then deliver"

[worktrees.fix]
kind = "issue-branch"
[worktrees.notes]
kind = "throwaway"

[[steps]]
name = "edit"
command = '''
printf '%s\n' "$PWD" > "$HOME/edited-checkout"
printf valuable > valuable.txt
'''
worktree = "fix"

[[steps]]
name = "architecture"
catalogue = "architecture-review"
worktree = "notes"

[[steps]]
name = "ship"
catalogue = "delivery"
worktree = "fix"
"#,
    );
    scenario.agent_does_in_session(
        1,
        &PUBLISH_PLAN
            .replace("Strong", "Worth exploring")
            .replace("review plan:", "review idea:"),
    );

    let result = scenario.run(&["process", "architecture", "issue", "7"]);
    let marker = scenario.path("home/edited-checkout");
    if !marker.exists() {
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        return; // Refusal before editing is also safe.
    }
    let checkout = fs::read_to_string(marker).unwrap();
    let local = fs::read_to_string(Path::new(checkout.trim()).join("valuable.txt")).ok();
    let published = scenario.origin_file("issue-7", "valuable.txt");
    assert!(
        local.as_deref() == Some("valuable") || published.as_deref() == Some("valuable"),
        "the early ending deleted unpublished Issue-branch work: {}",
        result.stderr,
    );
}

#[test]
fn an_early_architecture_end_releases_the_claim_when_delivery_does_not_run() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&[
        "bug",
        "needs-triage",
        "ready-for-agent",
        "in-progress",
        "architect-idea",
    ]);
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    write_process(
        &scenario,
        "architecture",
        r#"
version = 1
description = "Review architecture before editing the issue"

[worktrees.notes]
kind = "throwaway"
[worktrees.fix]
kind = "issue-branch"

[[steps]]
name = "architecture"
catalogue = "architecture-review"
worktree = "notes"

[[steps]]
name = "edit"
command = "printf valuable > valuable.txt"
worktree = "fix"

[[steps]]
name = "ship"
catalogue = "delivery"
worktree = "fix"
"#,
    );
    scenario.agent_does_in_session(
        1,
        &PUBLISH_PLAN
            .replace("Strong", "Worth exploring")
            .replace("review plan:", "review idea:"),
    );

    let result = scenario.run(&["process", "architecture", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("step ship: 0s skipped"),
        "{}",
        result.stderr
    );
    assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
    let mut labels = scenario.issue_labels(7);
    labels.sort();
    assert_eq!(
        labels,
        ["bug", "ready-for-agent"],
        "the early ending removed the Claim instead of releasing it\n{}",
        result.stderr
    );
}

const ARCHITECTURE: &str = r#"
version = 1
description = "Review architecture, then run the Strong plan"

[[steps]]
name = "architecture"
catalogue = "architecture-review"

[[steps]]
name = "run"
catalogue = "run-published-issue"
"#;

const PUBLISH_PLAN: &str = r#"
url=$(gh issue create --title "Deepen sessions" --body "The plan" --label needs-triage)
printf 'Recommendation strength: Strong\nArchitecture review plan: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

fn architecture_process() -> Scenario {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    write_process(&scenario, "architecture", ARCHITECTURE);
    scenario
}

#[test]
fn a_strong_plan_starts_a_ticket_run_on_the_base_branch() {
    let scenario = architecture_process();
    scenario.agent_does_in_session(1, PUBLISH_PLAN);
    scenario.agent_does_for(8, &agent_lands_ticket(8, "main"));

    let result = scenario.run(&["process", "architecture"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    let state = scenario.gh_state();
    assert_eq!(scenario.issue_labels(8), ["architect-plan", "in-progress"]);
    assert_eq!(state["prs"][0]["base"], "main");
    assert_eq!(
        process_record(&scenario)["dispatched"][0]["issue"]["number"],
        8
    );
}

#[test]
fn an_idea_ends_successfully_and_skips_every_later_step() {
    for already_filed in [false, true] {
        let scenario = architecture_process();
        write_process(
            &scenario,
            "architecture",
            &format!("{ARCHITECTURE}\n[[steps]]\nname = 'after'\ncommand = 'exit 99'\n"),
        );
        let (script, number, reason) = if already_filed {
            (
                format!(
                    "printf 'Recommendation strength: Strong\\nArchitecture review already filed: %s\\n' '{}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
                    scenario.issue_url(7)
                ),
                7,
                format!(
                    "no Strong candidate: {} already covers the Architecture review's top recommendation, so it filed nothing",
                    scenario.issue_url(7)
                ),
            )
        } else {
            (
                PUBLISH_PLAN
                    .replace("Strong", "Worth exploring")
                    .replace("review plan:", "review idea:"),
                8,
                format!(
                    "no Strong candidate: the Architecture review filed the idea {}",
                    scenario.issue_url(8)
                ),
            )
        };
        scenario.agent_does_in_session(1, &script);

        let result = scenario.run(&["process", "architecture"]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(result.stdout.trim(), scenario.issue_url(number));
        assert_eq!(scenario.claude_calls().len(), 1);
        let mut labels = scenario.issue_labels(number);
        labels.sort();
        assert_eq!(labels, ["architect-idea", "needs-triage"]);
        assert!(
            result.stderr.contains(&format!("skipped: {reason}")),
            "{}",
            result.stderr
        );
        assert!(
            result.stderr.contains("step after: 0s skipped"),
            "{}",
            result.stderr
        );
        let record = process_record(&scenario);
        assert_eq!(record["outcome"], "succeeded");
        assert_eq!(record["cause"], reason);
        assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
        let (_, ending) = saved_notification(&scenario, "process");
        assert!(ending.contains(&reason), "{ending}");
    }
}

#[test]
fn plan_only_marks_the_plan_ready_and_ends_without_dispatch() {
    let scenario = architecture_process();
    scenario.agent_does_in_session(1, PUBLISH_PLAN);

    let result = scenario.run(&["process", "architecture", "--plan-only"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.trim(), scenario.issue_url(8));
    assert_eq!(
        scenario.issue_labels(8),
        ["ready-for-agent", "architect-plan"]
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(result.stderr.contains("skipped: plan"), "{}", result.stderr);
    let reason = format!(
        "plan {} is ready for an agent; --plan-only requested",
        scenario.issue_url(8)
    );
    let record = process_record(&scenario);
    assert_eq!(record["outcome"], "succeeded");
    assert_eq!(record["cause"], reason);
    assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
    let (_, ending) = saved_notification(&scenario, "process");
    assert!(ending.contains(&reason), "{ending}");
}

#[test]
fn the_review_uses_its_own_harness_model_and_effort() {
    let scenario = architecture_process();
    write_process(&scenario, "architecture", &ARCHITECTURE.replace(
        "catalogue = \"architecture-review\"",
        "catalogue = \"architecture-review\"\nharness = 'codex'\nmodel = 'gpt-6.1-sol'\neffort = 'high'",
    ));
    scenario.agent_does(PUBLISH_PLAN);
    scenario.agent_does_for(8, &agent_lands_ticket(8, "main"));

    let checked = scenario.run(&["process", "check", "architecture"]);
    assert_eq!(checked.code, Some(0), "{}", checked.stderr);
    assert!(
        checked.stdout.contains("architecture: architecture-review"),
        "{}",
        checked.stdout
    );
    assert!(checked.stdout.contains("gpt-6.1-sol"), "{}", checked.stdout);
    let result = scenario.run(&["process", "architecture"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let codex = scenario.codex_calls();
    assert_eq!(codex.len(), 1);
    assert_flag(&codex[0], "-m", "gpt-6.1-sol");
    assert!(argv_of(&codex[0]).contains(&"model_reasoning_effort=\"high\"".to_string()));
    assert!(
        codex[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("thirdshift-improve-codebase-architecture")
    );
    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "the child inherits the Command's choice"
    );
}

#[test]
fn a_strong_spec_plan_starts_a_spec_run() {
    let scenario = architecture_process();
    scenario.agent_does_in_session(
        1,
        &format!(
            r#"
{PUBLISH_PLAN}
ticket=$(gh issue create --title "Move sessions" --body "The Ticket" --label ready-for-agent)
gh api --method POST repos/acme/widgets/issues/8/sub_issues -F sub_issue_id=${{ticket##*/}}
"#
        ),
    );
    scenario.agent_does_for(9, &agent_lands_ticket(9, "issue-8"));

    let result = scenario.run(&["process", "architecture"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let record = process_record(&scenario);
    assert_eq!(record["dispatched"][0]["kind"], "Spec run");
    assert_eq!(record["dispatched"][0]["issue"]["number"], 8);
    assert_eq!(record["dispatched"][0]["outcome"], "ready for review");
    let pr = pr_from(&scenario, "issue-8");
    assert_eq!(result.stdout.trim(), pr["url"].as_str().unwrap());
    assert_eq!(pr["base"], "main");
}

#[test]
fn an_invalid_or_failed_review_never_labels_or_dispatches_a_plan() {
    for (script, reason) in [
        ("printf malformed > \"$FAKE_CLAUDE_FINAL_MESSAGE\"".to_string(), "ended without the final line"),
        ("printf 'Architecture review plan: https://github.com/acme/widgets/issues/7' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"".to_string(), "was created before"),
        ("printf 'Architecture review plan: https://github.com/other/widgets/issues/7' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"".to_string(), "not in the repository at origin"),
        (format!("{PUBLISH_PLAN}\nexit 3\n"), "claude exited 3"),
    ] {
        let scenario = architecture_process();
        scenario.agent_does_in_session(1, &script);

        let result = scenario.run(&["process", "architecture"]);

        assert_stopped_before_a_run(&scenario, &result, reason);
        assert!(!scenario.issue_labels(7).contains(&"architect-plan".to_string()));
        assert!(!scenario.issue_labels(8).contains(&"architect-plan".to_string()));
        assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
    }
}

#[test]
fn the_review_owns_a_clean_detached_checkout_and_removes_it() {
    let scenario = architecture_process();
    write_process(&scenario, "architecture", &ARCHITECTURE.replace(
        "[[steps]]\nname = \"architecture\"",
        "[[steps]]\nname = 'before'\ncommand = 'printf dirty > pollution.txt'\n\n[[steps]]\nname = \"architecture\"",
    ));
    scenario.agent_does_in_session(
        1,
        &format!(
            r#"
test ! -e pollution.txt || exit 91
test -z "$(git branch --show-current)" || exit 92
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" || exit 93
{PUBLISH_PLAN}
"#
        ),
    );

    let result = scenario.run(&["process", "architecture", "--plan-only"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.claude_calls();
    let review_checkout = Path::new(calls[0]["cwd"].as_str().unwrap());
    assert!(!review_checkout.exists(), "{}", result.stderr);
}

#[test]
fn invalid_placement_is_refused_before_a_session() {
    for (body, reason) in [
        (ARCHITECTURE.replace("catalogue = \"architecture-review\"", "catalogue = \"architecture-review\"\nparallel = 'reviews'"), "architecture-review cannot be in a parallel group"),
        ("version = 1\ndescription = 'Invalid architecture review'\n[worktrees.fix]\nkind = 'issue-branch'\n[[steps]]\nname = 'architecture'\ncatalogue = 'architecture-review'\nworktree = 'fix'\n[[steps]]\nname = 'ship'\ncatalogue = 'delivery'\nworktree = 'fix'\n".to_string(), "architecture-review needs a throwaway worktree"),
    ] {
        let scenario = Scenario::new();
        write_process(&scenario, "architecture", &body);
        let result = scenario.run(&["process", "check", "architecture", "issue", "7"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(reason), "{}", result.stderr);
        assert!(scenario.claude_calls().is_empty());
    }
}

#[test]
fn plan_only_requires_a_review_step_and_cannot_be_repeated_or_used_for_check() {
    for (body, args, code, reason) in [
        (
            REVIEW,
            vec!["process", "architecture", "--plan-only"],
            1,
            "--plan-only needs an architecture-review Step",
        ),
        (
            ARCHITECTURE,
            vec!["process", "architecture", "--plan-only", "--plan-only"],
            2,
            "repeated argument: --plan-only",
        ),
        (
            ARCHITECTURE,
            vec!["process", "check", "architecture", "--plan-only"],
            2,
            "usage: thirdshift process",
        ),
    ] {
        let scenario = Scenario::new();
        write_process(&scenario, "architecture", body);
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(code), "{}", result.stderr);
        assert!(result.stderr.contains(reason), "{}", result.stderr);
        assert!(scenario.claude_calls().is_empty());
    }
}
