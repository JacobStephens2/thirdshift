//! Log pages and their independent append-only HTML stream.
use std::convert::Infallible;

use anyhow::{Result, ensure};
use askama::Template;
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{
    IntoResponse, Response, Sse,
    sse::{Event, KeepAlive},
};
use axum::{Extension, Json};
use serde::Serialize;

use super::conversation;
use crate::config::UserConfig;
use crate::logs::Framing;
use crate::logs::catalog::{self, Log};

#[derive(Template)]
#[template(path = "console/logs.html")]
struct LogsPage {
    page: super::PageContext,
    logs: Vec<Log>,
    owner: String,
}

#[derive(Template)]
#[template(path = "console/reader.html")]
struct ReaderPage {
    page: super::PageContext,
    log: Log,
    html: String,
    offset: u64,
    live: bool,
}

pub(super) async fn index() -> Response {
    inventory(None).await
}
pub(super) async fn command(Path(id): Path<String>) -> Response {
    inventory(Some(id)).await
}

async fn inventory(owner: Option<String>) -> Response {
    super::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            let config = UserConfig::load()?;
            let logs = catalog::load(&config, owner.as_deref())?;
            Ok(LogsPage {
                page: super::PageContext::new(&config),
                logs,
                owner: owner.unwrap_or_default(),
            }
            .render()?)
        })
        .await,
    )
}

pub(super) async fn reader(Path(id): Path<String>) -> Response {
    let result = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
        let config = UserConfig::load()?;
        let Some(log) = catalog::load(&config, None)?
            .into_iter()
            .find(|log| log.id == id)
        else {
            return Ok(None);
        };
        let lines = log.reader.read(0, Framing::Lines, usize::MAX)?;
        let html = conversation::render(&String::from_utf8_lossy(&lines.bytes), log.kind);
        Ok(Some(
            ReaderPage {
                page: super::PageContext::new(&config),
                log,
                html,
                offset: lines.offset,
                live: !lines.done,
            }
            .render()?,
        ))
    })
    .await;
    match result {
        Ok(Ok(Some(html))) => super::page(Ok(Ok(html))),
        Ok(Ok(None)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(error)) => super::page(Ok(Err(error))),
        Err(error) => super::page(Err(error)),
    }
}

#[derive(Debug, Serialize)]
struct Batch {
    html: String,
    offset: u64,
    done: bool,
}

fn appended(log: &Log, offset: u64) -> Result<Batch> {
    let lines = log.reader.read(offset, Framing::Lines, 256 * 1024)?;
    Ok(Batch {
        html: conversation::render(&String::from_utf8_lossy(&lines.bytes), log.kind),
        offset: lines.offset,
        done: lines.done,
    })
}

pub(super) async fn stream(
    Path((id, offset)): Path<(String, u64)>,
    Extension(changes): Extension<super::changes::Receiver>,
    Extension(notifier): Extension<super::changes::Notifier>,
    Extension(viewer): Extension<super::auth::Viewer>,
    headers: HeaderMap,
) -> Response {
    // A finite request uses the same reader and offset as SSE when a proxy
    // cannot keep streams open. It still crosses the normal account guard.
    if headers
        .get("accept")
        .is_some_and(|value| value == "application/json")
    {
        return match tokio::task::spawn_blocking(move || {
            appended(&catalog::find(&UserConfig::load()?, &id)?, offset)
        })
        .await
        {
            Ok(Ok(batch)) => Json(batch).into_response(),
            Ok(Err(error)) => super::page(Ok(Err(error))),
            Err(error) => super::page(Err(error)),
        };
    }
    let offset = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(offset);
    let checked = tokio::task::spawn_blocking(move || -> Result<_> {
        let config = UserConfig::load()?;
        let log = catalog::find(&config, &id)?;
        ensure!(
            offset <= std::fs::metadata(&log.path)?.len(),
            "invalid log offset"
        );
        Ok(LogStream {
            log: std::sync::Arc::new(log),
            offset,
            done: false,
            viewer,
            _following: notifier.follow(changes.clone()),
            changes,
            accounts_revision: None,
        })
    })
    .await;
    let state = match checked {
        Ok(Ok(state)) => state,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let events = futures_util::stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        loop {
            if state.changes.has_changed().is_err() {
                return None;
            }
            let revision = state.changes.borrow_and_update().accounts;
            let authorize = state.accounts_revision != Some(revision);
            state.accounts_revision = Some(revision);
            let selected = state.log.clone();
            let signed_in = state.viewer.clone();
            let offset = state.offset;
            let result = tokio::task::spawn_blocking(move || {
                if authorize && !signed_in.still_allowed()? {
                    return Ok(None);
                }
                // The captured reader rejects a file that leaves the inventory.
                appended(&selected, offset).map(Some)
            })
            .await;
            match result {
                Ok(Ok(None)) => return None,
                Ok(Ok(Some(batch))) if batch.offset != state.offset || batch.done => {
                    state.offset = batch.offset;
                    state.done = batch.done;
                    let event = Event::default()
                        .event("lines")
                        .id(state.offset.to_string())
                        .data(serde_json::to_string(&batch).expect("serializable log batch"));
                    return Some((Ok::<_, Infallible>(event), state));
                }
                Ok(Ok(Some(_))) => {
                    if state.changes.changed().await.is_err() {
                        return None;
                    }
                }
                error => {
                    eprintln!("thirdshift Console: log stream: {error:?}");
                    let event = Event::default()
                        .event("log-error")
                        .data("Can't read log; reopen the reader. Check the Console's stderr.");
                    state.done = true;
                    return Some((Ok(event), state));
                }
            }
        }
    });
    let mut response = Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response();
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
}

struct LogStream {
    log: std::sync::Arc<Log>,
    offset: u64,
    done: bool,
    viewer: super::auth::Viewer,
    changes: super::changes::Receiver,
    accounts_revision: Option<u64>,
    _following: std::sync::Arc<super::watching::Following>,
}
