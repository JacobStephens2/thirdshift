//! Codex's `exec --json` stream, condensed: `thread.started` gives the
//! session id, `item.*` events give a progress line per command, file
//! change and tool call, `turn.completed` the token totals and the commands
//! and sub-agent calls killed as the session ends, and `turn.failed` the
//! error the session failed with. See
//! `docs/research/codex-headless-harness.md`.

use crate::harness::interpretation::{Decoder, Ended, Facts, Report, TurnOutcome};
use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;

use crate::progress::{bash, shorten};

/// Condenses one Codex session's JSONL stream, a line at a time.
///
/// `codex exec` runs one turn per process. A command or sub-agent call the
/// agent left running shows as a `command_execution` or `collab_tool_call`
/// item still `in_progress` when the turn completes, and it is killed as
/// Codex exits: the completed interpretation records that evidence.
#[derive(Default)]
pub struct CodexProgress {
    /// The worktree the session runs in, which Codex's stream doesn't name.
    cwd: String,
    session_id: Option<String>,
    /// The ids of the items that have had their line.
    seen: HashSet<String>,
    /// The descriptions of the commands and sub-agent calls still running,
    /// by item id, in the order they started.
    running: Vec<(String, String)>,
    /// The descriptions of those still running when the turn completed.
    killed: Vec<String>,
    /// Token totals from `turn.completed`.
    tokens: Option<Tokens>,
    final_message: Option<String>,
    /// What failed the turn, from `turn.failed`.
    failure: Option<String>,
    /// The last error the stream reported, which may have been retried.
    last_error: Option<String>,
}

/// A session's token totals.
struct Tokens {
    input: u64,
    /// Of the input, how many were cached.
    cached: u64,
    output: u64,
}

impl CodexProgress {
    /// The stream of a session running in `worktree`.
    pub fn in_worktree(worktree: &Path) -> Self {
        CodexProgress {
            cwd: worktree.display().to_string(),
            ..CodexProgress::default()
        }
    }

    /// Keep the message of an `error`, an event or an item, as the last
    /// error reported.
    fn reported(&mut self, error: &Value) {
        if let Some(message) = error["message"].as_str() {
            self.last_error = Some(message.to_string());
        }
    }

    /// Track whether the command or sub-agent call `item`, with id `id`, is
    /// still running, as `description`.
    fn track_running(&mut self, id: &str, item: &Value, description: String) {
        self.running.retain(|(running, _)| running != id);
        if item["status"] == "in_progress" {
            self.running.push((id.to_string(), description));
        }
    }

    /// Track an item's state, and give its line the first time it is seen.
    fn item(&mut self, item: &Value) -> Vec<String> {
        let id = item["id"].as_str().unwrap_or("").to_string();
        match item["type"].as_str() {
            Some("command_execution") => {
                let command = unwrapped(item["command"].as_str().unwrap_or(""));
                self.track_running(&id, item, command);
            }
            Some("collab_tool_call") => {
                self.track_running(&id, item, sub_agent_call(item));
            }
            Some("agent_message") => {
                if let Some(text) = item["text"].as_str() {
                    self.final_message = Some(text.to_string());
                }
            }
            Some("error") => {
                self.reported(item);
            }
            _ => {}
        }
        if !self.seen.insert(id) {
            return Vec::new();
        }
        self.lines_for(item)
    }

    /// The progress lines an item gives.
    fn lines_for(&self, item: &Value) -> Vec<String> {
        let text = |key: &str| item[key].as_str().unwrap_or("?");
        match item["type"].as_str() {
            Some("command_execution") => vec![bash(&unwrapped(text("command")))],
            Some("file_change") => item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|change| {
                    let path = change["path"].as_str().unwrap_or("?");
                    let kind = match change["kind"].as_str() {
                        Some("add") => "Add",
                        Some("delete") => "Delete",
                        _ => "Edit",
                    };
                    format!("{kind} {}", shorten(self.relative(path)))
                })
                .collect(),
            Some("mcp_tool_call") => vec![format!("{} {}", text("server"), text("tool"))],
            Some("web_search") => vec![format!("web search {}", shorten(text("query")))],
            Some("collab_tool_call") => vec![format!("agent {}", text("tool"))],
            _ => Vec::new(),
        }
    }

    /// `path` relative to the worktree, if it is inside it.
    fn relative<'a>(&self, path: &'a str) -> &'a str {
        path.strip_prefix(&self.cwd)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(path)
    }
}

impl Decoder for CodexProgress {
    fn condense(&mut self, raw: &str) -> Vec<String> {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        match event["type"].as_str() {
            Some("thread.started") if self.session_id.is_none() => {
                self.session_id = event["thread_id"].as_str().map(String::from);
                vec!["session started".to_string()]
            }
            Some("item.started" | "item.updated" | "item.completed") => self.item(&event["item"]),
            Some("turn.completed") => {
                let usage = &event["usage"];
                let count = |key: &str| usage[key].as_u64();
                if let (Some(input), Some(output)) = (count("input_tokens"), count("output_tokens"))
                {
                    let cached = count("cached_input_tokens").unwrap_or(0);
                    self.tokens = Some(Tokens {
                        input,
                        cached,
                        output,
                    });
                }
                self.killed = self
                    .running
                    .iter()
                    .map(|(_, command)| command.clone())
                    .collect();
                Vec::new()
            }
            Some("turn.failed") => {
                if self.failure.is_none() {
                    let message = event["error"]["message"].as_str().unwrap_or("turn failed");
                    self.failure = Some(message.to_string());
                }
                Vec::new()
            }
            Some("error") => {
                self.reported(&event);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn complete(self: Box<Self>) -> Facts {
        let summary = self.summary();
        let outcome = TurnOutcome::from_failed(self.failure.is_some());
        Facts {
            report: Report {
                warnings: Vec::new(),
                summary,
                ..Report::default()
            },
            ended: Ended {
                session_id: self.session_id,
                killed: self.killed,
                final_message: self.final_message,
            },
            outcome,
            diagnostic: self.failure.or(self.last_error),
        }
    }
}

impl CodexProgress {
    /// The token totals from `turn.completed`: Codex reports no turns and no
    /// cost.
    fn summary(&self) -> Option<String> {
        let Tokens {
            input,
            cached,
            output,
        } = self.tokens.as_ref()?;
        Some(format!(
            "{input} input tokens ({cached} cached), {output} output tokens"
        ))
    }
}

/// The sub-agent call `item` as killed background work is described: by its
/// tool and the first line of the prompt it gave, if any, as in `agent
/// spawn_agent: Review the diff`.
fn sub_agent_call(item: &Value) -> String {
    let tool = item["tool"].as_str().unwrap_or("?");
    match item["prompt"]
        .as_str()
        .and_then(|prompt| prompt.lines().next())
    {
        Some(prompt) if !prompt.is_empty() => format!("agent {tool}: {}", shorten(prompt)),
        _ => format!("agent {tool}"),
    }
}

/// `command` without the shell Codex wraps it in, as in `/bin/bash -lc 'git
/// push'`, if it is wrapped so.
fn unwrapped(command: &str) -> String {
    let mut words = command.splitn(3, ' ');
    let (Some(shell), Some(flag), Some(script)) = (words.next(), words.next(), words.next()) else {
        return command.to_string();
    };
    let is_shell = ["bash", "sh", "zsh"]
        .iter()
        .any(|name| shell == *name || shell.ends_with(&format!("/{name}")));
    if !is_shell || !matches!(flag, "-c" | "-lc") {
        return command.to_string();
    }
    if let Some(quoted) = script
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    {
        return quoted.replace("'\\''", "'");
    }
    if let Some(quoted) = script
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        return quoted.replace("\\\"", "\"");
    }
    script.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{
        Harness,
        interpretation_tests::{finish, lines, stream},
    };
    use serde_json::json;

    fn item(kind: &str, id: &str, item: Value) -> Value {
        let mut item = item;
        item["id"] = json!(id);
        json!({ "type": kind, "item": item })
    }

    fn command(kind: &str, id: &str, command: &str, status: &str) -> Value {
        item(
            kind,
            id,
            json!({ "type": "command_execution", "command": command, "status": status }),
        )
    }

    fn completed(input: u64, output: u64) -> Value {
        json!({ "type": "turn.completed", "usage": {
            "input_tokens": input, "cached_input_tokens": 100, "output_tokens": output
        } })
    }

    #[test]
    fn the_first_thread_started_starts_the_session_and_gives_its_id() {
        let (progress, lines) = lines(
            Harness::Codex,
            &[
                json!({ "type": "thread.started", "thread_id": "019a-thread" }),
                json!({ "type": "turn.started" }),
            ],
        );

        assert_eq!(lines, [vec!["session started".to_string()], vec![]]);
        assert_eq!(
            progress.outcome.as_ref().unwrap().session_id.as_deref(),
            Some("019a-thread")
        );
    }

    #[test]
    fn a_command_gets_one_line_without_its_shell_naming_commits_and_pushes() {
        let (_, lines) = lines(
            Harness::Codex,
            &[
                command(
                    "item.started",
                    "item_1",
                    "/bin/bash -lc 'cargo test'",
                    "in_progress",
                ),
                command(
                    "item.completed",
                    "item_1",
                    "/bin/bash -lc 'cargo test'",
                    "completed",
                ),
                command(
                    "item.started",
                    "item_2",
                    "bash -lc 'git add -A && git commit -m '\\''x'\\'' && git push'",
                    "in_progress",
                ),
                command("item.completed", "item_3", "git push", "completed"),
            ],
        );

        assert_eq!(
            lines,
            [
                vec!["$ cargo test".to_string()],
                vec![],
                vec!["commit and push".to_string()],
                vec!["push".to_string()],
            ]
        );
    }

    #[test]
    fn file_changes_tool_calls_and_searches_get_a_line_each() {
        let (_, lines) = lines(
            Harness::Codex,
            &[
                item(
                    "item.completed",
                    "item_1",
                    json!({ "type": "file_change", "status": "completed", "changes": [
                    { "path": "/work/widgets-issue-7/src/run.rs", "kind": "update" },
                    { "path": "/work/widgets-issue-7/NOTES.md", "kind": "add" },
                    { "path": "/etc/hosts", "kind": "delete" }
                ] }),
                ),
                item(
                    "item.started",
                    "item_2",
                    json!({ "type": "mcp_tool_call", "server": "github", "tool": "get_issue" }),
                ),
                item(
                    "item.started",
                    "item_3",
                    json!({ "type": "web_search", "query": "codex exec json" }),
                ),
                item(
                    "item.started",
                    "item_4",
                    json!({ "type": "collab_tool_call", "tool": "spawn_agent" }),
                ),
            ],
        );

        assert_eq!(
            lines,
            [
                vec![
                    "Edit src/run.rs".to_string(),
                    "Add NOTES.md".to_string(),
                    "Delete /etc/hosts".to_string()
                ],
                vec!["github get_issue".to_string()],
                vec!["web search codex exec json".to_string()],
                vec!["agent spawn_agent".to_string()],
            ]
        );
    }

    #[test]
    fn messages_reasoning_and_turn_events_give_no_line() {
        let (_, lines) = lines(
            Harness::Codex,
            &[
                item(
                    "item.completed",
                    "item_1",
                    json!({ "type": "reasoning", "text": "Thinking." }),
                ),
                item(
                    "item.completed",
                    "item_2",
                    json!({ "type": "agent_message", "text": "Done." }),
                ),
                json!({ "type": "turn.started" }),
                completed(10, 2),
            ],
        );

        assert_eq!(lines, [vec![], vec![], vec![], vec![]] as [Vec<String>; 4]);
    }

    #[test]
    fn the_final_message_is_the_last_agent_message() {
        let said = |id: &str, text: &str| {
            item(
                "item.completed",
                id,
                json!({ "type": "agent_message", "text": text }),
            )
        };
        let (progress, _) = lines(
            Harness::Codex,
            &[said("item_1", "Running tests."), said("item_5", "Done.")],
        );

        assert_eq!(
            progress.outcome.as_ref().unwrap().final_message.as_deref(),
            Some("Done.")
        );
    }

    #[test]
    fn the_summary_is_the_token_totals() {
        let (progress, _) = lines(Harness::Codex, &[completed(12000, 345)]);

        assert_eq!(
            progress.report.as_ref().unwrap().summary.clone().as_deref(),
            Some("12000 input tokens (100 cached), 345 output tokens")
        );
        assert_eq!(lines(Harness::Codex, &[]).0.report.unwrap().summary, None);
    }

    #[test]
    fn a_command_still_running_when_the_turn_completes_is_killed_background_work() {
        let (progress, _) = lines(
            Harness::Codex,
            &[
                command(
                    "item.started",
                    "item_1",
                    "/bin/bash -lc 'sleep 188'",
                    "in_progress",
                ),
                command("item.started", "item_2", "cargo build", "in_progress"),
                command("item.completed", "item_2", "cargo build", "completed"),
                completed(10, 2),
            ],
        );

        assert_eq!(
            progress.outcome.as_ref().unwrap().killed_work(),
            ["sleep 188"]
        );
        assert!(progress.outcome.is_ok());
    }

    #[test]
    fn a_command_reconciled_as_in_progress_at_turn_end_is_killed_background_work() {
        let (progress, _) = lines(
            Harness::Codex,
            &[
                command("item.started", "item_1", "npm run dev", "in_progress"),
                command("item.completed", "item_1", "npm run dev", "in_progress"),
                completed(10, 2),
            ],
        );

        assert_eq!(
            progress.outcome.as_ref().unwrap().killed_work(),
            ["npm run dev"]
        );
    }

    #[test]
    fn a_sub_agent_call_still_running_when_the_turn_completes_is_killed_background_work() {
        let call = |kind: &str, id: &str, tool: &str, prompt: &str, status: &str| {
            item(
                kind,
                id,
                json!({ "type": "collab_tool_call", "tool": tool, "prompt": prompt, "status": status }),
            )
        };
        let (progress, _) = lines(
            Harness::Codex,
            &[
                call(
                    "item.started",
                    "item_1",
                    "spawn_agent",
                    "Review the diff\nThen report.",
                    "in_progress",
                ),
                call("item.started", "item_2", "wait", "", "in_progress"),
                call("item.completed", "item_2", "wait", "", "completed"),
                call("item.started", "item_3", "spawn_agent", "", "in_progress"),
                completed(10, 2),
            ],
        );

        assert_eq!(
            progress.outcome.as_ref().unwrap().killed_work(),
            ["agent spawn_agent: Review the diff", "agent spawn_agent"]
        );
    }

    #[test]
    fn a_failed_turn_fails_with_its_error_and_kills_no_background_work() {
        let (progress, _) = lines(
            Harness::Codex,
            &[
                command("item.started", "item_1", "sleep 188", "in_progress"),
                json!({ "type": "error", "message": "Reconnecting... 1/5" }),
                json!({ "type": "turn.failed", "error": { "message": "The 'GPT-6.1-Sol' model is not supported." } }),
            ],
        );

        assert!(progress.outcome.is_err());
        assert_eq!(
            progress.outcome.as_ref().unwrap_err().to_string(),
            "codex's turn failed: The 'GPT-6.1-Sol' model is not supported."
        );
    }

    #[test]
    fn a_retried_diagnostic_does_not_fail_a_turn_but_explains_a_nonzero_exit() {
        use std::os::unix::process::ExitStatusExt;
        for code in [0, 3] {
            let mut progress = stream(Harness::Codex, "");
            progress.condense(r#"{"type":"error","message":"Reconnecting... 1/5"}"#);
            progress.condense(&completed(10, 2).to_string());
            let completion = progress.finish(Ok(std::process::ExitStatus::from_raw(code << 8)));
            if code == 0 {
                assert!(completion.outcome.is_ok());
            } else {
                assert_eq!(
                    completion.outcome.unwrap_err().to_string(),
                    "codex exited 3: Reconnecting... 1/5"
                );
            }
        }
    }

    #[test]
    fn unknown_and_malformed_lines_are_skipped() {
        let mut progress = stream(Harness::Codex, "");
        for raw in [
            "",
            "not json",
            "[1, 2]",
            r#"{"type": "item.started""#,
            r#"{"type": "item.started", "item": 3}"#,
            r#"{"type": "item.started", "item": {"type": "command_execution"}}"#,
            r#"{"type": "turn.completed", "usage": {"input_tokens": "many"}}"#,
        ] {
            let lines = progress.condense(raw);
            assert!(lines.len() <= 1, "{raw}: {lines:?}");
        }
        assert_eq!(finish(progress).report.unwrap().summary, None);
    }

    #[test]
    fn a_command_not_wrapped_in_a_shell_is_kept_as_it_is() {
        for command in ["cargo test", "bash script.sh", "bash -lc", "python -c 'x'"] {
            assert_eq!(unwrapped(command), command);
        }
        assert_eq!(
            unwrapped("/usr/bin/zsh -c \"echo \\\"hi\\\"\""),
            "echo \"hi\""
        );
    }
}
