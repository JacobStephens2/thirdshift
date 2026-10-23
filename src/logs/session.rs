//! Resolve a Session's readable file and retain its records or raw Harness events.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::otlp_jsonl::{AnyValue, LogRecord, Logs, Reader, Request};

/// A live append may expose only part of its last request. Completed malformed
/// lines remain errors; readers retry an incomplete final request on refresh.
pub(in crate::logs) fn read_requests<R: Request>(path: &Path) -> anyhow::Result<Option<Vec<R>>> {
    use anyhow::Context;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("can't read {}", path.display())),
    };
    let mut reader = Reader::new(bytes.as_slice()).peekable();
    let mut requests = Vec::new();
    while let Some(result) = reader.next() {
        match result {
            Ok(request) => requests.push(request),
            Err(error)
                if !bytes.ends_with(b"\n")
                    && reader.peek().is_none()
                    && error
                        .get_ref()
                        .and_then(|error| error.downcast_ref::<serde_json::Error>())
                        .is_some_and(serde_json::Error::is_eof) =>
            {
                break;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("can't read Session trace {}", path.display()));
            }
        }
    }
    Ok(Some(requests))
}

/// Converted Command records can still name the former raw file. Resolve its
/// log-records file without ever falling back to the old Session-log format.
pub(crate) fn path(log: &Path) -> PathBuf {
    if is_trace(log) {
        return log.to_owned();
    }
    original(log).with_extension("logs.jsonl")
}

fn is_trace(path: &Path) -> bool {
    path.as_os_str()
        .as_encoded_bytes()
        .ends_with(b".logs.jsonl")
}

/// Sidecars keep the original Session stem after its raw log is removed.
pub(crate) fn original(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match name
        .strip_suffix(".logs.jsonl")
        .or_else(|| name.strip_suffix(".spans.jsonl"))
    {
        Some(stem) => path.with_file_name(format!("{stem}.jsonl")),
        None => path.to_owned(),
    }
}

/// Read a recent window, extending it to the start of the first OTLP request.
/// One encoded request can exceed the old raw-event budget just from escaping.
/// Work depends on that request's size rather than the entire Session trace.
pub(crate) fn tail_events(log: &Path) -> io::Result<Vec<String>> {
    const WINDOW: u64 = 64 * 1024;
    let path = path(log);
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut start = length.saturating_sub(WINDOW);
    {
        let mut cursor = start;
        while cursor > 0 {
            let previous = cursor.saturating_sub(WINDOW);
            file.seek(SeekFrom::Start(previous))?;
            let mut chunk = vec![0; (cursor - previous) as usize];
            file.read_exact(&mut chunk)?;
            if let Some(newline) = chunk.iter().rposition(|byte| *byte == b'\n') {
                start = previous + newline as u64 + 1;
                break;
            }
            cursor = previous;
            start = cursor;
        }
    }
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(length - start).read_to_end(&mut bytes)?;
    let tail = String::from_utf8_lossy(&bytes);
    Ok(events(&tail).collect())
}

/// A request can contain several resources, scopes and records. Keep their file
/// order, preserving raw Harness events inside the OTLP bodies.
pub(crate) fn events(lines: &str) -> impl Iterator<Item = String> + '_ {
    records(lines).filter_map(|record| match record.body {
        Some(AnyValue::String(raw)) => Some(raw),
        _ => None,
    })
}

/// Keep normalized attributes available to readers that understand them.
pub(crate) fn records(lines: &str) -> impl Iterator<Item = LogRecord> + '_ {
    Reader::<_, Logs>::new(lines.as_bytes())
        .filter_map(Result::ok)
        .flat_map(|request| request.resource_logs)
        .flat_map(|resource| resource.scope_logs)
        .flat_map(|scope| scope.log_records)
}
