//! Paired resource comparisons, separate from the quality decision. Bootstrap
//! weights are shared by a task's Trials so repeats are not independent tasks.
use std::collections::BTreeMap;

use rand::SeedableRng;
use rand_distr::{Distribution, Exp1};
use rand_pcg::Pcg64;
use serde::{Deserialize, Serialize};

use super::{Bar, TrialResult, accounting};
use crate::verdict::{Decision, Interval};

pub(super) fn optional_decimal<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<f64>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    value
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| serde::de::Error::custom("expected a decimal"))
        })
        .transpose()
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Efficiency {
    pub quality_verdict: Decision,
    pub method: String,
    pub cost: Comparison,
    pub time: Comparison,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Comparison {
    #[serde(deserialize_with = "optional_decimal")]
    pub ratio: Option<f64>,
    pub interval: Option<Interval>,
    pub pairs: usize,
    pub complete: bool,
    #[serde(deserialize_with = "optional_decimal")]
    pub ceiling: Option<f64>,
}

impl Efficiency {
    pub(super) fn of(results: &[TrialResult], bar: &Bar, quality_verdict: Decision) -> Self {
        let mut costs = BTreeMap::new();
        let mut times = BTreeMap::new();
        for (baseline, candidate) in accounting::pairs(results) {
            let key = (baseline.suite.as_str(), baseline.task.as_str());
            let baseline = accounting::trial_total(baseline);
            let candidate = accounting::trial_total(candidate);
            costs.entry(key).or_insert_with(Vec::new).push(
                baseline
                    .cost_usd
                    .zip(candidate.cost_usd)
                    .filter(|(base, _)| *base > 0.0)
                    .map(|(base, cand)| cand / base),
            );
            times.entry(key).or_insert_with(Vec::new).push(
                (baseline.time_ms > 0).then(|| candidate.time_ms as f64 / baseline.time_ms as f64),
            );
        }
        Self {
            quality_verdict,
            method: "paired Bayesian bootstrap median; task-cluster weights; pass level; Pcg64 0.10.2; rand_distr 0.6.0".into(),
            cost: Comparison::of(costs, bar, bar.cost_ceiling, bar.seed),
            time: Comparison::of(times, bar, bar.time_ceiling, bar.seed.wrapping_add(1)),
        }
    }

    pub(super) fn decision(&self) -> Decision {
        if self.cost.exceeded() || self.time.exceeded() {
            Decision::Fail
        } else if self.quality_verdict == Decision::Pass
            && (self.cost.unmeasured_ceiling() || self.time.unmeasured_ceiling())
        {
            Decision::Inconclusive
        } else {
            self.quality_verdict
        }
    }

    pub(super) fn annotation(&self) -> String {
        let mut parts = Vec::new();
        for (name, comparison, less, more) in [
            ("cost", &self.cost, "cheaper", "costs more"),
            ("time", &self.time, "faster", "takes longer"),
        ] {
            if comparison.exceeded() {
                parts.push(format!(
                    "{name} ceiling exceeded ({:.3}× > {:.3}×)",
                    comparison.ratio.unwrap(),
                    comparison.ceiling.unwrap()
                ));
            } else if comparison.unmeasured_ceiling() {
                parts.push(format!("{name} ceiling unmeasured"));
            } else if comparison
                .interval
                .is_some_and(|interval| interval.upper < 1.0)
            {
                parts.push(less.into());
            } else if comparison.ratio.is_some_and(|ratio| ratio > 1.0) {
                parts.push(more.into());
            }
        }
        parts.join(", and ")
    }
}

impl Comparison {
    fn of(
        tasks: BTreeMap<(&str, &str), Vec<Option<f64>>>,
        bar: &Bar,
        ceiling: Option<f64>,
        seed: u64,
    ) -> Self {
        let mut samples = Vec::new();
        let mut complete = !tasks.is_empty();
        for (task, ratios) in tasks.values().enumerate() {
            for ratio in ratios {
                if let Some(ratio) = ratio.filter(|ratio| ratio.is_finite() && *ratio >= 0.0) {
                    samples.push((ratio, task));
                } else {
                    complete = false;
                }
            }
        }
        let ratio =
            accounting::median(&mut samples.iter().map(|(ratio, _)| *ratio).collect::<Vec<_>>());
        samples.sort_by(|left, right| left.0.total_cmp(&right.0));
        let interval = if complete && tasks.len() >= 2 {
            let mut rng = Pcg64::seed_from_u64(seed);
            let mut draws = Vec::with_capacity(bar.draws);
            for _ in 0..bar.draws {
                let weights: Vec<f64> = tasks.values().map(|_| Exp1.sample(&mut rng)).collect();
                let total: f64 = samples.iter().map(|(_, task)| weights[*task]).sum();
                let mut cumulative = 0.0;
                for (ratio, task) in &samples {
                    cumulative += weights[*task];
                    if cumulative >= total / 2.0 {
                        draws.push(*ratio);
                        break;
                    }
                }
            }
            draws.sort_by(f64::total_cmp);
            let tail = (1.0 - bar.levels.pass()) / 2.0;
            Some(Interval {
                lower: draws[((draws.len() - 1) as f64 * tail).floor() as usize],
                upper: draws[((draws.len() - 1) as f64 * (1.0 - tail)).ceil() as usize],
            })
        } else {
            None
        };
        Self {
            ratio,
            interval,
            pairs: samples.len(),
            complete,
            ceiling,
        }
    }

    fn exceeded(&self) -> bool {
        self.ratio
            .zip(self.ceiling)
            .is_some_and(|(ratio, ceiling)| ratio > ceiling)
    }

    fn unmeasured_ceiling(&self) -> bool {
        self.ceiling.is_some() && (!self.complete || self.ratio.is_none())
    }
}
