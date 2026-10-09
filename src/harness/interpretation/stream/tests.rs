use crate::harness::{
    Harness,
    interpretation_tests::{finish, lines, stream},
};
use serde_json::json;
use std::os::unix::process::ExitStatusExt;

#[test]
fn claude_terminal_failure_keeps_the_captured_spend_limit_message_for_either_exit() {
    const MESSAGE: &str = "You've hit your monthly spend limit · raise it at https://claude.ai/settings/usage?from=cc_cli_limit_message · your weekly limit resets Oct 10, 2am (UTC)";
    for exit in [0, 1] {
        let mut interpretation = stream(Harness::Claude, "");
        interpretation.condense(r#"{"type":"result","subtype":"success","result":"Earlier work completed.","num_turns":17,"total_cost_usd":1.25}"#);
        interpretation.condense(&json!({"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":MESSAGE}).to_string());
        let completion = interpretation.finish(Ok(std::process::ExitStatus::from_raw(exit << 8)));
        let prefix = if exit == 0 {
            "claude's turn failed"
        } else {
            "claude exited 1"
        };
        let cause = completion.outcome.unwrap_err().to_string();
        assert!(
            cause.starts_with(&format!("{prefix}: {MESSAGE}")),
            "{cause}"
        );
        assert!(cause.contains("api_error_status: 429"), "{cause}");
        assert_eq!(
            completion.report.unwrap().summary.as_deref(),
            Some("17 turns, $1.25")
        );
    }
}

#[test]
fn claude_terminal_diagnostics_prefer_meaningful_errors_and_tolerate_malformed_fields() {
    for (errors, result, expected) in [
        (
            json!([null, "", " \n", 7, "first", "second"]),
            json!("result text"),
            "claude's turn failed: first\nsecond",
        ),
        (
            json!([null, 7, " "]),
            json!("result text"),
            "claude's turn failed: result text",
        ),
        (
            json!("malformed"),
            json!("result text"),
            "claude's turn failed: result text",
        ),
        (json!([""]), json!(" \n"), "claude's turn failed"),
        (
            json!({"message":"malformed"}),
            json!(42),
            "claude's turn failed",
        ),
    ] {
        let (completion, _) = lines(
            Harness::Claude,
            &[json!({"type":"result","subtype":"error","errors":errors,"result":result})],
        );
        assert_eq!(completion.outcome.unwrap_err().to_string(), expected);
    }
}

#[test]
fn claude_failed_results_keep_root_api_fallback_and_supplied_limit_facts() {
    let api_error = json!({
        "type":"assistant", "parent_tool_use_id":null, "is_api_error_message":true,
        "api_error":"usage_limit_reached", "api_error_status":429,
        "api_error_params":{"rate_limit_info":{
            "status":"rejected", "rateLimitType":"seven_day", "resetsAt":1791597600,
            "overageStatus":"rejected", "overageDisabledReason":"org_level_disabled_until",
            "unrelated":"private raw event content"
        }},
        "message":{"content":[{"type":"text","text":"Provider's limit message."}]}
    });
    for (terminal, message) in [
        (
            json!({"type":"result","subtype":"success","is_error":true,"api_error_status":429,"result":"Monthly spend limit; weekly reset Oct 10."}),
            "Monthly spend limit; weekly reset Oct 10.",
        ),
        (
            json!({"type":"result","subtype":"error","errors":["terminal error"],"result":"ignored"}),
            "terminal error",
        ),
        (
            json!({"type":"result","subtype":"error","errors":[null, " "],"result":""}),
            "Provider's limit message.",
        ),
    ] {
        let (completion, _) = lines(Harness::Claude, &[api_error.clone(), terminal]);
        let cause = completion.outcome.unwrap_err().to_string();
        assert!(
            cause.starts_with(&format!("claude's turn failed: {message}")),
            "{cause}"
        );
        for fact in [
            "api_error: usage_limit_reached",
            "api_error_status: 429",
            "rateLimitType: seven_day",
            "resetsAt: 1791597600",
            "overageStatus: rejected",
            "overageDisabledReason: org_level_disabled_until",
        ] {
            assert!(cause.contains(fact), "missing {fact:?}: {cause}");
        }
        assert!(!cause.contains("private raw event content"), "{cause}");
        assert!(!cause.contains("overageResetsAt"), "{cause}");
    }
}

#[test]
fn claude_child_errors_cannot_replace_the_successful_parent_result() {
    for exit in [0, 1] {
        let mut interpretation = stream(Harness::Claude, "");
        for event in [
            json!({"type":"result","subtype":"success","result":"Parent done.","num_turns":2,"total_cost_usd":0.3}),
            json!({"type":"assistant","parent_tool_use_id":"child","is_api_error_message":true,"api_error":"usage_limit_reached","message":{"content":[{"type":"text","text":"child failure"}]}}),
            json!({"type":"result","parent_tool_use_id":"child","subtype":"error","errors":["child failure"],"result":"child result","num_turns":99,"total_cost_usd":99}),
        ] {
            interpretation.condense(&event.to_string());
        }
        let completion = interpretation.finish(Ok(std::process::ExitStatus::from_raw(exit << 8)));
        if exit == 0 {
            assert_eq!(
                completion.outcome.unwrap().final_message.as_deref(),
                Some("Parent done.")
            );
        } else {
            assert_eq!(
                completion.outcome.unwrap_err().to_string(),
                "claude exited 1"
            );
        }
        assert_eq!(
            completion.report.unwrap().summary.as_deref(),
            Some("2 turns, $0.30")
        );
    }
}

#[test]
fn claude_parent_success_clears_recovered_errors_and_never_supplies_an_exit_diagnostic() {
    for exit in [0, 1] {
        let mut interpretation = stream(Harness::Claude, "");
        for event in [
            json!({"type":"assistant","parent_tool_use_id":null,"is_api_error_message":true,"api_error":"usage_limit_reached","api_error_params":{"rate_limit_info":{"rateLimitType":"seven_day","resetsAt":1791597600}},"message":{"content":[{"type":"text","text":"retried root error"}]}}),
            json!({"type":"result","subtype":"error","errors":["earlier failed turn"]}),
            json!({"type":"result","subtype":"success","errors":["not a failure"],"result":"Parent done."}),
        ] {
            interpretation.condense(&event.to_string());
        }
        let completion = interpretation.finish(Ok(std::process::ExitStatus::from_raw(exit << 8)));
        if exit == 0 {
            assert_eq!(
                completion.outcome.unwrap().final_message.as_deref(),
                Some("Parent done.")
            );
        } else {
            assert_eq!(
                completion.outcome.unwrap_err().to_string(),
                "claude exited 1"
            );
        }
    }
}

#[test]
fn claude_recovered_root_api_errors_do_not_contaminate_a_later_failure() {
    for recovery in [
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"Recovered."}]}}),
        json!({"type":"result","subtype":"success","result":"Recovered."}),
    ] {
        let (completion, _) = lines(
            Harness::Claude,
            &[
                json!({"type":"assistant","is_api_error_message":true,"api_error":"usage_limit_reached","api_error_params":{"rate_limit_info":{"rateLimitType":"seven_day","resetsAt":1791597600}},"message":{"content":[{"type":"text","text":"earlier limit"}]}}),
                recovery,
                json!({"type":"assistant","parent_tool_use_id":"child","error":"rate_limit","message":{"content":[{"type":"text","text":"child limit"}]}}),
                json!({"type":"result","subtype":"error"}),
            ],
        );
        assert_eq!(
            completion.outcome.unwrap_err().to_string(),
            "claude's turn failed"
        );
    }
}

#[test]
fn claude_reports_only_supplied_limit_facts_and_never_classifies_a_bare_429() {
    for params in [
        json!(null),
        json!({"rate_limit_info":"malformed"}),
        json!({"rate_limit_info":{"rateLimitType":4,"resetsAt":"tomorrow","overageResetsAt":-1,"overageStatus":false}}),
    ] {
        let (completion, _) = lines(
            Harness::Claude,
            &[
                json!({"type":"result","subtype":"success","is_error":true,"api_error_status":429,"api_error_params":params}),
            ],
        );
        assert_eq!(
            completion.outcome.unwrap_err().to_string(),
            "claude's turn failed: Claude's turn failed\napi_error_status: 429"
        );
    }
    let (completion, _) = lines(
        Harness::Claude,
        &[
            json!({"type":"result","subtype":"error","result":"Extra usage rejected.","api_error_params":{"rate_limit_info":{"overageStatus":"rejected","overageResetsAt":1792000000}}}),
        ],
    );
    assert_eq!(
        completion.outcome.unwrap_err().to_string(),
        "claude's turn failed: Extra usage rejected.\noverageResetsAt: 1792000000; overageStatus: rejected"
    );
}

#[test]
fn first_init_alone_sets_the_directory_subscription_and_init_id() {
    for harness in [Harness::Claude, Harness::Grok] {
        for id in [json!("first"), json!(""), json!(null), json!(7)] {
            let (completion, progress) = lines(
                harness,
                &[
                    json!({"type":"system","subtype":"init","cwd":"/first","session_id":id,"apiKeySource":"none"}),
                    json!({"type":"system","subtype":"init","cwd":"/later","session_id":"later","apiKeySource":"oauth"}),
                    json!({"type":"assistant","message":{"content":[
                        {"type":"tool_use","name":"Read","input":{"file_path":"/first/a.rs"}},
                        {"type":"tool_use","name":"Read","input":{"file_path":"/later/b.rs"}}
                    ]}}),
                    json!({"type":"result","subtype":"success","num_turns":2,"total_cost_usd":0.25}),
                ],
            );
            assert_eq!(
                progress,
                [
                    vec!["session started"],
                    vec![],
                    vec!["Read a.rs", "Read /later/b.rs"],
                    vec![]
                ]
            );
            let expected_id = id
                .as_str()
                .or((harness == Harness::Grok).then_some("later"));
            assert_eq!(
                completion.outcome.unwrap().session_id.as_deref(),
                expected_id
            );
            assert_eq!(
                completion.report.unwrap().summary.as_deref(),
                Some("2 turns, $0.25 at API prices")
            );
        }
    }
}

#[test]
fn later_init_does_not_fill_missing_directory_or_subscription() {
    for harness in [Harness::Claude, Harness::Grok] {
        let (completion, progress) = lines(
            harness,
            &[
                json!({"type":"system","subtype":"init"}),
                json!({"type":"system","subtype":"init","cwd":"/later","apiKeySource":"none"}),
                json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/later/a.rs"}}]}}),
                json!({"type":"result","subtype":"success","num_turns":2,"total_cost_usd":0.25}),
            ],
        );
        assert_eq!(
            progress,
            [
                vec!["session started"],
                vec![],
                vec!["Read /later/a.rs"],
                vec![]
            ]
        );
        assert_eq!(
            completion.report.unwrap().summary.as_deref(),
            Some("2 turns, $0.25")
        );
    }
}

#[test]
fn grok_keeps_the_first_any_event_id_but_prefers_the_first_init_id() {
    for harness in [Harness::Claude, Harness::Grok] {
        for init_id in [None, Some("init"), Some("")] {
            for fallback in ["fallback", ""] {
                let mut events = vec![json!({"type":"unknown","session_id":fallback})];
                if let Some(id) = init_id {
                    events.push(json!({"type":"system","subtype":"init","session_id":id}));
                }
                events.push(json!({"type":"result","subtype":"success","session_id":"last"}));
                let completion = lines(harness, &events).0;
                let expected = init_id.or((harness == Harness::Grok).then_some(fallback));
                assert_eq!(completion.outcome.unwrap().session_id.as_deref(), expected);
            }
        }
    }
}

#[test]
fn each_result_replaces_or_clears_final_text_in_both_dialects() {
    for harness in [Harness::Claude, Harness::Grok] {
        for text in [json!("last"), json!(""), json!(null), json!(7)] {
            let completion = lines(
                harness,
                &[
                    json!({"type":"result","subtype":"success","result":"first"}),
                    json!({"type":"result","subtype":"success","result":text}),
                ],
            )
            .0;
            assert_eq!(
                completion.outcome.unwrap().final_message.as_deref(),
                text.as_str()
            );
        }
        let completion = lines(
            harness,
            &[
                json!({"type":"result","subtype":"success","result":"first"}),
                json!({"type":"result","subtype":"success"}),
            ],
        )
        .0;
        assert_eq!(completion.outcome.unwrap().final_message, None);
    }
}

#[test]
fn last_result_cost_selects_retained_totals_or_current_tokens() {
    for harness in [Harness::Claude, Harness::Grok] {
        for (cost, grok_summary) in [
            (json!(0.75), "4 turns, $0.50 at API prices"),
            (
                json!(0),
                "8 input tokens, 0 cache read tokens, 0 cache creation tokens, 3 output tokens",
            ),
            (
                json!(-1),
                "8 input tokens, 0 cache read tokens, 0 cache creation tokens, 3 output tokens",
            ),
            (
                json!(null),
                "8 input tokens, 0 cache read tokens, 0 cache creation tokens, 3 output tokens",
            ),
        ] {
            let completion = lines(harness, &[
                json!({"type":"system","subtype":"init","apiKeySource":"none"}),
                json!({"type":"result","subtype":"success","num_turns":4,"total_cost_usd":0.5}),
                json!({"type":"result","subtype":"success","total_cost_usd":cost,"usage":{"input_tokens":8,"output_tokens":3}}),
            ]).0;
            let expected = if harness == Harness::Claude {
                "4 turns, $0.50 at API prices"
            } else {
                grok_summary
            };
            assert_eq!(
                completion.report.unwrap().summary.as_deref(),
                Some(expected)
            );
        }
    }
}

#[test]
fn complete_zero_totals_and_positive_cost_without_totals_keep_dialect_spend_rules() {
    for harness in [Harness::Claude, Harness::Grok] {
        let zero = lines(harness, &[
            json!({"type":"result","subtype":"success","num_turns":0,"total_cost_usd":0,"usage":{"input_tokens":8,"output_tokens":3,"cache_read_input_tokens":2,"cache_creation_input_tokens":1}}),
        ]).0;
        let expected = if harness == Harness::Claude {
            "0 turns, $0.00"
        } else {
            "8 input tokens, 2 cache read tokens, 1 cache creation tokens, 3 output tokens"
        };
        assert_eq!(zero.report.unwrap().summary.as_deref(), Some(expected));
        let no_totals = lines(harness, &[
            json!({"type":"result","subtype":"success","total_cost_usd":0.25,"usage":{"input_tokens":8,"output_tokens":3,"cache_read_input_tokens":-1,"cache_creation_input_tokens":"bad"}}),
        ]).0;
        let expected = (harness == Harness::Grok).then_some(
            "8 input tokens, 0 cache read tokens, 0 cache creation tokens, 3 output tokens",
        );
        assert_eq!(no_totals.report.unwrap().summary.as_deref(), expected);
    }
}

#[test]
fn incomplete_totals_are_retained_but_invalid_or_missing_last_tokens_are_cleared() {
    for harness in [Harness::Claude, Harness::Grok] {
        for last in [
            json!({"type":"result","subtype":"success"}),
            json!({"type":"result","subtype":"success","num_turns":-1,"total_cost_usd":0.75,"usage":{"input_tokens":-1,"output_tokens":3}}),
            json!({"type":"result","subtype":"success","num_turns":"bad","total_cost_usd":0,"usage":{"input_tokens":8,"output_tokens":null}}),
        ] {
            let positive = last["total_cost_usd"] == 0.75;
            let completion = lines(harness, &[
                json!({"type":"result","subtype":"success","num_turns":4,"total_cost_usd":0.5,"usage":{"input_tokens":8,"output_tokens":3}}),
                last,
            ]).0;
            let expected = (harness == Harness::Claude || positive).then_some("4 turns, $0.50");
            assert_eq!(completion.report.unwrap().summary.as_deref(), expected);
        }
    }
}

#[test]
fn standalone_errors_can_recover_but_grok_retains_the_diagnostic_for_process_failure() {
    for harness in [Harness::Claude, Harness::Grok] {
        for exit in [0, 1] {
            let mut interpretation = stream(harness, "");
            let progress = interpretation.condense(r#"{"type":"error","message":"retried"}"#);
            let expected = if harness == Harness::Grok {
                vec!["error: retried"]
            } else {
                vec![]
            };
            assert_eq!(progress, expected);
            assert!(
                interpretation
                    .condense(r#"{"type":"result","subtype":"success","result":"done"}"#)
                    .is_empty()
            );
            let completion =
                interpretation.finish(Ok(std::process::ExitStatus::from_raw(exit << 8)));
            if exit == 0 {
                assert_eq!(
                    completion.outcome.unwrap().final_message.as_deref(),
                    Some("done")
                );
            } else {
                let expected = if harness == Harness::Grok {
                    "grok exited 1: retried"
                } else {
                    "claude exited 1"
                };
                assert_eq!(completion.outcome.unwrap_err().to_string(), expected);
            }
        }
    }
}

#[test]
fn grok_error_frames_replace_or_clear_even_empty_diagnostics() {
    for (message, progress, failure) in [
        (json!(""), "error: ", "grok's turn failed: "),
        (
            json!(null),
            "error: Grok's turn failed",
            "grok's turn failed",
        ),
        (json!(7), "error: Grok's turn failed", "grok's turn failed"),
    ] {
        let (completion, actual) = lines(
            Harness::Grok,
            &[
                json!({"type":"error","message":"old"}),
                json!({"type":"error","message":message}),
            ],
        );
        assert_eq!(actual[1], [progress]);
        assert_eq!(completion.outcome.unwrap_err().to_string(), failure);
    }
}

#[test]
fn grok_result_diagnostics_prefer_string_errors_then_nonempty_text_then_prior_error() {
    for (errors, text, diagnostic) in [
        (
            json!([null, "first", 3, "second"]),
            "result text",
            "first\nsecond",
        ),
        (json!([""]), "result text", ""),
        (json!([null, 3]), "result text", "result text"),
        (json!([]), "", "prior"),
    ] {
        let (completion, progress) = lines(
            Harness::Grok,
            &[
                json!({"type":"error","message":"prior"}),
                json!({"type":"result","subtype":"error","errors":errors,"result":text}),
            ],
        );
        assert_eq!(progress[1], [format!("error: {diagnostic}")]);
        assert_eq!(
            completion.outcome.unwrap_err().to_string(),
            format!("grok's turn failed: {diagnostic}")
        );
    }
    let mut interpretation = stream(Harness::Grok, "");
    assert!(
        interpretation
            .condense(r#"{"type":"result","subtype":"success","errors":["","retained"]}"#)
            .is_empty()
    );
    let completion = interpretation.finish(Ok(std::process::ExitStatus::from_raw(256)));
    assert_eq!(
        completion.outcome.unwrap_err().to_string(),
        "grok exited 1: \nretained"
    );
}

#[test]
fn every_result_replaces_failure_and_failed_turns_supply_no_resume_facts() {
    for harness in [Harness::Claude, Harness::Grok] {
        for result in [
            json!({"type":"result","subtype":"error"}),
            json!({"type":"result","subtype":"success","is_error":true}),
            json!({"type":"result"}),
        ] {
            let mut interpretation = stream(harness, "");
            interpretation
                .condense(r#"{"type":"system","subtype":"init","session_id":"resume-id"}"#);
            interpretation.condense(&result.to_string());
            interpretation.condense(r#"{"type":"system","subtype":"task_notification","task_id":"test","status":"stopped","summary":"cargo test"}"#);
            let completion = finish(interpretation);
            assert!(completion.outcome.is_err());
            assert!(completion.report.is_some());
            let (recovered, _) = lines(
                harness,
                &[result, json!({"type":"result","subtype":"success"})],
            );
            assert!(recovered.outcome.is_ok());
        }
    }
}

#[test]
fn grok_tool_aliases_and_paths_are_selected_without_changing_claude_inputs() {
    for harness in [Harness::Claude, Harness::Grok] {
        let (completion, progress) = lines(
            harness,
            &[
                json!({"type":"system","subtype":"init","cwd":"/repo"}),
                json!({"type":"assistant","message":{"content":[
                    {"type":"tool_use","name":"bash","input":{"command":"git commit -m x"}},
                    {"type":"tool_use","name":"run_terminal_command","input":{"command":"git push"}},
                    {"type":"tool_use","name":"Bash","input":{"command":"cargo test\necho done"}},
                    {"type":"tool_use","name":"read_file","input":{"path":"/repo/native.rs","file_path":"/repo/claude.rs","pattern":"pattern"}},
                    {"type":"tool_use","name":"Read","input":{"path":"","file_path":"/repo/claude.rs"}},
                    {"type":"tool_use","name":"Read","input":{"path":7,"file_path":"/repo/claude.rs"}},
                    {"type":"tool_use","name":"Search","input":{"pattern":"pattern","description":"description","url":"url","query":"query"}}
                ]}}),
            ],
        );
        let expected = if harness == Harness::Grok {
            vec![
                "commit",
                "push",
                "$ cargo test",
                "read_file native.rs",
                "Read ",
                "Read claude.rs",
                "Search pattern",
            ]
        } else {
            vec![
                "bash",
                "run_terminal_command",
                "$ cargo test",
                "read_file claude.rs",
                "Read claude.rs",
                "Read claude.rs",
                "Search pattern",
            ]
        };
        assert_eq!(progress[1], expected);
        assert!(completion.outcome.is_ok());
    }
}

#[test]
fn progress_preserves_order_unicode_truncation_and_ignores_malformed_frames() {
    for harness in [Harness::Claude, Harness::Grok] {
        let mut interpretation = stream(harness, "");
        for raw in [
            "not json",
            "null",
            "[]",
            "42",
            "{",
            "{}",
            r#"{"type":"assistant","message":{"content":"bad"}}"#,
        ] {
            assert!(interpretation.condense(raw).is_empty());
        }
        let progress = interpretation.condense(
            &json!({"type":"assistant","message":{"content":[
                {"type":"text","text":"ignored"},
                {"type":"tool_use","name":"Read","input":{"file_path":"é".repeat(150)}},
                {"type":"tool_use"},
                {"type":"tool_use","name":"Bash","input":{"command":"é".repeat(150)}},
                {"type":"tool_use","name":"Skill","input":{"skill":"thirdshift-code-review"}}
            ]}})
            .to_string(),
        );
        assert_eq!(
            progress,
            [
                format!("Read {}…", "é".repeat(100)),
                format!("$ {}…", "é".repeat(100)),
                "skill thirdshift-code-review".to_string()
            ]
        );
        assert!(finish(interpretation).outcome.is_ok());
    }
}

#[test]
fn only_claude_keeps_ordered_deduplicated_killed_work_with_description_fallbacks() {
    for harness in [Harness::Claude, Harness::Grok] {
        let completion = lines(harness, &[
            json!({"type":"system","subtype":"task_started","task_id":"first","description":"cargo test"}),
            json!({"type":"system","subtype":"task_updated","task_id":"first","patch":{"status":"killed"}}),
            json!({"type":"result","subtype":"success"}),
            json!({"type":"system","subtype":"task_notification","task_id":"second","status":"stopped","summary":"npm run build"}),
            json!({"type":"system","subtype":"task_updated","task_id":"first","patch":{"status":"killed"}}),
            json!({"type":"system","subtype":"task_notification","task_id":"first","status":"stopped","summary":"ignored"}),
            json!({"type":"system","subtype":"task_updated","task_id":"third","patch":{"status":"killed"}}),
        ]).0;
        let expected = if harness == Harness::Claude {
            vec!["npm run build", "cargo test", "third"]
        } else {
            vec![]
        };
        assert_eq!(completion.outcome.unwrap().killed_work(), expected);
    }
}

mod claude {
    use crate::harness::{Harness, interpretation_tests::lines};
    use serde_json::{Value, json};

    fn tool_use(name: &str, input: Value) -> Value {
        json!({
            "type": "assistant",
            "message": { "content": [{ "type": "tool_use", "name": name, "input": input }] }
        })
    }

    fn init(cwd: &str) -> Value {
        json!({ "type": "system", "subtype": "init", "cwd": cwd })
    }

    fn line_for(event: Value) -> Option<String> {
        let mut lines = lines(Harness::Claude, &[init("/work/widgets-issue-7"), event])
            .1
            .pop()
            .unwrap();
        assert!(lines.len() <= 1, "several lines: {lines:?}");
        lines.pop()
    }

    fn result(turns: u64, cost: f64) -> Value {
        json!({ "type": "result", "subtype": "success", "num_turns": turns, "total_cost_usd": cost })
    }

    #[test]
    fn a_skill_is_named() {
        assert_eq!(
            line_for(tool_use(
                "Skill",
                json!({ "skill": "thirdshift-tdd", "args": "x" })
            )),
            Some("skill thirdshift-tdd".to_string())
        );
    }

    #[test]
    fn commits_and_pushes_are_named() {
        let line = |command: &str| line_for(tool_use("Bash", json!({ "command": command })));
        assert_eq!(line("git commit -m 'x'"), Some("commit".to_string()));
        assert_eq!(line("git push -u origin issue-7"), Some("push".to_string()));
        assert_eq!(
            line("git add -A && git commit -m x && git push"),
            Some("commit and push".to_string())
        );
    }

    #[test]
    fn mentions_of_commit_or_push_are_not_commits_or_pushes() {
        let line = |command: &str| line_for(tool_use("Bash", json!({ "command": command })));
        assert_eq!(
            line("grep 'git push' README.md"),
            Some("$ grep 'git push' README.md".to_string())
        );
        assert_eq!(
            line("git commit-tree HEAD^{tree}"),
            Some("$ git commit-tree HEAD^{tree}".to_string())
        );
    }

    #[test]
    fn other_commands_show_their_first_line() {
        assert_eq!(
            line_for(tool_use(
                "Bash",
                json!({ "command": "cargo test\necho done" })
            )),
            Some("$ cargo test".to_string())
        );
    }

    #[test]
    fn file_tools_show_the_path_relative_to_the_session() {
        assert_eq!(
            line_for(tool_use(
                "Edit",
                json!({ "file_path": "/work/widgets-issue-7/src/run.rs" })
            )),
            Some("Edit src/run.rs".to_string())
        );
        assert_eq!(
            line_for(tool_use("Read", json!({ "file_path": "/etc/hosts" }))),
            Some("Read /etc/hosts".to_string())
        );
    }

    #[test]
    fn other_tools_show_their_most_telling_input() {
        assert_eq!(
            line_for(tool_use("Grep", json!({ "pattern": "fn main" }))),
            Some("Grep fn main".to_string())
        );
        assert_eq!(
            line_for(tool_use(
                "Agent",
                json!({ "description": "Standards review" })
            )),
            Some("Agent Standards review".to_string())
        );
        assert_eq!(
            line_for(tool_use("TodoWrite", json!({ "todos": [] }))),
            Some("TodoWrite".to_string())
        );
    }

    #[test]
    fn text_tool_results_and_results_give_no_line() {
        let (_, lines) = lines(
            Harness::Claude,
            &[
                json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": "hi" }] } }),
                json!({ "type": "user", "message": { "content": [{ "type": "tool_result" }] } }),
                json!({ "type": "result", "num_turns": 3, "total_cost_usd": 0.1 }),
            ],
        );
        assert_eq!(lines, [vec![], vec![], vec![]] as [Vec<String>; 3]);
    }

    #[test]
    fn the_summary_is_the_last_result_with_totals() {
        let (progress, _) = lines(
            Harness::Claude,
            &[
                result(10, 0.5),
                result(34, 1.8249),
                json!({ "type": "result", "subtype": "success" }),
            ],
        );
        assert_eq!(
            progress.report.as_ref().unwrap().summary.clone().as_deref(),
            Some("34 turns, $1.82")
        );
    }

    #[test]
    fn on_a_subscription_the_cost_is_marked_as_at_api_prices() {
        let (progress, _) = lines(
            Harness::Claude,
            &[
                json!({ "type": "system", "subtype": "init", "apiKeySource": "none" }),
                result(34, 1.82),
            ],
        );
        assert_eq!(
            progress.report.as_ref().unwrap().summary.clone().as_deref(),
            Some("34 turns, $1.82 at API prices")
        );
    }

    #[test]
    fn no_result_means_no_summary() {
        assert_eq!(lines(Harness::Claude, &[]).0.report.unwrap().summary, None);
    }

    fn task_started(id: &str, description: &str) -> Value {
        json!({ "type": "system", "subtype": "task_started", "task_id": id, "description": description })
    }

    fn task_updated(id: &str, status: &str) -> Value {
        json!({ "type": "system", "subtype": "task_updated", "task_id": id, "patch": { "status": status } })
    }

    fn task_notification(id: &str, status: &str, summary: &str) -> Value {
        json!({ "type": "system", "subtype": "task_notification", "task_id": id, "status": status, "summary": summary })
    }

    #[test]
    fn a_task_killed_after_the_last_result_is_killed_background_work() {
        let (progress, _) = lines(
            Harness::Claude,
            &[
                json!({ "type": "system", "subtype": "init", "session_id": "s-1" }),
                task_started("b1", "./mvnw test -Dtest='GamesPageTest'"),
                result(12, 0.4),
                task_updated("b1", "killed"),
            ],
        );
        assert_eq!(
            progress.outcome.as_ref().unwrap().session_id.as_deref(),
            Some("s-1")
        );
        assert_eq!(
            progress.outcome.as_ref().unwrap().killed_work(),
            ["./mvnw test -Dtest='GamesPageTest'"]
        );
    }

    #[test]
    fn a_stopped_task_notification_is_killed_background_work_once() {
        let (progress, _) = lines(
            Harness::Claude,
            &[
                task_started("b1", "cargo test"),
                result(12, 0.4),
                task_updated("b1", "killed"),
                task_notification("b1", "stopped", "cargo test"),
                task_notification("b2", "stopped", "npm run build"),
            ],
        );
        assert_eq!(
            progress.outcome.as_ref().unwrap().killed_work(),
            ["cargo test", "npm run build"]
        );
    }

    #[test]
    fn no_killed_tasks_means_no_killed_background_work() {
        let (progress, _) = lines(
            Harness::Claude,
            &[
                task_started("b1", "cargo test"),
                task_notification("b1", "completed", "cargo test"),
                result(12, 0.4),
            ],
        );
        assert!(progress.outcome.as_ref().unwrap().killed_work().is_empty());
    }

    #[test]
    fn background_sub_agents_that_resumed_the_session_are_not_killed_background_work() {
        let (progress, _) = lines(
            Harness::Claude,
            &[
                task_started("a1", "Standards review"),
                result(12, 0.4),
                task_updated("a1", "completed"),
                task_notification("a1", "completed", "Standards review"),
                init("/work/widgets-issue-7"),
                result(20, 0.9),
            ],
        );
        assert!(progress.outcome.as_ref().unwrap().killed_work().is_empty());
    }

    #[test]
    fn a_task_killed_before_a_later_result_is_not_killed_background_work() {
        let (progress, _) = lines(
            Harness::Claude,
            &[
                task_started("b1", "cargo watch"),
                task_updated("b1", "killed"),
                result(12, 0.4),
                init("/work/widgets-issue-7"),
                task_notification("b1", "stopped", "cargo watch"),
                result(20, 0.9),
            ],
        );
        assert!(progress.outcome.as_ref().unwrap().killed_work().is_empty());
    }

    #[test]
    fn an_init_without_a_session_id_gives_none() {
        let (progress, _) = lines(Harness::Claude, &[init("/a")]);
        assert_eq!(
            progress.outcome.as_ref().unwrap().session_id.as_deref(),
            None
        );
    }
}

mod grok {
    use crate::harness::{
        Harness,
        interpretation_tests::{finish, stream},
    };

    #[test]
    fn messages_frames_keep_groks_progress_id_final_text_and_spend() {
        let mut stream = stream(Harness::Grok, "");
        assert_eq!(stream.condense(r#"{"type":"system","subtype":"init","session_id":"abc123","apiKeySource":"oauth","cwd":"/repo"}"#), ["session started"]);
        assert_eq!(stream.condense(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"read_file","input":{"path":"/repo/src/main.rs"}},{"type":"tool_use","name":"bash","input":{"command":"cargo test"}}]}}"#), ["read_file src/main.rs", "$ cargo test"]);
        stream.condense(r#"{"type":"result","subtype":"success","is_error":false,"num_turns":7,"result":"Here's a summary...","total_cost_usd":0.0127,"usage":{"input_tokens":812,"output_tokens":210,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"session_id":"abc123"}"#);
        let stream = finish(stream);
        assert_eq!(
            stream.outcome.as_ref().unwrap().session_id.as_deref(),
            Some("abc123")
        );
        assert_eq!(
            stream.outcome.as_ref().unwrap().final_message.as_deref(),
            Some("Here's a summary...")
        );
        assert_eq!(
            stream.report.as_ref().unwrap().summary.clone().as_deref(),
            Some("7 turns, $0.01")
        );
        assert!(stream.outcome.is_ok());
    }

    #[test]
    fn usage_without_a_complete_cost_and_a_result_without_init_are_still_read() {
        let mut stream = stream(Harness::Grok, "");
        stream.condense(r#"{"type":"result","subtype":"success","result":"done","num_turns":1,"total_cost_usd":0,"usage":{"input_tokens":812,"output_tokens":210,"cache_read_input_tokens":45,"cache_creation_input_tokens":12},"session_id":"abc123"}"#);
        let stream = finish(stream);
        assert_eq!(
            stream.outcome.as_ref().unwrap().session_id.as_deref(),
            Some("abc123")
        );
        assert_eq!(
            stream.report.as_ref().unwrap().summary.clone().as_deref(),
            Some(
                "812 input tokens, 45 cache read tokens, 12 cache creation tokens, 210 output tokens"
            )
        );
    }

    #[test]
    fn error_events_are_condensed_to_progress_with_groks_failure_text() {
        for (raw, message) in [
            (
                r#"{"type":"error","message":"backend unavailable"}"#,
                "backend unavailable",
            ),
            (
                r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["quota exceeded"]}"#,
                "quota exceeded",
            ),
        ] {
            let mut stream = stream(Harness::Grok, "");
            assert_eq!(stream.condense(raw), [format!("error: {message}")]);
            let stream = finish(stream);
            assert!(stream.outcome.is_err());
            assert_eq!(
                stream.outcome.as_ref().unwrap_err().to_string(),
                format!("grok's turn failed: {message}")
            );
        }
    }
}

#[test]
fn claude_root_api_error_is_a_fallback_when_process_exits_without_a_result() {
    for earlier_success in [true, false] {
        for exit in [0, 1] {
            let mut interpretation = stream(Harness::Claude, "");
            if earlier_success {
                interpretation.condense(
                    &json!({
                        "type":"result", "subtype":"success",
                        "result":"Earlier work completed."
                    })
                    .to_string(),
                );
            }
            interpretation.condense(
                &json!({
                    "type":"assistant", "parent_tool_use_id":null,
                    "is_api_error_message":true,
                    "api_error":"usage_limit_reached", "api_error_status":429,
                    "api_error_params":{"rate_limit_info":{
                        "rateLimitType":"seven_day", "resetsAt":1791597600,
                        "overageStatus":"rejected",
                        "overageDisabledReason":"org_level_disabled_until",
                        "unrelated":"private raw event content"
                    }},
                    "message":{"content":[{
                        "type":"text", "text":"Provider's limit message."
                    }]}
                })
                .to_string(),
            );
            let completion =
                interpretation.finish(Ok(std::process::ExitStatus::from_raw(exit << 8)));
            if exit == 0 {
                assert_eq!(
                    completion.outcome.unwrap().final_message.as_deref(),
                    earlier_success.then_some("Earlier work completed."),
                );
            } else {
                let cause = completion.outcome.unwrap_err().to_string();
                assert!(
                    cause.starts_with("claude exited 1: Provider's limit message."),
                    "{cause}",
                );
                for fact in [
                    "api_error: usage_limit_reached",
                    "api_error_status: 429",
                    "rateLimitType: seven_day",
                    "resetsAt: 1791597600",
                    "overageStatus: rejected",
                    "overageDisabledReason: org_level_disabled_until",
                ] {
                    assert!(cause.contains(fact), "missing {fact:?}: {cause}");
                }
                assert!(!cause.contains("private raw event content"), "{cause}");
            }
        }
    }
}
