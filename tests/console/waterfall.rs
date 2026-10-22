use super::*;

#[test]
fn waterfall_reads_start_records_without_spans_and_ignores_an_unfinished_tail() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut value = record(home.path(), 7, Some("merged"), 0);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.jsonl");
    fs::write(&session, "{\"type\":\"item.completed\",\"item\":{\"id\":\"legacy\",\"type\":\"command_execution\",\"command\":\"wrong\",\"status\":\"completed\"}}\n").unwrap();
    fs::write(
        session.with_extension("logs.jsonl"),
        format!(
            "{}{{\"resourceLogs\":",
            include_str!("../fixtures/waterfall-logs.jsonl")
        ),
    )
    .unwrap();
    value["started_at"] = json!("2026-10-01T12:00:00+00:00");
    value["ended_at"] = json!("2026-10-01T12:01:00+00:00");
    value["stages"] =
        json!([{ "name": "implement", "began_at": value["started_at"], "session_log": session }]);
    value["session_log"] = json!(session);
    write_record(home.path(), 7, &value);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = console.get("/waterfall?command=widgets%237");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert_eq!(
        html.matches("class=\"waterfall-tool\"").count(),
        4,
        "{html}"
    );
    assert_eq!(
        html.matches("data-start=\"2026-10-01T12:00:15+00:00\" data-seconds=\"\"")
            .count(),
        2,
        "{html}"
    );
    assert!(!html.contains("wrong"), "{html}");
}

#[test]
fn waterfall_running_tools_use_start_records_and_keep_scopes_distinct() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut value = record(home.path(), 7, Some("merged"), 0);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.logs.jsonl");
    fs::write(&session, include_str!("../fixtures/waterfall-logs.jsonl")).unwrap();
    fs::write(
        dir.join("7-20261001T120000+0000-implement.spans.jsonl"),
        include_str!("../fixtures/waterfall-spans.jsonl"),
    )
    .unwrap();
    value["harness"] = json!("opencode");
    value["started_at"] = json!("2026-10-01T12:00:00+00:00");
    value["ended_at"] = json!("2026-10-01T12:01:00+00:00");
    value["stages"] = json!([
        { "name": "implement", "began_at": value["started_at"], "session_log": session },
        { "name": "repair", "began_at": "2026-10-01T12:00:20.100+00:00", "session_log": session }
    ]);
    value["session_log"] = json!(session);
    write_record(home.path(), 7, &value);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console
        .get("/waterfall?command=widgets%237")
        .text()
        .unwrap();
    assert_eq!(
        html.matches("class=\"waterfall-tool\"").count(),
        4,
        "{html}"
    );
    let repair = html.split("<h4>repair</h4>").nth(1).unwrap();
    assert!(
        repair.contains("data-start=\"2026-10-01T12:00:20.125+00:00\" data-seconds=\"\""),
        "{html}"
    );
    assert!(
        repair.contains("data-start=\"2026-10-01T12:00:21.250+00:00\" data-seconds=\"\""),
        "{html}"
    );
    assert!(
        html.contains("data-start=\"2026-10-01T12:00:15.250+00:00\" data-seconds=\"3.5\""),
        "{html}"
    );
}

#[test]
fn waterfall_uses_session_trace_spans_instead_of_command_log_guesses() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut value = record(home.path(), 7, Some("merged"), 0);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.jsonl");
    fs::write(&session, "").unwrap();
    fs::write(
        session.with_extension("spans.jsonl"),
        include_str!("../fixtures/waterfall-spans.jsonl"),
    )
    .unwrap();
    fs::write(dir.join("7-fixture.log"), "thirdshift: 12:00:00 starting\nthirdshift: 12:00:15 implement: $ first <unsafe>\nthirdshift: 12:00:30 implement: $ second\nthirdshift: 12:01:00 finished\n").unwrap();
    value["harness"] = json!("muse");
    value["started_at"] = json!("2026-10-01T12:00:00+00:00");
    value["ended_at"] = json!("2026-10-01T12:01:00+00:00");
    value["stages"] =
        json!([{ "name": "implement", "began_at": value["started_at"], "session_log": session }]);
    value["session_log"] = json!(session);
    write_record(home.path(), 7, &value);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console
        .get("/waterfall?command=widgets%237")
        .text()
        .unwrap();
    assert_eq!(
        html.matches("class=\"waterfall-tool\"").count(),
        2,
        "{html}"
    );
    assert!(
        html.contains("data-start=\"2026-10-01T12:00:15.250+00:00\" data-seconds=\"3.5\""),
        "{html}"
    );
    assert!(
        html.contains("data-start=\"2026-10-01T12:00:16.500+00:00\" data-seconds=\"0.625\""),
        "{html}"
    );
    assert!(html.contains("first &#60;unsafe&#62;"), "{html}");

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
    let html = console
        .get("/waterfall?command=widgets%237")
        .text()
        .unwrap();
    assert!(
        html.contains("data-start=\"2026-10-01T12:00:16.500+00:00\" data-seconds=\"0.0005\""),
        "{html}"
    );
}
