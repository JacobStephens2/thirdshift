//! Price every retained Session once; compare complete logical Trials in pairs.
use std::collections::BTreeMap;
use std::io::{BufReader, Cursor};
use std::path::Path;

use anyhow::Result;
use serde::Serialize;

use super::trial_results::files::{Native, metadata};
use super::{TrialResult, trial_sessions};
use crate::otlp_jsonl::{AnyValue, Reader, Spans};
use crate::session::usage;
use crate::trial::accounting::{Attempt, Prices, Session, SessionStart};
use crate::trial::{Outcome, Output};

pub(super) fn capture(
    path: &Path,
    output: &Output,
    role: &str,
    prices: &Prices,
    stopped_at: u64,
) -> Result<Attempt> {
    let mut accounting = Attempt::default();
    for session in trial_sessions::attempt(&Native, path)?.sessions {
        let launched: Option<SessionStart> =
            metadata(&Native, &SessionStart::path(&session.original))?;
        let bytes = session.spans.as_ref().map(std::fs::read).transpose()?;
        let summary = bytes
            .as_ref()
            .map(|bytes| usage::from_spans(BufReader::new(Cursor::new(bytes))))
            .transpose()?
            .unwrap_or_default();
        let mut started = 0;
        let mut time_ms = 0;
        let mut recorded_role = None;
        let mut ended = false;
        if let Some(bytes) = &bytes {
            for request in Reader::<_, Spans>::new(Cursor::new(bytes)) {
                for span in request?
                    .resource_spans
                    .into_iter()
                    .flat_map(|resource| resource.scope_spans)
                    .flat_map(|scope| scope.spans)
                    .filter(|span| span.name.starts_with("invoke_agent "))
                {
                    started = span.start_time_unix_nano;
                    ended = span.status.is_some();
                    recorded_role = span.attributes.iter().find_map(|attribute| {
                        match (&*attribute.key, &attribute.value) {
                            ("thirdshift.trial.role", Some(AnyValue::String(role))) => {
                                Some(role.clone())
                            }
                            _ => None,
                        }
                    });
                    time_ms =
                        u128::from(span.end_time_unix_nano.saturating_sub(started)) / 1_000_000;
                }
            }
        }
        if !ended && let Some(launched) = &launched {
            time_ms = (u128::from(stopped_at.saturating_sub(launched.at_unix_nano)) / 1_000_000)
                .min(output.timings.total_ms);
        }
        let role = recorded_role
            .as_deref()
            .or_else(|| launched.as_ref().map(|start| start.role.as_str()))
            .unwrap_or_else(|| {
                output
                    .steps
                    .iter()
                    .find(|step| {
                        let nanos = |text: &str| {
                            chrono::DateTime::parse_from_rfc3339(text)
                                .ok()
                                .and_then(|date| date.timestamp_nanos_opt())
                                .and_then(|n| u64::try_from(n).ok())
                        };
                        nanos(&step.started_at)
                            .zip(nanos(&step.ended_at))
                            .is_some_and(|(begin, end)| begin <= started && started <= end)
                    })
                    .map_or(role, |step| step.name.as_str())
            });
        let priced = prices.cost(&summary);
        let gap = priced.as_ref().err().map(|error| {
            format!(
                "accounting gap in {} ({role}): {error:#}",
                session.original.display()
            )
        });
        accounting.sessions.push(Session {
            path: session.original.to_string_lossy().into_owned(),
            role: role.into(),
            time_ms,
            models: summary
                .models
                .into_iter()
                .map(|model| {
                    (
                        model.model.unwrap_or_else(|| "unknown".into()),
                        model.measurement,
                    )
                })
                .collect(),
            cost_usd: priced.ok(),
            gap,
        });
    }
    // A legacy Arm can report success without retaining any Session family.
    // Command-only Processes legitimately start none.
    if accounting.sessions.is_empty()
        && (output.log.is_some()
            || output.session_id.is_some()
            || (matches!(
                output.outcome,
                Outcome::Succeeded | Outcome::Delivered | Outcome::StoppedToAsk
            ) && role != "process"
                && output.timings.session_ms > 0))
    {
        accounting.sessions.push(Session {
            path: path.join("session").to_string_lossy().into_owned(),
            role: role.into(),
            time_ms: output.timings.session_ms,
            models: BTreeMap::new(),
            cost_usd: None,
            gap: Some("accounting gap: missing usage and retained Session evidence".into()),
        });
    }
    Ok(accounting)
}

#[derive(Default, Serialize)]
pub(crate) struct Report {
    pub baseline: Arm,
    pub candidate: Arm,
    pub trials: Vec<Trial>,
    pub paired_median_ratios: Ratios,
}

#[derive(Default, Serialize)]
pub(crate) struct Ratios {
    pub cost: Option<f64>,
    pub time: Option<f64>,
    pub pairs: usize,
}

#[derive(Default, Serialize)]
pub(crate) struct Arm {
    #[serde(flatten)]
    pub total: Total,
    pub roles: BTreeMap<String, Total>,
}

#[derive(Serialize)]
pub(crate) struct Total {
    pub sessions: usize,
    pub cost_usd: Option<f64>,
    pub time_ms: u128,
}

impl Default for Total {
    fn default() -> Self {
        Self {
            sessions: 0,
            cost_usd: Some(0.0),
            time_ms: 0,
        }
    }
}

impl Total {
    fn include(&mut self, cost: Option<f64>, time_ms: u128, sessions: usize) {
        self.sessions += sessions;
        self.time_ms += time_ms;
        self.cost_usd = self.cost_usd.zip(cost).map(|(left, right)| left + right);
    }
}

#[derive(Serialize)]
pub(crate) struct Trial {
    pub id: String,
    pub arm: super::Arm,
    pub cost_usd: Option<f64>,
    pub time_ms: u128,
    pub roles: BTreeMap<String, Total>,
    pub sessions: Vec<SessionRef>,
}

#[derive(Serialize)]
pub(crate) struct SessionRef {
    pub attempt: usize,
    #[serde(flatten)]
    pub session: Session,
}

pub(super) fn trial_total(result: &TrialResult) -> Total {
    let mut total = Total::default();
    for output in &result.attempts {
        let cost = output.accounting.as_ref().and_then(|accounting| {
            accounting
                .sessions
                .iter()
                .try_fold(0.0, |sum, session| session.cost_usd.map(|cost| sum + cost))
        });
        total.include(
            cost,
            output.timings.total_ms,
            output
                .accounting
                .as_ref()
                .map_or(0, |accounting| accounting.sessions.len()),
        );
    }
    total
}

impl Report {
    pub(super) fn render(&self) -> String {
        let cost =
            |amount: Option<f64>| amount.map_or("unknown".into(), |amount| format!("${amount:.6}"));
        let ratio =
            |value: Option<f64>| value.map_or("unknown".into(), |value| format!("{value:.3}"));
        let mut text = format!(
            "\nFrozen-price accounting: Baseline {}, Candidate {}\nPaired median Candidate/Baseline: cost {}, time {} ({} pairs)",
            cost(self.baseline.total.cost_usd),
            cost(self.candidate.total.cost_usd),
            ratio(self.paired_median_ratios.cost),
            ratio(self.paired_median_ratios.time),
            self.paired_median_ratios.pairs
        );
        for (name, arm) in [("Baseline", &self.baseline), ("Candidate", &self.candidate)] {
            for (role, total) in &arm.roles {
                text.push_str(&format!(
                    "\n{name} {role}: {} sessions, {}, {:.3} minutes",
                    total.sessions,
                    cost(total.cost_usd),
                    total.time_ms as f64 / 60_000.0
                ));
            }
        }
        text
    }

    pub(super) fn of(results: &[TrialResult], unfinished: &[TrialResult]) -> Self {
        let mut report = Self::default();
        for result in results.iter().chain(unfinished) {
            let total = trial_total(result);
            let mut trial = Trial {
                id: result.key(),
                arm: result.arm,
                cost_usd: total.cost_usd,
                time_ms: total.time_ms,
                roles: BTreeMap::new(),
                sessions: Vec::new(),
            };
            for (attempt, output) in result.attempts.iter().enumerate() {
                if let Some(accounting) = &output.accounting {
                    for session in &accounting.sessions {
                        trial
                            .roles
                            .entry(session.role.clone())
                            .or_default()
                            .include(session.cost_usd, session.time_ms, 1);
                        trial.sessions.push(SessionRef {
                            attempt,
                            session: session.clone(),
                        });
                    }
                }
            }
            let arm = match result.arm {
                super::Arm::Baseline => &mut report.baseline,
                super::Arm::Candidate => &mut report.candidate,
            };
            arm.total
                .include(total.cost_usd, total.time_ms, total.sessions);
            for (role, total) in &trial.roles {
                arm.roles.entry(role.clone()).or_default().include(
                    total.cost_usd,
                    total.time_ms,
                    total.sessions,
                );
            }
            report.trials.push(trial);
        }
        let mut costs = Vec::new();
        let mut times = Vec::new();
        for pair in paired_ratios(results) {
            report.paired_median_ratios.pairs += 1;
            costs.extend(pair.cost);
            times.extend(pair.time);
        }
        report.paired_median_ratios.cost = median(&mut costs);
        report.paired_median_ratios.time = median(&mut times);
        report
    }
}

pub(super) struct PairedRatios<'a> {
    pub suite: &'a str,
    pub task: &'a str,
    pub cost: Option<f64>,
    pub time: Option<f64>,
}

pub(super) fn paired_ratios(results: &[TrialResult]) -> impl Iterator<Item = PairedRatios<'_>> {
    results
        .iter()
        .filter(|result| result.arm == super::Arm::Baseline && result.outcome != Outcome::Void)
        .filter_map(|baseline| {
            results
                .iter()
                .find(|result| {
                    result.arm == super::Arm::Candidate
                        && result.suite == baseline.suite
                        && result.task == baseline.task
                        && result.trial == baseline.trial
                        && result.outcome != Outcome::Void
                })
                .map(|candidate| {
                    let base = trial_total(baseline);
                    let cand = trial_total(candidate);
                    PairedRatios {
                        suite: &baseline.suite,
                        task: &baseline.task,
                        cost: base
                            .cost_usd
                            .zip(cand.cost_usd)
                            .filter(|(base, _)| *base > 0.0)
                            .map(|(base, cand)| cand / base),
                        time: (base.time_ms > 0).then(|| cand.time_ms as f64 / base.time_ms as f64),
                    }
                })
        })
}

pub(super) fn median(values: &mut [f64]) -> Option<f64> {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.is_empty() {
        None
    } else if values.len().is_multiple_of(2) {
        Some(values[middle - 1] / 2.0 + values[middle] / 2.0)
    } else {
        Some(values[middle])
    }
}
