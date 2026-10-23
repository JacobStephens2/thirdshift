//! Session-trace fixtures at the on-disk OTLP seam.
use serde_json::json;

/// Results that fit in the old raw tail but overflow 64 KiB once OTLP escapes them.
pub fn large_result_tail() -> String {
    let result = json!({
        "type": "result", "subtype": "success", "result": "\"".repeat(2_000)
    });
    format!("{result}\n").repeat(12)
}

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
