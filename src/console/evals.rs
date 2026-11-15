//! Machine-local Eval data changes, distinct from people's act-scoped judgments.
use std::time::Duration;

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

/// A caller-selected local file stays CLI-only (ADR 0026): reading any
/// `.toml` path would echo its text back in the manifest problem. The API
/// names Custom runners under `~/.thirdshift/runners/` instead.
pub(in crate::console) fn named_runner(runner: &str) -> anyhow::Result<()> {
    ensure!(
        !runner.ends_with(".toml"),
        "the API names Custom runners; a .toml file path is CLI-only"
    );
    Ok(())
}

/// The CLI twin runs as its own process, as `process check` does, so a dry
/// run's Trial, interruption handling and progress stay out of the Console.
/// Its `--json` report is the response, whether or not the runner passed.
pub(super) async fn check_runner(Json(request): Json<CheckRunner>) -> Response {
    api::action(api::Format::Json, "/", move || {
        named_runner(&request.runner)?;
        let mut args = vec![
            "evals".to_owned(),
            "check-runner".to_owned(),
            request.runner,
        ];
        for (flag, value) in [("--suite", request.suite), ("--task", request.task)] {
            if let Some(value) = value {
                args.extend([flag.to_owned(), value]);
            }
        }
        args.push("--json".into());
        crate::evals::parse(&args[1..])?;
        let timeout = crate::evals::experiment::Bar::default().timeout_seconds;
        // The Trial and its code checks each have the Bar's timeout.
        let output = super::commands::cli_twin(
            "Custom runner check",
            &args,
            None,
            Duration::from_secs(3 * timeout),
        )?;
        match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            Ok(report) if output.status.success() || output.status.code() == Some(1) => Ok(report),
            _ => bail!("{}", String::from_utf8_lossy(&output.stderr).trim()),
        }
    })
    .await
}
