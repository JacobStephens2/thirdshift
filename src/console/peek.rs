//! Render every show field, including nested Runs, without a second schema.
use askama::Template;
use serde_json::Value;

use super::filters;

#[derive(Template)]
#[template(path = "console/peek.html")]
pub(super) struct Peek {
    pub account: Option<String>,
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
    pub advisories: Vec<Advisory>,
    pub issue_url: Option<String>,
    /// The Experiment page of an Experiment record that names its id.
    pub experiment_url: Option<String>,
    pub subject: String,
    pub title: String,
    pub failure_reason: Option<String>,
    pub pause_reason: Option<String>,
    pub resume_command: Option<String>,
    pub fix_command: Option<String>,
    pub output: Option<String>,
    pub log_url: String,
    pub waterfall_url: String,
    pub owning_console: Option<String>,
}

pub(super) struct Log {
    pub name: String,
    pub url: String,
}

pub(super) struct Advisory {
    label: &'static str,
    finding: crate::security::RecordedFinding,
}

impl Advisory {
    fn of(record: &Value, label: &'static str) -> Vec<Self> {
        serde_json::from_value::<Vec<crate::security::RecordedFinding>>(
            record["security_fix_findings"].clone(),
        )
        .unwrap_or_default()
        .into_iter()
        .map(|finding| Self { label, finding })
        .collect()
    }
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
    pub fn new(
        command: String,
        inspection: crate::logs::stop::Inspection,
        csrf: String,
        config: &crate::config::UserConfig,
    ) -> Self {
        let output = inspection.details["command_log"]
            .as_str()
            .and_then(|path| super::logs::preview(config, std::path::Path::new(path)).ok());
        let steps = step_facts(
            &inspection.details,
            inspection.accepted_log_directory.as_deref(),
        );
        let mut peek = Self::from_details(command.clone(), inspection.details);
        peek.output = output;
        peek.facts.extend(steps);
        peek.fix_command = inspection.fix_command;
        peek.dismissal = inspection.dismissal.map(|action| {
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
        });
        peek.csrf = csrf;
        peek.can_stop = inspection.can_stop;
        peek.stopping = inspection.stopping;
        peek.branches = inspection.branches;
        peek.pull_requests = inspection.pull_requests;
        peek
    }

    pub fn remote(
        command: String,
        value: Value,
        output: Option<String>,
        remote: &super::fleet::read::Remote,
    ) -> Self {
        // This constructor only renders values. In particular, none of the
        // owning machine's absolute paths are opened on this machine.
        let mut peek = Self::from_details(command.clone(), value);
        peek.output = output.map(|text| super::logs::recent_output(text.as_bytes()));
        peek.log_url = remote.url(&peek.log_url);
        peek.waterfall_url = remote.url(&peek.waterfall_url);
        for log in &mut peek.logs {
            log.url = remote.url(&log.url);
        }
        peek.cli.push_str(&format!(
            " --machine '{}'",
            remote.entry.name.replace('\'', "'\\''")
        ));
        peek.owning_console = Some(super::fleet::read::owning_url(
            &remote.entry,
            &crate::logs::status::inspect::url("/peek", &command, None),
        ));
        // Experiment pages, recovery commands and actions belong to their
        // own workflows; remote details offer the owning Console for them.
        peek.experiment_url = None;
        peek.resume_command = None;
        peek
    }

    fn from_details(command: String, value: Value) -> Self {
        let issue = serde_json::from_value::<crate::logs::Issue>(value["issue"].clone()).ok();
        let name = value["name"]
            .as_str()
            .or_else(|| value["kind"].as_str())
            .unwrap_or_default();
        let process =
            (value["kind"] == "Process").then(|| value["process_name"].as_str().unwrap_or(name));
        let (subject, title) = crate::logs::status::subject(
            value["repository"].as_str().unwrap_or_default(),
            name,
            issue.as_ref(),
            process,
        );
        let issue_url = crate::logs::status::issue_url(issue.as_ref());
        let experiment_url = (value["kind"] == "Experiment")
            .then(|| value["experiment"]["id"].as_str())
            .flatten()
            .map(|id| format!("/experiments/{id}"));
        let mut advisories = Advisory::of(&value, "Fixes advisory");
        if let Some(dispatched) = value["dispatched"].as_array() {
            for run in dispatched {
                advisories.extend(Advisory::of(run, "Fix dispatched for advisory"));
            }
        }
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
            output: None,
            log_url: crate::logs::status::inspect::url("/log", &command, None),
            waterfall_url: crate::logs::status::inspect::url("/waterfall", &command, None),
            owning_console: None,
            fix_command: None,
            failure_reason: failure_reason(&value),
            pause_reason: (value["outcome"] == "paused")
                .then(|| value["cause"].as_str().map(str::to_owned))
                .flatten(),
            resume_command: (value["outcome"] == "paused")
                .then(|| value["resume_command"].as_str().map(str::to_owned))
                .flatten(),
            account: value["account"].as_str().map(str::to_owned),
            dismissal: None,
            csrf: String::new(),
            live: value["state"] == "live",
            stop_cli: format!("thirdshift stop '{}'", command.replace('\'', "'\\''")),
            can_stop: false,
            stopping: false,
            branches: Vec::new(),
            pull_requests: Vec::new(),
            advisories,
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

fn failure_reason(value: &Value) -> Option<String> {
    if value["state"] != "ended" || !unsuccessful(value) {
        return None;
    }
    std::iter::once(value)
        .chain(value["dispatched"].as_array().into_iter().flatten())
        .filter(|run| unsuccessful(run))
        .find_map(|run| {
            run["cause"]
                .as_str()
                .filter(|cause| !cause.trim().is_empty())
        })
        .map(str::to_owned)
}

fn unsuccessful(value: &Value) -> bool {
    value["outcome"].as_str().is_some_and(|outcome| {
        outcome.contains("failed")
            || matches!(
                outcome,
                "interrupted" | "review required" | "fail" | "inconclusive" | "waiting_on_rechecks"
            )
    })
}

fn step_facts(value: &Value, accepted: Option<&std::path::Path>) -> Vec<Fact> {
    if value.get("kind").and_then(Value::as_str) != Some("Process") {
        return Vec::new();
    }
    let Some(log) = value.get("command_log").and_then(Value::as_str) else {
        return Vec::new();
    };
    crate::logs::status::inspect::process_steps(std::path::Path::new(log), accepted)
        .into_iter()
        .enumerate()
        .map(|(index, step)| {
            let path = format!("process steps/{}", index + 1);
            Fact {
                label: "step".into(),
                preview: step.message.clone(),
                value: step.message,
                children: step
                    .result
                    .into_iter()
                    .map(|result| {
                        Fact::new(
                            "result".into(),
                            Value::String(result.display().to_string()),
                            false,
                            &path,
                        )
                    })
                    .collect(),
                path,
            }
        })
        .collect()
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
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "only objects and arrays have a preview; every scalar JSON value has none"
        )]
        let preview = match &value {
            Value::Object(fields) => ["title", "name", "outcome", "state"]
                .iter()
                .find_map(|key| fields.get(*key).and_then(Value::as_str))
                .map(|text| crate::logs::display_name(text).to_owned())
                .unwrap_or_else(|| count(fields.len(), "field")),
            Value::Array(items) => count(items.len(), "item"),
            _ => String::new(),
        };
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "the JSON scalars without their own wording show as JSON shows them"
        )]
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
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "only objects and arrays have children; every scalar JSON value has none"
        )]
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

    fn config() -> crate::config::UserConfig {
        crate::config::UserConfig::parse(
            "",
            std::path::Path::new("config.toml"),
            std::path::Path::new("."),
        )
        .unwrap()
    }

    #[test]
    fn command_heading_precedes_stop_and_dismiss() {
        let inspection = crate::logs::stop::Inspection {
            security_details: false,
            details: json!({
                "repository": "acme/widgets", "name": "acme/widgets#7", "state": "live",
                "issue": {"number": 7, "url": "https://github.com/acme/widgets/issues/7",
                    "title": "Widgets <script>alert(1)</script>"}
            }),
            fix_command: None,
            dismissal: Some(crate::logs::dismiss::Action::Dismiss),
            can_stop: true,
            stopping: false,
            branches: vec![],
            pull_requests: vec![],
            accepted_log_directory: None,
        };
        let html = Peek::new("widgets#7".into(), inspection, "csrf".into(), &config())
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

    fn render_peek(details: Value) -> String {
        let inspection = crate::logs::stop::Inspection {
            security_details: false,
            details,
            fix_command: None,
            dismissal: None,
            can_stop: false,
            stopping: false,
            branches: vec![],
            pull_requests: vec![],
            accepted_log_directory: None,
        };
        Peek::new("widgets#7".into(), inspection, "csrf".into(), &config())
            .render()
            .unwrap()
    }

    #[test]
    fn failed_command_shows_its_full_cause_right_after_the_heading() {
        let cause = "codex exited 1: Selected model is at capacity. Please try a different model.";
        let html = render_peek(json!({
            "kind": "Pickup run", "state": "ended", "outcome": "failed", "cause": cause
        }));
        let after_heading = html.split_once("</h2>").unwrap().1.trim_start();
        assert!(
            after_heading.starts_with(&format!("<p class=\"peek-failure\">Failed: {cause}</p>")),
            "{html}"
        );
        // The generic cause fact still folds independently of the upfront reason.
        assert!(html.contains("<details data-fact=\"cause\">"), "{html}");
    }

    #[test]
    fn failed_command_without_a_cause_shows_the_first_failed_dispatched_runs_cause() {
        for cause in [Value::Null, json!(""), json!("  ")] {
            let html = render_peek(json!({
                "kind": "Pickup run", "state": "ended", "outcome": "failed", "cause": cause,
                "dispatched": [
                    {"outcome": "merged", "cause": "Successful Run"},
                    {"outcome": "failed", "cause": ""},
                    {"outcome": "failed", "cause": "First dispatch failed"},
                    {"outcome": "failed", "cause": "Second dispatch failed"}
                ]
            }));
            assert!(
                html.split_once("</h2>")
                    .unwrap()
                    .1
                    .trim_start()
                    .starts_with("<p class=\"peek-failure\">Failed: First dispatch failed</p>"),
                "{html}"
            );
        }
    }

    #[test]
    fn unsuccessful_command_outcomes_show_the_reason() {
        for outcome in [
            "failed",
            "review failed",
            "audit failed",
            "interrupted",
            "review required",
            "fail",
            "inconclusive",
            "waiting_on_rechecks",
        ] {
            let html = render_peek(json!({
                "state": "ended", "outcome": outcome, "cause": "Command could not finish"
            }));
            assert!(
                html.contains("<p class=\"peek-failure\">Failed: Command could not finish</p>"),
                "{outcome}: {html}"
            );
        }
    }

    #[test]
    fn successful_and_live_commands_show_no_failure_reason_even_with_a_cause() {
        for outcome in [
            "merged",
            "ready for review",
            "succeeded",
            "pass",
            "plan published",
            "idea filed",
            "idea already filed",
            "findings recorded",
            "sampled",
            "skipped",
        ] {
            let html = render_peek(json!({
                "state": "ended", "outcome": outcome, "cause": "Not a failure reason",
                "dispatched": [{"outcome": "failed", "cause": "Earlier dispatch failure"}]
            }));
            assert!(
                !html.contains("class=\"peek-failure\""),
                "{outcome}: {html}"
            );
        }
        let html = render_peek(json!({
            "state": "live", "outcome": "failed", "cause": "Still live",
            "dispatched": [{"outcome": "failed", "cause": "Earlier dispatch failure"}]
        }));
        assert!(!html.contains("class=\"peek-failure\""), "{html}");
    }

    #[test]
    fn commands_without_a_cause_anywhere_show_no_failure_reason() {
        for cause in [Value::Null, json!(""), json!("  ")] {
            let html = render_peek(json!({
                "state": "ended", "outcome": "failed", "cause": cause,
                "dispatched": [{"outcome": "failed", "cause": cause}]
            }));
            assert!(!html.contains("class=\"peek-failure\""), "{html}");
        }
    }

    #[test]
    fn command_failure_reason_prefers_its_own_cause_and_links_and_escapes_it() {
        let html = render_peek(json!({
            "state": "ended", "outcome": "failed",
            "cause": "See https://example.test/status <script>alert(1)</script>",
            "dispatched": [{"outcome": "failed", "cause": "Dispatch failure"}]
        }));
        assert!(
            html.split_once("</h2>").unwrap().1.trim_start().starts_with(
                "<p class=\"peek-failure\">Failed: See <a href=\"https://example.test/status\">https://example.test/status</a> &#60;script&#62;alert(1)&#60;/script&#62;</p>"
            ),
            "{html}"
        );
        assert!(!html.contains("<script>"), "{html}");
    }

    #[test]
    fn experiment_record_links_its_experiment_page() {
        let html = render_peek(json!({
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
        let html = render_peek(json!({
            "kind": "Run", "experiment": {"id": "exp-1"}
        }));
        assert!(!html.contains("/experiments/"), "{html}");
    }

    #[test]
    fn experiment_record_without_an_id_carries_no_experiment_link() {
        let html = render_peek(json!({"kind": "Experiment"}));
        assert!(!html.contains("/experiments/"), "{html}");
    }
}
