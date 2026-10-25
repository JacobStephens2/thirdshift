use super::*;
use crate::harness::interpretation::event::Usage;
use crate::otlp_jsonl::{AnyValue, Id};
use serde_json::json;

const SESSION: SpanId = Id([7; 8]);

fn calls() -> Calls {
    Calls::new(SESSION, false, [])
}

fn tool_started(id: &str, parent: Option<&str>) -> Event {
    Event::ToolCallStarted {
        id: id.into(),
        parent_id: parent.map(Into::into),
        tool: "Bash".into(),
        input: json!({"command": "cargo test"}),
    }
}

fn tool_ended(id: &str, outcome: Outcome, amended: bool) -> Event {
    Event::ToolCallEnded {
        id: id.into(),
        parent_id: None,
        tool: "Bash".into(),
        input: json!({"command": "cargo test"}),
        result: json!("output"),
        outcome,
        amended,
    }
}

fn agent_started(id: &str, parent: Option<&str>) -> Event {
    Event::SubagentStarted {
        id: id.into(),
        parent_id: parent.map(Into::into),
        description: Some("Review".into()),
    }
}

fn attribute<'a>(call: &'a Call, key: &str) -> Option<&'a AnyValue> {
    call.attributes
        .iter()
        .find(|attr| attr.key == key)
        .and_then(|attr| attr.value.as_ref())
}

fn text<'a>(call: &'a Call, key: &str) -> Option<&'a str> {
    match attribute(call, key) {
        Some(AnyValue::String(value)) => Some(value),
        _ => None,
    }
}

#[test]
fn identities_are_derived_from_the_session_and_causal_key() {
    let mut calls = calls();
    let call = calls.apply(&tool_started("a", None), Some(1)).unwrap().call;
    // SHA-256 of the length-prefixed Session span ID and `["tool",null,"a"]`,
    // computed outside this crate. Retained traces depend on this identity.
    assert_eq!(
        serde_json::to_value(call.span).unwrap(),
        json!("f50c70bcbd8badc8")
    );
    assert_eq!(call.parent_span, SESSION);
    assert_eq!(calls.locate("a"), Some(call.span));
    assert_eq!(calls.locate("missing"), None);
    assert!(
        calls
            .apply(
                &Event::WorkEnded {
                    id: "w".into(),
                    outcome: Outcome::Succeeded
                },
                Some(2)
            )
            .is_none()
    );
}

#[test]
fn parent_scoped_call_ids_stay_distinct_and_nest_under_their_parent() {
    let mut calls = calls();
    let top = calls.apply(&tool_started("a", None), Some(1)).unwrap().call;
    let agent = calls
        .apply(&agent_started("agent", None), Some(2))
        .unwrap()
        .call;
    let nested = calls
        .apply(&tool_started("a", Some("agent")), Some(3))
        .unwrap()
        .call;
    assert_ne!(top.span, nested.span);
    assert_eq!(top.parent_span, SESSION);
    assert_eq!(nested.parent_span, agent.span);
    assert_eq!(nested.parent_id.as_deref(), Some("agent"));
    assert_eq!(calls.calls().count(), 3);
    // Log association follows the latest call with that raw ID.
    assert_eq!(calls.locate("a"), Some(nested.span));
}

#[test]
fn a_parentless_subagent_completion_ends_its_earlier_start() {
    let mut calls = calls();
    let tool = calls
        .apply(&tool_started("task", None), Some(1))
        .unwrap()
        .call;
    let started = calls
        .apply(&agent_started("worker", Some("task")), Some(2))
        .unwrap()
        .call;
    let ended = calls
        .apply(
            &Event::SubagentEnded {
                id: "worker".into(),
                outcome: Outcome::Succeeded,
                result: json!("done"),
            },
            Some(5),
        )
        .unwrap();
    assert!(ended.first_completion);
    assert_eq!(ended.call.span, started.span);
    assert_eq!(ended.call.parent_span, tool.span);
    assert_eq!(ended.call.parent_id.as_deref(), Some("task"));
    assert_eq!((ended.call.started, ended.call.ended), (Some(2), Some(5)));
    assert_eq!(calls.calls().count(), 2);
}

#[test]
fn an_end_only_call_starts_and_ends_at_its_completion() {
    let mut calls = Calls::new(SESSION, true, []);
    let applied = calls
        .apply(
            &Event::ModelCallEnded {
                id: "m1".into(),
                parent_id: None,
                model: Some("model".into()),
                usage: Usage {
                    input: Some(10),
                    cache_read: Some(5),
                    ..Usage::default()
                },
                cost_usd: None,
                outcome: Outcome::Succeeded,
            },
            Some(4),
        )
        .unwrap();
    assert!(applied.first_completion);
    let call = applied.call;
    assert_eq!(call.kind, SpanKind::Model);
    assert_eq!((call.started, call.ended), (Some(4), Some(4)));
    assert!(call.completed);
    assert_eq!(call.outcome, Some(Outcome::Succeeded));
    // The Harness's cache-counting convention reaches the call's attributes.
    assert_eq!(
        attribute(&call, "gen_ai.usage.input_tokens"),
        Some(&AnyValue::Int(15))
    );
}

#[test]
fn repeated_completions_and_late_denial_refine_one_call_and_keep_first_times() {
    let mut calls = calls();
    calls.apply(&tool_started("deny", None), Some(1));
    let first = calls
        .apply(&tool_ended("deny", Outcome::Failed, false), Some(2))
        .unwrap();
    assert!(first.first_completion);
    let amended = calls
        .apply(&tool_ended("deny", Outcome::Denied, true), Some(9))
        .unwrap();
    assert!(!amended.first_completion);
    let call = amended.call;
    assert_eq!(call.span, first.call.span);
    assert_eq!((call.started, call.ended), (Some(1), Some(2)));
    assert_eq!(call.outcome, Some(Outcome::Denied));
    assert_eq!(text(&call, "thirdshift.outcome"), Some("denied"));
    assert_eq!(text(&call, "thirdshift.amended"), Some("true"));
    assert_eq!(text(&call, "gen_ai.tool.call.result"), Some("\"output\""));
    assert_eq!(calls.calls().collect::<Vec<_>>(), [&call]);
}

#[test]
fn a_late_success_keeps_the_last_failure_beside_the_latest_outcome() {
    let mut calls = calls();
    calls.apply(&tool_ended("t", Outcome::Failed, false), Some(2));
    let call = calls
        .apply(&tool_ended("t", Outcome::Succeeded, false), Some(3))
        .unwrap()
        .call;
    assert_eq!(call.outcome, Some(Outcome::Succeeded));
    assert_eq!(call.failure, Some(Outcome::Failed));
}

#[test]
fn unknown_times_stay_unknown_and_the_first_known_completion_time_wins() {
    let mut calls = calls();
    calls.apply(&tool_started("t", None), None);
    calls.apply(&tool_ended("t", Outcome::Succeeded, false), None);
    calls.apply(&tool_ended("t", Outcome::Succeeded, false), Some(7));
    let call = calls
        .apply(&tool_ended("t", Outcome::Succeeded, false), Some(3))
        .unwrap()
        .call;
    // A later completion, even one clocked earlier, keeps the first known end.
    assert_eq!((call.started, call.ended), (None, Some(7)));
}

#[test]
fn retained_identities_override_derived_ones_and_parent_links() {
    let agent = Id([1; 8]);
    let tool = Id([2; 8]);
    let mut calls = Calls::new(
        SESSION,
        false,
        [
            Retained {
                kind: SpanKind::Agent,
                id: "agent",
                parent: None,
                span: agent,
            },
            Retained {
                kind: SpanKind::Tool,
                id: "a",
                parent: Some("agent"),
                span: tool,
            },
        ],
    );
    let top = calls.apply(&tool_started("a", None), Some(1)).unwrap().call;
    calls.apply(&agent_started("agent", None), Some(2));
    let nested = calls
        .apply(&tool_started("a", Some("agent")), Some(3))
        .unwrap()
        .call;
    assert!(![agent, tool].contains(&top.span));
    assert_eq!((nested.span, nested.parent_span), (tool, agent));
    let ended = calls
        .apply(
            &Event::SubagentEnded {
                id: "agent".into(),
                outcome: Outcome::Succeeded,
                result: json!(null),
            },
            Some(4),
        )
        .unwrap()
        .call;
    assert_eq!(ended.span, agent);
    assert_eq!(calls.locate("a"), Some(tool));
}

#[test]
fn closing_completes_only_unfinished_calls_without_an_outcome() {
    let mut calls = calls();
    calls.apply(&tool_started("open", None), Some(1));
    calls.apply(&tool_ended("done", Outcome::Succeeded, false), Some(2));
    let closed = calls.close(10);
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].id, "open");
    assert_eq!((closed[0].started, closed[0].ended), (Some(1), Some(10)));
    assert!(closed[0].completed);
    assert_eq!(closed[0].outcome, None);
    let done = calls.calls().find(|call| call.id == "done").unwrap();
    assert_eq!(done.ended, Some(2));
    assert!(calls.close(11).is_empty());
}

#[test]
fn a_tool_keeps_its_first_observed_name_while_attributes_follow_the_latest() {
    let mut calls = calls();
    calls.apply(&tool_started("t", None), Some(1));
    let mut renamed = tool_ended("t", Outcome::Succeeded, false);
    if let Event::ToolCallEnded { tool, .. } = &mut renamed {
        *tool = "read_file".into();
    }
    let call = calls.apply(&renamed, Some(2)).unwrap().call;
    assert_eq!(call.tool.as_deref(), Some("Bash"));
    assert_eq!(text(&call, "gen_ai.tool.name"), Some("read_file"));
}

#[test]
fn an_end_only_subagent_completion_takes_its_retained_identity() {
    let retained = Id([3; 8]);
    let mut calls = Calls::new(
        SESSION,
        false,
        [Retained {
            kind: SpanKind::Agent,
            id: "solo",
            parent: None,
            span: retained,
        }],
    );
    let call = calls
        .apply(
            &Event::SubagentEnded {
                id: "solo".into(),
                outcome: Outcome::Succeeded,
                result: json!(null),
            },
            Some(4),
        )
        .unwrap()
        .call;
    assert_eq!((call.span, call.parent_span), (retained, SESSION));
}
