//! The CLI's retained Experiment view, rendered without starting work.
use anyhow::Result;
use askama::Template;
use axum::Extension;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};

use super::auth::Viewer;
use super::filters::escaped;
use crate::config::UserConfig;
use crate::evals::{approval, experiment};

mod render;

#[derive(Template)]
#[template(path = "console/experiments.html")]
struct ExperimentsPage {
    page: super::PageContext,
    title: String,
    html: String,
    csrf: String,
    proposals: Vec<approval::Proposal>,
}

pub(super) async fn index(Extension(viewer): Extension<Viewer>) -> Response {
    show(None, viewer.csrf).await
}

pub(super) async fn detail(
    Path(id): Path<String>,
    Extension(viewer): Extension<Viewer>,
) -> Response {
    show(Some(id), viewer.csrf).await
}

async fn show(id: Option<String>, csrf: String) -> Response {
    let detail = id.is_some();
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let config = UserConfig::load()?;
        let views = experiment::read(id.as_deref())?;
        let mut proposals = approval::proposed()?;
        if detail {
            proposals.retain(|proposal| views[0].description.suites.contains(&proposal.suite));
        }
        let (title, html) = if detail {
            let view = &views[0];
            (render::name(view), render::detail(view))
        } else {
            ("Experiments".into(), render::list(&views))
        };
        Ok(ExperimentsPage {
            page: super::PageContext::new(&config),
            title,
            html,
            csrf,
            proposals,
        }
        .render()?)
    })
    .await;
    if detail && matches!(&result, Ok(Err(_))) {
        return (StatusCode::NOT_FOUND, "Experiment is unavailable.").into_response();
    }
    super::page(result)
}

pub(super) async fn approve(Path((suite, task)): Path<(String, String)>) -> Response {
    match tokio::task::spawn_blocking(move || approval::approve(&suite, &task)).await {
        Ok(Ok(_)) => Redirect::to("/experiments").into_response(),
        Ok(Err(error)) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
        Err(error) => super::page(Err(error)),
    }
}

pub(super) async fn session(Path((id, session)): Path<(String, String)>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let config = UserConfig::load()?;
        let view = experiment::read(Some(&id))?.pop().unwrap();
        let log = view.sessions.iter().find(|log| log.id == session).ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        let contents = std::fs::read_to_string(&log.path)?;
        let evidence = if log.trace {
            super::conversation::render(&contents, crate::logs::catalog::Kind::Session)
        } else {
            super::conversation::render_trial_log(&contents)
        };
        let html = format!("<h1>{}</h1><p><a href=\"/experiments/{}\">Experiment</a></p><p><code>thirdshift experiments {} --session {}</code></p><section aria-label=\"Session evidence\">{evidence}</section>", if log.trace { "Session trace" } else { "Session log" }, escaped(&id), escaped(&id), escaped(&session));
        Ok(ExperimentsPage { page: super::PageContext::new(&config), title: "Trial session".into(), html, csrf: String::new(), proposals: Vec::new() }.render()?)
    }).await;
    match result {
        Ok(Ok(html)) => super::page(Ok(Ok(html))),
        _ => (StatusCode::NOT_FOUND, "Session is unavailable.").into_response(),
    }
}
