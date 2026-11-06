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
    log: ReaderLog,
    html: String,
    offset: u64,
    live: bool,
    cli: String,
    details_url: String,
    stream: String,
    snapshot: bool,
    owning_console: Option<String>,
}

/// Presentation facts carry no filesystem reader. Remote logs use the same
/// template without manufacturing a local Log or opening an owning path.
struct ReaderLog {
    account: Option<String>,
    kind: catalog::Kind,
    name: String,
    selector: String,
}

pub(super) fn remote(
    remote: &super::fleet::read::Remote,
    config: &UserConfig,
    viewer: &super::auth::Viewer,
    format: super::api::Format,
    command: &str,
    session: &str,
) -> Result<Response> {
    let session = (!session.is_empty()).then(|| std::path::Path::new(session));
    let route = crate::logs::status::inspect::url("/log", command, session);
    let value = remote.get(&route)?;
    format.render(value, |value| {
        let contents = value["contents"]
            .as_str()
            .context("incompatible remote log")?;
        let kind = if session.is_some() {
            catalog::Kind::Session
        } else {
            catalog::Kind::Command
        };
        let cli = format!(
            "thirdshift logs '{}'{} --machine '{}' --json",
            command.replace('\'', "'\\''"),
            session
                .map(|session| format!(
                    " --session '{}'",
                    session.display().to_string().replace('\'', "'\\''")
                ))
                .unwrap_or_default(),
            remote.entry.name.replace('\'', "'\\''")
        );
        Ok(ReaderPage {
            page: super::PageContext::new(config, &viewer.scopes),
            log: ReaderLog {
                account: None,
                kind,
                name: session.map_or_else(
                    || command.to_owned(),
                    |session| session.display().to_string(),
                ),
                selector: command.into(),
            },
            html: conversation::render(contents, kind),
            offset: 0,
            live: false,
            snapshot: true,
            stream: String::new(),
            cli,
            details_url: remote.command_url("/peek", command, None),
            owning_console: Some(super::fleet::read::owning_url(&remote.entry, &route)),
        }
        .render()?)
    })
}

/// A small, plain-text tail through the same confined reader as Full Command log.
pub(super) fn preview(config: &UserConfig, path: &std::path::Path) -> Result<String> {
    const BUDGET: usize = 4 * 1024;
    let log = catalog::load(config, None)?
        .into_iter()
        .find(|log| log.path == path)
        .context("Command log is unavailable")?;
    Ok(recent_output(&log.reader.tail(BUDGET)?))
}

pub(super) fn recent_output(bytes: &[u8]) -> String {
    // The reader applies the raw byte budget before UTF-8 normalization.
    // Applying it again to decoded text can discard readable output when
    // replacement characters expanded invalid bytes in the owning tail.
    let text = String::from_utf8_lossy(bytes);
    // The byte window can cut the oldest line; keep that suffix too, so a
    // complete line at the window boundary and very long lines stay visible.
    let mut lines: Vec<_> = text.lines().rev().take(5).collect();
    lines.reverse();
    lines.join("\n")
}

pub(super) async fn index(
    Extension(viewer): Extension<super::auth::Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    inventory(viewer, format, None).await
}
pub(super) async fn command(
    Path(id): Path<String>,
    Extension(viewer): Extension<super::auth::Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    inventory(viewer, format, Some(id)).await
}

async fn inventory(
    viewer: super::auth::Viewer,
    format: super::api::Format,
    owner: Option<String>,
) -> Response {
    super::api::read(format, move || {
        let config = UserConfig::load()?;
        let logs = catalog::load(&config, owner.as_deref())?;
        let value = serde_json::json!({"version": 1, "logs": logs});
        format.render(value, |_| {
            Ok(LogsPage {
                page: super::PageContext::new(&config, &viewer.scopes),
                groups: by_repository(logs),
                owner: owner.unwrap_or_default(),
            }
            .render()?)
        })
    })
    .await
}

async fn security_refusal(viewer: &super::auth::Viewer, id: &str) -> Option<Response> {
    if viewer.scopes.contains(&super::Scope::Security) {
        return None;
    }
    let selected = id.to_owned();
    match tokio::task::spawn_blocking(move || -> Result<bool> {
        let config = UserConfig::load()?;
        let log = catalog::find(&config, &selected)?;
        Ok(log.security_details)
    })
    .await
    {
        Ok(Ok(true)) => Some(super::api::error(
            StatusCode::FORBIDDEN,
            "Missing scope: security",
        )),
        Ok(Ok(false)) => None,
        Ok(Err(error)) => Some(super::api::error(StatusCode::UNPROCESSABLE_ENTITY, error)),
        Err(_) => Some(super::api::error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Can't read this view; check the Console's stderr.",
        )),
    }
}

pub(super) async fn reader(
    Path(id): Path<String>,
    Extension(viewer): Extension<super::auth::Viewer>,
    Extension(format): Extension<super::api::Format>,
) -> Response {
    if let Some(response) = security_refusal(&viewer, &id).await {
        return response;
    }
    if format == super::api::Format::Json {
        return super::api::read(format, move || {
            let log = catalog::find(&UserConfig::load()?, &id)?;
            Ok(Json(catalog::contents(&log.reader)?).into_response())
        })
        .await;
    }
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
                page: super::PageContext::new(&config, &viewer.scopes),
                cli: format!("thirdshift logs --file {}", log.id),
                details_url: crate::logs::status::inspect::url("/peek", &log.selector, None),
                stream: format!("/logs/{}/stream/{}", log.id, lines.offset),
                log: ReaderLog {
                    account: log.account,
                    kind: log.kind,
                    name: log.name,
                    selector: log.selector,
                },
                html,
                offset: lines.offset,
                live: !lines.done,
                snapshot: false,
                owning_console: None,
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
    if let Some(response) = security_refusal(&viewer, &id).await {
        return response;
    }
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
