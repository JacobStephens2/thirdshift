//! Session-trace fixtures at the on-disk OTLP seam.
use serde_json::json;

pub fn artifact(path: &std::path::Path, extension: &str) -> std::path::PathBuf {
    let name = path.file_name().unwrap().to_str().unwrap();
    let stem = name
        .strip_suffix(".logs.jsonl")
        .or_else(|| name.strip_suffix(".spans.jsonl"));
    match stem {
        Some(stem) => path.with_file_name(format!("{stem}.{extension}")),
        None => path.with_extension(extension),
    }
}

/// Write raw Harness fixture events only inside OTLP log records.
pub fn write(path: impl AsRef<std::path::Path>, lines: impl AsRef<str>) {
    std::fs::write(path, records(lines.as_ref())).unwrap();
}

/// Recover events for assertions about preserved Harness output.
pub fn events(path: impl AsRef<std::path::Path>) -> String {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|line| {
            let request: serde_json::Value = serde_json::from_str(line).unwrap();
            request["resourceLogs"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|resource| {
                    resource["scopeLogs"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .flat_map(|scope| {
                            scope["logRecords"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|record| {
                                    record["body"]["stringValue"].as_str().unwrap().to_owned()
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n")
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
