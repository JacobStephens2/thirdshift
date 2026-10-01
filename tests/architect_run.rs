//! Architect runs: `thirdshift architect` runs the Architecture review in its
//! own worktree, detached at the Base branch's head on origin, then checks
//! the plan the review published and swaps its `needs-triage` for
//! `ready-for-agent`. With `--plan-only` it prints the plan's URL and stops.
//! Without, it dispatches the plan as `thirdshift <plan URL>` would, a Spec
//! run or a Run, and ends as that does.

mod support;

use std::fs;

use support::{REPO, RunResult, Scenario};

/// The first issue the fake agent creates: the scenario starts with issue #7.
const PLAN_URL: &str = "https://github.com/acme/widgets/issues/8";

/// The first pull request opened on the fake GitHub.
const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

const NO_FINAL_LINE: &str = "the Architecture review ended without a final line naming its plan";

/// The agent publishes a single Ticket as the plan, labelled `needs-triage`,
/// and names it in the last line of its final message.
const AGENT_PUBLISHES_A_TICKET: &str = r#"
url=$(gh issue create --title "Deepen the session module" --body "The plan" --label needs-triage)
printf 'Published the plan.\n\nArchitecture review plan: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

/// A script in which the agent publishes the plan as
/// [`AGENT_PUBLISHES_A_TICKET`] does, then does `then`.
fn publishes_a_ticket_then(then: &str) -> String {
    format!("{AGENT_PUBLISHES_A_TICKET}{then}\n")
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

/// The Architecture review, the first session, publishes a single Ticket as
/// the plan, #8, and the session that implements #8 opens its PR into `main`.
fn single_ticket_plan() -> Scenario {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, AGENT_PUBLISHES_A_TICKET);
    scenario.agent_does_for(8, &agent_opens_pr(8, "main"));
    scenario
}

/// The Architecture review, the first session, publishes a Spec, #8, with
/// two Tickets that don't block each other, #9 and #10, as the plan. Each
/// Ticket's session opens its PR into the Spec branch, #10's after doing
/// `before_ticket_10`.
fn spec_plan(before_ticket_10: &str) -> Scenario {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        r#"
spec=$(gh issue create --title "Deepen the session module" --body "The Spec" --label needs-triage)
gh issue create --title "Move the logs" --body "A Ticket" --label ready-for-agent
gh issue create --title "Move the sessions" --body "A Ticket" --label ready-for-agent
gh fake sub-issues 8 '[9, 10]'
printf 'Architecture review plan: %s\n' "$spec" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(9, &agent_opens_pr(9, "issue-8"));
    scenario.agent_does_for(
        10,
        &format!("{before_ticket_10}\n{}", agent_opens_pr(10, "issue-8")),
    );
    scenario
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

/// The issue number each agent session after the Architecture review was
/// for, the first Issue URL its prompt names, in order.
fn dispatched_sessions(scenario: &Scenario) -> Vec<String> {
    scenario.claude_calls()[1..]
        .iter()
        .map(|call| {
            let prompt = call["prompt"].as_str().unwrap();
            let (_, after) = prompt.split_once("/issues/").unwrap();
            after.chars().take_while(char::is_ascii_digit).collect()
        })
        .collect()
}

/// Assert the Architect run ended with `pr` in `outcome`, as a Run or a Spec
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

/// A script in which the agent ends with `final_message`, publishing nothing.
fn ends_with(final_message: &str) -> String {
    format!("printf '%s\\n' '{final_message}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n")
}

/// Assert the Architect run left no worktree and no temp directory, and the
/// Launch directory with no branch but `main`.
fn assert_nothing_left_behind(scenario: &Scenario) {
    assert_eq!(scenario.entries("work"), vec![REPO]);
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert_eq!(
        scenario.launch_git(&["branch", "--format=%(refname:short)"]),
        "main\n"
    );
    assert_eq!(scenario.entries("tmp"), Vec::<String>::new());
}

/// What every failed Architect run shares once its review has started: exit
/// 1, nothing on stdout, `cause` on stderr, then the session log as its last
/// line, and nothing left behind.
fn assert_failed(scenario: &Scenario, result: &RunResult, cause: &str) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result.stderr.contains(&format!("thirdshift: {cause}\n")),
        "expected {cause:?} in stderr: {}",
        result.stderr
    );
    let logs = scenario.entries("home/.thirdshift/logs");
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    assert!(
        logs[0].starts_with("acme-widgets-architect-")
            && logs[0].ends_with("-architecture-review.jsonl"),
        "logs: {logs:?}"
    );
    let log = scenario.path("home/.thirdshift/logs").join(&logs[0]);
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: session log: {}", log.display()).as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_nothing_left_behind(scenario);
}

#[test]
fn plan_only_marks_the_published_ticket_ready_and_prints_its_url() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
    assert_eq!(scenario.issue_labels(8), ["ready-for-agent"]);
    assert_nothing_left_behind(&scenario);
}

#[test]
fn plan_only_marks_a_published_spec_ready_and_leaves_its_tickets_and_other_labels() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
spec=$(gh issue create --title "Deepen the session module" --body "The Spec" --label needs-triage,architecture)
gh issue create --title "Move the logs" --body "A Ticket" --label ready-for-agent
printf 'Architecture review plan: %s\n' "$spec" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
    assert_eq!(
        scenario.issue_labels(8),
        ["architecture", "ready-for-agent"]
    );
    assert_eq!(scenario.issue_labels(9), ["ready-for-agent"]);
}

#[test]
fn the_session_prompt_names_the_factory_skills_the_base_branch_and_the_final_line() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("develop", "main", &["Develop work"]);
    scenario.launch_checks_out("develop");
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let prompt = scenario.first_prompt();
    for expected in [
        "/thirdshift:improve-codebase-architecture",
        "/thirdshift:codebase-design",
        "/thirdshift:to-spec",
        "/thirdshift:to-tickets",
        "the base branch develop",
        "\nArchitecture review plan: <",
        "\nArchitecture review idea: <",
        "\nArchitecture review already filed: <",
    ] {
        assert!(prompt.contains(expected), "expected {expected:?}: {prompt}");
    }
    assert!(!prompt.contains("Focus"), "{prompt}");
}

#[test]
fn a_focus_goes_into_the_session_prompt() {
    for args in [
        ["architect", "the Spec run", "--plan-only"],
        ["architect", "--plan-only", "the Spec run"],
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
        let prompt = scenario.first_prompt();
        assert!(
            prompt.contains("\nFocus the review on: the Spec run\n"),
            "{args:?}: {prompt}"
        );
    }
}

#[test]
fn the_review_runs_detached_at_origins_base_branch_and_the_launch_directory_is_never_touched() {
    let scenario = Scenario::new();
    scenario.origin_has_commit("main", "upstream.txt", "upstream\n", "Upstream work");
    let launch = scenario.launch_dir();
    fs::write(launch.join("README.md"), "widgets, edited\n").unwrap();
    fs::write(launch.join("untracked.txt"), "mine\n").unwrap();
    let launch_head = scenario.launch_git(&["rev-parse", "HEAD"]);
    let launch_status = scenario.launch_git(&["status", "--porcelain"]);
    let origin_refs = scenario.origin_git(&["for-each-ref"]);
    scenario.agent_does(&publishes_a_ticket_then(
        r#"
root=$(dirname "$FAKE_CLAUDE_RECORD")
git rev-parse HEAD > "$root/review-head"
ls > "$root/review-files"
echo "a throwaway prototype" > prototype.txt
echo "widgets, by the review" > README.md
"#,
    ));

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let call = &scenario.claude_calls()[0];
    assert_eq!(
        call["cwd"],
        scenario.path("work/widgets-architect").to_str().unwrap()
    );
    assert_eq!(call["branch"], "", "the worktree is on a branch");
    assert_eq!(
        fs::read_to_string(scenario.path("review-head")).unwrap(),
        scenario.origin_git(&["rev-parse", "refs/heads/main"])
    );
    assert_eq!(
        fs::read_to_string(scenario.path("review-files")).unwrap(),
        "README.md\nupstream.txt\n"
    );
    assert_eq!(scenario.launch_git(&["rev-parse", "HEAD"]), launch_head);
    assert_eq!(
        scenario.launch_git(&["status", "--porcelain"]),
        launch_status
    );
    assert_eq!(
        fs::read_to_string(launch.join("README.md")).unwrap(),
        "widgets, edited\n"
    );
    assert_eq!(scenario.origin_git(&["for-each-ref"]), origin_refs);
    assert_nothing_left_behind(&scenario);
}

#[test]
fn progress_lines_cover_the_review_starting_the_plan_it_reported_and_the_label_swap() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "the Spec run", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let mut rest = result.stderr.as_str();
    for line in [
        "thirdshift: starting the Architecture review of main, focused on: the Spec run\n"
            .to_string(),
        "thirdshift: architecture-review: session started\n".to_string(),
        format!("thirdshift: the Architecture review published the plan {PLAN_URL}\n"),
        "thirdshift: marking the plan ready: swapping needs-triage for ready-for-agent on #8\n"
            .to_string(),
    ] {
        let Some(at) = rest.find(&line) else {
            panic!("expected {line:?}, in order, in stderr: {}", result.stderr);
        };
        rest = &rest[at + line.len()..];
    }
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: plan {PLAN_URL} is ready for an agent").as_str())
    );
}

#[test]
fn a_failing_session_fails_the_architect_run_and_leaves_the_plan_needing_triage() {
    let scenario = Scenario::new();
    scenario.agent_does(&publishes_a_ticket_then("exit 3"));

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_failed(&scenario, &result, "claude exited 3");
    assert_eq!(scenario.issue_labels(8), ["needs-triage"]);
}

#[test]
fn an_interrupted_session_fails_the_architect_run_and_leaves_the_plan_needing_triage() {
    let scenario = Scenario::new();
    scenario.agent_does(&publishes_a_ticket_then(
        r#"touch "$(dirname "$FAKE_CLAUDE_RECORD")/started"
sleep 60"#,
    ));

    let result = scenario.run_and_signal(&["architect", "--plan-only"], "started", "INT");

    assert_failed(&scenario, &result, "interrupted");
    assert_eq!(scenario.issue_labels(8), ["needs-triage"]);
}

#[test]
fn a_session_that_ends_without_a_valid_final_line_fails_the_architect_run() {
    for final_message in [
        None,
        Some("Published the plan: https://github.com/acme/widgets/issues/8"),
        Some("Architecture review plan: #8"),
        Some("Architecture review plan: https://github.com/acme/widgets/issues/8, a Ticket"),
    ] {
        let scenario = Scenario::new();
        let publish = "gh issue create --title Plan --body Plan --label needs-triage > /dev/null\n";
        let ending = final_message.map(ends_with).unwrap_or_default();
        scenario.agent_does(&format!("{publish}{ending}"));

        let result = scenario.run(&["architect", "--plan-only"]);

        assert_failed(&scenario, &result, NO_FINAL_LINE);
        assert_eq!(
            scenario.issue_labels(8),
            ["needs-triage"],
            "{final_message:?}"
        );
    }
}

#[test]
fn a_review_that_published_no_plan_fails_the_architect_run_naming_the_issue_it_reported() {
    for (line, cause) in [
        (
            "Architecture review idea: ",
            format!("the Architecture review published no plan: it filed the idea {PLAN_URL}"),
        ),
        (
            "Architecture review already filed: ",
            format!(
                "the Architecture review published no plan: {PLAN_URL} already covers its top recommendation"
            ),
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "gh issue create --title Idea --body Idea --label needs-triage > /dev/null\n{}",
            ends_with(&format!("{line}{PLAN_URL}"))
        ));

        let result = scenario.run(&["architect", "--plan-only"]);

        assert_failed(&scenario, &result, &cause);
        assert_eq!(scenario.issue_labels(8), ["needs-triage"], "{line}");
    }
}

#[test]
fn a_closed_plan_is_refused() {
    let scenario = Scenario::new();
    scenario.agent_does(&publishes_a_ticket_then("gh fake issue 8 CLOSED"));

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_failed(
        &scenario,
        &result,
        &format!("the plan {PLAN_URL} is closed"),
    );
    assert_eq!(scenario.issue_labels(8), ["needs-triage"]);
}

#[test]
fn a_plan_older_than_the_architect_run_is_refused() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["needs-triage"]);
    scenario.issue_created(7, "2026-09-30T23:59:59Z");
    let url = scenario.issue_url(7);
    scenario.agent_does(&ends_with(&format!("Architecture review plan: {url}")));

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_failed(
        &scenario,
        &result,
        &format!("the plan {url} was created before this Architect run started"),
    );
    assert_eq!(scenario.issue_labels(7), ["needs-triage"]);
}

#[test]
fn a_plan_with_another_unready_label_is_refused() {
    for label in ["ready-for-human", "needs-info", "wontfix"] {
        let scenario = Scenario::new();
        scenario.agent_does(&publishes_a_ticket_then(&format!(
            r#"gh fake labels 8 '["needs-triage", "{label}"]'"#
        )));

        let result = scenario.run(&["architect", "--plan-only"]);

        assert_failed(
            &scenario,
            &result,
            &format!("the plan {PLAN_URL} is labelled {label}"),
        );
        assert_eq!(scenario.issue_labels(8), ["needs-triage", label]);
    }
}

#[test]
fn a_plan_in_another_repository_is_refused() {
    let scenario = Scenario::new();
    let url = "https://github.com/acme/gadgets/issues/8";
    scenario.agent_does(&ends_with(&format!("Architecture review plan: {url}")));

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_failed(
        &scenario,
        &result,
        &format!(
            "the plan {url} is not in the repository at origin https://github.com/acme/widgets.git"
        ),
    );
}

#[test]
fn a_detached_head_is_rejected_before_any_work() {
    let scenario = Scenario::new();
    scenario.launch_git(&["checkout", "-q", "--detach"]);

    let result = scenario.run(&["architect", "--plan-only"]);

    scenario.assert_rejected_before_any_work(
        &result,
        "HEAD is detached; check out the branch the Architecture review should scan",
    );
}

#[test]
fn a_base_branch_ahead_of_origin_is_rejected_before_any_work() {
    let scenario = Scenario::new();
    scenario.commit_locally("local.txt", "local\n", "Local work");

    let result = scenario.run(&["architect", "--plan-only"]);

    scenario.assert_rejected_before_any_work(
        &result,
        "local main is 1 commit(s) ahead of origin/main; push them first",
    );
}

#[test]
fn a_missing_git_identity_is_rejected_before_any_work() {
    let scenario = Scenario::new();
    scenario.git_email_is(None);

    let result = scenario.run(&["architect", "--plan-only"]);

    scenario.assert_rejected_before_any_work(&result, "git user.email is not set");
}

#[test]
fn an_origin_that_is_not_on_github_is_rejected_before_any_work() {
    let scenario = Scenario::new();
    scenario.set_origin_url("https://gitlab.com/acme/widgets.git");

    let result = scenario.run(&["architect", "--plan-only"]);

    scenario.assert_rejected_before_any_work(
        &result,
        "origin https://gitlab.com/acme/widgets.git is not a GitHub repository",
    );
}

#[test]
fn a_closed_issue_in_the_repository_does_not_stop_an_architect_run() {
    let scenario = Scenario::new();
    scenario.issue_is(7, "CLOSED");
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
}

#[test]
fn launch_pull_brings_the_launch_directorys_base_branch_up_to_date_first() {
    let scenario = Scenario::new();
    scenario.origin_has_commit("main", "upstream.txt", "upstream\n", "Upstream work");
    scenario.user_config_is("[launch]\npull = true\n");
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.launch_git(&["rev-parse", "refs/heads/main"]),
        scenario.origin_git(&["rev-parse", "refs/heads/main"])
    );
    assert!(
        result
            .stderr
            .contains("thirdshift: updating main in the Launch directory from origin/main\n"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_single_ticket_plan_starts_a_run_on_it_and_ends_as_that_run_does() {
    let scenario = single_ticket_plan();

    let result = scenario.run(&["architect"]);

    let pr = pr_from(&scenario, "issue-8");
    assert_ended_with_pr(&result, &pr, "ready for review");
    assert_eq!(pr["url"], PR_URL);
    assert_eq!(scenario.issue_labels(8), ["ready-for-agent"]);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "sessions: {calls:?}");
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with(&format!("/thirdshift:implement {PLAN_URL}\n")),
        "{prompt}"
    );
    assert_eq!(calls[1]["branch"], "issue-8");
    assert_eq!(pr["base"], "main");
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(
        scenario.origin_log("issue-8").unwrap()[0],
        "Work on 8".to_string()
    );
    scenario.assert_cleaned_up("issue-8");
}

#[test]
fn a_plan_with_tickets_starts_a_spec_run_on_it_and_ends_as_that_spec_run_does() {
    let scenario = spec_plan("");

    let result = scenario.run(&["architect"]);

    let spec_pr = pr_from(&scenario, "issue-8");
    assert_ended_with_pr(&result, &spec_pr, "ready for review");
    assert_eq!(spec_pr["base"], "main");
    assert_eq!(spec_pr["state"], "OPEN");
    assert_eq!(spec_pr["isDraft"], false);
    assert_eq!(scenario.issue_labels(8), ["ready-for-agent"]);
    let mut sessions = dispatched_sessions(&scenario);
    assert_eq!(sessions.pop().as_deref(), Some("8"), "the Spec review");
    sessions.sort();
    assert_eq!(sessions, ["10", "9"]);
    let gh = scenario.gh_state();
    for ticket in [9, 10] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}"));
        assert_eq!(pr["base"], "issue-8");
        assert_eq!(pr["state"], "MERGED");
        assert_eq!(gh["issues"][ticket.to_string()], "CLOSED");
        assert_eq!(
            scenario.origin_file("issue-8", &format!("issue-{ticket}.txt")),
            Some(format!("{ticket}\n"))
        );
    }
    scenario.assert_cleaned_up("issue-8");
}

#[test]
fn merge_merges_the_runs_pull_request_or_the_spec_pr() {
    for (scenario, head) in [
        (single_ticket_plan(), "issue-8"),
        (spec_plan(""), "issue-8"),
    ] {
        let result = scenario.run(&["architect", "merge"]);

        let pr = pr_from(&scenario, head);
        assert_ended_with_pr(&result, &pr, "merged");
        assert_eq!(pr["base"], "main");
        assert_eq!(pr["state"], "MERGED");
        assert!(
            scenario.origin_log("issue-8").is_none(),
            "issue-8 is still on origin"
        );
    }
}

#[test]
fn the_user_configs_merge_default_applies_unless_no_merge_is_given() {
    for (args, outcome, state) in [
        (vec!["architect"], "merged", "MERGED"),
        (vec!["architect", "--no-merge"], "ready for review", "OPEN"),
    ] {
        let scenario = single_ticket_plan();
        scenario.user_config_is("[merge]\nalways = true\n");

        let result = scenario.run(&args);

        let pr = pr_from(&scenario, "issue-8");
        assert_ended_with_pr(&result, &pr, outcome);
        assert_eq!(pr["state"], state, "{args:?}");
    }
}

#[test]
fn parallel_passes_through_to_the_spec_run() {
    // #10's session only succeeds if #9 landed on the Spec branch before it
    // started, as it has only when the Tickets run one at a time.
    let scenario = spec_plan("test -f issue-9.txt");

    let result = scenario.run(&["architect", "parallel", "1"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-8"), "ready for review");
    assert_eq!(dispatched_sessions(&scenario), ["9", "10", "8"]);
}

#[test]
fn parallel_on_a_single_ticket_plan_fails_as_it_does_for_an_issue_that_is_not_a_spec() {
    let scenario = single_ticket_plan();

    let result = scenario.run(&["architect", "parallel", "2"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(
        result.stderr.lines().last(),
        Some("thirdshift: parallel is only for a Spec, and #8 has no sub-issues"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1, "a Run was started");
    assert_eq!(scenario.issue_labels(8), ["ready-for-agent"]);
    assert!(scenario.origin_log("issue-8").is_none());
    assert_nothing_left_behind(&scenario);
}

#[test]
fn a_dispatched_run_that_fails_fails_the_architect_run_as_a_failed_run_does() {
    let scenario = single_ticket_plan();
    scenario.agent_does_for(8, &format!("{}exit 3\n", agent_opens_pr(8, "main")));

    let result = scenario.run(&["architect"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert!(
        result.stderr.contains("thirdshift: claude exited 3\n"),
        "stderr: {}",
        result.stderr
    );
    let last = result.stderr.lines().last().unwrap();
    assert!(
        last.starts_with("thirdshift: session log: ") && last.ends_with("-implement.jsonl"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(8), ["ready-for-agent"]);
}

#[test]
fn progress_lines_show_the_dispatch_after_the_label_swap_and_before_the_run() {
    let scenario = single_ticket_plan();

    let result = scenario.run(&["architect"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let mut rest = result.stderr.as_str();
    for line in [
        "thirdshift: marking the plan ready: swapping needs-triage for ready-for-agent on #8\n"
            .to_string(),
        format!("thirdshift: dispatching the plan {PLAN_URL}, as thirdshift {PLAN_URL} would\n"),
        "thirdshift: implement: session started\n".to_string(),
    ] {
        let Some(at) = rest.find(&line) else {
            panic!("expected {line:?}, in order, in stderr: {}", result.stderr);
        };
        rest = &rest[at + line.len()..];
    }
    assert!(
        !result.stderr.contains("is ready for an agent"),
        "stderr: {}",
        result.stderr
    );
}
