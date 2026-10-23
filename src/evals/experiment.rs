//! A resumable Experiment owns its fixed inputs and Trial artifacts. Command
//! recording stays in the ordinary logs; private Eval data stays here.

mod options;
mod store;
mod workers;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use store::State;

use super::{arms, format, suite};
use crate::config::UserConfig;
use crate::git::Git;
use crate::harness::{Choice, ChosenBy, Harness};
use crate::issue::Repo;
use crate::trial::VoidCause;
use crate::{interrupt, logs, progress};

pub use options::{Options, parse};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Bar {
    #[serde(deserialize_with = "decimal")]
    margin: f64,
    #[serde(deserialize_with = "decimal")]
    credible_level: f64,
    #[serde(deserialize_with = "decimal")]
    prior: f64,
    seed: u64,
    draws: usize,
    trials_per_task: BTreeMap<String, usize>,
    timeout_seconds: u64,
    void_rules: VoidRules,
}

// serde's flattened-record buffer sees arbitrary-precision JSON decimals as
// Number maps. Value preserves that representation before conversion to f64.
fn decimal<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<f64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    value
        .as_f64()
        .ok_or_else(|| serde::de::Error::custom("expected a decimal"))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct VoidRules {
    retries: usize,
    causes: Vec<VoidCause>,
}

impl Default for Bar {
    fn default() -> Self {
        let statistics = crate::verdict::Bar::default();
        Self {
            margin: statistics.margin,
            credible_level: statistics.credible_level,
            prior: statistics.prior,
            seed: statistics.seed,
            draws: statistics.draws,
            trials_per_task: BTreeMap::new(),
            timeout_seconds: 1800,
            void_rules: VoidRules {
                retries: 2,
                causes: [
                    VoidCause::SessionUnavailable,
                    VoidCause::UsageLimit,
                    VoidCause::RateLimit,
                    VoidCause::Interrupted,
                ]
                .into(),
            },
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Setup {
    commit: String,
    harness: String,
    model: Option<String>,
    effort: Option<String>,
}

impl Setup {
    fn choice(&self) -> Choice {
        Choice {
            harness: Harness::named(&self.harness).expect("validated Arm Harness"),
            model: self.model.clone(),
            effort: self.effort.clone(),
            chosen_by: ChosenBy::Command,
        }
    }
}

/// Only these public Experiment facts enter factory logs, never task inputs.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Description {
    id: String,
    baseline: Setup,
    candidate: Setup,
    suites: Vec<String>,
    bar: Bar,
    prepared: Option<arms::Prepared>,
}

#[derive(Serialize, Deserialize)]
struct Task {
    suite: String,
    name: String,
    directory: std::path::PathBuf,
    manifest: format::Task,
}

pub fn run(options: Options) -> Result<()> {
    interrupt::install()?;
    let config = UserConfig::load()?;
    let repository = std::env::current_dir()?;
    let git = Git::new(&repository);
    let candidate = resolve(&git, &options.candidate)?;
    let baseline = match &options.baseline {
        Some(reference) => resolve(&git, reference)?,
        None => git.run(&["--no-replace-objects", "merge-base", "main", &candidate])?,
    };
    let baseline = options.baseline_setup.resolve(baseline, &config);
    let candidate = options.candidate_setup.resolve(candidate, &config);
    let root = crate::config::home()?.join(".thirdshift/evals");
    let suites_root = root.join("suites");
    let suites = if options.suites.is_empty() {
        let paths = git.run(&[
            "--no-replace-objects",
            "diff",
            "--name-only",
            "--no-renames",
            &baseline.commit,
            &candidate.commit,
            "--",
        ])?;
        suite::touched(
            &suites_root,
            &paths.lines().map(String::from).collect::<Vec<_>>(),
        )?
    } else {
        options.suites.clone()
    };
    ensure!(
        !suites.is_empty(),
        "no Eval suites selected; use --suite for a defaults-only Experiment"
    );
    let mut bar = options.bar;
    for name in &suites {
        let manifest = suite::Manifest::load(&suites_root.join(name))?;
        bar.trials_per_task.insert(
            name.clone(),
            options
                .trials_per_task
                .unwrap_or(manifest.trials_per_arm.get()),
        );
    }
    let identity = serde_json::to_vec(&(&baseline, &candidate, &suites))?;
    let id = format!("{:x}", Sha256::digest(identity));
    let description = Description {
        id,
        baseline,
        candidate,
        suites,
        bar,
        prepared: None,
    };
    let selected = description.suites.clone();
    let mut experiment = store::Stored::open(&root.join("experiments"), description, || {
        tasks(&suites_root, &selected)
    })?;
    if experiment.state == State::Completed {
        println!("{}", experiment.render(options.json)?);
        return Ok(());
    }
    logs::begin(logs::Begin::Experiment);
    logs::configured(&config);
    let repo = Repo {
        owner: "JacobStephens2".into(),
        name: "thirdshift".into(),
    };
    logs::started(
        logs::Work::Experiment(&repo),
        &experiment.description.candidate.choice(),
    );
    logs::experiment(&experiment.description);
    let result = (|| {
        experiment.state = State::Running;
        experiment.save()?;
        logs::stage(logs::Stage::ArmBuilds);
        if experiment.description.prepared.is_none() {
            let description = &experiment.description;
            let requested = [
                arms::Requested::new(&description.baseline.commit, &description.baseline.harness)?,
                arms::Requested::new(
                    &description.candidate.commit,
                    &description.candidate.harness,
                )?,
            ];
            let record = experiment.directory.join("arms.json");
            if record.exists() {
                experiment.description.prepared =
                    Some(serde_json::from_slice(&std::fs::read(record)?)?);
            } else {
                experiment.description.prepared =
                    Some(arms::prepare(&repository, requested, &record)?);
            }
            experiment.save()?;
            logs::experiment(&experiment.description);
        }
        logs::stage(logs::Stage::EvalTasks);
        workers::run(&experiment)?;
        experiment.state = State::Completed;
        Ok(())
    })();
    let cause = result
        .as_ref()
        .err()
        .map(|error: &anyhow::Error| format!("{error:#}"));
    if result.is_err() {
        experiment.state = if interrupt::requested() {
            State::Interrupted
        } else {
            State::Paused
        };
    }
    let saved = experiment.save();
    if result.is_ok() && saved.is_ok() {
        logs::print(&experiment.render(options.json)?);
    }
    let outcome = if interrupt::requested() {
        "interrupted"
    } else if result.is_err() || saved.is_err() {
        "failed"
    } else {
        "completed"
    };
    logs::ended(outcome);
    logs::experiment_finished(outcome, cause.as_deref());
    progress::step(format!(
        "Experiment {} {outcome}",
        experiment.description.id
    ));
    result.and(saved)
}

pub fn show(args: &[String]) -> Result<String> {
    let json = args.iter().any(|arg| arg == "--json");
    let positional: Vec<_> = args.iter().filter(|arg| arg.as_str() != "--json").collect();
    ensure!(
        positional.len() <= 1,
        "usage: thirdshift experiments [<id>] [--json]"
    );
    let root = crate::config::home()?.join(".thirdshift/evals/experiments");
    store::show(&root, positional.first().map(|id| id.as_str()), json)
}

fn resolve(git: &Git, reference: &str) -> Result<String> {
    git.run(&[
        "--no-replace-objects",
        "rev-parse",
        "--verify",
        "--end-of-options",
        &format!("{reference}^{{commit}}"),
    ])
}

fn tasks(root: &Path, suites: &[String]) -> Result<Vec<Task>> {
    let mut tasks = Vec::new();
    for name in suites {
        let suite = root.join(name);
        for (task_name, directory) in super::suite::directories(&suite.join("tasks"))? {
            let manifest = format::Task::load(&directory)?;
            if manifest.state == format::State::Proposed {
                continue;
            }
            ensure!(
                manifest.session == format::Session::SpecReview,
                "Code-review Trials require #1016"
            );
            let inputs = manifest
                .trial
                .as_ref()
                .context("Eval task needs a [trial] table; see docs/eval-tasks.md")?;
            crate::issue::IssueUrl::parse(&inputs.issue_url)?;
            ensure!(
                !inputs.base.is_empty() && !inputs.branch.is_empty() && !inputs.pr_url.is_empty(),
                "Trial inputs must be nonempty"
            );
            tasks.push(Task {
                suite: name.clone(),
                name: task_name,
                directory,
                manifest,
            });
        }
    }
    ensure!(
        !tasks.is_empty(),
        "selected suites have no approved Eval tasks"
    );
    Ok(tasks)
}
