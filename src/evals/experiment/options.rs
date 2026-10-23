//! Experiment flags are independent per Arm; Bar overrides never change identity.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail, ensure};

use super::{Bar, Setup};
use crate::config::UserConfig;
use crate::harness::{Asked, Choice, Harness, ModelAndEffort};

pub struct Options {
    pub(super) candidate: String,
    pub(super) baseline: Option<String>,
    pub(super) baseline_setup: ArmOptions,
    pub(super) candidate_setup: ArmOptions,
    pub(super) suites: Vec<String>,
    pub(super) bar: Bar,
    pub(super) trials_per_task: Option<usize>,
    pub(super) json: bool,
}

#[derive(Default)]
pub(super) struct ArmOptions {
    harness: Option<Harness>,
    model: Option<String>,
    effort: Option<String>,
}

impl ArmOptions {
    pub(super) fn resolve(&self, commit: String, config: &UserConfig) -> Setup {
        let choice = Choice::of(
            &Asked {
                harness: self.harness,
                model_and_effort: ModelAndEffort {
                    model: self.model.clone(),
                    effort: self.effort.clone(),
                },
            },
            &config.harness,
        );
        Setup {
            commit,
            harness: choice.harness.name().into(),
            model: choice.model,
            effort: choice.effort,
        }
    }
}

pub fn parse(args: &[String]) -> Result<Options> {
    let mut flags = BTreeMap::new();
    let mut suites = BTreeSet::new();
    let mut candidate = None;
    let mut json = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--json" {
            ensure!(!json, "repeated --json");
            json = true;
        } else if arg.starts_with("--") {
            let value = args
                .next()
                .with_context(|| format!("{arg} requires a value"))?;
            if arg == "--suite" {
                crate::evals::format::name(value)?;
                ensure!(suites.insert(value.clone()), "repeated suite {value}");
            } else {
                ensure!(
                    flags.insert(arg.as_str(), value.as_str()).is_none(),
                    "repeated {arg}"
                );
            }
        } else {
            ensure!(
                candidate.replace(arg.clone()).is_none(),
                "only one Candidate is accepted"
            );
        }
    }
    let mut arm = |prefix: &str| -> Result<ArmOptions> {
        let harness = flags
            .remove(format!("--{prefix}-harness").as_str())
            .map(|name| Harness::named(name).context("unknown Arm Harness"))
            .transpose()?;
        Ok(ArmOptions {
            harness,
            model: flags
                .remove(format!("--{prefix}-model").as_str())
                .map(String::from),
            effort: flags
                .remove(format!("--{prefix}-effort").as_str())
                .map(String::from),
        })
    };
    let baseline_setup = arm("baseline")?;
    let candidate_setup = arm("candidate")?;
    let baseline = flags.remove("--baseline").map(String::from);
    let mut bar = Bar::default();
    macro_rules! number {
        ($flag:literal, $target:expr) => {
            if let Some(value) = flags.remove($flag) {
                $target = value
                    .parse()
                    .with_context(|| format!("invalid {}: {value}", $flag))?;
            }
        };
    }
    number!("--margin", bar.margin);
    number!("--credible-level", bar.credible_level);
    number!("--prior", bar.prior);
    number!("--seed", bar.seed);
    number!("--draws", bar.draws);
    number!("--timeout", bar.timeout_seconds);
    number!("--void-retries", bar.void_rules.retries);
    let trials_per_task = flags
        .remove("--trials-per-task")
        .map(str::parse::<usize>)
        .transpose()
        .context("invalid --trials-per-task")?;
    ensure!(
        trials_per_task.is_none_or(|n| n > 0),
        "Trials per Eval task must be positive"
    );
    ensure!(
        bar.margin.is_finite() && (-1.0..=1.0).contains(&bar.margin),
        "margin must be within [-1, 1]"
    );
    ensure!(
        bar.credible_level.is_finite() && bar.credible_level > 0.0 && bar.credible_level < 1.0,
        "credible level must be between zero and one"
    );
    ensure!(
        bar.prior.is_finite() && bar.prior > 0.0 && bar.draws >= 2 && bar.timeout_seconds > 0,
        "prior and timeout must be positive; at least two draws are required"
    );
    if let Some(flag) = flags.keys().next() {
        bail!("unknown Experiment flag: {flag}");
    }
    Ok(Options {
        candidate: candidate.context(
            "usage: thirdshift experiment <candidate> [--baseline <ref>] [--suite <name>] [--json]",
        )?,
        baseline,
        baseline_setup,
        candidate_setup,
        suites: suites.into_iter().collect(),
        bar,
        trials_per_task,
        json,
    })
}
