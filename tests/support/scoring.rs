//! Retained Continuations shared by Scoring and Bug-origin command regressions.

use std::path::PathBuf;

use serde_json::json;

use super::Scenario;

pub fn continuations(scenario: &Scenario, starts: [&str; 2]) -> Vec<(PathBuf, Vec<u8>)> {
    let folder = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    std::fs::create_dir_all(&folder).unwrap();
    ["older.command.json", "newer.command.json"]
        .into_iter()
        .zip(starts)
        .map(|(filename, started_at)| {
            let record = json!({
                "version":1, "kind":"Run", "repository":"acme/widgets", "name":"acme/widgets#7",
                "issue":{"number":7,"url":scenario.issue_url(7)}, "hostname":"factory",
                "process_id":0, "process_start":"gone", "harness":"codex", "model":"model", "effort":"high",
                "started_at":started_at, "ended_at":"2026-10-19T13:00:00Z", "stages":[],
                "outcome":"ready for review", "session_log":null, "cause":null, "issue_branch":"issue-7",
                "pull_request":"https://github.com/ACME/WIDGETS/pull/23/"
            });
            let path = folder.join(filename);
            let bytes = serde_json::to_vec(&record).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            (path, bytes)
        })
        .collect()
}
