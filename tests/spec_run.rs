//! Spec runs: `thirdshift <Issue URL>` on a Spec, an issue with sub-issues,
//! works through its Tickets in dependency order, each a Merge run into the
//! Spec branch, then opens the Spec PR from the Spec branch into the Base
//! branch as a draft, has the Spec review review the Spec branch, and marks
//! the Spec PR ready for review.

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

/// The issue number each agent session was for, the first Issue URL its
/// prompt names, in order.
fn sessions_by_issue(scenario: &Scenario) -> Vec<String> {
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

/// The Spec review's session: the only one for the Spec.
fn spec_review_call(scenario: &Scenario) -> serde_json::Value {
    let calls = scenario.claude_calls();
    let sessions = sessions_by_issue(scenario);
    let reviews: Vec<_> = sessions
        .iter()
        .enumerate()
        .filter(|(_, issue)| *issue == &SPEC.to_string())
        .map(|(i, _)| calls[i].clone())
        .collect();
    assert_eq!(reviews.len(), 1, "sessions: {sessions:?}");
    reviews[0].clone()
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
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "20"]);

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
    for call in &scenario.claude_calls()[..2] {
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
fn once_the_last_ticket_lands_the_spec_review_reviews_the_spec_branch_against_the_base_branch() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let review = spec_review_call(&scenario);
    assert_eq!(review["branch"], "issue-20");
    let prompt = review["prompt"].as_str().unwrap();
    for part in [
        "/thirdshift:code-review main, with the Spec https://github.com/acme/widgets/issues/20 as the spec\n",
        &spec_url(&scenario),
        "using main as the fixed point",
        "/thirdshift:tdd",
        "Update PR https://github.com/acme/widgets/pull/3 using /thirdshift:pr",
        "\"Unaddressed findings\"",
        "Include \"Closes #20\"",
        "You run headless",
    ] {
        assert_contains(prompt, part);
    }
    let last_landed = result.stderr.find("thirdshift: #22 landed").unwrap();
    let reviewing = result.stderr.find("spec-review: session started").unwrap();
    assert!(last_landed < reviewing, "stderr: {}", result.stderr);
}

#[test]
fn the_spec_reviews_commits_reach_the_spec_branch_on_origin() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        r#"
echo "reviewed" > review.txt
git add review.txt
git commit -q -m "Fix a Spec finding"
git push -q origin issue-20
"#,
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_file("issue-20", "review.txt").as_deref(),
        Some("reviewed\n")
    );
}

#[test]
fn a_spec_review_that_commits_without_pushing_still_reaches_origin() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        r#"
echo "reviewed" > review.txt
git add review.txt
git commit -q -m "Fix a Spec finding"
"#,
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_file("issue-20", "review.txt").as_deref(),
        Some("reviewed\n")
    );
}

#[test]
fn the_spec_pr_is_a_draft_during_the_spec_review_and_marked_ready_after_it() {
    let scenario = linear_spec();
    // Fails the session unless the Spec PR is a draft while it runs.
    scenario.agent_does_for(
        SPEC,
        "gh pr view issue-20 --repo acme/widgets --json isDraft | grep -q '\"isDraft\": true'\n",
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let spec = &scenario.gh_state()["prs"][2];
    assert_eq!(spec["head"], "issue-20");
    assert_eq!(spec["isDraft"], false);
    let reviewed = result.stderr.find("spec-review: session ended").unwrap();
    let ready = result
        .stderr
        .find("PR https://github.com/acme/widgets/pull/3 is ready")
        .unwrap();
    assert!(reviewed < ready, "stderr: {}", result.stderr);
}

#[test]
fn the_spec_review_writes_the_spec_pr_body() {
    let scenario = linear_spec();
    scenario.agent_does_for(
        SPEC,
        r#"gh fake pr issue-20 body '"The whole Spec.\n\nUnaddressed findings: none\n\nCloses #20"'"#,
    );

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let spec = &scenario.gh_state()["prs"][2];
    assert_eq!(
        spec["body"],
        "The whole Spec.\n\nUnaddressed findings: none\n\nCloses #20"
    );
}

#[test]
fn the_spec_review_is_logged_under_the_specs_name() {
    let scenario = linear_spec();

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let logged = result
        .stderr
        .lines()
        .find(|line| line.ends_with("-spec-review.jsonl"))
        .unwrap_or_else(|| panic!("no Spec review log in: {}", result.stderr));
    assert_contains(logged, "/acme-widgets-issue-20-");
}

#[test]
fn a_failed_spec_review_ends_the_spec_run_without_marking_the_spec_pr_ready() {
    let scenario = linear_spec();
    scenario.agent_does_for(SPEC, "exit 1");

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/3\n");
    assert_eq!(sessions_by_issue(&scenario), ["21", "22", "20"]);
    let spec = &scenario.gh_state()["prs"][2];
    assert_eq!(spec["head"], "issue-20");
    assert_eq!(spec["state"], "OPEN");
    assert_eq!(spec["isDraft"], true);
    assert!(
        !result.stderr.contains("is ready for review"),
        "stderr: {}",
        result.stderr
    );
    assert_contains(&result.stderr, "thirdshift: session log: ");
}

#[test]
fn a_spec_pr_closed_during_the_spec_review_fails_the_spec_run() {
    let scenario = linear_spec();
    scenario.agent_does_for(SPEC, r#"gh fake pr issue-20 state '"CLOSED"'"#);

    let result = scenario.run(&[&spec_url(&scenario)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_contains(&result.stderr, "is closed, not open");
    assert_eq!(scenario.gh_state()["prs"][2]["isDraft"], true);
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
