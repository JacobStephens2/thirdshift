//! Render every show field, including nested Runs, without a second schema.
use askama::Template;
use serde_json::Value;

use super::filters;

#[derive(Template)]
#[template(path = "console/peek.html")]
pub(super) struct Peek {
    pub command: String,
    pub cli: String,
    pub facts: Vec<Fact>,
    pub logs: Vec<Log>,
    pub can_stop: bool,
    pub live: bool,
    pub stopping: bool,
    pub stop_cli: String,
    pub dismissal: Option<DismissAction>,
    pub csrf: String,
    pub branches: Vec<String>,
    pub pull_requests: Vec<String>,
    pub issue_url: Option<String>,
    pub issue_label: String,
}

pub(super) struct Log {
    pub name: String,
    pub url: String,
}

pub(super) struct DismissAction {
    path: &'static str,
    label: &'static str,
    cli: String,
}
pub(super) struct Fact {
    label: String,
    value: String,
    children: Vec<Fact>,
}

impl Peek {
    pub fn new(command: String, inspection: crate::logs::stop::Inspection, csrf: String) -> Self {
        let value = inspection.details;
        let (issue_url, issue_label) = match value.get("issue") {
            Some(Value::Object(issue)) => match (
                issue.get("url").and_then(Value::as_str),
                issue.get("number").and_then(Value::as_u64),
            ) {
                (Some(url), Some(number)) if url.starts_with("https://") => {
                    (Some(url.to_owned()), format!("Issue #{number}"))
                }
                _ => (None, String::new()),
            },
            _ => (None, String::new()),
        };
        let mut logs = Vec::new();
        if let Some(sessions) = value["session_logs"].as_array() {
            for session in sessions.iter().filter_map(Value::as_str) {
                logs.push(Log {
                    name: session.into(),
                    url: crate::logs::status::inspect::url(
                        "/log",
                        &command,
                        Some(std::path::Path::new(session)),
                    ),
                });
            }
        }
        let cli = format!(
            "thirdshift show '{}' --json",
            command.replace('\'', "'\\''")
        );
        Self {
            dismissal: inspection.dismissal.map(|action| {
                let (path, label, suffix) = match action {
                    crate::logs::dismiss::Action::Dismiss => ("/dismiss", "Dismiss", ""),
                    crate::logs::dismiss::Action::Undo => ("/dismiss/undo", "Undo", " --undo"),
                };
                DismissAction {
                    path,
                    label,
                    cli: format!(
                        "thirdshift dismiss '{}'{suffix}",
                        command.replace('\'', "'\\''")
                    ),
                }
            }),
            csrf,
            live: value["state"] == "live",
            stop_cli: format!("thirdshift stop '{}'", command.replace('\'', "'\\''")),
            can_stop: inspection.can_stop,
            stopping: inspection.stopping,
            branches: inspection.branches,
            pull_requests: inspection.pull_requests,
            issue_url,
            issue_label,
            command,
            cli,
            facts: Fact::children(value, false),
            logs,
        }
    }
    fn fact<'a>(&self, fact: &'a Fact) -> FactTemplate<'a> {
        FactTemplate { fact }
    }
}

impl Fact {
    fn new(label: String, value: Value, in_metrics: bool) -> Self {
        let in_metrics = in_metrics || label == "run_metrics";
        let text = match &value {
            Value::Null if in_metrics => "Missing".into(),
            Value::Null => "None".into(),
            Value::String(text) if label == "kind" || label == "name" => {
                crate::logs::display_name(text).to_owned()
            }
            Value::String(text) => text.clone(),
            Value::Array(items) if items.is_empty() => "None".into(),
            Value::Object(items) if items.is_empty() => "None".into(),
            Value::Array(_) | Value::Object(_) => String::new(),
            other => other.to_string(),
        };
        Self {
            label: label.replace('_', " "),
            value: text,
            children: Self::children(value, in_metrics),
        }
    }
    fn children(value: Value, in_metrics: bool) -> Vec<Self> {
        match value {
            Value::Object(fields) => fields
                .into_iter()
                .map(|(label, value)| Self::new(label, value, in_metrics))
                .collect(),
            Value::Array(items) => items
                .into_iter()
                .enumerate()
                .map(|(index, value)| Self::new((index + 1).to_string(), value, in_metrics))
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Template)]
#[template(path = "console/fact.html")]
struct FactTemplate<'a> {
    fact: &'a Fact,
}
impl FactTemplate<'_> {
    fn child<'a>(&self, fact: &'a Fact) -> FactTemplate<'a> {
        FactTemplate { fact }
    }
}
