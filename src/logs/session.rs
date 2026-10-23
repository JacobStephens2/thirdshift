//! Resolve a Session's readable file and retain its records or raw Harness events.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::otlp_jsonl::{AnyValue, LogRecord, Logs, Reader};

/// Command records may still name the old Session log during the transition.
pub(crate) fn path(log: &Path) -> PathBuf {
    if is_trace(log) {
        return log.to_owned();
    }
    let records = log.with_extension("logs.jsonl");
    if records.is_file() {
        records
    } else {
        log.to_owned()
    }
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

/// Read a recent window, measuring Session traces by decoded raw-event bytes.
/// Include whole OTLP requests until the old raw-log budget is covered; their
/// metadata and escaping must not shrink the window's event coverage.
pub(crate) fn tail_events(log: &Path) -> io::Result<Vec<String>> {
    const WINDOW: u64 = 64 * 1024;
    let path = path(log);
    let trace = is_trace(&path);
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut end = length;
    let mut decoded_bytes = 0;
    let mut batches = Vec::new();
    loop {
        let mut start = end.saturating_sub(WINDOW);
        if trace {
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
        (&mut file).take(end - start).read_to_end(&mut bytes)?;
        let tail = String::from_utf8_lossy(&bytes);
        // Old Session logs retain their bounded-tail behavior, including dropping
        // a partial first line when the window begins in the middle of an event.
        let tail = if !trace && start > 0 {
            tail.split_once('\n').map_or("", |(_, tail)| tail)
        } else {
            &tail
        };
        let recent: Vec<_> = events(tail).collect();
        decoded_bytes += recent
            .iter()
            .map(|event| event.len() as u64 + 1)
            .sum::<u64>();
        batches.push(recent);
        if !trace || start == 0 || decoded_bytes >= WINDOW {
            break;
        }
        end = start;
    }
    Ok(batches.into_iter().rev().flatten().collect())
}

/// A request can contain several resources, scopes and records. Keep their file
/// order, and preserve old logs and malformed diagnostics for the renderer.
pub(crate) fn events(lines: &str) -> impl Iterator<Item = String> + '_ {
    records(lines).filter_map(|record| match record.body {
        Some(AnyValue::String(raw)) => Some(raw),
        _ => None,
    })
}

/// Keep normalized attributes available to readers that understand them. Old
/// logs and malformed diagnostics become records containing only their raw body.
pub(crate) fn records(lines: &str) -> impl Iterator<Item = LogRecord> + '_ {
    lines.lines().flat_map(|line| {
        let is_trace = serde_json::from_str::<serde_json::Value>(line)
            .is_ok_and(|value| value.get("resourceLogs").is_some());
        if is_trace && let Some(Ok(request)) = Reader::<_, Logs>::new(line.as_bytes()).next() {
            return request
                .resource_logs
                .into_iter()
                .flat_map(|resource| resource.scope_logs)
                .flat_map(|scope| scope.log_records)
                .collect();
        }
        vec![LogRecord {
            body: Some(AnyValue::String(line.to_owned())),
            ..Default::default()
        }]
    })
}
