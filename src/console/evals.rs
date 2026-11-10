//! Machine-local Eval data changes, distinct from people's act-scoped judgments.
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
