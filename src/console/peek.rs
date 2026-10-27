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
    /// The Experiment page of an Experiment record that names its id.
    pub experiment_url: Option<String>,
    pub subject: String,
    pub title: String,
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
    /// Where the fact sits, such as `stages/1`, so its fold survives a refresh.
    path: String,
    /// The folded line of a fact with children: its title or name, or a count.
    preview: String,
    children: Vec<Fact>,
}

impl Peek {
    pub fn new(command: String, inspection: crate::logs::stop::Inspection, csrf: String) -> Self {
        let value = inspection.details;
        let issue = serde_json::from_value::<crate::logs::Issue>(value["issue"].clone()).ok();
        let (subject, title) = crate::logs::status::subject(
            value["repository"].as_str().unwrap_or_default(),
            value["name"]
                .as_str()
                .or_else(|| value["kind"].as_str())
                .unwrap_or_default(),
            issue.as_ref(),
        );
        let issue_url = crate::logs::status::issue_url(issue.as_ref());
        let experiment_url = (value["kind"] == "Experiment")
            .then(|| value["experiment"]["id"].as_str())
            .flatten()
            .map(|id| format!("/experiments/{id}"));
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
            experiment_url,
            subject,
            title,
            command,
            cli,
            facts: Fact::children(value, false, ""),
            logs,
        }
    }
    fn fact<'a>(&self, fact: &'a Fact) -> FactTemplate<'a> {
        FactTemplate { fact }
    }
}

impl Fact {
    fn new(label: String, value: Value, in_metrics: bool, parent: &str) -> Self {
        let in_metrics = in_metrics || label == "run_metrics";
        let label = label.replace('_', " ");
        let path = if parent.is_empty() {
            label.clone()
        } else {
            format!("{parent}/{label}")
        };
        let preview = match &value {
            Value::Object(fields) => ["title", "name", "outcome", "state"]
                .iter()
                .find_map(|key| fields.get(*key).and_then(Value::as_str))
                .map(|text| crate::logs::display_name(text).to_owned())
                .unwrap_or_else(|| count(fields.len(), "field")),
            Value::Array(items) => count(items.len(), "item"),
            _ => String::new(),
        };
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
        let children = Self::children(value, in_metrics, &path);
        Self {
            label,
            value: text,
            path,
            preview,
            children,
        }
    }
    fn children(value: Value, in_metrics: bool, parent: &str) -> Vec<Self> {
        match value {
            Value::Object(fields) => fields
                .into_iter()
                .map(|(label, value)| Self::new(label, value, in_metrics, parent))
                .collect(),
            Value::Array(items) => items
                .into_iter()
                .enumerate()
                .map(|(index, value)| Self::new((index + 1).to_string(), value, in_metrics, parent))
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl Fact {
    /// A value too long for its one line, which opens to show it whole.
    /// An absolute path, shown on one line with its end kept and a copy button.
    fn is_file(&self) -> bool {
        self.value.starts_with('/') && !self.value.contains(char::is_whitespace)
    }
    fn long(&self) -> bool {
        self.value.chars().count() > 60
    }
}

fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_heading_precedes_stop_and_dismiss() {
        let inspection = crate::logs::stop::Inspection {
            details: json!({
                "repository": "acme/widgets", "name": "acme/widgets#7", "state": "live",
                "issue": {"number": 7, "url": "https://github.com/acme/widgets/issues/7",
                    "title": "Widgets <script>alert(1)</script>"}
            }),
            dismissal: Some(crate::logs::dismiss::Action::Dismiss),
            can_stop: true,
            stopping: false,
            branches: vec![],
            pull_requests: vec![],
        };
        let html = Peek::new("widgets#7".into(), inspection, "csrf".into())
            .render()
            .unwrap();
        let heading = html
            .split("<h2")
            .nth(1)
            .unwrap()
            .split("</h2>")
            .next()
            .unwrap();
        assert!(
            heading.contains(
                "<a href=\"https://github.com/acme/widgets/issues/7\">acme/widgets#7</a>"
            ),
            "{heading}"
        );
        assert!(
            heading.contains("Widgets &#60;script&#62;alert(1)&#60;/script&#62;"),
            "{heading}"
        );
        assert!(html.find("</h2>").unwrap() < html.find(">Stop</button>").unwrap());
        assert!(html.find("</h2>").unwrap() < html.find(">Dismiss</button>").unwrap());
        assert!(html.contains("aria-label=\"Command details\""));
        assert!(!html.contains(">Issue #7</a>"));
    }

    fn details(details: Value) -> String {
        let inspection = crate::logs::stop::Inspection {
            details,
            dismissal: None,
            can_stop: false,
            stopping: false,
            branches: vec![],
            pull_requests: vec![],
        };
        Peek::new("widgets#7".into(), inspection, "csrf".into())
            .render()
            .unwrap()
    }

    #[test]
    fn experiment_record_links_its_experiment_page() {
        let html = details(json!({
            "kind": "Experiment", "outcome": "fail",
            "experiment": {"id": "exp-1<b>"}
        }));
        assert!(
            html.contains("<a href=\"/experiments/exp-1&#60;b&#62;\">Experiment</a>"),
            "{html}"
        );
        assert!(
            html.find(">Full Command log</a>").unwrap() < html.find(">Experiment</a>").unwrap()
        );
    }

    #[test]
    fn other_records_carry_no_experiment_link() {
        let html = details(json!({
            "kind": "Run", "experiment": {"id": "exp-1"}
        }));
        assert!(!html.contains("/experiments/"), "{html}");
    }

    #[test]
    fn experiment_record_without_an_id_carries_no_experiment_link() {
        let html = details(json!({"kind": "Experiment"}));
        assert!(!html.contains("/experiments/"), "{html}");
    }
}
