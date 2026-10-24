//! The CLI's retained Experiment view, rendered without starting work.
use anyhow::Result;
use askama::Template;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::filters::escaped;
use crate::config::UserConfig;
use crate::evals::experiment;

mod render;

#[derive(Template)]
#[template(path = "console/experiments.html")]
struct ExperimentsPage {
    page: super::PageContext,
    title: String,
    html: String,
}

pub(super) async fn index() -> Response {
    show(None).await
}

pub(super) async fn detail(Path(id): Path<String>) -> Response {
    show(Some(id)).await
}

async fn show(id: Option<String>) -> Response {
    let detail = id.is_some();
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let config = UserConfig::load()?;
        let views = experiment::read(id.as_deref())?;
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
        }
        .render()?)
    })
    .await;
    if detail && matches!(&result, Ok(Err(_))) {
        return (StatusCode::NOT_FOUND, "Experiment is unavailable.").into_response();
    }
    super::page(result)
}

pub(super) async fn session(Path((id, session)): Path<(String, String)>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<String> {
        let config = UserConfig::load()?;
        let view = experiment::read(Some(&id))?.pop().unwrap();
        let log = view.sessions.iter().find(|log| log.id == session).ok_or_else(|| anyhow::anyhow!("Session not found"))?;
        let contents = std::fs::read_to_string(&log.path)?;
        let html = format!("<h1>{}</h1><p><a href=\"/experiments/{}\">Experiment</a></p><p><code>thirdshift experiments {} --session {}</code></p><section aria-label=\"Session evidence\">{}</section>", if log.trace { "Session trace" } else { "Session log" }, escaped(&id), escaped(&id), escaped(&session), super::conversation::render(&contents, crate::logs::catalog::Kind::Session));
        Ok(ExperimentsPage { page: super::PageContext::new(&config), title: "Trial session".into(), html }.render()?)
    }).await;
    match result {
        Ok(Ok(html)) => super::page(Ok(Ok(html))),
        _ => (StatusCode::NOT_FOUND, "Session is unavailable.").into_response(),
    }
}
