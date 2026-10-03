//! Architect runs: `thirdshift architect` runs the Architecture review in its
//! own worktree, detached at the Base branch's head on origin, then checks
//! the plan the review published, swaps its `needs-triage` for
//! `ready-for-agent` and labels it `architect-plan`. With `--plan-only` it
//! prints the plan's URL and stops.
//! Without, it dispatches the plan as `thirdshift <plan URL>` would, a Spec
//! run or a Run, and ends as that does. A review with no Strong candidate
//! has no plan: the Architect run prints the URL of the idea issue it filed,
//! or of the open issue that already covers it, and dispatches nothing.
//! Asked to, by `email` or the User config, it sends one Run notification,
//! through Resend, here a local stand-in, however it ended, short of being
//! skipped, and the run it dispatched sends none.
//! The Base branch is the branch checked out in the Launch directory, or the
//! one `base <branch>` names, whatever is checked out there, which the run
//! the plan is dispatched as takes as its Base branch too.
//! An Architect run started while another on the repository, or a Pickup run,
//! is still running, or while an Architect plan is still open there, is
//! skipped.

mod support;

use std::fs;

use support::resend::ResendStandIn;
use support::{REPO, RunResult, Scenario, before_command_log, leaves_running};

/// The first issue the fake agent creates: the scenario starts with issue #7.
const PLAN_URL: &str = "https://github.com/acme/widgets/issues/8";

/// The first pull request opened on the fake GitHub.
const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

/// The label thirdshift marks an Architect plan with.
const ARCHITECT_PLAN: &str = "architect-plan";

/// The label of a Claimed issue, which the run a plan is dispatched as
/// swaps the plan's `ready-for-agent` for.
const IN_PROGRESS: &str = "in-progress";

const NO_FINAL_LINE: &str =
    "the Architecture review ended without the final line its prompt asks for";

/// The agent publishes a single Ticket as the plan, labelled `needs-triage`,
/// and names it in the last line of its final message.
const AGENT_PUBLISHES_A_TICKET: &str = r#"
url=$(gh issue create --title "Deepen the session module" --body "The plan" --label needs-triage)
printf 'Published the plan.\n\nArchitecture review plan: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

/// A scenario whose repository has the labels the fake agent publishes its
/// issues with, as `gh issue create` refuses a label the repository lacks.
fn scenario() -> Scenario {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent", "architecture"]);
    scenario
}

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
    let scenario = scenario();
    scenario.agent_does_in_session(1, AGENT_PUBLISHES_A_TICKET);
    scenario.agent_does_for(8, &agent_opens_pr(8, "main"));
    scenario
}

/// The Architecture review, the first session, publishes a Spec, #8, with
/// two Tickets that don't block each other, #9 and #10, as the plan. Each
/// Ticket's session opens its PR into the Spec branch, #10's after doing
/// `before_ticket_10`.
fn spec_plan(before_ticket_10: &str) -> Scenario {
    let scenario = scenario();
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

/// `stderr` after the line saying the Architect run is starting.
fn after_start(stderr: &str) -> &str {
    support::after_start(stderr, "Architect run starting")
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
/// 1, nothing on stdout, `cause` on stderr, then the session log and the
/// Command log as its last lines, and nothing left behind.
fn assert_failed(scenario: &Scenario, result: &RunResult, cause: &str) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result.stderr.contains(&format!("thirdshift: {cause}\n")),
        "expected {cause:?} in stderr: {}",
        result.stderr
    );
    let logs = scenario.entries("home/.thirdshift/logs/sessions");
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    assert!(
        logs[0].starts_with("acme-widgets-architect-")
            && logs[0].ends_with("-architecture-review.jsonl"),
        "logs: {logs:?}"
    );
    let log = scenario
        .path("home/.thirdshift/logs/sessions")
        .join(&logs[0]);
    assert_eq!(
        before_command_log(&result.stderr).last(),
        Some(&format!("thirdshift: session log: {}", log.display()).as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_nothing_left_behind(scenario);
}

/// What every Architect run whose review found no Strong candidate shares:
/// exit 0, `url` alone on stdout, `outcome` as stderr's last line, the
/// review as its only session, and nothing left behind.
fn assert_ended_without_a_plan(scenario: &Scenario, result: &RunResult, url: &str, outcome: &str) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{url}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: {outcome}").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_nothing_left_behind(scenario);
}

#[test]
fn plan_only_marks_the_published_ticket_ready_and_prints_its_url() {
    let scenario = scenario();
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
    assert_eq!(
        scenario.issue_labels(8),
        ["ready-for-agent", ARCHITECT_PLAN]
    );
    assert_nothing_left_behind(&scenario);
}

#[test]
fn plan_only_marks_a_published_spec_ready_and_leaves_its_tickets_and_other_labels() {
    let scenario = scenario();
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
        ["architecture", "ready-for-agent", ARCHITECT_PLAN]
    );
    assert_eq!(scenario.issue_labels(9), ["ready-for-agent"]);
}

#[test]
fn the_architect_plan_label_is_created_when_the_repository_lacks_it() {
    let scenario = scenario();
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);
    assert!(!scenario.repo_labels().contains(&ARCHITECT_PLAN.to_string()));

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.repo_labels(),
        [
            "needs-triage",
            "ready-for-agent",
            "architecture",
            ARCHITECT_PLAN
        ]
    );
    let created = scenario.gh_calls_of("label", "create");
    assert_eq!(created.len(), 1, "{created:?}");
    assert_eq!(created[0][2], ARCHITECT_PLAN);
}

#[test]
fn a_repository_that_has_the_architect_plan_label_keeps_it_as_it_is() {
    let scenario = scenario();
    scenario.repo_has_labels(&["needs-triage", "Architect-Plan"]);
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.repo_labels(), ["needs-triage", "Architect-Plan"]);
    assert!(scenario.gh_calls_of("label", "create").is_empty());
    assert_eq!(
        scenario.issue_labels(8),
        ["ready-for-agent", ARCHITECT_PLAN]
    );
}

#[test]
fn the_session_prompt_names_the_factory_skills_the_base_branch_and_the_final_line() {
    let scenario = scenario();
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
        let scenario = scenario();
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
    let scenario = scenario();
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
    let scenario = scenario();
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "the Spec run", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let mut rest = result.stderr.as_str();
    for line in [
        "thirdshift: starting the Architecture review of main, focused on: the Spec run\n"
            .to_string(),
        "thirdshift: architecture-review: session started\n".to_string(),
        format!("thirdshift: the Architecture review published the plan {PLAN_URL}\n"),
        "thirdshift: marking the plan ready: swapping needs-triage for ready-for-agent and adding architect-plan on #8\n"
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
    let scenario = scenario();
    scenario.agent_does(&publishes_a_ticket_then("exit 3"));

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_failed(&scenario, &result, "claude exited 3");
    assert_eq!(scenario.issue_labels(8), ["needs-triage"]);
}

#[test]
fn an_interrupted_session_fails_the_architect_run_and_leaves_the_plan_needing_triage() {
    let scenario = scenario();
    scenario.agent_does(&publishes_a_ticket_then(
        r#"touch "$(dirname "$FAKE_CLAUDE_RECORD")/started"
sleep 60"#,
    ));

    let result = scenario.run_and_signal(&["architect", "--plan-only"], "started", "INT");

    assert_failed(&scenario, &result, "interrupted");
    assert_eq!(scenario.issue_labels(8), ["needs-triage"]);
}

#[test]
fn a_review_whose_resume_ends_with_killed_background_work_is_read_by_the_resumes_final_message() {
    let scenario = scenario();
    scenario.agent_does_in_session(1, &leaves_running("cargo test"));
    scenario.agent_does_in_session(
        2,
        &format!("{AGENT_PUBLISHES_A_TICKET}{}", leaves_running("cargo test")),
    );

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    assert_eq!(result.stdout, format!("{}\n", scenario.issue_url(8)));
    assert!(
        result.stderr.contains(
            "thirdshift: architecture-review: the Resume ended with a background task still \
             running (cargo test), which was killed; carrying on"
        ),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        scenario.issue_labels(8),
        ["ready-for-agent", ARCHITECT_PLAN]
    );
}

#[test]
fn an_architect_run_that_fails_after_killed_background_work_names_that_work_too() {
    let scenario = scenario();
    scenario.agent_does(&leaves_running("cargo test"));

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    let cause = format!(
        "architecture-review session ended with a background task still running (cargo test), \
         which was killed, and a later step failed: {NO_FINAL_LINE}"
    );
    assert!(
        result.stderr.contains(&format!("thirdshift: {cause}\n")),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_session_that_ends_without_a_valid_final_line_fails_the_architect_run() {
    for final_message in [
        None,
        Some("Published the plan: https://github.com/acme/widgets/issues/8"),
        Some("Architecture review plan: #8"),
        Some("Architecture review plan: https://github.com/acme/widgets/issues/8, a Ticket"),
    ] {
        let scenario = scenario();
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
fn a_review_that_files_an_idea_prints_its_url_and_changes_no_label() {
    let scenario = scenario();
    let idea = scenario.issue_url(8);
    scenario.agent_does(
        r#"
url=$(gh issue create --title "Deepen the session module" --body "The idea" --label needs-triage)
printf 'No Strong candidate.\n\nArchitecture review idea: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_ended_without_a_plan(
        &scenario,
        &result,
        &idea,
        &format!("no Strong candidate: the Architecture review filed the idea {idea}"),
    );
    assert_eq!(scenario.issue_labels(8), ["needs-triage"]);
}

#[test]
fn a_review_whose_idea_is_already_filed_prints_that_issues_url_and_files_and_changes_nothing() {
    let scenario = scenario();
    scenario.issue_labelled(7, &["needs-triage"]);
    let url = scenario.issue_url(7);
    scenario.agent_does(&ends_with(&format!(
        "Architecture review already filed: {url}"
    )));
    let github = scenario.gh_state();

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_ended_without_a_plan(
        &scenario,
        &result,
        &url,
        &format!(
            "no Strong candidate: {url} already covers the Architecture review's top recommendation, so it filed nothing"
        ),
    );
    assert_eq!(scenario.gh_state(), github);
}

#[test]
fn a_review_with_no_strong_candidate_dispatches_nothing_without_plan_only() {
    let scenario = scenario();
    scenario.issue_labelled(7, &["needs-triage"]);
    let url = scenario.issue_url(7);
    scenario.agent_does(&ends_with(&format!("Architecture review idea: {url}")));
    let github = scenario.gh_state();

    let result = scenario.run(&["architect", "merge"]);

    assert_ended_without_a_plan(
        &scenario,
        &result,
        &url,
        &format!("no Strong candidate: the Architecture review filed the idea {url}"),
    );
    assert_eq!(scenario.gh_state(), github);
}

#[test]
fn a_closed_plan_is_refused() {
    let scenario = scenario();
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
    let scenario = scenario();
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
        let scenario = scenario();
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
    let scenario = scenario();
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
    let scenario = scenario();
    scenario.launch_git(&["checkout", "-q", "--detach"]);

    let result = scenario.run(&["architect", "--plan-only"]);

    scenario.assert_rejected_before_any_work(
        &result,
        "HEAD is detached; check out the branch the work should be based on, \
         or name it with base <branch>",
    );
}

#[test]
fn a_base_branch_ahead_of_origin_is_rejected_before_any_work() {
    let scenario = scenario();
    scenario.commit_locally("local.txt", "local\n", "Local work");

    let result = scenario.run(&["architect", "--plan-only"]);

    scenario.assert_rejected_before_any_work(
        &result,
        "local main is 1 commit(s) ahead of origin/main; push them first",
    );
}

#[test]
fn a_missing_git_identity_is_rejected_before_any_work() {
    let scenario = scenario();
    scenario.git_email_is(None);

    let result = scenario.run(&["architect", "--plan-only"]);

    scenario.assert_rejected_before_any_work(&result, "git user.email is not set");
}

#[test]
fn an_origin_that_is_not_on_github_is_rejected_before_any_work() {
    let scenario = scenario();
    scenario.set_origin_url("https://gitlab.com/acme/widgets.git");

    let result = scenario.run(&["architect", "--plan-only"]);

    scenario.assert_rejected_before_any_work(
        &result,
        "origin https://gitlab.com/acme/widgets.git is not a GitHub repository",
    );
}

#[test]
fn a_closed_issue_in_the_repository_does_not_stop_an_architect_run() {
    let scenario = scenario();
    scenario.issue_is(7, "CLOSED");
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
}

#[test]
fn launch_pull_brings_the_launch_directorys_base_branch_up_to_date_first() {
    let scenario = scenario();
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
    assert_eq!(scenario.issue_labels(8), [ARCHITECT_PLAN, IN_PROGRESS]);
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
fn a_plan_labelled_needs_triage_in_another_case_is_marked_ready_and_dispatched() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["Needs-Triage", "ready-for-agent"]);
    scenario.agent_does_in_session(
        1,
        r#"
url=$(gh issue create --title "Deepen the session module" --body "The plan" --label Needs-Triage)
printf 'Architecture review plan: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(8, &agent_opens_pr(8, "main"));

    let result = scenario.run(&["architect"]);

    let pr = pr_from(&scenario, "issue-8");
    assert_ended_with_pr(&result, &pr, "ready for review");
    assert_eq!(scenario.issue_labels(8), [ARCHITECT_PLAN, IN_PROGRESS]);
    assert_eq!(scenario.claude_calls().len(), 2);
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
    assert_eq!(scenario.issue_labels(8), [ARCHITECT_PLAN, IN_PROGRESS]);
    let dispatch = format!("dispatching the plan {PLAN_URL}, as thirdshift {PLAN_URL} would\n");
    let dispatched = result.stderr.find(&dispatch);
    let started = result.stderr.find("thirdshift: starting #9\n");
    assert!(
        dispatched.is_some() && dispatched < started,
        "stderr: {}",
        result.stderr
    );
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
        assert_eq!(scenario.issue_labels(ticket), ["ready-for-agent"]);
        assert_eq!(
            scenario.origin_file("issue-8", &format!("issue-{ticket}.txt")),
            Some(format!("{ticket}\n"))
        );
    }
    scenario.assert_cleaned_up("issue-8");
}

#[test]
fn merge_merges_the_runs_pull_request_or_the_spec_pr() {
    for scenario in [single_ticket_plan(), spec_plan("")] {
        let result = scenario.run(&["architect", "merge"]);

        let pr = pr_from(&scenario, "issue-8");
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
        before_command_log(&result.stderr).last(),
        Some(&"thirdshift: parallel is only for a Spec, and #8 has no sub-issues"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1, "a Run was started");
    assert_eq!(
        scenario.issue_labels(8),
        ["ready-for-agent", ARCHITECT_PLAN]
    );
    assert!(scenario.origin_log("issue-8").is_none());
    assert_nothing_left_behind(&scenario);
}

const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;

const GREEN: &str = r#"[{"name": "test", "conclusion": "success"}]"#;

/// A script that sets the check runs on the session's head to `checks`.
fn checks_on_head(checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse HEAD)\" '{checks}'\n")
}

/// A script that sets the check runs on origin's `branch` to `checks`.
fn checks_on_origin(branch: &str, checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse origin/{branch})\" '{checks}'\n")
}

/// A script in which the agent for the Base fix issue `issue` commits a fix
/// and opens its PR into `base`, with `test` green on its head.
fn base_fix_opens_pr(issue: u32, base: &str) -> String {
    format!(
        r#"
echo "fixed" > ci-fix.txt
git add ci-fix.txt
git commit -q -m "Fix CI on {base}"
gh pr create --base {base} --head issue-{issue} --title "Fix CI on {base}" --body "Closes #{issue}"
{green}"#,
        green = checks_on_head(GREEN)
    )
}

/// A single-Ticket plan, #8, whose Run opens its PR with `test` red on its
/// head and on `main`: an Inherited failure. The agent for the Base fix
/// issue, #9, the next issue, fixes it.
fn single_ticket_plan_that_inherits_a_failure() -> Scenario {
    let scenario = single_ticket_plan();
    scenario.agent_does_for(
        8,
        &format!(
            "{}{}{}",
            agent_opens_pr(8, "main"),
            checks_on_head(RED),
            checks_on_origin("main", RED)
        ),
    );
    scenario.agent_does_for(9, &base_fix_opens_pr(9, "main"));
    scenario
}

/// The cause of a Run that failed on `test`, an Inherited failure from
/// `main` as it was when the scenario started.
fn inherited_failure(scenario: &Scenario) -> String {
    let red_base = scenario.origin_git(&["rev-parse", "main"]);
    format!(
        "CI red on test, which also fails on main at {}; fix main first",
        &red_base[..7]
    )
}

#[test]
fn base_fix_lets_the_dispatched_run_start_a_base_fix() {
    for flag in ["base-fix", "--base-fix"] {
        let scenario = single_ticket_plan_that_inherits_a_failure();

        let result = scenario.run(&["architect", flag]);

        let pr = pr_from(&scenario, "issue-8");
        assert_ended_with_pr(&result, &pr, "ready for review");
        assert!(
            result.stderr.contains(
                "thirdshift: starting Base fix #9 into main: https://github.com/acme/widgets/issues/9\n"
            ),
            "stderr: {}",
            result.stderr
        );
        let gh = scenario.gh_state();
        assert_eq!(gh["titles"]["9"], "CI red on main: test", "{flag}");
        let fix = pr_from(&scenario, "issue-9");
        assert_eq!(fix["base"], "main");
        assert_eq!(fix["state"], "MERGED");
        assert_eq!(
            scenario.origin_file("issue-8", "ci-fix.txt").as_deref(),
            Some("fixed\n")
        );
    }
}

#[test]
fn without_base_fix_the_dispatched_run_fails_on_an_inherited_failure() {
    let scenario = single_ticket_plan_that_inherits_a_failure();
    let cause = inherited_failure(&scenario);

    let result = scenario.run(&["architect", "merge", "no-email"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    // The offer retries the plan as a Run of its own: another Architect run
    // would start a new review instead.
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: {cause}\n\
             thirdshift: Base check: test\n\
             thirdshift: Retry with: thirdshift {PLAN_URL} merge --no-email base-fix\n"
        )),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
}

#[test]
fn base_fix_reaches_each_tickets_run_of_the_dispatched_spec_run() {
    // The first Ticket's Run, #9's, meets an Inherited failure from the
    // Spec branch. One Ticket at a time, so the Spec branch can't move
    // under it, which would have it merged in again instead. The Base
    // fix issue is the next after the Tickets, #11.
    let scenario = spec_plan("");
    scenario.agent_does_for(
        9,
        &format!(
            "{}{}{}",
            agent_opens_pr(9, "issue-8"),
            checks_on_head(RED),
            checks_on_origin("issue-8", RED)
        ),
    );
    scenario.agent_does_for(11, &base_fix_opens_pr(11, "issue-8"));

    let result = scenario.run(&["architect", "base-fix", "parallel", "1"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-8"), "ready for review");
    assert!(
        result.stderr.contains(
            "#9: starting Base fix #11 into issue-8: https://github.com/acme/widgets/issues/11\n"
        ),
        "stderr: {}",
        result.stderr
    );
    let fix = pr_from(&scenario, "issue-11");
    assert_eq!(fix["base"], "issue-8");
    assert_eq!(fix["state"], "MERGED");
    for ticket in ["issue-9", "issue-10"] {
        assert_eq!(pr_from(&scenario, ticket)["state"], "MERGED", "{ticket}");
    }
    for file in ["issue-9.txt", "issue-10.txt", "ci-fix.txt"] {
        assert!(scenario.origin_file("issue-8", file).is_some(), "{file}");
    }
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
    let last = *before_command_log(&result.stderr).last().unwrap();
    assert!(
        last.starts_with("thirdshift: session log: ") && last.ends_with("-implement.jsonl"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(8), [ARCHITECT_PLAN, IN_PROGRESS]);
}

#[test]
fn progress_lines_show_the_dispatch_after_the_label_swap_and_before_the_run() {
    let scenario = single_ticket_plan();

    let result = scenario.run(&["architect"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let mut rest = result.stderr.as_str();
    for line in [
        "thirdshift: marking the plan ready: swapping needs-triage for ready-for-agent and adding architect-plan on #8\n"
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

/// A script that records the commit the session's worktree is at as
/// `review-head` in the scenario root.
const RECORDS_THE_REVIEW_HEAD: &str =
    r#"git rev-parse HEAD > "$(dirname "$FAKE_CLAUDE_RECORD")/review-head""#;

/// A scenario whose origin has a `develop` branch, one commit ahead of
/// `main`, that the Launch directory has never fetched. The Launch directory
/// stays on `main`, or with `detached` on a detached HEAD, with an
/// uncommitted change.
fn develop_on_origin(detached: bool) -> Scenario {
    let scenario = scenario();
    scenario.origin_has_branch("develop", "main", &["Develop work"]);
    if detached {
        scenario.launch_git(&["checkout", "-q", "--detach"]);
    }
    fs::write(scenario.launch_dir().join("README.md"), "widgets, edited\n").unwrap();
    scenario
}

/// The head of `branch` on origin.
fn origin_head(scenario: &Scenario, branch: &str) -> String {
    scenario.origin_git(&["rev-parse", &format!("refs/heads/{branch}")])
}

#[test]
fn base_starts_the_review_at_the_named_branchs_origin_head_whatever_is_checked_out() {
    for detached in [false, true] {
        let scenario = develop_on_origin(detached);
        let checked_out = scenario.launch_git(&["branch", "--show-current"]);
        let launch_status = scenario.launch_git(&["status", "--porcelain"]);
        scenario.agent_does(&publishes_a_ticket_then(RECORDS_THE_REVIEW_HEAD));

        let result = scenario.run(&["architect", "base", "develop", "--plan-only"]);

        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
        assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
        assert_eq!(
            fs::read_to_string(scenario.path("review-head")).unwrap(),
            origin_head(&scenario, "develop"),
            "detached: {detached}"
        );
        let prompt = scenario.first_prompt();
        assert!(prompt.contains("the base branch develop"), "{prompt}");
        assert_eq!(
            scenario.launch_git(&["branch", "--show-current"]),
            checked_out
        );
        assert_eq!(
            scenario.launch_git(&["status", "--porcelain"]),
            launch_status
        );
    }
}

#[test]
fn base_carries_the_named_branch_to_the_dispatched_run_whatever_is_checked_out() {
    for detached in [false, true] {
        let scenario = develop_on_origin(detached);
        scenario.agent_does_in_session(1, AGENT_PUBLISHES_A_TICKET);
        scenario.agent_does_for(8, &agent_opens_pr(8, "develop"));

        let result = scenario.run(&["architect", "base", "develop"]);

        let pr = pr_from(&scenario, "issue-8");
        assert_ended_with_pr(&result, &pr, "ready for review");
        assert_eq!(pr["base"], "develop", "detached: {detached}");
        assert_eq!(
            scenario.origin_log("issue-8").unwrap(),
            ["Work on 8", "Develop work", "Initial commit"],
            "detached: {detached}"
        );
        let prompt = scenario.claude_calls()[1]["prompt"].to_string();
        assert!(prompt.contains("develop"), "{prompt}");
        scenario.assert_cleaned_up("issue-8");
    }
}

#[test]
fn base_carries_the_named_branch_to_the_dispatched_spec_run_whatever_is_checked_out() {
    for detached in [false, true] {
        let scenario = spec_plan("");
        scenario.origin_has_branch("develop", "main", &["Develop work"]);
        if detached {
            scenario.launch_git(&["checkout", "-q", "--detach"]);
        }

        let result = scenario.run(&["architect", "--base", "develop"]);

        let spec_pr = pr_from(&scenario, "issue-8");
        assert_ended_with_pr(&result, &spec_pr, "ready for review");
        assert_eq!(spec_pr["base"], "develop", "detached: {detached}");
        assert_eq!(spec_pr["isDraft"], false);
        assert_eq!(
            scenario.origin_file("issue-8", "develop-0.txt").as_deref(),
            Some("Develop work"),
            "the Spec branch is not branched off develop"
        );
        for ticket in [9, 10] {
            let pr = pr_from(&scenario, &format!("issue-{ticket}"));
            assert_eq!(pr["base"], "issue-8");
            assert_eq!(pr["state"], "MERGED");
        }
        scenario.assert_cleaned_up("issue-8");
    }
}

#[test]
fn merge_merges_the_dispatched_runs_pull_request_into_the_named_branch() {
    let scenario = develop_on_origin(false);
    scenario.agent_does_in_session(1, AGENT_PUBLISHES_A_TICKET);
    scenario.agent_does_for(8, &agent_opens_pr(8, "develop"));
    let main = origin_head(&scenario, "main");

    let result = scenario.run(&["architect", "merge", "base", "develop"]);

    let pr = pr_from(&scenario, "issue-8");
    assert_ended_with_pr(&result, &pr, "merged");
    assert_eq!(pr["base"], "develop");
    assert_eq!(
        scenario.origin_file("develop", "issue-8.txt").as_deref(),
        Some("8\n")
    );
    assert_eq!(origin_head(&scenario, "main"), main);
}

#[test]
fn a_named_branch_that_is_not_on_origin_is_rejected_before_any_work() {
    let scenario = scenario();
    scenario.launch_git(&["branch", "local-only"]);

    for branch in ["nowhere", "local-only"] {
        let result = scenario.run(&["architect", "base", branch]);

        scenario.assert_rejected_before_any_work(
            &result,
            &format!("base branch {branch} does not exist on origin; push it first"),
        );
        assert_eq!(result.code, Some(1), "{branch}");
    }
}

#[test]
fn a_named_branch_whose_local_copy_is_ahead_of_origin_is_rejected_before_any_work() {
    let scenario = scenario();
    scenario.origin_has_branch("develop", "main", &["Develop work"]);
    scenario.launch_checks_out("develop");
    scenario.commit_locally("local.txt", "local\n", "Local work");
    scenario.launch_git(&["checkout", "-q", "main"]);

    let result = scenario.run(&["architect", "base", "develop", "--plan-only"]);

    scenario.assert_rejected_before_any_work(
        &result,
        "local develop is 1 commit(s) ahead of origin/develop; push them first",
    );
}

#[test]
fn launch_pull_leaves_the_checkout_alone_unless_the_named_branch_is_the_one_checked_out() {
    for (checkout, updated) in [
        (vec!["checkout", "-q", "main"], false),
        (vec!["checkout", "-q", "--detach", "main"], false),
        (vec!["checkout", "-q", "develop"], true),
    ] {
        let scenario = scenario();
        scenario.origin_has_branch("develop", "main", &["Develop work"]);
        scenario.launch_checks_out("develop");
        scenario.launch_git(&checkout);
        scenario.origin_has_commit("main", "upstream.txt", "upstream\n", "Upstream work");
        scenario.origin_has_commit("develop", "more.txt", "more\n", "More develop work");
        scenario.user_config_is("[launch]\npull = true\n");
        scenario.agent_does(AGENT_PUBLISHES_A_TICKET);
        let head = scenario.launch_git(&["rev-parse", "HEAD"]);
        let main = scenario.launch_git(&["rev-parse", "refs/heads/main"]);

        let result = scenario.run(&["architect", "base", "develop", "--plan-only"]);

        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
        assert_eq!(
            result.stderr.contains("Launch directory"),
            updated,
            "{checkout:?}: {}",
            result.stderr
        );
        assert_eq!(
            scenario.launch_git(&["rev-parse", "refs/heads/main"]),
            main,
            "{checkout:?}"
        );
        if updated {
            assert_eq!(
                scenario.launch_git(&["rev-parse", "HEAD"]),
                origin_head(&scenario, "develop")
            );
            assert!(
                result.stderr.contains(
                    "thirdshift: updating develop in the Launch directory from origin/develop\n"
                ),
                "stderr: {}",
                result.stderr
            );
        } else {
            assert_eq!(
                scenario.launch_git(&["rev-parse", "HEAD"]),
                head,
                "{checkout:?}"
            );
        }
    }
}

#[test]
fn launch_pull_leaves_the_checkout_alone_when_the_dispatched_run_starts_too() {
    let scenario = develop_on_origin(false);
    scenario.origin_has_commit("main", "upstream.txt", "upstream\n", "Upstream work");
    scenario.user_config_is("[launch]\npull = true\n");
    scenario.agent_does_in_session(1, AGENT_PUBLISHES_A_TICKET);
    scenario.agent_does_for(8, &agent_opens_pr(8, "develop"));
    let head = scenario.launch_git(&["rev-parse", "HEAD"]);

    let result = scenario.run(&["architect", "base", "develop"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-8"), "ready for review");
    assert_eq!(scenario.launch_git(&["rev-parse", "HEAD"]), head);
    assert!(
        !result.stderr.contains("Launch directory"),
        "stderr: {}",
        result.stderr
    );
}

const KEY: &str = "re_test_123";

const ACCEPTED: &str = r#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#;

/// The environment for an Architect run against `resend`, with a Resend API
/// key.
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

#[test]
fn plan_only_sends_one_notification_naming_the_plan() {
    let scenario = scenario();
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "--plan-only", "--email", "me@example.com"],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
    assert_eq!(resend.requests()[0].body["to"], "me@example.com");
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Architect run: plan published"
    );
    assert!(
        text.starts_with(&format!("Review:       plan published: {PLAN_URL}\n")),
        "{text}"
    );
    assert!(!text.contains("Dispatched:"), "{text}");
    assert!(text.contains("Took:"), "{text}");
}

#[test]
fn a_review_that_files_an_idea_sends_one_notification_naming_the_idea() {
    let scenario = scenario();
    scenario.agent_does(
        r#"
url=$(gh issue create --title "Deepen the session module" --body "The idea" --label needs-triage)
printf 'Architecture review idea: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "email", "me@example.com"],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Architect run: idea filed"
    );
    assert!(
        text.starts_with(&format!("Review:       idea filed: {PLAN_URL}\n")),
        "{text}"
    );
    assert!(!text.contains("Dispatched:"), "{text}");
}

#[test]
fn a_review_whose_idea_is_already_filed_sends_one_notification_naming_that_issue() {
    let scenario = scenario();
    let url = scenario.issue_url(7);
    scenario.agent_does(&ends_with(&format!(
        "Architecture review already filed: {url}"
    )));
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "--email", "me@example.com", "--plan-only"],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Architect run: idea already filed"
    );
    assert!(
        text.starts_with(&format!("Review:       idea already filed: {url}\n")),
        "{text}"
    );
}

#[test]
fn a_failed_review_sends_one_notification_with_the_cause_and_the_session_log() {
    let scenario = scenario();
    scenario.agent_does(&publishes_a_ticket_then("exit 3"));
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "--email", "me@example.com"],
    );

    assert_failed(&scenario, &result, "claude exited 3");
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Architect run: review failed"
    );
    let ending = result.stderr.lines().rev().take(2).collect::<Vec<_>>();
    let command_log = ending[0].strip_prefix("thirdshift: command log: ").unwrap();
    let log = ending[1].strip_prefix("thirdshift: session log: ").unwrap();
    assert!(
        text.starts_with(&format!(
            "Review:       failed\n\
             Cause:        claude exited 3\n\
             Session log:  {log}\n\
             Command log:  {command_log}\n"
        )),
        "{text}"
    );
}

/// The User config of a machine where every Run and every Architect run
/// sends a Run notification.
const EMAIL_ALWAYS: &str = "[email]\nalways = true\nto = \"config@example.com\"\n";

#[test]
fn email_always_sends_one_notification_for_the_review_and_the_run_it_dispatched() {
    let scenario = single_ticket_plan();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["architect"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(resend.requests()[0].body["to"], "config@example.com");
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Architect run: ready for review"
    );
    assert!(
        text.starts_with(&format!(
            "Review:       plan published: {PLAN_URL}\n\
             Dispatched:   ready for review\n\
             Pull request: {PR_URL}\n"
        )),
        "{text}"
    );
}

#[test]
fn the_one_notification_says_what_became_of_the_dispatched_runs_base_fix() {
    let scenario = single_ticket_plan_that_inherits_a_failure();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["architect", "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Architect run: ready for review"
    );
    assert!(
        text.starts_with(&format!(
            "Review:       plan published: {PLAN_URL}\n\
             Dispatched:   ready for review\n\
             Pull request: {PR_URL}\n\
             Base fix:     https://github.com/acme/widgets/issues/9 merged\n"
        )),
        "{text}"
    );
}

#[test]
fn a_dispatched_spec_run_sends_no_notification_of_its_own_and_its_tickets_are_in_the_one_sent() {
    let scenario = spec_plan("");
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["architect", "merge"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(subject, "[thirdshift] acme/widgets Architect run: merged");
    let spec_pr = pr_from(&scenario, "issue-8");
    assert!(
        text.starts_with(&format!(
            "Review:       plan published: {PLAN_URL}\n\
             Dispatched:   merged\n\
             Pull request: {}\n",
            spec_pr["url"].as_str().unwrap()
        )),
        "{text}"
    );
    let (_, tickets) = text.split_once("\nTickets:\n").expect(&text);
    for ticket in [9, 10] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}"));
        let line = format!("#{ticket} landed with {}\n", pr["url"].as_str().unwrap());
        assert!(tickets.contains(&line), "expected {line:?} in: {text}");
    }
}

#[test]
fn a_dispatched_run_that_fails_sends_one_notification_with_its_outcome_and_cause() {
    let scenario = single_ticket_plan();
    scenario.agent_does_for(8, &format!("{}exit 3\n", agent_opens_pr(8, "main")));
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "--email", "me@example.com"],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(subject, "[thirdshift] acme/widgets Architect run: failed");
    assert!(
        text.starts_with(&format!(
            "Review:       plan published: {PLAN_URL}\n\
             Dispatched:   failed\n\
             Pull request: {PR_URL}\n\
             Cause:        claude exited 3\n"
        )),
        "{text}"
    );
}

#[test]
fn with_no_address_known_the_architect_run_stops_before_any_work_and_sends_nothing() {
    let scenario = scenario();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["architect", "--email"]);

    scenario.assert_rejected_before_any_work(&result, "no email address");
    assert!(resend.requests().is_empty());
}

#[test]
fn a_failed_send_is_a_warning_that_changes_neither_the_exit_code_nor_stdout() {
    let scenario = scenario();
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);
    let resend = ResendStandIn::replying(500, "upstream exploded");

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "--plan-only", "--email", "me@example.com"],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
    assert_eq!(resend.requests().len(), 1);
    let warning = result.stderr.lines().find(|line| line.contains("warning:"));
    assert!(
        warning.is_some_and(|warning| warning.contains("upstream exploded")),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn an_interrupted_review_sends_one_notification_that_it_was_interrupted() {
    let scenario = scenario();
    scenario.agent_does(
        r#"touch "$(dirname "$FAKE_CLAUDE_RECORD")/started"
sleep 60"#,
    );
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = scenario.run_and_signal_with_env(
        &["architect", "--email", "me@example.com"],
        &resend_env(&resend),
        "started",
        "TERM",
    );

    assert_failed(&scenario, &result, "interrupted");
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Architect run: interrupted"
    );
    assert!(text.starts_with("Review:       interrupted\n"), "{text}");
    assert!(!text.contains("Cause:"), "{text}");
}

#[test]
fn a_plan_that_fails_its_checks_sends_one_notification_naming_it_in_the_cause() {
    let scenario = scenario();
    scenario.agent_does(&publishes_a_ticket_then("gh fake issue 8 CLOSED"));
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "--email", "me@example.com"],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Architect run: review failed"
    );
    assert!(
        text.starts_with(&format!(
            "Review:       failed\n\
             Cause:        the plan {PLAN_URL} is closed\n"
        )),
        "{text}"
    );
}

#[test]
fn a_dispatched_spec_run_that_fails_sends_one_notification_with_each_tickets_outcome() {
    let scenario = spec_plan("exit 3");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "--email", "me@example.com"],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(subject, "[thirdshift] acme/widgets Architect run: failed");
    assert!(
        text.starts_with(&format!(
            "Review:       plan published: {PLAN_URL}\n\
             Dispatched:   failed\n"
        )),
        "{text}"
    );
    let (_, tickets) = text.split_once("\nTickets:\n").expect(&text);
    let landed = pr_from(&scenario, "issue-9");
    let landed = format!("#9 landed with {}\n", landed["url"].as_str().unwrap());
    assert!(tickets.contains(&landed), "expected {landed:?} in: {text}");
    assert!(tickets.contains("#10 failed: "), "{text}");
}

/// What a skipped Architect run says when another is running on the
/// scenario's repository.
const ALREADY_RUNNING: &str =
    "thirdshift: an Architect run or a Pickup run is already running on acme/widgets\n";

/// Assert the Architect run was skipped as one is already running: exit 0,
/// that line alone on stderr, and nothing on stdout.
fn assert_skipped_as_already_running(result: &RunResult) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(after_start(&result.stderr), ALREADY_RUNNING);
    assert_eq!(result.stdout, "");
}

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
fn a_second_architect_run_on_the_repository_is_skipped_while_the_first_is_mid_review() {
    let scenario = scenario();
    scenario.user_config_is("[launch]\npull = true\n");
    scenario.agent_does(&format!(
        "{AGENT_WAITS_FOR_RELEASE}{AGENT_PUBLISHES_A_TICKET}"
    ));
    let first = scenario.run_until(&["architect", "--plan-only"], &[], "started");
    scenario.origin_has_commit("main", "upstream.txt", "upstream\n", "Upstream work");
    let launch_main = scenario.launch_git(&["rev-parse", "refs/heads/main"]);
    let (github, gh_calls) = (scenario.gh_state(), scenario.gh_calls());

    let second = scenario.run(&["architect"]);

    assert_skipped_as_already_running(&second);
    assert_eq!(scenario.claude_calls().len(), 1, "a session was started");
    assert_eq!(
        scenario.launch_git(&["rev-parse", "refs/heads/main"]),
        launch_main,
        "the skipped run did a launch pull"
    );
    assert_eq!(scenario.entries("work"), [REPO, "widgets-architect"]);
    assert_eq!(scenario.gh_state(), github);
    assert_eq!(scenario.gh_calls(), gh_calls);

    release(&scenario);
    let first = first.finish();
    assert_eq!(first.code, Some(0), "stderr: {}", first.stderr);
    assert_eq!(first.stdout, format!("{PLAN_URL}\n"));
}

#[test]
fn a_second_architect_run_is_skipped_while_the_firsts_dispatched_run_or_spec_run_is_still_going() {
    for (plan, held_issue, base) in [
        (single_ticket_plan as fn() -> Scenario, 8, "main"),
        (|| spec_plan(""), 9, "issue-8"),
    ] {
        let scenario = plan();
        scenario.agent_does_for(
            held_issue,
            &format!(
                "{AGENT_WAITS_FOR_RELEASE}{}",
                agent_opens_pr(held_issue, base)
            ),
        );
        let first = scenario.run_until(&["architect"], &[], "started");
        assert!(
            scenario
                .issue_labels(8)
                .contains(&ARCHITECT_PLAN.to_string())
        );

        let second = scenario.run(&["architect"]);

        // Never as one whose Architect plan is still open: the lock is tried
        // first, so the first run's own, labelled by now, is not reported.
        assert_skipped_as_already_running(&second);
        release(&scenario);
        let first = first.finish();
        assert_ended_with_pr(&first, &pr_from(&scenario, "issue-8"), "ready for review");
        let reviews = scenario.claude_calls().into_iter().filter(|call| {
            let prompt = call["prompt"].as_str().unwrap();
            prompt.contains("/thirdshift:improve-codebase-architecture")
        });
        assert_eq!(reviews.count(), 1, "a second review was started");
    }
}

#[test]
fn once_the_first_architect_run_has_ended_a_new_one_runs() {
    let scenario = scenario();
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);
    let first = scenario.run(&["architect", "--plan-only"]);
    assert_eq!(first.code, Some(0), "stderr: {}", first.stderr);
    // Its plan is done with, so only a first run still going could skip the
    // next.
    scenario.issue_is(8, "CLOSED");

    let second = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(second.code, Some(0), "stderr: {}", second.stderr);
    assert_eq!(second.stdout, format!("{}\n", scenario.issue_url(9)));
    assert_eq!(scenario.claude_calls().len(), 2);
    assert_nothing_left_behind(&scenario);
}

#[test]
fn once_the_first_architect_run_is_killed_a_new_one_runs_with_nothing_to_clean_up() {
    let scenario = scenario();
    scenario.agent_does_in_session(1, AGENT_WAITS_FOR_RELEASE);
    scenario.agent_does_in_session(2, AGENT_PUBLISHES_A_TICKET);
    let mut first = scenario.run_until(&["architect", "--plan-only"], &[], "started");
    first.kill();
    assert_eq!(scenario.entries("work"), [REPO, "widgets-architect"]);

    // The killed run could not stop its session, which is still going.
    let second = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(second.code, Some(0), "stderr: {}", second.stderr);
    assert_eq!(second.stdout, format!("{PLAN_URL}\n"));
    assert_eq!(scenario.claude_calls().len(), 2);
    assert_eq!(scenario.entries("work"), [REPO]);
    release(&scenario);
    assert_eq!(first.finish().code, None);
}

#[test]
fn an_architect_run_on_a_different_repository_at_the_same_time_is_not_skipped() {
    let scenario = scenario();
    scenario.agent_does_in_session(1, AGENT_WAITS_FOR_RELEASE);
    let gadgets_issue = "https://github.com/acme/gadgets/issues/3";
    scenario.agent_does_in_session(
        2,
        &ends_with(&format!(
            "Architecture review already filed: {gadgets_issue}"
        )),
    );
    let first = scenario.run_until(&["architect", "--plan-only"], &[], "started");
    // The first Architect run has read its repository, acme/widgets, by now:
    // the next one from the Launch directory is on another, with a fake
    // GitHub of its own.
    scenario.set_origin_url("https://github.com/acme/gadgets.git");
    let gadgets = scenario.path("gh-state-gadgets.json");
    fs::write(
        &gadgets,
        r#"{"repo": "acme/gadgets", "issues": {}, "prs": []}"#,
    )
    .unwrap();

    let second = scenario.run_with_env(
        &["architect", "--plan-only"],
        &[("FAKE_GH_STATE", gadgets.to_str().unwrap())],
    );

    assert_eq!(second.code, Some(0), "stderr: {}", second.stderr);
    assert_eq!(second.stdout, format!("{gadgets_issue}\n"));
    assert!(
        !second.stderr.contains("already running"),
        "stderr: {}",
        second.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 2);
    release(&scenario);
    first.finish();
}

#[test]
fn a_skipped_architect_run_sends_no_notification_asked_for_by_email_or_by_email_always() {
    for (config, args) in [
        (None, vec!["architect", "--email", "me@example.com"]),
        (Some(EMAIL_ALWAYS), vec!["architect"]),
    ] {
        let scenario = scenario();
        scenario.agent_does(AGENT_WAITS_FOR_RELEASE);
        let first = scenario.run_until(&["architect", "--plan-only"], &[], "started");
        if let Some(config) = config {
            scenario.user_config_is(config);
        }
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let second = run_with_resend(&scenario, &resend, &args);

        assert_skipped_as_already_running(&second);
        assert!(resend.requests().is_empty(), "{args:?}");
        release(&scenario);
        first.finish();
    }
}

#[test]
fn the_notifications_checks_come_before_the_skip_so_a_broken_setup_fails_a_pass_that_would_be_skipped()
 {
    let scenario = scenario();
    scenario.agent_does(AGENT_WAITS_FOR_RELEASE);
    let first = scenario.run_until(&["architect", "--plan-only"], &[], "started");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let second = run_with_resend(&scenario, &resend, &["architect", "--email"]);

    assert_eq!(second.code, Some(1), "stderr: {}", second.stderr);
    assert!(
        second.stderr.contains("no email address"),
        "stderr: {}",
        second.stderr
    );
    assert!(
        !second.stderr.contains("already running"),
        "stderr: {}",
        second.stderr
    );
    assert!(resend.requests().is_empty());
    release(&scenario);
    first.finish();
}

/// Make issue `number` an open Architect plan titled `title`, as thirdshift
/// left the plan of an earlier Architect run.
fn open_architect_plan(scenario: &Scenario, number: u32, title: &str) {
    scenario.issue_is(number, "OPEN");
    scenario.issue_titled(number, title);
    scenario.issue_labelled(number, &["ready-for-agent", ARCHITECT_PLAN]);
}

/// What a skipped Architect run says of the open Architect plan `number`,
/// titled `title`, on the scenario's repository.
fn still_open(scenario: &Scenario, number: u32, title: &str) -> String {
    format!(
        "Architect plan #{number} \"{title}\" is still open: pick it up with thirdshift {}",
        scenario.issue_url(number)
    )
}

#[test]
fn an_open_architect_plan_skips_the_architect_run_before_any_review_whatever_its_flags() {
    for args in [
        vec!["architect"],
        vec!["architect", "--plan-only"],
        vec!["architect", "the Spec run", "merge", "base", "main"],
    ] {
        let scenario = scenario();
        scenario.user_config_is("[launch]\npull = true\n");
        open_architect_plan(&scenario, 7, "Deepen the session module");
        scenario.origin_has_commit("main", "upstream.txt", "upstream\n", "Upstream work");
        let launch_main = scenario.launch_git(&["rev-parse", "refs/heads/main"]);
        let github = scenario.gh_state();

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(0), "{args:?}: stderr: {}", result.stderr);
        assert_eq!(result.stdout, format!("{}\n", scenario.issue_url(7)));
        assert_eq!(
            after_start(&result.stderr),
            format!(
                "thirdshift: {}\n",
                still_open(&scenario, 7, "Deepen the session module")
            )
        );
        assert!(scenario.claude_calls().is_empty(), "claude was started");
        assert_eq!(
            scenario.launch_git(&["rev-parse", "refs/heads/main"]),
            launch_main,
            "the skipped run did a launch pull"
        );
        assert_eq!(scenario.gh_state(), github);
        assert_nothing_left_behind(&scenario);
    }
}

#[test]
fn a_skipped_architect_run_names_each_open_architect_plan_and_prints_its_url() {
    let scenario = scenario();
    open_architect_plan(&scenario, 5, "Deepen the worktree module");
    open_architect_plan(&scenario, 7, "Deepen the session module");

    let result = scenario.run(&["architect"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        result.stdout,
        format!("{}\n{}\n", scenario.issue_url(7), scenario.issue_url(5))
    );
    assert_eq!(
        after_start(&result.stderr),
        format!(
            "thirdshift: {}; {}\n",
            still_open(&scenario, 7, "Deepen the session module"),
            still_open(&scenario, 5, "Deepen the worktree module")
        )
    );
    assert!(scenario.claude_calls().is_empty(), "claude was started");
}

#[test]
fn base_skips_an_architect_run_from_a_clone_on_another_branch_or_a_detached_head_as_from_any() {
    for detached in [false, true] {
        let scenario = develop_on_origin(detached);
        let launch_status = scenario.launch_git(&["status", "--porcelain"]);
        scenario.agent_does(&format!(
            "{AGENT_WAITS_FOR_RELEASE}{AGENT_PUBLISHES_A_TICKET}"
        ));
        let first = scenario.run_until(
            &["architect", "base", "develop", "--plan-only"],
            &[],
            "started",
        );

        let second = scenario.run(&["architect", "base", "develop"]);

        assert_skipped_as_already_running(&second);
        release(&scenario);
        let first = first.finish();
        assert_eq!(first.code, Some(0), "stderr: {}", first.stderr);

        // The first run's Architect plan is open now.
        let third = scenario.run(&["architect", "base", "develop"]);

        assert_eq!(third.code, Some(0), "stderr: {}", third.stderr);
        assert_eq!(
            third.stdout,
            format!("{PLAN_URL}\n"),
            "detached: {detached}"
        );
        assert!(
            third.stderr.contains("is still open: pick it up with"),
            "stderr: {}",
            third.stderr
        );
        assert_eq!(scenario.claude_calls().len(), 1, "a session was started");
        assert_eq!(
            scenario.launch_git(&["status", "--porcelain"]),
            launch_status
        );
    }
}

#[test]
fn an_architect_plan_that_is_closed_or_no_longer_labelled_lets_the_architect_run_go_ahead() {
    for lift_the_rule in [
        (|scenario| scenario.issue_is(7, "CLOSED")) as fn(&Scenario),
        |scenario| scenario.issue_labelled(7, &["ready-for-agent"]),
    ] {
        let scenario = scenario();
        open_architect_plan(&scenario, 7, "Deepen the session module");
        scenario.agent_does(AGENT_PUBLISHES_A_TICKET);
        lift_the_rule(&scenario);

        let result = scenario.run(&["architect", "--plan-only"]);

        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
        assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
        assert_eq!(scenario.claude_calls().len(), 1);
    }
}

#[test]
fn an_open_idea_issue_or_a_plan_left_needing_triage_does_not_skip_the_architect_run() {
    // #7 is the idea issue an earlier review filed, or the plan one that
    // failed left half-published: neither was ever labelled an Architect plan.
    let scenario = scenario();
    scenario.issue_labelled(7, &["needs-triage", "architecture"]);
    scenario.agent_does(AGENT_PUBLISHES_A_TICKET);

    let result = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PLAN_URL}\n"));
    assert_eq!(scenario.issue_labels(7), ["needs-triage", "architecture"]);
}

#[test]
fn a_review_that_failed_after_publishing_its_plan_does_not_skip_the_next_architect_run() {
    let scenario = scenario();
    scenario.agent_does_in_session(1, &publishes_a_ticket_then("exit 3"));
    scenario.agent_does_in_session(2, AGENT_PUBLISHES_A_TICKET);
    let first = scenario.run(&["architect", "--plan-only"]);
    assert_eq!(first.code, Some(1), "stderr: {}", first.stderr);

    let second = scenario.run(&["architect", "--plan-only"]);

    assert_eq!(second.code, Some(0), "stderr: {}", second.stderr);
    assert_eq!(second.stdout, format!("{}\n", scenario.issue_url(9)));
    assert_eq!(scenario.issue_labels(8), ["needs-triage"]);
}

#[test]
fn a_plan_whose_dispatched_run_failed_stays_open_and_the_next_architect_run_never_retries_it() {
    let scenario = single_ticket_plan();
    scenario.agent_does_for(8, "exit 3");
    let first = scenario.run(&["architect"]);
    assert_eq!(first.code, Some(1), "stderr: {}", first.stderr);
    // The run left nothing on origin, so its Claim on the plan is released.
    assert_eq!(
        scenario.issue_labels(8),
        [ARCHITECT_PLAN, "ready-for-agent"]
    );
    let sessions = scenario.claude_calls().len();

    let second = scenario.run(&["architect"]);

    assert_eq!(second.code, Some(0), "stderr: {}", second.stderr);
    assert_eq!(second.stdout, format!("{PLAN_URL}\n"));
    assert_eq!(
        after_start(&second.stderr),
        format!(
            "thirdshift: {}\n",
            still_open(&scenario, 8, "Deepen the session module")
        )
    );
    assert_eq!(scenario.claude_calls().len(), sessions, "a session started");
}

#[test]
fn an_architect_run_skipped_for_open_plans_sends_no_notification_and_still_prints_each_url() {
    let scenario = scenario();
    open_architect_plan(&scenario, 5, "Deepen the worktree module");
    open_architect_plan(&scenario, 7, "Deepen the session module");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["architect", "--plan-only", "--email", "me@example.com"],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        result.stdout,
        format!("{}\n{}\n", scenario.issue_url(7), scenario.issue_url(5))
    );
    assert!(resend.requests().is_empty());
}
