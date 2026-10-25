//! Retained Bug origins and the CLI's confirm, correct and reject actions.
use anyhow::{Result, bail};
use askama::Template;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Form};
use serde::Deserialize;

use super::auth::Viewer;
use crate::config::UserConfig;
use crate::scoring::origins;

#[derive(Template)]
#[template(path = "console/bug_origins.html")]
struct BugOriginsPage {
    page: super::PageContext,
    origins: Vec<origins::Proposal>,
    csrf: String,
}

pub(super) async fn show(Extension(viewer): Extension<Viewer>) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            let config = UserConfig::load()?;
            Ok(BugOriginsPage {
                page: super::PageContext::new(&config),
                origins: origins::view()?,
                csrf: viewer.csrf,
            }
            .render()?)
        })
        .await,
    )
}

#[derive(Deserialize)]
pub(super) struct SettlementForm {
    issue: String,
    action: String,
    #[serde(default)]
    pull_request: String,
}

pub(super) async fn submit(Form(form): Form<SettlementForm>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let mut args = vec![form.issue];
        match form.action.as_str() {
            "confirm" => args.push("--confirm".into()),
            "correct" => {
                args.push("--correct".into());
                args.push(form.pull_request);
            }
            "reject" => args.push("--none".into()),
            _ => bail!("choose confirm, correct or reject"),
        }
        args.push("--json".into());
        origins::apply(origins::parse(&args)?)
    })
    .await;
    match result {
        Ok(Ok(_)) => Redirect::to("/bug-origins").into_response(),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        Err(error) => super::page(Err(error)),
    }
}
