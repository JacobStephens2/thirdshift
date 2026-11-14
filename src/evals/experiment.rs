//! A resumable Experiment owns its fixed inputs and Trial artifacts. Command
//! recording stays in the ordinary logs; private Eval data stays here.

mod accounting;
mod coordination;
mod cost;
pub(crate) mod deciders;
mod difficulty;
mod efficiency;
mod evidence;
mod execution;
mod grading;
mod integrity;
mod leaks;
mod model_grader;
mod names;
mod options;
pub(crate) mod pairs;
pub(crate) mod pairwise;
mod panel;
mod reading;
mod rechecks;
mod release;
mod retention;
mod session_evidence;
mod store;
mod trace;
mod trial;
mod trial_results;
mod trial_sessions;
mod usage;
mod workers;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{arms, format, suite};
use crate::config::UserConfig;
use crate::git::Git;
use crate::harness::{Choice, ChosenBy, Harness};
use crate::issue::Repo;
use crate::trial::VoidCause;
use crate::verdict::{Levels, decimal};
use crate::{interrupt, logs, progress};
pub(crate) use arms::{Arm as PreparedArm, Runner as CustomRunner};

pub(crate) use evidence::RunningTrial;
pub(crate) use grading::{Conclusion, Flip, Pending};
pub(crate) use leaks::decide as decide_leak;
pub(crate) use names::setup_changes;
pub use options::{Options, parse};
pub(crate) use rechecks::record as recheck;
pub(crate) use session_evidence::{SessionLog, read as read_session};
pub(crate) use store::{State, View, read};
pub(crate) use trial_results::{Arm, TrialResult, trial_key};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Bar {
    #[serde(deserialize_with = "decimal")]
    pub margin: f64,
    #[serde(flatten)]
    pub levels: Levels,
    #[serde(deserialize_with = "decimal")]
    pub prior: f64,
    pub seed: u64,
    pub draws: usize,
    pub trials_per_task: BTreeMap<String, usize>,
    /// Absent in Experiments that judged every approved Eval task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminating_tasks: Option<BTreeMap<String, Vec<String>>>,
    /// Absent in saved Experiments from before Advisory suites.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_discriminating_tasks: Option<usize>,
    pub timeout_seconds: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "efficiency::optional_decimal"
    )]
    pub cost_ceiling: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "efficiency::optional_decimal"
    )]
    pub time_ceiling: Option<f64>,
    pub void_rules: VoidRules,
    /// Absent when no Grader panel grades the pairs of Trials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<panel::Panel>,
    /// Absent when anyone may read any pair, beside the Verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub people: Option<deciders::PeopleSetting>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct VoidRules {
    pub retries: usize,
    pub causes: Vec<VoidCause>,
}

impl Default for Bar {
    fn default() -> Self {
        let statistics = crate::verdict::Bar::default();
        Self {
            margin: statistics.margin,
            levels: statistics.levels,
            prior: statistics.prior,
            seed: statistics.seed,
            draws: statistics.draws,
            trials_per_task: BTreeMap::new(),
            discriminating_tasks: None,
            min_discriminating_tasks: None,
            timeout_seconds: 1800,
            cost_ceiling: None,
            time_ceiling: None,
            void_rules: VoidRules {
                retries: 2,
                causes: [
                    VoidCause::SessionUnavailable,
                    VoidCause::UsageLimit,
                    VoidCause::RateLimit,
                    VoidCause::Interrupted,
                    VoidCause::RepositoryChanged,
                    VoidCause::SandboxUnavailable,
                    VoidCause::HostKilled,
                    VoidCause::MemoryCap,
                ]
                .into(),
            },
            panel: None,
            people: None,
        }
    }
}

impl Bar {
    fn advisory(&self, suite: &str) -> Option<bool> {
        self.min_discriminating_tasks.map(|minimum| {
            self.discriminating_tasks
                .as_ref()
                .and_then(|suites| suites.get(suite))
                .map_or(0, Vec::len)
                < minimum
        })
    }

    fn judges(&self, suite: &str, task: &str) -> bool {
        self.discriminating_tasks.as_ref().is_none_or(|suites| {
            suites
                .get(suite)
                .is_some_and(|tasks| tasks.iter().any(|name| name == task))
        })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Setup {
    pub commit: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_digest: Option<String>,
}

impl Setup {
    fn requested(&self) -> Result<arms::Requested> {
        if let Some(runner) = &self.runner {
            return Ok(arms::Requested::runner(runner));
        }
        Ok(arms::Requested::new(
            self.commit.as_deref().context("missing Arm commit")?,
            self.harness.as_deref().context("missing Arm Harness")?,
        )?
        .with_process(
            self.process.as_deref(),
            self.model.as_deref(),
            self.effort.as_deref(),
        ))
    }

    fn choice(&self) -> Choice {
        Choice {
            harness: self
                .harness
                .as_deref()
                .and_then(Harness::named)
                .unwrap_or(Harness::Claude),
            model: self.model.clone(),
            effort: self.effort.clone(),
            chosen_by: ChosenBy::Command,
        }
    }

    pub(crate) fn reference(&self) -> &str {
        self.runner
            .as_deref()
            .or(self.commit.as_deref())
            .unwrap_or("unknown")
    }

    pub(crate) fn executor(&self) -> &str {
        self.runner
            .as_deref()
            .or(self.harness.as_deref())
            .unwrap_or("unknown")
    }
}

/// Only these public Experiment facts enter factory logs, never task inputs.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Description {
    pub id: String,
    pub baseline: Setup,
    pub candidate: Setup,
    pub suites: Vec<String>,
    pub bar: Bar,
    pub prepared: Option<arms::Prepared>,
}

impl Description {
    pub(crate) fn runner(&self, arm: Arm) -> Option<&arms::Runner> {
        let prepared = self.prepared.as_ref()?;
        match arm {
            Arm::Baseline => prepared.baseline.runner.as_ref(),
            Arm::Candidate => prepared.candidate.runner.as_ref(),
        }
    }

    pub(crate) fn setup(&self, arm: Arm) -> &Setup {
        match arm {
            Arm::Baseline => &self.baseline,
            Arm::Candidate => &self.candidate,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Task {
    suite: String,
    name: String,
    directory: std::path::PathBuf,
    manifest: format::Task,
}

fn validate_runner_suites(options: &Options, root: &Path) -> Result<()> {
    if options.baseline_setup.runner.is_none() && options.candidate_setup.runner.is_none() {
        return Ok(());
    }
    for name in &options.suites {
        ensure!(
            !matches!(name.as_str(), "code-review" | "spec-review"),
            "Custom runners require process suites: {name}"
        );
        suite::Manifest::load(&root.join(name))?;
        for (_, directory) in suite::directories(&root.join(name).join("tasks"))? {
            ensure!(
                format::Task::load(&directory)?.session == format::Session::Process,
                "Custom runners require process suites: {name}"
            );
        }
    }
    Ok(())
}

fn process_setups(setups: [&mut Setup; 2]) {
    for setup in setups {
        if setup.runner.is_none() {
            setup.process.get_or_insert_with(|| "ticket-run".into());
        }
    }
}

struct SelectedArms {
    baseline: Setup,
    candidate: Setup,
    baseline_reference: Option<String>,
}

/// Both CLI paths resolve native builds and Custom runners under one policy.
fn select_arms(options: &Options, git: &Git, config: &UserConfig) -> Result<SelectedArms> {
    let candidate = if options.candidate_setup.runner.is_some() {
        String::new()
    } else {
        resolve(git, &options.candidate)?
    };
    let baseline_reference = if options.release && options.baseline.is_none() {
        Some(git.run(&[
            "--no-replace-objects",
            "describe",
            "--tags",
            "--abbrev=0",
            "--match",
            "v[0-9]*",
            &candidate,
        ])?)
    } else {
        options.baseline.clone().or_else(|| {
            (options.candidate_setup.runner.is_some() && options.baseline_setup.runner.is_none())
                .then(|| "HEAD".into())
        })
    };
    let baseline = if options.baseline_setup.runner.is_some() {
        String::new()
    } else {
        match &baseline_reference {
            Some(reference) => resolve(git, reference)?,
            None => git.run(&["--no-replace-objects", "merge-base", "main", &candidate])?,
        }
    };
    Ok(SelectedArms {
        baseline: options.baseline_setup.resolve(baseline, config),
        candidate: options.candidate_setup.resolve(candidate, config),
        baseline_reference,
    })
}

pub(super) fn prepare_arms(
    repository: &Path,
    options: Options,
    record: &Path,
) -> Result<arms::Prepared> {
    let root = crate::config::home()?.join(".thirdshift/evals/suites");
    validate_runner_suites(&options, &root)?;
    let SelectedArms {
        mut baseline,
        mut candidate,
        ..
    } = select_arms(&options, &Git::new(repository), &UserConfig::load()?)?;
    if baseline.process.is_some()
        || candidate.process.is_some()
        || baseline.runner.is_some()
        || candidate.runner.is_some()
    {
        process_setups([&mut baseline, &mut candidate]);
    }
    arms::prepare(
        repository,
        [baseline.requested()?, candidate.requested()?],
        record,
    )
}

pub fn run(options: Options) -> Result<()> {
    interrupt::install()?;
    let repository = std::env::current_dir()?;
    let git = Git::new(&repository);
    let root = crate::config::home()?.join(".thirdshift/evals");
    let suites_root = root.join("suites");
    validate_runner_suites(&options, &suites_root)?;
    let config = UserConfig::load()?;
    let SelectedArms {
        mut baseline,
        mut candidate,
        baseline_reference,
    } = select_arms(&options, &git, &config)?;
    let suites = if options.suites.is_empty() {
        let paths = git.run(&[
            "--no-replace-objects",
            "diff",
            "--name-only",
            "--no-renames",
            baseline
                .commit
                .as_deref()
                .context("missing Baseline commit for suite selection")?,
            candidate
                .commit
                .as_deref()
                .context("missing Candidate commit for suite selection")?,
            "--",
        ])?;
        suite::touched(
            &suites_root,
            &paths.lines().map(String::from).collect::<Vec<_>>(),
        )?
    } else {
        options.suites.clone()
    };
    if suites.is_empty() && options.release {
        return release::skip(
            "no_suites",
            "no Eval suite touched; no release Experiment needed",
            options.json,
        );
    }
    ensure!(
        !suites.is_empty(),
        "no Eval suites selected; use --suite for a defaults-only Experiment"
    );
    let mut bar = options.bar;
    // Keep the judged list and the retained manifest snapshots on one reading
    // of Difficulty, fenced against approval and Difficulty publication.
    let suite_locks = suites
        .iter()
        .map(|name| suite::lock(&suites_root.join(name)))
        .collect::<Result<Vec<_>>>()?;
    let mut discriminating = BTreeMap::new();
    for name in &suites {
        let manifest = suite::Manifest::load(&suites_root.join(name))?;
        bar.trials_per_task.insert(
            name.clone(),
            options
                .trials_per_task
                .unwrap_or(manifest.trials_per_arm.get()),
        );
        let mut listed = Vec::new();
        for (name, directory) in suite::directories(&suites_root.join(name).join("tasks"))? {
            let task = format::Task::load(&directory)?;
            if task.state == format::State::Approved && task.session == format::Session::Process {
                process_setups([&mut baseline, &mut candidate]);
            }
            if task.state == format::State::Approved
                && task
                    .difficulty
                    .as_ref()
                    .is_some_and(format::Difficulty::discriminating)
            {
                listed.push(name);
            }
        }
        discriminating.insert(name.clone(), listed);
    }
    bar.discriminating_tasks = Some(discriminating);
    ensure!(
        bar.trials_per_task.values().all(|n| matches!(n, 1 | 3)),
        "Trials per Eval task must be one or three"
    );
    for setup in [&mut baseline, &mut candidate] {
        if let Some(runner) = &setup.runner {
            setup.runner_digest = Some(arms::Runner::source_digest(runner)?);
        } else {
            setup.process_digest = arms::Process::source_digest(&setup.requested()?)?;
        }
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
    panel::check_stand_in(&description)?;
    let fresh = !root
        .join("experiments")
        .join(&description.id)
        .join("experiment.json")
        .exists();
    let Some(mut experiment) =
        store::Stored::open(&root.join("experiments"), description, |description| {
            // The release Baseline probe can build an Arm too. Check new
            // Experiments first; resumes keep the tasks already recorded.
            let tasks = tasks(&suites_root, &description.suites)?;
            deciders::check(&description.bar, &tasks)?;
            if options.release
                && !release::supports_trials(
                    &repository,
                    &root,
                    description
                        .baseline
                        .commit
                        .as_deref()
                        .context("release Baseline has no commit")?,
                )?
            {
                return Ok(None);
            }
            Ok(Some(tasks))
        })?
    else {
        let tag = baseline_reference.as_deref().expect("release Baseline tag");
        return release::skip(
            "baseline_without_trial",
            &format!("last release {tag} has no Trial flag; carrying on without an Experiment"),
            options.json,
        );
    };
    drop(suite_locks);
    if let Some(path) = &options.prices {
        let prices = crate::trial::accounting::Prices::load(path)?;
        ensure!(
            !experiment.directory.join("started").exists()
                || experiment.pricing.as_ref() == Some(&prices),
            "price table cannot change after the first Trial"
        );
        experiment.pricing = Some(prices);
        experiment.save()?;
    }
    if let Some(name) = options.name {
        experiment.rename(Some(name))?;
    }
    if experiment.state == State::Completed {
        retention::repositories(&experiment);
        println!("{}", experiment.render(options.json)?);
        logs::start_notice::skipped(format!(
            "Experiment {} is already completed",
            experiment.description.id
        ));
        return Ok(());
    }
    if let Err(error) = cost::confirm(&experiment, options.yes) {
        // A declined new Experiment leaves no record to list as running.
        if fresh {
            experiment.discard()?;
        }
        return Err(error);
    }
    if experiment.details.candidate_ref.is_none() {
        experiment.details.baseline_ref =
            Some(baseline_reference.unwrap_or_else(|| "main (merge base)".into()));
        experiment.details.candidate_ref = Some(
            options
                .candidate_setup
                .runner
                .as_deref()
                .unwrap_or(&options.candidate)
                .to_owned(),
        );
        experiment.details.title = if let Some(commit) = &experiment.description.candidate.commit {
            Some(git.run(&["log", "-1", "--format=%s", commit])?)
        } else {
            Some(format!(
                "Custom runner {}",
                experiment.description.candidate.reference()
            ))
        };
    }
    experiment.details.release |= options.release;
    experiment.details.resume_command = Some(
        std::iter::once("thirdshift".to_owned())
            .chain(
                resume_args(std::env::args().skip(1))
                    .into_iter()
                    .map(|arg| format!("'{}'", arg.replace('\'', "'\\''"))),
            )
            .collect::<Vec<_>>()
            .join(" "),
    );
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
    logs::resumable(experiment.details.resume_command.as_deref().unwrap());
    experiment.details.command_log = logs::command_log_path();
    experiment.details.ended_at = None;
    experiment.details.pause_reason = None;
    let result = (|| {
        experiment.state = State::Running;
        experiment.save()?;
        coordination::start(&mut experiment, options.no_wait)?;
        logs::stage(logs::Stage::ArmBuilds);
        if experiment.description.prepared.is_none() {
            let description = &experiment.description;
            let requested = [
                description.baseline.requested()?,
                description.candidate.requested()?,
            ];
            let record = experiment.directory.join("arms.json");
            if record.exists() {
                experiment.description.prepared =
                    Some(serde_json::from_slice(&std::fs::read(record)?)?);
            } else {
                experiment.description.prepared =
                    Some(arms::prepare(&repository, requested, &record)?);
            }
            let prepared = experiment
                .description
                .prepared
                .as_ref()
                .context("missing prepared Arms")?;
            for (setup, arm) in [
                (&experiment.description.baseline, &prepared.baseline),
                (&experiment.description.candidate, &prepared.candidate),
            ] {
                if let Some(expected) = &setup.runner_digest {
                    ensure!(
                        arm.runner
                            .as_ref()
                            .context("missing frozen Custom runner")?
                            .digest()?
                            == *expected,
                        "Custom runner inputs changed during Arm preparation; start the Experiment again"
                    );
                }
                if let Some(expected) = &setup.process_digest {
                    ensure!(
                        arm.process
                            .as_ref()
                            .context("missing frozen Arm Process")?
                            .digest()?
                            == *expected,
                        "Custom process inputs changed during Arm preparation; start the Experiment again"
                    );
                }
            }
            experiment.save()?;
            logs::experiment(&experiment.description);
        }
        logs::stage(logs::Stage::EvalTasks);
        model_grader::prepare(&experiment, &config.harness)?;
        panel::prepare(&mut experiment)?;
        workers::run(&experiment)?;
        pairs::run(&experiment)?;
        // The panel grades last, so a pause in it holds back no model Score.
        model_grader::run(&experiment)?;
        panel::run(&experiment)?;
        experiment.refresh_verdict()?;
        experiment.state = State::Completed;
        Ok(())
    })();
    let cause = result
        .as_ref()
        .err()
        .map(|error: &anyhow::Error| format!("{error:#}"));
    coordination::publish_waiting_for(&mut experiment, &[]);
    if result.is_err() {
        experiment.state = if interrupt::requested() {
            State::Interrupted
        } else {
            State::Paused
        };
    }
    experiment.details.ended_at =
        (experiment.state == State::Completed).then(|| chrono::Local::now().to_rfc3339());
    experiment.details.pause_reason = cause.clone();
    let saved = experiment.save();
    if result.is_ok() && saved.is_ok() {
        retention::repositories(&experiment);
        // Results contain private Grader evidence, unlike the public Command
        // facts. Print them without copying them into the factory's logs.
        println!("{}", experiment.render(options.json)?);
    }
    let outcome = if interrupt::requested() {
        "interrupted"
    } else if saved.is_ok()
        && result
            .as_ref()
            .err()
            .is_some_and(|error| error.is::<crate::session::limits::Reached>())
    {
        let reached = result
            .as_ref()
            .unwrap_err()
            .downcast_ref::<crate::session::limits::Reached>()
            .unwrap();
        logs::paused(&reached.0);
        "paused"
    } else if result.is_err() || saved.is_err() {
        "failed"
    } else {
        experiment
            .verdict
            .as_ref()
            .context("completed Experiment has no Verdict")?
            .outcome()
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
    let root = crate::config::home()?.join(".thirdshift/evals/experiments");
    if let [id, flag, hit, decision, reason_flag, reason] = positional.as_slice()
        && flag.as_str() == "--leak"
        && reason_flag.as_str() == "--reason"
    {
        return decide_leak(
            id,
            hit,
            decision,
            reason,
            logs::Actor::cli("a leak decision")?,
            json,
        );
    }
    if let [id, flag, pair, rest @ ..] = positional.as_slice()
        && flag.as_str() == "--pair"
    {
        let page = match rest {
            [] => pairwise::page(id, pair, &logs::Actor::cli("a blind pair")?)?,
            [flag, a, b] if flag.as_str() == "--merge" => {
                pairwise::merge(id, pair, a, b, logs::Actor::cli("merge calls")?)?
            }
            [flag, preference, rubric @ ..] if flag.as_str() == "--prefer" => {
                let mut graded = Vec::new();
                for chunk in rubric.chunks(2) {
                    let [flag, value] = chunk else {
                        anyhow::bail!("--rubric needs <criterion>=<a|b|tie|both-bad>");
                    };
                    let (criterion, value) = value
                        .split_once('=')
                        .filter(|_| flag.as_str() == "--rubric")
                        .context("--rubric needs <criterion>=<a|b|tie|both-bad>")?;
                    graded.push((criterion.to_owned(), value.to_owned()));
                }
                pairwise::prefer(
                    id,
                    pair,
                    preference,
                    &graded,
                    logs::Actor::cli("a preference")?,
                )?
            }
            _ => anyhow::bail!("{PAIR_USAGE}"),
        };
        return if json {
            Ok(serde_json::to_string_pretty(&page)?)
        } else {
            Ok(pairwise::text(&page))
        };
    }
    if let [id, flag] = positional.as_slice()
        && flag.as_str() == "--record-difficulty"
    {
        return difficulty::record(&root, id, json);
    }
    if let [id, flag, name] = positional.as_slice()
        && flag.as_str() == "--name"
    {
        return store::rename(&root, id, names::parse(name)?, json);
    }
    if let [id, flag, session] = positional.as_slice()
        && flag.as_str() == "--session"
    {
        let session = read_session(id, session)?;
        return Ok(if json {
            serde_json::to_string_pretty(&session)?
        } else {
            session.contents
        });
    }
    ensure!(
        positional.len() <= 1,
        "usage: thirdshift experiments [<id>] [--json], experiments <id> --record-difficulty [--json], experiments <id> --name <text> [--json], experiments <id> --session <session-id> [--json], experiments <id> --leak <hit-id> <confirm|dismiss> --reason <text> [--json] or {PAIR_USAGE}"
    );
    store::show(&root, positional.first().map(|id| id.as_str()), json)
}

const PAIR_USAGE: &str = "experiments <id> --pair <pair-id> [--merge <ready|not-ready> <ready|not-ready> | --prefer <a|b|tie|both-bad> [--rubric <criterion>=<a|b|tie|both-bad>]...] [--json]";

/// The shown resume command leaves out `--name`, so following it keeps a
/// later rename. Flags pair with values as `options::parse` reads them.
fn resume_args(mut args: impl Iterator<Item = String>) -> Vec<String> {
    let mut kept = Vec::new();
    while let Some(arg) = args.next() {
        if matches!(arg.as_str(), "--json" | "--release" | "--yes" | "--no-wait")
            || !arg.starts_with("--")
        {
            kept.push(arg);
        } else if arg == "--name" {
            args.next();
        } else {
            kept.push(arg);
            kept.extend(args.next());
        }
    }
    kept
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
    let mut missing = Vec::new();
    for name in suites {
        let suite = root.join(name);
        for (task_name, directory) in super::suite::directories(&suite.join("tasks"))? {
            let manifest = format::Task::load(&directory)?;
            if manifest.state == format::State::Proposed {
                continue;
            }
            let Some(inputs) = manifest.trial.as_ref() else {
                missing.push(format!("{name}/{task_name}"));
                continue;
            };
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
        missing.is_empty(),
        "Eval tasks need a [trial] table: {}; see docs/eval-tasks.md",
        missing.join(", ")
    );
    ensure!(
        !tasks.is_empty(),
        "selected suites have no approved Eval tasks"
    );
    Ok(tasks)
}
