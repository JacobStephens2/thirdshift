//! Posterior-predictive assurance for one metric, assuming new non-void tasks.

use anyhow::{Result, ensure};
use rand::RngExt;
use rand_distr::{Distribution, Exp1};
use rand_pcg::Pcg64;

use super::{Bar, Decision, Draw, Intervals, MoreTasks, Posterior};

struct Path {
    probabilities: Vec<f64>,
    posterior_draws: Vec<Draw>,
}

pub(super) fn more_tasks(posterior: &Posterior, bar: &Bar, rng: &mut Pcg64) -> Result<MoreTasks> {
    let budget = &bar.prediction;
    let mut paths: Vec<_> = (0..budget.simulations)
        .map(|_| {
            let mut probabilities: Vec<_> =
                posterior.samplers.iter().map(|g| g.sample(rng)).collect();
            let total: f64 = probabilities.iter().sum();
            ensure!(
                total.is_finite() && total > 0.0,
                "predictive probabilities are outside floating-point range"
            );
            for probability in &mut probabilities {
                *probability /= total;
            }
            let path = Path {
                probabilities,
                posterior_draws: (0..budget.interval_draws)
                    .map(|_| posterior.draw(rng))
                    .collect(),
            };
            ensure!(
                path.posterior_draws.iter().all(|d| d.mean().is_finite()),
                "predictive posterior draws are outside floating-point range"
            );
            Ok(path)
        })
        .collect::<Result<_>>()?;
    let mut result = MoreTasks {
        target_probability: 0.8,
        additional_tasks: None,
        clear_probabilities: Vec::new(),
    };
    let mut means = Vec::with_capacity(budget.interval_draws);
    for additional in 1..=budget.max_additional_tasks {
        let mut clear = 0;
        for path in &mut paths {
            // One latent distribution per path, drawn from the current posterior;
            // its future Eval tasks are conditionally independent categorical draws.
            let point = rng.random::<f64>();
            let mut cumulative = 0.0;
            let category = path
                .probabilities
                .iter()
                .position(|probability| {
                    cumulative += probability;
                    point < cumulative
                })
                .unwrap_or(path.probabilities.len() - 1);
            let value = posterior.values[category];
            means.clear();
            for draw in &mut path.posterior_draws {
                // Gamma(a,1) + Gamma(1,1) = Gamma(a+1,1). Updating only the
                // observed category gives exact updated Dirichlet marginal draws
                // without resampling every category at every future task count.
                let increment: f64 = Exp1.sample(rng);
                draw.total += increment;
                draw.weighted += increment * value;
                means.push(draw.mean());
            }
            if Intervals::from_draws(&mut means, bar.levels)
                .decide(bar.margin, bar.levels.pass_rule())
                != Decision::Inconclusive
            {
                clear += 1;
            }
        }
        let probability = clear as f64 / budget.simulations as f64;
        result.clear_probabilities.push(probability);
        if probability >= result.target_probability {
            result.additional_tasks = Some(additional);
            break;
        }
    }
    Ok(result)
}
