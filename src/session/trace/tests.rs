use super::*;
use crate::harness::interpretation::event::Outcome;
use serde_json::json;

fn spans(path: &Path) -> Vec<Span> {
    Reader::<_, Spans>::new(std::fs::read(path).unwrap().as_slice())
        .flat_map(|request| request.unwrap().resource_spans)
        .flat_map(|resource| resource.scope_spans)
        .flat_map(|scope| scope.spans)
        .collect()
}

fn tool_ended(id: &str, outcome: Outcome) -> Event {
    Event::ToolCallEnded {
        id: id.into(),
        parent_id: None,
        tool: "Bash".into(),
        input: json!({}),
        result: json!(""),
        outcome,
        amended: false,
    }
}

#[test]
fn recording_clamps_backwards_ends_projects_the_latest_outcome_and_closes_open_calls() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("7-20261003T120000-0400-implement.jsonl");
    let mut trace = Trace::create(
        &log,
        "implement",
        &Choice::default(),
        "prompt",
        Context::of_session(&log),
        &std::process::Command::new("true"),
        None,
    )
    .unwrap();
    let started = |id: &str| Event::ToolCallStarted {
        id: id.into(),
        parent_id: None,
        tool: "Bash".into(),
        input: json!({}),
    };
    trace.record_at(b"{}", &[started("t")], 2_000).unwrap();
    trace.record_at(b"{}", &[started("open")], 500).unwrap();
    // This completion's clock reads before its start.
    trace
        .record_at(b"{}", &[tool_ended("t", Outcome::Failed)], 1_000)
        .unwrap();
    let path = log.with_extension("spans.jsonl");
    let flushed = spans(&path);
    assert_eq!(flushed.len(), 1);
    assert_eq!(
        (
            flushed[0].start_time_unix_nano,
            flushed[0].end_time_unix_nano
        ),
        (2_000, 2_000)
    );
    assert_eq!(flushed[0].status.as_ref().unwrap().code, 2);
    trace
        .record_at(b"{}", &[tool_ended("t", Outcome::Succeeded)], 3_000)
        .unwrap();
    assert_eq!(
        spans(&path).len(),
        1,
        "a repeated completion is not streamed"
    );
    trace.finish_at(Some("interrupted"), 400).unwrap();
    let retained = spans(&path);
    assert_eq!(retained.len(), 3);
    let tool = retained
        .iter()
        .find(|span| span.span_id == flushed[0].span_id)
        .unwrap();
    assert_eq!(tool.end_time_unix_nano, 2_000);
    assert_eq!(
        tool.status,
        Some(Status {
            code: 1,
            message: "Succeeded".into()
        })
    );
    let open = retained
        .iter()
        .find(|span| span.start_time_unix_nano == 500)
        .unwrap();
    assert_eq!(open.name, "execute_tool Bash");
    assert_eq!(open.end_time_unix_nano, 500);
    assert_eq!(
        open.status,
        Some(Status {
            code: 2,
            message: "interrupted".into()
        })
    );
}
