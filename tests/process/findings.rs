use super::*;
use serde_json::json;

const FINDINGS: &str = r#"version = 1
description = "Fix reproduced findings and close merged fixes"
[[steps]]
name = "fix"
catalogue = "fix-findings"
[[steps]]
name = "close"
catalogue = "close-findings"
"#;

fn findings() -> Scenario {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    write_process(&scenario, "findings", FINDINGS);
    let mut state = scenario.gh_state();
    state["advisories"] = json!(["reproduced high single", "not reproduced", "reproduced low single"]
        .iter().enumerate().map(|(index, outcome)| json!({
            "ghsa_id": format!("GHSA-finding-{index}"), "state": "draft",
            "summary": format!("Finding {index}"),
            "html_url": format!("https://github.com/acme/widgets/security/advisories/GHSA-finding-{index}"),
            "description": format!("Private candidate write-up.\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: {outcome}\n\n### Proof-of-concept test\n\n```\nbounded_fixture();\n```\n")
        })).collect::<Vec<_>>());
    scenario.write_gh_state(&state);
    scenario
}

fn stage_fix(record: usize) -> String {
    format!(
        r#"printf 'Proposed public issue:\nTitle: Bound accepted input\nBody:\nReject oversized input. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-{record}\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
    )
}

fn implement(issue: usize) -> String {
    format!(
        r#"echo bounded > bounded-{issue}.txt
git add bounded-{issue}.txt
git commit -qm 'Bound accepted input'
gh pr create --base main --head issue-{issue} --title Fix --body 'Closes #{issue}'
"#
    )
}

#[test]
fn otlp_never_receives_private_finding_sessions_or_nested_ticket_runs() {
    for spec in [false, true] {
        let receiver = ResendStandIn::replying(200, "{}");
        let scenario = findings();
        let mut state = scenario.gh_state();
        state["advisories"].as_array_mut().unwrap().truncate(1);
        if spec {
            state["advisories"][0]["description"] = json!(
                state["advisories"][0]["description"]
                    .as_str()
                    .unwrap()
                    .replace("high single", "high spec")
            );
        }
        scenario.write_gh_state(&state);
        scenario.user_config_is(&format!(
            "[otlp]\nendpoint = \"{}/v1/traces\"\n",
            receiver.url()
        ));
        if spec {
            scenario.agent_does_in_session(
                1,
                r#"printf 'Proposed public spec:\nTitle: Bound accepted input\nBody:\nBound input across storage and transport. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\nTicket: Bound storage input\nBody:\nBound storage input. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\nBlocked by: none\nTicket: Bound transport input\nBody:\nBound transport input. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\nBlocked by: 1\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
            );
            for ticket in [9, 10] {
                scenario.agent_does_for(
                    ticket,
                    &implement(ticket as usize).replace("--base main", "--base issue-8"),
                );
            }
        } else {
            scenario.agent_does_in_session(1, &stage_fix(0));
            scenario.agent_does_for(8, &implement(8));
            scenario.agent_does_for_in_session(
                8,
                2,
                "printf 'Security close check: passes\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
            );
        }

        let args: &[&str] = if spec {
            &["process", "findings", "security-fix", "no-security-close"]
        } else {
            &["process", "findings", "security-fix", "merge"]
        };
        let result = scenario.run(args);

        assert_eq!(result.code, Some(0), "spec={spec}: {}", result.stderr);
        let local_traces = scenario
            .entries("home/.thirdshift/logs/acme/widgets/commands/process")
            .into_iter()
            .filter(|name| name.ends_with(".spans.jsonl"))
            .count();
        assert!(
            local_traces >= 2,
            "spec={spec}: private finding sessions were not exercised: {local_traces}"
        );
        let requests = receiver.requests();
        let exported = requests
            .iter()
            .map(|request| request.body.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            requests.is_empty(),
            "spec={spec}: private finding sessions were exported: {exported}"
        );
    }
}

#[test]
fn fixing_dispatches_each_reproduced_finding_and_keeps_unreproduced_findings_private() {
    let scenario = findings();
    scenario.user_config_is("[security]\nfix = true\n");
    scenario.agent_does_in_session(1, &stage_fix(0));
    scenario.agent_does_for(8, &implement(8));
    scenario.agent_does_in_session(3, &stage_fix(2));
    scenario.agent_does_for(9, &implement(9));

    let result = scenario.run(&["process", "findings"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    for (record, issue) in [(0, 8), (2, 9)] {
        let description = state["advisories"][record]["description"].as_str().unwrap();
        assert!(
            description.contains(&scenario.issue_url(issue)),
            "{description}"
        );
        assert!(description.contains("Fix Run: succeeded"), "{description}");
        let body = state["bodies"][issue.to_string()].as_str().unwrap();
        assert!(!body.contains("Private candidate write-up"), "{body}");
        assert!(!body.contains("bounded_fixture"), "{body}");
    }
    assert_eq!(state["prs"].as_array().unwrap().len(), 2);
    assert!(
        !state["advisories"][1]["description"]
            .as_str()
            .unwrap()
            .contains("Fix Ticket:")
    );
    assert_eq!(
        scenario.claude_calls().len(),
        4,
        "unmerged fixes must not be rechecked"
    );
}

#[test]
fn fixing_requires_permission_and_the_command_overrides_the_user_config() {
    for (config, args) in [
        ("", vec!["process", "findings"]),
        (
            "[security]\nfix = true\n",
            vec!["process", "findings", "no-security-fix", "security-close"],
        ),
    ] {
        let scenario = findings();
        scenario.user_config_is(config);
        let before = scenario.gh_state();
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(scenario.claude_calls().is_empty());
        assert_eq!(scenario.gh_state()["advisories"], before["advisories"]);
        assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
    }
}

#[test]
fn closing_rechecks_each_merged_base_and_leaves_a_failing_finding_for_the_day_shift() {
    let scenario = findings();
    scenario.user_config_is("[security]\nfix = false\n");
    scenario.agent_does_in_session(1, &stage_fix(0));
    scenario.agent_does_for(8, &implement(8));
    scenario.agent_does_in_session(3, &stage_fix(2));
    scenario.agent_does_for(9, &implement(9));
    for (issue, outcome) in [(8, "passes"), (9, "fails: the fixture still fails")] {
        scenario.agent_does_for_in_session(issue, 2, &format!(r#"
test -f bounded-8.txt && test -f bounded-9.txt
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)"
printf '%s\n' 'Command: bounded-fixture; actual result recorded privately' 'Security close check: {outcome}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#));
    }

    let result = scenario.run(&["process", "findings", "security-fix", "merge"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(
        state["advisories"][0]["state"], "closed",
        "{}",
        result.stderr
    );
    assert_eq!(state["advisories"][2]["state"], "draft");
    let body = state["advisories"][2]["description"].as_str().unwrap();
    assert!(body.contains("the fixture still fails"), "{body}");
    assert!(
        result.stderr.contains("left GHSA-finding-2 open"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 6);
    assert!(
        state["prs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pr| pr["state"] == "MERGED")
    );
}

#[test]
fn closing_can_be_disabled_for_a_merged_fix() {
    let scenario = findings();
    let mut state = scenario.gh_state();
    state["advisories"].as_array_mut().unwrap().truncate(1);
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(1, &stage_fix(0));
    scenario.agent_does_for(8, &implement(8));

    let result = scenario.run(&[
        "process",
        "findings",
        "security-fix",
        "merge",
        "no-security-close",
    ]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_eq!(scenario.gh_state()["advisories"][0]["state"], "draft");
    assert_eq!(scenario.claude_calls().len(), 2);
}

#[test]
fn a_failed_fix_records_its_ending_and_is_never_retried_unattended() {
    let scenario = findings();
    scenario.user_config_is("[security]\nfix = true\n");
    scenario.agent_does_in_session(1, &stage_fix(0));
    scenario.agent_does_for(8, "exit 1");

    let failed = scenario.run(&["process", "findings"]);

    assert_eq!(failed.code, Some(1), "{}", failed.stderr);
    let mut state = scenario.gh_state();
    assert!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .contains("Fix Run: failed")
    );
    assert!(scenario.issue_labels(8).contains(&"in-progress".into()));
    // Remove other pending findings: the failed fix itself must stay with the Day shift.
    state["advisories"].as_array_mut().unwrap().truncate(1);
    scenario.write_gh_state(&state);
    let calls = scenario.claude_calls().len();
    let next = scenario.run(&["process", "findings"]);
    assert_eq!(next.code, Some(0), "{}", next.stderr);
    assert_eq!(scenario.claude_calls().len(), calls);
}

#[test]
fn fixing_runs_the_selected_custom_ticket_process() {
    let scenario = findings();
    let mut state = scenario.gh_state();
    state["advisories"].as_array_mut().unwrap().truncate(1);
    scenario.write_gh_state(&state);
    write_process(
        &scenario,
        "findings",
        &FINDINGS.replace(
            "catalogue = \"fix-findings\"",
            "catalogue = \"fix-findings\"\nprocess = \"contractor\"",
        ),
    );
    write_process(
        &scenario,
        "contractor",
        r#"version = 1
description = "Custom fix"
[worktrees.ticket]
kind = "issue-branch"
[[steps]]
name = "edit"
command = "echo contractor > contractor.txt; git add contractor.txt; git commit -qm contractor; gh pr create --base main --head issue-8 --title Fix --body 'Closes #8'"
worktree = "ticket"
[[steps]]
name = "deliver"
catalogue = "delivery"
worktree = "ticket"
"#,
    );
    scenario.agent_does_in_session(1, &stage_fix(0));

    let result = scenario.run(&["process", "findings", "security-fix"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.origin_git(&["show", "issue-8:contractor.txt"]),
        "contractor\n"
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    let record = process_record(&scenario);
    assert_eq!(
        record["dispatched"][0]["process_name"], "contractor",
        "{record}"
    );
}

#[test]
fn a_private_finding_is_reused_and_closed_only_after_a_merged_base_check() {
    let scenario = findings();
    let mut state = scenario.gh_state();
    state["private"] = json!(true);
    state["bodies"]["7"] = state["advisories"][0]["description"].clone();
    state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
    let evidence = state["bodies"]["7"].as_str().unwrap().to_owned();
    scenario.write_gh_state(&state);
    scenario.agent_does_for(7, &implement(7));
    scenario.agent_does_for_in_session(
        7,
        2,
        r#"test -f bounded-7.txt
printf 'Security close check: passes\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["process", "findings", "security-fix", "merge"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["issues"]["7"], "CLOSED");
    let body = state["bodies"]["7"].as_str().unwrap();
    assert!(body.starts_with(&evidence), "{body}");
    assert!(body.contains("Closed by:"), "{body}\n{}", result.stderr);
    assert_eq!(state["issues"].as_object().unwrap().len(), 1);
    assert_eq!(scenario.claude_calls().len(), 2);
}

#[test]
fn a_reproduced_spec_fix_publishes_tickets_and_dispatches_a_spec_run() {
    let scenario = findings();
    let mut state = scenario.gh_state();
    state["advisories"].as_array_mut().unwrap().truncate(1);
    state["advisories"][0]["description"] = json!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .replace("high single", "high spec")
    );
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(1, r#"printf 'Proposed public spec:\nTitle: Bound accepted input\nBody:\nBound input across storage and transport. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\nTicket: Bound storage input\nBody:\nBound storage input. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\nBlocked by: none\nTicket: Bound transport input\nBody:\nBound transport input. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\nBlocked by: 1\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    for ticket in [9, 10] {
        scenario.agent_does_for(
            ticket,
            &implement(ticket as usize).replace("--base main", "--base issue-8"),
        );
    }

    let result = scenario.run(&["process", "findings", "security-fix"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let pr = pr_from(&scenario, "issue-8");
    assert_eq!(pr["base"], "main");
    assert_eq!(pr["isDraft"], false);
    assert_eq!(state["sub_issues"]["8"], json!([9, 10]));
    assert!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .contains("Fix Run: succeeded")
    );
    assert_eq!(
        process_record(&scenario)["dispatched"][0]["kind"],
        "Spec run"
    );
}

#[test]
fn finding_steps_reject_parallel_groups_and_invalid_child_processes_before_sessions() {
    for (text, reason) in [
        (
            FINDINGS.replace(
                "catalogue = \"fix-findings\"",
                "catalogue = \"fix-findings\"\nparallel = \"same\"",
            ),
            "fix-findings cannot be in a parallel group",
        ),
        (
            FINDINGS.replace(
                "catalogue = \"close-findings\"",
                "catalogue = \"close-findings\"\nparallel = \"same\"",
            ),
            "close-findings cannot be in a parallel group",
        ),
        (
            FINDINGS.replace(
                "catalogue = \"fix-findings\"",
                "catalogue = \"fix-findings\"\nprocess = \"absent\"",
            ),
            "unknown process absent",
        ),
        (
            FINDINGS.replace(
                "catalogue = \"fix-findings\"",
                "catalogue = \"fix-findings\"\nprocess = \"notes\"",
            ),
            "the child Ticket Process notes needs an Issue-branch worktree",
        ),
    ] {
        let scenario = findings();
        write_process(&scenario, "findings", &text);
        write_process(&scenario, "notes", REVIEW);
        let result = scenario.run(&["process", "check", "findings"]);
        assert_no_session(&scenario, &result);
        assert!(result.stderr.contains(reason), "{}", result.stderr);
    }
}

#[test]
fn failed_fix_publication_does_not_email_private_background_work() {
    let scenario = findings();
    let secret = "Private exploit trace: POST /admin/debug with token";
    scenario.agent_does(&support::leaves_running(secret));
    let resend = ResendStandIn::replying(200, ACCEPTED);
    let result = scenario.run_with_env(
        &[
            "process",
            "findings",
            "security-fix",
            "email",
            "me@example.com",
        ],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", KEY),
        ],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2, "{}", result.stderr);
    assert!(result.stderr.contains(secret), "{}", result.stderr);
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(
        !requests[0].body.to_string().contains(secret),
        "private Security publication evidence leaked to Resend: {}",
        requests[0].body
    );
}

#[test]
fn merged_fix_is_rechecked_after_a_later_fix_fails() {
    let scenario = findings();
    scenario.agent_does_in_session(1, &stage_fix(0));
    scenario.agent_does_for(8, &implement(8));
    scenario.agent_does_in_session(3, &stage_fix(2));
    scenario.agent_does_for(9, "exit 1");
    scenario.agent_does_in_session(
        5,
        "printf 'Security close check: passes\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );
    scenario.agent_does_for_in_session(
        8,
        2,
        "printf 'Security close check: passes\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );
    let failed = scenario.run(&["process", "findings", "security-fix", "merge"]);
    assert_eq!(failed.code, Some(1), "{}", failed.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    let retry = scenario.run(&["process", "findings", "security-fix", "merge"]);
    assert_eq!(retry.code, Some(0), "{}", retry.stderr);
    assert_eq!(
        scenario.gh_state()["advisories"][0]["state"],
        "closed",
        "{}",
        retry.stderr
    );
    assert_eq!(scenario.gh_state()["advisories"][2]["state"], "draft");
}

#[test]
fn failed_close_check_keeps_a_private_finding_open() {
    let scenario = findings();
    let mut state = scenario.gh_state();
    state["private"] = json!(true);
    state["bodies"]["7"] = state["advisories"][0]["description"].clone();
    state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
    scenario.write_gh_state(&state);
    scenario.agent_does_for(7, &implement(7));
    scenario.agent_does_for_in_session(
        7,
        2,
        r#"printf 'Security close check: fails: bounded fixture still fails\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = scenario.run(&["process", "findings", "security-fix", "merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2, "{}", result.stderr);
    let state = scenario.gh_state();
    let body = state["bodies"]["7"].as_str().unwrap();
    assert!(body.contains("Security close left open:"), "{body}");
    assert!(!body.contains("Closed by:"), "{body}");
    assert_eq!(state["issues"]["7"], "OPEN", "{}", result.stderr);
}

#[test]
fn close_accepts_a_fix_link_with_different_repository_case() {
    let scenario = findings();
    let mut state = scenario.gh_state();
    state["advisories"].as_array_mut().unwrap().truncate(1);
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(1, &stage_fix(0));
    scenario.agent_does_for(8, &implement(8));
    let first = scenario.run(&[
        "process",
        "findings",
        "security-fix",
        "merge",
        "no-security-close",
    ]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let mut state = scenario.gh_state();
    let description = state["advisories"][0]["description"]
        .as_str()
        .unwrap()
        .replace(
            "https://github.com/acme/widgets/issues/8",
            "https://github.com/ACME/widgets/issues/8",
        );
    state["advisories"][0]["description"] = json!(description);
    scenario.write_gh_state(&state);
    scenario.agent_does_for_in_session(
        8,
        2,
        r#"printf 'Security close check: passes\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let closed = scenario.run(&["process", "findings", "security-fix"]);
    assert_eq!(closed.code, Some(0), "{}", closed.stderr);
    assert_eq!(scenario.gh_state()["advisories"][0]["state"], "closed");
}

#[test]
fn disabling_close_keeps_a_private_finding_open() {
    let scenario = findings();
    let mut state = scenario.gh_state();
    state["private"] = json!(true);
    state["bodies"]["7"] = state["advisories"][0]["description"].clone();
    state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
    scenario.write_gh_state(&state);
    scenario.agent_does_for(7, &implement(7));
    let result = scenario.run(&[
        "process",
        "findings",
        "security-fix",
        "merge",
        "no-security-close",
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1, "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["issues"]["7"],
        "OPEN",
        "{}",
        result.stderr
    );
}
