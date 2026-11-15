//! Machine-local Eval data changes, distinct from people's act-scoped judgments.
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure};
use axum::{Extension, Json, response::Response};
use serde::Deserialize;

use super::{
    accounts::{Accounts, EventKind},
    api,
    auth::Viewer,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProposeProcess {
    repository: String,
    limit: Option<u32>,
}

pub(super) async fn propose_process(
    Extension(format): Extension<api::Format>,
    Json(request): Json<ProposeProcess>,
) -> Response {
    api::action(format, "/experiments", move || {
        let directory = super::commands::directory(&request.repository)?;
        let args = vec![
            "propose-process".into(),
            directory.path.to_string_lossy().into_owned(),
            "--limit".into(),
            request.limit.unwrap_or(100).to_string(),
        ];
        Ok(serde_json::from_str::<serde_json::Value>(
            &crate::evals::execute(crate::evals::parse(&args)?)?,
        )?)
    })
    .await
}

pub(super) async fn import_replays(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<api::Format>,
) -> Response {
    api::action(format, "/experiments", move || {
        let accounts = Accounts::load()?;
        let result = crate::evals::execute(crate::evals::Action::ImportReplays)?;
        accounts.record(&viewer.caller_label, EventKind::EvalsImported)?;
        Ok(serde_json::from_str::<serde_json::Value>(&result)?)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckRunner {
    runner: String,
    suite: Option<String>,
    task: Option<String>,
}

/// The CLI twin runs as its own process, as `process check` does, so a dry
/// run's Trial, interruption handling and progress stay out of the Console.
/// Its `--json` report is the response, whether or not the runner passed.
pub(super) async fn check_runner(Json(request): Json<CheckRunner>) -> Response {
    api::action(api::Format::Json, "/", move || {
        // The Console's working directory means nothing to its caller.
        ensure!(
            !request.runner.ends_with(".toml") || Path::new(&request.runner).is_absolute(),
            "a Custom runner file must be an absolute path"
        );
        let mut args = vec!["check-runner".to_owned(), request.runner];
        for (flag, value) in [("--suite", request.suite), ("--task", request.task)] {
            if let Some(value) = value {
                args.extend([flag.to_owned(), value]);
            }
        }
        args.push("--json".into());
        crate::evals::parse(&args)?;
        let timeout = crate::evals::experiment::Bar::default().timeout_seconds;
        let output = crate::process::output_until(
            Command::new(std::env::current_exe()?)
                .arg("evals")
                .args(&args),
            None,
            crate::process::Control {
                name: "Custom runner check",
                interruption: crate::process::Interruption::Ordinary,
                stop: &|child| crate::process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
            },
            // The Trial and its code checks each have the Bar's timeout.
            Instant::now() + Duration::from_secs(3 * timeout),
            &|| Ok(()),
        )?;
        match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            Ok(report) if output.status.success() || output.status.code() == Some(1) => Ok(report),
            _ => bail!("{}", String::from_utf8_lossy(&output.stderr).trim()),
        }
    })
    .await
}
