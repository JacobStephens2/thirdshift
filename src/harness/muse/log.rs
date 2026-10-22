//! Read Muse's retained session records for the last reply and token usage.

use crate::harness::interpretation::{Facts, Retained};
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(in crate::harness) struct SessionLog {
    raw: Vec<String>,
    pub(in crate::harness) events: Vec<crate::harness::interpretation::event::Event>,
    pub(in crate::harness) message: Option<String>,
    pub(in crate::harness) models: Vec<String>,
    tokens: Option<Tokens>,
    decoder: super::events::Events,
}

#[derive(Default)]
struct Tokens {
    input: u64,
    cached: u64,
    output: u64,
}

impl SessionLog {
    pub fn parse(text: &str, session_id: &str) -> Result<Self> {
        let mut log = Self::default();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let record: Value = serde_json::from_str(line)?;
            if let Some(children) = record["children"].as_array() {
                for child in children {
                    let raw = child["record_json"].as_str().unwrap_or("");
                    let record: Value = serde_json::from_str(raw)?;
                    log.record(&record, raw, session_id);
                }
            } else {
                log.record(&record, line, session_id);
            }
        }
        Ok(log)
    }
    fn record(&mut self, record: &Value, raw: &str, session_id: &str) {
        if record["stream"]["id"].as_str() != Some(session_id) {
            return;
        }
        self.raw.push(raw.into());
        if matches!(record["payload"]["kind"].as_str(), Some("run" | "task")) {
            crate::harness::interpretation::event::reconcile(
                &mut self.events,
                self.decoder.record(record),
            );
        }
        if record["payload"]["kind"] != "run" {
            return;
        }
        let event = &record["payload"]["event"];
        match event["kind"].as_str() {
            Some("assistant_message_committed") => {
                self.message = event["text"].as_str().map(String::from);
            }
            Some("model_completed") => {
                if let Some(model) = event["model"].as_str().filter(|model| !model.is_empty())
                    && self.models.last().map(String::as_str) != Some(model)
                {
                    self.models.push(model.to_string());
                }
                if let Some(usage) = event["usage"].as_object() {
                    let tokens = self.tokens.get_or_insert_default();
                    tokens.input += usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    tokens.cached += usage
                        .get("cached_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    tokens.output += usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                }
            }
            _ => {}
        }
    }
    pub(in crate::harness) fn summary(&self) -> Option<String> {
        self.tokens.as_ref().map(|tokens| {
            format!(
                "{} input tokens ({} cached), {} output tokens",
                tokens.input, tokens.cached, tokens.output
            )
        })
    }
}

/// Date folders are year/month/day; never descend into sub-agent logs.
fn find_log(root: &Path, id: &str, depth: usize) -> Option<PathBuf> {
    let log = root.join(id).join("session.jsonl");
    if log.is_file() {
        return Some(log);
    }
    if depth == 0 {
        return None;
    }
    fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .find_map(|entry| find_log(&entry.path(), id, depth - 1))
}

pub(in crate::harness) fn read(root: &Path, id: &str) -> Option<SessionLog> {
    if id.contains(['/', '\\']) || id == ".." {
        return None;
    }
    let path = find_log(&root.join("sessions"), id, 3)?;
    SessionLog::parse(&fs::read_to_string(path).ok()?, id).ok()
}

/// Muse's readable records own usage and Models, but text only when present.
pub(super) struct Completion {
    root: Option<PathBuf>,
}

impl Completion {
    pub(super) fn new(root: Option<PathBuf>) -> Self {
        Self { root }
    }
}

impl Retained for Completion {
    fn reconcile(self: Box<Self>, facts: &mut Facts) -> Result<()> {
        if let Some(log) = self
            .root
            .as_deref()
            .zip(facts.ended.session_id.as_deref())
            .and_then(|(root, id)| read(root, id))
        {
            facts.report.summary = log.summary();
            facts.report.retained_records = log.raw;
            if log.message.is_some() {
                facts.ended.final_message = log.message;
            }
            facts.report.models = log.models;
            crate::harness::interpretation::event::reconcile(&mut facts.report.events, log.events);
        }
        facts.report.set_model_progress();
        Ok(())
    }
}

#[cfg(test)]
pub(in crate::harness) fn recording_interpretation(
    root: &Path,
    decoder: Box<dyn crate::harness::interpretation::Decoder>,
) -> crate::harness::interpretation::Interpretation {
    crate::harness::interpretation::Interpretation::new("claude", decoder)
        .with_retained(Box::new(Completion::new(Some(root.to_path_buf()))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::interpretation::event::{Event, Outcome, Usage};
    use serde_json::json;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn retained_only_model_completions_remain_distinct_calls_without_task_envelopes() {
        let records = [10, 20].map(|input| {
            json!({
                "stream":{"kind":"session","id":"s"},
                "payload":{
                    "kind":"run","run_id":"r",
                    "event":{"kind":"model_completed","usage":{"input_tokens":input}}
                }
            })
        });
        let text = records
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let log = SessionLog::parse(&text, "s").unwrap();
        let starts: Vec<_> = log
            .events
            .iter()
            .filter_map(|event| match event {
                Event::ModelCallStarted { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let ends: Vec<_> = log
            .events
            .iter()
            .filter_map(|event| match event {
                Event::ModelCallEnded { id, usage, .. } => Some((id.as_str(), usage.input)),
                _ => None,
            })
            .collect();
        assert_eq!(starts.len(), 2);
        assert_ne!(starts[0], starts[1]);
        assert_eq!(ends.len(), 2);
        assert_eq!(ends, [(starts[0], Some(10)), (starts[1], Some(20))]);
    }

    #[test]
    fn review_muse_retained_task_envelopes_enrich_the_live_model_call() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("sessions/2026/10/06/s");
        fs::create_dir_all(&folder).unwrap();
        let record = |kind: &str, event: serde_json::Value| {
            json!({
                "stream":{"kind":"session","id":"s"},
                "payload_type":"runtime.session",
                "recorded_at":20000,
                "payload":{"kind":kind,"run_id":"r","event":event}
            })
        };
        let records = [
            record(
                "task",
                json!({"kind":"proposed","task_id":"m1","task_kind":"model.meta.response"}),
            ),
            record("task", json!({"kind":"started","task_id":"m1"})),
            record(
                "run",
                json!({"kind":"model_completed","model":"muse-spark-1.3","duration_ms":10,"usage":{"input_tokens":100,"output_tokens":12}}),
            ),
            record("task", json!({"kind":"completed","task_id":"m1"})),
        ];
        fs::write(
            folder.join("session.jsonl"),
            records
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let mut interpretation = recording_interpretation(
            root.path(),
            Box::new(super::super::stream::MuseProgress::default()),
        );
        for line in include_str!("../../../tests/fixtures/normalized-muse.jsonl").lines() {
            interpretation.decode(line);
        }
        let events = interpretation
            .finish(Ok(std::process::ExitStatus::from_raw(0)))
            .report
            .unwrap()
            .events;
        let model_ends: Vec<_> = events
            .iter()
            .filter(|event| matches!(event, Event::ModelCallEnded { .. }))
            .collect();
        assert_eq!(
            model_ends.len(),
            1,
            "one real model call must remain one call: {model_ends:?}"
        );
        assert!(
            matches!(model_ends[0], Event::ModelCallEnded { id, usage, .. }
            if id == "r:m1" && usage.input == Some(100) && usage.output == Some(12))
        );
    }

    #[test]
    fn review_muse_retained_terminal_usage_limit_is_normalized() {
        use crate::harness::interpretation::event::ErrorKind;
        let record = json!({
            "stream":{"kind":"session","id":"s"},
            "payload_type":"runtime.session",
            "recorded_at":20000,
            "payload":{
                "kind":"run","run_id":"r",
                "event":{"kind":"terminal","terminal":"failed","reason":"quota exceeded"}
            }
        });
        let log = SessionLog::parse(&record.to_string(), "s").unwrap();
        assert!(
            log.events.iter().any(|event| matches!(
                event,
                Event::Error { kind: ErrorKind::UsageLimit, message, .. } if message == "quota exceeded"
            )),
            "retained terminal error is absent: {:?}",
            log.events
        );
    }

    #[test]
    fn normalized_muse_retained_transactions_enrich_live_calls_without_duplicating_usage() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("sessions/2026/10/06/s");
        fs::create_dir_all(&folder).unwrap();
        let record = |kind: &str, event| json!({"stream":{"id":"s"},"recorded_at":4000,"payload":{"kind":kind,"run_id":"r","event":event}});
        let records = [
            record(
                "task",
                json!({"kind":"proposed","task_id":"m1","task_kind":"model.meta.response"}),
            ),
            record("task", json!({"kind":"started","task_id":"m1"})),
            record(
                "run",
                json!({"kind":"model_completed","model":"muse-spark-1.3","duration_ms":10,"usage":{"input_tokens":100,"output_tokens":12,"cached_tokens":80,"cache_write_tokens":5,"reasoning_tokens":2}}),
            ),
            record("task", json!({"kind":"completed","task_id":"m1"})),
            record(
                "run",
                json!({"kind":"assistant_message_committed","message_id":"reply","text":"retained reply"}),
            ),
            json!({"stream":{"id":"foreign"},"payload":{"kind":"run","event":{"kind":"model_completed","usage":{"input_tokens":999999}}}}),
        ];
        fs::write(folder.join("session.jsonl"), json!({"children":records.iter().map(|record| json!({"record_json":record.to_string()})).collect::<Vec<_>>()} ).to_string()).unwrap();
        let mut interpretation = recording_interpretation(
            root.path(),
            Box::new(super::super::stream::MuseProgress::default()),
        );
        for line in include_str!("../../../tests/fixtures/normalized-muse.jsonl").lines() {
            interpretation.decode(line);
        }
        let events = interpretation
            .finish(Ok(std::process::ExitStatus::from_raw(0)))
            .report
            .unwrap()
            .events;
        let ends: Vec<_> = events
            .iter()
            .filter(|event| matches!(event, Event::ModelCallEnded { .. }))
            .collect();
        assert_eq!(ends.len(), 1);
        assert_eq!(
            *ends[0],
            Event::ModelCallEnded {
                id: "r:m1".into(),
                parent_id: None,
                model: Some("muse-spark-1.3".into()),
                usage: Usage {
                    input: Some(100),
                    output: Some(12),
                    cache_read: Some(80),
                    cache_write: Some(5),
                },
                cost_usd: None,
                outcome: Outcome::Succeeded,
            }
        );
        assert!(
            events.iter().any(
                |event| matches!(event, Event::Message { text, .. } if text == "retained reply")
            )
        );
    }
}
