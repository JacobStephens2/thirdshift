use super::*;

const LAND_TICKETS: &str = r#"version = 1
description = "Land the Spec's Tickets"

[worktrees.spec]
kind = "spec-branch"

[[steps]]
name = "land"
catalogue = "land-tickets"
worktree = "spec"
"#;

#[test]
fn a_spec_branch_lands_dependent_tickets_and_leaves_the_spec_pr_draft() {
    let scenario = Scenario::new();
    write_process(&scenario, "gather", LAND_TICKETS);
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[8])]);
    scenario.agent_does_for(8, &agent_lands_ticket(8, "issue-7"));
    scenario.agent_does_for(
        9,
        &format!("test -f issue-8.txt\n{}", agent_lands_ticket(9, "issue-7")),
    );

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let pr = pr_from(&scenario, "issue-7");
    assert_eq!(result.stdout.trim(), pr["url"].as_str().unwrap());
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], true);
    let body = pr["body"].as_str().unwrap();
    for ticket in [8, 9] {
        assert!(body.contains(&format!("- [x] #{ticket}")), "{body}");
        assert_eq!(
            scenario.origin_git(&["show", &format!("issue-7:issue-{ticket}.txt")]),
            format!("{ticket}\n")
        );
    }
    assert_eq!(scenario.claude_calls().len(), 2, "no Spec review session");
    scenario.assert_cleaned_up("issue-7");
    assert!(
        scenario
            .issue_labels(7)
            .contains(&"in-progress".to_string())
    );
    assert!(
        !scenario
            .issue_labels(7)
            .contains(&"agent-failed".to_string()),
        "a successful Process leaving a draft Spec PR is not a failed run"
    );
}

#[test]
fn every_ticket_runs_the_selected_custom_process() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "gather",
        &format!("{LAND_TICKETS}process = \"contractor\"\n"),
    );
    write_process(
        &scenario,
        "contractor",
        r#"version = 1
description = "Custom Ticket delivery"
[worktrees.ticket]
kind = "issue-branch"
[[steps]]
name = "change"
command = '''
set -e
number=${THIRDSHIFT_ISSUE_URL##*/}
if test "$number" = 9; then test -f custom-8.txt; fi
echo "$number" > "custom-$number.txt"
git add .
git commit -qm custom
'''
worktree = "ticket"
[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
[[steps]]
name = "merge"
catalogue = "self-merge"
worktree = "ticket"
"#,
    );
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[8])]);

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    for ticket in [8, 9] {
        assert_eq!(
            scenario.origin_git(&["show", &format!("issue-7:custom-{ticket}.txt")]),
            format!("{ticket}\n")
        );
        assert_eq!(
            pr_from(&scenario, &format!("issue-{ticket}"))["state"],
            "MERGED"
        );
    }
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(pr_from(&scenario, "issue-7")["isDraft"], true);
}

#[test]
fn branch_worktree_kinds_refuse_the_wrong_issue_shape_before_any_session() {
    let mixed = LAND_TICKETS.replace(
        "[[steps]]",
        "[worktrees.ticket]\nkind = \"issue-branch\"\n\n[[steps]]",
    );
    for check in [true, false] {
        for (body, is_spec, reason) in [
            (
                LAND_TICKETS,
                false,
                "Spec-branch worktree needs an issue with sub-issues",
            ),
            (
                SHIP,
                true,
                "Issue-branch worktree refuses an issue with sub-issues",
            ),
            (mixed.as_str(), true, "not both"),
        ] {
            let scenario = Scenario::new();
            if is_spec {
                scenario.spec_has_tickets(7, &[(8, &[])]);
            }
            write_process(&scenario, "gather", body);
            let result = if check {
                scenario.run(&["process", "check", "gather", "issue", "7"])
            } else {
                scenario.run(&["process", "gather", "issue", "7"])
            };
            assert_no_session(&scenario, &result);
            assert!(result.stderr.contains(reason), "{}", result.stderr);
            assert!(scenario.issue_labels(7).is_empty());
            assert_eq!(
                scenario
                    .launch_git(&["worktree", "list", "--porcelain"])
                    .matches("worktree ")
                    .count(),
                1
            );
        }
    }
}

#[test]
fn a_failed_ticket_stops_only_its_dependents_and_fails_the_process() {
    let scenario = Scenario::new();
    write_process(&scenario, "gather", LAND_TICKETS);
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[8]), (10, &[])]);
    scenario.agent_does_for(8, "exit 9");
    scenario.agent_does_for(10, &agent_lands_ticket(10, "issue-7"));

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("Tickets not done: #8"),
        "{}",
        result.stderr
    );
    let pr = pr_from(&scenario, "issue-7");
    assert_eq!(pr["isDraft"], true);
    let body = pr["body"].as_str().unwrap();
    for expected in ["- [ ] #8 failed", "- [ ] #9 blocked by #8", "- [x] #10"] {
        assert!(body.contains(expected), "{body}");
    }
    assert!(scenario.origin_log("issue-9").is_none());
    assert_eq!(
        scenario.origin_git(&["show", "issue-7:issue-10.txt"]),
        "10\n"
    );
    assert!(
        scenario
            .issue_labels(7)
            .contains(&"in-progress".to_string())
    );
}

#[test]
fn land_tickets_honors_the_configured_parallel_limit() {
    let scenario = Scenario::new();
    scenario.user_config_is("[spec]\nparallel = 2\n");
    write_process(&scenario, "gather", LAND_TICKETS);
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[]), (10, &[])]);
    for (ticket, other) in [(8, 9), (9, 8)] {
        scenario.agent_does_for(ticket, &format!(
            "touch '{started}'\nwhile ! test -f '{other}'; do sleep 0.02; done\ntouch '{both}'\nwhile ! test -f '{release}'; do sleep 0.02; done\n{land}",
            started = scenario.path(&format!("started-{ticket}")).display(),
            other = scenario.path(&format!("started-{other}")).display(),
            both = scenario.path("both").display(),
            release = scenario.path("release").display(),
            land = agent_lands_ticket(ticket, "issue-7"),
        ));
    }
    scenario.agent_does_for(
        10,
        &format!(
            "touch '{}'\n{}",
            scenario.path("third").display(),
            agent_lands_ticket(10, "issue-7")
        ),
    );

    let held = scenario.run_until(&["process", "gather", "issue", "7"], &[], "both");
    let third_started = scenario.path("third").exists();
    fs::write(scenario.path("release"), "").unwrap();
    let result = held.finish();

    assert!(!third_started, "the third Ticket exceeded parallel = 2");
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.path("third").exists());
    for ticket in [8, 9, 10] {
        assert_eq!(
            scenario.origin_git(&["show", &format!("issue-7:issue-{ticket}.txt")]),
            format!("{ticket}\n")
        );
    }
}

#[test]
fn a_custom_ticket_does_not_self_merge_after_the_spec_pr_closes() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "gather",
        &format!("{LAND_TICKETS}process = \"contractor\"\n"),
    );
    write_process(
        &scenario,
        "contractor",
        r#"version = 1
description = "Custom Ticket delivery"
[worktrees.ticket]
kind = "issue-branch"
[[steps]]
name = "change"
command = '''
set -e
number=${THIRDSHIFT_ISSUE_URL##*/}
echo "$number" > "custom-$number.txt"
git add .
git commit -qm custom
if test "$number" = 9; then gh fake pr issue-7 state '"CLOSED"'; fi
'''
worktree = "ticket"
[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
[[steps]]
name = "merge"
catalogue = "self-merge"
worktree = "ticket"
"#,
    );
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[8])]);

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_eq!(pr_from(&scenario, "issue-7")["state"], "CLOSED");
    assert_eq!(pr_from(&scenario, "issue-8")["state"], "MERGED");
    assert_ne!(
        pr_from(&scenario, "issue-9")["state"],
        "MERGED",
        "ticket self-merged after the Spec PR closed: {}",
        result.stderr
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("was closed under this Spec run"),
        "{}",
        result.stderr
    );
}

#[test]
fn a_following_step_reads_the_landed_tickets_in_the_spec_worktree() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "gather",
        &format!(
            r#"{LAND_TICKETS}
[[steps]]
name = "inspect"
command = "cat issue-8.txt"
worktree = "spec"
"#
        ),
    );
    scenario.spec_has_tickets(7, &[(8, &[])]);
    scenario.agent_does_for(8, &agent_lands_ticket(8, "issue-7"));

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.trim(), "8");
}

#[test]
fn the_ticket_refusal_rules_apply_to_custom_spec_processes() {
    for refusal in [
        "closed",
        "foreign",
        "unready",
        "spec",
        "blocked",
        "untrusted",
    ] {
        let scenario = Scenario::new();
        write_process(&scenario, "gather", LAND_TICKETS);
        scenario.spec_has_tickets(7, &[(8, &[]), (9, &[])]);
        scenario.agent_does_for(9, &agent_lands_ticket(9, "issue-7"));
        match refusal {
            "closed" => scenario.issue_is(8, "CLOSED"),
            "foreign" => {
                let mut state = scenario.gh_state();
                state["issue_urls"] =
                    serde_json::json!({"8": "https://github.com/other/repo/issues/8"});
                scenario.write_gh_state(&state);
            }
            "unready" => scenario.issue_labelled(8, &["needs-triage"]),
            "spec" => scenario.spec_has_tickets(8, &[(10, &[])]),
            "blocked" => scenario.issue_blocked_by(8, &[99]),
            "untrusted" => {
                let mut state = scenario.gh_state();
                state["author_associations"] = serde_json::json!({"8": "CONTRIBUTOR"});
                scenario.write_gh_state(&state);
            }
            _ => unreachable!(),
        }

        let result = scenario.run(&["process", "gather", "issue", "7"]);

        assert_eq!(
            result.code,
            Some(if refusal == "closed" { 0 } else { 1 }),
            "{refusal}: {}",
            result.stderr
        );
        assert!(scenario.origin_log("issue-8").is_none(), "{refusal}");
        for call in scenario.claude_calls() {
            assert_ne!(call["branch"], "issue-8", "{refusal}: {call}");
        }
        if refusal == "untrusted" {
            assert!(
                result.stderr.contains("not a Trusted account"),
                "{}",
                result.stderr
            );
            assert!(scenario.claude_calls().is_empty());
        }
    }
}

#[test]
fn spec_process_preflight_and_claim_failure_start_no_session_or_branch_worktree() {
    for check in [false, true] {
        for child in ["absent", "notes", "gather"] {
            let scenario = Scenario::new();
            scenario.spec_has_tickets(7, &[(8, &[])]);
            write_process(&scenario, "notes", REVIEW);
            write_process(
                &scenario,
                "gather",
                &format!("{LAND_TICKETS}process = \"{child}\"\n"),
            );
            let result = if check {
                scenario.run(&["process", "check", "gather", "issue", "7"])
            } else {
                scenario.run(&["process", "gather", "issue", "7"])
            };
            assert_no_session(&scenario, &result);
            assert!(
                result.stderr.contains(if child == "absent" {
                    "unknown process absent"
                } else {
                    "needs an Issue-branch worktree"
                }),
                "{}",
                result.stderr
            );
            assert!(scenario.issue_labels(7).is_empty());
        }
        let scenario = Scenario::new();
        write_process(&scenario, "gather", LAND_TICKETS);
        let result = if check {
            scenario.run(&["process", "check", "gather"])
        } else {
            scenario.run(&["process", "gather"])
        };
        assert_no_session(&scenario, &result);
        assert!(
            result
                .stderr
                .contains("Spec-branch worktree needs an issue"),
            "{}",
            result.stderr
        );
    }

    let scenario = Scenario::new();
    scenario.spec_has_tickets(7, &[(8, &[])]);
    write_process(&scenario, "gather", LAND_TICKETS);
    scenario.gh_fails("api --method PUT repos/acme/widgets/issues/7/labels");

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_no_session(&scenario, &result);
    assert!(result.stderr.contains("Claim"), "{}", result.stderr);
    assert!(
        !result.stderr.contains("creating worktree"),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[test]
fn a_no_security_review_override_reaches_the_ticket_children() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[harness]\ndefault = 'codex'\n[security]\nharness = 'codex'\nreview = true\n",
    );
    write_process(&scenario, "gather", LAND_TICKETS);
    scenario.spec_has_tickets(7, &[(8, &[])]);
    scenario.agent_does_for(8, &agent_lands_ticket(8, "issue-7"));

    let result = scenario.run_with_env(
        &[
            "process",
            "gather",
            "issue",
            "7",
            "--harness",
            "claude",
            "no-security-review",
        ],
        &[("FAKE_CODEX_SIGNED_OUT", "1")],
    );

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(pr_from(&scenario, "issue-8")["state"], "MERGED");
}

#[test]
fn a_spec_branch_keeps_committed_work_when_a_step_before_land_fails() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(7, &[(8, &[])]);
    let source = LAND_TICKETS.replacen(
        "[[steps]]",
        r#"[[steps]]
name = "prepare"
command = "echo proof > proof.txt; git add proof.txt; git commit -qm proof; exit 23"
worktree = "spec"

[[steps]]"#,
        1,
    );
    write_process(&scenario, "gather", &source);

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let preserved = preserved_spec_proof(&scenario);
    assert_eq!(
        preserved.as_deref().map(str::trim),
        Some("proof"),
        "the failed Spec-branch Step lost its committed work: {}",
        result.stderr
    );
}

#[test]
fn a_successful_spec_branch_keeps_work_added_after_land() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(7, &[(8, &[])]);
    scenario.agent_does_for(8, &agent_lands_ticket(8, "issue-7"));
    let source = format!(
        r#"{LAND_TICKETS}
[[steps]]
name = "finish"
command = "echo proof > proof.txt; git add proof.txt; git commit -qm proof"
worktree = "spec"
"#
    );
    write_process(&scenario, "gather", &source);

    let result = scenario.run(&["process", "gather", "issue", "7"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let preserved = preserved_spec_proof(&scenario);
    assert_eq!(
        preserved.as_deref().map(str::trim),
        Some("proof"),
        "the successful Spec-branch Step lost its committed work: {}",
        result.stderr
    );
}

fn preserved_spec_proof(scenario: &Scenario) -> Option<String> {
    scenario.origin_file("issue-7", "proof.txt").or_else(|| {
        if scenario
            .launch_git(&["branch", "--list", "issue-7"])
            .trim()
            .is_empty()
        {
            None
        } else {
            Some(scenario.launch_git(&["show", "issue-7:proof.txt"]))
        }
    })
}
