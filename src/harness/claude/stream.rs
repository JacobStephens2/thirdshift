//! Claude Code's stream-json session protocol.

use std::collections::HashMap;

use serde_json::Value;

use crate::progress::{Stream, bash, shorten};

/// Condenses one Claude session's stream-json, a line at a time. Unknown and
/// malformed lines are skipped, never an error.
///
/// A `result` event doesn't mean the session is over: a session that waits on
/// background sub-agents resumes, with another `system`/`init` event, and ends
/// each resumption with another `result`. So only the first `init` gives a
/// line, `result` events give none, and [`ClaudeProgress::summary`] reports the last
/// one once the process has exited.
///
/// A background task still running when the session ends its turn for good is
/// killed as the process exits, so a task killed after the last `result` is
/// work the session may have been waiting on, or work it had given up on and
/// left running: see [`ClaudeProgress::killed_background_work`].
#[derive(Default)]
pub struct ClaudeProgress {
    started: bool,
    cwd: Option<String>,
    session_id: Option<String>,
    /// Each background task's description, by task id.
    tasks: HashMap<String, String>,
    /// Descriptions of the tasks killed since the last `result`, by task id,
    /// in the order they were killed.
    killed: Vec<(String, String)>,
    /// The last `result` was an error, such as running out of turns.
    failed: bool,
    /// Signed in with a claude.ai subscription rather than an API key.
    subscription: bool,
    turns_and_cost: Option<(u64, f64)>,
    /// The text of the last `result`, if it had any.
    final_message: Option<String>,
}

impl Stream for ClaudeProgress {
    fn condense(&mut self, raw: &str) -> Vec<String> {
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
                self.track_task(&event);
                Vec::new()
            }
            Some("result") => {
                self.killed.clear();
                self.failed = event["subtype"] != "success" || event["is_error"] == true;
                self.final_message = event["result"].as_str().map(String::from);
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
    fn summary(&self) -> Option<String> {
        let (turns, cost) = self.turns_and_cost?;
        let basis = if self.subscription {
            " at API prices"
        } else {
            ""
        };
        Some(format!("{turns} turns, ${cost:.2}{basis}"))
    }

    /// The text of the last `result` event. None if that `result` had no
    /// text, as one that is an error may not.
    fn final_message(&self) -> Option<&str> {
        self.final_message.as_deref()
    }

    /// The session's id, from its first `init` event.
    fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Descriptions of the background tasks killed after the last `result`.
    /// The stream doesn't say whether the session was waiting on them, or
    /// had abandoned them. None if that `result` was an error.
    fn killed_background_work(&self) -> Vec<&str> {
        if self.failed {
            return Vec::new();
        }
        self.killed
            .iter()
            .map(|(_, description)| description.as_str())
            .collect()
    }
}

impl ClaudeProgress {
    /// Track a background task's description and whether it was killed.
    fn track_task(&mut self, event: &Value) {
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
