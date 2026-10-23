//! Weekly independent signal summaries, with explicit observation coverage.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Days, Utc};
use serde::Serialize;

use super::super::command_record::StageName;
use super::super::reading::{Command, Entry, Snapshot};
use super::super::run_metrics::{Position, StageMetrics};
use super::super::session_signals::{ModelTokens, SessionSignals};
use crate::args::MetricsArgs;
use crate::session::recording::SessionModels;

#[derive(Default, PartialEq, Eq, PartialOrd, Ord, Clone, Serialize)]
struct Group {
    week: String,
    repository: String,
    harness: Option<String>,
    harness_version: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    kind: String,
    session_kind: Option<String>,
}

#[derive(Default, Serialize)]
pub(super) struct Week {
    #[serde(flatten)]
    group: Group,
    runs: usize,
    outcomes: BTreeMap<String, usize>,
    missing_outcomes: usize,
    repairs: Signal,
    ci_rounds: Signal,
    total_minutes: Signal,
    unaddressed_findings: UnaddressedFindings,
    stages: BTreeMap<String, Stage>,
    session_signals: TraceSignals,
}

#[derive(Default, Serialize)]
struct Signal {
    observed: usize,
    missing: usize,
    sum: Option<f64>,
    mean: Option<f64>,
}

#[derive(Default, Serialize)]
struct TraceSignals {
    tokens_by_model: Vec<ModelSignals>,
    cost_usd: Signal,
    tool_errors: Signal,
    permission_denials: Signal,
    killed_work: Signal,
    repeated_tool_calls: Signal,
    #[serde(skip)]
    sessions: usize,
}

#[derive(Serialize)]
struct ModelSignals {
    model: Option<String>,
    input_tokens: Signal,
    output_tokens: Signal,
    cache_read_tokens: Signal,
    cache_write_tokens: Signal,
}

impl ModelSignals {
    fn new(model: Option<String>, missing: usize) -> Self {
        let signal = || Signal {
            missing,
            ..Default::default()
        };
        Self {
            model,
            input_tokens: signal(),
            output_tokens: signal(),
            cache_read_tokens: signal(),
            cache_write_tokens: signal(),
        }
    }

    fn add(&mut self, tokens: Option<&ModelTokens>) {
        self.input_tokens
            .add(tokens.and_then(|t| t.input_tokens).map(|n| n as f64));
        self.output_tokens
            .add(tokens.and_then(|t| t.output_tokens).map(|n| n as f64));
        self.cache_read_tokens
            .add(tokens.and_then(|t| t.cache_read_tokens).map(|n| n as f64));
        self.cache_write_tokens
            .add(tokens.and_then(|t| t.cache_write_tokens).map(|n| n as f64));
    }
}

impl TraceSignals {
    fn add(&mut self, session: Option<&SessionSignals>) {
        let signals = session.map(|s| &s.signals);
        self.cost_usd.add(signals.and_then(|s| s.cost_usd));
        self.tool_errors.add(signals.map(|s| s.tool_errors as f64));
        self.permission_denials
            .add(signals.map(|s| s.permission_denials as f64));
        self.killed_work.add(signals.map(|s| s.killed_work as f64));
        self.repeated_tool_calls
            .add(signals.map(|s| s.repeated_tool_calls as f64));
        let tokens = signals
            .map(|s| s.tokens_by_model.as_slice())
            .unwrap_or_default();
        for token in tokens {
            if !self.tokens_by_model.iter().any(|t| t.model == token.model) {
                self.tokens_by_model
                    .push(ModelSignals::new(token.model.clone(), self.sessions));
            }
        }
        for model in &mut self.tokens_by_model {
            model.add(tokens.iter().find(|t| t.model == model.model));
        }
        self.sessions += 1;
    }
}

impl Signal {
    fn add(&mut self, value: Option<f64>) {
        match value {
            Some(value) => {
                self.observed += 1;
                let sum = self.sum.get_or_insert(0.0);
                *sum += value;
                self.mean = Some(*sum / self.observed as f64);
            }
            None => self.missing += 1,
        }
    }
}

#[derive(Default, Serialize)]
struct Stage {
    seconds: Signal,
    minutes: Signal,
    usual_lower_seconds: Signal,
    usual_upper_seconds: Signal,
    positions: BTreeMap<Position, usize>,
    review_findings: Findings,
}

#[derive(Default, Serialize)]
struct Findings {
    standards: Signal,
    spec: Signal,
}

#[derive(Default, Serialize)]
struct UnaddressedFindings {
    standards: Signal,
    spec: Signal,
    security: Signal,
}

impl Stage {
    fn add(&mut self, stage: &StageMetrics) {
        self.seconds.add(stage.seconds);
        self.minutes.add(stage.minutes);
        self.usual_lower_seconds
            .add(stage.usual_range.map(|range| range.lower_seconds));
        self.usual_upper_seconds
            .add(stage.usual_range.map(|range| range.upper_seconds));
        *self.positions.entry(stage.position).or_default() += 1;
        self.review_findings.standards.add(
            stage
                .review_findings
                .as_ref()
                .and_then(|counts| counts.standards)
                .map(|n| n as f64),
        );
        self.review_findings.spec.add(
            stage
                .review_findings
                .as_ref()
                .and_then(|counts| counts.spec)
                .map(|n| n as f64),
        );
    }
}

pub(super) fn weekly(snapshot: &Snapshot, asked: &MetricsArgs) -> Vec<Week> {
    let mut weeks = BTreeMap::new();
    for entry in snapshot.entries() {
        if let Entry::Record(command) = entry {
            collect(command, asked, &mut weeks);
        }
    }
    weeks
        .into_iter()
        .map(|(group, mut week)| {
            week.group = group;
            week
        })
        .collect()
}

fn collect(command: &Command, asked: &MetricsArgs, weeks: &mut BTreeMap<Group, Week>) {
    // Traverse ownership once, even when the parent falls outside the filters.
    for child in command.dispatched.iter().chain(&command.children) {
        collect(child, asked, weeks);
    }
    let record = &command.record;
    let Ok(start) = DateTime::parse_from_rfc3339(&record.started_at) else {
        return;
    };
    let date = start.with_timezone(&Utc).date_naive();
    let Some(week) = date.checked_sub_days(Days::new(date.weekday().num_days_from_monday().into()))
    else {
        return;
    };
    if asked.from.is_some_and(|from| week < from)
        || asked.through.is_some_and(|through| week > through)
        || (!asked.repositories.is_empty() && !asked.repositories.contains(&record.repository))
    {
        return;
    }
    let group = Group {
        week: week.to_string(),
        repository: record.repository.clone(),
        harness: Some(record.harness.clone()),
        harness_version: None,
        model: record.model.clone(),
        effort: record.effort.clone(),
        kind: record.kind.clone(),
        session_kind: None,
    };
    let row = weeks.entry(group.clone()).or_default();
    row.runs += 1;
    if let Some(outcome) = &record.outcome {
        *row.outcomes.entry(outcome.clone()).or_default() += 1;
    } else {
        row.missing_outcomes += 1;
    }
    let metrics = record.run_metrics.as_ref();
    row.repairs
        .add(metrics.map(|metrics| metrics.repairs as f64));
    row.ci_rounds
        .add(metrics.map(|metrics| metrics.ci_rounds as f64));
    row.total_minutes
        .add(metrics.and_then(|metrics| metrics.total_minutes));
    let unaddressed = metrics.and_then(|metrics| metrics.unaddressed_findings.as_ref());
    row.unaddressed_findings.standards.add(
        unaddressed
            .and_then(|findings| findings.standards)
            .map(|n| n as f64),
    );
    row.unaddressed_findings.spec.add(
        unaddressed
            .and_then(|findings| findings.spec)
            .map(|n| n as f64),
    );
    row.unaddressed_findings.security.add(
        unaddressed
            .and_then(|findings| findings.security)
            .map(|n| n as f64),
    );
    let Some(metrics) = metrics else { return };
    let mut session_kinds = BTreeSet::new();
    let mut sessions = BTreeSet::new();
    let mut security_session = None;
    for (index, stage) in metrics.stages.iter().enumerate() {
        // Session stages are separate rows. Delivery stages stay with Run totals.
        let row = if StageName::is_session(&stage.name) {
            if stage.name != StageName::Resume.name() {
                security_session = Some(stage.name == StageName::SecurityReview.name());
            }
            let mut session_group = Group {
                session_kind: Some(stage.name.clone()),
                ..group.clone()
            };
            // A Security review may use another setup (ADR 0021). Without
            // its evidence, the owning Command cannot tell us which one,
            // including on its Resume. An orphan Resume is unknown too.
            if security_session != Some(false) {
                session_group.harness = None;
                session_group.model = None;
                session_group.effort = None;
            }
            if let Some(setup) = record
                .stages
                .get(index)
                .filter(|recorded| recorded.name == stage.name)
                .and_then(|recorded| recorded.session_log.as_deref())
                .and_then(|log| SessionModels::for_session(log, &command.command_log))
            {
                session_group.harness = Some(setup.harness);
                session_group.model = setup.requested;
                session_group.effort = setup.effort;
            }
            if let Some(signals) = &stage.session_signals {
                session_group.harness = signals.harness.clone().or(session_group.harness);
                session_group.harness_version = signals.harness_version.clone();
            }
            let first = session_kinds.insert(session_group.clone());
            let row = weeks.entry(session_group).or_default();
            // A Run may have several Repairs; count it once per session kind.
            row.runs += usize::from(first);
            if record
                .stages
                .get(index)
                .and_then(|stage| stage.session_log.as_deref())
                .is_none_or(|log| sessions.insert(super::super::session::original(log)))
            {
                row.session_signals.add(stage.session_signals.as_ref());
            }
            row
        } else {
            weeks.get_mut(&group).expect("Run group exists")
        };
        row.stages.entry(stage.name.clone()).or_default().add(stage);
    }
}
