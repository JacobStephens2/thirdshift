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
    let scopes = viewer.scopes.clone();
    let csrf = viewer.csrf;
    match tokio::task::spawn_blocking(move || -> Result<Option<String>> {
        let config = UserConfig::load()?;
        let inspection = stop::inspect(&config, &command)?;
        // The confirmation page embeds the same Command details as Peek.
        // Act may stop the run; it does not reveal a Security run.
        if inspection.security_details && !scopes.contains(&super::Scope::Security) {
            return Ok(None);
        }
        Ok(Some(
            StopPage {
                page: super::PageContext::new(&config, &scopes),
                csrf: csrf.clone(),
                details: Peek::new(command, inspection, csrf, &config),
            }
            .render()?,
        ))
    })
    .await
    {
        Ok(Ok(Some(html))) => Html(html).into_response(),
        Ok(Ok(None)) => super::api::error(StatusCode::FORBIDDEN, "Missing scope: security"),
        _ => (StatusCode::NOT_FOUND, "Command details are unavailable.").into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct StopForm {
    command: String,
    #[serde(default)]
    confirmed: String,
    #[serde(default)]
    ending: String,
    #[serde(default)]
    requeue: bool,
}

pub(super) async fn submit(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<StopForm>,
) -> Response {
    if format == super::api::Format::Json {
        return super::api::action(format, "/stop", move || {
            stop::request_json(
                &form.command,
                stop::Request {
                    actor: viewer.activity_actor(),
                    requeue: form.requeue,
                },
            )
        })
        .await;
    }
    if form.confirmed != "yes" || !matches!(form.ending.as_str(), "triage" | "requeue") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let location = crate::logs::status::inspect::url("/stop", &form.command, None);
    let result = tokio::task::spawn_blocking(move || {
        stop::request_stop(
            &form.command,
            stop::Request {
                actor: viewer.activity_actor(),
                requeue: form.ending == "requeue",
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
