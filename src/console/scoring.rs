//! Weekly Scores and the CLI's atomic human scoring operation.
use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use askama::Template;

use super::filters;
use axum::extract::RawQuery;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Extension, Form};

use super::auth::Viewer;
use crate::config::UserConfig;
use crate::scoring::{self, Criterion, Score};

#[derive(Template)]
#[template(path = "console/scores.html")]
struct ScoresPage {
    page: super::PageContext,
    view: scoring::View,
    criteria: [Criterion; 5],
    csrf: String,
}

pub(super) async fn show(
    Extension(viewer): Extension<Viewer>,
    RawQuery(query): RawQuery,
) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            let config = UserConfig::load()?;
            let mine = super::query_value(&query, "mine");
            let mut view = scoring::view((!mine.is_empty()).then_some(mine.as_str()))?;
            // Starting another entry is blind too, even when replacing saved Scores.
            if !mine.is_empty() {
                for pr in view.weeks.iter_mut().flat_map(|week| &mut week.sample) {
                    pr.mine = None;
                    pr.assessment = None;
                }
            }
            Ok(ScoresPage {
                page: super::PageContext::new(&config),
                view,
                criteria: Criterion::ALL,
                csrf: viewer.csrf,
            }
            .render()?)
        })
        .await,
    )
}

pub(super) async fn submit(Form(form): Form<HashMap<String, String>>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<()> {
        let target = form
            .get("pull_request")
            .context("a pull request is required")?;
        let scores = Criterion::ALL
            .into_iter()
            .map(|criterion| {
                let passed = match form.get(criterion.name()).map(String::as_str) {
                    Some("pass") => true,
                    Some("fail") => false,
                    _ => bail!("all five Scores must be pass or fail"),
                };
                let reason = form
                    .get(&format!("{}_reason", criterion.name()))
                    .context("every Score needs a reason")?
                    .clone();
                Ok(Score {
                    criterion,
                    passed,
                    reason,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        scoring::save_mine(target, scores)
    })
    .await;
    match result {
        Ok(Ok(())) => Redirect::to("/scores").into_response(),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        Err(error) => super::page(Err(error)),
    }
}
