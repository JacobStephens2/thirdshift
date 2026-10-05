//! A Command's sessions on Codex, with `harness codex`: each runs `codex exec
//! --json` unsandboxed in the Run's worktree, with stdin set to null, on the
//! Model and Effort as Codex names them, reading `CLAUDE.md` where there is
//! no `AGENTS.md`, with the Factory skills linked into the worktree's
//! `.agents/skills/` and its prompt loading its skill as
//! `$thirdshift-<skill>`. Codex's stream gives the progress lines, the
//! Session log, the session id a Resume continues and the final message, and
//! a failed turn fails the Run with Codex's error. The Model and Effort are
//! checked against `codex debug models` before any work.

mod support;

use std::fs;

use serde_json::Value;
use support::{RunResult, Scenario};

/// The agent commits its work and opens a PR that closes issue #7.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

/// The scenario's repository's logs, relative to the scenario.
const LOGS: &str = "home/.thirdshift/logs/acme/widgets";

/// Bash that has the agent run `command` as Codex streams it, wrapped in its
/// shell, still running when its turn ends.
fn leaves_running(command: &str) -> String {
    format!(
        r#"echo '{{"type": "item.started", "item": {{"id": "item_9", "type": "command_execution", "command": "/bin/bash -lc '\''{command}'\''", "status": "in_progress"}}}}'
"#
    )
}

/// Bash that has the agent run `command` to completion, as Codex streams it.
fn runs(command: &str) -> String {
    format!(
        r#"echo '{{"type": "item.started", "item": {{"id": "item_1", "type": "command_execution", "command": "{command}", "status": "in_progress"}}}}'
echo '{{"type": "item.completed", "item": {{"id": "item_1", "type": "command_execution", "command": "{command}", "status": "completed", "exit_code": 0}}}}'
"#
    )
}

/// Bash that sets the check runs on the worktree's HEAD to `checks`.
fn checks_on_head(checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse HEAD)\" '{checks}'\n")
}

const RED: &str =
    r#"[{"name": "test", "conclusion": "failure", "url": "https://ci.example/test"}]"#;
const GREEN: &str =
    r#"[{"name": "test", "conclusion": "success", "url": "https://ci.example/test"}]"#;

fn argv(call: &Value) -> Vec<String> {
    call["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_string())
        .collect()
}

/// The arguments a Codex session on `model` and `effort` starts with, before
/// any `resume` and its prompt.
fn codex_args(model: &str, effort: &str) -> Vec<String> {
    [
        "exec",
        "--json",
        "--dangerously-bypass-approvals-and-sandbox",
        "-m",
        model,
        "-c",
        &format!("model_reasoning_effort=\"{effort}\""),
        "-c",
        "project_doc_fallback_filenames=[\"CLAUDE.md\"]",
    ]
    .map(String::from)
    .to_vec()
}

/// Assert every session ran on `codex exec` with the bypass flag, `model`
/// and `effort`, the `CLAUDE.md` fallback and stdin set to null, its prompt
/// last, and found every Factory skill in `.agents/skills/` with none of
/// them in `git status`; that its prompt's first line loads its skill with
/// Codex's sigil, if it loads one; and that none ran on `claude`.
fn assert_sessions_on_codex(scenario: &Scenario, model: &str, effort: &str) {
    let calls = scenario.codex_calls();
    assert!(!calls.is_empty(), "no Codex session ran");
    assert!(
        scenario.claude_calls().is_empty(),
        "a session ran on claude"
    );
    for call in &calls {
        let args = argv(call);
        let prompt = call["prompt"].as_str().unwrap();
        assert_eq!(args.last().map(String::as_str), Some(prompt));
        assert_eq!(args[..9], codex_args(model, effort), "{args:?}");
        assert_eq!(call["stdin_null"], true, "{args:?}");
        assert_eq!(call["git_status"], "", "{args:?}");
        assert!(!prompt.starts_with("/thirdshift-"), "{prompt}");
    }
    scenario.assert_every_codex_session_found_the_factory_skills();
}

/// The first line of each Codex session's prompt.
fn first_lines(scenario: &Scenario) -> Vec<String> {
    scenario
        .codex_calls()
        .iter()
        .map(|call| {
            let prompt = call["prompt"].as_str().unwrap();
            prompt.lines().next().unwrap().to_string()
        })
        .collect()
}

#[test]
fn a_runs_implement_session_and_its_repair_run_on_codex_with_the_model_and_effort_as_codex_names_them()
 {
    let scenario = Scenario::new();
    scenario.agent_does_for_in_session(
        7,
        1,
        &format!(
            "{}{AGENT_OPENS_PR}{}",
            runs("/bin/bash -lc 'cargo test'"),
            checks_on_head(RED)
        ),
    );
    scenario.agent_does_for_in_session(
        7,
        2,
        &format!(
            "echo fix > fix.txt\ngit add fix.txt\ngit commit -q -m Fix\n{}",
            checks_on_head(GREEN)
        ),
    );
    let url = scenario.issue_url(7);

    let result = scenario.run(&[
        "harness",
        "codex",
        "model",
        "GPT-6.1-Sol",
        &url,
        "effort",
        "Max",
    ]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_sessions_on_codex(&scenario, "gpt-6.1-sol", "max");
    assert_eq!(scenario.codex_calls().len(), 2);
    assert_eq!(
        first_lines(&scenario)[0],
        format!("$thirdshift-implement {url}")
    );
    for line in [
        "thirdshift: checking the Model and Effort against codex debug models\n",
        "thirdshift: sessions run on codex · gpt-6.1-sol · max\n",
        "thirdshift: implement: session started\n",
        "thirdshift: implement: $ cargo test\n",
        ": 1200 input tokens (200 cached), 300 output tokens\n",
    ] {
        assert!(
            result.stderr.contains(line),
            "{line:?} in {}",
            result.stderr
        );
    }
    let body = scenario.gh_state()["prs"][0]["body"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        body.ends_with("Built with codex · gpt-6.1-sol · max <!-- thirdshift:built-with -->\n"),
        "{body}"
    );
    let logs = scenario.entries(&format!("{LOGS}/sessions"));
    let implement = logs
        .iter()
        .find(|log| log.ends_with("-implement.jsonl"))
        .unwrap_or_else(|| panic!("no implement session log: {logs:?}"));
    let log = fs::read_to_string(scenario.path(&format!("{LOGS}/sessions/{implement}"))).unwrap();
    assert!(
        log.starts_with(r#"{"type": "thread.started", "thread_id": "fake-thread-1"}"#),
        "{log}"
    );
    assert!(log.contains(r#""type": "turn.completed""#), "{log}");
}

#[test]
fn a_session_left_with_a_command_running_is_resumed_on_its_thread_with_every_flag_again() {
    let scenario = Scenario::new();
    scenario.agent_does_for_in_session(
        7,
        1,
        &format!("{AGENT_OPENS_PR}{}", leaves_running("sleep 188")),
    );
    let url = scenario.issue_url(7);

    let result = scenario.run(&[
        "harness",
        "codex",
        "model",
        "gpt-6-luna",
        "effort",
        "high",
        &url,
    ]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_sessions_on_codex(&scenario, "gpt-6-luna", "high");
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 2);
    let resume = argv(&calls[1]);
    assert_eq!(resume[9..11], ["resume", "fake-thread-1"], "{resume:?}");
    assert!(
        calls[1]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("Your background work (sleep 188) was killed"),
        "{resume:?}"
    );
    assert!(
        result.stderr.contains(
            "implement: background work was killed as the session ended; resuming it once\n"
        ),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_failed_turn_fails_the_run_with_codexs_error_in_the_cause() {
    for (script, cause) in [
        (
            "echo \"The 'gpt-6.1-sol' model is not supported.\" > \"$FAKE_CODEX_ERROR\"\nexit 1",
            "codex exited 1: The 'gpt-6.1-sol' model is not supported.",
        ),
        (
            r#"echo '{"type": "turn.failed", "error": {"message": "stream disconnected"}}'"#,
            "codex's turn failed: stream disconnected",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_for(7, script);

        let result = scenario.run(&["harness", "codex", &scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
        assert!(
            result.stderr.contains(cause),
            "{cause:?} in {}",
            result.stderr
        );
        assert_eq!(scenario.codex_calls().len(), 1);
    }
}

#[test]
fn a_retried_error_alone_does_not_fail_the_session() {
    let scenario = Scenario::new();
    scenario.agent_does_for(
        7,
        &format!(
            "echo '{{\"type\": \"error\", \"message\": \"Reconnecting... 1/5\"}}'\n{AGENT_OPENS_PR}"
        ),
    );

    let result = scenario.run(&["harness", "codex", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        !result.stderr.contains("checking the Model"),
        "a Model was checked with none named: {}",
        result.stderr
    );
}

/// Assert the Run failed with `error` before any work: no Claim, no
/// worktree, no session and no Command log.
fn assert_failed_before_any_work(scenario: &Scenario, result: &RunResult, error: &str) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(error),
        "expected {error:?} in stderr: {}",
        result.stderr
    );
    assert!(scenario.codex_calls().is_empty(), "a session ran");
    assert_eq!(scenario.entries("work"), vec!["widgets"]);
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert!(scenario.issue_labels(7).is_empty(), "the Claim was made");
    assert!(
        !scenario.path(&format!("{LOGS}/commands")).exists(),
        "a Command log was kept"
    );
}

#[test]
fn a_model_or_effort_codex_lacks_fails_before_any_work_naming_the_valid_choices() {
    for (model, effort, error) in [
        (
            "gpt-7",
            "max",
            "thirdshift: the Model gpt-7 is not in Codex's catalog: choose one of gpt-6.1-sol, \
             gpt-6-luna, gpt-5.5\n",
        ),
        (
            "GPT-5.5",
            "Max",
            "thirdshift: the Effort Max is not one the Codex Model gpt-5.5 supports: choose one \
             of low, medium, high, xhigh\n",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_for(7, AGENT_OPENS_PR);

        let result = scenario.run(&[
            &scenario.issue_url(7),
            "harness",
            "codex",
            "model",
            model,
            "effort",
            effort,
        ]);

        assert_failed_before_any_work(&scenario, &result, error);
    }
}

/// A script in which the agent for issue `issue` commits its work and opens
/// its PR into `base`.
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

#[test]
fn an_architecture_review_and_the_spec_run_it_dispatches_run_every_session_on_codex() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent", "architecture"]);
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
    scenario.agent_does_for(10, &agent_opens_pr(10, "issue-8"));

    let result = scenario.run(&["architect", "harness", "codex", "model", "GPT-6-Luna"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let gh = scenario.gh_state();
    let spec_pr = gh["prs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pr| pr["head"] == "issue-8")
        .unwrap_or_else(|| panic!("no Spec PR: {gh}"));
    assert_eq!(spec_pr["isDraft"], false);
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 4, "{calls:?}");
    for call in &calls {
        let args = argv(call);
        assert_eq!(
            args[..7],
            [
                "exec",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "-m",
                "gpt-6-luna",
                "-c",
                "project_doc_fallback_filenames=[\"CLAUDE.md\"]",
            ],
            "{args:?}"
        );
        assert_eq!(call["stdin_null"], true, "{args:?}");
        assert_eq!(call["git_status"], "", "{args:?}");
    }
    assert!(scenario.claude_calls().is_empty());
    scenario.assert_every_codex_session_found_the_factory_skills();
    let mut first_lines = first_lines(&scenario);
    assert_eq!(
        first_lines.remove(0),
        "$thirdshift-improve-codebase-architecture"
    );
    assert!(
        first_lines[2].starts_with("$thirdshift-code-review main, with the Spec "),
        "{first_lines:?}"
    );
    first_lines.pop();
    first_lines.sort();
    assert_eq!(
        first_lines,
        [
            "$thirdshift-implement https://github.com/acme/widgets/issues/10",
            "$thirdshift-implement https://github.com/acme/widgets/issues/9",
        ]
    );
}
