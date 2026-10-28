//! Paired Verdicts from already graded Trials; no session or storage dependencies.
//!
//! This Bar holds statistical settings. The Experiment runner owns commits,
//! timeouts and Void Trial causes/re-runs, and marks exhausted tasks as dropped.
//! The dropout guard takes precedence; otherwise either failed metric fails the
//! Experiment, and both metrics must pass for a pass. Forecasts are per metric
//! and assume additional completed, non-void Eval tasks. Accuracy uses the Bar's
//! seed; wrong findings use seed + 1 (wrapping), independently of each forecast.

use anyhow::{Result, ensure};
use rand::SeedableRng;
use rand_distr::{Distribution, Gamma};
use rand_pcg::Pcg64;
use serde::{Deserialize, Serialize};

mod levels;
mod prediction;

pub(crate) use levels::decimal;
pub use levels::{Intervals, Levels, PassRule};

const MARGIN_METHOD: &str =
    "paired-dirichlet-v3; Pcg64 0.10.2; rand 0.10.3; rand_distr 0.6.0; linear quantiles";
const REACHES_ZERO_METHOD: &str =
    "paired-dirichlet-v2; Pcg64 0.10.2; rand 0.10.3; rand_distr 0.6.0; linear quantiles";
const LEGACY_METHOD: &str =
    "paired-dirichlet-v1; Pcg64 0.10.2; rand 0.10.3; rand_distr 0.6.0; linear quantiles";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    /// Signed lower bound on Candidate minus Baseline (a ten-point tolerance).
    pub margin: f64,
    #[serde(flatten)]
    pub levels: Levels,
    /// Total pseudo Eval tasks, spread evenly across all possible differences.
    pub prior: f64,
    pub seed: u64,
    pub draws: usize,
    pub trials_per_task: usize,
    pub prediction: PredictionBudget,
}

/// Finite Monte Carlo budget for the posterior-predictive search.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PredictionBudget {
    pub simulations: usize,
    pub interval_draws: usize,
    pub max_additional_tasks: usize,
}

impl Default for PredictionBudget {
    fn default() -> Self {
        Self {
            simulations: 200,
            interval_draws: 512,
            max_additional_tasks: 2_000,
        }
    }
}

impl Default for Bar {
    fn default() -> Self {
        Self {
            margin: -0.10,
            levels: Levels::default(),
            prior: 1.0,
            seed: 0,
            draws: 20_000,
            trials_per_task: 3,
            prediction: PredictionBudget::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Score {
    pub passed: bool,
    pub wrong_finding: bool,
}

#[derive(Clone, Debug)]
pub struct TaskScores {
    pub baseline: Vec<Score>,
    pub candidate: Vec<Score>,
    /// The runner sets this only after the Bar's Void Trial re-runs are exhausted.
    pub dropped_as_void: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    #[serde(deserialize_with = "decimal")]
    pub lower: f64,
    #[serde(deserialize_with = "decimal")]
    pub upper: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub bar: Bar,
    pub thirdshift_version: String,
    pub method: String,
    pub accuracy: Assessment,
    pub wrong_findings: Assessment,
    pub verdict: Decision,
    pub retained_tasks: usize,
    pub dropped_tasks: usize,
    pub pass_cubed: Option<PassCubed>,
}

/// Share of retained Eval tasks for which all three Trials passed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassCubed {
    pub baseline: f64,
    pub candidate: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assessment {
    /// Counts ordered from -1 to +1 in steps of 1 / Trials per Eval task.
    /// For wrong findings, positive means fewer wrong findings in the Candidate.
    pub difference_counts: Vec<usize>,
    #[serde(flatten)]
    pub intervals: Intervals,
    pub verdict: Decision,
    /// A pass whose pass-level interval lies wholly below zero: statistically
    /// worse, but within the margin. It never changes the decision.
    #[serde(default, skip_serializing_if = "is_false")]
    pub worse_within_margin: bool,
    pub more_tasks: Option<MoreTasks>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoreTasks {
    pub target_probability: f64,
    /// None means the search budget was exhausted, not that no number exists.
    pub additional_tasks: Option<usize>,
    /// Entry k-1 is the simulated chance of a clear Verdict after k more tasks.
    pub clear_probabilities: Vec<f64>,
}

pub fn judge(scores: &[TaskScores], bar: &Bar) -> Result<Verdict> {
    evaluate(scores, bar, true)
}

/// The same seeded decision, without forecasting extra tasks for counterfactuals.
pub fn judge_without_predictions(scores: &[TaskScores], bar: &Bar) -> Result<Verdict> {
    evaluate(scores, bar, false)
}

fn evaluate(scores: &[TaskScores], bar: &Bar, predictions: bool) -> Result<Verdict> {
    validate(scores, bar)?;
    let trials = bar.trials_per_task;
    let mut counts = vec![0; 2 * trials + 1];
    let mut wrong_counts = vec![0; 2 * trials + 1];
    let mut baseline_all_passed = 0;
    let mut candidate_all_passed = 0;
    let dropped_tasks = scores.iter().filter(|task| task.dropped_as_void).count();
    for task in scores {
        if task.dropped_as_void {
            continue;
        }
        let baseline = task.baseline.iter().filter(|s| s.passed).count();
        let candidate = task.candidate.iter().filter(|s| s.passed).count();
        counts[candidate + trials - baseline] += 1;
        baseline_all_passed += usize::from(baseline == trials);
        candidate_all_passed += usize::from(candidate == trials);
        let baseline_wrong = task.baseline.iter().filter(|s| s.wrong_finding).count();
        let candidate_wrong = task.candidate.iter().filter(|s| s.wrong_finding).count();
        wrong_counts[baseline_wrong + trials - candidate_wrong] += 1;
    }
    let accuracy = assess(&counts, bar, bar.seed, predictions)?;
    let wrong_findings = assess(&wrong_counts, bar, bar.seed.wrapping_add(1), predictions)?;
    let decisions = [accuracy.verdict, wrong_findings.verdict];
    let verdict = if scores.is_empty() || dropped_tasks > scores.len() / 10 {
        Decision::Inconclusive
    } else if decisions.contains(&Decision::Fail) {
        Decision::Fail
    } else if decisions.contains(&Decision::Inconclusive) {
        Decision::Inconclusive
    } else {
        Decision::Pass
    };
    let retained_tasks = scores.len() - dropped_tasks;
    Ok(Verdict {
        bar: bar.clone(),
        thirdshift_version: env!("CARGO_PKG_VERSION").to_owned(),
        method: match bar.levels {
            Levels::Separate {
                pass_rule: PassRule::Margin,
                ..
            } => MARGIN_METHOD,
            Levels::Separate {
                pass_rule: PassRule::ReachesZero,
                ..
            } => REACHES_ZERO_METHOD,
            Levels::Legacy { .. } => LEGACY_METHOD,
        }
        .to_owned(),
        accuracy,
        wrong_findings,
        verdict,
        retained_tasks,
        dropped_tasks,
        pass_cubed: (trials == 3 && retained_tasks > 0).then(|| PassCubed {
            baseline: baseline_all_passed as f64 / retained_tasks as f64,
            candidate: candidate_all_passed as f64 / retained_tasks as f64,
        }),
    })
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn validate(scores: &[TaskScores], bar: &Bar) -> Result<()> {
    ensure!(
        matches!(bar.trials_per_task, 1 | 3),
        "Trials per Eval task must be one or three"
    );
    ensure!(
        bar.margin.is_finite() && (-1.0..=1.0).contains(&bar.margin),
        "margin must be finite and within [-1, 1]"
    );
    bar.levels.validate()?;
    let prior_per_value = bar.prior / (2 * bar.trials_per_task + 1) as f64;
    ensure!(
        bar.prior.is_finite() && prior_per_value > 0.0,
        "prior must be finite and positive in every category"
    );
    ensure!(bar.draws >= 2, "at least two interval draws are required");
    ensure!(
        bar.prediction.simulations > 0
            && bar.prediction.interval_draws >= 2
            && bar.prediction.max_additional_tasks > 0,
        "prediction budget requires simulations, at least two interval draws and a positive task limit"
    );
    for (index, task) in scores.iter().enumerate() {
        if !task.dropped_as_void {
            ensure!(
                task.baseline.len() == bar.trials_per_task
                    && task.candidate.len() == bar.trials_per_task,
                "Eval task {index} must have the Bar's Trials in both Arms"
            );
        }
    }
    Ok(())
}

fn assess(counts: &[usize], bar: &Bar, seed: u64, predictions: bool) -> Result<Assessment> {
    let posterior = Posterior::new(counts, bar)?;
    let mut rng = Pcg64::seed_from_u64(seed);
    let mut draws: Vec<_> = (0..bar.draws)
        .map(|_| posterior.draw(&mut rng).mean())
        .collect();
    ensure!(
        draws.iter().all(|d| d.is_finite()),
        "posterior draws are outside floating-point range"
    );
    let intervals = Intervals::from_draws(&mut draws, bar.levels);
    let verdict = intervals.decide(bar.margin, bar.levels.pass_rule());
    Ok(Assessment {
        difference_counts: counts.to_vec(),
        intervals,
        verdict,
        worse_within_margin: verdict == Decision::Pass && intervals.pass().upper < 0.0,
        more_tasks: (predictions && verdict == Decision::Inconclusive)
            .then(|| prediction::more_tasks(&posterior, bar, &mut rng))
            .transpose()?,
    })
}

struct Posterior {
    samplers: Vec<Gamma<f64>>,
    values: Vec<f64>,
}

impl Posterior {
    fn new(counts: &[usize], bar: &Bar) -> Result<Self> {
        let samplers = counts
            .iter()
            .map(|&n| Gamma::new(n as f64 + bar.prior / counts.len() as f64, 1.0))
            .collect::<std::result::Result<_, _>>()?;
        let values = (0..counts.len())
            .map(|i| (i as f64 - bar.trials_per_task as f64) / bar.trials_per_task as f64)
            .collect();
        Ok(Self { samplers, values })
    }

    fn draw(&self, rng: &mut Pcg64) -> Draw {
        let mut draw = Draw {
            total: 0.0,
            weighted: 0.0,
        };
        for (sampler, &value) in self.samplers.iter().zip(&self.values) {
            let weight = sampler.sample(rng);
            draw.total += weight;
            draw.weighted += weight * value;
        }
        draw
    }
}

struct Draw {
    total: f64,
    weighted: f64,
}

impl Draw {
    fn mean(&self) -> f64 {
        self.weighted / self.total
    }
}

fn interval(draws: &mut [f64], credible_level: f64) -> Interval {
    let tail = (1.0 - credible_level) / 2.0;
    Interval {
        lower: quantile(draws, tail),
        upper: quantile(draws, 1.0 - tail),
    }
}

fn quantile(draws: &mut [f64], q: f64) -> f64 {
    let index = q * (draws.len() - 1) as f64;
    let lower = *draws
        .select_nth_unstable_by(index.floor() as usize, f64::total_cmp)
        .1;
    let upper = *draws
        .select_nth_unstable_by(index.ceil() as usize, f64::total_cmp)
        .1;
    lower + (upper - lower) * index.fract()
}

#[cfg(test)]
mod tests;
