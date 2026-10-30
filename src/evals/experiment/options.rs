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
    pub(super) release: bool,
    pub(super) yes: bool,
    pub(super) name: Option<String>,
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

enum ArgumentKind {
    Json,
    Release,
    Yes,
    ValuedFlag,
    Positional,
}

fn argument_kind(word: &str) -> ArgumentKind {
    match word {
        "--json" => ArgumentKind::Json,
        "--release" => ArgumentKind::Release,
        "--yes" => ArgumentKind::Yes,
        flag if flag.starts_with("--") => ArgumentKind::ValuedFlag,
        _ => ArgumentKind::Positional,
    }
}

/// Translate a captured legacy command for readers, retaining every other
/// argument literally and taking both levels from its recorded Bar.
pub(super) fn resume_command(command: &str, levels: crate::verdict::Levels) -> Option<String> {
    if !matches!(levels, crate::verdict::Levels::Legacy { .. }) {
        return Some(command.to_owned());
    }
    let mut original = shlex::split(command)?.into_iter();
    let mut updated = Vec::new();
    while let Some(word) = original.next() {
        if matches!(
            word.as_str(),
            "--credible-level" | "--pass-level" | "--fail-level"
        ) {
            original.next()?;
        } else {
            let takes_value = matches!(argument_kind(&word), ArgumentKind::ValuedFlag);
            updated.push(word);
            if takes_value {
                updated.push(original.next()?);
            }
        }
    }
    updated.extend([
        "--pass-level".into(),
        levels.pass().to_string(),
        "--fail-level".into(),
        levels.fail().to_string(),
    ]);
    Some(crate::args::shell_command(updated))
}

pub fn parse(args: &[String]) -> Result<Options> {
    let mut flags = BTreeMap::new();
    let mut suites = BTreeSet::new();
    let mut candidate = None;
    let mut json = false;
    let mut release = false;
    let mut yes = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match argument_kind(arg) {
            ArgumentKind::Json => {
                ensure!(!json, "repeated --json");
                json = true;
            }
            ArgumentKind::Release => {
                ensure!(!release, "repeated --release");
                release = true;
            }
            ArgumentKind::Yes => {
                ensure!(!yes, "repeated --yes");
                yes = true;
            }
            ArgumentKind::ValuedFlag => {
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
            }
            ArgumentKind::Positional => {
                ensure!(
                    candidate.replace(arg.clone()).is_none(),
                    "only one Candidate is accepted"
                );
            }
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
    let name = flags
        .remove("--name")
        .map(|name| super::names::parse(name)?.context("Experiment name must not be empty"))
        .transpose()?;
    ensure!(
        !release || suites.is_empty(),
        "release mode selects its touched suites; --suite does not apply"
    );
    let mut bar = Bar {
        min_discriminating_tasks: Some(20),
        ..Bar::default()
    };
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
    let mut pass_level = bar.levels.pass();
    let mut fail_level = bar.levels.fail();
    number!("--pass-level", pass_level);
    number!("--fail-level", fail_level);
    bar.levels = crate::verdict::Levels::Separate {
        pass_level,
        fail_level,
        pass_rule: crate::verdict::PassRule::Margin,
    };
    number!("--prior", bar.prior);
    number!("--seed", bar.seed);
    number!("--draws", bar.draws);
    let mut minimum = bar.min_discriminating_tasks.unwrap();
    number!("--min-discriminating-tasks", minimum);
    bar.min_discriminating_tasks = Some(minimum);
    number!("--timeout", bar.timeout_seconds);
    number!("--void-retries", bar.void_rules.retries);
    ensure!(
        bar.void_rules.retries <= 2,
        "Void Trials allow at most two fresh retries"
    );
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
    bar.levels.validate()?;
    ensure!(
        bar.prior.is_finite() && bar.prior > 0.0 && bar.draws >= 2 && bar.timeout_seconds > 0,
        "prior and timeout must be positive; at least two draws are required"
    );
    if let Some(flag) = flags.keys().next() {
        bail!("unknown Experiment flag: {flag}");
    }
    Ok(Options {
        candidate: candidate.context(
            "usage: thirdshift experiment <candidate> [--baseline <ref>] [--suite <name>] [--json] [--yes]",
        )?,
        baseline,
        baseline_setup,
        candidate_setup,
        suites: suites.into_iter().collect(),
        bar,
        trials_per_task,
        json,
        release,
        yes,
        name,
    })
}
