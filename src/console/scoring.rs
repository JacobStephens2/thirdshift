//! Weekly Scores and the CLI's atomic human scoring operation.
use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use askama::Template;

use super::filters;
use axum::extract::RawQuery;
use axum::response::Response;
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
    Extension(format): Extension<super::api::Format>,
    RawQuery(query): RawQuery,
) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        let mine = if format == super::api::Format::Html {
            super::query_value(&query, "mine")
        } else {
            String::new()
        };
        let target = super::query_value(&query, "pull_request");
        let target = if mine.is_empty() {
            target.as_str()
        } else {
            mine.as_str()
        };
        let mut view = scoring::view((!target.is_empty()).then_some(target))?;
        // Starting another entry is blind too, even when replacing saved Scores.
        if !mine.is_empty() {
            for pr in view.weeks.iter_mut().flat_map(|week| &mut week.sample) {
                pr.mine = None;
                pr.assessment = None;
            }
        }
        format.render(scoring::json_view(&view.weeks, &view.calibration)?, |_| {
            Ok(ScoresPage {
                page: super::PageContext::new(&config),
                view,
                criteria: Criterion::ALL,
                csrf: viewer.csrf,
            }
            .render()?)
        })
    })
    .await
}

pub(super) async fn submit(
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    super::api::action(format, "/scores", move || -> Result<serde_json::Value> {
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
        scoring::save_mine(target, scores)?;
        let view = scoring::view(Some(target))?;
        scoring::json_view(&view.weeks, &view.calibration)
    })
    .await
}
