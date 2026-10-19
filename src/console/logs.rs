//! Log pages and their independent append-only HTML stream.
use std::convert::Infallible;
use std::io::{BufRead, BufReader, Seek, SeekFrom};

use anyhow::{Result, ensure};
use askama::Template;
use axum::Extension;
use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{
    IntoResponse, Response, Sse,
    sse::{Event, KeepAlive},
};
use serde::Serialize;

use super::conversation;
use crate::config::UserConfig;
use crate::logs::catalog::{self, Log};

#[derive(Template)]
#[template(path = "console/logs.html")]
struct LogsPage {
    theme: String,
    logs: Vec<Log>,
    owner: String,
}

#[derive(Template)]
#[template(path = "console/reader.html")]
struct ReaderPage {
    theme: String,
    log: Log,
    html: String,
    offset: u64,
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
                theme: config.console.theme,
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
        let lines = read_lines(&log, 0, usize::MAX)?;
        let html = conversation::render(&String::from_utf8_lossy(&lines.bytes), log.kind);
        Ok(Some(
            ReaderPage {
                theme: config.console.theme,
                log,
                html,
                offset: lines.offset,
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

fn appended(config: &UserConfig, id: &str, offset: u64) -> Result<Batch> {
    // Observe completion before the final drain, so final unterminated output
    // is included even if the Command ended between polls.
    let log = catalog::find(config, id)?;
    let lines = read_lines(&log, offset, 256 * 1024)?;
    Ok(Batch {
        html: conversation::render(&String::from_utf8_lossy(&lines.bytes), log.kind),
        offset: lines.offset,
        done: lines.done,
    })
}

struct Lines {
    bytes: Vec<u8>,
    offset: u64,
    done: bool,
}

fn read_lines(log: &Log, offset: u64, budget: usize) -> Result<Lines> {
    let mut file = BufReader::new(std::fs::File::open(&log.path)?);
    let length = file.get_ref().metadata()?.len();
    ensure!(offset <= length, "log was truncated; reopen the reader");
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    let mut next = offset;
    while next < length && bytes.len() < budget {
        let mut line = Vec::new();
        if file.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        if log.live && !line.ends_with(b"\n") {
            break;
        }
        next += line.len() as u64;
        bytes.extend(line);
    }
    let done = !log.live && next >= file.get_ref().metadata()?.len();
    Ok(Lines {
        bytes,
        offset: next,
        done,
    })
}

pub(super) async fn stream(
    Path((id, offset)): Path<(String, u64)>,
    Extension(executable): Extension<super::restart::Executable>,
    headers: HeaderMap,
) -> Response {
    let offset = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(offset);
    let checked = tokio::task::spawn_blocking(move || -> Result<_> {
        let config = UserConfig::load()?;
        let log = catalog::find(&config, &id)?;
        ensure!(
            offset <= std::fs::metadata(log.path)?.len(),
            "invalid log offset"
        );
        Ok((std::sync::Arc::new(config), id, offset, false, executable))
    })
    .await;
    let state = match checked {
        Ok(Ok(state)) => state,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let events = futures_util::stream::unfold(
        state,
        |(config, id, mut offset, done, executable)| async move {
            if done {
                return None;
            }
            loop {
                let root = config.clone();
                let selected = id.clone();
                let watched = executable.clone();
                let result = tokio::task::spawn_blocking(move || {
                    if watched.changed() {
                        return Ok(None);
                    }
                    appended(&root, &selected, offset).map(Some)
                })
                .await;
                match result {
                    Ok(Ok(None)) => return None,
                    Ok(Ok(Some(batch))) if batch.offset != offset || batch.done => {
                        offset = batch.offset;
                        let done = batch.done;
                        let event = Event::default()
                            .event("lines")
                            .id(offset.to_string())
                            .data(serde_json::to_string(&batch).expect("serializable log batch"));
                        return Some((
                            Ok::<_, Infallible>(event),
                            (config, id, offset, done, executable),
                        ));
                    }
                    Ok(Ok(Some(_))) => {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await
                    }
                    error => {
                        eprintln!("thirdshift Console: log stream: {error:?}");
                        let event = Event::default()
                            .event("log-error")
                            .data("Can't read log; reopen the reader. Check the Console's stderr.");
                        return Some((Ok(event), (config, id, offset, true, executable)));
                    }
                }
            }
        },
    );
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
