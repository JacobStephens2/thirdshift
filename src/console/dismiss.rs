//! Authenticated form posts for the shared CLI Dismissal and Undo.
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Form};
use serde::Deserialize;

use super::auth::Viewer;
use crate::logs::dismiss;

#[derive(Deserialize)]
pub(super) struct DismissForm {
    command: String,
    #[serde(default)]
    machine: String,
}

pub(super) async fn submit(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<DismissForm>,
) -> Response {
    change(viewer, format, form, false).await
}

pub(super) async fn undo(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<DismissForm>,
) -> Response {
    change(viewer, format, form, true).await
}

async fn change(
    viewer: Viewer,
    format: super::api::Format,
    form: DismissForm,
    undo: bool,
) -> Response {
    if !form.machine.is_empty() && format == super::api::Format::Html {
        return super::fleet_actions::submit(
            viewer,
            form.machine,
            form.command,
            super::fleet_actions::Action::Dismiss { undo },
        )
        .await;
    }
    match tokio::task::spawn_blocking(move || {
        dismiss::request(
            &form.command,
            dismiss::Request {
                actor: viewer.activity_actor(),
                undo,
            },
        )
    })
    .await
    {
        Ok(Ok(change)) if format == super::api::Format::Json => {
            axum::Json(change.json()).into_response()
        }
        Ok(Ok(_)) => Redirect::to("/").into_response(),
        Ok(Err(error)) if format == super::api::Format::Json => {
            super::api::error(StatusCode::UNPROCESSABLE_ENTITY, error)
        }
        Ok(Err(error)) => (StatusCode::CONFLICT, error.to_string()).into_response(),
        Err(error) => super::page(Err(error)),
    }
}
