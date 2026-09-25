//! Progress on stderr: thirdshift condenses the session's stream-json output
//! into short stderr lines and prints its own steps, while stdout stays
//! reserved for the PR URL.

mod support;

use serde_json::json;
use support::Scenario;

/// The agent commits its work and opens a PR, but leaves the pushing to
/// thirdshift.
const AGENT_COMMITS_AND_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

/// A bash line that makes the fake session emit `event` on its stream.
fn emits(event: serde_json::Value) -> String {
    format!("echo '{event}'\n")
}

fn tool_use(name: &str, input: serde_json::Value) -> serde_json::Value {
    json!({
        "type": "assistant",
        "message": { "content": [{ "type": "tool_use", "name": name, "input": input }] }
    })
}

fn result(turns: u64, cost: f64) -> serde_json::Value {
    json!({ "type": "result", "subtype": "success", "num_turns": turns, "total_cost_usd": cost })
}

fn stderr_lines(stderr: &str) -> Vec<&str> {
    stderr.lines().collect()
}

#[test]
fn prints_one_line_per_notable_session_event() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{}{}{}{}{AGENT_COMMITS_AND_OPENS_PR}",
        emits(tool_use(
            "Skill",
            json!({ "skill": "thirdshift:implement" })
        )),
        emits(tool_use(
            "Read",
            json!({ "file_path": scenario.path("work/widgets-issue-7/src/lib.rs") })
        )),
        emits(tool_use(
            "Bash",
            json!({ "command": "git commit -m 'Add feature'" })
        )),
        emits(tool_use(
            "Bash",
            json!({ "command": "git push origin issue-7" })
        )),
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let lines = stderr_lines(&result.stderr);
    for expected in [
        "thirdshift: implement: session started",
        "thirdshift: implement: skill thirdshift:implement",
        "thirdshift: implement: Read src/lib.rs",
        "thirdshift: implement: commit",
        "thirdshift: implement: push",
    ] {
        assert!(
            lines.contains(&expected),
            "missing {expected:?} in {lines:?}"
        );
    }
}

#[test]
fn reports_the_last_result_once_the_session_exits_even_after_several() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{}{}{}{AGENT_COMMITS_AND_OPENS_PR}",
        emits(result(10, 0.5)),
        emits(json!({ "type": "system", "subtype": "init", "cwd": "/elsewhere" })),
        emits(result(34, 1.82)),
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let lines = stderr_lines(&result.stderr);
    let started = lines
        .iter()
        .filter(|l| l.contains("session started"))
        .count();
    assert_eq!(started, 1, "stderr: {lines:?}");
    let ended: Vec<_> = lines
        .iter()
        .filter(|l| l.contains("session ended"))
        .collect();
    assert_eq!(ended.len(), 1, "stderr: {lines:?}");
    // The fake's own closing result event carries no totals, so the last
    // result with totals wins.
    assert!(ended[0].ends_with("34 turns, $1.82"), "{}", ended[0]);
}

#[test]
fn prints_a_line_for_each_of_its_own_steps() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let steps = [
        "thirdshift: creating worktree ",
        "thirdshift: implement: session started",
        "thirdshift: implement: session ended",
        "thirdshift: pushing issue-7",
        "thirdshift: checking the PR",
        "thirdshift: cleaning up",
    ];
    let lines = stderr_lines(&result.stderr);
    let mut positions = steps.iter().map(|step| {
        lines
            .iter()
            .position(|line| line.starts_with(step))
            .unwrap_or_else(|| panic!("missing {step:?} in {lines:?}"))
    });
    let first = positions.next().unwrap();
    positions.fold(first, |previous, position| {
        assert!(position > previous, "steps out of order: {lines:?}");
        position
    });
}

#[test]
fn ends_a_successful_run_by_naming_the_pr() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        stderr_lines(&result.stderr).last(),
        Some(&"thirdshift: PR https://github.com/acme/widgets/pull/1 is ready for review")
    );
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
}

#[test]
fn skips_unknown_and_malformed_stream_events_and_keeps_stdout_to_the_pr_url() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "echo 'not json at all'\necho '[1, 2]'\necho '{{\"type\": \"assistant\"'\n{}{}{AGENT_COMMITS_AND_OPENS_PR}",
        emits(json!({ "type": "stream_event", "event": {} })),
        emits(json!({ "type": "assistant", "message": { "content": "oops" } })),
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert!(
        result
            .stderr
            .lines()
            .all(|line| line.starts_with("thirdshift: ")),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn still_logs_the_whole_stream() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "echo 'not json at all'\n{AGENT_COMMITS_AND_OPENS_PR}"
    ));

    scenario.run(&[&scenario.issue_url(7)]);

    let logs = scenario.entries("home/.thirdshift/logs");
    let log =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs").join(&logs[0])).unwrap();
    assert!(log.contains("not json at all\n"), "log: {log}");
    assert!(log.contains(r#""type": "result""#), "log: {log}");
}
