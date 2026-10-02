//! Pickup runs: `thirdshift pickup` takes the lowest-numbered Ready issue in
//! the repository, an open issue labelled `ready-for-agent` with no label
//! that makes an Unready Ticket, no Claim and no `base-fix` label, that is
//! not a sub-issue, has no open blocker and nothing started on it, is not a
//! Spec whose Tickets are all closed, and has settled, and
//! dispatches it as `thirdshift <Issue URL>` would, a Spec run or a Run,
//! ending as that does. Each `ready-for-agent` issue it passes over on the
//! way gets a line saying why. With no Ready issue, with the repository at
//! its Claim limit, or while an Architect run or another Pickup run on the
//! repository is still running, it is skipped. Each pass that gets the lock
//! first sweeps `in-progress` off the repository's closed issues.
//! Asked for a Run notification, a pass that took an issue sends one, the
//! dispatched run's, and a skipped one sends none.

mod support;

use std::fs;

use support::resend::ResendStandIn;
use support::{HeldRun, REPO, RunResult, Scenario, TimelineEvent};

/// The label of an issue a Pickup run may take.
const READY_FOR_AGENT: &str = "ready-for-agent";

/// The label of a Claimed issue, which the dispatched run swaps the issue's
/// `ready-for-agent` for.
const IN_PROGRESS: &str = "in-progress";

/// Make issue `number` open and labelled `ready-for-agent` on the fake
/// GitHub, with `more` labels after it.
fn ready_issue(scenario: &Scenario, number: u32, more: &[&str]) {
    scenario.issue_is(number, "OPEN");
    scenario.issue_labelled(number, &[&[READY_FOR_AGENT], more].concat());
}

/// A script in which the agent for issue `issue` commits its work and opens
/// its PR into `base`, leaving the pushing to thirdshift.
fn agent_opens_pr(issue: u32, base: &str) -> String {
    format!(
        r#"
echo "{issue}" > issue-{issue}.txt
git add issue-{issue}.txt
git commit -q -m "Work on {issue}"
gh pr create --base {base} --head issue-{issue} --title "Work on {issue}" --body "Closes #{issue}"
"#
    )
}

/// The newest pull request from `head` on the fake GitHub.
fn pr_from(scenario: &Scenario, head: &str) -> serde_json::Value {
    let prs = scenario.gh_state()["prs"].clone();
    let from_head = prs
        .as_array()
        .unwrap()
        .iter()
        .rfind(|pr| pr["head"] == head);
    from_head
        .unwrap_or_else(|| panic!("no PR from {head}: {prs}"))
        .clone()
}

/// Assert the Pickup run ended with `pr` in `outcome`, as a Run or a Spec
/// run that reached its goal does: exit 0, the PR's URL alone on stdout, and
/// the outcome as the last line on stderr.
fn assert_ended_with_pr(result: &RunResult, pr: &serde_json::Value, outcome: &str) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let url = pr["url"].as_str().unwrap();
    assert_eq!(result.stdout, format!("{url}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {url} is {outcome}").as_str()),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn the_lower_numbered_of_two_ready_issues_is_run_and_the_other_left_untouched() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 9, &["bug"]);
    ready_issue(&scenario, 7, &["bug"]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));

    let result = scenario.run(&["pickup"]);

    let pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &pr, "ready for review");
    assert_eq!(pr["base"], "main");
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(scenario.issue_labels(7), ["bug", IN_PROGRESS]);
    assert_eq!(scenario.issue_labels(9), [READY_FOR_AGENT, "bug"]);
    assert!(scenario.origin_log("issue-9").is_none());
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1, "sessions: {calls:?}");
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with(&format!(
            "/thirdshift:implement {}\n",
            scenario.issue_url(7)
        )),
        "{prompt}"
    );
    scenario.assert_cleaned_up("issue-7");
}

/// What a skipped Pickup run says when the scenario's repository has no
/// Ready issue.
const NO_READY_ISSUE: &str = "thirdshift: no Ready issue on acme/widgets\n";

/// What a Pickup run says when it passed over each `ready-for-agent` issue
/// with one of `lines`, in order, and so found no Ready issue.
fn no_ready_issue_after(lines: &[&str]) -> String {
    let lines: String = lines
        .iter()
        .map(|line| format!("thirdshift: {line}\n"))
        .collect();
    lines + NO_READY_ISSUE
}

/// Assert the Pickup run was skipped with `reason` as its one line: exit 0,
/// that line alone on stderr, nothing on stdout, and no session started.
fn assert_skipped(scenario: &Scenario, result: &RunResult, reason: &str) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stderr, reason);
    assert_eq!(result.stdout, "");
    assert!(scenario.claude_calls().is_empty(), "a session was started");
}

#[test]
fn a_progress_line_names_the_issue_taken_before_it_is_dispatched() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.issue_titled(7, "Sharpen the widgets");
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let url = scenario.issue_url(7);
    let taking = format!(
        "thirdshift: taking Ready issue #7 \"Sharpen the widgets\", as thirdshift {url} would\n"
    );
    assert!(
        result.stderr.starts_with(&taking),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn an_issue_without_ready_for_agent_is_never_taken_and_with_no_ready_issue_the_pass_is_skipped() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug"]);
    scenario.issue_is(8, "OPEN");
    let github = scenario.gh_state();

    let result = scenario.run(&["pickup"]);

    assert_skipped(&scenario, &result, NO_READY_ISSUE);
    assert_eq!(scenario.gh_state(), github);
}

#[test]
fn a_closed_issue_labelled_ready_for_agent_is_not_taken() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.issue_is(7, "CLOSED");

    let result = scenario.run(&["pickup"]);

    assert_skipped(&scenario, &result, NO_READY_ISSUE);
}

#[test]
fn an_issue_that_also_has_a_label_that_makes_an_unready_ticket_is_not_taken() {
    // Whatever its case: GitHub's label names are case-insensitive.
    for unready in [
        "ready-for-human",
        "needs-info",
        "wontfix",
        "needs-triage",
        "Needs-Info",
    ] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[unready]);

        let result = scenario.run(&["pickup"]);

        // The line names the label as thirdshift spells it.
        let line = format!("#7 labelled {}", unready.to_lowercase());
        assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
        assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT, unready]);
    }
}

#[test]
fn an_issue_labelled_in_progress_is_not_taken() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[IN_PROGRESS]);

    let result = scenario.run(&["pickup"]);

    let passed_over = no_ready_issue_after(&["#7 labelled in-progress"]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT, IN_PROGRESS]);
}

#[test]
fn an_issue_with_an_issue_branch_on_origin_is_not_taken() {
    for branch in ["issue-7", "issue-7-branch-2"] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.origin_has_branch(branch, "main", &["Earlier work"]);

        let result = scenario.run(&["pickup"]);

        let line = format!("#7 already started: {branch} is on origin");
        assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
        assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT], "{branch}");
    }
}

#[test]
fn an_issue_with_a_pull_request_from_an_issue_branch_in_any_state_is_not_taken() {
    for state in ["OPEN", "MERGED", "CLOSED"] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        let pr = scenario.github_has_pr("issue-7", "main", state);

        let result = scenario.run(&["pickup"]);

        let line = format!("#7 already started: PR {pr}");
        assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
        assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT], "{state}");
    }
}

#[test]
fn when_a_later_issue_is_ready_the_earlier_ones_lines_are_printed_and_it_is_taken() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let pr = scenario.github_has_pr("issue-7", "main", "CLOSED");
    ready_issue(&scenario, 8, &["wontfix"]);
    ready_issue(&scenario, 9, &[IN_PROGRESS]);
    // Neither #7's Issue branch nor its pull request is one of #70's.
    ready_issue(&scenario, 70, &[]);
    scenario.agent_does_for(70, &agent_opens_pr(70, "main"));
    // Never looked at: the pass ends with the first Ready issue.
    ready_issue(&scenario, 71, &["needs-info"]);

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-70"), "ready for review");
    let url = scenario.issue_url(70);
    let lines = format!(
        "thirdshift: #7 already started: PR {pr}\n\
         thirdshift: #8 labelled wontfix\n\
         thirdshift: #9 labelled in-progress\n\
         thirdshift: taking Ready issue #70 \"Issue 70\", as thirdshift {url} would\n"
    );
    assert!(
        result.stderr.starts_with(&lines),
        "stderr: {}",
        result.stderr
    );
    assert!(!result.stderr.contains("#71"), "stderr: {}", result.stderr);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    assert_eq!(scenario.issue_labels(70), [IN_PROGRESS]);
}

#[test]
fn a_ready_for_agent_sub_issue_is_never_run_on_its_own_whatever_its_specs_labels() {
    for spec_labels in [&[][..], &["needs-triage"], &[READY_FOR_AGENT, "needs-info"]] {
        let scenario = Scenario::new();
        scenario.spec_has_tickets(7, &[(8, &[])]);
        scenario.issue_labelled(7, spec_labels);
        ready_issue(&scenario, 8, &[]);

        let result = scenario.run(&["pickup"]);

        // The Spec's own line is there only when it is labelled
        // `ready-for-agent` itself.
        let spec = "#7 labelled needs-info";
        let ticket = "#8 is a Ticket of #7, which is not ready";
        let lines = if spec_labels.contains(&READY_FOR_AGENT) {
            vec![spec, ticket]
        } else {
            vec![ticket]
        };
        assert_skipped(&scenario, &result, &no_ready_issue_after(&lines));
        assert_eq!(scenario.issue_labels(8), [READY_FOR_AGENT]);
        assert_eq!(scenario.issue_labels(7), spec_labels);
    }
}

#[test]
fn a_ticket_numbered_below_its_spec_names_the_spec_that_is_not_ready() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.spec_has_tickets(20, &[(7, &[])]);
    ready_issue(&scenario, 20, &[]);
    scenario.issue_blocked_by(20, &[30]);

    let result = scenario.run(&["pickup"]);

    let lines = [
        "#7 is a Ticket of #20, which is not ready",
        "#20 blocked by #30",
    ];
    assert_skipped(&scenario, &result, &no_ready_issue_after(&lines));
}

#[test]
fn a_spec_that_is_a_ready_issue_is_taken_and_its_tickets_get_no_lines() {
    // #7 is a Ticket of the Spec #20, numbered below it, and #21 one numbered
    // above it: both labelled `ready-for-agent`, as the Spec is.
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    ready_issue(&scenario, 20, &[]);
    ready_issue(&scenario, 21, &[]);
    scenario.spec_has_tickets(20, &[(7, &[]), (21, &[])]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "issue-20"));
    scenario.agent_does_for(21, &agent_opens_pr(21, "issue-20"));

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-20"), "ready for review");
    let taking = format!(
        "thirdshift: taking Ready issue #20 \"Issue 20\", as thirdshift {} would\n",
        scenario.issue_url(20)
    );
    assert!(
        result.stderr.starts_with(&taking),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(20), [IN_PROGRESS]);
    for ticket in [7, 21] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}"));
        assert_eq!(pr["base"], "issue-20");
        assert_eq!(pr["state"], "MERGED");
    }
}

#[test]
fn an_issue_labelled_base_fix_is_not_taken() {
    // Whatever its case: GitHub's label names are case-insensitive.
    for base_fix in ["base-fix", "Base-Fix"] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[base_fix]);

        let result = scenario.run(&["pickup"]);

        let passed_over = no_ready_issue_after(&["#7 labelled base-fix"]);
        assert_skipped(&scenario, &result, &passed_over);
        assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT, base_fix]);
    }
}

#[test]
fn an_issue_with_an_open_blocker_is_not_taken_and_with_that_blocker_closed_it_is() {
    let scenario = ready_ticket();
    scenario.issue_blocked_by(7, &[5, 6]);
    scenario.issue_is(5, "CLOSED");
    scenario.issue_is(6, "OPEN");

    let blocked = scenario.run(&["pickup"]);

    let passed_over = no_ready_issue_after(&["#7 blocked by #6"]);
    assert_skipped(&scenario, &blocked, &passed_over);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);

    scenario.issue_is(6, "CLOSED");
    let unblocked = scenario.run(&["pickup"]);

    assert_ended_with_pr(
        &unblocked,
        &pr_from(&scenario, "issue-7"),
        "ready for review",
    );
}

#[test]
fn every_open_blocker_is_named_in_the_line() {
    let scenario = ready_ticket();
    scenario.issue_blocked_by(7, &[5, 6]);

    let result = scenario.run(&["pickup"]);

    let passed_over = no_ready_issue_after(&["#7 blocked by #5, #6"]);
    assert_skipped(&scenario, &result, &passed_over);
}

/// What a Pickup run says of #7 when it was labelled `ready-for-agent` too
/// recently to have settled.
const LABELLED_TOO_RECENTLY: &str =
    "#7 not settled: labelled ready-for-agent less than 10 minutes ago";

#[test]
fn an_issue_labelled_ready_for_agent_less_than_ten_minutes_ago_is_not_taken() {
    let scenario = ready_ticket();
    // Its other labels, however recent, and an earlier `ready-for-agent`
    // don't count: only the last time `ready-for-agent` was applied does.
    scenario.issue_timeline(
        7,
        &[
            (TimelineEvent::Labelled(READY_FOR_AGENT), 60),
            (TimelineEvent::Labelled(READY_FOR_AGENT), 9),
            (TimelineEvent::Labelled("bug"), 1),
        ],
    );

    let result = scenario.run(&["pickup"]);

    let passed_over = no_ready_issue_after(&[LABELLED_TOO_RECENTLY]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
}

#[test]
fn an_issue_labelled_ready_for_agent_more_than_ten_minutes_ago_is_taken() {
    let scenario = ready_ticket();
    scenario.issue_timeline(
        7,
        &[
            (TimelineEvent::Labelled(READY_FOR_AGENT), 11),
            (TimelineEvent::Labelled("bug"), 1),
        ],
    );

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
}

#[test]
fn a_spec_labelled_long_ago_whose_sub_issues_or_blockers_changed_less_than_ten_minutes_ago_is_not_taken()
 {
    for (event, changed) in [
        (TimelineEvent::SubIssueAdded, "a sub-issue"),
        (TimelineEvent::SubIssueRemoved, "a sub-issue"),
        (TimelineEvent::BlockedByAdded, "a \"blocked by\" link"),
        (TimelineEvent::BlockedByRemoved, "a \"blocked by\" link"),
    ] {
        let scenario = ready_spec("");
        scenario.issue_timeline(
            7,
            &[
                (TimelineEvent::Labelled(READY_FOR_AGENT), 600),
                (TimelineEvent::SubIssueAdded, 590),
                (event, 5),
            ],
        );

        let result = scenario.run(&["pickup"]);

        let line = format!("#7 not settled: {changed} added or removed less than 10 minutes ago");
        assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
        assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    }
}

#[test]
fn an_issue_passed_over_for_being_unsettled_is_taken_by_a_later_pass_once_it_has_settled() {
    let scenario = ready_spec("");
    let labelled = (TimelineEvent::Labelled(READY_FOR_AGENT), 600);
    scenario.issue_timeline(7, &[labelled, (TimelineEvent::SubIssueAdded, 9)]);
    let first = scenario.run(&["pickup"]);
    let line = "#7 not settled: a sub-issue added or removed less than 10 minutes ago";
    assert_skipped(&scenario, &first, &no_ready_issue_after(&[line]));

    // The same events, a few minutes on.
    let labelled = (TimelineEvent::Labelled(READY_FOR_AGENT), 602);
    scenario.issue_timeline(7, &[labelled, (TimelineEvent::SubIssueAdded, 11)]);
    let later = scenario.run(&["pickup"]);

    assert_ended_with_pr(&later, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
}

/// What a Pickup run says of #7 when it is a Spec whose Tickets are all
/// closed, with nothing started on it.
const EVERY_TICKET_CLOSED: &str = "#7 every Ticket is closed";

#[test]
fn each_passed_over_issue_has_one_line_with_the_first_reason_that_applies() {
    // Every reason applies to #7: each row takes the first one away.
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &["needs-info", IN_PROGRESS, "base-fix"]);
    scenario.spec_has_tickets(3, &[(7, &[])]);
    scenario.issue_blocked_by(7, &[5]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.spec_has_tickets(7, &[(4, &[])]);
    scenario.issue_is(4, "CLOSED");
    scenario.issue_timeline(7, &[(TimelineEvent::Labelled(READY_FOR_AGENT), 1)]);
    let expect = |line: &str| {
        let result = scenario.run(&["pickup"]);
        assert_skipped(&scenario, &result, &no_ready_issue_after(&[line]));
    };

    expect("#7 labelled needs-info");
    scenario.issue_labelled(7, &[READY_FOR_AGENT, IN_PROGRESS, "base-fix"]);
    expect("#7 labelled in-progress");
    scenario.issue_labelled(7, &[READY_FOR_AGENT, "base-fix"]);
    expect("#7 is a Ticket of #3, which is not ready");
    scenario.spec_has_tickets(3, &[]);
    expect("#7 labelled base-fix");
    scenario.issue_labelled(7, &[READY_FOR_AGENT]);
    expect("#7 blocked by #5");
    scenario.issue_blocked_by(7, &[]);
    expect("#7 already started: issue-7 is on origin");
    scenario.origin_git(&["update-ref", "-d", "refs/heads/issue-7"]);
    expect(EVERY_TICKET_CLOSED);
    scenario.spec_has_tickets(7, &[]);
    expect(LABELLED_TOO_RECENTLY);
}

/// An open issue labelled `ready-for-agent`, #7, that is a Spec whose
/// Tickets, #5 and #6, are both closed, with no Spec branch.
fn spec_with_every_ticket_closed() -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.spec_has_tickets(7, &[(5, &[]), (6, &[])]);
    scenario.issue_is(5, "CLOSED");
    scenario.issue_is(6, "CLOSED");
    scenario
}

#[test]
fn a_spec_whose_tickets_are_all_closed_is_passed_over_and_the_next_ready_issue_is_taken() {
    let scenario = spec_with_every_ticket_closed();
    ready_issue(&scenario, 9, &[]);
    scenario.agent_does_for(9, &agent_opens_pr(9, "main"));

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-9"), "ready for review");
    let lines = format!(
        "thirdshift: {EVERY_TICKET_CLOSED}\n\
         thirdshift: taking Ready issue #9 \"Issue 9\", as thirdshift {} would\n",
        scenario.issue_url(9)
    );
    assert!(
        result.stderr.starts_with(&lines),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    assert_eq!(scenario.issue_labels(9), [IN_PROGRESS]);
    assert_eq!(sessions(&scenario), ["9"]);
}

#[test]
fn with_only_a_spec_whose_tickets_are_all_closed_the_pass_is_skipped_changing_nothing() {
    let scenario = spec_with_every_ticket_closed();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);
    let github = scenario.gh_state();

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    let passed_over = no_ready_issue_after(&[EVERY_TICKET_CLOSED]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.gh_state(), github);
    assert!(resend.requests().is_empty());
}

#[test]
fn a_spec_whose_tickets_are_all_closed_and_that_was_started_keeps_its_already_started_line() {
    let scenario = spec_with_every_ticket_closed();
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    let result = scenario.run(&["pickup"]);
    let line = "#7 already started: issue-7 is on origin";
    assert_skipped(&scenario, &result, &no_ready_issue_after(&[line]));

    let scenario = spec_with_every_ticket_closed();
    let pr = scenario.github_has_pr("issue-7", "main", "OPEN");
    let result = scenario.run(&["pickup"]);
    let line = format!("#7 already started: PR {pr}");
    assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
}

#[test]
fn a_spec_with_one_ticket_still_open_is_taken() {
    let scenario = ready_spec("");
    scenario.issue_is(8, "CLOSED");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
    assert_eq!(sessions(&scenario), ["9", "7"]);
}

#[test]
fn a_spec_whose_tickets_are_all_closed_is_never_said_to_be_not_settled() {
    let scenario = spec_with_every_ticket_closed();
    scenario.issue_timeline(
        7,
        &[
            (TimelineEvent::SubIssueAdded, 5),
            (TimelineEvent::Labelled(READY_FOR_AGENT), 1),
        ],
    );

    let result = scenario.run(&["pickup"]);

    let passed_over = no_ready_issue_after(&[EVERY_TICKET_CLOSED]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
}

/// A Ready issue, #7, that is a Spec with two Tickets that don't block each
/// other, #8 and #9. Each Ticket's session opens its PR into the Spec branch,
/// #9's after doing `before_ticket_9`.
fn ready_spec(before_ticket_9: &str) -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[])]);
    scenario.agent_does_for(8, &agent_opens_pr(8, "issue-7"));
    scenario.agent_does_for(
        9,
        &format!("{before_ticket_9}\n{}", agent_opens_pr(9, "issue-7")),
    );
    scenario
}

/// A Ready issue, #7, with no sub-issues, whose session opens its PR into
/// `main`.
fn ready_ticket() -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));
    scenario
}

/// The issue number each agent session was for, the first Issue URL its
/// prompt names, in order.
fn sessions(scenario: &Scenario) -> Vec<String> {
    scenario
        .claude_calls()
        .iter()
        .map(|call| {
            let prompt = call["prompt"].as_str().unwrap();
            let (_, after) = prompt.split_once("/issues/").unwrap();
            after.chars().take_while(char::is_ascii_digit).collect()
        })
        .collect()
}

#[test]
fn a_ready_issue_with_sub_issues_is_run_as_a_spec_run_and_ends_as_that_spec_run_does() {
    let scenario = ready_spec("");

    let result = scenario.run(&["pickup"]);

    let spec_pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &spec_pr, "ready for review");
    assert_eq!(spec_pr["base"], "main");
    assert_eq!(spec_pr["state"], "OPEN");
    assert_eq!(spec_pr["isDraft"], false);
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
    let mut sessions = sessions(&scenario);
    assert_eq!(sessions.pop().as_deref(), Some("7"), "the Spec review");
    sessions.sort();
    assert_eq!(sessions, ["8", "9"]);
    for ticket in [8, 9] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}"));
        assert_eq!(pr["base"], "issue-7");
        assert_eq!(pr["state"], "MERGED");
    }
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_dispatched_run_that_fails_fails_the_pickup_run_as_a_failed_run_does() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does_for(7, "echo work > work.txt\nexit 1");

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    let last = result.stderr.lines().last().unwrap();
    assert!(
        last.starts_with("thirdshift: session log: ") && last.ends_with("-implement.jsonl"),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.origin_log("issue-7").is_some(), "work not pushed");
}

#[test]
fn parallel_reaches_a_dispatched_spec_run() {
    // #9's session only succeeds if #8 landed on the Spec branch before it
    // started, as it has only when the Tickets run one at a time.
    let scenario = ready_spec("test -f issue-8.txt");

    let result = scenario.run(&["pickup", "parallel", "1"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(sessions(&scenario), ["8", "9", "7"]);
}

#[test]
fn parallel_is_ignored_without_error_for_an_issue_that_is_not_a_spec() {
    for flag in ["parallel", "--parallel"] {
        let scenario = ready_ticket();

        let result = scenario.run(&["pickup", flag, "2"]);

        assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    }
}

#[test]
fn merge_and_no_merge_and_the_user_configs_default_reach_the_dispatched_run() {
    for ready in [ready_ticket, || ready_spec("")] {
        for (config, args, outcome, state) in [
            ("", vec!["pickup"], "ready for review", "OPEN"),
            ("", vec!["pickup", "merge"], "merged", "MERGED"),
            ("", vec!["pickup", "--merge"], "merged", "MERGED"),
            (
                "[merge]\nalways = true\n",
                vec!["pickup"],
                "merged",
                "MERGED",
            ),
            (
                "[merge]\nalways = true\n",
                vec!["pickup", "no-merge"],
                "ready for review",
                "OPEN",
            ),
            (
                "[merge]\nalways = true\n",
                vec!["pickup", "--no-merge"],
                "ready for review",
                "OPEN",
            ),
        ] {
            let scenario = ready();
            scenario.user_config_is(config);

            let result = scenario.run(&args);

            let pr = pr_from(&scenario, "issue-7");
            assert_ended_with_pr(&result, &pr, outcome);
            assert_eq!(pr["state"], state, "{args:?} with {config:?}");
        }
    }
}

const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;

const GREEN: &str = r#"[{"name": "test", "conclusion": "success"}]"#;

/// A Ready issue, #7, whose Run opens its PR with `test` red on its head and
/// on `main`: an Inherited failure. The agent for the Base fix issue, #8, the
/// next issue, fixes it.
fn ready_ticket_that_inherits_a_failure() -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does_for(
        7,
        &format!(
            r#"{}
gh fake checks "$(git rev-parse HEAD)" '{RED}'
gh fake checks "$(git rev-parse origin/main)" '{RED}'
"#,
            agent_opens_pr(7, "main")
        ),
    );
    scenario.agent_does_for(
        8,
        &format!(
            r#"
echo "fixed" > ci-fix.txt
git add ci-fix.txt
git commit -q -m "Fix CI on main"
gh pr create --base main --head issue-8 --title "Fix CI on main" --body "Closes #8"
gh fake checks "$(git rev-parse HEAD)" '{GREEN}'
"#
        ),
    );
    scenario
}

#[test]
fn base_fix_and_no_base_fix_and_the_user_configs_default_reach_the_dispatched_run() {
    for (config, args, fixed) in [
        ("", vec!["pickup"], false),
        ("", vec!["pickup", "base-fix"], true),
        ("", vec!["pickup", "--base-fix"], true),
        ("[base]\nfix = true\n", vec!["pickup"], true),
        ("[base]\nfix = true\n", vec!["pickup", "no-base-fix"], false),
        (
            "[base]\nfix = true\n",
            vec!["pickup", "--no-base-fix"],
            false,
        ),
    ] {
        let scenario = ready_ticket_that_inherits_a_failure();
        scenario.user_config_is(config);
        let red_base = scenario.origin_git(&["rev-parse", "main"]);

        let result = scenario.run(&args);

        let started = result.stderr.contains(
            "thirdshift: starting Base fix #8 into main: https://github.com/acme/widgets/issues/8\n",
        );
        assert_eq!(
            started, fixed,
            "{args:?} with {config:?}: {}",
            result.stderr
        );
        if fixed {
            assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
            assert_eq!(pr_from(&scenario, "issue-8")["state"], "MERGED");
        } else {
            assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
            let cause = format!(
                "thirdshift: CI red on test, which also fails on main at {}; fix main first\n",
                &red_base[..7]
            );
            assert!(result.stderr.contains(&cause), "stderr: {}", result.stderr);
        }
    }
}

#[test]
fn a_run_that_fails_on_an_inherited_failure_offers_the_command_that_retries_its_issue_by_hand() {
    let scenario = ready_ticket_that_inherits_a_failure();

    let result = scenario.run(&["pickup", "merge", "parallel", "2", "base", "main"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    // Not a Spec, so without the `parallel` that would fail its Run by hand.
    let retry = format!("thirdshift {} merge base-fix\n", scenario.issue_url(7));
    assert!(result.stderr.contains(&retry), "stderr: {}", result.stderr);
}

#[test]
fn base_makes_the_named_branch_the_dispatched_runs_base_branch_whatever_is_checked_out() {
    for (flag, detached) in [("base", false), ("--base", true)] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.agent_does_for(7, &agent_opens_pr(7, "develop"));
        scenario.origin_has_branch("develop", "main", &["Develop work"]);
        if detached {
            scenario.launch_git(&["checkout", "-q", "--detach"]);
        }
        let checked_out = scenario.launch_git(&["branch", "--show-current"]);

        let result = scenario.run(&["pickup", flag, "develop"]);

        let pr = pr_from(&scenario, "issue-7");
        assert_ended_with_pr(&result, &pr, "ready for review");
        assert_eq!(pr["base"], "develop", "detached: {detached}");
        assert_eq!(
            scenario.origin_log("issue-7").unwrap(),
            ["Work on 7", "Develop work", "Initial commit"],
            "detached: {detached}"
        );
        assert_eq!(
            scenario.launch_git(&["branch", "--show-current"]),
            checked_out
        );
        scenario.assert_cleaned_up("issue-7");
    }
}

#[test]
fn base_makes_the_named_branch_the_dispatched_spec_runs_base_branch() {
    let scenario = ready_spec("");
    scenario.origin_has_branch("develop", "main", &["Develop work"]);

    let result = scenario.run(&["pickup", "base", "develop", "merge"]);

    let spec_pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &spec_pr, "merged");
    assert_eq!(spec_pr["base"], "develop");
    assert_eq!(
        scenario.origin_file("develop", "issue-8.txt").as_deref(),
        Some("8\n")
    );
    assert!(scenario.origin_file("main", "issue-8.txt").is_none());
}

/// Assert the Pickup run was stopped by a preflight check with `message`,
/// exit 1, before any work: no session, and no call to GitHub, so no label
/// was read or changed.
fn assert_stopped_by_preflight(scenario: &Scenario, result: &RunResult, message: &str) {
    scenario.assert_rejected_before_any_work(result, message);
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
}

#[test]
fn a_detached_head_without_base_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();
    scenario.launch_git(&["checkout", "-q", "--detach"]);

    let result = scenario.run(&["pickup"]);

    assert_stopped_by_preflight(
        &scenario,
        &result,
        "thirdshift: HEAD is detached; check out the branch the work should be based on, \
         or name it with base <branch>\n",
    );
}

#[test]
fn a_base_branch_ahead_of_origin_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();
    scenario.commit_locally("local.txt", "local\n", "Local work");

    let result = scenario.run(&["pickup"]);

    assert_stopped_by_preflight(
        &scenario,
        &result,
        "local main is 1 commit(s) ahead of origin/main; push them first",
    );
}

#[test]
fn a_named_branch_that_is_not_on_origin_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();

    let result = scenario.run(&["pickup", "base", "nowhere"]);

    assert_stopped_by_preflight(
        &scenario,
        &result,
        "base branch nowhere does not exist on origin; push it first",
    );
}

#[test]
fn a_missing_git_identity_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();
    scenario.git_email_is(None);

    let result = scenario.run(&["pickup"]);

    assert_stopped_by_preflight(&scenario, &result, "git user.email is not set");
}

#[test]
fn an_origin_that_is_not_on_github_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();
    scenario.set_origin_url("https://gitlab.com/acme/widgets.git");

    let result = scenario.run(&["pickup"]);

    assert_stopped_by_preflight(
        &scenario,
        &result,
        "origin https://gitlab.com/acme/widgets.git is not a GitHub repository",
    );
}

/// What a skipped Pickup run or Architect run says when another of either is
/// running on the scenario's repository.
const ALREADY_RUNNING: &str =
    "thirdshift: an Architect run or a Pickup run is already running on acme/widgets\n";

/// A script in which the agent touches `started` in the scenario root, then
/// waits there until the test touches `release`, or until the scenario is
/// gone, as after a test that failed while holding it.
const AGENT_WAITS_FOR_RELEASE: &str = r#"
root=$(dirname "$FAKE_CLAUDE_RECORD")
touch "$root/started"
while [ -d "$root" ] && [ ! -e "$root/release" ]; do sleep 0.05; done
"#;

/// Let an agent held by [`AGENT_WAITS_FOR_RELEASE`] go on.
fn release(scenario: &Scenario) {
    fs::write(scenario.path("release"), "").unwrap();
}

#[test]
fn a_pickup_run_is_skipped_while_an_architect_run_on_the_repository_is_still_running() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does(AGENT_WAITS_FOR_RELEASE);
    let architect = scenario.run_until(&["architect", "--plan-only"], &[], "started");
    let (github, gh_calls) = (scenario.gh_state(), scenario.gh_calls());

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stderr, ALREADY_RUNNING);
    assert_eq!(result.stdout, "");
    assert_eq!(scenario.claude_calls().len(), 1, "a session was started");
    assert_eq!(scenario.gh_state(), github);
    assert_eq!(scenario.gh_calls(), gh_calls);
    assert_eq!(scenario.entries("work"), [REPO, "widgets-architect"]);
    release(&scenario);
    architect.finish();
}

#[test]
fn a_pickup_run_or_an_architect_run_is_skipped_while_a_pickup_runs_dispatched_run_is_still_going() {
    for second in ["pickup", "architect"] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        ready_issue(&scenario, 9, &[]);
        scenario.agent_does_for(
            7,
            &format!("{AGENT_WAITS_FOR_RELEASE}{}", agent_opens_pr(7, "main")),
        );
        let first = scenario.run_until(&["pickup"], &[], "started");
        let (github, gh_calls) = (scenario.gh_state(), scenario.gh_calls());

        let skipped = scenario.run(&[second]);

        assert_eq!(skipped.code, Some(0), "stderr: {}", skipped.stderr);
        assert_eq!(skipped.stderr, ALREADY_RUNNING, "{second}");
        assert_eq!(skipped.stdout, "");
        assert_eq!(scenario.claude_calls().len(), 1, "a session was started");
        assert_eq!(scenario.gh_state(), github);
        assert_eq!(scenario.gh_calls(), gh_calls);
        release(&scenario);
        let first = first.finish();
        assert_ended_with_pr(&first, &pr_from(&scenario, "issue-7"), "ready for review");
        assert_eq!(scenario.issue_labels(9), [READY_FOR_AGENT]);
    }
}

#[test]
fn once_a_pickup_run_has_ended_the_next_takes_the_next_ready_issue() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    ready_issue(&scenario, 9, &[]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));
    scenario.agent_does_for(9, &agent_opens_pr(9, "main"));
    let first = scenario.run(&["pickup"]);
    assert_ended_with_pr(&first, &pr_from(&scenario, "issue-7"), "ready for review");

    let second = scenario.run(&["pickup"]);

    assert_ended_with_pr(&second, &pr_from(&scenario, "issue-9"), "ready for review");
    let third = scenario.run(&["pickup"]);
    assert_eq!(third.code, Some(0), "stderr: {}", third.stderr);
    assert_eq!(third.stderr, NO_READY_ISSUE);
    assert_eq!(third.stdout, "");
    assert_eq!(sessions(&scenario), ["7", "9"]);
}

const KEY: &str = "re_test_123";

const ACCEPTED: &str = r#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#;

/// The environment for a Pickup run against `resend`, with a Resend API key.
fn resend_env(resend: &ResendStandIn) -> [(&str, &str); 2] {
    [
        ("THIRDSHIFT_RESEND_URL", resend.url()),
        ("RESEND_API_KEY", KEY),
    ]
}

/// Run thirdshift with `args` against `resend`, with a Resend API key in the
/// environment.
fn run_with_resend(scenario: &Scenario, resend: &ResendStandIn, args: &[&str]) -> RunResult {
    scenario.run_with_env(args, &resend_env(resend))
}

/// The subject and the text of the one Run notification `resend` received.
fn the_one_notification(resend: &ResendStandIn) -> (String, String) {
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let body = &requests[0].body;
    let part = |name: &str| body[name].as_str().unwrap().to_string();
    (part("subject"), part("text"))
}

/// A Ready issue, #7, titled, with no sub-issues, whose session opens its PR
/// into `main`.
fn titled_ready_ticket() -> Scenario {
    let scenario = ready_ticket();
    scenario.issue_titled(7, "Sharpen the widgets");
    scenario
}

/// The User config of a machine where every Run and every Pickup run that
/// takes an issue sends a Run notification.
const EMAIL_ALWAYS: &str = "[email]\nalways = true\nto = \"config@example.com\"\n";

#[test]
fn a_pass_that_took_an_issue_sends_one_notification_when_the_command_or_the_user_config_asks() {
    for (config, args, to) in [
        (
            "",
            vec!["pickup", "email", "me@example.com"],
            Some("me@example.com"),
        ),
        (EMAIL_ALWAYS, vec!["pickup"], Some("config@example.com")),
        (
            EMAIL_ALWAYS,
            vec!["pickup", "--email", "me@example.com"],
            Some("me@example.com"),
        ),
        (
            "[email]\nto = \"config@example.com\"\n",
            vec!["pickup", "email"],
            Some("config@example.com"),
        ),
        (EMAIL_ALWAYS, vec!["pickup", "no-email"], None),
        (EMAIL_ALWAYS, vec!["pickup", "--no-email"], None),
        (
            "[email]\nto = \"config@example.com\"\n",
            vec!["pickup"],
            None,
        ),
    ] {
        let scenario = titled_ready_ticket();
        scenario.user_config_is(config);
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = run_with_resend(&scenario, &resend, &args);

        assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
        let requests = resend.requests();
        let sent: Vec<_> = requests.iter().map(|request| &request.body).collect();
        match to {
            // Exactly one: the dispatched run sends none of its own.
            Some(to) => {
                assert_eq!(sent.len(), 1, "{args:?} with {config:?}: {sent:?}");
                assert_eq!(sent[0]["to"], to, "{args:?} with {config:?}");
            }
            None => assert!(sent.is_empty(), "{args:?} with {config:?}: {sent:?}"),
        }
    }
}

#[test]
fn the_notification_of_a_run_left_ready_for_review_is_that_runs() {
    let scenario = titled_ready_ticket();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

    let pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &pr, "ready for review");
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen the widgets: ready for review"
    );
    let logs = scenario.entries("home/.thirdshift/logs");
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    let log = scenario.path("home/.thirdshift/logs").join(&logs[0]);
    assert!(
        text.starts_with(&format!(
            "Pull request: {}\n\
             Session log:  {}\n\
             Host:         ",
            pr["url"].as_str().unwrap(),
            log.display()
        )),
        "{text}"
    );
    assert!(text.contains("\nTook:         "), "{text}");
}

#[test]
fn the_notification_of_a_merge_run_says_it_merged() {
    let scenario = titled_ready_ticket();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "merge"]);

    let pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &pr, "merged");
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen the widgets: merged"
    );
    let pull_request = format!("Pull request: {}\n", pr["url"].as_str().unwrap());
    assert!(text.starts_with(&pull_request), "{text}");
}

#[test]
fn the_notification_of_a_failed_run_has_its_pull_request_and_its_cause() {
    let scenario = titled_ready_ticket();
    scenario.agent_does_for(7, &format!("{}exit 3\n", agent_opens_pr(7, "main")));
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "--email", "me@example.com"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen the widgets: failed"
    );
    assert!(
        text.starts_with(&format!(
            "Pull request: {}\n\
             Cause:        claude exited 3\n\
             Session log:  ",
            pr_from(&scenario, "issue-7")["url"].as_str().unwrap()
        )),
        "{text}"
    );
}

#[test]
fn the_notification_of_an_interrupted_run_says_it_was_interrupted() {
    let scenario = titled_ready_ticket();
    scenario.agent_does_for(
        7,
        r#"touch "$(dirname "$FAKE_CLAUDE_RECORD")/started"
sleep 60"#,
    );
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = scenario.run_and_signal_with_env(
        &["pickup", "--email", "me@example.com"],
        &resend_env(&resend),
        "started",
        "TERM",
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen the widgets: interrupted"
    );
    assert!(!text.contains("Cause:"), "{text}");
}

#[test]
fn the_notification_of_a_spec_run_is_that_spec_runs_with_a_line_per_ticket() {
    let scenario = ready_spec("");
    scenario.issue_titled(7, "Sharpen every widget");
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    let spec_pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &spec_pr, "ready for review");
    // Exactly one: neither the Spec run nor a Ticket's Run sends its own.
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen every widget: ready for review"
    );
    let pull_request = format!("Pull request: {}\n", spec_pr["url"].as_str().unwrap());
    assert!(text.starts_with(&pull_request), "{text}");
    let (_, tickets) = text.split_once("\nTickets:\n").expect(&text);
    for ticket in [8, 9] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}"));
        let line = format!("#{ticket} landed with {}\n", pr["url"].as_str().unwrap());
        assert!(tickets.contains(&line), "expected {line:?} in: {text}");
    }
}

#[test]
fn the_notification_of_a_failed_spec_run_has_each_tickets_outcome() {
    let scenario = ready_spec("exit 3");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert!(subject.ends_with(": failed"), "{subject}");
    let (_, tickets) = text.split_once("\nTickets:\n").expect(&text);
    let landed = pr_from(&scenario, "issue-8");
    let landed = format!("#8 landed with {}\n", landed["url"].as_str().unwrap());
    assert!(tickets.contains(&landed), "expected {landed:?} in: {text}");
    assert!(tickets.contains("#9 failed: "), "{text}");
}

#[test]
fn a_pass_skipped_for_having_no_ready_issue_sends_no_notification() {
    for (config, args) in [
        ("", vec!["pickup", "email", "me@example.com"]),
        (EMAIL_ALWAYS, vec!["pickup"]),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(config);
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = run_with_resend(&scenario, &resend, &args);

        assert_skipped(&scenario, &result, NO_READY_ISSUE);
        assert!(resend.requests().is_empty(), "{args:?} with {config:?}");
    }
}

/// Start a Pickup run that takes #7, made a Ready issue here, and holds the
/// repository's lock until [`release`], its session waiting once started.
fn pickup_run_held_on_issue_7(scenario: &Scenario) -> HeldRun {
    ready_issue(scenario, 7, &[]);
    scenario.agent_does_for(
        7,
        &format!("{AGENT_WAITS_FOR_RELEASE}{}", agent_opens_pr(7, "main")),
    );
    scenario.run_until(&["pickup"], &[], "started")
}

#[test]
fn a_pass_skipped_for_the_lock_sends_no_notification() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 9, &[]);
    let first = pickup_run_held_on_issue_7(&scenario);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let skipped = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

    assert_eq!(skipped.code, Some(0), "stderr: {}", skipped.stderr);
    assert_eq!(skipped.stderr, ALREADY_RUNNING);
    assert_eq!(skipped.stdout, "");
    release(&scenario);
    first.finish();
    assert!(resend.requests().is_empty());
}

#[test]
fn asked_for_a_notification_with_no_address_known_the_pass_stops_before_any_work() {
    for (config, args) in [
        ("", ["pickup", "--email"]),
        ("[email]\nalways = true\n", ["pickup", "merge"]),
    ] {
        let scenario = ready_ticket();
        scenario.user_config_is(config);
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = run_with_resend(&scenario, &resend, &args);

        assert_stopped_by_preflight(&scenario, &result, "no email address");
        assert!(resend.requests().is_empty());
    }
}

#[test]
fn asked_for_a_notification_with_no_resend_api_key_the_pass_stops_before_any_work() {
    let scenario = ready_ticket();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = scenario.run_with_env(
        &["pickup", "email", "me@example.com"],
        &[("THIRDSHIFT_RESEND_URL", resend.url())],
    );

    assert_stopped_by_preflight(&scenario, &result, "no Resend API key");
    assert!(resend.requests().is_empty());
}

#[test]
fn a_pass_that_would_be_skipped_for_having_no_ready_issue_still_stops_on_the_notifications_checks()
{
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "--email"]);

    scenario.assert_rejected_before_any_work(&result, "no email address");
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    assert!(resend.requests().is_empty());
}

#[test]
fn a_pass_that_would_be_skipped_for_the_lock_still_stops_on_the_notifications_checks() {
    let scenario = Scenario::new();
    let first = pickup_run_held_on_issue_7(&scenario);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let stopped = run_with_resend(&scenario, &resend, &["pickup", "--email"]);

    assert_eq!(stopped.code, Some(1), "stderr: {}", stopped.stderr);
    assert!(
        stopped.stderr.contains("no email address"),
        "stderr: {}",
        stopped.stderr
    );
    assert_eq!(stopped.stdout, "");
    release(&scenario);
    first.finish();
    assert!(resend.requests().is_empty());
}

#[test]
fn the_notifications_checks_come_before_the_checks_on_the_clone() {
    let scenario = ready_ticket();
    scenario.launch_git(&["checkout", "-q", "--detach"]);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "--email"]);

    assert_stopped_by_preflight(&scenario, &result, "no email address");
    assert!(
        !result.stderr.contains("HEAD is detached"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_pass_that_cant_tell_whether_its_issue_is_a_spec_fails_before_saying_it_took_it() {
    let scenario = titled_ready_ticket();
    scenario.gh_fails("api graphql");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        !result.stderr.contains("taking Ready issue"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    assert!(scenario.claude_calls().is_empty(), "a session was started");
    assert!(resend.requests().is_empty());
}

#[test]
fn a_notification_that_cant_be_sent_is_a_warning_that_changes_neither_the_exit_code_nor_stdout() {
    for (script, code) in [(agent_opens_pr(7, "main"), 0), ("exit 3".to_string(), 1)] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.agent_does_for(7, &script);
        let unasked = scenario.run(&["pickup"]);
        assert_eq!(unasked.code, Some(code), "stderr: {}", unasked.stderr);

        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.agent_does_for(7, &script);
        let resend = ResendStandIn::replying(500, "upstream exploded");

        let result = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

        assert_eq!(result.code, Some(code), "stderr: {}", result.stderr);
        assert_eq!(result.stdout, unasked.stdout);
        assert_eq!(resend.requests().len(), 1);
        let last = result.stderr.lines().last().unwrap();
        assert!(
            last.contains("warning: could not send the Run notification")
                && last.contains("upstream exploded"),
            "stderr: {}",
            result.stderr
        );
    }
}

#[test]
fn the_user_configs_spec_parallel_reaches_a_dispatched_spec_run() {
    // As in `parallel_reaches_a_dispatched_spec_run`: one Ticket at a time.
    let scenario = ready_spec("test -f issue-8.txt");
    scenario.user_config_is("[spec]\nparallel = 1\n");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(sessions(&scenario), ["8", "9", "7"]);
}

/// Make each of `issues` open and labelled `in-progress` on the fake GitHub:
/// an issue that carries a Claim.
fn claimed_issues(scenario: &Scenario, issues: &[u32]) {
    for issue in issues {
        scenario.issue_is(*issue, "OPEN");
        scenario.issue_labelled(*issue, &[IN_PROGRESS]);
    }
}

#[test]
fn with_three_open_issues_in_progress_and_no_pickup_limit_the_pass_is_skipped() {
    let scenario = ready_ticket();
    claimed_issues(&scenario, &[1, 2, 3]);

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        "thirdshift: at the Claim limit on acme/widgets: 3 open issue(s) labelled in-progress, \
         pickup.limit is 3\n",
    );
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
}

#[test]
fn a_pass_skipped_for_the_claim_limit_sends_no_notification() {
    let scenario = ready_ticket();
    claimed_issues(&scenario, &[1, 2, 3]);
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        "thirdshift: at the Claim limit on acme/widgets: 3 open issue(s) labelled in-progress, \
         pickup.limit is 3\n",
    );
    assert!(resend.requests().is_empty());
}

#[test]
fn with_two_open_issues_in_progress_the_pass_takes_a_ready_issue() {
    let scenario = ready_ticket();
    claimed_issues(&scenario, &[1, 2]);

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
}

#[test]
fn pickup_limit_in_the_user_config_raises_and_lowers_the_claim_limit() {
    // Raised, three Claims no longer stop the pass.
    let scenario = ready_ticket();
    claimed_issues(&scenario, &[1, 2, 3]);
    scenario.user_config_is("[pickup]\nlimit = 4\n");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");

    // Lowered, one Claim stops it.
    let scenario = ready_ticket();
    claimed_issues(&scenario, &[1]);
    scenario.user_config_is("[pickup]\nlimit = 1\n");

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        "thirdshift: at the Claim limit on acme/widgets: 1 open issue(s) labelled in-progress, \
         pickup.limit is 1\n",
    );
}

#[test]
fn over_the_claim_limit_the_pass_is_skipped_naming_the_count_and_the_limit() {
    let scenario = ready_ticket();
    claimed_issues(&scenario, &[1, 2, 3, 4]);

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        "thirdshift: at the Claim limit on acme/widgets: 4 open issue(s) labelled in-progress, \
         pickup.limit is 3\n",
    );
}

#[test]
fn a_closed_issue_labelled_in_progress_does_not_count_against_the_claim_limit() {
    let scenario = ready_ticket();
    claimed_issues(&scenario, &[1, 2, 3]);
    scenario.issue_is(3, "CLOSED");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
}

#[test]
fn a_pickup_limit_that_is_not_a_whole_number_from_1_up_stops_the_pass_naming_the_file_and_key() {
    for limit in ["0", "-1", "1.5", "\"3\"", "true"] {
        let scenario = ready_ticket();
        let path = scenario.user_config_is(&format!("[pickup]\nlimit = {limit}\n"));

        let result = scenario.run(&["pickup"]);

        scenario.assert_rejected_before_any_work(
            &result,
            &format!(
                "thirdshift: pickup.limit must be a whole number from 1 up in the User config {}\n",
                path.display()
            ),
        );
        assert_eq!(result.code, Some(1), "{limit}: {}", result.stderr);
        assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    }
}

/// Make issue `number` closed on the fake GitHub, still labelled
/// `in-progress`, after the labels `before` it: an issue merged by hand,
/// which no Self-merge took the Claim off.
fn closed_issue_in_progress(scenario: &Scenario, number: u32, before: &[&str]) {
    scenario.issue_is(number, "CLOSED");
    scenario.issue_labelled(number, &[before, &[IN_PROGRESS]].concat());
}

#[test]
fn a_pass_takes_in_progress_off_every_closed_issue_and_leaves_its_other_labels() {
    let scenario = ready_ticket();
    closed_issue_in_progress(&scenario, 3, &["bug", "architect-plan"]);
    closed_issue_in_progress(&scenario, 4, &[]);
    claimed_issues(&scenario, &[5]);

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.issue_labels(3), ["bug", "architect-plan"]);
    assert_eq!(scenario.issue_labels(4), Vec::<String>::new());
    assert_eq!(scenario.issue_labels(5), [IN_PROGRESS]);
    for closed in [3, 4] {
        let swept = format!("thirdshift: taking in-progress off #{closed}, which is closed\n");
        assert!(result.stderr.contains(&swept), "stderr: {}", result.stderr);
    }
}

/// What the Sweep says as it takes `in-progress` off closed issue #3.
const SWEEPING_3: &str = "thirdshift: taking in-progress off #3, which is closed\n";

#[test]
fn the_sweep_runs_on_a_pass_that_is_then_skipped_for_the_claim_limit() {
    let scenario = ready_ticket();
    closed_issue_in_progress(&scenario, 3, &["bug"]);
    claimed_issues(&scenario, &[4, 5, 6]);

    let result = scenario.run(&["pickup"]);

    let reason = "thirdshift: at the Claim limit on acme/widgets: 3 open issue(s) labelled \
                  in-progress, pickup.limit is 3\n";
    assert_skipped(&scenario, &result, &format!("{SWEEPING_3}{reason}"));
    assert_eq!(scenario.issue_labels(3), ["bug"]);
}

#[test]
fn the_sweep_runs_on_a_pass_that_is_then_skipped_for_having_no_ready_issue() {
    let scenario = Scenario::new();
    closed_issue_in_progress(&scenario, 3, &["bug"]);

    let result = scenario.run(&["pickup"]);

    assert_skipped(&scenario, &result, &format!("{SWEEPING_3}{NO_READY_ISSUE}"));
    assert_eq!(scenario.issue_labels(3), ["bug"]);
}

#[test]
fn the_sweep_does_not_run_on_a_pass_skipped_for_the_lock() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    closed_issue_in_progress(&scenario, 3, &["bug"]);
    scenario.agent_does(AGENT_WAITS_FOR_RELEASE);
    let architect = scenario.run_until(&["architect", "--plan-only"], &[], "started");

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stderr, ALREADY_RUNNING);
    assert_eq!(scenario.issue_labels(3), ["bug", IN_PROGRESS]);
    release(&scenario);
    architect.finish();
}

#[test]
fn a_sweep_that_fails_prints_a_warning_and_the_pass_carries_on() {
    let scenario = ready_ticket();
    closed_issue_in_progress(&scenario, 3, &["bug"]);
    closed_issue_in_progress(&scenario, 4, &[]);
    scenario.gh_fails("api --method DELETE");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    for closed in [3, 4] {
        let warning = format!(
            "thirdshift: warning: could not take in-progress off #{closed}: gh api --method \
             DELETE repos/acme/widgets/issues/{closed}/labels/in-progress --silent failed: \
             HTTP 502"
        );
        assert!(
            result.stderr.contains(&warning),
            "stderr: {}",
            result.stderr
        );
    }
    assert_eq!(scenario.issue_labels(3), ["bug", IN_PROGRESS]);
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
}

#[test]
fn a_sweep_that_cannot_list_the_closed_issues_prints_a_warning_and_the_pass_carries_on() {
    let scenario = ready_ticket();
    closed_issue_in_progress(&scenario, 3, &["bug"]);
    scenario.gh_fails("issue list --state closed");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    let warning = "thirdshift: warning: could not list the closed issues labelled in-progress: \
                   gh issue list failed: HTTP 502";
    assert!(result.stderr.contains(warning), "stderr: {}", result.stderr);
    assert_eq!(scenario.issue_labels(3), ["bug", IN_PROGRESS]);
}

#[test]
fn the_sweep_takes_the_label_off_as_the_closed_issue_spells_it() {
    let scenario = Scenario::new();
    scenario.issue_is(3, "CLOSED");
    scenario.issue_labelled(3, &["In-Progress", "bug"]);

    let result = scenario.run(&["pickup"]);

    assert_skipped(&scenario, &result, &format!("{SWEEPING_3}{NO_READY_ISSUE}"));
    assert_eq!(scenario.issue_labels(3), ["bug"]);
    let deleted = scenario.gh_calls_of("api", "--method");
    assert_eq!(deleted.len(), 1, "{deleted:?}");
    assert_eq!(
        deleted[0][3],
        "repos/acme/widgets/issues/3/labels/In-Progress"
    );
}
