mod support;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::trace::v1::Span;
use serde_json::Value;
use support::Scenario;

fn opens_pr(issue: u32, base: &str) -> String {
    format!(
        "echo work > work-{issue}.txt\ngit add .\ngit commit -qm 'Do work'\ngh pr create --base {base} --head issue-{issue} --title Work --body 'Closes #{issue}'\n"
    )
}

fn exported_spans(dir: &Path) -> Vec<Span> {
    fs::read_to_string(dir.join("spans.jsonl"))
        .unwrap()
        .lines()
        .flat_map(|line| {
            serde_json::from_str::<ExportTraceServiceRequest>(line)
                .unwrap()
                .resource_spans
                .into_iter()
                .flat_map(|resource| resource.scope_spans)
                .flat_map(|scope| scope.spans)
        })
        .collect()
}

fn nanos(time: &Value) -> u64 {
    chrono::DateTime::parse_from_rfc3339(time.as_str().unwrap())
        .unwrap()
        .timestamp_nanos_opt()
        .unwrap() as u64
}

fn assert_record_times(spans: &[Span], command: &Value) {
    let run = spans
        .iter()
        .find(|span| {
            span.name
                == format!(
                    "{} {}",
                    command["kind"].as_str().unwrap(),
                    command["name"].as_str().unwrap()
                )
        })
        .unwrap();
    assert_eq!(run.start_time_unix_nano, nanos(&command["started_at"]));
    assert_eq!(run.end_time_unix_nano, nanos(&command["ended_at"]));
    let stages = command["stages"].as_array().unwrap();
    for (index, stage) in stages.iter().enumerate() {
        let span = spans
            .iter()
            .find(|span| {
                span.parent_span_id == run.span_id
                    && span.name == stage["name"].as_str().unwrap()
                    && span.start_time_unix_nano == nanos(&stage["began_at"])
            })
            .unwrap();
        let end = stages
            .get(index + 1)
            .map(|next| &next["began_at"])
            .unwrap_or(&command["ended_at"]);
        assert_eq!(span.end_time_unix_nano, nanos(end));
    }
}

fn assert_one_trace(spans: &[Span]) {
    assert!(!spans.is_empty());
    let ids: HashSet<_> = spans.iter().map(|span| &span.span_id).collect();
    assert_eq!(ids.len(), spans.len(), "duplicate span IDs");
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.parent_span_id.is_empty())
            .count(),
        1
    );
    for span in spans {
        assert_eq!(span.trace_id, spans[0].trace_id);
        assert!(
            span.parent_span_id.is_empty() || ids.contains(&span.parent_span_id),
            "orphan: {span:?}"
        );
    }
}

#[test]
fn export_decodes_with_official_types_and_keeps_record_times_and_all_session_logs() {
    let scenario = Scenario::new();
    scenario.agent_does(&opens_pr(7, "main"));
    let run = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let command = &shown["command"];
    let dir = scenario.path("export");
    let result = scenario.run(&["logs", "widgets#7", "--export", dir.to_str().unwrap()]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let spans = exported_spans(&dir);
    assert_one_trace(&spans);
    assert_record_times(&spans, command);
    let root = spans
        .iter()
        .find(|span| span.parent_span_id.is_empty())
        .unwrap();
    assert_eq!(root.name, "Command");
    assert_eq!(root.start_time_unix_nano, nanos(&command["started_at"]));
    assert_eq!(root.end_time_unix_nano, nanos(&command["ended_at"]));

    let logs = fs::read_to_string(dir.join("logs.jsonl")).unwrap();
    let source = command["session_logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| {
            fs::read_to_string(Path::new(path.as_str().unwrap()).with_extension("logs.jsonl"))
                .unwrap()
        })
        .collect::<String>();
    assert!(!logs.is_empty());
    assert_eq!(logs, source);
    for line in logs.lines() {
        let request: ExportLogsServiceRequest = serde_json::from_str(line).unwrap();
        for log in request
            .resource_logs
            .into_iter()
            .flat_map(|resource| resource.scope_logs)
            .flat_map(|scope| scope.log_records)
        {
            assert_eq!(log.trace_id, root.trace_id);
            assert!(spans.iter().any(|span| span.span_id == log.span_id));
        }
    }
    assert_eq!(fs::read_dir(dir).unwrap().count(), 2);
}

#[test]
fn trace_only_sessions_export_once_even_when_the_record_names_the_old_log() {
    let scenario = Scenario::new();
    scenario.agent_does(&opens_pr(7, "main"));
    let run = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    for session in shown["command"]["session_logs"].as_array().unwrap() {
        fs::remove_file(session.as_str().unwrap()).unwrap();
    }
    let dir = scenario.path("export");
    let result = scenario.run(&["logs", "widgets#7", "--export", dir.to_str().unwrap()]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_one_trace(&exported_spans(&dir));
}

#[test]
fn a_pickup_commands_dispatched_spec_and_tickets_export_as_one_whole_trace() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[]), (22, &[21])]);
    scenario.issue_labelled(20, &["ready-for-agent"]);
    scenario.agent_does_for(21, &opens_pr(21, "issue-20"));
    scenario.agent_does_for(22, &opens_pr(22, "issue-20"));
    let run = scenario.run(&["pickup"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let status = scenario.run(&["status", "--all", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let outer = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["kind"] == "Pickup run")
        .unwrap();
    let shown = scenario.run(&["show", outer["command_log"].as_str().unwrap(), "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let command = &shown["command"];
    let dir = scenario.path("export");
    // A Ticket selector must still include its siblings and outer Command.
    let result = scenario.run(&["logs", "widgets#21", "--export", dir.to_str().unwrap()]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let spans = exported_spans(&dir);
    assert_one_trace(&spans);
    assert_record_times(&spans, command);
    let spec = &command["dispatched"][0];
    assert_record_times(&spans, spec);
    assert_eq!(spec["children"].as_array().unwrap().len(), 2);
    for child in spec["children"].as_array().unwrap() {
        assert_record_times(&spans, child);
    }
    let source_dir = Path::new(command["command_log"].as_str().unwrap())
        .parent()
        .unwrap();
    let source_lines: HashSet<_> = fs::read_dir(source_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".logs.jsonl"))
        .flat_map(|path| {
            fs::read_to_string(path)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    let logs = fs::read_to_string(dir.join("logs.jsonl")).unwrap();
    assert!(!source_lines.is_empty());
    assert_eq!(
        logs.lines().collect::<HashSet<_>>(),
        source_lines.iter().map(String::as_str).collect()
    );
    for line in logs.lines() {
        let request: ExportLogsServiceRequest = serde_json::from_str(line).unwrap();
        for log in request
            .resource_logs
            .into_iter()
            .flat_map(|resource| resource.scope_logs)
            .flat_map(|scope| scope.log_records)
        {
            assert_eq!(log.trace_id, spans[0].trace_id);
            assert!(spans.iter().any(|span| span.span_id == log.span_id));
        }
    }
}
