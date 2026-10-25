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
fn waterfall_reads_start_records_without_spans_and_ignores_an_unfinished_tail() {
    let home = tempfile::tempdir().unwrap();
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.logs.jsonl");
    let value = retained_command(home.path(), &session);
    fs::write(
        &session,
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
    let session = dir.join("7-20261001T120000+0000-implement.logs.jsonl");
    let mut value = retained_command(home.path(), &session);
    fs::write(&session, "").unwrap();
    fs::write(
        session.with_file_name("7-20261001T120000+0000-implement.spans.jsonl"),
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
        session.with_file_name("7-20261001T120000+0000-implement.spans.jsonl"),
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

#[test]
fn waterfall_replay_keeps_unknown_clocks_backwards_intervals_and_the_last_failure() {
    let home = tempfile::tempdir().unwrap();
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.logs.jsonl");
    let value = retained_command(home.path(), &session);
    let record = |time: u64, events: Value| {
        json!({"resourceLogs":[{"scopeLogs":[{"logRecords":[{
            "timeUnixNano": time.to_string(),
            "attributes":[{"key":"thirdshift.events","value":{"stringValue":events.to_string()}}]
        }]}]}]})
        .to_string()
            + "\n"
    };
    let started = |id: &str| json!({"type":"tool_call_started","id":id,"parent_id":null,"tool":"Bash","input":{}});
    let ended = |id: &str, outcome: &str| json!({"type":"tool_call_ended","id":id,"parent_id":null,"tool":"Bash","input":{},"result":"","outcome":outcome,"amended":false});
    fs::write(
        &session,
        [
            record(0, json!([started("unknown")])),
            record(1790856020000000000, json!([started("backwards")])),
            record(
                1790856019000000000,
                json!([ended("backwards", "succeeded")]),
            ),
            record(1790856021000000000, json!([ended("retried", "failed")])),
            record(1790856022000000000, json!([ended("retried", "succeeded")])),
        ]
        .concat(),
    )
    .unwrap();
    let console = start_console(home.path(), &value);
    let tree = span_tree(&console, "widgets%237");
    let tool = |id: &str| {
        tool_spans(&tree)
            .into_iter()
            .find(|span| span["attributes"]["thirdshift.id"] == id)
            .unwrap()
            .clone()
    };
    let unknown = tool("unknown");
    assert!(unknown["start"].is_null(), "{unknown}");
    assert_eq!(unknown["status"], "unknown");
    let backwards = tool("backwards");
    assert_eq!(backwards["start"], 1790856020000.0);
    assert_eq!(backwards["end"], 1790856019000.0);
    assert!(backwards["duration_ms"].is_null(), "{backwards}");
    let retried = tool("retried");
    assert_eq!(retried["start"], 1790856021000.0);
    assert_eq!(retried["end"], 1790856021000.0);
    assert_eq!(retried["status"], "ok");
    assert_eq!(retried["error"], "Failed", "{retried}");
}
