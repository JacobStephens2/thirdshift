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

/// Read a recent window, extending it to the start of the first OTLP request.
/// One encoded request can exceed the old raw-event budget just from escaping.
/// Work depends on that request's size rather than the entire Session trace.
pub(crate) fn tail_events(log: &Path) -> io::Result<Vec<String>> {
    const WINDOW: u64 = 64 * 1024;
    let path = path(log);
    let trace = is_trace(&path);
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut start = length.saturating_sub(WINDOW);
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
    file.take(length - start).read_to_end(&mut bytes)?;
    let tail = String::from_utf8_lossy(&bytes);
    // Old Session logs retain their bounded-tail behavior, including dropping
    // a partial first line when the window begins in the middle of an event.
    let tail = if !trace && start > 0 {
        tail.split_once('\n').map_or("", |(_, tail)| tail)
    } else {
        &tail
    };
    Ok(events(tail).collect())
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
