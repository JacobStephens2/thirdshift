//! Kimi Code's session record, which holds the usage its stream leaves out.
//! Each agent of a session appends `wire.jsonl` under
//! `<KIMI_CODE_HOME>/sessions/<workspace>/<session>/agents/<agent>/`; every
//! `step.end` loop event there carries that model request's usage, split into
//! uncached input, output, cache reads and cache writes. Kimi's own `vis`
//! sums the same events. A record with none leaves usage missing, not zero.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::harness::interpretation::event::{Event, Outcome, Usage, UsageScope};
use crate::harness::interpretation::{Facts, Retained, RetainedRecord};

/// The session's usage by Model, in first-seen order, with the events and
/// raw `step.end` lines they came from.
#[derive(Default)]
pub(super) struct Wire {
    pub records: Vec<RetainedRecord>,
    pub totals: Vec<(Option<String>, Usage)>,
}

impl Wire {
    /// Read every agent's record of session `id` under Kimi's `home`, or
    /// none when the session left no record there.
    pub fn read(home: &Path, id: &str) -> Option<Self> {
        let dir = session_dir(home, id)?;
        let mut agents: Vec<PathBuf> = fs::read_dir(dir.join("agents"))
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        // The main agent first, then its subagents in a stable order.
        agents.sort_by_key(|agent| (!agent.ends_with("main"), agent.clone()));
        let mut wire = Self::default();
        for agent in agents {
            let Ok(text) = fs::read_to_string(agent.join("wire.jsonl")) else {
                continue;
            };
            let name = agent.file_name().unwrap_or_default().to_string_lossy();
            wire.add(id, &name, &text);
        }
        Some(wire)
    }

    /// Add one agent's `wire.jsonl`. A `step.end` counts for the Model
    /// alias its agent was last bound to.
    pub fn add(&mut self, session: &str, agent: &str, text: &str) {
        let mut model: Option<String> = None;
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let Some(alias) = record["modelAlias"]
                .as_str()
                .filter(|alias| !alias.is_empty())
            {
                model = Some(alias.to_string());
            }
            let event = &record["event"];
            if record["type"] != "context.append_loop_event"
                || event["type"] != "step.end"
                || !event["usage"].is_object()
            {
                continue;
            }
            let usage = usage(&event["usage"]);
            let step = event["uuid"]
                .as_str()
                .map(String::from)
                .unwrap_or_else(|| format!("step:{}", self.records.len() + 1));
            let id = format!("{session}:{agent}:{step}");
            let outcome = match event["finishReason"].as_str() {
                Some("interrupted") => Outcome::Interrupted,
                _ => Outcome::Succeeded,
            };
            self.records.push(RetainedRecord {
                raw: line.to_string(),
                events: vec![
                    Event::ModelCallStarted {
                        id: id.clone(),
                        parent_id: None,
                        model: model.clone(),
                    },
                    Event::ModelCallEnded {
                        id,
                        parent_id: None,
                        model: model.clone(),
                        usage: usage.clone(),
                        cost_usd: None,
                        outcome,
                    },
                ],
            });
            match self.totals.iter_mut().find(|(seen, _)| *seen == model) {
                Some((_, total)) => add(total, &usage),
                None => self.totals.push((model.clone(), usage)),
            }
        }
    }

    /// Each Model's total, as the Session's usage evidence.
    pub fn events(&self) -> Vec<Event> {
        let mut events: Vec<Event> = self
            .records
            .iter()
            .flat_map(|record| record.events.clone())
            .collect();
        events.extend(self.totals.iter().map(|(model, usage)| Event::Usage {
            scope: UsageScope::Model,
            model: model.clone(),
            usage: usage.clone(),
            cost_usd: None,
        }));
        events
    }

    /// The whole session's usage in one line, or none with no usage.
    pub fn summary(&self) -> Option<String> {
        let mut total = None::<Usage>;
        for (_, usage) in &self.totals {
            add(total.get_or_insert_default(), usage);
        }
        let total = total?;
        Some(format!(
            "{} input tokens ({} cache read, {} cache write), {} output tokens",
            total.input.unwrap_or(0),
            total.cache_read.unwrap_or(0),
            total.cache_write.unwrap_or(0),
            total.output.unwrap_or(0)
        ))
    }
}

/// `<home>/sessions/<workspace>/<id>`, whichever workspace holds it.
fn session_dir(home: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() || id.contains(['/', '\\']) || id.starts_with('.') {
        return None;
    }
    fs::read_dir(home.join("sessions"))
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path().join(id)))
        .find(|dir| dir.is_dir())
}

/// Input includes cache reads and writes, as thirdshift's usage fields do;
/// Kimi's `inputOther` is the uncached part alone.
fn usage(usage: &Value) -> Usage {
    let read = usage["inputCacheRead"].as_u64();
    let write = usage["inputCacheCreation"].as_u64();
    Usage {
        input: usage["inputOther"]
            .as_u64()
            .map(|other| other + read.unwrap_or(0) + write.unwrap_or(0)),
        output: usage["output"].as_u64(),
        cache_read: read,
        cache_write: write,
    }
}

fn add(total: &mut Usage, usage: &Usage) {
    for (total, count) in [
        (&mut total.input, usage.input),
        (&mut total.output, usage.output),
        (&mut total.cache_read, usage.cache_read),
        (&mut total.cache_write, usage.cache_write),
    ] {
        if let Some(count) = count {
            *total = Some(total.unwrap_or(0) + count);
        }
    }
}

/// Reconcile a finished session with its record: the stream's session id
/// names it, and the launched process's home locates it.
pub(super) struct Completion {
    home: Option<PathBuf>,
}

impl Completion {
    pub(super) fn new() -> Self {
        Self {
            home: super::home(None),
        }
    }
}

impl Retained for Completion {
    fn set_home(&mut self, home: &Path) {
        self.home = super::home(Some(home));
    }

    fn reconcile(self: Box<Self>, facts: &mut Facts) -> Result<()> {
        if let (Some(id), Some(home)) = (facts.ended.session_id.as_deref(), &self.home)
            && let Some(wire) = Wire::read(home, id)
        {
            facts.report.summary = wire.summary();
            for (model, _) in &wire.totals {
                if let Some(model) = model
                    && !facts.report.models.contains(model)
                {
                    facts.report.models.push(model.clone());
                }
            }
            crate::harness::interpretation::event::reconcile(
                &mut facts.report.events,
                wire.events(),
            );
            facts.report.retained_records.extend(wire.records);
        }
        facts.report.set_model_progress();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORDED: &str = include_str!("../../../tests/fixtures/kimi-wire.jsonl");

    #[test]
    fn usage_is_the_total_of_every_step_end_with_cache_reads_and_writes_kept_apart() {
        let mut wire = Wire::default();
        wire.add("s1", "main", RECORDED);
        assert_eq!(
            wire.totals,
            [(
                Some("kimi-k3.1-pro".to_string()),
                Usage {
                    input: Some(4200 + 2048 + 512 + 300 + 6144),
                    output: Some(180 + 95),
                    cache_read: Some(2048 + 6144),
                    cache_write: Some(512),
                }
            )]
        );
        assert_eq!(
            wire.summary().as_deref(),
            Some("13204 input tokens (8192 cache read, 512 cache write), 275 output tokens")
        );
        assert_eq!(wire.records.len(), 2);
        assert!(matches!(
            &wire.records[1].events[1],
            Event::ModelCallEnded { id, model: Some(model), .. }
                if id == "s1:main:step-2" && model == "kimi-k3.1-pro"
        ));
    }

    #[test]
    fn a_record_without_step_end_usage_leaves_usage_missing() {
        let mut wire = Wire::default();
        wire.add(
            "s1",
            "main",
            r#"{"type":"context.append_loop_event","agentId":"main","event":{"type":"step.end","uuid":"a","turnId":"1","step":1,"finishReason":"interrupted"}}
{"type":"usage.record","agentId":"main","model":"kimi-k3.1-pro","usage":{"inputOther":5,"output":1,"inputCacheCreation":0}}"#,
        );
        assert!(wire.totals.is_empty());
        assert_eq!(wire.summary(), None);
        assert!(wire.events().is_empty());
    }

    #[test]
    fn every_agent_of_the_session_counts_under_its_own_model() {
        let home = tempfile::tempdir().unwrap();
        let session = home.path().join("sessions/ws-1/s1/agents");
        for (agent, alias, input) in [
            ("main", "kimi-k3.1-pro", 10),
            ("sub-1", "kimi-k3.1-flash", 7),
        ] {
            fs::create_dir_all(session.join(agent)).unwrap();
            fs::write(
                session.join(agent).join("wire.jsonl"),
                format!(
                    "{{\"type\":\"profile.bind\",\"agentId\":\"{agent}\",\"modelAlias\":\"{alias}\"}}\n\
                     {{\"type\":\"context.append_loop_event\",\"agentId\":\"{agent}\",\"event\":{{\"type\":\"step.end\",\"uuid\":\"u\",\"usage\":{{\"inputOther\":{input},\"output\":1,\"inputCacheCreation\":0}}}}}}\n"
                ),
            )
            .unwrap();
        }
        let wire = Wire::read(home.path(), "s1").unwrap();
        assert_eq!(
            wire.totals
                .iter()
                .map(|(model, usage)| (model.as_deref().unwrap(), usage.input.unwrap()))
                .collect::<Vec<_>>(),
            [("kimi-k3.1-pro", 10), ("kimi-k3.1-flash", 7)]
        );
        assert!(Wire::read(home.path(), "missing").is_none());
        assert!(Wire::read(home.path(), "../ws-1/s1").is_none());
    }
}
