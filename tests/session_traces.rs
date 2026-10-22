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
        assert!(folder.join(format!("{stem}.jsonl")).exists());
        assert!(folder.join(format!("{stem}.models.json")).exists());
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
        }
    }
}
