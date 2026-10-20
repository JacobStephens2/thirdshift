//! Render every show field, including nested Runs, without a second schema.
use askama::Template;
use serde_json::Value;

#[derive(Template)]
#[template(path = "console/peek.html")]
pub(super) struct Peek {
    pub command: String,
    pub cli: String,
    pub facts: Vec<Fact>,
    pub logs: Vec<Log>,
}

pub(super) struct Log {
    pub name: String,
    pub url: String,
}
pub(super) struct Fact {
    label: String,
    value: String,
    children: Vec<Fact>,
}

impl Peek {
    pub fn new(command: String, value: Value) -> Self {
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
            command,
            cli,
            facts: Fact::children(value),
            logs,
        }
    }
    fn fact<'a>(&self, fact: &'a Fact) -> FactTemplate<'a> {
        FactTemplate { fact }
    }
}

impl Fact {
    fn new(label: String, value: Value) -> Self {
        let text = match &value {
            Value::Null => "None".into(),
            Value::String(text) => text.clone(),
            Value::Array(items) if items.is_empty() => "None".into(),
            Value::Object(items) if items.is_empty() => "None".into(),
            Value::Array(_) | Value::Object(_) => String::new(),
            other => other.to_string(),
        };
        Self {
            label: label.replace('_', " "),
            value: text,
            children: Self::children(value),
        }
    }
    fn children(value: Value) -> Vec<Self> {
        match value {
            Value::Object(fields) => fields
                .into_iter()
                .map(|(label, value)| Self::new(label, value))
                .collect(),
            Value::Array(items) => items
                .into_iter()
                .enumerate()
                .map(|(index, value)| Self::new((index + 1).to_string(), value))
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
