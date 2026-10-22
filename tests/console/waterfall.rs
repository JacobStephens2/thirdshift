use super::*;

fn retained_command(home: &Path, session: &Path) -> Value {
    configure(home);
    let mut value = record(home, 7, Some("merged"), 0);
    value["started_at"] = json!("2026-10-01T12:00:00+00:00");
    value["ended_at"] = json!("2026-10-01T12:01:00+00:00");
    value["stages"] =
        json!([{ "name": "implement", "began_at": value["started_at"], "session_log": session }]);
    value["session_log"] = json!(session);
    value
}

fn start_console(home: &Path, value: &Value) -> Console {
    write_record(home, 7, value);
    Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home)
}

#[test]
fn converted_session_keeps_its_waterfall_stage_and_duration_after_cleanup() {
    let home = tempfile::tempdir().unwrap();
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.jsonl");
    let mut value = retained_command(home.path(), &session);
    value["harness"] = json!("codex");
    value["stages"] = json!([
        {"name":"implement","began_at":value["started_at"],"session_log":session},
        {"name":"review","began_at":"2026-10-01T12:00:05.500Z","session_log":null}
    ]);
    fs::write(&session, "{\"type\":\"item.completed\",\"item\":{\"id\":\"c1\",\"type\":\"command_execution\",\"command\":\"cargo test\",\"status\":\"completed\",\"exit_code\":0}}\n").unwrap();
    fs::write(dir.join("7-fixture.log"), "thirdshift: 12:00:00 starting\nthirdshift: 12:00:05 implement: $ cargo test\nthirdshift: 12:01:00 finished\n").unwrap();
    let console = start_console(home.path(), &value);
    let before = span_tree(&console, "widgets%237");
    assert_eq!(tool_spans(&before)[0]["start"], 1790856005000.0);
    assert_eq!(tool_spans(&before)[0]["duration_ms"], 500.0);
    for flag in ["--convert", "--remove-converted"] {
        let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .args(["logs", flag])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    assert!(!session.exists());
    let after = span_tree(&console, "widgets%237");
    assert_eq!(tool_spans(&after)[0]["start"], 1790856005000.0);
    assert_eq!(tool_spans(&after)[0]["duration_ms"], 500.0);
    assert_eq!(tool_spans(&after)[0]["estimated"], true);
}

#[test]
fn waterfall_reads_start_records_without_spans_and_ignores_an_unfinished_tail() {
    let home = tempfile::tempdir().unwrap();
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.jsonl");
    let value = retained_command(home.path(), &session);
    fs::write(&session, "{\"type\":\"item.completed\",\"item\":{\"id\":\"legacy\",\"type\":\"command_execution\",\"command\":\"wrong\",\"status\":\"completed\"}}\n").unwrap();
    fs::write(
        session.with_extension("logs.jsonl"),
        format!(
            "{}{{\"resourceLogs\":",
            include_str!("../fixtures/waterfall-logs.jsonl")
        ),
    )
    .unwrap();
    let console = start_console(home.path(), &value);
    let response = console.get("/spans?command=widgets%237");
    assert_eq!(response.status(), 200);
    let tree: Value = response.json().unwrap();
    let tools = tool_spans(&tree);
    assert_eq!(tools.len(), 4);
    assert_eq!(
        tools
            .iter()
            .filter(|span| span["start"] == 1790856015000.0 && span["duration_ms"].is_null())
            .count(),
        2
    );
    assert!(!tree.to_string().contains("wrong"));
}

#[test]
fn waterfall_running_tools_use_start_records_and_keep_scopes_distinct() {
    let home = tempfile::tempdir().unwrap();
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.logs.jsonl");
    let mut value = retained_command(home.path(), &session);
    fs::write(&session, include_str!("../fixtures/waterfall-logs.jsonl")).unwrap();
    fs::write(
        dir.join("7-20261001T120000+0000-implement.spans.jsonl"),
        include_str!("../fixtures/waterfall-spans.jsonl"),
    )
    .unwrap();
    value["harness"] = json!("opencode");
    value["stages"] = json!([
        { "name": "implement", "began_at": value["started_at"], "session_log": session },
        { "name": "repair", "began_at": "2026-10-01T12:00:20.100+00:00", "session_log": session }
    ]);
    let console = start_console(home.path(), &value);
    let tree = span_tree(&console, "widgets%237");
    let tools = tool_spans(&tree);
    assert_eq!(tools.len(), 4);
    for start in [1790856020125.0, 1790856021250.0] {
        let tool = tools.iter().find(|span| span["start"] == start).unwrap();
        assert!(tool["duration_ms"].is_null());
    }
    let tool = tools
        .iter()
        .find(|span| span["start"] == 1790856015250.0)
        .unwrap();
    assert_eq!(tool["duration_ms"], 3500.0);
    // The reused Session and all calls stay under the first naming stage.
    let session = tree["spans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|span| span["kind"] == "sess")
        .unwrap();
    let stage = tree["spans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|span| span["name"] == "implement")
        .unwrap();
    assert_eq!(session["parent_id"], stage["id"]);
}

#[test]
fn waterfall_uses_session_trace_spans_instead_of_command_log_guesses() {
    let home = tempfile::tempdir().unwrap();
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.jsonl");
    let mut value = retained_command(home.path(), &session);
    fs::write(&session, "").unwrap();
    fs::write(
        session.with_extension("spans.jsonl"),
        include_str!("../fixtures/waterfall-spans.jsonl"),
    )
    .unwrap();
    fs::write(dir.join("7-fixture.log"), "thirdshift: 12:00:00 starting\nthirdshift: 12:00:15 implement: $ first <unsafe>\nthirdshift: 12:00:30 implement: $ second\nthirdshift: 12:01:00 finished\n").unwrap();
    value["harness"] = json!("muse");
    let console = start_console(home.path(), &value);
    let tree = span_tree(&console, "widgets%237");
    let tools = tool_spans(&tree);
    assert_eq!(tools.len(), 2);
    let first = tools
        .iter()
        .find(|span| span["id"] == "0000000000000001")
        .unwrap();
    let second = tools
        .iter()
        .find(|span| span["id"] == "0000000000000002")
        .unwrap();
    assert_eq!(first["start"], 1790856015250.0);
    assert_eq!(first["duration_ms"], 3500.0);
    assert_eq!(second["start"], 1790856016500.0);
    assert_eq!(second["duration_ms"], 625.0);
    assert!(first["name"].as_str().unwrap().contains("first <unsafe>"));

    // A sub-millisecond span must retain its duration rather than round to zero.
    let mut spans: Value =
        serde_json::from_str(include_str!("../fixtures/waterfall-spans.jsonl")).unwrap();
    spans["resourceSpans"][0]["scopeSpans"][0]["spans"][1]["endTimeUnixNano"] =
        json!("1790856016500500000");
    fs::write(
        session.with_extension("spans.jsonl"),
        serde_json::to_vec(&spans).unwrap(),
    )
    .unwrap();
    let tree = span_tree(&console, "widgets%237");
    let tools = tool_spans(&tree);
    let second = tools
        .iter()
        .find(|span| span["id"] == "0000000000000002")
        .unwrap();
    assert_eq!(second["duration_ms"], 0.5);
}
