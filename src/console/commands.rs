//! Remaining CLI twins. Repository-dependent reads run in a child so they
//! cannot change the Console's current directory or install signal handlers.
use anyhow::{Context, Result, ensure};
use axum::{
    Extension, Json,
    extract::Path,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::Value;

use super::{api, auth::Viewer};
use crate::config::UserConfig;

pub(super) async fn help() -> Json<&'static str> {
    Json(crate::HELP)
}
pub(super) async fn version() -> Json<String> {
    Json(format!("thirdshift {}", env!("CARGO_PKG_VERSION")))
}

fn json(text: String) -> Result<Response> {
    Ok(Json(serde_json::from_str::<Value>(&text)?).into_response())
}

pub(super) async fn processes() -> Response {
    api::read(api::Format::Json, || {
        Ok(Json(crate::processes::execute(crate::args::ProcessAction::List)?).into_response())
    })
    .await
}

pub(super) fn process_name(name: &str) -> Result<()> {
    ensure!(
        crate::args::one_path_component(name),
        "name must be a Process name, not a path"
    );
    Ok(())
}

pub(super) async fn process(Path(name): Path<String>) -> Response {
    api::read(api::Format::Json, move || {
        process_name(&name)?;
        Ok(Json(crate::processes::execute(
            crate::args::ProcessAction::Show(name),
        )?)
        .into_response())
    })
    .await
}

pub(super) fn directory(repository: &str) -> Result<crate::launch_directories::Directory> {
    crate::launch_directories::load()?
        .directories
        .into_iter()
        .find(|directory| directory.repository.eq_ignore_ascii_case(repository))
        .with_context(|| format!("no known Launch directory for {repository}"))
}

pub(super) async fn evals() -> Response {
    api::read(api::Format::Json, || {
        json(crate::evals::execute(crate::evals::Action::List)?)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Touched {
    paths: Vec<String>,
}

pub(super) async fn touched(Json(request): Json<Touched>) -> Response {
    api::read(api::Format::Json, move || {
        let args = std::iter::once("touched".into())
            .chain(request.paths)
            .collect::<Vec<_>>();
        json(crate::evals::execute(crate::evals::parse(&args)?)?)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Verify {
    repository: String,
    base: String,
}

pub(super) async fn verify(Json(request): Json<Verify>) -> Response {
    api::read(api::Format::Json, move || {
        let directory = directory(&request.repository)?;
        let args = vec![
            "verify".into(),
            directory.path.to_string_lossy().into_owned(),
            request.base,
        ];
        json(crate::evals::execute(crate::evals::parse(&args)?)?)
    })
    .await
}

pub(super) async fn other_accounts() -> Response {
    api::read(api::Format::Json, || {
        json(crate::config::console::other_accounts(None, true)?)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Accounts {
    value: String,
}

pub(super) async fn set_other_accounts(
    Extension(viewer): Extension<Viewer>,
    Json(request): Json<Accounts>,
) -> Response {
    api::action(api::Format::Json, "/", move || {
        let value = crate::config::console::OtherAccounts::parse(&request.value)?
            .value()
            .to_string();
        super::user_config::change("console.other_accounts", &value, &viewer.caller_label)?;
        Ok(serde_json::from_str::<Value>(
            &crate::config::console::other_accounts(None, true)?,
        )?)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Email {
    address: Option<String>,
}

pub(super) async fn email_test(Json(request): Json<Email>) -> Response {
    api::action(api::Format::Json, "/", move || {
        crate::email::send_test(request.address, &UserConfig::load()?.email)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rename {
    name: String,
}

pub(super) async fn rename(Path(id): Path<String>, Json(request): Json<Rename>) -> Response {
    api::action(api::Format::Json, "/experiments", move || {
        Ok(serde_json::from_str::<Value>(
            &crate::evals::experiment::show(&[id, "--name".into(), request.name, "--json".into()])?,
        )?)
    })
    .await
}

pub(super) async fn difficulty(Path(id): Path<String>) -> Response {
    api::action(api::Format::Json, "/experiments", move || {
        Ok(serde_json::from_str::<Value>(
            &crate::evals::experiment::show(&[id, "--record-difficulty".into(), "--json".into()])?,
        )?)
    })
    .await
}

pub(super) async fn pending(Path(id): Path<String>) -> Response {
    api::read(api::Format::Json, move || {
        json(crate::evals::experiment::recheck(&[
            id,
            "--pending".into(),
            "--json".into(),
        ])?)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
enum Decision {
    Pass,
    Fail,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recheck {
    trial: Option<String>,
    fingerprint: Option<String>,
    decision: Decision,
    reason: String,
    wrong_finding: Option<bool>,
}

pub(super) async fn recheck(Path(id): Path<String>, Json(request): Json<Recheck>) -> Response {
    api::action(api::Format::Json, "/experiments", move || {
        let mut args = vec![id];
        match (request.trial, request.fingerprint) {
            (Some(trial), None) => {
                ensure!(!trial.starts_with("--"), "trial must be a Trial id");
                args.push(trial);
            }
            (None, Some(fingerprint)) => args.extend(["--batch".into(), fingerprint]),
            _ => anyhow::bail!("supply exactly one of trial or fingerprint"),
        }
        args.extend([
            match request.decision {
                Decision::Pass => "pass",
                Decision::Fail => "fail",
            }
            .into(),
            "--reason".into(),
            request.reason,
            "--json".into(),
        ]);
        if let Some(wrong) = request.wrong_finding {
            args.extend(["--wrong-finding".into(), wrong.to_string()]);
        }
        Ok(serde_json::from_str::<Value>(
            &crate::evals::experiment::recheck(&args)?,
        )?)
    })
    .await
}
