//! A Command's sessions on Codex, with `harness codex`: each runs `codex exec
//! --json` unsandboxed in the Run's worktree, with stdin set to null, on the
//! Model and Effort as Codex names them, reading `CLAUDE.md` where there is
//! no `AGENTS.md`, with the Factory skills linked into the worktree's
//! `.agents/skills/` and its prompt loading its skill as
//! `$thirdshift-<skill>`. Codex's stream gives the progress lines, the
//! Session log, the session id a Resume continues and the final message, and
//! a failed turn fails the Run with Codex's error. A session that leaves a
//! command or sub-agent call running gets one Resume on its thread, as on
//! Claude, and an interrupt sends a session SIGINT before SIGTERM. The Model
//! and Effort are checked against `codex debug models` before any work.

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
/// Codex's sigil, if it loads one, and it names no tool only Claude has; and
/// that none ran on `claude`.
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
        assert!(!prompt.contains("Bash"), "{prompt}");
        assert_eq!(
            prompt.matches("`TaskStop`").count(),
            prompt.matches("`TaskStop` tool, if you have it").count(),
            "{prompt}"
        );
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

/// Code review sessions keep their reviewers independent without taking
/// Security's thread-cap setting.
fn assert_fresh_reviewers(call: &Value) {
    let prompt = call["prompt"].as_str().unwrap();
    assert!(prompt.contains("fresh sub-agents"), "{prompt}");
    assert!(prompt.contains("fork_turns: \"none\""), "{prompt}");
    assert!(
        prompt.contains("only its own task and necessary evidence"),
        "{prompt}"
    );
    assert!(
        !argv(call)
            .iter()
            .any(|arg| arg.contains("max_concurrent_threads")),
        "{call}"
    );
}

#[test]
fn implement_sessions_and_their_resumes_request_fresh_reviewers() {
    // Fresh, Continuation without a PR, and Continuation with an open PR.
    for continuation in [None, Some(false), Some(true)] {
        let scenario = Scenario::new();
        let mut script = AGENT_OPENS_PR.to_string();
        if let Some(has_pr) = continuation {
            scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
            if has_pr {
                scenario.github_has_pr("issue-7", "main", "OPEN");
                script = "echo more > more.txt\ngit add more.txt\ngit commit -q -m Continue\n"
                    .to_string();
            }
        }
        scenario.agent_does_in_session(1, &format!("{script}{}", leaves_running("cargo test")));
        let result = scenario.run(&["harness", "codex", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let calls = scenario.codex_calls();
        assert_eq!(calls.len(), 2);
        for call in &calls {
            assert_fresh_reviewers(call);
        }
        assert_eq!(argv(&calls[1])[5..7], ["resume", "fake-thread-1"]);
    }
}

#[test]
fn spec_reviews_and_their_resumes_request_fresh_reviewers() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[])]);
    scenario.issue_is(21, "CLOSED");
    scenario.origin_has_branch("issue-20", "main", &["Ticket 21"]);
    scenario.github_has_pr("issue-20", "main", "OPEN");
    scenario.agent_does_in_session(1, &leaves_running("cargo test"));

    let result = scenario.run(&["harness", "codex", &scenario.issue_url(20)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("$thirdshift-code-review main, with the Spec ")
    );
    for call in &calls {
        assert_fresh_reviewers(call);
    }
    assert_eq!(argv(&calls[1])[5..7], ["resume", "fake-thread-1"]);
}

#[test]
fn foreign_commit_reviews_and_their_resumes_request_fresh_reviewers() {
    let scenario = Scenario::new();
    // Publish a Foreign commit when CI is first read, after the Run has
    // captured its own head. The new commit needs a review Repair.
    let foreign_commit = r#"
other="$(mktemp -d)"
git clone -q -b issue-7 https://github.com/acme/widgets.git "$other"
echo late > "$other/late.txt"
git -C "$other" add late.txt
git -C "$other" commit -q -m "Foreign commit"
git -C "$other" push -q origin issue-7
rm -rf "$other"
"#;
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}gh fake on-ci-read 1 '{}'\n",
            foreign_commit.replace('\'', r"'\''")
        ),
    );
    scenario.agent_does_in_session(2, &leaves_running("cargo test"));

    let result = scenario.run(&["merge", "harness", "codex", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 3);
    assert!(
        calls[1]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("$thirdshift-code-review ")
    );
    for call in &calls[1..] {
        assert_fresh_reviewers(call);
    }
    assert_eq!(argv(&calls[2])[5..7], ["resume", "fake-thread-2"]);
    assert_eq!(scenario.origin_file("main", "late.txt").unwrap(), "late\n");
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
    let calls = scenario.codex_calls();
    assert_fresh_reviewers(&calls[0]);
    assert!(!calls[1]["prompt"].as_str().unwrap().contains("fork_turns"));
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
    let logs = scenario.log_files(&format!("{LOGS}/commands/issue"), "jsonl");
    let implement = logs
        .iter()
        .find(|log| log.ends_with("-implement.jsonl"))
        .unwrap_or_else(|| panic!("no implement session log: {logs:?}"));
    let log =
        fs::read_to_string(scenario.path(&format!("{LOGS}/commands/issue/{implement}"))).unwrap();
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

/// Bash that has the agent leave a sub-agent call, spawning a sub-agent
/// given `prompt`, running when its turn ends, as Codex streams it.
fn leaves_a_sub_agent_running(prompt: &str) -> String {
    format!(
        r#"echo '{{"type": "item.started", "item": {{"id": "item_8", "type": "collab_tool_call", "tool": "spawn_agent", "prompt": "{prompt}", "status": "in_progress"}}}}'
"#
    )
}

#[test]
fn a_session_left_with_a_sub_agent_running_is_resumed_too() {
    let scenario = Scenario::new();
    scenario.agent_does_for_in_session(
        7,
        1,
        &format!(
            "{AGENT_OPENS_PR}{}",
            leaves_a_sub_agent_running("Review the diff")
        ),
    );

    let result = scenario.run(&["harness", "codex", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(argv(&calls[1])[5..7], ["resume", "fake-thread-1"]);
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with("Your background work (agent spawn_agent: Review the diff) was killed"),
        "{prompt}"
    );
}

#[test]
fn a_resume_that_ends_the_same_way_gets_no_second_resume_and_a_later_failure_names_the_work() {
    let scenario = Scenario::new();
    // Every session, the Resume included, leaves its tests running and opens
    // no PR.
    scenario.agent_does(&format!(
        "echo wip >> feature.txt\n{}",
        leaves_running("cargo test")
    ));

    let result = scenario.run(&["harness", "codex", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 2, "one Resume, never two");
    assert_eq!(argv(&calls[1])[5..7], ["resume", "fake-thread-1"]);
    let ending = "ended with a background task still running (cargo test), which was killed";
    for line in [
        format!("thirdshift: implement: the Resume {ending}; carrying on"),
        format!("thirdshift: implement session {ending}, and a later step failed: no PR found"),
    ] {
        assert!(
            result.stderr.lines().any(|said| said.starts_with(&line)),
            "{line:?} in {}",
            result.stderr
        );
    }
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        format!(
            "thirdshift: failed run (implement session {ending}, and a later step failed: no PR found)"
        )
    );
}

#[test]
fn resumes_count_against_no_repair_cap() {
    let scenario = Scenario::new();
    scenario.agent_does_for_in_session(7, 1, &format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    // Repairs 1 to 4 each leave CI red and their tests running, so each gets
    // a Resume, which does nothing; Repair 5, the cap's last, turns CI green.
    for repair in 1..=4 {
        scenario.agent_does_for_in_session(
            7,
            repair + 1,
            &format!(
                "echo {repair} > fix.txt\ngit add fix.txt\ngit commit -q -m Fix\n{}{}",
                checks_on_head(RED),
                leaves_running("cargo test")
            ),
        );
    }
    scenario.agent_does_for_in_session(
        7,
        6,
        &format!(
            "echo 5 > fix.txt\ngit add fix.txt\ngit commit -q -m Fix\n{}",
            checks_on_head(GREEN)
        ),
    );

    let result = scenario.run(&["harness", "codex", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.codex_calls().len(), 10);
    for line in [
        "thirdshift: repair-4-resume: session started\n",
        "starting Repair 5 of 5\n",
    ] {
        assert!(
            result.stderr.contains(line),
            "{line:?} in {}",
            result.stderr
        );
    }
}

/// Bash that has the agent record each SIGINT and SIGTERM its session gets
/// to `signals` in the scenario, ending on SIGTERM and, if `ends_on_sigint`,
/// on SIGINT too, touch `agent-started` there and then keep working until a
/// signal ends it.
fn records_signals(scenario: &Scenario, ends_on_sigint: bool) -> String {
    let on_sigint = if ends_on_sigint { "; exit 130" } else { "" };
    format!(
        r#"trap 'echo INT >> "{signals}"{on_sigint}' INT
trap 'echo TERM >> "{signals}"; exit 143' TERM
echo 'half done' > wip.txt
touch "{started}"
while :; do sleep 0.1 || :; done
"#,
        signals = scenario.path("signals").display(),
        started = scenario.path("agent-started").display(),
    )
}

#[test]
fn an_interrupt_sends_a_codex_session_sigint_and_the_run_ends_as_interrupted() {
    let scenario = Scenario::new();
    let signals = scenario.path("signals");
    scenario.agent_does_for(7, &records_signals(&scenario, true));

    let result = scenario.run_and_signal(
        &["harness", "codex", &scenario.issue_url(7)],
        "agent-started",
        "INT",
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(fs::read_to_string(&signals).unwrap(), "INT\n");
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "thirdshift: failed run (interrupted)"
    );
    assert_eq!(
        scenario.origin_file("issue-7", "wip.txt"),
        Some("half done\n".to_string())
    );
}

#[test]
fn a_codex_session_that_outlasts_sigint_is_sent_sigterm_after_it() {
    let scenario = Scenario::new();
    let signals = scenario.path("signals");
    scenario.agent_does_for(7, &records_signals(&scenario, false));

    let result = scenario.run_and_signal(
        &["harness", "codex", &scenario.issue_url(7)],
        "agent-started",
        "TERM",
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(fs::read_to_string(&signals).unwrap(), "INT\nTERM\n");
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "thirdshift: failed run (interrupted)"
    );
}

#[test]
fn a_failed_turn_or_a_non_zero_exit_fails_the_run_with_codexs_error_in_the_cause() {
    for (script, cause) in [
        (
            "echo \"The 'gpt-6.1-sol' model is not supported.\" > \"$FAKE_CODEX_ERROR\"\nexit 1",
            "codex exited 1: The 'gpt-6.1-sol' model is not supported.",
        ),
        (
            r#"echo '{"type": "turn.failed", "error": {"message": "stream disconnected"}}'"#,
            "codex's turn failed: stream disconnected",
        ),
        (
            "echo '{\"type\": \"error\", \"message\": \"Reconnecting... 1/5\"}'\nkill -KILL $PPID",
            "codex exited by signal: Reconnecting... 1/5",
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

#[test]
fn a_merge_run_on_codex_merges_the_pr() {
    let scenario = Scenario::new();
    scenario.agent_does_for(7, AGENT_OPENS_PR);

    let result = scenario.run(&["merge", "harness", "codex", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_eq!(scenario.codex_calls().len(), 1);
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn ordinary_runs_ignore_the_security_harness_and_keep_codexs_ordinary_session_protocol() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[security]\nharness = \"claude\"\n\
         [harness]\ndefault = \"codex\"\n\
         [harness.codex]\nmodel = \"gpt-6-luna\"\neffort = \"high\"\n",
    );
    scenario.agent_does_for(7, AGENT_OPENS_PR);
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_sessions_on_codex(&scenario, "gpt-6-luna", "high");
    for call in scenario.codex_calls() {
        assert!(
            !argv(&call)
                .iter()
                .any(|arg| arg.contains("max_concurrent_threads"))
        );
    }
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
    assert!(!calls[0]["prompt"].as_str().unwrap().contains("fork_turns"));
    for call in &calls[1..] {
        assert_fresh_reviewers(call);
    }
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
