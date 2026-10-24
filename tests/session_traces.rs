//! The binary's recording contract, including child processes and live files.
mod support;

use serde_json::Value;
use std::fs;
use std::path::Path;
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";

fn opens_pr(issue: u32, base: &str) -> String {
    format!(
        "echo work > work-{issue}.txt\ngit add .\ngit commit -qm 'Do work'\ngh pr create --base {base} --head issue-{issue} --title Work --body 'Closes #{issue}'\n"
    )
}

fn spans(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|line| {
            let request: Value = serde_json::from_str(line).unwrap();
            request["resourceSpans"][0]["scopeSpans"][0]["spans"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect()
}

fn assert_harness_version(scenario: &Scenario, expected: &str) {
    let mut resources = 0;
    for name in scenario
        .entries(LOGS)
        .into_iter()
        .filter(|name| name.ends_with(".logs.jsonl") || name.ends_with(".spans.jsonl"))
    {
        for line in fs::read_to_string(scenario.path(LOGS).join(name))
            .unwrap()
            .lines()
        {
            let request: Value = serde_json::from_str(line).unwrap();
            let resource = request["resourceLogs"]
                .as_array()
                .or_else(|| request["resourceSpans"].as_array())
                .unwrap();
            for batch in resource {
                let version = batch["resource"]["attributes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|attr| attr["key"] == "thirdshift.harness.version");
                assert_eq!(
                    version.map(|attr| &attr["value"]["stringValue"]),
                    Some(&Value::String(expected.into())),
                    "{batch}"
                );
                resources += 1;
            }
        }
    }
    assert!(resources > 0, "no Session trace resources");
}

fn version_queries(scenario: &Scenario, harness: &str) -> usize {
    fs::read(scenario.path(&format!("{harness}-calls.versions.json")))
        .map(|bytes| {
            serde_json::from_slice::<Value>(&bytes)
                .unwrap()
                .as_array()
                .unwrap()
                .len()
        })
        .unwrap_or(0)
}

#[test]
fn all_six_harnesses_record_their_version_in_session_trace_resources() {
    for harness in ["claude", "codex", "agy", "grok", "muse", "opencode"] {
        let scenario = Scenario::new();
        scenario.agent_does(&opens_pr(7, "main"));
        let result = scenario.run(&["harness", harness, &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(0), "{harness}: {}", result.stderr);
        assert_harness_version(&scenario, "9.8.7");
        assert_eq!(
            version_queries(&scenario, harness),
            usize::from(harness != "claude")
        );
    }
}

#[test]
fn parallel_ticket_sessions_share_one_version_query_but_a_new_command_reads_again() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[]), (22, &[])]);
    scenario.agent_does_for(21, &opens_pr(21, "issue-20"));
    scenario.agent_does_for(22, &opens_pr(22, "issue-20"));
    let result = scenario.run(&["harness", "codex", &scenario.issue_url(20)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.codex_calls().len(), 3);
    assert_harness_version(&scenario, "9.8.7");
    assert_eq!(version_queries(&scenario, "codex"), 1);

    scenario.agent_does_for(7, &opens_pr(7, "main"));
    let result = scenario.run(&["harness", "codex", &scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(version_queries(&scenario, "codex"), 2);
    assert_harness_version(&scenario, "9.8.7");
}

#[test]
fn missing_or_failed_version_lookups_record_unknown_without_failing_the_session() {
    for env in [
        vec![("FAKE_HARNESS_VERSION", "  ")],
        vec![("FAKE_HARNESS_VERSION_ERROR", "1")],
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&opens_pr(7, "main"));
        let mut env = env;
        env.push(("FAKE_CLAUDE_STREAM_VERSION", ""));
        let result = scenario.run_with_env(&[&scenario.issue_url(7)], &env);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_harness_version(&scenario, "unknown");
        assert_eq!(version_queries(&scenario, "claude"), 1);
    }
}

#[test]
fn a_tools_version_field_does_not_replace_the_harness_version() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "echo '{{\"type\":\"tool\",\"version\":\"tool-v2\"}}'\n{}",
        opens_pr(7, "main")
    ));
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_harness_version(&scenario, "9.8.7");
    assert_eq!(version_queries(&scenario, "claude"), 0);
}

#[test]
fn the_security_reviews_harness_has_its_own_command_version_cache() {
    let scenario = Scenario::new();
    scenario.user_config_is("[harness]\ndefault = \"codex\"\n[security]\nharness = \"claude\"\n");
    scenario.agent_does_in_session(1, &opens_pr(7, "main"));
    scenario.agent_does_in_session(2, r#"
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
printf '[]\n' > "$report"
printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    let result = scenario.run_with_env(
        &[&scenario.issue_url(7), "security-review"],
        &[("FAKE_CLAUDE_STREAM_VERSION", "")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_harness_version(&scenario, "9.8.7");
    assert_eq!(version_queries(&scenario, "codex"), 1);
    assert_eq!(version_queries(&scenario, "claude"), 1);
}

#[test]
fn all_six_harnesses_leave_one_inventory_entry_and_both_otlp_files() {
    for harness in ["claude", "codex", "agy", "grok", "muse", "opencode"] {
        let scenario = Scenario::new();
        scenario.agent_does(&opens_pr(7, "main"));
        let result = scenario.run(&["harness", harness, &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(0), "{harness}: {}", result.stderr);
        let folder = scenario.path(LOGS);
        let logs: Vec<_> = scenario
            .entries(LOGS)
            .into_iter()
            .filter(|name| name.ends_with(".logs.jsonl"))
            .collect();
        assert_eq!(logs.len(), 1, "{harness}");
        let path = folder.join(&logs[0]);
        for line in fs::read_to_string(&path).unwrap().lines() {
            let request: opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest =
                serde_json::from_value(serde_json::from_str::<Value>(line).unwrap()).unwrap();
            assert_eq!(request.resource_logs[0].scope_logs[0].log_records.len(), 1);
        }
        let stem = logs[0].strip_suffix(".logs.jsonl").unwrap();
        assert!(!folder.join(format!("{stem}.jsonl")).exists());
        assert!(folder.join(format!("{stem}.models.json")).exists());
        let record = scenario.command_record(LOGS);
        assert_eq!(record["session_log"], serde_json::json!(path));
        assert!(
            record["stages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|stage| { stage["session_log"] == serde_json::json!(path) })
        );
        assert!(
            spans(&folder.join(format!("{stem}.spans.jsonl")))
                .iter()
                .any(|span| span["name"] == "invoke_agent implement"
                    && span["parentSpanId"].is_string())
        );
        let inventory = scenario.run(&["logs", "--json"]);
        assert_eq!(inventory.code, Some(0), "{}", inventory.stderr);
        let inventory: Value = serde_json::from_str(&inventory.stdout).unwrap();
        assert_eq!(
            inventory["logs"].as_array().unwrap().len(),
            2,
            "{inventory}"
        );
    }
}

#[test]
fn ticket_runs_share_the_spec_commands_trace() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[]), (22, &[21])]);
    scenario.agent_does_for(21, &opens_pr(21, "issue-20"));
    scenario.agent_does_for(22, &opens_pr(22, "issue-20"));
    let result = scenario.run(&[&scenario.issue_url(20)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let traces: Vec<_> = scenario
        .entries(LOGS)
        .into_iter()
        .filter(|name| name.ends_with(".spans.jsonl"))
        .flat_map(|name| spans(&scenario.path(LOGS).join(name)))
        .collect();
    assert!(traces.len() >= 3);
    assert!(
        traces
            .iter()
            .all(|span| span["traceId"] == traces[0]["traceId"])
    );
    let parents: std::collections::HashSet<_> = traces
        .iter()
        .filter(|span| span["name"].as_str().unwrap().starts_with("invoke_agent "))
        .map(|span| span["parentSpanId"].as_str().unwrap())
        .collect();
    assert_eq!(parents.len(), 3);
}

#[test]
fn live_logs_survive_a_hard_kill_and_interruption_closes_open_spans() {
    for signal in ["INT", "KILL"] {
        let scenario = Scenario::new();
        scenario.agent_does(r#"
echo '{"type":"stream_event","event":{"type":"message_start","message":{"id":"open","model":"test-model","usage":{"input_tokens":8}}}}'
touch "$HOME/ready"
while ! test -f "$HOME/release"; do sleep 0.01; done
"#);
        let mut run = scenario.run_until(&[&scenario.issue_url(7)], &[], "home/ready");
        let path = scenario
            .entries(LOGS)
            .into_iter()
            .find(|name| name.ends_with(".logs.jsonl"))
            .unwrap();
        let path = scenario.path(LOGS).join(path);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let before = loop {
            let bytes = fs::read_to_string(&path).unwrap();
            if bytes.contains("message_start") {
                break bytes;
            }
            assert!(std::time::Instant::now() < deadline, "live log never grew");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let shown = scenario.run(&["show", "widgets#7", "--spans", "--json"]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        let tree: Value = serde_json::from_str(&shown.stdout).unwrap();
        let session = tree["spans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|span| span["kind"] == "sess")
            .unwrap();
        assert!(
            session["input"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("Closes #7")),
            "live Session must retain its prompt: {session}"
        );
        let model = tree["spans"]
            .as_array()
            .unwrap()
            .iter()
            .find(|span| span["kind"] == "model")
            .unwrap();
        assert_eq!(model["status"], "running");
        let live_id = model["id"].clone();
        if signal == "KILL" {
            run.kill();
            // A hard-killed owner cannot clean up its child. Let this exact
            // fixture's script finish before joining its inherited pipes.
            fs::write(scenario.path("home/release"), "").unwrap();
        } else {
            run.signal(signal);
        }
        let result = run.finish();
        assert_ne!(result.code, Some(0));
        assert!(fs::read_to_string(&path).unwrap().starts_with(&before));
        if signal == "INT" {
            let path = path.with_file_name(
                path.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace(".logs.jsonl", ".spans.jsonl"),
            );
            let spans = spans(&path);
            assert_eq!(spans.len(), 2, "{spans:?}");
            assert!(spans.iter().all(|span| span["status"]["code"] == 2));
            assert!(
                spans.iter().any(|span| span["spanId"] == live_id),
                "the live reader and recorder must share call identity"
            );
        }
    }
}

#[test]
fn a_live_call_keeps_its_id_through_completion_amendment_and_a_retained_custom_id() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        r#"
echo '{{"type":"assistant","message":{{"id":"m1","content":[{{"type":"tool_use","id":"deny","name":"Bash","input":{{"command":"git push"}}}}]}}}}'
touch "$HOME/started"
while ! test -f "$HOME/complete"; do sleep 0.01; done
echo '{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"deny","content":"opaque failure","is_error":true}}]}}}}'
touch "$HOME/completed"
while ! test -f "$HOME/amend"; do sleep 0.01; done
echo '{{"type":"result","subtype":"success","permission_denials":[{{"tool_use_id":"deny","tool_name":"Bash","tool_input":{{"command":"git push"}}}}]}}'
touch "$HOME/amended"
while ! test -f "$HOME/finish"; do sleep 0.01; done
{}"#,
        opens_pr(7, "main")
    ));
    let tool = || {
        let shown = scenario.run(&["show", "widgets#7", "--spans", "--json"]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        let tree: Value = serde_json::from_str(&shown.stdout).unwrap();
        let tools: Vec<_> = tree["spans"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|span| span["kind"] == "tool")
            .cloned()
            .collect();
        assert_eq!(tools.len(), 1, "{tree}");
        tools[0].clone()
    };
    let step = |release: &str, reached: &str| {
        fs::write(scenario.path(&format!("home/{release}")), "").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !scenario.path(&format!("home/{reached}")).exists() {
            assert!(std::time::Instant::now() < deadline, "{reached} never came");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    let run = scenario.run_until(&[&scenario.issue_url(7)], &[], "home/started");
    let live = tool();
    assert_eq!(live["status"], "running", "{live}");
    step("complete", "completed");
    let completed = tool();
    assert_eq!(completed["id"], live["id"]);
    assert_eq!(completed["start"], live["start"]);
    assert_eq!(completed["error"], "Failed", "{completed}");
    step("amend", "amended");
    // Until Session finish rewrites it, the flushed span outranks replay.
    let amended = tool();
    assert_eq!(amended["id"], live["id"]);
    assert_eq!(amended["error"], "Failed", "{amended}");
    fs::write(scenario.path("home/finish"), "").unwrap();
    let result = run.finish();
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let finished = tool();
    assert_eq!(finished["id"], live["id"]);
    assert_eq!(finished["start"], live["start"]);
    assert_eq!(finished["end"], completed["end"]);
    assert_eq!(finished["error"], "Denied", "{finished}");
    assert_eq!(finished["attributes"]["thirdshift.outcome"], "denied");

    // A retained custom ID, as a migration writes, replaces replay's identity.
    let path = scenario
        .entries(LOGS)
        .into_iter()
        .find(|name| name.ends_with(".spans.jsonl"))
        .map(|name| scenario.path(LOGS).join(name))
        .unwrap();
    let live_id = live["id"].as_str().unwrap();
    let retained = fs::read_to_string(&path).unwrap();
    assert!(retained.contains(live_id));
    fs::write(&path, retained.replace(live_id, "00000000000000ff")).unwrap();
    let custom = tool();
    assert_eq!(custom["id"], "00000000000000ff");
    assert_eq!(custom["error"], "Denied", "{custom}");
}
