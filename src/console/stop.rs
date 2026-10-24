//! Confirmation and observation of the shared CLI Stop.
use anyhow::Result;
use askama::Template;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::{Extension, Form, extract::RawQuery, http::StatusCode};
use serde::Deserialize;

use super::{auth::Viewer, peek::Peek};
use crate::{config::UserConfig, logs::stop};

#[derive(Template)]
#[template(path = "console/stop.html")]
struct StopPage {
    page: super::PageContext,
    csrf: String,
    details: Peek,
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    RawQuery(query): RawQuery,
) -> Response {
    let command = super::query_value(&query, "command");
    match tokio::task::spawn_blocking(move || -> Result<String> {
        let config = UserConfig::load()?;
        let inspection = stop::inspect(&config, &command)?;
        Ok(StopPage {
            page: super::PageContext::new(&config),
            csrf: viewer.csrf.clone(),
            details: Peek::new(command, inspection, viewer.csrf),
        }
        .render()?)
    })
    .await
    {
        Ok(Ok(html)) => Html(html).into_response(),
        _ => (StatusCode::NOT_FOUND, "Command details are unavailable.").into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct StopForm {
    command: String,
    #[serde(default)]
    confirmed: String,
    ending: String,
}

pub(super) async fn submit(
    Extension(viewer): Extension<Viewer>,
    Form(form): Form<StopForm>,
) -> Response {
    if form.confirmed != "yes" || !matches!(form.ending.as_str(), "triage" | "requeue") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let location = crate::logs::status::inspect::url("/stop", &form.command, None);
    let result = tokio::task::spawn_blocking(move || {
        stop::request_stop(
            &form.command,
            stop::Request {
                who: viewer.email,
                requeue: form.ending == "requeue",
                from_console: true,
            },
        )
    })
    .await;
    match result {
        Ok(Ok(_)) => Redirect::to(&location).into_response(),
        Ok(Err(error)) => (StatusCode::CONFLICT, error.to_string()).into_response(),
        Err(error) => super::page(Err(error)),
    }
}
