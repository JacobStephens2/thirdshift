//! The CLI's retained Experiment view, rendered without starting work.
use std::collections::HashMap;

use askama::Template;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Form};

use super::auth::Viewer;
use super::filters::escaped;
use crate::config::UserConfig;
use crate::evals::{approval, experiment};

mod pair;
mod render;
mod setup;

#[derive(Template)]
#[template(path = "console/experiment_pair.html")]
struct PairPage {
    page: super::PageContext,
    html: String,
}

#[derive(Template)]
#[template(path = "console/experiments.html")]
struct ExperimentsPage {
    page: super::PageContext,
    title: String,
    html: String,
    csrf: String,
    proposals: Vec<approval::Proposal>,
}

pub(super) async fn index(
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    show(format, None, viewer).await
}

pub(super) async fn detail(
    Path(id): Path<String>,
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    show(format, Some(id), viewer).await
}

async fn show(format: super::api::Format, id: Option<String>, viewer: Viewer) -> Response {
    let detail = id.is_some();
    let response = super::api::read(format, move || {
        let config = UserConfig::load()?;
        let views = experiment::read(id.as_deref())?;
        let value = if detail {
            serde_json::to_value(&views[0])?
        } else {
            serde_json::to_value(&views)?
        };
        format.render(value, |_| {
            let mut proposals = approval::proposed()?;
            if detail {
                proposals.retain(|proposal| views[0].description.suites.contains(&proposal.suite));
            }
            let (title, html) = if detail {
                let view = &views[0];
                (
                    view.name.clone(),
                    render::detail(
                        view,
                        &viewer.csrf,
                        viewer.scopes.contains(&super::Scope::Act),
                    ),
                )
            } else {
                ("Experiments".into(), render::list(&views))
            };
            Ok(ExperimentsPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                title,
                html,
                csrf: viewer.csrf,
                proposals,
            }
            .render()?)
        })
    })
    .await;
    if detail && format == super::api::Format::Html && !response.status().is_success() {
        return (StatusCode::NOT_FOUND, "Experiment is unavailable.").into_response();
    }
    response
}

/// One blind pair, as the signed-in reader may see it.
pub(super) async fn pair(
    Path((id, pair)): Path<(String, String)>,
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    let response = super::api::read(format, move || {
        let config = UserConfig::load()?;
        let shown = experiment::pairwise::page(&id, &pair, &viewer.activity_actor())?;
        format.render(&shown, |shown| {
            Ok(PairPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                html: pair::page(
                    shown,
                    &viewer.csrf,
                    viewer.scopes.contains(&super::Scope::Act),
                ),
            }
            .render()?)
        })
    })
    .await;
    if format == super::api::Format::Html && !response.status().is_success() {
        return (StatusCode::NOT_FOUND, "Pair is unavailable.").into_response();
    }
    response
}

pub(super) async fn merge_calls(
    Path((id, pair)): Path<(String, String)>,
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let redirect = format!("/experiments/{id}/pairs/{pair}");
    super::api::action(format, &redirect, move || {
        let side = |name: &str| form.get(name).map_or("", String::as_str);
        experiment::pairwise::merge(&id, &pair, side("a"), side("b"), viewer.activity_actor())
    })
    .await
}

pub(super) async fn preference(
    Path((id, pair)): Path<(String, String)>,
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let redirect = format!("/experiments/{id}/pairs/{pair}");
    super::api::action(format, &redirect, move || {
        // A skipped rubric criterion arrives empty.
        let rubric: Vec<_> = form
            .iter()
            .filter_map(|(name, value)| {
                Some((name.strip_prefix("rubric_")?.to_owned(), value.clone()))
            })
            .filter(|(_, value)| !value.is_empty())
            .collect();
        experiment::pairwise::prefer(
            &id,
            &pair,
            form.get("preference").map_or("", String::as_str),
            &rubric,
            viewer.activity_actor(),
        )
    })
    .await
}

#[derive(serde::Deserialize)]
pub(super) struct LeakDecision {
    decision: String,
    reason: String,
}

pub(super) async fn decide_leak(
    Path((id, hit)): Path<(String, String)>,
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
    Form(form): Form<LeakDecision>,
) -> Response {
    let redirect = format!("/experiments/{id}");
    super::api::action(
        format,
        &redirect,
        move || -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::from_str(&experiment::decide_leak(
                &id,
                &hit,
                &form.decision,
                &form.reason,
                viewer.activity_actor(),
                true,
            )?)?)
        },
    )
    .await
}

pub(super) async fn approve(
    Path((suite, task)): Path<(String, String)>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    super::api::action(format, "/experiments", move || {
        approval::approve(&suite, &task)
    })
    .await
}

pub(super) async fn session(
    Path((id, session)): Path<(String, String)>,
    Extension(viewer): Extension<Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    let response = super::api::read(format, move || {
        let config = UserConfig::load()?;
        let log = experiment::read_session(&id, &session)?;
        format.render(&log, |_| {
        let evidence = if log.trace {
            super::conversation::render(&log.contents, crate::logs::catalog::Kind::Session)
        } else {
            super::conversation::render_trial_log(&log.contents)
        };
        let html = format!("<h1>{}</h1><p><a href=\"/experiments/{}\">Experiment</a></p><p><code>thirdshift experiments {} --session {}</code></p><section aria-label=\"Session evidence\">{evidence}</section>", if log.trace { "Session trace" } else { "Session log" }, escaped(&id), escaped(&id), escaped(&session));
        Ok(ExperimentsPage { page: super::PageContext::new(&config, &viewer.scopes), title: "Trial session".into(), html, csrf: String::new(), proposals: Vec::new() }.render()?)
        })
    }).await;
    if format == super::api::Format::Html && !response.status().is_success() {
        (StatusCode::NOT_FOUND, "Session is unavailable.").into_response()
    } else {
        response
    }
}
