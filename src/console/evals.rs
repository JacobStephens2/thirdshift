//! Machine-local Eval data changes, distinct from people's act-scoped judgments.
use axum::{Extension, response::Response};

use super::{
    accounts::{Accounts, EventKind},
    api,
    auth::Viewer,
};

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
