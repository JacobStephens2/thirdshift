use super::*;

fn cli(home: &Path, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn span_json_retains_stable_run_and_dispatch_names() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 7, Some("merged"), 0);
    fixture["kind"] = json!("Run");
    fixture["issue"]["title"] = json!("Widgets");
    fixture["stages"] = json!([
        {"name": "implement", "began_at": fixture["started_at"], "session_log": null},
        {"name": "dispatched Run", "began_at": fixture["started_at"], "session_log": null}
    ]);
    write_record(home.path(), 7, &fixture);
    let tree = cli(home.path(), &["show", "widgets#7", "--spans", "--json"]);
    let spans = tree["spans"].as_array().unwrap();
    for span in spans
        .iter()
        .filter(|span| span["kind"] == "cmd" || span["kind"] == "run")
    {
        assert_eq!(span["name"], "Run · acme/widgets#7 · Widgets");
        let detail = cli(
            home.path(),
            &[
                "show",
                "widgets#7",
                "--span",
                span["id"].as_str().unwrap(),
                "--json",
            ],
        );
        assert_eq!(detail["name"], span["name"]);
    }
    assert!(
        spans
            .iter()
            .any(|span| span["kind"] == "stage" && span["name"] == "dispatched Run"),
        "{tree}"
    );
    let shown = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["show", "widgets#7", "--spans"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(shown.status.success(), "{shown:?}");
    let text = String::from_utf8(shown.stdout).unwrap();
    assert!(
        text.contains("Ticket run · acme/widgets#7 · Widgets"),
        "{text}"
    );
    assert!(text.contains("dispatched Ticket run"), "{text}");
}

#[test]
fn span_tree_and_details_pair_cli_with_routes_and_nest_child_runs() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut parent = record(home.path(), 7, Some("merged"), 0);
    parent["started_at"] = json!("2026-10-01T12:00:00Z");
    parent["ended_at"] = json!("2026-10-01T12:01:00Z");
    parent["stages"] = json!([{ "name": "Tickets", "began_at": parent["started_at"] }]);
    write_record(home.path(), 7, &parent);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("99-20261001T120000+0000-implement.logs.jsonl");
    let mut child = parent.clone();
    child["kind"] = json!("Run");
    child["name"] = json!("ticket-99");
    child["issue"]["number"] = json!(99);
    child["parent_record"] = json!(dir.join("7-fixture.command.json"));
    child["session_log"] = json!(session);
    child["stages"] = json!([
        {"name":"implement","began_at":"2026-10-01T12:00:10Z","session_log":session},
        {"name":"pull request","began_at":"2026-10-01T12:00:50Z","session_log":session}
    ]);
    fs::write(
        dir.join("7-fixture.issue-99.child.json"),
        serde_json::to_vec(&child).unwrap(),
    )
    .unwrap();
    fs::write(support::session_trace::artifact(&session, "logs.jsonl"), "").unwrap();
    let attr = |key: &str, value: &str| json!({"key":key,"value":{"stringValue":value}});
    fs::write(support::session_trace::artifact(&session, "spans.jsonl"), serde_json::to_vec(&json!({
        "resourceSpans":[{"scopeSpans":[{"spans":[
            {"spanId":"1111111111111111","name":"invoke_agent implement","startTimeUnixNano":"1790856010000000000","endTimeUnixNano":"1790856050000000000","attributes":[attr("gen_ai.operation.name","invoke_agent"),attr("thirdshift.session.prompt","Build widgets")]},
            {"spanId":"2222222222222222","parentSpanId":"1111111111111111","name":"agent reviewer","startTimeUnixNano":"1790856015000000000","endTimeUnixNano":"1790856040000000000","attributes":[attr("thirdshift.type","subagent_started"),attr("thirdshift.description","Review widgets")]},
            {"spanId":"3333333333333333","parentSpanId":"2222222222222222","name":"execute_tool Bash","startTimeUnixNano":"1790856016250000000","endTimeUnixNano":"1790856019750000000","attributes":[attr("gen_ai.operation.name","execute_tool"),attr("gen_ai.tool.name","Bash"),attr("gen_ai.tool.call.arguments","{\"command\":\"cargo test <unsafe>\"}"),attr("gen_ai.tool.call.result","passed")],"status":{"code":2,"message":"test failed"}}
        ]}]}]
    })).unwrap()).unwrap();
    let tree = cli(home.path(), &["show", "widgets#7", "--spans", "--json"]);
    let nodes = tree["spans"].as_array().unwrap();
    let of_kind = |kind| {
        nodes
            .iter()
            .filter(|node| node["kind"] == kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(of_kind("cmd").len(), 1);
    assert_eq!(of_kind("run").len(), 2);
    assert_eq!(of_kind("sess").len(), 1, "a reused session appears once");
    let tickets = nodes.iter().find(|node| node["name"] == "Tickets").unwrap();
    let run = nodes
        .iter()
        .find(|node| node["name"].as_str().unwrap().contains("#99"))
        .unwrap();
    assert_eq!(run["parent_id"], tickets["id"]);
    let implement = nodes
        .iter()
        .find(|node| node["name"] == "implement")
        .unwrap();
    assert_eq!(of_kind("sess")[0]["parent_id"], implement["id"]);
    let tool = of_kind("tool")[0];
    assert_eq!(tool["parent_id"], "2222222222222222");
    assert_eq!(tool["duration_ms"], 3500.0);
    assert_eq!(tool["error"], "test failed");
    let detail = cli(
        home.path(),
        &["show", "widgets#7", "--span", "3333333333333333", "--json"],
    );
    assert_eq!(&detail, tool);
    assert_eq!(detail["input"], "{\"command\":\"cargo test <unsafe>\"}");
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let route: Value = console.get("/spans?command=widgets%237").json().unwrap();
    assert_eq!(route["spans"], tree["spans"]);
    let route: Value = console
        .get("/spans?command=widgets%237&span=3333333333333333")
        .json()
        .unwrap();
    assert_eq!(route, detail);
    let html = console
        .get("/waterfall?command=widgets%237")
        .text()
        .unwrap();
    assert!(html.contains("id=\"span-tree\""));
    assert!(!html.contains("cargo test <unsafe>"));
}

fn export(home: &Path, selector: &str, name: &str) -> std::path::PathBuf {
    let dir = home.join(name);
    let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["logs", selector, "--export", dir.to_str().unwrap()])
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    dir
}

fn exported_spans(dir: &Path) -> Vec<Value> {
    fs::read_to_string(dir.join("spans.jsonl"))
        .unwrap()
        .lines()
        .flat_map(|line| {
            let request: Value = serde_json::from_str(line).unwrap();
            request["resourceSpans"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|resource| resource["scopeSpans"].as_array().unwrap().clone())
                .flat_map(|scope| scope["spans"].as_array().unwrap().clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn exported<'a>(spans: &'a [Value], id: &Value) -> &'a Value {
    spans
        .iter()
        .find(|span| &span["spanId"] == id)
        .unwrap_or_else(|| panic!("{id} was not exported"))
}

#[test]
fn waterfall_shows_a_selected_child_subtree_while_export_keeps_the_whole_owner() {
    let mut states = vec![(Some("merged"), "merged"), (None, "died")];
    #[cfg(unix)]
    states.push((None, "running"));
    for (outcome, status) in states {
        let home = tempfile::tempdir().unwrap();
        configure(home.path());
        #[cfg(unix)]
        let work =
            super::watching::FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
        let mut parent = record(home.path(), 7, outcome, 0);
        #[cfg(unix)]
        if status == "running" {
            parent = super::watching::live_record(home.path(), 7, work.0.id());
        }
        parent["stages"] = json!([{ "name": "Tickets", "began_at": parent["started_at"] }]);
        write_record(home.path(), 7, &parent);
        let dir = home
            .path()
            .join(".thirdshift/logs/acme/widgets/commands/issue");
        for number in [98, 99] {
            let mut child = parent.clone();
            child["kind"] = json!("Run");
            child["name"] = json!(format!("ticket-{number}"));
            child["issue"]["number"] = json!(number);
            child["parent_record"] = json!(dir.join("7-fixture.command.json"));
            child["stages"] = json!([{"name": "implement", "began_at": parent["started_at"]}]);
            fs::write(
                dir.join(format!("7-fixture.issue-{number}.child.json")),
                serde_json::to_vec(&child).unwrap(),
            )
            .unwrap();
        }
        let whole = cli(home.path(), &["show", "widgets#7", "--spans", "--json"]);
        let whole = whole["spans"].as_array().unwrap();
        let selected = cli(home.path(), &["show", "widgets#99", "--spans", "--json"]);
        let selected = selected["spans"].as_array().unwrap();
        let run = |spans: &[Value], subject: &str| {
            spans
                .iter()
                .find(|span| {
                    span["kind"] == "run" && span["name"].as_str().unwrap().contains(subject)
                })
                .cloned()
        };

        // The selected Ticket keeps its outer Command's identity and only its subtree.
        let kinds: Vec<_> = selected.iter().map(|span| span["kind"].clone()).collect();
        assert_eq!(kinds, ["cmd", "run", "stage"], "{status}: {selected:?}");
        assert_eq!(selected[0]["id"], whole[0]["id"], "{status}");
        assert!(selected[0]["name"].as_str().unwrap().contains("#99"));
        assert_eq!(selected[0]["status"], status);
        assert_eq!(selected[1]["id"], run(whole, "#99").unwrap()["id"]);
        assert_eq!(selected[1]["parent_id"], selected[0]["id"]);
        assert!(run(selected, "#98").is_none() && run(selected, "#7").is_none());

        // Export through the same Ticket includes its sibling and its owner.
        let spans = exported_spans(&export(home.path(), "widgets#99", "export"));
        assert_eq!(spans.len(), whole.len(), "{status}");
        for node in whole {
            let span = exported(&spans, &node["id"]);
            let parent = node["parent_id"].as_str().unwrap_or_default();
            assert_eq!(span["parentSpanId"].as_str().unwrap_or_default(), parent);
        }
        assert_eq!(exported(&spans, &whole[0]["id"])["name"], "Command");
    }
}

#[test]
fn waterfall_attaches_a_reused_session_to_its_first_stage_while_export_keeps_its_parent() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 7, Some("merged"), 0);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T120000+0000-implement.logs.jsonl");
    fixture["kind"] = json!("Run");
    fixture["started_at"] = json!("2026-10-01T12:00:00Z");
    fixture["ended_at"] = json!("2026-10-01T12:01:00Z");
    fixture["session_log"] = json!(session);
    fixture["stages"] = json!([
        {"name":"implement","began_at":"2026-10-01T12:00:10Z","session_log":session},
        {"name":"pull request","began_at":"2026-10-01T12:00:50Z","session_log":session}
    ]);
    write_record(home.path(), 7, &fixture);
    let spans_file = support::session_trace::artifact(&session, "spans.jsonl");
    fs::write(&spans_file, "").unwrap();
    fs::write(&session, "").unwrap();
    // An empty Session exports only the records, revealing the Trace ID.
    let records = exported_spans(&export(home.path(), "widgets#7", "records"));
    let trace = records[0]["traceId"].as_str().unwrap();
    let tree = cli(home.path(), &["show", "widgets#7", "--spans", "--json"]);
    let stage = |name: &str| {
        tree["spans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|span| span["name"] == name)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (implement, pull_request) = (stage("implement"), stage("pull request"));
    // An older trace recorded the later stage that reused this Session.
    let request = format!(
        r#"{{"resourceSpans":[{{"scopeSpans":[{{"spans":[{{"traceId":"{trace}","spanId":"4444444444444444","parentSpanId":"{pull_request}","name":"invoke_agent implement","startTimeUnixNano":"1790856010000000000","endTimeUnixNano":"1790856050000000000","attributes":[{{"key":"gen_ai.operation.name","value":{{"stringValue":"invoke_agent"}}}}],"futureField":true}}]}}]}}],"futureRequest":1}}"#
    );
    fs::write(&spans_file, format!("{request}\n")).unwrap();
    let log = format!(
        r#"{{"resourceLogs":[{{"scopeLogs":[{{"logRecords":[{{"traceId":"{trace}","spanId":"4444444444444444","body":{{"stringValue":"retained"}},"futureField":true}}]}}]}}]}}"#
    );
    fs::write(&session, format!("{log}\n")).unwrap();

    let tree = cli(home.path(), &["show", "widgets#7", "--spans", "--json"]);
    let shown = tree["spans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|span| span["id"] == "4444444444444444")
        .unwrap();
    assert_eq!(shown["kind"], "sess");
    assert_eq!(shown["parent_id"], implement.as_str());

    let dir = export(home.path(), "widgets#7", "export");
    let spans = exported_spans(&dir);
    assert_eq!(
        exported(&spans, &json!("4444444444444444"))["parentSpanId"],
        pull_request.as_str()
    );
    let lines = fs::read_to_string(dir.join("spans.jsonl")).unwrap();
    assert!(lines.lines().any(|line| line == request), "{lines}");
    assert_eq!(
        fs::read_to_string(dir.join("logs.jsonl")).unwrap(),
        format!("{log}\n")
    );
}
