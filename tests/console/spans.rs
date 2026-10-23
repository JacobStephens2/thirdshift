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
