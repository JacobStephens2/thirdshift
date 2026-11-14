use std::fs;
use std::io::Cursor;

use serde_json::json;

use super::import;
use crate::logs::trace::Context;
use crate::otlp_jsonl::{AnyValue, Reader, Spans};
use crate::session::usage;
use crate::trial::accounting::{Prices, Rate};

#[test]
fn sdk_sessions_retain_roles_clocks_and_cache_inclusive_usage_for_frozen_prices() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("output");
    let retained = root.path().join("session");
    fs::create_dir(&output).unwrap();
    fs::write(
        output.join("review.jsonl"),
        "The hidden fix was described here.\n",
    )
    .unwrap();
    fs::write(
        output.join("sessions.json"),
        serde_json::to_vec(&json!([{
            "role":"review", "transcript_file":"review.jsonl",
            "started_at":"2026-11-14T15:00:00Z", "ended_at":"2026-11-14T15:00:02Z",
            "models":{"sdk-model":{"uncached_input_tokens":100,"cache_read_tokens":200,
                "cache_write_tokens":300,"output_tokens":400}},
            "cost_usd":9999
        }]))
        .unwrap(),
    )
    .unwrap();
    let context = Context::of_session(root.path());
    let imported = import(&output, &retained, root.path(), &context).unwrap();
    assert_eq!(imported.sessions, 1);
    assert_eq!(imported.harness_sessions, 0);
    let spans = retained.join("custom-import-1.spans.jsonl");
    let summary = usage::from_spans(Cursor::new(fs::read(&spans).unwrap())).unwrap();
    assert_eq!(summary.models.len(), 1);
    assert_eq!(summary.models[0].model.as_deref(), Some("sdk-model"));
    assert_eq!(summary.models[0].measurement.input_tokens, Some(600));
    assert_eq!(summary.models[0].measurement.output_tokens, Some(400));
    assert_eq!(summary.cost_usd, None);
    let prices = Prices {
        date: "2026-11-14".into(),
        source: "frozen test prices".into(),
        models: [(
            "sdk-model".into(),
            Rate {
                input: 1.0,
                output: 2.0,
                cache_read: 0.1,
                cache_write: 1.25,
            },
        )]
        .into_iter()
        .collect(),
    };
    assert!((prices.cost(&summary).unwrap() - 0.001295).abs() < 1e-12);
    let session = Reader::<_, Spans>::new(Cursor::new(fs::read(spans).unwrap()))
        .flat_map(|request| request.unwrap().resource_spans)
        .flat_map(|resource| resource.scope_spans)
        .flat_map(|scope| scope.spans)
        .find(|span| span.name.starts_with("invoke_agent "))
        .unwrap();
    assert_eq!(session.start_time_unix_nano, 1_794_668_400_000_000_000);
    assert_eq!(
        session.end_time_unix_nano - session.start_time_unix_nano,
        2_000_000_000
    );
    assert!(
        session
            .attributes
            .iter()
            .any(|attr| attr.key == "thirdshift.trial.role"
                && attr.value == Some(AnyValue::String("review".into())))
    );
    let logs = fs::read_to_string(retained.join("custom-import-1.logs.jsonl")).unwrap();
    assert!(logs.contains("The hidden fix was described here."));
}

#[test]
fn raw_harness_sessions_retain_normalized_web_search_and_quota_evidence() {
    for (harness, lines) in [
        (
            "claude",
            vec![
                json!({"type":"assistant","message":{"id":"msg1","model":"claude-test",
                "content":[{"type":"tool_use","id":"search","name":"WebSearch","input":{"query":"hidden fix"}}],
                "usage":{"input_tokens":100,"cache_read_input_tokens":200,"cache_creation_input_tokens":300,"output_tokens":400}}}),
                json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"seven_day","utilization":0.9}}),
                json!({"type":"result","subtype":"success","result":"done","total_cost_usd":9999,
                "modelUsage":{"claude-test":{"inputTokens":100,"cacheReadInputTokens":200,
                    "cacheCreationInputTokens":300,"outputTokens":400,"costUSD":9999}}}),
            ],
        ),
        (
            "codex",
            vec![
                json!({"type":"thread.started","thread_id":"trial"}),
                json!({"type":"item.completed","item":{"id":"search","type":"web_search","query":"hidden fix"}}),
                json!({"type":"event_msg","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":90}}}}),
                json!({"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":20,"output_tokens":50}}),
            ],
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("output");
        let retained = root.path().join("session");
        fs::create_dir(&output).unwrap();
        fs::write(
            output.join("stream.jsonl"),
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .unwrap();
        fs::write(
            output.join("sessions.json"),
            serde_json::to_vec(&json!([{
                "role":"implement", "harness":harness, "stream_file":"stream.jsonl",
                "started_at":"2026-11-14T15:00:00Z", "ended_at":"2026-11-14T15:00:02Z"
            }]))
            .unwrap(),
        )
        .unwrap();
        let context = Context::of_session(root.path());
        let imported = import(&output, &retained, root.path(), &context).unwrap();
        assert_eq!(imported.sessions, 1, "{harness}");
        assert_eq!(imported.harness_sessions, 1, "{harness}");
        assert!(imported.web_search, "{harness}");
        let limit = imported.usage_limit.unwrap();
        assert_eq!(limit.harness.as_deref(), Some(harness));
        assert_eq!(limit.utilization, Some(0.9));
        let spans = fs::read(retained.join("custom-import-1.spans.jsonl")).unwrap();
        let summary = usage::from_spans(Cursor::new(&spans)).unwrap();
        assert_eq!(summary.cost_usd, None);
        if harness == "claude" {
            assert_eq!(summary.models[0].model.as_deref(), Some("claude-test"));
            assert_eq!(summary.models[0].measurement.input_tokens, Some(600));
        }
        assert!(String::from_utf8(spans).unwrap().contains("execute_tool"));
    }
}

#[test]
fn sdk_transcripts_normalize_actual_web_search_tool_calls() {
    for (transcript, expected) in [
        (
            json!({"type":"tool_call_started","id":"search","parent_id":null,
            "tool":"WebSearch","input":{"query":"hidden fix"}}),
            true,
        ),
        (
            json!({"role":"assistant","content":[{"type":"tool_use","id":"search",
            "name":"WebSearch","input":{"query":"hidden fix"}}]}),
            true,
        ),
        (
            json!({"output":[{"type":"web_search_call","id":"search",
            "action":{"query":"hidden fix"}}]}),
            true,
        ),
        (
            json!({"role":"assistant","tool_calls":[{"id":"search","type":"function",
            "function":{"name":"web_search","arguments":"{\"query\":\"hidden fix\"}"}}]}),
            true,
        ),
        (
            json!({"role":"assistant","content":"The WebSearch tool must remain disabled."}),
            false,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("output");
        let retained = root.path().join("session");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("transcript.jsonl"), format!("{transcript}\n")).unwrap();
        fs::write(
            output.join("sessions.json"),
            serde_json::to_vec(&json!([sdk_entry("transcript.jsonl")])).unwrap(),
        )
        .unwrap();
        let imported = import(
            &output,
            &retained,
            root.path(),
            &Context::of_session(root.path()),
        )
        .unwrap();
        assert_eq!(imported.web_search, expected, "{transcript}");
        let spans = fs::read_to_string(retained.join("custom-import-1.spans.jsonl")).unwrap();
        assert_eq!(spans.contains("execute_tool"), expected, "{transcript}");
    }
}

#[test]
fn session_import_rejects_unconfined_or_invalid_evidence() {
    use std::os::unix::fs::symlink;

    for invalid in [
        "parent",
        "absolute",
        "nested",
        "backslash",
        "symlink",
        "manifest-symlink",
        "directory-symlink",
        "reversed-clock",
        "duplicate",
    ] {
        let root = tempfile::tempdir().unwrap();
        let output = root.path().join("output");
        let retained = root.path().join("retained");
        fs::create_dir(&output).unwrap();
        fs::create_dir(output.join("nested")).unwrap();
        let outside = root.path().join("outside.jsonl");
        fs::write(&outside, "outside Session evidence").unwrap();
        fs::write(
            output.join("transcript.jsonl"),
            "legitimate Session evidence",
        )
        .unwrap();
        fs::write(
            output.join("nested/transcript.jsonl"),
            "nested Session evidence",
        )
        .unwrap();
        fs::write(
            output.join("back\\slash.jsonl"),
            "backslash Session evidence",
        )
        .unwrap();
        symlink(&outside, output.join("link.jsonl")).unwrap();
        let mut entry = sdk_entry("transcript.jsonl");
        entry["transcript_file"] = json!(match invalid {
            "parent" => "../outside.jsonl",
            "absolute" => outside.to_str().unwrap(),
            "nested" => "nested/transcript.jsonl",
            "backslash" => "back\\slash.jsonl",
            "symlink" => "link.jsonl",
            _ => "transcript.jsonl",
        });
        if invalid == "reversed-clock" {
            entry["ended_at"] = json!("2026-11-14T14:59:59Z");
        }
        let entries = if invalid == "duplicate" {
            vec![entry.clone(), entry]
        } else {
            vec![entry]
        };
        let manifest = serde_json::to_vec(&entries).unwrap();
        if invalid == "manifest-symlink" {
            let external_manifest = root.path().join("manifest.json");
            fs::write(&external_manifest, &manifest).unwrap();
            symlink(external_manifest, output.join("sessions.json")).unwrap();
        } else {
            fs::write(output.join("sessions.json"), manifest).unwrap();
        }
        let selected = if invalid == "directory-symlink" {
            let alias = root.path().join("alias");
            symlink(&output, &alias).unwrap();
            alias
        } else {
            output
        };
        assert!(
            import(
                &selected,
                &retained,
                root.path(),
                &Context::of_session(root.path())
            )
            .is_err(),
            "{invalid}"
        );
    }
}

fn sdk_entry(file: &str) -> serde_json::Value {
    json!({
        "role":"review", "transcript_file":file,
        "started_at":"2026-11-14T15:00:00Z", "ended_at":"2026-11-14T15:00:02Z",
        "models":{"sdk-model":{"uncached_input_tokens":100,"cache_read_tokens":200,
            "cache_write_tokens":300,"output_tokens":400}}
    })
}
