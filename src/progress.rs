//! Progress lines on stderr: thirdshift's own steps, and a session's
//! stream-json output condensed to one short line per notable event.

use std::collections::HashMap;
use std::fmt::Display;

use serde_json::Value;

/// The longest detail a session line shows before it is cut short.
const MAX_DETAIL: usize = 100;

/// Print one of thirdshift's own steps.
pub fn step(message: impl Display) {
    eprintln!("thirdshift: {message}");
}

/// Condenses one session's stream, a line at a time. Unknown and malformed
/// lines are skipped, never an error.
///
/// A `result` event doesn't mean the session is over: a session that waits on
/// background sub-agents resumes, with another `system`/`init` event, and ends
/// each resumption with another `result`. So only the first `init` gives a
/// line, `result` events give none, and [`Progress::summary`] reports the last
/// one once the process has exited.
///
/// A background task still running when the session ends its turn for good is
/// killed as the process exits, so a task killed after the last `result` is
/// work the session was waiting on: see [`Progress::killed_background_work`].
#[derive(Default)]
pub struct Progress {
    started: bool,
    cwd: Option<String>,
    session_id: Option<String>,
    /// Each background task's description, by task id.
    tasks: HashMap<String, String>,
    /// Descriptions of the tasks killed since the last `result`, by task id,
    /// in the order they were killed.
    killed: Vec<(String, String)>,
    /// Signed in with a claude.ai subscription rather than an API key.
    subscription: bool,
    turns_and_cost: Option<(u64, f64)>,
}

impl Progress {
    /// The stderr lines, without the `thirdshift: ` prefix, for one line of
    /// the stream: one per notable event in it, often none.
    pub fn condense(&mut self, raw: &str) -> Vec<String> {
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        match event["type"].as_str() {
            Some("system") if event["subtype"] == "init" && !self.started => {
                self.started = true;
                self.cwd = event["cwd"].as_str().map(String::from);
                self.session_id = event["session_id"].as_str().map(String::from);
                self.subscription = event["apiKeySource"] == "none";
                vec!["session started".to_string()]
            }
            Some("assistant") => event["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "tool_use")
                .filter_map(|block| Some(self.tool_use(block["name"].as_str()?, &block["input"])))
                .collect(),
            Some("system") => {
                self.task(&event);
                Vec::new()
            }
            Some("result") => {
                self.killed.clear();
                // Both are cumulative across a session's result events.
                if let (Some(turns), Some(cost)) = (
                    event["num_turns"].as_u64(),
                    event["total_cost_usd"].as_f64(),
                ) {
                    self.turns_and_cost = Some((turns, cost));
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Turns and cost from the last `result` event that reported them. On a
    /// claude.ai subscription the cost is the API-price equivalent, not a
    /// charge, and says so.
    pub fn summary(&self) -> Option<String> {
        let (turns, cost) = self.turns_and_cost?;
        let basis = if self.subscription {
            " at API prices"
        } else {
            ""
        };
        Some(format!("{turns} turns, ${cost:.2}{basis}"))
    }

    /// The session's id, from its first `init` event.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Descriptions of the background tasks killed after the last `result`:
    /// the work the session was still waiting on when it ended.
    pub fn killed_background_work(&self) -> Vec<&str> {
        self.killed
            .iter()
            .map(|(_, description)| description.as_str())
            .collect()
    }

    /// Track a background task's description and whether it was killed.
    fn task(&mut self, event: &Value) {
        let Some(id) = event["task_id"].as_str() else {
            return;
        };
        let killed = match event["subtype"].as_str() {
            Some("task_started") => {
                if let Some(description) = event["description"].as_str() {
                    self.tasks.insert(id.to_string(), description.to_string());
                }
                false
            }
            Some("task_updated") => event["patch"]["status"] == "killed",
            Some("task_notification") => event["status"] == "stopped",
            _ => false,
        };
        if killed && !self.killed.iter().any(|(killed_id, _)| killed_id == id) {
            let description = self
                .tasks
                .get(id)
                .map(String::as_str)
                .or(event["summary"].as_str())
                .unwrap_or(id)
                .to_string();
            self.killed.push((id.to_string(), description));
        }
    }

    fn tool_use(&self, name: &str, input: &Value) -> String {
        match name {
            "Skill" => format!("skill {}", input["skill"].as_str().unwrap_or("?")),
            "Bash" => bash(input["command"].as_str().unwrap_or("")),
            _ => {
                let detail = ["file_path", "pattern", "description", "url", "query"]
                    .iter()
                    .find_map(|key| input[key].as_str());
                match detail {
                    Some(detail) => format!("{name} {}", shorten(&self.relative(detail))),
                    None => name.to_string(),
                }
            }
        }
    }

    /// `path` relative to the session's working directory, if it is inside it.
    fn relative(&self, path: &str) -> String {
        self.cwd
            .as_deref()
            .and_then(|cwd| path.strip_prefix(cwd)?.strip_prefix('/'))
            .unwrap_or(path)
            .to_string()
    }
}

/// Commits and pushes by name; any other command by its first line.
fn bash(command: &str) -> String {
    let actions: Vec<&str> = ["commit", "push"]
        .into_iter()
        .filter(|subcommand| runs_git(command, subcommand))
        .collect();
    if actions.is_empty() {
        format!("$ {}", shorten(command.lines().next().unwrap_or("")))
    } else {
        actions.join(" and ")
    }
}

/// Does one of the commands chained in `command` start `git <subcommand>`?
fn runs_git(command: &str, subcommand: &str) -> bool {
    command.split(['\n', ';', '&', '|']).any(|part| {
        let mut words = part.split_whitespace();
        words.next() == Some("git") && words.next() == Some(subcommand)
    })
}

fn shorten(text: &str) -> String {
    match text.char_indices().nth(MAX_DETAIL) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines(events: &[Value]) -> (Progress, Vec<Vec<String>>) {
        let mut progress = Progress::default();
        let lines = events
            .iter()
            .map(|event| progress.condense(&event.to_string()))
            .collect();
        (progress, lines)
    }

    fn tool_use(name: &str, input: Value) -> Value {
        json!({
            "type": "assistant",
            "message": { "content": [{ "type": "tool_use", "name": name, "input": input }] }
        })
    }

    fn init(cwd: &str) -> Value {
        json!({ "type": "system", "subtype": "init", "cwd": cwd })
    }

    fn line_for(event: Value) -> Option<String> {
        let mut lines = lines(&[init("/work/widgets-issue-7"), event])
            .1
            .pop()
            .unwrap();
        assert!(lines.len() <= 1, "several lines: {lines:?}");
        lines.pop()
    }

    fn result(turns: u64, cost: f64) -> Value {
        json!({ "type": "result", "num_turns": turns, "total_cost_usd": cost })
    }

    #[test]
    fn only_the_first_init_starts_the_session() {
        let (_, lines) = lines(&[init("/a"), init("/a")]);
        assert_eq!(lines, [vec!["session started".to_string()], vec![]]);
    }

    #[test]
    fn a_skill_is_named() {
        assert_eq!(
            line_for(tool_use(
                "Skill",
                json!({ "skill": "thirdshift:tdd", "args": "x" })
            )),
            Some("skill thirdshift:tdd".to_string())
        );
    }

    #[test]
    fn commits_and_pushes_are_named() {
        let line = |command: &str| line_for(tool_use("Bash", json!({ "command": command })));
        assert_eq!(line("git commit -m 'x'"), Some("commit".to_string()));
        assert_eq!(line("git push -u origin issue-7"), Some("push".to_string()));
        assert_eq!(
            line("git add -A && git commit -m x && git push"),
            Some("commit and push".to_string())
        );
    }

    #[test]
    fn mentions_of_commit_or_push_are_not_commits_or_pushes() {
        let line = |command: &str| line_for(tool_use("Bash", json!({ "command": command })));
        assert_eq!(
            line("grep 'git push' README.md"),
            Some("$ grep 'git push' README.md".to_string())
        );
        assert_eq!(
            line("git commit-tree HEAD^{tree}"),
            Some("$ git commit-tree HEAD^{tree}".to_string())
        );
    }

    #[test]
    fn other_commands_show_their_first_line() {
        assert_eq!(
            line_for(tool_use(
                "Bash",
                json!({ "command": "cargo test\necho done" })
            )),
            Some("$ cargo test".to_string())
        );
    }

    #[test]
    fn file_tools_show_the_path_relative_to_the_session() {
        assert_eq!(
            line_for(tool_use(
                "Edit",
                json!({ "file_path": "/work/widgets-issue-7/src/run.rs" })
            )),
            Some("Edit src/run.rs".to_string())
        );
        assert_eq!(
            line_for(tool_use("Read", json!({ "file_path": "/etc/hosts" }))),
            Some("Read /etc/hosts".to_string())
        );
    }

    #[test]
    fn other_tools_show_their_most_telling_input() {
        assert_eq!(
            line_for(tool_use("Grep", json!({ "pattern": "fn main" }))),
            Some("Grep fn main".to_string())
        );
        assert_eq!(
            line_for(tool_use(
                "Agent",
                json!({ "description": "Standards review" })
            )),
            Some("Agent Standards review".to_string())
        );
        assert_eq!(
            line_for(tool_use("TodoWrite", json!({ "todos": [] }))),
            Some("TodoWrite".to_string())
        );
    }

    #[test]
    fn long_details_are_cut_short() {
        let line = line_for(tool_use("Bash", json!({ "command": "é".repeat(150) }))).unwrap();
        assert_eq!(line, format!("$ {}…", "é".repeat(100)));
    }

    #[test]
    fn several_tool_uses_in_one_message_get_a_line_each() {
        let event = json!({
            "type": "assistant",
            "message": { "content": [
                { "type": "text", "text": "Reading both." },
                { "type": "tool_use", "name": "Read", "input": { "file_path": "a.rs" } },
                { "type": "tool_use", "name": "Read", "input": { "file_path": "b.rs" } }
            ] }
        });
        let (_, lines) = lines(&[event]);
        assert_eq!(
            lines,
            [vec!["Read a.rs".to_string(), "Read b.rs".to_string()]]
        );
    }

    #[test]
    fn text_tool_results_and_results_give_no_line() {
        let (_, lines) = lines(&[
            json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "hi" }] } }),
            json!({ "type": "user", "message": { "content": [{ "type": "tool_result" }] } }),
            json!({ "type": "result", "num_turns": 3, "total_cost_usd": 0.1 }),
        ]);
        assert_eq!(lines, [vec![], vec![], vec![]] as [Vec<String>; 3]);
    }

    #[test]
    fn the_summary_is_the_last_result_with_totals() {
        let (progress, _) = lines(&[
            result(10, 0.5),
            result(34, 1.8249),
            json!({ "type": "result", "subtype": "success" }),
        ]);
        assert_eq!(progress.summary().as_deref(), Some("34 turns, $1.82"));
    }

    #[test]
    fn on_a_subscription_the_cost_is_marked_as_at_api_prices() {
        let (progress, _) = lines(&[
            json!({ "type": "system", "subtype": "init", "apiKeySource": "none" }),
            result(34, 1.82),
        ]);
        assert_eq!(
            progress.summary().as_deref(),
            Some("34 turns, $1.82 at API prices")
        );
    }

    #[test]
    fn no_result_means_no_summary() {
        assert_eq!(Progress::default().summary(), None);
    }

    fn task_started(id: &str, description: &str) -> Value {
        json!({ "type": "system", "subtype": "task_started", "task_id": id, "description": description })
    }

    fn task_updated(id: &str, status: &str) -> Value {
        json!({ "type": "system", "subtype": "task_updated", "task_id": id, "patch": { "status": status } })
    }

    fn task_notification(id: &str, status: &str, summary: &str) -> Value {
        json!({ "type": "system", "subtype": "task_notification", "task_id": id, "status": status, "summary": summary })
    }

    #[test]
    fn a_task_killed_after_the_last_result_is_killed_background_work() {
        let (progress, _) = lines(&[
            json!({ "type": "system", "subtype": "init", "session_id": "s-1" }),
            task_started("b1", "./mvnw test -Dtest='GamesPageTest'"),
            result(12, 0.4),
            task_updated("b1", "killed"),
        ]);
        assert_eq!(progress.session_id(), Some("s-1"));
        assert_eq!(
            progress.killed_background_work(),
            ["./mvnw test -Dtest='GamesPageTest'"]
        );
    }

    #[test]
    fn a_stopped_task_notification_is_killed_background_work_once() {
        let (progress, _) = lines(&[
            task_started("b1", "cargo test"),
            result(12, 0.4),
            task_updated("b1", "killed"),
            task_notification("b1", "stopped", "cargo test"),
            task_notification("b2", "stopped", "npm run build"),
        ]);
        assert_eq!(
            progress.killed_background_work(),
            ["cargo test", "npm run build"]
        );
    }

    #[test]
    fn no_killed_tasks_means_no_killed_background_work() {
        let (progress, _) = lines(&[
            task_started("b1", "cargo test"),
            task_notification("b1", "completed", "cargo test"),
            result(12, 0.4),
        ]);
        assert!(progress.killed_background_work().is_empty());
    }

    #[test]
    fn background_sub_agents_that_resumed_the_session_are_not_killed_background_work() {
        let (progress, _) = lines(&[
            task_started("a1", "Standards review"),
            result(12, 0.4),
            task_updated("a1", "completed"),
            task_notification("a1", "completed", "Standards review"),
            init("/work/widgets-issue-7"),
            result(20, 0.9),
        ]);
        assert!(progress.killed_background_work().is_empty());
    }

    #[test]
    fn a_task_killed_before_a_later_result_is_not_killed_background_work() {
        let (progress, _) = lines(&[
            task_started("b1", "cargo watch"),
            task_updated("b1", "killed"),
            result(12, 0.4),
            init("/work/widgets-issue-7"),
            task_notification("b1", "stopped", "cargo watch"),
            result(20, 0.9),
        ]);
        assert!(progress.killed_background_work().is_empty());
    }

    #[test]
    fn unknown_and_malformed_lines_are_skipped() {
        let mut progress = Progress::default();
        for raw in [
            "",
            "not json",
            "[1, 2]",
            "42",
            r#"{"type": "assistant""#,
            r#"{"no_type": true}"#,
            r#"{"type": 7}"#,
            r#"{"type": "stream_event", "event": {}}"#,
            r#"{"type": "assistant", "message": {"content": "oops"}}"#,
            r#"{"type": "assistant", "message": {"content": [{"type": "tool_use"}]}}"#,
            r#"{"type": "result", "num_turns": "many", "total_cost_usd": null}"#,
        ] {
            assert_eq!(progress.condense(raw), Vec::<String>::new(), "{raw}");
        }
        assert_eq!(progress.summary(), None);
    }
}
