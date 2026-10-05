//! Killed background work: a session that ends its turn while a background
//! task it started is still running is killed with that task. thirdshift
//! resumes such a session once. If the Resume ends the same way, the task may
//! have been abandoned, not awaited: the Run carries on, and names the killed
//! work in its cause if it then fails.

mod support;

use support::{Scenario, leaves_running};

/// What the background test run the agent leaves running, with
/// [`leaves_running`], is described as.
const TESTS: &str = "./mvnw test -Dtest=GamesPageTest";

const AGENT_COMMITS_AND_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn a_session_whose_background_work_was_killed_is_resumed_once_and_the_run_goes_on() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!("echo wip > feature.txt\n{}", leaves_running(TESTS)),
    );
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    let argv: Vec<&str> = calls[1]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--resume", "fake-session-1"]),
        "argv: {argv:?}"
    );
    assert!(
        argv.windows(2)
            .any(|pair| pair == ["--permission-mode", "auto"]),
        "argv: {argv:?}"
    );
    assert_eq!(calls[1]["cwd"], calls[0]["cwd"]);
    scenario.assert_every_session_found_the_factory_skills();
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("was killed when your turn ended") && prompt.contains("in the foreground"),
        "prompt: {prompt}"
    );
    for expected in [
        "thirdshift: implement: session ended",
        "thirdshift: implement-resume: session started",
        "thirdshift: implement-resume: session ended",
    ] {
        assert!(
            result.stderr.lines().any(|line| line.starts_with(expected)),
            "missing {expected:?} in stderr: {}",
            result.stderr
        );
    }
    let logs = scenario.entries("home/.thirdshift/logs/acme/widgets/sessions");
    assert_eq!(logs.len(), 2, "logs: {logs:?}");
    assert!(logs.iter().any(|log| log.ends_with("-implement.jsonl")));
    assert!(
        logs.iter()
            .any(|log| log.ends_with("-implement-resume.jsonl"))
    );
    scenario.assert_cleaned_up("issue-7");
}

/// How a session that ended with the [`TESTS`] still running is said to have
/// ended.
const LEFT_TESTS_RUNNING: &str = "ended with a background task still running \
                                  (./mvnw test -Dtest=GamesPageTest), which was killed";

#[test]
fn a_merge_refused_after_killed_background_work_still_leaves_the_pr_ready_for_review() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_COMMITS_AND_OPENS_PR}{}\
             gh fake refuse-merges 1 'Merge commits are not allowed on this repository.'\n",
            leaves_running(TESTS)
        ),
    );
    scenario.agent_does_in_session(2, &leaves_running(TESTS));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    for part in [
        &format!("implement session {LEFT_TESTS_RUNNING}, and a later step failed: "),
        "Merge commits are not allowed on this repository.",
    ] {
        assert!(result.stderr.contains(part), "stderr: {}", result.stderr);
    }
    let pr = &scenario.gh_state()["prs"][0];
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], false);
    // No failure commit: the PR stays on the head whose CI was watched.
    assert_eq!(scenario.origin_log("issue-7").unwrap()[0], "Add feature");
}

#[test]
fn a_session_that_ends_in_an_error_with_killed_background_work_is_not_resumed_or_warned_about() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{}exit 3\n", leaves_running(TESTS)));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(
        result.stderr.contains("thirdshift: claude exited 3\n"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("background"),
        "stderr: {}",
        result.stderr
    );
}
