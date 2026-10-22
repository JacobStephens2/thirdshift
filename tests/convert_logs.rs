mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::Scenario;

const STAMP: &str = "20261001T120000+0000";

fn fixture(scenario: &Scenario, harness: &str, raw: &str) -> PathBuf {
    let folder = scenario.path(&format!(
        "home/.thirdshift/logs/acme/{harness}/commands/secure"
    ));
    fs::create_dir_all(&folder).unwrap();
    let session = folder.join(format!("secure-{STAMP}-security-audit.jsonl"));
    let command = folder.join(format!("{STAMP}.log"));
    fs::write(&session, raw).unwrap();
    fs::write(&command, "thirdshift: 12:00:00 starting\nthirdshift: 12:00:05 security-audit: $ cargo test\nthirdshift: 12:01:00 finished\n").unwrap();
    fs::write(
        session.with_extension("models.json"),
        serde_json::to_vec(&json!({
            "command_log":command,"kind":"security-audit","harness":harness,
            "requested":null,"effort":null,"observed":[]
        }))
        .unwrap(),
    )
    .unwrap();
    session
}

fn report(scenario: &Scenario) -> Value {
    serde_json::from_slice(
        &fs::read(scenario.path("home/.thirdshift/logs/session-trace-conversion.json")).unwrap(),
    )
    .unwrap()
}

fn records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|line| {
            let request: Value = serde_json::from_str(line).unwrap();
            request["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect()
}

#[test]
fn converts_all_six_harnesses_with_round_trip_report_and_no_duplicates() {
    let scenario = Scenario::new();
    let fixtures = [
        ("claude", include_str!("fixtures/console-claude.jsonl")),
        ("codex", include_str!("fixtures/console-codex.jsonl")),
        ("agy", include_str!("fixtures/normalized-agy.jsonl")),
        ("grok", include_str!("fixtures/normalized-grok.jsonl")),
        ("muse", include_str!("fixtures/normalized-muse.jsonl")),
        (
            "opencode",
            include_str!("fixtures/normalized-opencode.jsonl"),
        ),
    ];
    let paths: Vec<_> = fixtures
        .iter()
        .map(|(harness, raw)| fixture(&scenario, harness, raw))
        .collect();
    let result = scenario.run(&["logs", "--convert"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let report = report(&scenario);
    assert_eq!(report["sessions"].as_array().unwrap().len(), 6, "{report}");
    for entry in report["sessions"].as_array().unwrap() {
        assert_eq!(entry["passed"], true, "{entry}");
        for reader in ["conversation", "waterfall", "show", "model_evidence"] {
            assert_eq!(entry["readers"][reader], true, "{entry}");
        }
    }
    for ((_, raw), path) in fixtures.iter().zip(&paths) {
        assert_eq!(fs::read_to_string(path).unwrap(), *raw);
        let logs = records(&path.with_extension("logs.jsonl"));
        let bodies: Vec<_> = logs
            .iter()
            .map(|r| r["body"]["stringValue"].as_str().unwrap())
            .collect();
        assert_eq!(bodies, raw.lines().collect::<Vec<_>>());
        for (raw, record) in raw.lines().zip(&logs) {
            let raw: Value = serde_json::from_str(raw).unwrap_or_default();
            let estimated = ["timestamp", "recorded_at", "time"]
                .iter()
                .all(|key| raw.get(key).is_none());
            assert!(
                record["attributes"].as_array().unwrap().iter().any(|a| {
                    a["key"] == "thirdshift.time.estimated" && a["value"]["boolValue"] == estimated
                }),
                "{record}"
            );
        }
        assert!(path.with_extension("spans.jsonl").is_file());
    }
    let before: Vec<_> = paths
        .iter()
        .map(|p| fs::read(p.with_extension("logs.jsonl")).unwrap())
        .collect();
    let result = scenario.run(&["logs", "--convert"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("Converted 0"), "{}", result.stdout);
    for (path, before) in paths.iter().zip(before) {
        assert_eq!(fs::read(path.with_extension("logs.jsonl")).unwrap(), before);
    }
}

#[test]
fn cleanup_removes_only_passed_unchanged_originals() {
    let scenario = Scenario::new();
    let raw = include_str!("fixtures/console-claude.jsonl");
    let good = fixture(&scenario, "claude", raw);
    let changed = fixture(&scenario, "codex", raw);
    let damaged = fixture(&scenario, "grok", raw);
    let failed = fixture(&scenario, "unknown", "unrecognized old output\n");
    let result = scenario.run(&["logs", "--convert"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let report = report(&scenario);
    let sessions = report["sessions"].as_array().unwrap();
    assert_eq!(
        sessions.iter().filter(|s| s["passed"] == true).count(),
        3,
        "{report}"
    );
    assert_eq!(
        sessions.iter().filter(|s| s["passed"] == false).count(),
        1,
        "{report}"
    );
    fs::write(&changed, "new original\n").unwrap();
    fs::write(damaged.with_extension("spans.jsonl"), "broken trace\n").unwrap();
    let result = scenario.run(&["logs", "--remove-converted"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("Removed 1"), "{}", result.stdout);
    assert!(!good.exists());
    assert!(good.with_extension("logs.jsonl").is_file());
    assert!(good.with_extension("models.json").is_file());
    assert!(changed.is_file());
    assert!(damaged.is_file());
    assert!(failed.is_file());
    let result = scenario.run(&["logs", "--remove-converted"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("Removed 0"), "{}", result.stdout);
}

fn command_record(session: &Path, harness: &str) -> PathBuf {
    let command = session.parent().unwrap().join(format!("{STAMP}.log"));
    let record = command.with_extension("command.json");
    fs::write(&record, serde_json::to_vec(&json!({
        "version":1,"kind":"Security run","repository":format!("acme/{harness}"),
        "name":format!("secure-{STAMP}"),"issue":null,"hostname":"factory",
        "process_id":0,"process_start":"gone","harness":harness,"model":null,"effort":null,
        "started_at":"2026-10-01T12:00:00Z","ended_at":"2026-10-01T12:01:00Z",
        "stages":[{"name":"security-audit","began_at":"2026-10-01T12:00:00Z","session_log":session}],
        "session_log":session,"outcome":"done","cause":null,"issue_branch":null,"pull_request":null
    })).unwrap()).unwrap();
    command
}

#[test]
fn show_and_logs_find_one_session_after_removing_a_recorded_original() {
    let scenario = Scenario::new();
    let raw = "{\"type\":\"item.completed\",\"item\":{\"id\":\"test\",\"type\":\"command_execution\",\"command\":\"cargo test\",\"status\":\"completed\",\"exit_code\":0}}\n";
    let session = fixture(&scenario, "codex", raw);
    let command = command_record(&session, "codex");
    let command = command.to_str().unwrap();
    for args in [["logs", "--convert"], ["logs", "--remove-converted"]] {
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
    }
    assert!(!session.exists(), "{}", report(&scenario));
    let result = scenario.run(&["show", command, "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let shown: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(
        shown["command"]["session_logs"],
        json!([session.with_extension("logs.jsonl")])
    );
    let result = scenario.run(&["logs", command, "--session", "security-audit"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("resourceLogs"), "{}", result.stdout);
    let logs = records(&session.with_extension("logs.jsonl"));
    assert_eq!(logs[0]["timeUnixNano"], "1790856005000000000");
}

#[test]
fn recovers_old_codex_diffs_from_the_matching_native_session_file() {
    let scenario = Scenario::new();
    let session = fixture(
        &scenario,
        "codex",
        include_str!("fixtures/console-codex.jsonl"),
    );
    let native = scenario
        .path("home/.codex/sessions/2026/10/01/rollout-2026-10-01T12-00-00-made-up-thread.jsonl");
    fs::create_dir_all(native.parent().unwrap()).unwrap();
    let directory = scenario.path("work/widgets");
    let diff = "--- a/widgets.rs\n+++ b/widgets.rs\n@@ -1 +1 @@\n-old\n+new\n";
    fs::write(&native, format!("{}\n{}\n", json!({"type":"session_meta","payload":{"id":"made-up-thread","cwd":directory}}),
        json!({"timestamp":"2026-10-01T12:00:40Z","type":"event_msg","payload":{
            "type":"patch_apply_end","success":true,"changes":{
                directory.join("widgets.rs").to_str().unwrap():{"type":"update","unified_diff":diff,"move_path":null}
            }
        }})
    )).unwrap();
    let result = scenario.run(&["logs", "--convert"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let report = report(&scenario);
    assert_eq!(report["sessions"][0]["passed"], true, "{report}");
    assert_eq!(report["sessions"][0]["codex_diffs"], 1, "{report}");
    let logs = records(&session.with_extension("logs.jsonl"));
    let amendment: Value = logs
        .iter()
        .filter_map(|record| {
            serde_json::from_str::<Value>(record["body"]["stringValue"].as_str()?).ok()
        })
        .find(|event| event["result"]["source"] == "codex_session_file")
        .unwrap();
    assert_eq!(amendment["id"], "f1");
    assert_eq!(amendment["result"]["changes"][0]["diff"], diff);
    assert!(session.exists());
}

#[test]
fn empty_sessions_convert_but_existing_trace_files_are_never_overwritten() {
    let scenario = Scenario::new();
    let empty = fixture(&scenario, "claude", "");
    let existing = fixture(
        &scenario,
        "codex",
        include_str!("fixtures/console-codex.jsonl"),
    );
    fs::write(existing.with_extension("spans.jsonl"), "existing trace\n").unwrap();
    let result = scenario.run(&["logs", "--convert"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let report = report(&scenario);
    let entries = report["sessions"].as_array().unwrap();
    assert_eq!(
        entries
            .iter()
            .find(|e| e["source"].as_str().unwrap().contains("/claude/"))
            .unwrap()["passed"],
        true,
        "{report}"
    );
    assert_eq!(
        fs::read_to_string(existing.with_extension("spans.jsonl")).unwrap(),
        "existing trace\n"
    );
    let result = scenario.run(&["logs", "--remove-converted"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(!empty.exists());
    assert!(existing.exists());
}

#[test]
fn removed_tokenless_original_keeps_its_evidenced_command_owner() {
    let scenario = Scenario::new();
    let session = fixture(
        &scenario,
        "claude",
        include_str!("fixtures/console-claude.jsonl"),
    );
    let legacy_command = session.parent().unwrap().join(format!("{STAMP}.log"));
    let owned_command = legacy_command.with_file_name(format!("{STAMP}~owner.log"));
    fs::rename(&legacy_command, &owned_command).unwrap();
    let models_path = session.with_extension("models.json");
    let mut models: Value = serde_json::from_slice(&fs::read(&models_path).unwrap()).unwrap();
    models["command_log"] = json!(owned_command);
    fs::write(&models_path, serde_json::to_vec(&models).unwrap()).unwrap();
    for args in [["logs", "--convert"], ["logs", "--remove-converted"]] {
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
    }
    assert!(!session.exists(), "{}", report(&scenario));
    let result = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert!(
        commands
            .iter()
            .any(|c| c["command_log"] == json!(owned_command)),
        "{status}"
    );
    assert!(
        !commands
            .iter()
            .any(|c| c["command_log"] == json!(legacy_command)),
        "{status}"
    );
}
