//! Session-trace fixtures at the on-disk OTLP seam.
use serde_json::json;

pub fn records(lines: &str) -> String {
    lines
        .lines()
        .map(|line| {
            format!(
                "{}\n",
                json!({"resourceLogs": [{"scopeLogs": [{"logRecords": [{
                    "body": {"stringValue": line}
                }]}]}]})
            )
        })
        .collect()
}
