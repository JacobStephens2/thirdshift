use super::{Measurement, Summary, from_events, from_spans};
use crate::harness::interpretation::event::Event;
use crate::harness::{Choice, Harness};
use crate::logs::trace::Context;
use crate::session::trace::Trace;
use serde_json::{Value, json};

fn paired(harness: Harness, events: Value) -> Summary {
    let events: Vec<Event> = serde_json::from_value(events).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("session.jsonl");
    let choice = Choice {
        harness,
        model: Some("requested-only".into()),
        ..Choice::default()
    };
    let mut trace = Trace::create(
        &log,
        "implement",
        &choice,
        "prompt",
        Context::of_session(&log),
        &std::process::Command::new("true"),
        None,
    )
    .unwrap();
    for event in &events {
        trace.record(b"{}", std::slice::from_ref(event)).unwrap();
    }
    let mut spans = std::fs::read(log.with_extension("spans.jsonl")).unwrap();
    trace.finish(None).unwrap();
    let final_spans = std::fs::read(log.with_extension("spans.jsonl")).unwrap();
    // Historical append-only evidence can contain the first completion, a
    // later amendment and repeated final exports. Only the latest ID counts.
    spans.extend_from_slice(&final_spans);
    spans.extend_from_slice(&final_spans);
    let mut retained = from_spans(spans.as_slice()).unwrap();
    let mut normalized = from_events(&events, harness.input_excludes_cache());
    // Consumer ordering is presentation; measurements are grouped by identity.
    retained.models.sort_by(|a, b| a.model.cmp(&b.model));
    normalized.models.sort_by(|a, b| a.model.cmp(&b.model));
    assert_eq!(normalized, retained);
    normalized
}

#[test]
fn costs_choose_session_then_each_models_total_without_erasing_omitted_costs() {
    for session_cost in [None, Some(0.0), Some(0.9)] {
        let summary = paired(
            Harness::Claude,
            json!([
                {"type":"model_call_ended", "id":"a1", "parent_id":null, "model":"a",
                 "usage":{"input":1}, "cost_usd":0.1, "outcome":"succeeded"},
                {"type":"model_call_ended", "id":"a2", "parent_id":null, "model":"a",
                 "usage":{"output":2}, "cost_usd":0.2, "outcome":"succeeded"},
                {"type":"model_call_ended", "id":"b", "parent_id":null, "model":"b",
                 "usage":{"input":3}, "cost_usd":0.3, "outcome":"succeeded"},
                {"type":"usage", "scope":"model", "model":"a", "usage":{"output":0}, "cost_usd":0.4},
                {"type":"usage", "scope":"model", "model":"b", "usage":{"output":7}, "cost_usd":null},
                {"type":"usage", "scope":"session", "model":null, "usage":{}, "cost_usd":session_cost}
            ]),
        );
        assert_eq!(measurement(&summary, Some("a")).cost_usd, Some(0.4));
        assert_eq!(measurement(&summary, Some("a")).output_tokens, Some(0));
        assert_eq!(measurement(&summary, Some("b")).cost_usd, Some(0.3));
        assert_eq!(summary.cost_usd, Some(session_cost.unwrap_or(0.7)));
    }
}

#[test]
fn scoped_calls_and_sparse_repeated_completions_keep_the_merged_facts_once() {
    let summary = paired(
        Harness::Claude,
        json!([
            {"type":"model_call_started", "id":"same", "parent_id":"parent-a", "model":"observed"},
            {"type":"model_call_ended", "id":"same", "parent_id":"parent-a", "model":null,
             "usage":{"input":11,"output":2,"cache_read":5}, "cost_usd":0.1, "outcome":"succeeded"},
            {"type":"model_call_ended", "id":"same", "parent_id":"parent-a", "model":null,
             "usage":{"output":4}, "cost_usd":null, "outcome":"succeeded"},
            {"type":"model_call_ended", "id":"same", "parent_id":"parent-a", "model":null,
             "usage":{"output":4}, "cost_usd":null, "outcome":"succeeded"},
            {"type":"model_call_ended", "id":"same", "parent_id":"parent-b", "model":"observed",
             "usage":{"input":3,"output":1}, "cost_usd":null, "outcome":"succeeded"}
        ]),
    );
    assert_eq!(
        measurement(&summary, Some("observed")),
        Measurement {
            input_tokens: Some(19),
            output_tokens: Some(5),
            cache_read_tokens: Some(5),
            cost_usd: Some(0.1),
            ..Measurement::default()
        }
    );
    assert_eq!(summary.models.len(), 1);
    assert_eq!(summary.cost_usd, Some(0.1));
}

#[test]
fn overlapping_turns_and_ambiguous_session_tokens_do_not_add_to_model_calls() {
    let summary = paired(
        Harness::Codex,
        json!([
            {"type":"model_call_ended", "id":"a", "parent_id":null, "model":"a",
             "usage":{"input":10,"cache_read":2}, "cost_usd":0.1, "outcome":"succeeded"},
            {"type":"model_call_ended", "id":"b", "parent_id":null, "model":"b",
             "usage":{"input":20,"output":3}, "cost_usd":0.2, "outcome":"succeeded"},
            {"type":"turn_ended", "id":"turn", "usage":{"input":999,"output":999}, "outcome":"succeeded"},
            {"type":"usage", "scope":"session", "model":"a", "usage":{"input":100,"output":200}, "cost_usd":0.9}
        ]),
    );
    assert_eq!(summary.models.len(), 2);
    assert_eq!(measurement(&summary, Some("a")).input_tokens, Some(10));
    assert_eq!(measurement(&summary, Some("a")).output_tokens, None);
    assert_eq!(measurement(&summary, Some("b")).input_tokens, Some(20));
    assert_eq!(summary.tokens(), Some(33));
    assert_eq!(summary.cost_usd, Some(0.9));
}

#[test]
fn cost_only_evidence_keeps_the_unknown_model_turn_fallback() {
    let summary = paired(
        Harness::Codex,
        json!([
            {"type":"model_call_ended", "id":"a", "parent_id":null, "model":"a",
             "usage":{}, "cost_usd":0.2, "outcome":"succeeded"},
            {"type":"usage", "scope":"model", "model":"a", "usage":{}, "cost_usd":0.3},
            {"type":"usage", "scope":"session", "model":null, "usage":{}, "cost_usd":1.0},
            {"type":"turn_ended", "id":"one", "usage":{"input":10,"output":2}, "outcome":"succeeded"},
            {"type":"turn_ended", "id":"one", "usage":{"output":4}, "outcome":"succeeded"},
            {"type":"turn_ended", "id":"two", "usage":{"input":5,"output":1}, "outcome":"succeeded"}
        ]),
    );
    assert_eq!(
        measurement(&summary, None),
        Measurement {
            input_tokens: Some(15),
            output_tokens: Some(5),
            ..Measurement::default()
        }
    );
    assert_eq!(measurement(&summary, Some("a")).input_tokens, None);
    assert_eq!(measurement(&summary, Some("a")).cost_usd, Some(0.3));
    assert_eq!(summary.cost_usd, Some(1.0));
}

#[test]
fn session_totals_refine_one_matching_model_and_keep_unattributed_totals() {
    for model in [None, Some("observed")] {
        let summary = paired(
            Harness::Claude,
            json!([
                {"type":"model_call_ended", "id":"m", "parent_id":null, "model":model,
                 "usage":{"output":2}, "cost_usd":0.1, "outcome":"succeeded"},
                {"type":"usage", "scope":"session", "model":model,
                 "usage":{"input":11,"output":3,"cache_read":5}, "cost_usd":null}
            ]),
        );
        assert_eq!(
            measurement(&summary, model),
            Measurement {
                input_tokens: Some(16),
                output_tokens: Some(3),
                cache_read_tokens: Some(5),
                cost_usd: Some(0.1),
                ..Measurement::default()
            }
        );
        let total_only = paired(
            Harness::Codex,
            json!([
                {"type":"usage", "scope":"session", "model":model,
                 "usage":{"input":16,"output":3,"cache_read":5}, "cost_usd":null},
                {"type":"turn_ended", "id":"t", "usage":{"input":999}, "outcome":"succeeded"}
            ]),
        );
        assert_eq!(
            measurement(&total_only, model),
            Measurement {
                input_tokens: Some(16),
                output_tokens: Some(3),
                cache_read_tokens: Some(5),
                ..Measurement::default()
            }
        );
        assert_eq!(total_only.models.len(), 1);
        assert_eq!(total_only.cost_usd, None);
    }
}

#[test]
fn latest_model_snapshot_refines_calls_without_summing_earlier_totals() {
    let summary = paired(
        Harness::Claude,
        json!([
            {"type":"model_call_ended", "id":"m", "parent_id":null, "model":"observed",
             "usage":{"input":11,"output":2,"cache_read":5,"cache_write":2}, "cost_usd":0.1, "outcome":"succeeded"},
            {"type":"usage", "scope":"model", "model":"observed",
             "usage":{"input":100,"output":100,"cache_read":20}, "cost_usd":9.0},
            {"type":"usage", "scope":"model", "model":"observed", "usage":{"output":3}, "cost_usd":null}
        ]),
    );
    assert_eq!(
        measurement(&summary, Some("observed")),
        Measurement {
            input_tokens: Some(18),
            output_tokens: Some(3),
            cache_read_tokens: Some(5),
            cache_write_tokens: Some(2),
            cost_usd: Some(0.1)
        }
    );
    assert_eq!(summary.cost_usd, Some(0.1));
}

#[test]
fn partial_cache_totals_preserve_uncached_input_and_reported_zeroes() {
    for (harness, input) in [
        (Harness::Claude, 14),
        (Harness::Grok, 14),
        (Harness::Codex, 11),
    ] {
        let summary = paired(
            harness,
            json!([
                {"type":"model_call_ended", "id":"m", "parent_id":null, "model":"observed",
                 "usage":{"input":11,"output":2,"cache_read":5,"cache_write":2}, "cost_usd":0.1, "outcome":"succeeded"},
                {"type":"usage", "scope":"model", "model":"observed",
                 "usage":{"output":0,"cache_read":3,"cache_write":0}, "cost_usd":0.0}
            ]),
        );
        assert_eq!(
            measurement(&summary, Some("observed")),
            Measurement {
                input_tokens: Some(input),
                output_tokens: Some(0),
                cache_read_tokens: Some(3),
                cache_write_tokens: Some(0),
                cost_usd: Some(0.0)
            }
        );
        assert_eq!(summary.cost_usd, Some(0.0));
    }
}

#[test]
fn absent_measurements_stay_absent_and_unknown_models_keep_reported_zeroes() {
    assert_eq!(paired(Harness::Claude, json!([])), Summary::default());
    let summary = paired(
        Harness::Claude,
        json!([
            {"type":"model_call_ended", "id":"m", "parent_id":null, "model":null,
             "usage":{"input":0,"output":0,"cache_read":0}, "cost_usd":null, "outcome":"succeeded"}
        ]),
    );
    assert_eq!(summary.models.len(), 1);
    assert_eq!(
        measurement(&summary, None),
        Measurement {
            input_tokens: Some(0),
            output_tokens: Some(0),
            cache_read_tokens: Some(0),
            ..Measurement::default()
        }
    );
    assert_eq!(summary.tokens(), Some(0));
    assert_eq!(summary.cost_usd, None);
}

#[test]
fn separate_session_addition_saturates_tokens_after_resolving_totals() {
    let session = paired(
        Harness::Codex,
        json!([
            {"type":"usage", "scope":"model", "model":"observed",
             "usage":{"input":u64::MAX,"output":1}, "cost_usd":0.25}
        ]),
    );
    let mut combined = Summary::default();
    combined.add(&session);
    combined.add(&session);
    assert_eq!(
        measurement(&combined, Some("observed")),
        Measurement {
            input_tokens: Some(u64::MAX),
            output_tokens: Some(2),
            cost_usd: Some(0.5),
            ..Measurement::default()
        }
    );
    assert_eq!(combined.tokens(), Some(u64::MAX));
    assert_eq!(combined.cost_usd, Some(0.5));
}

fn measurement(summary: &Summary, model: Option<&str>) -> Measurement {
    summary
        .models
        .iter()
        .find(|m| m.model.as_deref() == model)
        .unwrap()
        .measurement
}

#[test]
fn partial_model_totals_refine_only_reported_fields_and_normalize_cache_once() {
    for harness in [Harness::Claude, Harness::Grok] {
        for (total, input) in [
            (json!({"output":3}), 16),
            (json!({"input":20,"output":3}), 25),
        ] {
            let summary = paired(
                harness,
                json!([
                    {"type":"model_call_ended", "id":"m", "parent_id":null, "model":"observed",
                     "usage":{"input":11,"output":2,"cache_read":5}, "cost_usd":0.1, "outcome":"succeeded"},
                    {"type":"usage", "scope":"model", "model":"observed", "usage":total, "cost_usd":null}
                ]),
            );
            assert_eq!(
                measurement(&summary, Some("observed")),
                Measurement {
                    input_tokens: Some(input),
                    output_tokens: Some(3),
                    cache_read_tokens: Some(5),
                    cost_usd: Some(0.1),
                    ..Measurement::default()
                }
            );
            assert_eq!(summary.cost_usd, Some(0.1));
        }
    }
}
