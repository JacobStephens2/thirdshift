//! Resolve a Session's readable file and unwrap its retained raw Harness events.
use std::path::{Path, PathBuf};

use crate::otlp_jsonl::{AnyValue, Logs, Reader};

/// Command records may still name the old Session log during the transition.
pub(crate) fn path(log: &Path) -> PathBuf {
    if log.to_string_lossy().ends_with(".logs.jsonl") {
        return log.to_owned();
    }
    let records = log.with_extension("logs.jsonl");
    if records.is_file() {
        records
    } else {
        log.to_owned()
    }
}

/// A request can contain several resources, scopes and records. Keep their file
/// order, and preserve old logs and malformed diagnostics for the renderer.
pub(crate) fn events(lines: &str) -> impl Iterator<Item = String> + '_ {
    lines.lines().flat_map(|line| {
        let is_trace = serde_json::from_str::<serde_json::Value>(line)
            .is_ok_and(|value| value.get("resourceLogs").is_some());
        if is_trace && let Some(Ok(request)) = Reader::<_, Logs>::new(line.as_bytes()).next() {
            return request
                .resource_logs
                .into_iter()
                .flat_map(|resource| resource.scope_logs)
                .flat_map(|scope| scope.log_records)
                .filter_map(|record| match record.body {
                    Some(AnyValue::String(raw)) => Some(raw),
                    _ => None,
                })
                .collect();
        }
        vec![line.to_owned()]
    })
}
