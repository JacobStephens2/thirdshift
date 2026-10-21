use crate::harness::{Harness, interpretation_tests::stream};
use serde_json::{Value, json};

fn decoded(harness: Harness, records: impl IntoIterator<Item = Value>) -> Vec<Value> {
    let mut decoder = stream(harness, "");
    records
        .into_iter()
        .flat_map(|record| {
            decoder
                .decode(&record.to_string())
                .events
                .into_iter()
                .map(|event| serde_json::to_value(event).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn codex_final_snapshots_keep_inputs_and_turn_end_stops_open_work_and_subagents() {
    let events = decoded(
        Harness::Codex,
        include_str!("../../../tests/fixtures/normalized-codex.jsonl")
            .lines()
            .map(|line| serde_json::from_str(line).unwrap()),
    );
    let web = events
        .iter()
        .find(|event| event["type"] == "tool_call_ended" && event["id"] == "web")
        .unwrap();
    assert_eq!(web["input"]["query"], "rust docs");
    let build = events
        .iter()
        .find(|event| event["type"] == "tool_call_ended" && event["id"] == "build")
        .unwrap();
    assert_eq!(build["outcome"], "killed");
    assert!(
        events.contains(
            &json!({"type":"subagent_ended","id":"child","outcome":"killed","result":null})
        )
    );
    assert!(
        events.contains(&json!({"type":"work_ended","id":"interrupted","outcome":"interrupted"}))
    );
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "error" && event["kind"] == "rate_limit")
    );
    assert!(events.iter().any(|event| event["type"] == "turn_ended"
        && event["id"] == "turn-2"
        && event["outcome"] == "interrupted"));
}

#[test]
fn claude_partial_frames_close_one_model_call_with_the_final_usage() {
    let events = decoded(
        Harness::Claude,
        include_str!("../../../tests/fixtures/normalized-claude.jsonl")
            .lines()
            .map(|line| serde_json::from_str(line).unwrap()),
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["model_call_started", "message", "model_call_ended", "usage"]
    );
    assert_eq!(
        events[2]["usage"],
        json!({"input":8,"output":4,"cache_read":2,"cache_write":3})
    );
    assert_eq!(events[3]["cost_usd"].as_f64(), Some(0.0));
}

#[test]
fn grok_messages_keep_usage_but_zero_cost_stays_unknown_and_result_text_is_not_lost() {
    let mut decoder = stream(Harness::Grok, "");
    let mut events = Vec::new();
    let mut progress = Vec::new();
    for raw in include_str!("../../../tests/fixtures/normalized-grok.jsonl").lines() {
        let update = decoder.decode(raw);
        progress.extend(update.progress);
        events.extend(
            update
                .events
                .into_iter()
                .map(|event| serde_json::to_value(event).unwrap()),
        );
    }
    assert_eq!(progress, ["session started", "$ cargo check"]);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "model_call_started")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "model_call_ended")
            .count(),
        2
    );
    assert!(events.contains(&json!({"type":"message","role":"assistant","text":"Final reply from Grok.","parent_id":null})));
    let usage = events
        .iter()
        .find(|event| event["type"] == "usage")
        .unwrap();
    assert_eq!(usage["cost_usd"], Value::Null);
    assert_eq!(
        usage["usage"],
        json!({"input":20,"output":6,"cache_read":2,"cache_write":3})
    );
    let completion = crate::harness::interpretation_tests::finish(decoder);
    assert_eq!(
        completion.outcome.unwrap().final_message.as_deref(),
        Some("Final reply from Grok.")
    );
}

#[test]
fn codex_normalizes_item_lifecycles_and_keeps_turn_totals_distinct_from_model_calls() {
    let events = decoded(
        Harness::Codex,
        [
            json!({"type":"thread.started","thread_id":"session"}),
            json!({"type":"turn.started"}),
            json!({"type":"item.started","item":{"id":"c1","type":"command_execution","command":"cargo test","status":"in_progress"}}),
            json!({"type":"item.updated","item":{"id":"c1","type":"command_execution","command":"cargo test","status":"in_progress","aggregated_output":"running"}}),
            json!({"type":"item.completed","item":{"id":"c1","type":"command_execution","command":"cargo test","status":"failed","exit_code":1,"aggregated_output":"assertion failed"}}),
            json!({"type":"item.completed","item":{"id":"patch","type":"file_change","changes":[{"path":"src/main.rs","kind":"update"}],"status":"completed"}}),
            json!({"type":"item.completed","item":{"id":"deny","type":"command_execution","command":"git push","status":"declined"}}),
            json!({"type":"item.started","item":{"id":"mcp","type":"mcp_tool_call","server":"files","tool":"read","arguments":{"path":"README.md"},"status":"in_progress"}}),
            json!({"type":"item.completed","item":{"id":"mcp","type":"mcp_tool_call","server":"files","tool":"read","arguments":{"path":"README.md"},"error":{"message":"unavailable"},"status":"failed"}}),
            json!({"type":"item.started","item":{"id":"web","type":"web_search","query":"rust docs"}}),
            json!({"type":"item.completed","item":{"id":"web","type":"web_search","query":"rust docs","results":[{"url":"https://doc.rust-lang.org"}]}}),
            json!({"type":"item.started","item":{"id":"spawn","type":"collab_tool_call","tool":"spawn_agent","sender_thread_id":"session","receiver_thread_ids":[],"status":"in_progress"}}),
            json!({"type":"item.completed","item":{"id":"spawn","type":"collab_tool_call","tool":"spawn_agent","sender_thread_id":"session","receiver_thread_ids":["child"],"agents_states":{"child":{"status":"running"}},"status":"completed"}}),
            json!({"type":"item.completed","item":{"id":"wait","type":"collab_tool_call","tool":"wait","receiver_thread_ids":["child"],"agents_states":{"child":{"status":"completed","message":"Reviewed."}},"status":"completed"}}),
            json!({"type":"item.started","item":{"id":"background","type":"command_execution","command":"cargo build","status":"in_progress"}}),
            json!({"type":"item.completed","item":{"id":"reply","type":"agent_message","text":"Done."}}),
            json!({"type":"turn.completed","usage":{"input_tokens":100,"output_tokens":7,"cached_input_tokens":20,"cache_write_input_tokens":3}}),
        ],
    );
    let starts: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "tool_call_started")
        .map(|event| event["id"].as_str().unwrap())
        .collect();
    let ends: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "tool_call_ended")
        .map(|event| event["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        starts,
        [
            "c1",
            "patch",
            "deny",
            "mcp",
            "web",
            "spawn",
            "wait",
            "background"
        ]
    );
    assert_eq!(ends, starts);
    assert_eq!(
        events
            .iter()
            .find(|event| event["type"] == "tool_call_ended" && event["id"] == "c1")
            .unwrap()["outcome"],
        "failed"
    );
    assert_eq!(
        events
            .iter()
            .find(|event| event["type"] == "tool_call_ended" && event["id"] == "deny")
            .unwrap()["outcome"],
        "denied"
    );
    assert_eq!(
        events
            .iter()
            .find(|event| event["type"] == "tool_call_ended" && event["id"] == "mcp")
            .unwrap()["result"],
        json!({"message":"unavailable"})
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "subagent_started")
            .count(),
        1
    );
    assert!(events.contains(
        &json!({"type":"subagent_ended","id":"child","outcome":"succeeded","result":"Reviewed."})
    ));
    assert!(events.contains(&json!({"type":"work_ended","id":"background","outcome":"killed"})));
    assert!(
        events.contains(
            &json!({"type":"message","role":"assistant","text":"Done.","parent_id":null})
        )
    );
    assert!(events.contains(&json!({"type":"turn_ended","id":"turn-1","usage":{"input":100,"output":7,"cache_read":20,"cache_write":3},"outcome":"succeeded"})));
    assert!(
        !events
            .iter()
            .any(|event| event["type"] == "model_call_started")
    );
}

#[test]
fn claude_keeps_denials_limits_killed_tasks_and_background_subagent_lifecycles() {
    let events = decoded(
        Harness::Claude,
        [
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"a1","name":"Agent","input":{"description":"Review","run_in_background":true}}]}}),
            json!({"type":"system","subtype":"task_started","task_id":"agent-task","tool_use_id":"a1","task_type":"local_agent","description":"Review"}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a1","content":"Async agent launched"}]}}),
            json!({"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git push"}}]}}),
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Permission to use Bash has been denied.","is_error":true}]}}),
            json!({"type":"result","subtype":"success"}),
            json!({"type":"system","subtype":"task_notification","task_id":"agent-task","status":"completed","summary":"Reviewed."}),
            json!({"type":"system","subtype":"task_started","task_id":"build","task_type":"local_bash","description":"cargo test"}),
            json!({"type":"system","subtype":"task_updated","task_id":"build","patch":{"status":"killed"}}),
            json!({"type":"system","subtype":"task_notification","task_id":"build","status":"stopped"}),
            json!({"type":"assistant","is_api_error_message":true,"api_error":"usage_limit_reached","api_error_status":429,"api_error_params":{"rate_limit_info":{"resetsAt":1791597600}},"message":{"content":[{"type":"text","text":"You've hit your limit"}]}}),
            json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1791597600}}),
        ],
    );
    assert!(events.iter().any(|event| event["type"] == "tool_call_ended"
        && event["id"] == "t1"
        && event["outcome"] == "denied"));
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "subagent_started")
            .count(),
        1
    );
    let ended: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "subagent_ended")
        .collect();
    assert_eq!(
        ended,
        [&json!({"type":"subagent_ended","id":"a1","outcome":"succeeded","result":"Reviewed."})]
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "work_ended")
            .collect::<Vec<_>>(),
        [&json!({"type":"work_ended","id":"build","outcome":"killed"})]
    );
    let errors: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "error")
        .collect();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0]["kind"], "usage_limit");
    assert_eq!(errors[0]["details"]["api_error_status"], 429);
    assert_eq!(errors[1]["kind"], "rate_limit");
    assert_eq!(
        errors[1]["details"]["rate_limit_info"]["resetsAt"],
        1791597600_u64
    );
}

#[test]
fn claude_normalizes_calls_messages_and_reported_usage_without_changing_progress() {
    let mut decoder = stream(Harness::Claude, "");
    let mut events = Vec::new();
    let mut progress = Vec::new();
    for raw in [
        json!({"type":"assistant","message":{"id":"m1","model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"Checking."}],"usage":{"input_tokens":8,"output_tokens":1,"cache_read_input_tokens":2}}}),
        json!({"type":"assistant","message":{"id":"m1","model":"claude-opus-4-8","role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo check"}}],"usage":{"input_tokens":8,"output_tokens":4,"cache_read_input_tokens":2,"cache_creation_input_tokens":3}}}),
        json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok","is_error":false}]}}),
        json!({"type":"assistant","message":{"id":"m2","model":"claude-opus-4-8","role":"assistant","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":12,"output_tokens":2}}}),
        json!({"type":"result","subtype":"success","result":"Done.","total_cost_usd":0.25,"usage":{"input_tokens":20,"output_tokens":6}}),
    ] {
        let update = decoder.decode(&raw.to_string());
        progress.extend(update.progress);
        events.extend(update.events);
    }
    assert_eq!(progress, ["$ cargo check"]);
    let events: Value = serde_json::to_value(events).unwrap();
    assert_eq!(
        events,
        json!([
            {"type":"model_call_started","id":"m1","parent_id":null,"model":"claude-opus-4-8"},
            {"type":"message","role":"assistant","text":"Checking.","parent_id":null},
            {"type":"tool_call_started","id":"t1","parent_id":null,"tool":"Bash","input":{"command":"cargo check"}},
            {"type":"model_call_ended","id":"m1","parent_id":null,"model":"claude-opus-4-8","usage":{"input":8,"output":4,"cache_read":2,"cache_write":3},"cost_usd":null,"outcome":"succeeded"},
            {"type":"tool_call_ended","id":"t1","parent_id":null,"tool":"Bash","input":{"command":"cargo check"},"result":"ok","outcome":"succeeded"},
            {"type":"model_call_started","id":"m2","parent_id":null,"model":"claude-opus-4-8"},
            {"type":"message","role":"assistant","text":"Done.","parent_id":null},
            {"type":"model_call_ended","id":"m2","parent_id":null,"model":"claude-opus-4-8","usage":{"input":12,"output":2,"cache_read":null,"cache_write":null},"cost_usd":null,"outcome":"succeeded"},
            {"type":"usage","scope":"session","model":null,"usage":{"input":20,"output":6,"cache_read":null,"cache_write":null},"cost_usd":0.25}
        ])
    );
    assert_eq!(decoder.observed_models(), ["claude-opus-4-8"]);
}
