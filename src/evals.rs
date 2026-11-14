//! Private Eval tasks: manifests, suite selection and time-safe repositories.

use std::path::PathBuf;

use anyhow::{Result, bail};

pub(crate) mod approval;
mod arms;
pub(crate) mod bug_origin;
pub(crate) mod experiment;
mod format;
mod import;
mod process_tasks;
mod repository;
mod suite;

pub enum Action {
    List,
    ImportReplays,
    ProposeProcess {
        repository: PathBuf,
        limit: u32,
    },
    Approve {
        suite: String,
        task: String,
    },
    Touched(Vec<String>),
    Prepare {
        suite: String,
        task: String,
        destination: PathBuf,
    },
    Verify {
        repository: PathBuf,
        base: String,
    },
    PrepareArms {
        repository: PathBuf,
        arms: [arms::Requested; 2],
        record: PathBuf,
    },
    PrepareSelectedArms {
        repository: PathBuf,
        options: Box<experiment::Options>,
        record: PathBuf,
    },
}

pub fn parse(args: &[String]) -> Result<Action> {
    match args {
        [] => Ok(Action::List),
        [flag] if flag == "--json" => Ok(Action::List),
        [action, flag] if action == "list" && flag == "--json" => Ok(Action::List),
        [action, suite, task] if action == "approve" => {
            format::name(suite)?;
            format::name(task)?;
            Ok(Action::Approve {
                suite: suite.clone(),
                task: task.clone(),
            })
        }
        [action, suite, task, flag] if action == "approve" && flag == "--json" => {
            parse(&[action.clone(), suite.clone(), task.clone()])
        }
        [action] if action == "list" => Ok(Action::List),
        [action] if action == "import-replays" => Ok(Action::ImportReplays),
        [action, repository] if action == "propose-process" => Ok(Action::ProposeProcess {
            repository: repository.into(),
            limit: 100,
        }),
        [action, repository, flag, limit] if action == "propose-process" && flag == "--limit" => {
            let limit: u32 = limit.parse()?;
            anyhow::ensure!((1..=100).contains(&limit), "proposal limit must be 1..100");
            Ok(Action::ProposeProcess {
                repository: repository.into(),
                limit,
            })
        }
        [action, paths @ ..] if action == "touched" => {
            for path in paths {
                format::relative_path(path)?;
            }
            Ok(Action::Touched(paths.to_vec()))
        }
        [action, suite, task, destination] if action == "prepare" => {
            format::name(suite)?;
            format::name(task)?;
            Ok(Action::Prepare {
                suite: suite.clone(),
                task: task.clone(),
                destination: destination.into(),
            })
        }
        [action, repository, base] if action == "verify" => {
            format::commit_id(base)?;
            Ok(Action::Verify {
                repository: repository.into(),
                base: base.clone(),
            })
        }
        [action, repository, record, options @ ..]
            if action == "prepare-arms"
                && options.iter().any(|flag| {
                    matches!(flag.as_str(), "--baseline-runner" | "--candidate-runner")
                }) =>
        {
            Ok(Action::PrepareSelectedArms {
                repository: repository.into(),
                record: record.into(),
                options: Box::new(experiment::parse(options)?),
            })
        }
        [
            action,
            repository,
            baseline,
            baseline_harness,
            candidate,
            candidate_harness,
            record,
            options @ ..,
        ] if action == "prepare-arms" => {
            let mut arms = [
                arms::Requested::new(baseline, baseline_harness)?,
                arms::Requested::new(candidate, candidate_harness)?,
            ];
            arms::Requested::options(&mut arms, options)?;
            Ok(Action::PrepareArms {
                repository: repository.into(),
                arms,
                record: record.into(),
            })
        }
        _ => bail!(
            "usage: thirdshift evals list | approve <suite> <task> [--json] | import-replays | propose-process <repository> [--limit <1..100>] | touched [<path>...] | prepare <suite> <task> <destination> | verify <repository> <base> | prepare-arms <repository> <baseline-commit> <baseline-harness> <candidate-commit> <candidate-harness> <record.json>"
        ),
    }
}

pub fn execute(action: Action) -> Result<String> {
    let root = crate::config::home()?.join(".thirdshift/evals/suites");
    match action {
        Action::ProposeProcess { repository, limit } => Ok(serde_json::to_string_pretty(
            &process_tasks::propose(&repository, limit)?,
        )?),
        Action::Approve { suite, task } => Ok(serde_json::to_string_pretty(&approval::approve(
            &suite, &task,
        )?)?),
        Action::List => Ok(serde_json::to_string_pretty(&suite::list(&root)?)?),
        Action::ImportReplays => Ok(serde_json::to_string(&import::replays(
            &crate::config::home()?,
        )?)?),
        Action::Touched(paths) => Ok(serde_json::to_string_pretty(&suite::touched(
            &root, &paths,
        )?)?),
        Action::Prepare {
            suite,
            task,
            destination,
        } => {
            let suite = root.join(suite);
            suite::Manifest::load(&suite)?;
            let directory = suite.join("tasks").join(task);
            let task = format::Task::load(&directory)?;
            Ok(serde_json::to_string_pretty(&repository::prepare(
                &directory,
                &task,
                &destination,
            )?)?)
        }
        Action::Verify { repository, base } => {
            repository::verify(&repository, &base)?;
            Ok(r#"{"safe":true}"#.into())
        }
        Action::PrepareArms {
            repository,
            arms,
            record,
        } => Ok(serde_json::to_string_pretty(&arms::prepare(
            &repository,
            arms,
            &record,
        )?)?),
        Action::PrepareSelectedArms {
            repository,
            options,
            record,
        } => Ok(serde_json::to_string_pretty(&experiment::prepare_arms(
            &repository,
            *options,
            &record,
        )?)?),
    }
}
