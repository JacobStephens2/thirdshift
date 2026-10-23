//! Private Eval tasks: manifests, suite selection and time-safe repositories.

use std::path::PathBuf;

use anyhow::{Result, bail};

mod format;
mod repository;
mod suite;

pub enum Action {
    List,
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
}

pub fn parse(args: &[String]) -> Result<Action> {
    match args {
        [action] if action == "list" => Ok(Action::List),
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
        _ => bail!(
            "usage: thirdshift evals list | touched [<path>...] | prepare <suite> <task> <destination> | verify <repository> <base>"
        ),
    }
}

pub fn execute(action: Action) -> Result<String> {
    let root = crate::config::home()?.join(".thirdshift/evals/suites");
    match action {
        Action::List => Ok(serde_json::to_string_pretty(&suite::list(&root)?)?),
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
    }
}
