//! Killed background work: a session that ends its turn while a background
//! task it started is still running is killed with that task. thirdshift
//! resumes such a session once, and fails with the reason if it happens again.

mod support;

use support::Scenario;

/// The agent starts a background test run, then ends its turn without waiting
/// for it, so the task is killed after the session's last `result`.
const AGENT_LEAVES_TESTS_RUNNING: &str = r#"
echo '{"type": "system", "subtype": "task_started", "task_id": "b1", "description": "./mvnw test -Dtest=GamesPageTest"}'
echo '{"type": "system", "subtype": "task_updated", "task_id": "b1", "patch": {"status": "killed"}}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#;

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
        &format!("echo wip > feature.txt\n{AGENT_LEAVES_TESTS_RUNNING}"),
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
    let logs = scenario.entries("home/.thirdshift/logs");
    assert_eq!(logs.len(), 2, "logs: {logs:?}");
    assert!(logs.iter().any(|log| log.ends_with("-implement.jsonl")));
    assert!(
        logs.iter()
            .any(|log| log.ends_with("-implement-resume.jsonl"))
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_resumed_session_whose_background_work_is_killed_again_is_a_failed_run_that_says_so() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "echo wip >> feature.txt\n{AGENT_LEAVES_TESTS_RUNNING}"
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    let reason = "implement session ended while waiting on a background task \
                  (./mvnw test -Dtest=GamesPageTest), which was killed";
    assert!(result.stderr.contains(reason), "stderr: {}", result.stderr);
    assert!(
        !result.stderr.contains("no PR found"),
        "stderr: {}",
        result.stderr
    );
    // The Failed run path still pushes the uncommitted work.
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        format!("thirdshift: failed run ({reason})")
    );
    assert_eq!(
        scenario.origin_file("issue-7", "feature.txt"),
        Some("wip\nwip\n".to_string())
    );
    assert!(
        result.stderr.contains("-implement-resume.jsonl"),
        "stderr: {}",
        result.stderr
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_repair_whose_background_work_was_killed_is_resumed_too() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_COMMITS_AND_OPENS_PR}gh fake checks \"$(git rev-parse HEAD)\" \
             '[{{\"name\": \"test\", \"conclusion\": \"failure\"}}]'\n"
        ),
    );
    scenario.agent_does_in_session(2, AGENT_LEAVES_TESTS_RUNNING);
    scenario.agent_does_in_session(
        3,
        "echo fix > fix.txt\ngit add fix.txt\ngit commit -q -m Fix\n\
         gh fake checks \"$(git rev-parse HEAD)\" '[{\"name\": \"test\", \"conclusion\": \"success\"}]'\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 3);
    assert!(
        result
            .stderr
            .contains("thirdshift: repair-1-resume: session started"),
        "stderr: {}",
        result.stderr
    );
}
