//! Log pages and their independent append-only HTML stream.
use std::convert::Infallible;

use anyhow::{Context, Result, ensure};
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
use super::subscription::{Readiness, Subscription};
use crate::config::UserConfig;
use crate::logs::Framing;
use crate::logs::catalog::{self, Log};

#[derive(Template)]
#[template(path = "console/logs.html")]
struct LogsPage {
    page: super::PageContext,
    groups: Vec<LogGroup>,
    owner: String,
}

/// One repository's logs, in catalog order; the page folds each group.
struct LogGroup {
    repository: String,
    logs: Vec<Log>,
}

impl LogGroup {
    fn live(&self) -> usize {
        self.logs.iter().filter(|log| log.live).count()
    }
}

/// Logs grouped by repository, the groups in order of their first log.
fn by_repository(logs: Vec<Log>) -> Vec<LogGroup> {
    let mut groups: Vec<LogGroup> = Vec::new();
    for log in logs {
        match groups
            .iter_mut()
            .find(|group| group.repository == log.repository)
        {
            Some(group) => group.logs.push(log),
            None => groups.push(LogGroup {
                repository: log.repository.clone(),
                logs: vec![log],
            }),
        }
    }
    groups
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

/// A small, plain-text tail through the same confined reader as Full Command log.
pub(super) fn preview(config: &UserConfig, path: &std::path::Path) -> Result<String> {
    const BUDGET: usize = 4 * 1024;
    let log = catalog::load(config, None)?
        .into_iter()
        .find(|log| log.path == path)
        .context("Command log is unavailable")?;
    let offset = std::fs::metadata(&log.path)?
        .len()
        .saturating_sub(BUDGET as u64);
    let batch = log.reader.read(offset, Framing::Raw, BUDGET)?;
    let text = String::from_utf8_lossy(&batch.bytes);
    // Drop the cut first line when possible, retaining a tail even for a
    // single line longer than the budget or output without a final newline.
    let text = if offset > 0 {
        text.split_once('\n')
            .filter(|(_, tail)| !tail.is_empty())
            .map_or(text.as_ref(), |(_, tail)| tail)
    } else {
        text.as_ref()
    };
    let mut lines: Vec<_> = text.lines().rev().take(5).collect();
    lines.reverse();
    Ok(lines.join("\n"))
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
                groups: by_repository(logs),
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
        Ok(log)
    })
    .await;
    let state = match checked {
        Ok(Ok(log)) => LogStream {
            log: std::sync::Arc::new(log),
            offset,
            done: false,
            subscription: Subscription::new(changes, viewer, &notifier),
        },
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let events = futures_util::stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        // Buffered output drains without another notification; only an
        // empty unfinished read sleeps until the next change.
        let mut mode = Readiness::Drain;
        loop {
            let result = match state.subscription.ready(mode).await {
                Ok(true) => {
                    let selected = state.log.clone();
                    let offset = state.offset;
                    // The captured reader rejects a file that leaves the inventory.
                    match tokio::task::spawn_blocking(move || appended(&selected, offset)).await {
                        Ok(read) => read,
                        Err(error) => Err(error.into()),
                    }
                }
                Ok(false) => return None,
                Err(error) => Err(error),
            };
            match result {
                Ok(batch) if batch.offset != state.offset || batch.done => {
                    state.offset = batch.offset;
                    state.done = batch.done;
                    let event = Event::default()
                        .event("lines")
                        .id(state.offset.to_string())
                        .data(serde_json::to_string(&batch).expect("serializable log batch"));
                    return Some((Ok::<_, Infallible>(event), state));
                }
                Ok(_) => mode = Readiness::Wait,
                Err(error) => {
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
    subscription: Subscription,
}
