//! Spec runs: `thirdshift <Issue URL>` on a Spec, an issue with sub-issues,
//! works through its Tickets in dependency order, each a Merge run into the
//! Spec branch, then opens the Spec PR from the Spec branch into the Base
//! branch, ready for review.

mod support;

use support::Scenario;
use support::resend::ResendStandIn;

const SPEC: u32 = 20;
const SPEC_TITLE: &str = "Widgets, all of them";

/// The agent for Ticket `ticket` commits `file` and opens its PR, closing
/// the Ticket, into the Spec branch.
fn agent_lands(ticket: u32, file: &str) -> String {
    format!(
        r#"
echo "{ticket}" > {file}
git add {file}
git commit -q -m "Ticket {ticket}"
gh pr create --base issue-{SPEC} --head issue-{ticket} --title "Ticket {ticket}" --body "Closes #{ticket}"
"#
    )
}

/// A Spec #20 with a linear chain of Tickets: #21, then #22, blocked by #21,
/// and #23, already closed. Each open Ticket's agent lands its own file, and
/// #22's first checks that #21's work is already on its branch.
fn linear_spec() -> Scenario {
    let scenario = Scenario::new();
    scenario.issue_titled(SPEC, SPEC_TITLE);
    scenario.spec_has_tickets(SPEC, &[(21, &[]), (22, &[21]), (23, &[])]);
    scenario.issue_is(23, "CLOSED");
    scenario.agent_does_for(21, &agent_lands(21, "first.txt"));
    scenario.agent_does_for(
        22,
        &format!("test -f first.txt\n{}", agent_lands(22, "second.txt")),
    );
    scenario
}

fn spec_url(scenario: &Scenario) -> String {
    scenario.issue_url(SPEC)
}

/// The issue number each agent session was for, in order.
fn sessions_by_issue(scenario: &Scenario) -> Vec<String> {
    scenario
        .claude_calls()
        .iter()
        .map(|call| {
            let prompt = call["prompt"].as_str().unwrap();
            let url = prompt.split_whitespace().nth(1).unwrap();
            url.rsplit('/').next().unwrap().to_string()
        })
        .collect()
}

fn assert_contains(text: &str, part: &str) {
    assert!(text.contains(part), "expected {part:?} in: {text}");
}

#[test]
fn a_linear_spec_lands_each_ticket_in_order_then_opens_a_ready_spec_pr() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let spec_pr = "https://github.com/acme/widgets/pull/3";
    assert_eq!(result.stdout, format!("{spec_pr}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {spec_pr} is ready for review").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(sessions_by_issue(&scenario), ["21", "22"]);

    let gh = scenario.gh_state();
    for (i, ticket) in [21, 22].into_iter().enumerate() {
        let pr = &gh["prs"][i];
        assert_eq!(pr["head"], format!("issue-{ticket}"));
        assert_eq!(pr["base"], "issue-20");
        assert_eq!(pr["state"], "MERGED");
        assert_eq!(gh["issues"][ticket.to_string()], "CLOSED");
    }
    let spec = &gh["prs"][2];
    assert_eq!(spec["url"], spec_pr);
    assert_eq!(spec["head"], "issue-20");
    assert_eq!(spec["base"], "main");
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], false);
    assert_eq!(spec["title"], SPEC_TITLE);
    assert_contains(spec["body"].as_str().unwrap(), "Closes #20");
    assert_eq!(gh["issues"]["20"], "OPEN");

    assert_eq!(
        scenario.origin_file("issue-20", "second.txt").as_deref(),
        Some("22\n")
    );
    assert_eq!(scenario.origin_file("main", "first.txt"), None);
    scenario.assert_cleaned_up("issue-20");
}

#[test]
fn each_ticket_branches_off_the_spec_branch_and_its_prompts_name_it_as_the_base() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    for ticket in [21, 22] {
        assert_contains(
            &result.stderr,
            &format!("thirdshift: #{ticket}: creating worktree"),
        );
        assert_contains(
            &result.stderr,
            &format!("on issue-{ticket} from origin/issue-20\n"),
        );
    }
    for call in scenario.claude_calls() {
        assert_contains(
            call["prompt"].as_str().unwrap(),
            "The base branch is issue-20.",
        );
    }
}

#[test]
fn closed_tickets_are_not_run() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(!sessions_by_issue(&scenario).contains(&"23".to_string()));
    assert!(!result.stderr.contains("#23"), "stderr: {}", result.stderr);
}

#[test]
fn the_spec_branch_is_pushed_before_the_first_ticket_starts() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        21,
        &format!(
            "git ls-remote --exit-code origin refs/heads/issue-20 >/dev/null\n{}",
            agent_lands(21, "first.txt")
        ),
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let pushed = result.stderr.find("thirdshift: pushing issue-20").unwrap();
    let started = result.stderr.find("thirdshift: starting #21").unwrap();
    assert!(pushed < started, "stderr: {}", result.stderr);
}

#[test]
fn ticket_lines_are_relayed_with_the_ticket_number_and_the_spec_run_says_what_it_does() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    for line in [
        "thirdshift: starting #21\n",
        "thirdshift: #21: implement: session started\n",
        "thirdshift: #21: PR https://github.com/acme/widgets/pull/1 is merged\n",
        "thirdshift: #21 landed\n",
        "thirdshift: starting #22\n",
        "thirdshift: #22 landed\n",
    ] {
        assert_contains(&result.stderr, line);
    }
    let first_landed = result.stderr.find("#21 landed").unwrap();
    let second_started = result.stderr.find("starting #22").unwrap();
    assert!(first_landed < second_started, "stderr: {}", result.stderr);
    // A Ticket's own stderr never reaches the terminal unprefixed.
    assert!(
        !result.stderr.contains("thirdshift: thirdshift:"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn ticket_runs_send_no_run_notification() {
    let scenario = linear_spec();
    scenario.user_config_is("[email]\nalways = true\nto = \"me@example.com\"\n");
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = scenario.run_with_env(
        &[&spec_url(&scenario)],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_contains(
        requests[0].body["subject"].as_str().unwrap(),
        "acme/widgets#20",
    );
}

#[test]
fn ticket_runs_leave_the_launch_directory_to_the_spec_run() {
    let scenario = linear_spec();
    scenario.origin_has_commit("main", "README.md", "widgets, updated\n", "Upstream work");
    scenario.user_config_is("[launch]\npull = true\n");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        result
            .stderr
            .matches("in the Launch directory from")
            .count(),
        1,
        "stderr: {}",
        result.stderr
    );
    assert_contains(
        &result.stderr,
        "thirdshift: updating main in the Launch directory from origin/main\n",
    );
}

#[test]
fn a_merge_ask_from_the_user_config_leaves_the_spec_pr_ready_for_review() {
    let scenario = linear_spec();
    scenario.user_config_is("[merge]\nalways = true\n");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let spec = &scenario.gh_state()["prs"][2];
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], false);
}

#[test]
fn a_failed_ticket_ends_the_spec_run_before_the_tickets_it_blocks() {
    let scenario = linear_spec();
    scenario.agent_does_for(21, "exit 1");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(sessions_by_issue(&scenario), ["21"]);
    assert_contains(&result.stderr, "thirdshift: #21 failed");
    let gh = scenario.gh_state();
    assert!(
        gh["prs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pr| pr["head"] != "issue-20"),
        "a Spec PR was opened: {}",
        gh["prs"]
    );
}

#[test]
fn an_issue_with_no_sub_issues_is_an_ordinary_run() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#,
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(sessions_by_issue(&scenario), ["7"]);
    assert!(!result.stderr.contains("#7:"), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["base"], "main");
}

#[test]
fn help_does_not_mention_the_ticket_runs_hidden_argument() {
    let scenario = Scenario::new();

    let result = scenario.run(&["help"]);

    assert_eq!(result.code, Some(0));
    assert!(!result.stdout.contains("spec-branch"), "{}", result.stdout);
}

/// A Spec #20 whose Tickets are `tickets`, each with the issues it is
/// blocked by, and whose open Tickets' agents each land their own file.
fn spec_of(tickets: &[(u32, &[u32])]) -> Scenario {
    let scenario = Scenario::new();
    scenario.issue_titled(SPEC, SPEC_TITLE);
    scenario.spec_has_tickets(SPEC, tickets);
    for (ticket, _) in tickets {
        scenario.agent_does_for(*ticket, &agent_lands(*ticket, &format!("{ticket}.txt")));
    }
    scenario
}

/// Assert the Spec run ended as a Failed spec run: exit 1, nothing on
/// stdout, and no Spec PR or Spec review.
fn assert_failed_spec_run(scenario: &Scenario, result: &support::RunResult) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        !sessions_by_issue(scenario).contains(&SPEC.to_string()),
        "a Spec review was started"
    );
    let gh = scenario.gh_state();
    assert!(
        gh["prs"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|pr| pr["head"] != "issue-20"),
        "a Spec PR was opened: {}",
        gh["prs"]
    );
}

#[test]
fn a_needs_info_ticket_and_what_it_blocks_are_not_run_while_an_independent_ticket_lands() {
    let scenario = spec_of(&[(21, &[]), (22, &[21]), (23, &[])]);
    scenario.issue_labelled(21, &["needs-info"]);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(&scenario, &result);
    assert_eq!(sessions_by_issue(&scenario), ["23"]);
    assert_eq!(scenario.gh_state()["issues"]["23"], "CLOSED");
    for line in [
        "thirdshift: #21 unready: labelled needs-info\n",
        "thirdshift: #22 blocked by #21\n",
        "thirdshift: #23 landed with https://github.com/acme/widgets/pull/1\n",
    ] {
        assert_contains(&result.stderr, line);
    }
}

#[test]
fn each_unready_label_keeps_a_ticket_from_running() {
    for label in ["ready-for-human", "wontfix", "needs-triage"] {
        let scenario = spec_of(&[(21, &[]), (22, &[])]);
        scenario.issue_labelled(21, &[label]);
        scenario.issue_labelled(22, &["ready-for-agent"]);

        let result = scenario.run(&[&spec_url(&scenario)]);

        assert_failed_spec_run(&scenario, &result);
        assert_eq!(sessions_by_issue(&scenario), ["22"], "{label}");
        assert_contains(
            &result.stderr,
            &format!("thirdshift: #21 unready: labelled {label}\n"),
        );
    }
}

#[test]
fn a_failed_ticket_stops_only_its_dependents_and_is_not_started_again() {
    let scenario = spec_of(&[(21, &[]), (22, &[21]), (23, &[]), (24, &[])]);
    scenario.agent_does_for(21, "exit 1");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(&scenario, &result);
    assert_eq!(sessions_by_issue(&scenario), ["21", "23", "24"]);
    let gh = scenario.gh_state();
    assert_eq!(gh["issues"]["23"], "CLOSED");
    assert_eq!(gh["issues"]["24"], "CLOSED");
    let failed = result
        .stderr
        .lines()
        .rfind(|line| line.starts_with("thirdshift: #21 failed: "))
        .unwrap_or_else(|| panic!("no failed line for #21 in: {}", result.stderr));
    assert!(failed.contains("session log: "), "{failed}");
    let log = failed.rsplit("session log: ").next().unwrap();
    assert!(
        std::path::Path::new(log.trim_end_matches(')')).exists(),
        "{failed}"
    );
    assert_contains(&result.stderr, "thirdshift: #22 blocked by #21\n");
}

#[test]
fn an_open_outside_blocker_holds_a_ticket_back_and_a_closed_one_does_not() {
    let scenario = spec_of(&[(21, &[99]), (22, &[98])]);
    scenario.issue_is(98, "CLOSED");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(&scenario, &result);
    assert_eq!(sessions_by_issue(&scenario), ["22"]);
    assert_contains(
        &result.stderr,
        "thirdshift: #21 blocked by #99 (outside the Spec)\n",
    );
}

#[test]
fn tickets_in_a_cycle_are_not_run_and_the_summary_names_the_cycle() {
    let scenario = spec_of(&[(21, &[22]), (22, &[21]), (23, &[21]), (24, &[])]);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(&scenario, &result);
    assert_eq!(sessions_by_issue(&scenario), ["24"]);
    for line in [
        "thirdshift: #21 in a cycle: #21 blocked by #22 blocked by #21\n",
        "thirdshift: #22 in a cycle: #22 blocked by #21 blocked by #22\n",
        "thirdshift: #23 blocked by #21\n",
    ] {
        assert_contains(&result.stderr, line);
    }
}

#[test]
fn a_ticket_with_its_own_sub_issues_is_unready() {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.spec_has_tickets(21, &[(30, &[])]);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_failed_spec_run(&scenario, &result);
    assert_eq!(sessions_by_issue(&scenario), ["22"]);
    assert_contains(&result.stderr, "thirdshift: #21 unready: has sub-issues\n");
}

#[test]
fn removing_a_needs_info_label_while_another_ticket_runs_lets_it_run() {
    let scenario = spec_of(&[(21, &[]), (22, &[])]);
    scenario.issue_labelled(21, &["needs-info"]);
    scenario.agent_does_for(
        22,
        &format!("gh fake labels 21 '[]'\n{}", agent_lands(22, "22.txt")),
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(sessions_by_issue(&scenario), ["22", "21"]);
}

#[test]
fn help_explains_unready_tickets_and_that_a_spec_run_takes_every_ticket_it_can_reach() {
    let scenario = Scenario::new();

    let result = scenario.run(&["help"]);

    assert_eq!(result.code, Some(0));
    for part in [
        "Spec run",
        "every Ticket",
        "can reach",
        "Unready Ticket",
        "ready-for-human, needs-info, wontfix or\nneeds-triage",
        "sub-issues",
    ] {
        assert_contains(&result.stdout, part);
    }
}
