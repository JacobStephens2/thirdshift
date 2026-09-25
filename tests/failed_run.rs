//! The Failed run path: whatever goes wrong after the worktree exists, the
//! agent's work is committed and pushed, an open PR goes back to draft, the
//! log path is reported, and cleanup still happens.

mod support;

use support::{RunResult, Scenario};

/// The agent commits some work, leaves more uncommitted, and exits 3.
const AGENT_LEAVES_WORK_AND_EXITS_3: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
echo "half done" > wip.txt
exit 3
"#;

#[test]
fn a_failing_session_pushes_a_failure_commit_with_the_uncommitted_work() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_LEAVES_WORK_AND_EXITS_3);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_failed(&scenario, &result, "");
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "thirdshift: failed run (claude exited 3)".to_string(),
            "Add feature".to_string(),
            "Initial commit".to_string(),
        ])
    );
    assert_eq!(
        scenario.origin_file("issue-7", "wip.txt"),
        Some("half done\n".to_string())
    );
}

/// What every Failed run shares: a non-zero exit, `stdout` (empty unless a PR
/// exists), the session log path on stderr, and nothing left behind.
fn assert_failed(scenario: &Scenario, result: &RunResult, stdout: &str) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, stdout);
    let logs = scenario.entries("home/.thirdshift/logs");
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    let log = scenario.path("home/.thirdshift/logs").join(&logs[0]);
    assert!(
        result.stderr.contains(log.to_str().unwrap()),
        "stderr: {}",
        result.stderr
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn the_failure_commit_says_when_and_where_it_was_made() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_LEAVES_WORK_AND_EXITS_3);

    scenario.run(&[&scenario.issue_url(7)]);

    let body = scenario.origin_git(&["log", "-1", "--format=%b", "issue-7"]);
    let (timestamp, rest) = body.trim().split_once(", host ").expect(&body);
    assert!(
        chrono::DateTime::parse_from_rfc3339(timestamp).is_ok(),
        "body: {body}"
    );
    assert!(
        rest.ends_with(". Uncommitted work at the time of failure is included in this commit."),
        "body: {body}"
    );
}

#[test]
fn no_pr_after_the_session_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(
        "echo feature > feature.txt\ngit add feature.txt\ngit commit -q -m 'Add feature'\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_failed(&scenario, &result, "");
    assert!(
        result.stderr.contains("no PR found"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "thirdshift: failed run (no PR found)"
    );
}

#[test]
fn a_pr_against_the_wrong_base_is_sent_back_to_draft() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("develop", "main", &[]);
    scenario.agent_does(
        "echo feature > feature.txt\ngit add feature.txt\ngit commit -q -m 'Add feature'\n\
         gh pr create --base develop --head issue-7 --title 'Add feature' --body 'Closes #7'\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_failed(
        &scenario,
        &result,
        "https://github.com/acme/widgets/pull/1\n",
    );
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "thirdshift: failed run (PR targets develop, not main)"
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
}

#[test]
fn no_changes_against_the_base_branch_means_no_failure_commit_and_no_push() {
    let scenario = Scenario::new();
    scenario.agent_does("exit 1");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_failed(&scenario, &result, "");
    assert_eq!(scenario.origin_log("issue-7"), None);
}

#[test]
fn an_unfinished_merge_is_aborted_before_the_failure_commit() {
    let scenario = Scenario::new();
    scenario.agent_does(
        "git checkout -q -b other\n\
         echo theirs > README.md\ngit commit -q -am 'Theirs'\n\
         git checkout -q issue-7\n\
         echo ours > README.md\ngit commit -q -am 'Ours'\n\
         git merge -q other || true\n\
         exit 1\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_failed(&scenario, &result, "");
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "thirdshift: failed run (claude exited 1)".to_string(),
            "Ours".to_string(),
            "Initial commit".to_string(),
        ])
    );
    assert_eq!(
        scenario.origin_file("issue-7", "README.md"),
        Some("ours\n".to_string())
    );
}

#[test]
fn sigint_while_the_session_runs_stops_it_and_fails_the_run() {
    assert_interrupt_fails_the_run("INT");
}

#[test]
fn sigterm_while_the_session_runs_stops_it_and_fails_the_run() {
    assert_interrupt_fails_the_run("TERM");
}

fn assert_interrupt_fails_the_run(signal: &str) {
    let scenario = Scenario::new();
    let started = scenario.path("agent-started");
    scenario.agent_does(&format!(
        "echo 'half done' > wip.txt\ntouch {}\nsleep 30\n",
        started.display()
    ));

    let began = std::time::Instant::now();
    let result = scenario.run_and_signal(&[&scenario.issue_url(7)], "agent-started", signal);

    assert!(
        began.elapsed() < std::time::Duration::from_secs(20),
        "the session was not stopped"
    );
    assert_failed(&scenario, &result, "");
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "thirdshift: failed run (interrupted)"
    );
    assert_eq!(
        scenario.origin_file("issue-7", "wip.txt"),
        Some("half done\n".to_string())
    );
}
