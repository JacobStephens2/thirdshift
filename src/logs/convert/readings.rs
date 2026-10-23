//! Exercise the same reader functions as the Console, Waterfall and show.
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local};
use serde_json::{Value, json};

use crate::harness::Harness;
use crate::logs::reading::Command;
use crate::logs::status::waterfall::{self, PlacedCall, matching};
use crate::logs::{catalog, owner, session};

pub(super) struct Readings {
    pub conversation: String,
    waterfall: Vec<PlacedCall>,
    pub(super) show: Option<String>,
    model_evidence: Value,
}

impl Readings {
    pub(super) fn compare(&self, other: &Self) -> BTreeMap<String, bool> {
        BTreeMap::from([
            (
                "conversation".into(),
                self.conversation == other.conversation,
            ),
            ("waterfall".into(), self.waterfall == other.waterfall),
            ("show".into(), self.show == other.show),
            (
                "model_evidence".into(),
                self.model_evidence == other.model_evidence,
            ),
        ])
    }
}

pub(super) fn read(
    path: &Path,
    identity: &Path,
    command: &Command,
    relays: &[u64],
    harness: Harness,
    models: &Value,
) -> Result<Readings> {
    let text = fs::read_to_string(path)?;
    let start = DateTime::parse_from_rfc3339(&command.record.started_at)?;
    let mut log = matching::Log::read(&command.command_log, start)?;
    let mut stream = harness.adapter().interpretation(Path::new(""), "");
    for raw in session::events(&text) {
        stream.condense(&raw);
    }
    let (calls, measured) = waterfall::session_calls(path, command, relays, &mut log)?;
    // `show` in use reads only a live Command's latest event. The round-trip
    // check measures what `show` would report for the Session, over the whole
    // Session, without depending on the live 64 KiB tail window. The live tail
    // behavior itself is unchanged.
    let show_events: Vec<String> = session::events(&text).collect();
    Ok(Readings {
        conversation: crate::console::conversation::render(&text, catalog::Kind::Session),
        waterfall: waterfall::place_calls(
            identity,
            command,
            &calls,
            measured,
            Local::now().fixed_offset(),
        ),
        show: crate::logs::reading::latest_session_event(&show_events, harness),
        model_evidence: json!({"sidecar":models,"stream_models":stream.observed_models()}),
    })
}

pub(super) fn nanos(time: DateTime<FixedOffset>) -> u64 {
    time.timestamp_nanos_opt()
        .and_then(|n| n.try_into().ok())
        .unwrap_or_default()
}

pub(super) fn historical_command(
    source: &Path,
    harness: Harness,
    models: &Value,
) -> Result<Command> {
    let decoded = owner::decode(source).context("invalid Session name")?;
    let command_log = models["command_log"]
        .as_str()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| decoded.command_log(source));
    let record = serde_json::from_value(json!({
        "version":1,"kind":"Historical Session","repository":"","name":decoded.subject,
        "issue":null,"hostname":"","process_id":0,"process_start":"",
        "harness":harness.name(),"model":null,"effort":null,
        "started_at":decoded.time().to_rfc3339(),"ended_at":null,"stages":[],
        "session_log":source,"outcome":null,"cause":null,"issue_branch":null,"pull_request":null
    }))?;
    let mut command = Command::new(
        record,
        command_log.with_extension("command.json"),
        Local::now(),
    );
    command.command_log = command_log;
    Ok(command)
}

/// Prefer durable Harness evidence. For very old logs lacking it, recognize
/// protocol envelopes rather than assuming that every Session ran on Claude.
pub(super) fn harness(text: &str) -> Option<Harness> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|event| {
            if event["type"] == "thread.started"
                || event["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("item."))
            {
                Some(Harness::Codex)
            } else if event["type"] == "system" || event["type"] == "assistant" {
                Some(Harness::Claude)
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readings(show: Option<&str>) -> Readings {
        Readings {
            conversation: String::new(),
            waterfall: Vec::new(),
            show: show.map(str::to_owned),
            model_evidence: Value::Null,
        }
    }

    #[test]
    fn compare_still_catches_a_changed_latest_event() {
        let readers =
            readings(Some("Bash: cargo test")).compare(&readings(Some("Bash: cargo clippy")));
        assert!(!readers["show"]);
        let readers =
            readings(Some("Bash: cargo test")).compare(&readings(Some("Bash: cargo test")));
        assert!(readers["show"]);
    }
}
