//! Progress lines on stderr: thirdshift's own steps, and a session's
//! stream-json output condensed to one short line per notable event.

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
#[derive(Default)]
pub struct Progress {
    started: bool,
    cwd: Option<String>,
    totals: Option<String>,
}

impl Progress {
    /// The stderr line, without the `thirdshift: ` prefix, for one line of the
    /// stream, if it is a notable event.
    pub fn line(&mut self, raw: &str) -> Option<String> {
        let event: Value = serde_json::from_str(raw).ok()?;
        match event["type"].as_str()? {
            "system" if event["subtype"] == "init" => {
                if self.started {
                    return None;
                }
                self.started = true;
                self.cwd = event["cwd"].as_str().map(String::from);
                Some("session started".to_string())
            }
            "assistant" => {
                let lines: Vec<String> = event["message"]["content"]
                    .as_array()?
                    .iter()
                    .filter(|block| block["type"] == "tool_use")
                    .filter_map(|block| {
                        Some(self.tool_use(block["name"].as_str()?, &block["input"]))
                    })
                    .collect();
                (!lines.is_empty()).then(|| lines.join("; "))
            }
            "result" => {
                // Both are cumulative across a session's result events.
                if let (Some(turns), Some(cost)) = (
                    event["num_turns"].as_u64(),
                    event["total_cost_usd"].as_f64(),
                ) {
                    self.totals = Some(format!("{turns} turns, ${cost:.2} at API prices"));
                }
                None
            }
            _ => None,
        }
    }

    /// Turns and cost from the last `result` event that reported them. On a
    /// claude.ai subscription the cost is the API-price equivalent, not a
    /// charge.
    pub fn summary(&self) -> Option<&str> {
        self.totals.as_deref()
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
    let actions: Vec<&str> = [("git commit", "commit"), ("git push", "push")]
        .into_iter()
        .filter(|(needle, _)| command.contains(needle))
        .map(|(_, action)| action)
        .collect();
    if actions.is_empty() {
        format!("$ {}", shorten(command.lines().next().unwrap_or("")))
    } else {
        actions.join(" and ")
    }
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

    fn lines(events: &[Value]) -> (Progress, Vec<Option<String>>) {
        let mut progress = Progress::default();
        let lines = events
            .iter()
            .map(|event| progress.line(&event.to_string()))
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
        lines(&[init("/work/widgets-issue-7"), event])
            .1
            .pop()
            .unwrap()
    }

    #[test]
    fn only_the_first_init_starts_the_session() {
        let (_, lines) = lines(&[init("/a"), init("/a")]);
        assert_eq!(lines, [Some("session started".to_string()), None]);
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
    fn several_tool_uses_in_one_message_share_a_line() {
        let event = json!({
            "type": "assistant",
            "message": { "content": [
                { "type": "text", "text": "Reading both." },
                { "type": "tool_use", "name": "Read", "input": { "file_path": "a.rs" } },
                { "type": "tool_use", "name": "Read", "input": { "file_path": "b.rs" } }
            ] }
        });
        assert_eq!(line_for(event), Some("Read a.rs; Read b.rs".to_string()));
    }

    #[test]
    fn text_tool_results_and_results_give_no_line() {
        let (_, lines) = lines(&[
            json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "hi" }] } }),
            json!({ "type": "user", "message": { "content": [{ "type": "tool_result" }] } }),
            json!({ "type": "result", "num_turns": 3, "total_cost_usd": 0.1 }),
        ]);
        assert_eq!(lines, [None, None, None]);
    }

    #[test]
    fn the_summary_is_the_last_result_with_totals() {
        let (progress, _) = lines(&[
            json!({ "type": "result", "num_turns": 10, "total_cost_usd": 0.5 }),
            json!({ "type": "result", "num_turns": 34, "total_cost_usd": 1.8249 }),
            json!({ "type": "result", "subtype": "success" }),
        ]);
        assert_eq!(progress.summary(), Some("34 turns, $1.82 at API prices"));
    }

    #[test]
    fn no_result_means_no_summary() {
        assert_eq!(Progress::default().summary(), None);
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
            assert_eq!(progress.line(raw), None, "{raw}");
        }
        assert_eq!(progress.summary(), None);
    }
}
