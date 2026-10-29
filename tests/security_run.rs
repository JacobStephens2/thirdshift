mod support;

use std::fs;

use serde_json::{Value, json};
use support::resend::ResendStandIn;
use support::{REPO, Scenario, write_executable};

fn resend_env(resend: &ResendStandIn) -> [(&str, &str); 2] {
    [
        ("THIRDSHIFT_RESEND_URL", resend.url()),
        ("RESEND_API_KEY", "re_security_test"),
    ]
}

fn the_one_notification(resend: &ResendStandIn) -> (String, String) {
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let body = &requests[0].body;
    (
        body["subject"].as_str().unwrap().to_string(),
        body["text"].as_str().unwrap().to_string(),
    )
}

fn finding(fingerprint: &str) -> Value {
    json!({
        "verdict": "needs_validation", "fingerprint": fingerprint,
        "title": "Unchecked input size", "description": "Private candidate write-up.",
        "claimed_root_cause": "Input reaches storage without a bound.",
        "trace": [{"kind":"entrypoint", "file":"README.md", "line":1, "scope":"input", "description":"Private trace."}],
        "evidence": [{"file":"README.md", "line":1, "description":"Private evidence."}],
        "blockers": ["No sandbox available."],
        "validation_plan": {"local":"Exercise a bounded fixture."}
    })
}

fn safeguard_refusals() -> [(&'static str, &'static str, &'static str); 2] {
    [
        (
            "claude",
            r#"printf '%s\n' 'API Error: [cyber] Private refusal evidence.' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
            "Claude Code's [cyber] safeguard refusal",
        ),
        (
            "codex",
            r#"printf '%s\n' 'Cybersecurity safeguard refused: Private refusal evidence.' > "$FAKE_CODEX_ERROR"
exit 1"#,
            "Codex's cybersecurity safeguard refusal",
        ),
    ]
}

fn security_command_log(scenario: &Scenario) -> String {
    let logs = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/secure", "log");
    fs::read_to_string(scenario.path(&format!(
        "home/.thirdshift/logs/acme/widgets/commands/secure/{}",
        logs[0]
    )))
    .unwrap()
}

#[test]
fn otlp_never_receives_a_security_runs_sessions_or_nested_ticket_runs() {
    for spec in [false, true] {
        let receiver = ResendStandIn::replying(200, "{}");
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
        scenario.user_config_is(&format!(
            "[otlp]\nendpoint = \"{}/v1/traces\"\n",
            receiver.url()
        ));
        scenario.agent_does_in_session(
            1,
            &audit_script(&json!([finding("otlp-private")]).to_string()),
        );
        scenario.agent_does_in_session(
            2,
            &reproduction_script(if spec {
                "reproduced high spec"
            } else {
                "reproduced high single"
            }),
        );
        let record = "https://github.com/acme/widgets/security/advisories/GHSA-test-test-0001";
        scenario.agent_does_in_session(
            3,
            &if spec {
                stage_spec(record)
            } else {
                stage_fix("GHSA-test-test-0001")
            },
        );
        if spec {
            scenario.agent_does_for(9, &implement_spec_ticket(9, 8));
            scenario.agent_does_for(10, &implement_spec_ticket(10, 8));
        } else {
            scenario.agent_does_for(8, implement_fix());
        }
        let result = scenario.run(&["secure", "security-fix", "parallel", "1"]);
        assert_eq!(result.code, Some(0), "spec={spec}: {}", result.stderr);
        let local_traces = scenario
            .entries("home/.thirdshift/logs/acme/widgets/commands/secure")
            .into_iter()
            .filter(|name| name.ends_with(".spans.jsonl"))
            .count();
        assert!(
            local_traces >= 4,
            "Security sessions were not exercised: {local_traces}"
        );
        assert!(
            receiver.requests().is_empty(),
            "Security spans were exported: {:?}",
            receiver.requests()
        );
    }
}

#[test]
fn a_claude_cyber_refusal_fails_the_security_audit_with_its_cause() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
printf '%s\n' 'API Error: [cyber] This request was refused by the safeguard.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("Claude Code's [cyber] safeguard refusal"),
        "{}",
        result.stderr
    );
    assert!(scenario.gh_state()["advisories"].is_null());
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(scenario.entries("work"), vec![REPO]);
}

#[test]
fn safeguard_refusals_notify_the_cause_without_private_diagnostics() {
    for (harness, script, cause) in safeguard_refusals() {
        let scenario = Scenario::new();
        scenario.agent_does(script);
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result = scenario.run_with_env(
            &["secure", "harness", harness, "email", "me@example.com"],
            &resend_env(&resend),
        );
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(cause), "{}", result.stderr);
        let (subject, text) = the_one_notification(&resend);
        assert_eq!(
            subject,
            "[thirdshift] acme/widgets Security run: audit failed"
        );
        assert!(text.contains("Audit:        audit failed"), "{text}");
        assert!(!text.contains("audit complete"), "{text}");
        assert!(!text.contains("Reproduction:"), "{text}");
        assert_eq!(
            scenario.claude_calls().len() + scenario.codex_calls().len(),
            1
        );
        assert!(text.contains(&format!("Cause:        {cause}")), "{text}");
        assert!(!text.contains("Private refusal evidence."), "{text}");
    }
}

#[test]
fn security_sessions_log_claudes_answering_models_in_progress_and_the_command_log() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, "true\n"); // Requested Model check.
    scenario.agent_does_in_session(
        2,
        &format!(
            r#"
printf '%s\n' '{{"type":"assistant","message":{{"model":"claude-opus-5-5","content":[]}}}}'
printf '%s\n' '{{"type":"assistant","parent_tool_use_id":"child","message":{{"model":"claude-sonnet-5","content":[]}}}}'
printf '%s\n' '{{"type":"system","subtype":"model_refusal_fallback","original_model":"claude-opus-5-5","fallback_model":"claude-opus-4-8","api_refusal_category":"cyber"}}'
printf '%s\n' '{{"type":"assistant","message":{{"model":"claude-opus-4-8","content":[]}}}}'
{}
"#,
            audit_script(&json!([finding("model-switch")]).to_string())
        ),
    );
    scenario.agent_does_in_session(
        3,
        &format!(
            "printf '%s\\n' '{{\"type\":\"assistant\",\"message\":{{\"model\":\"claude-opus-4-8\",\"content\":[]}}}}'\n{}",
            reproduction_script("not reproduced")
        ),
    );
    let result = scenario.run(&["secure", "model", "opus"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let command_log = security_command_log(&scenario);
    let session_logs = scenario.log_files(
        "home/.thirdshift/logs/acme/widgets/commands/secure",
        "jsonl",
    );
    assert_eq!(session_logs.len(), 2, "{session_logs:?}");
    assert!(
        session_logs
            .iter()
            .any(|name| name.ends_with("-security-audit.logs.jsonl"))
    );
    assert!(
        session_logs
            .iter()
            .any(|name| name.ends_with("-security-reproduction-1.logs.jsonl"))
    );
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/sessions")
            .exists()
    );
    for line in [
        "security-audit: Model: claude-opus-5-5",
        "security-audit: Model: claude-opus-4-8",
        "security-reproduction-1: Model: claude-opus-4-8",
    ] {
        assert!(result.stderr.contains(line), "{}", result.stderr);
        assert!(command_log.contains(line), "{command_log}");
    }
    assert!(
        !command_log.contains("Model: claude-sonnet-5"),
        "{command_log}"
    );
}

#[test]
fn codex_logs_the_requested_model_for_each_security_session_and_resume() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{}\nprintf '%s\\n' '{{\"type\":\"item.started\",\"item\":{{\"id\":\"child\",\"type\":\"collab_tool_call\",\"tool\":\"spawn_agent\",\"status\":\"in_progress\"}}}}'\n",
            audit_script(&json!([finding("codex-model")]).to_string())
        ),
    );
    scenario.agent_does_in_session(
        2,
        r#"printf '%s\n' 'Security audit: complete' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
    );
    scenario.agent_does_in_session(3, &reproduction_script("not reproduced"));
    let result = scenario.run(&["secure", "harness", "codex", "model", "GPT-6.1-Sol"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let command_log = security_command_log(&scenario);
    for kind in [
        "security-audit",
        "security-audit-resume",
        "security-reproduction-1",
    ] {
        let line = format!("{kind}: Model: gpt-6.1-sol (requested)");
        assert!(result.stderr.contains(&line), "{}", result.stderr);
        assert!(command_log.contains(&line), "{command_log}");
    }
}

#[test]
fn refused_security_reproductions_keep_the_record_and_stop_before_the_next_finding() {
    for (harness, script, cause) in safeguard_refusals() {
        for private in [false, true] {
            let scenario = Scenario::new();
            let mut github = scenario.gh_state();
            github["private"] = json!(private);
            scenario.write_gh_state(&github);
            scenario.agent_does_in_session(
                1,
                &audit_script(&json!([finding("first"), finding("second")]).to_string()),
            );
            scenario.agent_does_in_session(2, script);
            scenario.agent_does_in_session(3, "exit 99\n");
            let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
            let result = scenario.run_with_env(
                &[
                    "secure",
                    "security-fix",
                    "harness",
                    harness,
                    "email",
                    "me@example.com",
                ],
                &resend_env(&resend),
            );
            assert_eq!(result.code, Some(1), "{}", result.stderr);
            assert!(result.stderr.contains(cause), "{}", result.stderr);
            let (subject, text) = the_one_notification(&resend);
            assert!(subject.ends_with(": reproduction failed"), "{subject}");
            for line in [
                "Result:       reproduction failed",
                "Audit:        audit complete",
                "Reproduction: 1 refused",
                "Session trace:",
                "security-reproduction-1.logs.jsonl",
                "Command log:",
            ] {
                assert!(text.contains(line), "missing {line}: {text}");
            }
            assert!(text.contains(cause), "{text}");
            assert!(!text.contains("Private refusal evidence."), "{text}");
            let state = scenario.gh_state();
            let records = if private {
                state["bodies"]
                    .as_object()
                    .unwrap()
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                let advisories = state["advisories"].as_array().unwrap();
                assert_eq!(advisories.len(), 2);
                assert!(advisories.iter().all(|record| record["severity"].is_null()));
                advisories
                    .iter()
                    .map(|record| record["description"].clone())
                    .collect()
            };
            assert_eq!(records.len(), 2);
            for record in records {
                assert!(!record.as_str().unwrap().contains("## Reproduction"));
            }
            assert_eq!(
                scenario.claude_calls().len() + scenario.codex_calls().len(),
                2
            );
            assert_eq!(scenario.entries("work"), vec![REPO]);
            assert!(state["prs"].as_array().unwrap().is_empty());
            for private_detail in [
                "Private candidate write-up.",
                "Private trace.",
                "Private evidence.",
            ] {
                assert!(
                    !resend.requests()[0]
                        .body
                        .to_string()
                        .contains(private_detail)
                );
            }
        }
    }
}

#[test]
fn a_reproduction_scores_the_finding_and_keeps_its_test_in_the_private_record() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("input-size")]).to_string()),
    );
    scenario.agent_does_in_session(2, r#"
printf '%s' "$FAKE_CLAUDE_PROMPT" > prompt.txt
test_file=$(sed -n 's/^Test file: `\(.*\)`\.$/\1/p' prompt.txt)
test -n "$test_file"
printf '%s\n' 'assert_eq!(bounded_fixture(), "overflow");' > "$test_file"
printf '%s\n' 'A harmless local fixture crossed the storage bound. Likelihood high, impact high; one session can hold the fix.' 'Security reproduction: reproduced high single' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let record = &state["advisories"][0];
    assert_eq!(record["severity"], "high");
    let body = record["description"].as_str().unwrap();
    assert!(body.contains("Likelihood high, impact high"), "{body}");
    assert!(
        body.contains("assert_eq!(bounded_fixture(), \"overflow\");"),
        "{body}"
    );
    assert!(body.contains("Fix size: single"), "{body}");
    assert!(
        result
            .stderr
            .contains("reproduction 1: reproduced high single"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 2);
    scenario.assert_every_session_found_the_factory_skills();
}

#[test]
fn a_private_reproduction_writes_severity_size_notes_and_test_into_the_finding_issue() {
    let scenario = Scenario::new();
    let mut github = scenario.gh_state();
    github["private"] = json!(true);
    scenario.write_gh_state(&github);
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("input-size")]).to_string()),
    );
    scenario.agent_does_in_session(2, &reproduction_script("reproduced medium spec"));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let body = state["bodies"]["8"].as_str().unwrap();
    for required in [
        "Fingerprint: `input-size`",
        "Private candidate write-up.",
        "Severity: medium",
        "Fix size: spec",
        "Local command: bounded-fixture",
        "bounded_fixture();",
    ] {
        assert!(body.contains(required), "missing {required}: {body}");
    }
    assert_eq!(
        state["labels"]["8"],
        json!(["security-finding", "needs-triage"])
    );
    assert_eq!(state["issues"]["8"], "OPEN");
    assert!(result.stderr.contains("issue #8"), "{}", result.stderr);
    assert!(!result.stderr.contains("Local command: bounded-fixture"));
}

fn reproduction_script(outcome: &str) -> String {
    format!(
        r#"
printf '%s' "$FAKE_CLAUDE_PROMPT" > prompt.txt
test_file=$(sed -n 's/^Test file: `\(.*\)`\.$/\1/p' prompt.txt)
test -n "$test_file"
printf '%s\n' 'bounded_fixture();' > "$test_file"
printf '%s\n' 'Local command: bounded-fixture; harmless local evidence and likelihood/impact reasoning.' 'Security reproduction: {outcome}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
    )
}

fn with_reproduced_findings(severities: &[&str]) -> Scenario {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    let commit = scenario.origin_git(&["rev-parse", "main"]);
    let commit = commit.trim_end();
    let mut state = scenario.gh_state();
    state["advisories"] = json!(severities.iter().enumerate().map(|(index, severity)| {
        json!({
            "ghsa_id": format!("GHSA-finding-{index}"), "state": "draft",
            "summary": format!("Finding {index}"), "severity": severity,
            "html_url": format!("https://github.com/acme/widgets/security/advisories/GHSA-finding-{index}"),
            "description": format!("Fingerprint: `finding-{index}`\nAudited commit: `{commit}`\nPrivate candidate write-up.\n\n<!-- thirdshift:security-reproduction -->\n## Reproduction\n\nOutcome: reproduced {severity} single\nSeverity: {severity}\nFix size: single\n\nPrivate reproduction notes.\n\n### Proof-of-concept test\n\n```\nbounded_fixture();\n```\n")
        })
    }).collect::<Vec<_>>());
    scenario.write_gh_state(&state);
    scenario
}

/// A session-created URL final line with no staged proposal and no
/// session-created issue: thirdshift itself must create nothing.
fn url_line_only() -> &'static str {
    r#"
printf 'Security fix Ticket: https://github.com/acme/widgets/issues/8\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
}

fn implement_fix() -> &'static str {
    r#"
echo bounded > bounded.txt
git add bounded.txt
git commit -q -m 'Bound accepted input'
gh pr create --base main --head issue-8 --title 'Bound accepted input' --body 'Closes #8'
"#
}

/// A staged proposal whose title copies the private write-up, which
/// thirdshift rejects before creating any public issue.
fn stage_private_title() -> &'static str {
    "printf 'Proposed public issue:\\nTitle: Private candidate write-up.\\nBody:\\nReject oversized input. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n"
}

/// The #627 staged path: the session returns its proposed public issue text
/// without running `gh issue create` itself.
fn stage_fix(record: &str) -> String {
    format!(
        r#"
printf 'Proposed public issue:\nTitle: Bound accepted input\nBody:\nReject oversized input. Private record: https://github.com/acme/widgets/security/advisories/{record}\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
    )
}

#[test]
fn status_tracks_security_stages_and_nests_the_dispatched_fix() {
    for (held_session, stage) in [
        (1, "Security audit"),
        (2, "Security reproduction"),
        (3, "Security fix publishing"),
        (4, "implement"),
    ] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
        let scripts = [
            audit_script(&json!([finding("status-stages")]).to_string()),
            reproduction_script("reproduced high single"),
            stage_fix("GHSA-test-test-0001"),
            implement_fix().into(),
        ];
        for (index, script) in scripts.iter().enumerate() {
            let hold = if index + 1 == held_session {
                "root=$(dirname \"$FAKE_CLAUDE_RECORD\")\ntouch \"$root/started\"\nwhile [ -d \"$root\" ] && [ ! -e \"$root/release\" ]; do sleep 0.05; done\n"
            } else {
                ""
            };
            scenario.agent_does_in_session(index + 1, &format!("{hold}{script}"));
        }
        let held = scenario.run_until(&["secure", "security-fix"], &[], "started");
        let result = scenario.run(&["status", "--json"]);
        let text = scenario.run(&["status"]);
        let shown = (held_session == 4)
            .then(|| scenario.run(&["show", "https://github.com/acme/widgets/issues/8", "--json"]));
        fs::write(scenario.path("release"), "").unwrap();
        let ended = held.finish();
        assert_eq!(ended.code, Some(0), "{}", ended.stderr);
        if let Some(shown) = shown {
            assert_eq!(shown.code, Some(0), "{}", shown.stderr);
            let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
            assert_eq!(shown["command"]["state"], "live");
            assert_eq!(
                shown["command"]["security_fix_findings"],
                json!([{
                    "title": "Unchecked input size", "severity": "high",
                    "url": "https://github.com/acme/widgets/security/advisories/GHSA-test-test-0001"
                }])
            );
        }
        let status: Value = serde_json::from_str(&result.stdout).unwrap();
        let pass = &status["commands"][0];
        assert_eq!(pass["kind"], "Security run", "{status}");
        assert_eq!(pass["issue"], Value::Null);
        let active = if held_session == 4 {
            &pass["dispatched"][0]
        } else {
            pass
        };
        assert_eq!(active["stage"]["name"], stage, "{status}");
        assert!(active["stage"]["age_seconds"].is_number());
        if held_session == 4 {
            assert_eq!(pass["stage"]["name"], "dispatched Run", "{status}");
            assert_eq!(pass["health"]["state"], "ok", "{status}");
            let row = text
                .stdout
                .lines()
                .find(|line| line.starts_with("Security run "))
                .expect(&text.stdout);
            assert!(row.contains(" · dispatched Ticket run · "), "{row}");
            assert_eq!(active["issue"]["number"], 8);
            assert!(
                text.stdout.contains("\n  Ticket run acme/widgets#8"),
                "{}",
                text.stdout
            );
        }
        let folder = "home/.thirdshift/logs/acme/widgets/commands/secure";
        let record = scenario.command_record(folder);
        let stages: Vec<_> = record["stages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stage| stage["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            stages,
            [
                "Security audit",
                "Security reproduction",
                "Security fix publishing",
                "dispatched Run"
            ]
        );
        assert_eq!(record["outcome"], "ready for review");
        assert_eq!(record["dispatched"][0]["outcome"], "ready for review");
    }
}

#[test]
fn public_fix_publishing_cannot_write_github_content_with_any_harness() {
    for harness in ["claude", "codex", "agy", "grok", "muse", "opencode"] {
        let scenario = with_reproduced_findings(&["high"]);
        let before = scenario.gh_state();
        scenario.agent_does_in_session(
            1,
            r#"
for command in \
  "issue create --title Unvalidated --body Unvalidated" \
  "issue comment 7 --body Unvalidated" \
  "issue edit 7 --body Unvalidated" \
  "pr create --base main --head main --title Unvalidated --body Unvalidated" \
  "pr comment 1 --body Unvalidated" \
  "pr edit 1 --body Unvalidated" \
  "api --method POST repos/acme/widgets/issues -f title=Unvalidated"
do
  if gh $command 2> denied.txt; then
    echo "publishing session retained GitHub authority: $command" >&2
    exit 90
  fi
  grep -q 'authentication required' denied.txt
done
echo 'all publishing writes denied' >&2
exit 1
"#,
        );
        let result = scenario.run_with_env(
            &["secure", "security-fix", "harness", harness],
            &[
                ("FAKE_GH_REQUIRE_AUTH", "1"),
                ("GH_TOKEN", "factory-token"),
                ("GITHUB_TOKEN", "factory-token"),
                ("GH_ENTERPRISE_TOKEN", "factory-token"),
                ("GITHUB_ENTERPRISE_TOKEN", "factory-token"),
            ],
        );
        assert_eq!(result.code, Some(1), "{harness}: {}", result.stderr);
        assert!(
            result.stderr.contains("all publishing writes denied"),
            "{harness}: {}",
            result.stderr
        );
        let after = scenario.gh_state();
        for field in ["issues", "bodies", "titles", "comments", "prs"] {
            assert_eq!(after[field], before[field], "{harness}: {field}");
        }
    }
}

/// The staged spec path: the session returns its proposed public spec text
/// without running `gh issue create` itself. thirdshift validates every
/// staged text before creating any public issue.
fn stage_spec(record: &str) -> String {
    format!(
        r#"
printf 'Proposed public spec:\nTitle: Bound accepted input\nBody:\nBound input across storage and transport. Private record: {record}\nTicket: Bound storage input\nBody:\nBound storage input. Private record: {record}\nBlocked by: none\nTicket: Bound transport input\nBody:\nBound transport input. Private record: {record}\nBlocked by: 1\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
    )
}

fn implement_spec_ticket(ticket: u32, spec: u32) -> String {
    format!(
        r#"
echo bounded > bounded-{ticket}.txt
git add bounded-{ticket}.txt
git commit -q -m 'Bound accepted input'
gh pr create --base issue-{spec} --head issue-{ticket} --title 'Bound accepted input' --body 'Closes #{ticket}'
"#
    )
}

#[test]
fn unauthenticated_public_publishers_stage_single_fixes_and_specs_with_every_harness() {
    for harness in ["claude", "codex", "agy", "grok", "muse", "opencode"] {
        for spec in [false, true] {
            let scenario = with_reproduced_findings(&["high"]);
            let mut state = scenario.gh_state();
            if spec {
                state["advisories"][0]["description"] = json!(
                    state["advisories"][0]["description"]
                        .as_str()
                        .unwrap()
                        .replace("high single", "high spec")
                        .replace("Fix size: single", "Fix size: spec")
                );
                scenario.write_gh_state(&state);
            }
            let config = scenario.path("factory-gh");
            fs::create_dir_all(&config).unwrap();
            let credentials = "github.com:\n  oauth_token: factory-token\n";
            fs::write(config.join("hosts.yml"), credentials).unwrap();
            let proposal = if spec {
                stage_spec("https://github.com/acme/widgets/security/advisories/GHSA-finding-0")
            } else {
                stage_fix("GHSA-finding-0")
            };
            scenario.agent_does_in_session(
                1,
                &format!(
                    r#"
test -f README.md
git rev-parse --is-inside-work-tree
test -d "$GH_CONFIG_DIR"
test ! -f "$GH_CONFIG_DIR/hosts.yml"
if gh issue create --title Unvalidated --body Unvalidated 2> denied.txt; then exit 90; fi
grep -q 'authentication required' denied.txt
{proposal}
"#,
                ),
            );
            if spec {
                scenario.agent_does_for(9, &implement_spec_ticket(9, 8));
                scenario.agent_does_for(10, &implement_spec_ticket(10, 8));
            } else {
                scenario.agent_does_for(8, implement_fix());
            }
            let result = scenario.run_with_env(
                &[
                    "secure",
                    "security-fix",
                    "harness",
                    harness,
                    "parallel",
                    "1",
                ],
                &[
                    ("FAKE_GH_REQUIRE_AUTH", "1"),
                    ("GH_CONFIG_DIR", config.to_str().unwrap()),
                ],
            );
            assert_eq!(
                result.code,
                Some(0),
                "{harness}, spec={spec}: {}",
                result.stderr
            );
            let after = scenario.gh_state();
            assert_eq!(after["titles"]["8"], "Bound accepted input");
            if spec {
                assert_eq!(after["sub_issues"]["8"], json!([9, 10]));
                assert_eq!(after["blocked_by"]["10"], json!([9]));
            }
            assert!(
                after["prs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|pr| pr["head"] == "issue-8" && pr["isDraft"] == false)
            );
            let calls: Vec<Value> = serde_json::from_str(
                &fs::read_to_string(scenario.path(&format!("{harness}-calls.json"))).unwrap(),
            )
            .unwrap();
            let prompt = calls
                .iter()
                .filter_map(|call| call["prompt"].as_str())
                .find(|prompt| prompt.contains("Private record:"))
                .unwrap();
            assert!(prompt.contains(state["advisories"][0]["description"].as_str().unwrap()));
            assert_eq!(
                fs::read_to_string(config.join("hosts.yml")).unwrap(),
                credentials
            );
            assert!(scenario.entries("tmp").is_empty());
        }
    }
}

#[test]
fn public_publishing_resumes_still_cannot_write_github_content() {
    for (harness, killed) in [
        (
            "claude",
            r#"printf '%s\n' '{"type":"system","subtype":"task_notification","task_id":"pending","status":"stopped","summary":"bounded-fixture"}' > "$FAKE_CLAUDE_AFTER_RESULT""#,
        ),
        (
            "codex",
            r#"printf '%s\n' '{"type":"item.started","item":{"id":"pending","type":"command_execution","command":"bounded-fixture","status":"in_progress"}}'"#,
        ),
        (
            "muse",
            r#"printf '%s\n' '{"payload_type":"task.lifecycle.proposed","payload":{"event":{"task_id":"pending","task_kind":"tool.bash"}}}' '{"payload_type":"task.lifecycle.cancelled","payload":{"event":{"task_id":"pending","reason":"session ended"}}}'"#,
        ),
    ] {
        let scenario = with_reproduced_findings(&["high"]);
        let denied = r#"
if gh issue create --title Unvalidated --body Unvalidated 2> denied.txt; then exit 90; fi
grep -q 'authentication required' denied.txt
"#;
        scenario.agent_does_in_session(1, &format!("{denied}\n{killed}\n"));
        scenario.agent_does_in_session(2, &format!("{denied}\n{}", stage_fix("GHSA-finding-0")));
        scenario.agent_does_for(8, implement_fix());
        let result = scenario.run_with_env(
            &["secure", "security-fix", "harness", harness],
            &[("FAKE_GH_REQUIRE_AUTH", "1"), ("GH_TOKEN", "factory-token")],
        );
        assert_eq!(result.code, Some(0), "{harness}: {}", result.stderr);
        assert!(
            result.stderr.contains("resuming it once"),
            "{harness}: {}",
            result.stderr
        );
        assert_eq!(scenario.gh_state()["titles"]["8"], "Bound accepted input");
        assert!(scenario.gh_state()["bodies"].get("9").is_none());
        assert!(scenario.entries("tmp").is_empty());
    }
}

#[test]
fn a_bigger_public_fix_publishes_tickets_and_ends_as_the_spec_run() {
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    state["advisories"][0]["description"] = json!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .replace("high single", "high spec")
            .replace("Fix size: single", "Fix size: spec")
    );
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(
        1,
        &stage_spec("https://github.com/acme/widgets/security/advisories/GHSA-finding-0"),
    );
    scenario.agent_does_for(9, &implement_spec_ticket(9, 8));
    scenario.agent_does_for(
        10,
        &format!("test -f bounded-9.txt\n{}", implement_spec_ticket(10, 8)),
    );
    let result = scenario.run(&["secure", "security-fix", "parallel", "1"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let spec_pr = state["prs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pr| pr["head"] == "issue-8")
        .unwrap();
    assert_eq!(
        result.stdout,
        format!("{}\n", spec_pr["url"].as_str().unwrap())
    );
    assert_eq!(spec_pr["base"], "main");
    assert_eq!(spec_pr["state"], "OPEN");
    assert_eq!(spec_pr["isDraft"], false);
    for ticket in [9, 10] {
        assert_eq!(state["issues"][ticket.to_string()], "CLOSED");
        assert_eq!(
            state["labels"][ticket.to_string()],
            json!(["ready-for-agent", "security-fix"])
        );
    }
    assert_eq!(state["labels"]["8"], json!(["security-fix", "in-progress"]));
    // thirdshift created the Spec's native links from the staged text.
    assert_eq!(state["sub_issues"]["8"], json!([9, 10]));
    assert_eq!(state["blocked_by"]["10"], json!([9]));
    // The post-publish window sweep must leave the staged issues alone.
    assert_eq!(
        state["issues"]["8"], "OPEN",
        "the staged spec must survive the window sweep: {}",
        state["issues"]
    );
    assert!(
        !result.stderr.contains("outside the staged path"),
        "{}",
        result.stderr
    );
    let spec_body = state["bodies"]["8"].as_str().unwrap();
    assert!(spec_body.contains("storage and transport"), "{spec_body}");
    assert!(!spec_body.contains("Private candidate write-up"));
    let calls = scenario.claude_calls();
    assert!(
        calls[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("thirdshift-to-spec")
    );
    assert!(
        calls[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("thirdshift-to-tickets")
    );
    assert!(
        calls[0]["prompt"]
            .as_str()
            .unwrap()
            .contains("instead of creating any public issue directly"),
        "{}",
        calls[0]["prompt"].as_str().unwrap()
    );
    assert_eq!(calls.len(), 4);
}

#[test]
fn invalid_security_fix_specs_are_not_marked_ready_or_dispatched() {
    // A public spec has no session-created path: every entry here is either
    // a missing staged spec or staged text that validation rejects before
    // thirdshift creates anything.
    let record = "https://github.com/acme/widgets/security/advisories/GHSA-finding-0";
    let staged = stage_spec(record);
    for script in [
        // No staged spec at all.
        "printf '%s\\n' 'No protocol block' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n".to_string(),
        // A session-created URL final line stages nothing.
        "printf 'Security fix Spec: https://github.com/acme/widgets/issues/8\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n".to_string(),
        // The top issue copies private write-up text.
        staged.replace(
            "Bound input across storage and transport. Private record:",
            "Private candidate write-up. Private record:",
        ),
        // A Ticket copies private write-up text.
        staged.replace(
            "Bound storage input. Private record:",
            "Private candidate write-up. Private record:",
        ),
        // The top issue omits the private record link.
        staged.replace(
            "Bound input across storage and transport. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0",
            "Bound input across storage and transport.",
        ),
        // A Ticket omits the private record link.
        staged.replace(
            "Bound transport input. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0",
            "Bound transport input.",
        ),
        // A Ticket blocked by itself.
        staged.replace("Blocked by: 1", "Blocked by: 2"),
        // A Ticket blocked by no such Ticket.
        staged.replace("Blocked by: 1", "Blocked by: 3"),
    ] {
        let scenario = with_reproduced_findings(&["high"]);
        let mut state = scenario.gh_state();
        state["advisories"][0]["description"] = json!(state["advisories"][0]["description"].as_str().unwrap().replace("high single", "high spec"));
        scenario.write_gh_state(&state);
        scenario.agent_does_in_session(1, &script);
        let result = scenario.run(&["secure", "security-fix"]);
        assert_eq!(result.code, Some(1), "{script}: {}", result.stderr);
        assert!(
            scenario.gh_state()["bodies"].get("8").is_none(),
            "{script}: a rejected staged spec must not create a public issue: {}",
            scenario.gh_state()["bodies"]
        );
        assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
        assert_eq!(scenario.claude_calls().len(), 1);
    }
}

#[test]
fn passing_window_issues_stay_open_and_are_named_in_activity_in_both_sweeps() {
    for spec in [false, true] {
        let scenario = with_reproduced_findings(&["high"]);
        let record = "https://github.com/acme/widgets/security/advisories/GHSA-finding-0";
        if spec {
            let mut state = scenario.gh_state();
            state["advisories"][0]["description"] = json!(
                state["advisories"][0]["description"]
                    .as_str()
                    .unwrap()
                    .replace("high single", "high spec")
                    .replace("Fix size: single", "Fix size: spec")
            );
            scenario.write_gh_state(&state);
            scenario.agent_does_for(10, &implement_spec_ticket(10, 9));
            scenario.agent_does_for(11, &implement_spec_ticket(11, 9));
        } else {
            scenario.agent_does_for(
                9,
                &implement_fix()
                    .replace("issue-8", "issue-9")
                    .replace("#8", "#9"),
            );
        }
        let staged = if spec {
            stage_spec(record)
        } else {
            stage_fix("GHSA-finding-0")
        };
        scenario.agent_does_in_session(
            1,
            &format!(
                "{staged}\ngh issue create --title 'Other work' --body 'Separate change. Private record: {record}' > /dev/null\n"
            ),
        );
        let result = scenario.run(&["secure", "security-fix", "parallel", "1"]);
        assert_eq!(result.code, Some(0), "spec={spec}: {}", result.stderr);
        assert_eq!(scenario.gh_state()["issues"]["8"], "OPEN");
        let activity =
            fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
                .unwrap();
        assert_eq!(
            activity
                .matches("Security fix publication left https://github.com/acme/widgets/issues/8 open: public-text check passed")
                .count(),
            if spec { 2 } else { 1 },
            "{activity}"
        );
    }
}

#[test]
fn a_spec_session_side_effect_public_issue_fails_the_fix_and_is_closed() {
    // The staged spec path validates before creating: a session that stages
    // valid spec text but also creates a public issue on the side must fail
    // loudly, and thirdshift closes the leak instead of reporting success.
    let record = "https://github.com/acme/widgets/security/advisories/GHSA-finding-0";
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    state["advisories"][0]["description"] = json!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .replace("high single", "high spec")
    );
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(
        1,
        &format!(
            "{}\ngh issue create --title 'Leaked write-up' --body 'Private candidate write-up. Private record: {record}' --label needs-triage > /dev/null\n",
            stage_spec(record),
        ),
    );
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("created public issue(s) outside the staged path: #8"),
        "{}",
        result.stderr
    );
    let state = scenario.gh_state();
    assert_eq!(
        state["issues"]["8"], "CLOSED",
        "the session-created side-effect issue must be closed: {}",
        state["issues"]
    );
    assert!(
        state["bodies"].get("9").is_none(),
        "no validated issue may follow a side-effect leak: {}",
        state["bodies"]
    );
    assert!(state["prs"].as_array().unwrap().is_empty());
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn a_staged_spec_with_cyclic_blocking_edges_is_rejected_before_creating_anything() {
    // Ticket 1 names a later Ticket as its blocker and vice versa: no
    // execution order satisfies both edges, so validation must reject the
    // staged text before thirdshift creates any public issue.
    let record = "https://github.com/acme/widgets/security/advisories/GHSA-finding-0";
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    state["advisories"][0]["description"] = json!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .replace("high single", "high spec")
    );
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(
        1,
        &format!(
            "printf 'Proposed public spec:\\nTitle: Bound accepted input\\nBody:\\nBound input across storage and transport. Private record: {record}\\nTicket: Bound storage input\\nBody:\\nBound storage input. Private record: {record}\\nBlocked by: 2\\nTicket: Bound transport input\\nBody:\\nBound transport input. Private record: {record}\\nBlocked by: 1\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
        ),
    );
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(
        result.code,
        Some(1),
        "cyclic staged spec must be rejected: {}",
        result.stderr
    );
    assert!(
        scenario.gh_state()["bodies"].get("8").is_none(),
        "a rejected staged spec must not create a public issue: {}",
        scenario.gh_state()["bodies"]
    );
}

#[test]
fn fixing_selects_the_most_severe_record_then_dispatches_one_ticket_before_auditing() {
    let scenario = with_reproduced_findings(&["low", "critical", "critical", "high"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-1"));
    scenario.agent_does_for(8, implement_fix());
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert!(
        result
            .stderr
            .ends_with("PR https://github.com/acme/widgets/pull/1 is ready for review\n"),
        "{}",
        result.stderr
    );
    let state = scenario.gh_state();
    assert_eq!(state["labels"]["8"], json!(["security-fix", "in-progress"]));
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(prompt.contains("GHSA-finding-1"), "{prompt}");
    assert!(!prompt.contains("GHSA-finding-2"));
    assert!(!prompt.contains("Security audit: complete"));
    let ticket = state["bodies"]["8"].as_str().unwrap();
    assert!(ticket.contains("GHSA-finding-1"));
    assert!(!ticket.contains("Private candidate write-up"));
    assert!(
        state["advisories"][1]["description"]
            .as_str()
            .unwrap()
            .contains("https://github.com/acme/widgets/issues/8")
    );
    let shown = scenario.run(&["show", "https://github.com/acme/widgets/issues/8", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(
        shown["command"]["security_fix_findings"],
        json!([{
            "title": "Finding 1", "severity": "critical",
            "url": "https://github.com/acme/widgets/security/advisories/GHSA-finding-1"
        }])
    );
}

#[test]
fn an_audit_then_fix_notification_marks_the_fixed_finding_apart_from_those_waiting() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    let mut other = finding("other-input");
    other["title"] = json!("Unchecked header size");
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("input-size"), other]).to_string()),
    );
    scenario.agent_does_in_session(2, &reproduction_script("reproduced high single"));
    scenario.agent_does_in_session(3, &reproduction_script("reproduced low single"));
    scenario.agent_does_in_session(4, &stage_fix("GHSA-test-test-0001"));
    scenario.agent_does_for(8, implement_fix());
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result = scenario.run_with_env(
        &["secure", "security-fix", "email", "me@example.com"],
        &resend_env(&resend),
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let (_, text) = the_one_notification(&resend);
    assert!(
        text.contains(
            "Audit:        Security audit recorded 2 new finding(s), 0 already recorded\n\
             \n\
             Addressed by the pull request:\n\
             - high: Unchecked input size: https://github.com/acme/widgets/security/advisories/GHSA-test-test-0001\n\
             \n\
             Recorded, waiting for the Day shift:\n\
             - low: Unchecked header size: https://github.com/acme/widgets/security/advisories/GHSA-test-test-0002\n\
             \n\
             Pull request: https://github.com/acme/widgets/pull/1\n"
        ),
        "{text}"
    );
    assert!(!text.contains("Security findings:"), "{text}");
    // The Command's notification retains its completion-time account, while
    // the Inbox now sees both graded advisories as triaged on GitHub.
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let audit = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "Security run")
        .unwrap();
    assert_eq!(audit["left_reason"], "findings triaged", "{status}");
    // Removing the other record's severity makes it wait again, independently
    // of the fix's pull request and its completion-time notification.
    let mut gh = scenario.gh_state();
    gh["advisories"][1]["severity"] = Value::Null;
    scenario.write_gh_state(&gh);
    fs::remove_file(scenario.path("home/.thirdshift/logs/github-inbox.json")).unwrap();
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(
        status["commands"][0]["needs_you_kind"], "finding",
        "{status}"
    );
}

#[test]
fn an_audit_then_fix_that_leaves_nothing_waiting_flags_no_finding() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("input-size")]).to_string()),
    );
    scenario.agent_does_in_session(2, &reproduction_script("reproduced high single"));
    scenario.agent_does_in_session(3, &stage_fix("GHSA-test-test-0001"));
    scenario.agent_does_for(8, implement_fix());
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_ne!(
        status["commands"][0]["needs_you_kind"], "finding",
        "{status}"
    );
}

#[test]
fn an_audit_then_fix_notification_lists_a_graded_finding_apart_from_those_waiting() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    let mut github = scenario.gh_state();
    github["advisories"] = json!([{
        "ghsa_id": "GHSA-graded", "state": "draft", "severity": "medium",
        "summary": "Graded finding", "description": "Fingerprint: `graded`\nPrivate record.\n",
        "html_url": "https://github.com/acme/widgets/security/advisories/GHSA-graded"
    }]);
    scenario.write_gh_state(&github);
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("graded"), finding("input-size")]).to_string()),
    );
    scenario.agent_does_in_session(2, &reproduction_script("reproduced high single"));
    scenario.agent_does_in_session(3, &stage_fix("GHSA-test-test-0002"));
    scenario.agent_does_for(8, implement_fix());
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result = scenario.run_with_env(
        &["secure", "security-fix", "email", "me@example.com"],
        &resend_env(&resend),
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let (_, text) = the_one_notification(&resend);
    assert!(
        text.contains(
            "Audit:        Security audit recorded 1 new finding(s), 1 already recorded\n\
             \n\
             Addressed by the pull request:\n\
             - high: Unchecked input size: https://github.com/acme/widgets/security/advisories/GHSA-test-test-0002\n\
             \n\
             Also recorded by the audit:\n\
             - medium: Graded finding: https://github.com/acme/widgets/security/advisories/GHSA-graded\n\
             \n\
             Pull request: https://github.com/acme/widgets/pull/1\n"
        ),
        "{text}"
    );
    assert!(!text.contains("Recorded, waiting"), "{text}");
}

#[test]
fn a_fix_without_an_audit_notification_says_the_audit_did_not_run() {
    let scenario = with_reproduced_findings(&["low", "critical"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-1"));
    scenario.agent_does_for(8, implement_fix());
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result = scenario.run_with_env(
        &["secure", "security-fix", "email", "me@example.com"],
        &resend_env(&resend),
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let (_, text) = the_one_notification(&resend);
    assert!(
        text.contains(
            "Audit:        not run\n\
             \n\
             Addressed by the pull request:\n\
             - critical: Finding 1: https://github.com/acme/widgets/security/advisories/GHSA-finding-1\n\
             \n\
             Pull request: https://github.com/acme/widgets/pull/1\n"
        ),
        "{text}"
    );
    assert!(!text.contains("Finding 0"), "{text}");
    assert!(!text.contains("Recorded, waiting"), "{text}");
}

#[test]
fn a_failed_fix_notification_marks_its_finding_attempted_not_fixed() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, stage_private_title());
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result = scenario.run_with_env(
        &["secure", "security-fix", "email", "me@example.com"],
        &resend_env(&resend),
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(subject, "[thirdshift] acme/widgets Security run: failed");
    assert!(
        text.contains(
            "Audit:        not run\n\
             \n\
             Fix attempted, not fixed:\n\
             - high: Finding 0: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\n"
        ),
        "{text}"
    );
    assert!(!text.contains("Addressed by"), "{text}");
}

#[test]
fn a_staged_ticket_title_cannot_copy_the_private_write_up() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, stage_private_title());
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("includes private write-up text"),
        "{}",
        result.stderr
    );
    let state = scenario.gh_state();
    assert!(
        state["bodies"].get("8").is_none(),
        "a rejected staged title must not create a public issue: {}",
        state["bodies"]
    );
    assert!(state["prs"].as_array().unwrap().is_empty());
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn a_session_created_ticket_url_is_rejected_for_a_public_single_fix() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, url_line_only());
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("did not stage its proposed public issue text"),
        "{}",
        result.stderr
    );
    let state = scenario.gh_state();
    assert!(
        state["bodies"].get("8").is_none(),
        "a session-created URL must not make thirdshift create a public issue: {}",
        state["bodies"]
    );
    assert!(state["prs"].as_array().unwrap().is_empty());
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn a_session_side_effect_public_issue_fails_the_fix_and_is_closed() {
    // Ticket #629: the publishing session cannot create public issues on its
    // own. A session that stages valid text but also creates a public issue
    // on the side must fail loudly, and thirdshift closes the leak for
    // Day-shift triage instead of reporting success. Failing closed rather
    // than succeeding with one issue: stopping the session's `gh issue
    // create` itself is now denied by the unauthenticated launch policy.
    // This fixture deliberately leaves fake authentication off to exercise
    // the second check when an issue nevertheless appears in that window.
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(
        1,
        &format!(
            "{}\ngh issue create --title 'Leaked write-up' --body 'Private candidate write-up. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0' --label needs-triage > /dev/null\n",
            stage_fix("GHSA-finding-0"),
        ),
    );
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("created public issue(s) outside the staged path: #8"),
        "{}",
        result.stderr
    );
    let state = scenario.gh_state();
    assert_eq!(
        state["issues"]["8"], "CLOSED",
        "the session-created side-effect issue must be closed: {}",
        state["issues"]
    );
    assert!(
        state["bodies"].get("9").is_none(),
        "no validated issue may follow a side-effect leak: {}",
        state["bodies"]
    );
    assert!(state["prs"].as_array().unwrap().is_empty());
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn a_staged_proposal_is_created_by_thirdshift_not_the_session() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, implement_fix());
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let state = scenario.gh_state();
    let ticket = state["bodies"]["8"].as_str().unwrap();
    assert!(ticket.contains("Reject oversized input."), "{ticket}");
    assert!(ticket.contains("GHSA-finding-0"), "{ticket}");
    assert!(!ticket.contains("Private candidate write-up"), "{ticket}");
    assert_eq!(state["labels"]["8"], json!(["security-fix", "in-progress"]));
    assert!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .contains("https://github.com/acme/widgets/issues/8")
    );
    let calls = scenario.claude_calls();
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("instead of creating any public issue directly"),
        "{prompt}"
    );
}

#[test]
fn a_staged_proposal_is_validated_before_any_public_issue_is_created() {
    for staged in [
        // Copies private write-up text.
        "printf 'Proposed public issue:\\nTitle: Bound accepted input\\nBody:\\nPrivate candidate write-up. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
        // Omits the private record link.
        "printf 'Proposed public issue:\\nTitle: Bound accepted input\\nBody:\\nReject oversized input.\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
    ] {
        let scenario = with_reproduced_findings(&["high"]);
        scenario.agent_does_in_session(1, staged);
        let result = scenario.run(&["secure", "security-fix"]);
        assert_eq!(result.code, Some(1), "{staged}: {}", result.stderr);
        let state = scenario.gh_state();
        assert!(
            state["bodies"].get("8").is_none(),
            "{staged}: a rejected staged proposal must not create a public issue: {}",
            state["bodies"]
        );
        assert!(state["prs"].as_array().unwrap().is_empty());
        assert_eq!(scenario.claude_calls().len(), 1);
    }
}

#[test]
fn a_dispatched_fix_holds_off_no_audit_and_is_not_dispatched_again() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, implement_fix());
    let fixed = scenario.run(&["secure", "security-fix"]);
    assert_eq!(fixed.code, Some(0), "{}", fixed.stderr);
    let issues = scenario.gh_state()["issues"].clone();
    // Its fix Ticket is still open, and its finding still waits for the Day shift.
    scenario.agent_does(&audit_script(&json!([finding("finding-0")]).to_string()));
    for args in [vec!["secure"], vec!["secure", "security-fix"]] {
        scenario.origin_has_commit(
            "main",
            &format!("{}.txt", args.len()),
            "new work",
            "Move Base branch",
        );
        let audited = scenario.run(&args);
        assert_eq!(audited.code, Some(0), "{args:?}: {}", audited.stderr);
        assert!(
            audited
                .stderr
                .contains("0 new finding(s), 1 already recorded"),
            "{args:?}: {}",
            audited.stderr
        );
    }
    assert_eq!(scenario.claude_calls().len(), 4);
    assert_eq!(scenario.gh_state()["issues"], issues);
}

#[test]
fn reproduced_findings_without_permission_are_not_fixed_and_hold_off_no_audit() {
    for config in ["", "[security]\nfix = false\n", "[security]\nfix = true\n"] {
        let scenario = with_reproduced_findings(&["high"]);
        scenario.user_config_is(config);
        scenario.agent_does(&audit_script("[]"));
        let args = if config.contains("true") {
            vec!["secure", "no-security-fix"]
        } else {
            vec!["secure"]
        };
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            result
                .stderr
                .contains("0 new finding(s), 0 already recorded"),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.claude_calls().len(), 1);
        assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
    }
}

#[test]
fn a_private_one_session_fix_reuses_the_findings_issue_and_preserves_its_evidence() {
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    state["private"] = json!(true);
    state["bodies"]["7"] = state["advisories"][0]["description"].clone();
    state["labels"]["7"] = json!(["security-finding", "needs-triage", "bug"]);
    let evidence = state["bodies"]["7"].as_str().unwrap().to_string();
    scenario.write_gh_state(&state);
    scenario.agent_does_for(
        7,
        &implement_fix()
            .replace("issue-8", "issue-7")
            .replace("#8", "#7"),
    );
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["issues"].as_object().unwrap().len(), 1);
    assert!(
        state["bodies"]["7"]
            .as_str()
            .unwrap()
            .starts_with(&evidence)
    );
    assert!(
        state["bodies"]["7"]
            .as_str()
            .unwrap()
            .contains("Fix Ticket: https://github.com/acme/widgets/issues/7")
    );
    assert_eq!(
        state["labels"]["7"],
        json!(["security-finding", "bug", "security-fix", "in-progress"])
    );
    assert_eq!(state["prs"][0]["head"], "issue-7");
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn a_bigger_private_fix_adds_tickets_to_the_findings_issue() {
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    state["private"] = json!(true);
    state["bodies"]["7"] = json!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .replace("high single", "high spec")
            .replace("Fix size: single", "Fix size: spec")
    );
    state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
    state["blocked_by"]["9"] = json!([8]);
    let evidence = state["bodies"]["7"].as_str().unwrap().to_string();
    scenario.write_gh_state(&state);
    scenario.agent_does_for_in_session(7, 1, r#"
gh issue create --title 'Bound storage input' --body 'Bound storage input. Private record: https://github.com/acme/widgets/issues/7' --label ready-for-agent
gh issue create --title 'Bound transport input' --body 'Bound transport input. Private record: https://github.com/acme/widgets/issues/7' --label ready-for-agent
gh fake sub-issues 7 '[8,9]'
printf '%s\n' 'Security fix Spec: https://github.com/acme/widgets/issues/7' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    scenario.agent_does_for_in_session(7, 2, "true");
    scenario.agent_does_for(8, &implement_spec_ticket(8, 7));
    scenario.agent_does_for(
        9,
        &format!("test -f bounded-8.txt\n{}", implement_spec_ticket(9, 7)),
    );
    let result = scenario.run_with_env(
        &["secure", "security-fix", "merge", "parallel", "1"],
        &[("FAKE_GH_REQUIRE_AUTH", "1"), ("GH_TOKEN", "factory-token")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["issues"].as_object().unwrap().len(), 3);
    assert_eq!(state["sub_issues"]["7"], json!([8, 9]));
    assert!(
        state["bodies"]["7"]
            .as_str()
            .unwrap()
            .starts_with(&evidence)
    );
    assert!(
        state["bodies"]["7"]
            .as_str()
            .unwrap()
            .contains("Fix Ticket: https://github.com/acme/widgets/issues/7")
    );
    assert_eq!(state["issues"]["7"], "CLOSED");
    let spec_pr = state["prs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pr| pr["head"] == "issue-7")
        .unwrap();
    assert_eq!(spec_pr["state"], "MERGED");
    assert_eq!(spec_pr["base"], "main");
    assert_eq!(
        result.stdout,
        format!("{}\n", spec_pr["url"].as_str().unwrap())
    );
    for ticket in [8, 9] {
        assert!(
            state["bodies"][ticket.to_string()]
                .as_str()
                .unwrap()
                .contains("https://github.com/acme/widgets/issues/7")
        );
        assert_eq!(
            state["labels"][ticket.to_string()],
            json!(["ready-for-agent", "security-fix"])
        );
    }
}

#[test]
fn the_setting_allows_a_merge_fix_and_the_command_can_override_either_setting() {
    for (config, word, merged) in [
        (
            "[security]\nfix = true\n[merge]\nalways = true\n",
            None,
            true,
        ),
        ("[security]\nfix = false\n", Some("security-fix"), false),
        (
            "[security]\nfix = true\n[merge]\nalways = true\n",
            Some("no-merge"),
            false,
        ),
    ] {
        let scenario = with_reproduced_findings(&["medium"]);
        scenario.user_config_is(config);
        scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
        scenario.agent_does_for(8, implement_fix());
        scenario.agent_does_in_session(3, &close_check_script("passes"));
        let mut args = vec!["secure"];
        args.extend(word);
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            scenario.gh_state()["prs"][0]["state"],
            if merged { "MERGED" } else { "OPEN" }
        );
        assert_eq!(scenario.claude_calls().len(), if merged { 3 } else { 2 });
    }
}

/// The close check's scripted session: it saw the private record, then its
/// proof-of-concept run ended as `outcome` (`passes` or `fails: <reason>`).
fn close_check_script(outcome: &str) -> String {
    format!(
        r#"
printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'finding-0'
printf '%s\n' 'Command: bounded-fixture; exit 0' 'Security close check: {outcome}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
    )
}

#[test]
fn a_merged_fix_closes_the_advisory_by_default_naming_its_pull_request() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, implement_fix());
    scenario.agent_does_in_session(3, &close_check_script("passes"));
    let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
    let result = scenario.run_with_env(
        &[
            "secure",
            "security-fix",
            "merge",
            "email",
            "day@example.com",
        ],
        &resend_env(&resend),
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let pr = &state["prs"][0];
    assert_eq!(pr["state"], "MERGED");
    let pr_url = pr["url"].as_str().unwrap();
    let record = &state["advisories"][0];
    assert_eq!(record["state"], "closed");
    let body = record["description"].as_str().unwrap();
    assert!(body.contains("Fix Run: succeeded"), "{body}");
    assert!(body.contains(&format!("Closed by: {pr_url}")), "{body}");
    assert!(
        result
            .stderr
            .contains(&format!("closed GHSA-finding-0 by {pr_url}")),
        "{}",
        result.stderr
    );
    assert_eq!(result.stdout, format!("{pr_url}\n"));
    let (subject, body) = the_one_notification(&resend);
    assert_eq!(subject, "[thirdshift] acme/widgets Security run: merged");
    assert!(
        body.contains(&format!("Closed: Finding 0 by {pr_url}")),
        "{body}"
    );
    assert!(!body.contains("Private candidate write-up."), "{body}");
}

#[test]
fn a_merged_fix_closes_the_finding_issue_by_default_naming_its_pull_request() {
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    state["private"] = json!(true);
    state["bodies"]["7"] = state["advisories"][0]["description"].clone();
    state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
    scenario.write_gh_state(&state);
    scenario.agent_does_for(
        7,
        &implement_fix()
            .replace("issue-8", "issue-7")
            .replace("#8", "#7"),
    );
    scenario.agent_does_in_session(2, &close_check_script("passes"));
    let result = scenario.run(&["secure", "security-fix", "merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let pr_url = state["prs"][0]["url"].as_str().unwrap().to_string();
    // The merged fix already closed its issue: the close still names the
    // pull request in the record and in a comment.
    assert_eq!(state["issues"]["7"], "CLOSED");
    let body = state["bodies"]["7"].as_str().unwrap();
    assert!(body.contains(&format!("Closed by: {pr_url}")), "{body}");
    assert!(
        state["comments"]["7"].to_string().contains(&pr_url),
        "{}",
        state["comments"]["7"]
    );
    assert!(
        result
            .stderr
            .contains(&format!("closed issue #7 by {pr_url}")),
        "{}",
        result.stderr
    );
}

#[test]
fn a_failing_close_check_leaves_the_advisory_open_with_a_note_and_tells_the_day_shift() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, implement_fix());
    scenario.agent_does_in_session(3, &close_check_script("fails: the bound still overflows"));
    let result = scenario.run(&["secure", "security-fix", "merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "MERGED");
    let record = &state["advisories"][0];
    assert_eq!(record["state"], "draft");
    let body = record["description"].as_str().unwrap();
    assert!(
        body.contains("Security close left open: the proof-of-concept test did not pass"),
        "{body}"
    );
    assert!(!body.contains("Closed by:"), "{body}");
    assert!(
        result.stderr.contains("left GHSA-finding-0 open"),
        "{}",
        result.stderr
    );
    assert_eq!(
        result.stdout,
        format!("{}\n", state["prs"][0]["url"].as_str().unwrap())
    );
}

#[test]
fn email_unless_merged_still_emails_a_merged_fix_that_left_its_finding_open() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, implement_fix());
    scenario.agent_does_in_session(3, &close_check_script("fails: the bound still overflows"));
    let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
    let result = scenario.run_with_env(
        &[
            "secure",
            "security-fix",
            "security-close",
            "merge",
            "email-unless-merged",
            "day@example.com",
        ],
        &resend_env(&resend),
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    let (subject, body) = the_one_notification(&resend);
    assert_eq!(subject, "[thirdshift] acme/widgets Security run: merged");
    assert!(body.contains("open"), "{body}");
}

#[test]
fn email_unless_merged_emails_nothing_for_a_merged_fix_that_closed_its_finding() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, implement_fix());
    scenario.agent_does_in_session(3, &close_check_script("passes"));
    let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
    let result = scenario.run_with_env(
        &[
            "secure",
            "security-fix",
            "security-close",
            "merge",
            "email-unless-merged",
            "day@example.com",
        ],
        &resend_env(&resend),
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["advisories"][0]["state"], "closed");
    assert!(resend.requests().is_empty(), "{:?}", resend.requests());
}

#[test]
fn closing_follows_the_setting_unless_the_command_overrides_it() {
    for (config, words, merged, closed) in [
        ("", vec!["security-fix", "merge"], true, true),
        (
            "[security]\nfix = true\n[merge]\nalways = true\n",
            Vec::new(),
            true,
            true,
        ),
        (
            "[security]\nfix = true\nclose = false\n[merge]\nalways = true\n",
            Vec::new(),
            true,
            false,
        ),
        (
            "[security]\nfix = true\n[merge]\nalways = true\n",
            vec!["no-security-close"],
            true,
            false,
        ),
        (
            "[security]\nfix = true\nclose = false\n[merge]\nalways = true\n",
            vec!["security-close"],
            true,
            true,
        ),
        (
            "[security]\nfix = true\nclose = true\n[merge]\nalways = true\n",
            Vec::new(),
            true,
            true,
        ),
        (
            "[security]\nfix = true\nclose = true\n[merge]\nalways = true\n",
            vec!["no-security-close"],
            true,
            false,
        ),
        (
            "[security]\nfix = true\n[merge]\nalways = true\n",
            vec!["security-close"],
            true,
            true,
        ),
        (
            "[security]\nfix = true\nclose = true\n",
            Vec::new(),
            false,
            false,
        ),
    ] {
        let scenario = with_reproduced_findings(&["medium"]);
        scenario.user_config_is(config);
        scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
        scenario.agent_does_for(8, implement_fix());
        scenario.agent_does_in_session(3, &close_check_script("passes"));
        let mut args = vec!["secure"];
        args.extend(words.clone());
        let result = scenario.run(&args);
        assert_eq!(
            result.code,
            Some(0),
            "{config:?} {words:?}: {}",
            result.stderr
        );
        let state = scenario.gh_state();
        assert_eq!(
            state["prs"][0]["state"].as_str().unwrap(),
            if merged { "MERGED" } else { "OPEN" },
            "{config:?} {words:?}"
        );
        assert_eq!(
            state["advisories"][0]["state"].as_str().unwrap(),
            if closed { "closed" } else { "draft" },
            "{config:?} {words:?}"
        );
        assert_eq!(scenario.claude_calls().len(), if closed { 3 } else { 2 });
    }
}

#[test]
fn an_audit_goes_on_to_one_reproduced_fix_when_fixing_is_allowed() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("first"), finding("second")]).to_string()),
    );
    scenario.agent_does_in_session(2, &reproduction_script("reproduced high single"));
    scenario.agent_does_in_session(3, &reproduction_script("reproduced critical spec"));
    scenario.agent_does_in_session(
        4,
        &stage_spec("https://github.com/acme/widgets/security/advisories/GHSA-test-test-0002"),
    );
    scenario.agent_does_for(9, &implement_spec_ticket(9, 8));
    scenario.agent_does_for(10, &implement_spec_ticket(10, 8));
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let spec_pr = state["prs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pr| pr["head"] == "issue-8")
        .unwrap();
    assert_eq!(
        result.stdout,
        format!("{}\n", spec_pr["url"].as_str().unwrap())
    );
    assert_eq!(scenario.claude_calls().len(), 7);
    assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 4);
    assert!(
        scenario.gh_state()["bodies"]["8"]
            .as_str()
            .unwrap()
            .contains("GHSA-test-test-0002")
    );
}

#[test]
fn the_security_run_ends_as_a_failed_fix_and_sends_one_private_metadata_notification() {
    let scenario = with_reproduced_findings(&["critical"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, &format!("{}exit 1\n", implement_fix()));
    let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
    let result = scenario.run_with_env(
        &["secure", "security-fix", "email", "day@example.com"],
        &resend_env(&resend),
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("claude exited 1"),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario.gh_state()["labels"]["8"],
        json!(["security-fix", "in-progress"])
    );
    let (subject, body) = the_one_notification(&resend);
    assert_eq!(subject, "[thirdshift] acme/widgets Security run: failed");
    assert!(body.contains("critical"));
    assert!(body.contains("GHSA-finding-0"));
    assert!(!body.contains("Private candidate write-up"));
    assert!(!body.contains("bounded_fixture"));
}

#[test]
fn a_failed_fix_keeps_its_claim_and_pauses_security_and_pickup_until_closed() {
    for private in [false, true] {
        for pushed in [false, true] {
            let scenario = with_reproduced_findings(&["critical"]);
            let issue = if private { 7 } else { 8 };
            if private {
                let mut state = scenario.gh_state();
                state["private"] = json!(true);
                state["bodies"]["7"] = state["advisories"][0]["description"].clone();
                state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
                scenario.write_gh_state(&state);
            } else {
                scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
            }
            scenario.agent_does_for(
                issue,
                &format!(
                    "{}exit 1\n",
                    if pushed { implement_fix() } else { "" }
                        .replace("issue-8", &format!("issue-{issue}"))
                        .replace("#8", &format!("#{issue}"))
                ),
            );
            let failed = scenario.run(&["secure", "security-fix"]);
            assert_eq!(failed.code, Some(1), "{}", failed.stderr);
            assert_eq!(
                scenario.issue_labels(issue),
                if private {
                    vec!["security-finding", "security-fix", "in-progress"]
                } else {
                    vec!["security-fix", "in-progress"]
                }
            );
            // Even contradictory ready labelling cannot make Pickup retry a Claim.
            scenario.issue_labelled(
                issue,
                if private {
                    &[
                        "security-finding",
                        "security-fix",
                        "in-progress",
                        "ready-for-agent",
                    ]
                } else {
                    &["security-fix", "in-progress", "ready-for-agent"]
                },
            );
            let reason = format!("failed Security fix #{issue} is still open");
            let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
            for args in [
                vec!["secure", "security-fix", "email", "day@example.com"],
                vec!["secure", "no-security-fix", "email", "day@example.com"],
                vec!["pickup"],
            ] {
                let skipped = scenario.run_with_env(&args, &resend_env(&resend));
                assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
                assert!(skipped.stdout.is_empty());
                if args[0] == "secure" {
                    assert!(skipped.stderr.contains(&reason), "{}", skipped.stderr);
                }
            }
            let fix_sessions = if private { 1 } else { 2 };
            assert_eq!(scenario.claude_calls().len(), fix_sessions);
            assert!(resend.requests().is_empty());
            assert_eq!(
                scenario
                    .log_files("home/.thirdshift/logs/acme/widgets/commands/secure", "log")
                    .len(),
                1
            );
            let activity = fs::read_to_string(
                scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"),
            )
            .unwrap();
            assert!(
                activity
                    .matches(&format!("Security run skipped: {reason}"))
                    .count()
                    == 1,
                "{activity}"
            );
            let mut state = scenario.gh_state();
            state["issues"][issue.to_string()] = json!("CLOSED");
            scenario.write_gh_state(&state);
            scenario.agent_does(&audit_script("[]"));
            let after = scenario.run(&["secure", "security-fix"]);
            assert_eq!(after.code, Some(0), "{}", after.stderr);
            assert!(
                after
                    .stderr
                    .contains("Security audit recorded 0 new finding(s)"),
                "{}",
                after.stderr
            );
            assert_eq!(scenario.claude_calls().len(), fix_sessions + 1);
        }
    }
}

#[test]
fn a_failed_spec_fix_keeps_its_claim_and_pauses_security_and_pickup_until_closed() {
    for private in [false, true] {
        let scenario = with_reproduced_findings(&["high"]);
        let mut state = scenario.gh_state();
        let evidence = state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .replace("high single", "high spec")
            .replace("Fix size: single", "Fix size: spec");
        let (spec, ticket) = if private { (7, 8) } else { (8, 9) };
        if private {
            state["private"] = json!(true);
            state["bodies"]["7"] = json!(evidence);
            state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
            scenario.agent_does_for_in_session(7, 1, r#"
gh issue create --title 'Bound storage input' --body 'Bound storage input. Private record: https://github.com/acme/widgets/issues/7' --label ready-for-agent
gh issue create --title 'Bound transport input' --body 'Bound transport input. Private record: https://github.com/acme/widgets/issues/7' --label ready-for-agent
gh fake sub-issues 7 '[8,9]'
printf '%s\n' 'Security fix Spec: https://github.com/acme/widgets/issues/7' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
        } else {
            state["advisories"][0]["description"] = json!(evidence);
            scenario.agent_does_in_session(
                1,
                &stage_spec("https://github.com/acme/widgets/security/advisories/GHSA-finding-0"),
            );
        }
        if private {
            // The private session links its own Tickets; the public staged
            // path has thirdshift create the blocking edge itself.
            state["blocked_by"][(ticket + 1).to_string()] = json!([ticket]);
        }
        scenario.write_gh_state(&state);
        scenario.agent_does_for(ticket, "exit 1\n");
        let failed = scenario.run(&["secure", "security-fix", "parallel", "1"]);
        assert_eq!(failed.code, Some(1), "{}", failed.stderr);
        assert!(
            failed.stderr.contains("claude exited 1"),
            "{}",
            failed.stderr
        );
        assert_eq!(
            scenario.issue_labels(spec),
            if private {
                vec!["security-finding", "security-fix", "in-progress"]
            } else {
                vec!["security-fix", "in-progress"]
            }
        );
        let state = scenario.gh_state();
        assert_eq!(state["issues"][spec.to_string()], "OPEN");
        assert_eq!(state["issues"][ticket.to_string()], "OPEN");
        let description = if private {
            &state["bodies"]["7"]
        } else {
            &state["advisories"][0]["description"]
        }
        .as_str()
        .unwrap();
        assert!(description.starts_with(&evidence), "{description}");
        assert!(
            description.contains(&format!(
                "Fix Ticket: https://github.com/acme/widgets/issues/{spec}"
            )),
            "{description}"
        );
        assert!(description.ends_with("Fix Run: failed\n"), "{description}");
        assert_eq!(scenario.claude_calls().len(), 2);
        let reason = format!("failed Security fix #{spec} is still open");
        for permission in ["security-fix", "no-security-fix"] {
            let skipped = scenario.run(&["secure", permission]);
            assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
            assert!(skipped.stderr.contains(&reason), "{}", skipped.stderr);
        }
        let pickup = scenario.run(&["pickup"]);
        assert_eq!(pickup.code, Some(0), "{}", pickup.stderr);
        assert!(
            !pickup.stderr.contains("taking Ready issue"),
            "{}",
            pickup.stderr
        );
        assert_eq!(scenario.claude_calls().len(), 2);
        let mut state = scenario.gh_state();
        state["issues"][spec.to_string()] = json!("CLOSED");
        scenario.write_gh_state(&state);
        scenario.agent_does(&audit_script("[]"));
        let after = scenario.run(&["secure", "security-fix"]);
        assert_eq!(after.code, Some(0), "{}", after.stderr);
        assert!(
            after
                .stderr
                .contains("Security audit recorded 0 new finding(s)"),
            "{}",
            after.stderr
        );
        assert_eq!(scenario.claude_calls().len(), 3);
    }
}

#[test]
fn a_successful_fix_awaiting_review_does_not_pause_allowed_security_work() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, implement_fix());
    let fixed = scenario.run(&["secure", "security-fix"]);
    assert_eq!(fixed.code, Some(0), "{}", fixed.stderr);
    scenario.agent_does(&audit_script("[]"));
    let audited = scenario.run(&["secure", "security-fix"]);
    assert_eq!(audited.code, Some(0), "{}", audited.stderr);
    assert!(
        audited
            .stderr
            .contains("Security audit recorded 0 new finding(s)"),
        "{}",
        audited.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 3);
}

#[test]
fn an_undecided_run_offers_both_ways_to_allow_a_reproduced_fix() {
    for base in [None, Some("main")] {
        for (config, permission, reproduced, offers) in [
            ("", None, true, true),
            ("", Some("no-security-fix"), true, false),
            ("[security]\nfix = false\n", None, true, false),
            (
                "[security]\nfix = true\n",
                Some("no-security-fix"),
                true,
                false,
            ),
            ("", None, false, false),
        ] {
            let scenario = Scenario::new();
            scenario.user_config_is(config);
            scenario.agent_does_in_session(1, "true\n"); // Requested Model check.
            scenario
                .agent_does_in_session(2, &audit_script(&json!([finding("offer")]).to_string()));
            scenario.agent_does_in_session(
                3,
                &reproduction_script(if reproduced {
                    "reproduced high single"
                } else {
                    "not reproduced"
                }),
            );
            let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
            let mut args = vec![
                "secure",
                "harness",
                "claude",
                "email",
                "day@example.com",
                "model",
                "Model Label With Spaces",
            ];
            if let Some(base) = base {
                args.extend(["base", base]);
            }
            if let Some(permission) = permission {
                args.push(permission);
            }
            let result = scenario.run_with_env(&args, &resend_env(&resend));
            assert_eq!(result.code, Some(0), "{}", result.stderr);
            let (_, body) = the_one_notification(&resend);
            let mut offered_command = None;
            for text in [&result.stderr, &body] {
                let command = text.lines().find_map(|line| {
                    line.split_once("Allow fixing:")
                        .map(|(_, command)| command.trim())
                });
                assert_eq!(command.is_some(), offers, "{text}");
                if let Some(command) = command {
                    if let Some(previous) = offered_command {
                        assert_eq!(command, previous);
                    }
                    offered_command = Some(command);
                    let words = support::guidance::words(command);
                    assert_eq!(words.first().map(String::as_str), Some("secure"));
                    assert_eq!(words.len(), if base.is_some() { 10 } else { 8 });
                    assert_eq!(
                        words
                            .iter()
                            .filter(|word| word.trim_start_matches('-') == "security-fix")
                            .count(),
                        1
                    );
                    if let Some(base) = base {
                        assert!(
                            words
                                .windows(2)
                                .any(|pair| pair[0].trim_start_matches('-') == "base"
                                    && pair[1] == base),
                            "{words:?}"
                        );
                    } else {
                        assert!(
                            !words
                                .iter()
                                .any(|word| word.trim_start_matches('-') == "base")
                        );
                    }
                    for pair in [
                        ["email", "day@example.com"],
                        ["harness", "claude"],
                        ["model", "Model Label With Spaces"],
                    ] {
                        assert!(
                            words
                                .windows(2)
                                .any(|actual| actual[0].trim_start_matches('-') == pair[0]
                                    && actual[1] == pair[1]),
                            "{words:?}"
                        );
                    }
                }
                assert_eq!(
                    text.contains("fix = true under [security]"),
                    offers,
                    "{text}"
                );
                assert!(!text.contains("Private candidate write-up"), "{text}");
            }
            assert_eq!(scenario.claude_calls().len(), 3);
        }
    }
}

#[test]
fn invalid_fix_tickets_are_not_marked_ready_or_dispatched() {
    // A single public fix has no session-created path: every entry here is
    // either a missing staged proposal or staged text that validation
    // rejects before thirdshift creates anything.
    for script in [
        "printf '%s\\n' 'No protocol line' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n".to_string(),
        url_line_only().to_string(),
        "printf 'Proposed public issue:\\nTitle: Bound accepted input\\nBody:\\nPrivate candidate write-up. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n".to_string(),
        "printf 'Proposed public issue:\\nTitle: Bound accepted input\\nBody:\\nReject oversized input.\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n".to_string(),
    ] {
        let scenario = with_reproduced_findings(&["high"]);
        scenario.agent_does_in_session(1, &script);
        let result = scenario.run(&["secure", "security-fix"]);
        assert_eq!(result.code, Some(1), "{script}: {}", result.stderr);
        assert_eq!(scenario.claude_calls().len(), 1);
        assert!(
            !scenario.gh_state()["labels"]["8"]
                .as_array()
                .is_some_and(|labels| labels.contains(&json!("ready-for-agent")))
        );
        assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
    }
}

#[test]
fn security_fix_words_are_accepted_once_on_every_command_that_starts_runs() {
    for prefix in [
        vec!["https://github.com/acme/widgets/issues/7"],
        vec!["architect"],
        vec!["pickup"],
        vec!["secure"],
    ] {
        for word in [
            "security-fix",
            "--security-fix",
            "no-security-fix",
            "--no-security-fix",
        ] {
            let scenario = Scenario::new();
            scenario.user_config_is("[security]\nfix = 'invalid'\n");
            let mut args = prefix.clone();
            args.push(word);
            let result = scenario.run(&args);
            assert_eq!(result.code, Some(1), "{args:?}: {}", result.stderr);
            assert!(
                result.stderr.contains("security.fix must be true or false"),
                "{}",
                result.stderr
            );
        }
        for (words, error) in [
            (
                vec!["security-fix", "no-security-fix"],
                "can't be used together",
            ),
            (vec!["security-fix", "--security-fix"], "repeated argument"),
        ] {
            let scenario = Scenario::new();
            let mut args = prefix.clone();
            args.extend(words);
            let result = scenario.run(&args);
            assert_eq!(result.code, Some(2), "{args:?}: {}", result.stderr);
            assert!(result.stderr.contains(error));
        }
    }
}

#[test]
fn reproductions_are_sequential_fresh_and_pinned_to_the_audited_commit() {
    let scenario = Scenario::new();
    let audited = scenario.origin_git(&["rev-parse", "main"]);
    scenario.agent_does_in_session(
        1,
        &format!(
            r#"{}
touch audit-only
new=$(git commit-tree "$(git rev-parse 'HEAD^{{tree}}')" -p HEAD -m 'Base advanced')
git push origin "$new:refs/heads/main"
"#,
            audit_script(&json!([finding("first"), finding("second")]).to_string())
        ),
    );
    scenario.agent_does_in_session(
        2,
        &format!(
            r#"
test -z "$(git branch --show-current)"
test ! -e audit-only
git rev-parse HEAD >> "$HOME/../reproductions"
printf '%s\n' first >> "$HOME/../sequence"
touch previous-poc
{}
"#,
            reproduction_script("reproduced low single")
        ),
    );
    scenario.agent_does_in_session(
        3,
        &format!(
            r#"
test ! -e previous-poc
test ! -e audit-only
test "$(cat "$HOME/../sequence")" = first
git rev-parse HEAD >> "$HOME/../reproductions"
printf '%s\n' second >> "$HOME/../sequence"
{}
"#,
            reproduction_script("not reproduced")
        ),
    );
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_ne!(scenario.origin_git(&["rev-parse", "main"]), audited);
    assert_eq!(
        fs::read_to_string(scenario.path("reproductions")).unwrap(),
        format!("{audited}{audited}")
    );
    assert_eq!(
        fs::read_to_string(scenario.path("sequence")).unwrap(),
        "first\nsecond\n"
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 3);
    for (index, fingerprint) in [(1, "first"), (2, "second")] {
        let prompt = calls[index]["prompt"].as_str().unwrap();
        for required in [
            "validation plan",
            "untouched code",
            "harmless payloads only",
            "never a deployed site or a real third-party service",
            "likelihood-and-impact",
            "one session",
            "Test file:",
            "Security reproduction:",
            "You run headless",
        ] {
            assert!(prompt.contains(required), "missing {required}: {prompt}");
        }
        assert!(
            prompt.contains(&format!("Fingerprint: `{fingerprint}`")),
            "{prompt}"
        );
        assert_eq!(calls[index]["branch"], "");
    }
    assert_eq!(scenario.entries("work"), vec![REPO]);
    scenario.assert_every_session_found_the_factory_skills();
}

#[test]
fn incomplete_reproductions_preserve_the_record_and_stop_before_the_next_session() {
    for private in [false, true] {
        for (outcome, cause) in [
            ("No final line", "without the final line"),
            ("reproduced invalid single", "invalid severity"),
            ("reproduced high invalid", "invalid fix size"),
        ] {
            let scenario = Scenario::new();
            let commit = scenario.origin_git(&["rev-parse", "main"]);
            let original = format!(
                "Fingerprint: `first`\nAudited commit: `{}`\nExisting private evidence.\n",
                commit.trim()
            );
            let mut github = scenario.gh_state();
            github["private"] = json!(private);
            scenario.write_gh_state(&github);
            if private {
                github["issues"]["8"] = json!("OPEN");
                github["bodies"] = json!({"8": original});
                github["labels"] = json!({"8": ["security-finding", "needs-triage"]});
            } else {
                github["advisories"] = json!([{
                    "ghsa_id": "GHSA-existing", "description": original, "severity": null, "state": "draft",
                    "summary": "Existing finding", "html_url": "https://github.com/acme/widgets/security/advisories/GHSA-existing"
                }]);
            }
            scenario.agent_does_in_session(
                1,
                &audit_script_with_records(
                    &scenario,
                    &json!([finding("first"), finding("second")]).to_string(),
                    &github,
                ),
            );
            scenario.agent_does_in_session(2, &reproduction_script(outcome));
            let result = scenario.run(&["secure"]);
            assert_eq!(result.code, Some(1), "{}", result.stderr);
            assert!(result.stderr.contains(cause), "{}", result.stderr);
            assert!(
                result.stderr.contains("security-reproduction-1.logs.jsonl"),
                "{}",
                result.stderr
            );
            let state = scenario.gh_state();
            if private {
                assert_eq!(state["bodies"]["8"], original);
                assert_eq!(state["labels"]["8"], github["labels"]["8"]);
                assert!(
                    state["bodies"]["9"]
                        .as_str()
                        .unwrap()
                        .contains("Fingerprint: `second`")
                );
            } else {
                assert_eq!(state["advisories"][0], github["advisories"][0]);
                assert_eq!(state["advisories"].as_array().unwrap().len(), 2);
            }
            assert_eq!(scenario.claude_calls().len(), 2);
            assert!(
                scenario
                    .gh_calls_of("api", "--method")
                    .iter()
                    .all(|call| call[2] != "PATCH")
            );
            assert_eq!(scenario.entries("work"), vec![REPO]);
        }
    }
}

#[test]
fn an_unreproduced_finding_keeps_no_severity_and_keeps_the_test_and_notes() {
    let scenario = Scenario::new();
    let commit = scenario.origin_git(&["rev-parse", "main"]);
    let original = format!(
        "Fingerprint: `input-size`\nAudited commit: `{}`\nExisting private evidence.\n",
        commit.trim()
    );
    let mut github = scenario.gh_state();
    github["advisories"] = json!([{
        "ghsa_id": "GHSA-existing", "description": original, "severity": null, "state": "draft",
        "summary": "Existing finding", "html_url": "https://github.com/acme/widgets/security/advisories/GHSA-existing"
    }]);
    scenario.agent_does_in_session(
        1,
        &audit_script_with_records(
            &scenario,
            &json!([finding("input-size")]).to_string(),
            &github,
        ),
    );
    scenario.agent_does_in_session(2, &reproduction_script("not reproduced"));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let record = &state["advisories"][0];
    assert_eq!(record["severity"], Value::Null);
    let body = record["description"].as_str().unwrap();
    assert!(body.starts_with(original.trim_end()), "{body}");
    assert!(body.contains("Outcome: not reproduced"), "{body}");
    assert!(body.contains("Local command: bounded-fixture"), "{body}");
    assert!(body.contains("bounded_fixture();"), "{body}");
    assert!(!body.contains("Severity:"), "{body}");
    assert!(!body.contains("Fix size:"), "{body}");
    assert!(
        result.stderr.contains("reproduction 1: not reproduced"),
        "{}",
        result.stderr
    );
}

#[test]
fn records_a_finding_privately_as_a_draft_without_severity_or_versions() {
    let scenario = Scenario::new();
    scenario.origin_has_commit(
        "main",
        "Cargo.toml",
        "[package]\nname = \"widgets\"\nversion = \"1.2.3\"\n",
        "Add manifest",
    );
    scenario.agent_does(&audit_script(&json!([finding("input-size")]).to_string()));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "needs you");
    assert_eq!(status["commands"][0]["needs_you_kind"], "finding");
    let retained = &status["commands"][0]["security_findings"];
    assert_eq!(retained[0]["title"], "Unchecked input size");
    assert!(
        retained[0]["url"]
            .as_str()
            .unwrap()
            .contains("/security/advisories/")
    );
    let shown = scenario.run(&[
        "show",
        status["commands"][0]["name"].as_str().unwrap(),
        "--json",
    ]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(&shown["command"]["security_findings"], retained);
    let state = scenario.gh_state();
    let advisory = &state["advisories"][0];
    assert_eq!(advisory["state"], "draft");
    assert_eq!(advisory["summary"], "Unchecked input size");
    assert_eq!(advisory["severity"], Value::Null);
    assert_eq!(advisory["cwe_ids"], json!([]));
    assert_eq!(
        advisory["vulnerabilities"][0]["package"],
        json!({"ecosystem":"rust", "name":"widgets"})
    );
    assert_eq!(
        advisory["vulnerabilities"][0]["vulnerable_version_range"],
        Value::Null
    );
    let description = advisory["description"]
        .as_str()
        .expect("no private advisory was recorded");
    for required in [
        "Private candidate write-up.",
        "Private trace.",
        "Private evidence.",
        "Exercise a bounded fixture.",
        "input-size",
        "thirdshift's Security run",
        scenario.origin_git(&["rev-parse", "main"]).trim(),
    ] {
        assert!(
            description.contains(required),
            "missing {required}: {description}"
        );
    }
    assert!(!result.stderr.contains("Private candidate write-up."));
    assert!(state["prs"].as_array().unwrap().is_empty());
    assert_eq!(state["issues"].as_object().unwrap().len(), 1);
    assert_eq!(
        scenario.origin_git(&["log", "--format=%s", "main"]),
        "Add manifest\nInitial commit\n"
    );
}

#[test]
fn a_private_repository_records_each_finding_as_a_labelled_issue_with_the_advisory_body() {
    let scenario = Scenario::new();
    let mut github = scenario.gh_state();
    github["private"] = json!(true);
    scenario.write_gh_state(&github);
    scenario.agent_does(&audit_script(
        &json!([finding("input-size"), finding("storage-bound")]).to_string(),
    ));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert!(state["advisories"].is_null());
    assert_eq!(state["issues"].as_object().unwrap().len(), 3);
    let commit = scenario.origin_git(&["rev-parse", "main"]);
    for (number, fingerprint) in [("8", "input-size"), ("9", "storage-bound")] {
        assert_eq!(state["titles"][number], "Unchecked input size");
        assert_eq!(
            state["labels"][number],
            json!(["security-finding", "needs-triage"])
        );
        let expected = format!(
            "Found by thirdshift's Security run.\n\nFingerprint: `{fingerprint}`\nAudited commit: `{}`\n\nPrivate candidate write-up.\n\n```json\n{}\n```\n",
            commit.trim(),
            serde_json::to_string_pretty(&finding(fingerprint)).unwrap()
        );
        let body = state["bodies"][number].as_str().unwrap();
        assert!(body.starts_with(&expected), "{body}");
        assert!(body.contains("Outcome: not reproduced"), "{body}");
        assert!(!body.contains("Severity:"), "{body}");
    }
    assert!(!result.stderr.contains("Private candidate write-up."));
}

#[test]
fn repeat_private_audits_match_open_and_closed_findings_after_triage_without_duplicates() {
    for state in ["OPEN", "CLOSED"] {
        let scenario = Scenario::new();
        let mut github = scenario.gh_state();
        github["private"] = json!(true);
        scenario.write_gh_state(&github);
        scenario.agent_does(&audit_script(
            &json!([finding("input-size"), finding("input-size-extra")]).to_string(),
        ));
        let first = scenario.run(&["secure"]);
        assert_eq!(first.code, Some(0), "{}", first.stderr);
        assert!(
            first
                .stderr
                .contains("2 new finding(s), 0 already recorded"),
            "{}",
            first.stderr
        );
        let mut github = scenario.gh_state();
        for number in ["8", "9"] {
            github["issues"][number] = json!(state);
            github["labels"][number] = json!(["Security-Finding"]);
        }
        scenario.write_gh_state(&github);
        scenario.origin_has_commit("main", "new-work.txt", "new work", "Move Base branch");
        let second = scenario.run(&["secure"]);
        assert_eq!(second.code, Some(0), "{}", second.stderr);
        assert!(
            second
                .stderr
                .contains("0 new finding(s), 2 already recorded"),
            "{}",
            second.stderr
        );
        assert_eq!(scenario.gh_state(), github);
        assert_eq!(scenario.gh_calls_of("issue", "create").len(), 2);
    }
}

#[test]
fn private_finding_labels_are_created_with_descriptions_and_existing_labels_are_kept() {
    for existing in [
        vec!["bug"],
        vec!["bug", "Security-Finding"],
        vec!["bug", "NEEDS-TRIAGE"],
        vec!["bug", "Security-Finding", "NEEDS-TRIAGE"],
    ] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&existing);
        let mut github = scenario.gh_state();
        github["private"] = json!(true);
        scenario.write_gh_state(&github);
        scenario.agent_does(&audit_script(&json!([finding("input-size")]).to_string()));
        let result = scenario.run(&["secure"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let created = scenario.gh_calls_of("label", "create");
        let mut expected_labels: Vec<String> =
            existing.iter().map(|name| name.to_string()).collect();
        let mut missing = 0;
        for (label, description) in [
            (
                "security-finding",
                "A Security finding recorded privately for the Day shift",
            ),
            ("needs-triage", "Maintainer needs to evaluate this issue"),
        ] {
            if !existing.iter().any(|name| name.eq_ignore_ascii_case(label)) {
                missing += 1;
                assert!(
                    created.iter().any(|call| call[2] == label
                        && call
                            .windows(2)
                            .any(|args| args == ["--description", description])),
                    "{created:?}"
                );
                expected_labels.push(label.to_string());
            }
        }
        assert_eq!(created.len(), missing);
        assert_eq!(scenario.repo_labels(), expected_labels);
        assert_eq!(
            scenario.issue_labels(8),
            ["security-finding", "needs-triage"]
        );
    }
}

#[test]
fn advisory_errors_never_record_findings_in_a_public_repository_or_on_non_404_failures() {
    for (private, error) in [
        (false, "gh: Not Found (HTTP 404)"),
        (true, "gh: Forbidden (HTTP 403)"),
        (true, "gh: Bad Gateway (HTTP 502)"),
    ] {
        let scenario = Scenario::new();
        let mut github = scenario.gh_state();
        github["private"] = json!(private);
        github["advisory_error"] = json!(error);
        scenario.write_gh_state(&github);
        scenario.agent_does(&audit_script(&json!([finding("input-size")]).to_string()));
        let result = scenario.run(&["secure"]);
        assert_ne!(result.code, Some(0), "{}", result.stderr);
        assert!(scenario.gh_calls_of("issue", "create").is_empty());
        assert!(scenario.gh_calls_of("label", "create").is_empty());
        assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
        assert!(!result.stderr.contains("Private candidate write-up."));
    }
}

#[test]
fn reads_the_package_from_an_npm_manifest() {
    let scenario = Scenario::new();
    scenario.origin_has_commit(
        "main",
        "package.json",
        r#"{"name":"@acme/widgets","version":"1.0.0"}"#,
        "Add npm manifest",
    );
    scenario.agent_does(&audit_script(&json!([finding("input-size")]).to_string()));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["advisories"][0]["vulnerabilities"][0]["package"],
        json!({"ecosystem":"npm", "name":"@acme/widgets"})
    );
}

fn audit_script(findings: &str) -> String {
    format!(
        r#"
printf '%s' "$FAKE_CLAUDE_PROMPT" > prompt.txt
test_file=$(sed -n 's/^Test file: `\(.*\)`\.$/\1/p' prompt.txt)
if [ -n "$test_file" ]; then
    printf '%s\n' 'bounded_fixture();' > "$test_file"
    printf '%s\n' 'The harmless bounded fixture did not reproduce the claim.' 'Security reproduction: not reproduced' > "$FAKE_CLAUDE_FINAL_MESSAGE"
    exit 0
fi
output=$(sed -n 's/^Output directory: `\(.*\)`\.$/\1/p' prompt.txt)
test -n "$output"
printf '%s\n' '{findings}' > "$output/findings.json"
printf '%s\n' '[]' > "$output/coverage-ledger.json"
printf '%s\n' '{{"run_status":"complete"}}' > "$output/run-metadata.json"
printf '%s\n' 'Security audit: complete' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
    )
}

/// A finding recorded during the audit is first seen after the gates.
fn audit_script_with_records(scenario: &Scenario, findings: &str, records: &Value) -> String {
    fs::write(
        scenario.path("audit-records.json"),
        serde_json::to_vec(records).unwrap(),
    )
    .unwrap();
    format!(
        "cp \"$HOME/../audit-records.json\" \"$FAKE_GH_STATE\"\n{}",
        audit_script(findings)
    )
}

fn assert_two_quiet_skips_send_no_email(scenario: &Scenario) {
    let resend = support::resend::ResendStandIn::replying(200, r#"{"id":"unused"}"#);
    for _ in 0..2 {
        let skipped = scenario.run_with_env(
            &["secure", "email", "me@example.com"],
            &[
                ("THIRDSHIFT_RESEND_URL", resend.url()),
                ("RESEND_API_KEY", "re_test"),
            ],
        );
        assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
        assert_eq!(skipped.stdout, "");
        assert_eq!(skipped.stderr, "");
    }
    assert!(resend.requests().is_empty());
}

#[test]
fn an_unchanged_base_skips_after_a_completed_audit_and_resumes_when_origin_moves() {
    let scenario = Scenario::new();
    scenario.user_config_is("[activity]\nquiet_skips = true\n");
    scenario.agent_does(&audit_script("[]"));
    let first = scenario.run(&["secure"]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let before =
        fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    let commands = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/secure", "log");
    assert_two_quiet_skips_send_no_email(&scenario);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(
        scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/secure", "log"),
        commands
    );
    let after =
        fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert_eq!(after.lines().count(), before.lines().count() + 1);
    assert!(
        after.contains("Base branch main hasn't changed since the last completed Security audit"),
        "{after}"
    );
    scenario.origin_has_commit("main", "change.txt", "new work", "Change Base branch");
    let next = scenario.run(&["secure"]);
    assert_eq!(next.code, Some(0), "{}", next.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
}

#[test]
fn waiting_findings_hold_off_no_audit_and_keep_their_completed_reproductions() {
    for private in [false, true] {
        let scenario = Scenario::new();
        let mut github = scenario.gh_state();
        github["private"] = json!(private);
        scenario.write_gh_state(&github);
        scenario.agent_does(&audit_script(
            &json!([finding("input-size"), finding("storage-bound")]).to_string(),
        ));
        let first = scenario.run(&["secure"]);
        assert_eq!(first.code, Some(0), "{}", first.stderr);
        assert_eq!(scenario.claude_calls().len(), 3);
        // Both findings are not reproduced and still wait for the Day shift.
        let waiting = scenario.gh_state();
        scenario.origin_has_commit("main", "new-work.txt", "new work", "Move Base branch");
        let second = scenario.run(&["secure"]);
        assert_eq!(second.code, Some(0), "{private}: {}", second.stderr);
        assert!(
            second
                .stderr
                .contains("0 new finding(s), 2 already recorded"),
            "{}",
            second.stderr
        );
        assert_eq!(
            second
                .stderr
                .matches("keeping the completed Security reproduction of ")
                .count(),
            2,
            "{}",
            second.stderr
        );
        if private {
            for issue in ["issue #8", "issue #9"] {
                assert!(
                    second.stderr.contains(&format!(
                        "keeping the completed Security reproduction of {issue}: not reproduced"
                    )),
                    "{}",
                    second.stderr
                );
            }
        }
        assert_eq!(scenario.claude_calls().len(), 4);
        assert_eq!(scenario.gh_state(), waiting);
    }
}

#[test]
fn incomplete_or_rejected_audit_artifacts_do_not_hold_the_next_attempt() {
    for script in [
        audit_script("[{}]"),
        audit_script("[]").replace("Security audit: complete", "Missing protocol line"),
        audit_script("[]").replace("Security audit: complete", "Security audit: incomplete"),
        audit_script("[]").replace(
            "\"run_status\":\"complete\"",
            "\"run_status\":\"incomplete\"",
        ),
        format!("{}\nexit 1\n", audit_script("[]")),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&script);
        let failed = scenario.run(&["secure"]);
        assert_eq!(failed.code, Some(1), "{}", failed.stderr);
        let runs = scenario.entries("home/.thirdshift/logs/acme/widgets/audits");
        let record: Value = serde_json::from_slice(
            &fs::read(scenario.path(&format!(
                "home/.thirdshift/logs/acme/widgets/audits/{}/run-metadata.json",
                runs[0]
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(record["run_status"], "incomplete");
        scenario.agent_does(&audit_script("[]"));
        let retry = scenario.run(&["secure"]);
        assert_eq!(retry.code, Some(0), "{}", retry.stderr);
        assert_eq!(scenario.claude_calls().len(), 2);
    }
}

#[test]
fn keeps_fingerprints_in_every_advisory_state_and_creates_each_new_finding_once() {
    for state in ["draft", "published", "closed", "triage"] {
        let scenario = Scenario::new();
        scenario.agent_does(&audit_script(
            &json!([finding("input-size"), finding("storage-bound")]).to_string(),
        ));
        let first = scenario.run(&["secure"]);
        assert_eq!(first.code, Some(0), "{}", first.stderr);
        let mut github = scenario.gh_state();
        assert_eq!(github["advisories"].as_array().unwrap().len(), 2);
        for advisory in github["advisories"].as_array_mut().unwrap() {
            advisory["state"] = json!(state);
            if state == "draft" {
                advisory["severity"] = json!("low");
            }
        }
        scenario.write_gh_state(&github);
        scenario.origin_has_commit("main", "new-work.txt", "new work", "Move Base branch");
        let second = scenario.run(&["secure"]);
        assert_eq!(second.code, Some(0), "{}", second.stderr);
        assert!(
            second
                .stderr
                .contains("0 new finding(s), 2 already recorded"),
            "{}",
            second.stderr
        );
        assert_eq!(scenario.gh_state()["advisories"], github["advisories"]);
        let api_calls = scenario.gh_calls_of("api", "--method");
        assert_eq!(api_calls.iter().filter(|call| call[2] == "POST").count(), 2);
        assert_eq!(
            api_calls.iter().filter(|call| call[2] == "PATCH").count(),
            2
        );
        assert_eq!(
            scenario
                .entries("home/.thirdshift/logs/acme/widgets/audits")
                .len(),
            2
        );
    }
}

#[test]
fn an_explicit_base_and_harness_audit_origin_without_pulling_local_work() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("stable", "main", &["Stable"]);
    scenario.user_config_is("[launch]\npull = true\n");
    scenario.agent_does(&audit_script("[]"));
    let head = scenario.launch_git(&["rev-parse", "HEAD"]);
    let result = scenario.run(&[
        "secure", "base", "stable", "harness", "codex", "model", "gpt-5.5", "effort", "high",
        "merge", "base-fix", "parallel", "2",
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.launch_git(&["rev-parse", "HEAD"]), head);
    assert_eq!(scenario.launch_git(&["branch", "--show-current"]), "main\n");
    assert!(scenario.claude_calls().is_empty());
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["branch"], "");
    assert!(
        calls[0]["prompt"]
            .as_str()
            .unwrap()
            .contains(scenario.origin_git(&["rev-parse", "stable"]).trim())
    );
    scenario.assert_every_codex_session_found_the_factory_skills();
}

#[test]
fn the_security_harness_overrides_the_default_and_uses_its_own_model_and_effort() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[security]\nharness = \"codex\"\n\
         [harness]\ndefault = \"claude\"\n\
         [harness.claude]\nmodel = \"opus\"\neffort = \"low\"\n\
         [harness.codex]\nmodel = \"gpt-6.1-sol\"\neffort = \"xhigh\"\n",
    );
    scenario.agent_does(&audit_script("[]"));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty());
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 1);
    let args = calls[0]["argv"].as_array().unwrap();
    assert!(
        args.windows(2)
            .any(|pair| pair == [json!("-m"), json!("gpt-6.1-sol")])
    );
    assert!(
        args.windows(2)
            .any(|pair| pair == [json!("-c"), json!("model_reasoning_effort=\"xhigh\"")])
    );
}

#[test]
fn codex_security_sessions_and_their_resumes_raise_the_thread_cap_and_request_fresh_sub_agents() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{}\necho '{{\"type\":\"item.started\",\"item\":{{\"id\":\"pending\",\"type\":\"collab_tool_call\",\"tool\":\"spawn_agent\",\"prompt\":\"Verify finding\",\"status\":\"in_progress\"}}}}'\n",
            audit_script(&json!([finding("input-size")]).to_string())
        ),
    );
    scenario
        .agent_does("printf '%s\\n' 'Security audit: complete' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n");
    scenario.agent_does_in_session(
        3,
        &format!(
            "{}\necho '{{\"type\":\"item.started\",\"item\":{{\"id\":\"pending\",\"type\":\"command_execution\",\"command\":\"bounded-fixture\",\"status\":\"in_progress\"}}}}'\n",
            reproduction_script("reproduced high single")
        ),
    );
    scenario.agent_does_in_session(
        4,
        "printf '%s\\n' 'The resumed local fixture confirmed the finding.' 'Security reproduction: reproduced high single' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
    );
    let result = scenario.run(&["secure", "harness", "codex"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 4);
    for call in &calls {
        let args = call["argv"].as_array().unwrap();
        assert!(
            args.windows(2).any(|pair| pair
                == [
                    json!("-c"),
                    json!("agents.max_concurrent_threads_per_session=8")
                ]),
            "missing Security thread cap: {args:?}"
        );
        let prompt = call["prompt"].as_str().unwrap();
        assert!(prompt.contains("fork_turns: \"none\""), "{prompt}");
        assert!(prompt.contains("fresh sub-agents"), "{prompt}");
    }
    for (index, thread) in [(1, "fake-thread-1"), (3, "fake-thread-3")] {
        let resumed = calls[index]["argv"].as_array().unwrap();
        assert!(
            resumed
                .windows(2)
                .any(|pair| pair == [json!("resume"), json!(thread)]),
            "{resumed:?}"
        );
    }
    assert_eq!(scenario.gh_state()["advisories"][0]["severity"], "high");
}

#[test]
fn the_command_harness_wins_over_security_and_default_settings_and_uses_claudes_settings() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[security]\nharness = \"codex\"\n\
         [harness]\ndefault = \"agy\"\n\
         [harness.claude]\nmodel = \"opus\"\neffort = \"high\"\n\
         [harness.codex]\nmodel = \"gpt-6.1-sol\"\neffort = \"xhigh\"\n",
    );
    // Claude's Model check is its first call; the second runs the audit.
    scenario.agent_does_in_session(1, "true\n");
    scenario.agent_does(&audit_script("[]"));
    let result = scenario.run(&["secure", "harness", "claude"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.codex_calls().is_empty());
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    let args = calls[1]["argv"].as_array().unwrap();
    for pair in [
        [json!("--model"), json!("opus")],
        [json!("--effort"), json!("high")],
    ] {
        assert!(args.windows(2).any(|actual| actual == pair), "{args:?}");
    }
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert!(!prompt.contains("fork_turns"), "{prompt}");
    assert!(
        !args
            .iter()
            .any(|arg| arg.as_str().unwrap().contains("max_concurrent_threads"))
    );
}

#[test]
fn a_blank_or_missing_security_harness_preserves_the_default_harness_and_its_settings() {
    for security in [
        "",
        "[security]\n",
        "[security]\nharness = \"\"\n",
        "[security]\nharness = \"  \"\n",
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(&format!(
            "{security}[harness]\ndefault = \"codex\"\n\
             [harness.codex]\nmodel = \"gpt-6-luna\"\neffort = \"high\"\n"
        ));
        scenario.agent_does(&audit_script("[]"));
        let result = scenario.run(&["secure"]);
        assert_eq!(result.code, Some(0), "{security}: {}", result.stderr);
        assert!(scenario.claude_calls().is_empty());
        let calls = scenario.codex_calls();
        let args = calls[0]["argv"].as_array().unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair == [json!("-m"), json!("gpt-6-luna")])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == [json!("-c"), json!("model_reasoning_effort=\"high\"")])
        );

        let scenario = Scenario::new();
        scenario.user_config_is(security);
        scenario.agent_does(&audit_script("[]"));
        let result = scenario.run(&["secure"]);
        assert_eq!(result.code, Some(0), "{security}: {}", result.stderr);
        assert_eq!(scenario.claude_calls().len(), 1, "{security}");
        assert!(scenario.codex_calls().is_empty(), "{security}");
    }
}

#[test]
fn invalid_security_harness_settings_name_the_key_and_stop_before_any_work() {
    for config in [
        "[security]\nharness = \"unknown\"\n",
        "[security]\nharness = true\n",
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(config);
        let result = scenario.run(&["secure"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("security.harness"),
            "{}",
            result.stderr
        );
        assert!(scenario.gh_calls().is_empty());
        assert!(scenario.claude_calls().is_empty());
        assert!(scenario.codex_calls().is_empty());
    }
}

#[test]
fn a_ready_issue_skips_before_any_session_and_keeps_only_the_activity_log() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.user_config_is("[activity]\nquiet_skips = true\n");
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(result.stderr, "");
    assert!(scenario.claude_calls().is_empty());
    let log = fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
        .unwrap();
    assert!(
        log.contains("Security run skipped: Ready issue #7"),
        "{log}"
    );
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands")
            .exists()
    );
}

#[test]
fn a_settling_issue_does_not_skip_the_security_audit() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.issue_timeline(
        7,
        &[(support::TimelineEvent::Labelled("ready-for-agent"), 4)],
    );
    scenario.agent_does(&audit_script("[]"));

    let result = scenario.run(&["secure"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.is_empty(), "{}", result.stdout);
    assert!(
        result
            .stderr
            .contains("#7 not settled: labelled ready-for-agent less than 5 minutes ago"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn an_untrusted_ready_label_does_not_skip_the_security_audit() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    let mut state = scenario.gh_state();
    state["trust_events"] = json!({"7": [
        {"created_at": "2020-01-01T00:00:00Z", "event": "labeled", "label": {"name": "ready-for-agent"}, "actor": {"login": "outsider"}}
    ]});
    scenario.write_gh_state(&state);
    scenario.agent_does(&audit_script("[]"));

    let result = scenario.run(&["secure"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("#7 most recent ready-for-agent label actor is not a Trusted account"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn another_pass_holds_the_security_run_and_its_skip_is_logged() {
    let scenario = Scenario::new();
    let waiting = format!(
        "touch \"$HOME/../started\"\nwhile [ ! -f \"$HOME/../release\" ]; do sleep 0.05; done\n{}",
        audit_script("[]")
    );
    scenario.agent_does(&waiting);
    let first = scenario.run_until(&["secure"], &[], "started");
    // Another Pass goes before a Ready issue.
    scenario.issue_labelled(7, &["ready-for-agent"]);
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let second =
        scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
    let pickup = scenario.run(&["pickup"]);
    fs::write(scenario.path("release"), "").unwrap();
    let first = first.finish();
    assert!(resend.requests().is_empty());
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    for skipped in [second, pickup] {
        assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
        assert!(
            skipped.stderr.contains("another Pass is already running"),
            "{}",
            skipped.stderr
        );
        assert!(
            !skipped.stderr.contains("Ready issue"),
            "{}",
            skipped.stderr
        );
    }
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(
        scenario
            .log_files("home/.thirdshift/logs/acme/widgets/commands/secure", "log")
            .len(),
        1
    );
    let log = fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
        .unwrap();
    assert!(log.contains("Security run skipped: another Pass is already running"));
}

/// Link the git on `PATH` into the scenario's `bin`, for a test that runs
/// thirdshift with `bin` as its whole `PATH`. Over a local transport git runs
/// `git-upload-pack` and `git-receive-pack` by name. Linux's and Homebrew's
/// git find them in their exec path, but Xcode's finds them only beside
/// itself, so they are linked too wherever they are beside `git`.
fn link_git(scenario: &Scenario) {
    let git = std::process::Command::new("which")
        .arg("git")
        .output()
        .unwrap();
    let git = std::path::PathBuf::from(String::from_utf8(git.stdout).unwrap().trim());
    std::os::unix::fs::symlink(&git, scenario.path("bin/git")).unwrap();
    for name in ["git-upload-pack", "git-receive-pack"] {
        let program = git.with_file_name(name);
        if program.exists() {
            std::os::unix::fs::symlink(program, scenario.path("bin").join(name)).unwrap();
        }
    }
}

#[test]
fn missing_node_fails_before_any_session_and_names_the_prerequisite() {
    let scenario = Scenario::new();
    link_git(&scenario);
    let path = scenario.path("bin");
    let result = scenario.run_with_env(&["secure"], &[("PATH", path.to_str().unwrap())]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("Node.js is required"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(scenario.entries("work"), vec![REPO]);
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands")
            .exists()
    );
}

#[test]
fn missing_final_line_or_invalid_reports_fail_without_recording() {
    for (script, cause) in [
        (
            audit_script("[]").replace("Security audit: complete", "No protocol line"),
            "without the final line",
        ),
        (
            audit_script("[]").replace("Security audit: complete", "Security audit: incomplete"),
            "ended incomplete",
        ),
        (audit_script("[{}]"), "validator validate-findings.cjs"),
        (
            audit_script("[]").replace(
                "'[]' > \"$output/coverage-ledger.json\"",
                "'[{}]' > \"$output/coverage-ledger.json\"",
            ),
            "validator validate-coverage-ledger.cjs",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&script);
        let result = scenario.run(&["secure"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains(cause),
            "expected {cause}: {}",
            result.stderr
        );
        assert!(result.stderr.contains("session trace:"));
        assert!(scenario.gh_state()["advisories"].is_null());
        assert_eq!(scenario.entries("work"), vec![REPO]);
        let log =
            fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
                .unwrap();
        assert!(log.contains("Security run started"));
        assert!(log.contains("Security run ended: failed:"), "{log}");
        let logs = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/secure", "log");
        let command_log = fs::read_to_string(scenario.path(&format!(
            "home/.thirdshift/logs/acme/widgets/commands/secure/{}",
            logs[0]
        )))
        .unwrap();
        assert!(command_log.contains(cause));
    }
}

#[test]
fn rejected_candidates_stay_in_the_report_without_a_private_advisory() {
    let scenario = Scenario::new();
    let mut rejected = finding("refuted-claim");
    rejected.as_object_mut().unwrap().remove("blockers");
    rejected.as_object_mut().unwrap().remove("validation_plan");
    rejected["verdict"] = json!("rejected");
    rejected["reason"] = json!("The source already checks the bound.");
    scenario.agent_does(&audit_script(
        &json!([finding("input-size"), rejected]).to_string(),
    ));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["advisories"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        scenario.gh_state()["advisories"][0]["vulnerabilities"][0]["package"],
        json!({"ecosystem":"other", "name":null})
    );
}

#[test]
fn an_incomplete_run_record_cannot_be_overridden_by_a_complete_final_line() {
    let scenario = Scenario::new();
    scenario.agent_does(&audit_script("[]").replace(
        "\"run_status\":\"complete\"",
        "\"run_status\":\"incomplete\"",
    ));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("run-metadata.json"),
        "{}",
        result.stderr
    );
    assert!(scenario.gh_state()["advisories"].is_null());
}

#[test]
fn audits_origins_base_without_changing_the_launch_directory() {
    let scenario = Scenario::new();
    scenario.origin_has_commit("main", "SECURITY.md", "Trust model", "Document trust model");
    scenario.agent_does(&format!(
        "{}\ntest -z \"$(git branch --show-current)\"\ntest -f SECURITY.md\n",
        audit_script("[]")
    ));
    let head = scenario.launch_git(&["rev-parse", "HEAD"]);
    fs::write(scenario.launch_dir().join("local.txt"), "local work").unwrap();
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("Security audit recorded 0 new finding(s)"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.launch_git(&["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read_to_string(scenario.launch_dir().join("local.txt")).unwrap(),
        "local work"
    );
    assert_eq!(scenario.entries("work"), vec![REPO]);
    assert_eq!(
        scenario.origin_git(&["log", "--format=%s", "main"]),
        "Document trust model\nInitial commit\n"
    );
    assert!(scenario.gh_state()["prs"].as_array().unwrap().is_empty());
    let prompt = scenario.first_prompt();
    for required in [
        "thirdshift-security-audit",
        "full audit mode",
        "quick",
        "vendored",
        "third-party",
        "SECURITY.md",
        "earlier runs",
        "incomplete",
        "Security audit: complete",
        "You run headless",
        "commits, pushes, opens and publishes nothing",
    ] {
        assert!(prompt.contains(required), "missing {required}: {prompt}");
    }
    scenario.assert_every_session_found_the_factory_skills();
}

#[test]
fn security_advisories_preserve_python_and_go_manifest_identity() {
    for (manifest, contents, package) in [
        (
            "pyproject.toml",
            "[project]\nname = \"widgets\"\nversion = \"1.0.0\"\n",
            json!({"ecosystem":"pip", "name":"widgets"}),
        ),
        (
            "go.mod",
            "module example.com/acme/widgets\n\ngo 1.24\n",
            json!({"ecosystem":"go", "name":"example.com/acme/widgets"}),
        ),
    ] {
        let scenario = Scenario::new();
        scenario.origin_has_commit("main", manifest, contents, "Add manifest");
        scenario.agent_does(&audit_script(&json!([finding("input-size")]).to_string()));
        let result = scenario.run(&["secure"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            scenario.gh_state()["advisories"][0]["vulnerabilities"][0]["package"],
            package,
            "package identity was lost for {manifest}"
        );
    }
}

#[test]
fn security_audit_names_the_threat_model_under_docs() {
    let scenario = Scenario::new();
    fs::create_dir_all(scenario.launch_dir().join("docs")).unwrap();
    fs::write(
        scenario.launch_dir().join("docs/THREAT-MODEL.md"),
        "# Threat model\nAll callers are authenticated; tenant isolation is the boundary.\n",
    )
    .unwrap();
    scenario.launch_git(&["add", "docs/THREAT-MODEL.md"]);
    scenario.launch_git(&["commit", "-m", "Document threat model"]);
    scenario.launch_git(&["push", "origin", "main"]);
    scenario.agent_does(&audit_script("[]"));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let prompt = scenario.first_prompt();
    assert!(
        prompt.contains("Read the repository's threat-model document `docs/THREAT-MODEL.md`."),
        "the existing threat-model document was not named: {prompt}"
    );
}

#[test]
fn reproduction_preserves_an_original_finding_reproduction_heading() {
    for private in [false, true] {
        let scenario = Scenario::new();
        let mut github = scenario.gh_state();
        github["private"] = json!(private);
        scenario.write_gh_state(&github);
        let mut candidate = finding("markdown-evidence");
        let original_writeup = "Candidate notes.\n\n## Reproduction\n\nOriginal local validation evidence must survive.";
        candidate["description"] = json!(original_writeup);
        scenario.agent_does_in_session(1, &audit_script(&json!([candidate]).to_string()));
        scenario.agent_does_in_session(2, &reproduction_script("reproduced medium single"));
        let result = scenario.run(&["secure"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let state = scenario.gh_state();
        let body = if private {
            state["bodies"]["8"].as_str().unwrap()
        } else {
            state["advisories"][0]["description"].as_str().unwrap()
        };
        assert!(
            body.contains(original_writeup),
            "original finding was deleted: {body}"
        );
        assert!(
            body.contains("\"validation_plan\""),
            "validation plan was deleted: {body}"
        );
        assert!(body.contains("Severity: medium"), "{body}");
    }
}

#[test]
fn a_published_finding_keeps_new_reproduction_evidence_private_and_its_grade() {
    let scenario = Scenario::new();
    let commit = scenario.origin_git(&["rev-parse", "main"]);
    let original = format!(
        "Fingerprint: `published-finding`\nAudited commit: `{}`\nDay-shift-approved published description.\n",
        commit.trim()
    );
    let mut github = scenario.gh_state();
    github["advisories"] = json!([{
        "ghsa_id": "GHSA-existing", "description": original,
        "severity": "high", "state": "published",
        "summary": "Existing finding", "html_url": "https://github.com/acme/widgets/security/advisories/GHSA-existing"
    }]);
    scenario.write_gh_state(&github);
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("published-finding")]).to_string()),
    );
    scenario.agent_does_in_session(2, &reproduction_script("reproduced critical single"));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(
        state["advisories"][0]["description"], original,
        "new reproduction evidence became part of a published advisory"
    );
    assert_eq!(
        state["advisories"][0]["severity"], "high",
        "the Day shift grade changed"
    );
}

#[test]
fn a_private_reproduction_accepts_the_rubrics_informational_severity() {
    let scenario = Scenario::new();
    let mut github = scenario.gh_state();
    github["private"] = json!(true);
    scenario.write_gh_state(&github);
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("minimal-impact")]).to_string()),
    );
    scenario.agent_does_in_session(2, &reproduction_script("reproduced informational single"));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let body = state["bodies"]["8"].as_str().unwrap();
    assert!(body.contains("Severity: informational"), "{body}");
    assert!(body.contains("bounded_fixture();"), "{body}");
    assert!(body.contains("Local command: bounded-fixture"), "{body}");
}

#[test]
fn a_finding_triaged_during_reproduction_is_left_unchanged() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("input-size")]).to_string()),
    );
    scenario.agent_does_in_session(2, &format!(r#"
printf '%s\n' '{{"state":"published","severity":"high"}}' | gh api --method PATCH repos/acme/widgets/security-advisories/GHSA-test-test-0001 --input - >/dev/null
{}
"#, reproduction_script("reproduced critical single")));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("triaged during its reproduction"),
        "{}",
        result.stderr
    );
    let state = scenario.gh_state();
    let record = &state["advisories"][0];
    assert_eq!(record["state"], "published");
    assert_eq!(record["severity"], "high");
    assert!(
        !record["description"]
            .as_str()
            .unwrap()
            .contains("bounded_fixture();")
    );
}

#[test]
fn an_informational_advisory_requires_a_day_shift_decision_without_an_invented_grade() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &audit_script(&json!([finding("minimal-impact")]).to_string()),
    );
    scenario.agent_does_in_session(2, &reproduction_script("reproduced informational single"));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("the Day shift must decide its representation"),
        "{}",
        result.stderr
    );
    let state = scenario.gh_state();
    let record = &state["advisories"][0];
    assert_eq!(record["severity"], Value::Null);
    assert!(
        !record["description"]
            .as_str()
            .unwrap()
            .contains("<!-- thirdshift:security-reproduction -->")
    );
    assert!(
        scenario
            .gh_calls_of("api", "--method")
            .iter()
            .all(|call| call[2] != "PATCH")
    );
}

#[test]
fn a_completed_security_audit_sends_one_notification_when_asked() {
    let scenario = Scenario::new();
    scenario.agent_does(&audit_script("[]"));
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result =
        scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let folder = scenario.path("home/.thirdshift/logs/acme/widgets/commands/secure");
    let traces: Vec<_> = fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".logs.jsonl"))
        .collect();
    assert_eq!(traces.len(), 1);
    for line in fs::read_to_string(&traces[0]).unwrap().lines() {
        let request: Value = serde_json::from_str(line).unwrap();
        assert!(request["resourceLogs"].is_array());
    }
    assert!(fs::read_dir(&folder).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .to_string_lossy()
            .ends_with(".spans.jsonl")
    }));
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "done");
    assert_eq!(status["commands"][0]["needs_you_kind"], Value::Null);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Security run: findings recorded"
    );
    assert_eq!(resend.requests()[0].body["to"], "me@example.com");
    assert!(text.contains("Built with claude"), "{text}");
}

#[test]
fn the_notification_lists_each_recorded_finding_without_its_private_write_up() {
    let scenario = Scenario::new();
    let mut github = scenario.gh_state();
    github["advisories"] = json!([{
        "ghsa_id": "GHSA-existing",
        "state": "draft", "severity": "high", "summary": "Existing finding",
        "html_url": "https://github.com/acme/widgets/security/advisories/GHSA-existing",
        "description": "Fingerprint: `existing`\nPrivate existing write-up."
    }]);
    scenario.write_gh_state(&github);
    scenario.agent_does(&audit_script(
        &json!([finding("existing"), finding("new")]).to_string(),
    ));
    scenario.agent_does_in_session(2, &reproduction_script("reproduced medium single"));
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result =
        scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let (_, text) = the_one_notification(&resend);
    assert!(
        text.contains("Security audit recorded 1 new finding(s), 1 already recorded"),
        "{text}"
    );
    assert!(text.contains("high: Existing finding"), "{text}");
    assert!(
        text.contains("https://github.com/acme/widgets/security/advisories/GHSA-existing"),
        "{text}"
    );
    assert!(text.contains("Unchecked input size"), "{text}");
    assert!(text.contains("medium: Unchecked input size"), "{text}");
    assert_eq!(scenario.claude_calls().len(), 2);
    assert!(
        text.contains("https://github.com/acme/widgets/security/advisories/GHSA-test-test-0002"),
        "{text}"
    );
    for private in [
        "Private candidate write-up.",
        "Private existing write-up.",
        "Input reaches storage without a bound.",
        "Private trace.",
        "Private evidence.",
        "No sandbox available.",
        "Exercise a bounded fixture.",
        "Fingerprint:",
        "Local command: bounded-fixture",
        "bounded_fixture();",
    ] {
        assert!(
            !resend.requests()[0].body.to_string().contains(private),
            "leaked {private}: {text}"
        );
    }
}

#[test]
fn a_private_security_notification_lists_existing_and_new_issue_links_without_write_ups() {
    for state in ["OPEN", "CLOSED"] {
        let scenario = Scenario::new();
        let mut github = scenario.gh_state();
        github["private"] = json!(true);
        github["issues"]["8"] = json!(state);
        github["labels"]["8"] = json!(["security-finding"]);
        github["titles"]["8"] = json!("Existing finding");
        github["bodies"]["8"] = json!("Fingerprint: `existing`\nPrivate existing write-up.");
        scenario.write_gh_state(&github);
        scenario.agent_does(&audit_script(
            &json!([finding("existing"), finding("new")]).to_string(),
        ));
        scenario.agent_does_in_session(2, &reproduction_script("reproduced high single"));
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result =
            scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let (_, text) = the_one_notification(&resend);
        for metadata in [
            "Security audit recorded 1 new finding(s), 1 already recorded",
            "Existing finding",
            "https://github.com/acme/widgets/issues/8",
            "high: Unchecked input size",
            "https://github.com/acme/widgets/issues/9",
        ] {
            assert!(text.contains(metadata), "missing {metadata}: {text}");
        }
        for private in [
            "Private candidate write-up.",
            "Private existing write-up.",
            "Input reaches storage without a bound.",
            "Private trace.",
            "Private evidence.",
            "No sandbox available.",
            "Exercise a bounded fixture.",
            "Fingerprint:",
            "Local command: bounded-fixture",
            "bounded_fixture();",
        ] {
            assert!(
                !resend.requests()[0].body.to_string().contains(private),
                "leaked {private}: {text}"
            );
        }
        assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
        assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 3);
    }
}

#[test]
fn security_notifications_follow_email_defaults_and_command_overrides() {
    for (config, args, expected_to) in [
        ("", vec!["secure"], None),
        (
            "[email]\nalways = true\nto = \"default@example.com\"\n",
            vec!["secure"],
            Some("default@example.com"),
        ),
        (
            "[email]\nto = \"default@example.com\"\n",
            vec!["secure", "--email"],
            Some("default@example.com"),
        ),
        (
            "[email]\nalways = true\nto = \"default@example.com\"\n",
            vec!["secure", "no-email"],
            None,
        ),
        (
            "[email]\nto = \"default@example.com\"\n",
            vec!["secure", "--email", "override@example.com"],
            Some("override@example.com"),
        ),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(config);
        scenario.agent_does(&audit_script("[]"));
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result = scenario.run_with_env(&args, &resend_env(&resend));
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        match expected_to {
            Some(to) => {
                the_one_notification(&resend);
                assert_eq!(resend.requests()[0].body["to"], to);
            }
            None => assert!(resend.requests().is_empty()),
        }
    }
}

#[test]
fn a_recording_failure_still_notifies_each_finding_already_recorded() {
    let scenario = Scenario::new();
    let mut github = scenario.gh_state();
    github["advisory_create_fails_after"] = json!(1);
    scenario.write_gh_state(&github);
    let mut first = finding("first");
    first["title"] = json!("First recorded finding");
    let mut second = finding("second");
    second["title"] = json!("Unrecorded finding");
    scenario.agent_does(&audit_script(&json!([first, second]).to_string()));
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result =
        scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["advisories"].as_array().unwrap().len(),
        1
    );
    let (subject, text) = the_one_notification(&resend);
    assert!(subject.ends_with(": audit failed"), "{subject}");
    assert!(text.contains("First recorded finding"), "{text}");
    assert!(
        text.contains("https://github.com/acme/widgets/security/advisories/GHSA-test-test-0001"),
        "{text}"
    );
    assert!(!text.contains("Unrecorded finding"), "{text}");
    assert!(!text.contains("Private candidate write-up."), "{text}");
}

#[test]
fn a_skipped_security_run_sends_no_notification_requested_by_word_or_config() {
    for args in [vec!["secure"], vec!["secure", "email", "me@example.com"]] {
        let scenario = Scenario::new();
        scenario.user_config_is("[email]\nalways = true\nto = \"me@example.com\"\n");
        scenario.issue_labelled(7, &["ready-for-agent"]);
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result = scenario.run_with_env(&args, &resend_env(&resend));
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            result.stderr.contains("Ready issue #7"),
            "{}",
            result.stderr
        );
        assert!(scenario.claude_calls().is_empty());
        assert!(resend.requests().is_empty());
    }
}

#[test]
fn failed_security_audits_send_one_notification_with_a_safe_status() {
    for (script, recording_fails, cause) in [
        (
            audit_script("[]").replace("Security audit: complete", "Security audit: incomplete"),
            false,
            "ended incomplete",
        ),
        (
            audit_script("[{}]"),
            false,
            "validator validate-findings.cjs",
        ),
        (
            audit_script(&json!([finding("new")]).to_string()),
            true,
            "creating a draft repository security advisory failed",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&script);
        if recording_fails {
            // The API consumes the request before rejecting it, avoiding
            // a race between writing stdin and an early fake gh exit.
            let mut github = scenario.gh_state();
            github["advisory_create_fails_after"] = json!(0);
            scenario.write_gh_state(&github);
        }
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result =
            scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let (subject, text) = the_one_notification(&resend);
        assert_eq!(
            subject,
            "[thirdshift] acme/widgets Security run: audit failed"
        );
        assert!(text.contains("Audit:        audit failed"), "{text}");
        assert!(!text.contains("audit complete"), "{text}");
        assert!(!text.contains("Reproduction:"), "{text}");
        assert!(
            result.stderr.contains(cause),
            "expected {cause}: {}",
            result.stderr
        );
        assert!(!text.contains("Cause:"), "{text}");
        assert!(!text.contains("Private candidate write-up."), "{text}");
    }
}

fn assert_base_branch_pre_audit_cause(
    scenario: &Scenario,
    args: &[&str],
    diagnostic: &str,
    cause: &str,
) {
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result = scenario.run_with_env(args, &resend_env(&resend));
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains(diagnostic), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Security run: pre-audit failed"
    );
    assert!(text.contains("Result:       pre-audit failed"), "{text}");
    assert!(text.contains("Audit:        not started"), "{text}");
    assert!(text.contains(&format!("Cause:        {cause}")), "{text}");
    assert!(
        text.contains("Diagnostics:  Check this command's stderr on "),
        "{text}"
    );
    for private in [
        diagnostic,
        "PRIVATE-BRANCH",
        "PRIVATE-COMMIT",
        "create or obtain",
        "git push",
        "pre-audit checks failed",
    ] {
        assert!(
            !resend.requests()[0].body.to_string().contains(private),
            "{text}"
        );
    }
}

#[test]
fn missing_base_branch_reports_a_safe_pre_audit_cause() {
    let scenario = Scenario::new();
    assert_base_branch_pre_audit_cause(
        &scenario,
        &[
            "secure",
            "base",
            "PRIVATE-BRANCH",
            "email",
            "me@example.com",
        ],
        "base branch PRIVATE-BRANCH does not exist on origin or locally",
        "Base branch not found",
    );
}

#[test]
fn other_base_branch_preparation_failures_report_safe_pre_audit_causes() {
    for (setup, diagnostic, cause) in [
        (
            &["checkout", "-q", "-b", "PRIVATE-BRANCH"][..],
            "base branch PRIVATE-BRANCH does not exist on origin; push it first",
            "Base branch not on origin",
        ),
        (
            &["checkout", "-q", "--detach"][..],
            "HEAD is detached; check out the branch",
            "Base branch not selected",
        ),
        (
            &["commit", "--allow-empty", "-qm", "PRIVATE-COMMIT"][..],
            "local main is 1 commit(s) ahead of origin/main; push them first",
            "Base branch ahead of origin",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.launch_git(setup);
        assert_base_branch_pre_audit_cause(
            &scenario,
            &["secure", "email", "me@example.com"],
            diagnostic,
            cause,
        );
    }
}

#[test]
fn pre_audit_github_failures_report_a_safe_cause_without_claiming_an_audit_ran() {
    for (diagnostic, expected_cause) in [
        (
            "GraphQL: API rate limit already exceeded for user ID PRIVATE-IDENTIFIER.",
            "GitHub rate-limit exhaustion",
        ),
        (
            "gh: API rate limit exceeded for PRIVATE-IDENTIFIER. (HTTP 403)",
            "GitHub rate-limit exhaustion",
        ),
        (
            "gh: You have exceeded a secondary rate limit. (HTTP 403)",
            "GitHub rate-limit exhaustion",
        ),
        (
            "gh: Resource not accessible by PRIVATE-IDENTIFIER (HTTP 403)",
            "GitHub dependency failure",
        ),
        (
            "HTTP 502: Bad Gateway (https://api.github.com/graphql)",
            "GitHub dependency failure",
        ),
        (
            "HTTP 502: GraphQL: rate limit configuration unknown",
            "GitHub dependency failure",
        ),
    ] {
        for call in [
            "api --paginate --jq .[] repos/acme/widgets/issues?state=open&labels=ready-for-agent&sort=created&direction=desc&per_page=100",
            "api --paginate --jq .[] repos/acme/widgets/security-advisories?per_page=100",
        ] {
            let scenario = Scenario::new();
            scenario.user_config_is(&format!(
                "[harness.codex]\nmodel = \"gpt-6.1-sol\"\neffort = \"high\"\n[logs]\ndir = {:?}\n",
                scenario.path("custom-logs")
            ));
            scenario.gh_fails(call);
            let mut github = scenario.gh_state();
            github["failing_stderr"] = json!(format!(
                "{diagnostic} token=PRIVATE-CREDENTIAL Private candidate write-up. Private provider/verifier diagnostic."
            ));
            scenario.write_gh_state(&github);
            let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
            let result = scenario.run_with_env(
                &["secure", "harness", "codex", "email", "me@example.com"],
                &resend_env(&resend),
            );
            assert_eq!(result.code, Some(1), "{}", result.stderr);
            assert!(result.stderr.contains(diagnostic), "{}", result.stderr);
            assert!(scenario.claude_calls().is_empty());
            assert!(scenario.codex_calls().is_empty());
            assert!(scenario.gh_calls_of("issue", "create").is_empty());
            assert!(scenario.gh_state()["advisories"].is_null());
            assert!(
                !scenario
                    .path("custom-logs/acme/widgets/commands/secure")
                    .exists()
            );
            let (subject, text) = the_one_notification(&resend);
            assert_eq!(
                subject,
                "[thirdshift] acme/widgets Security run: pre-audit failed"
            );
            assert!(text.contains("Result:       pre-audit failed"), "{text}");
            assert!(text.contains("Audit:        not started"), "{text}");
            assert!(
                text.contains(&format!("Cause:        {expected_cause}")),
                "{text}"
            );
            assert!(
                text.contains("Diagnostics:  Check this command's stderr on "),
                "{text}"
            );
            let started = text
                .lines()
                .find_map(|line| line.strip_prefix("Started (UTC): "))
                .expect(&text);
            chrono::NaiveDateTime::parse_from_str(started, "%Y-%m-%d %H:%M:%S UTC").expect(&text);
            let host = text
                .lines()
                .find_map(|line| line.strip_prefix("Host:         "))
                .expect(&text);
            assert!(
                text.contains(&format!(
                    "Check this command's stderr on {host} at that time"
                )),
                "{text}"
            );
            assert!(text.contains("gpt-6.1-sol · high (configured)"), "{text}");
            assert!(text.contains("Configured:"), "{text}");
            assert!(!text.contains("Built with"), "{text}");
            for private in [
                "PRIVATE-IDENTIFIER",
                "PRIVATE-CREDENTIAL",
                "Private candidate write-up.",
                "GraphQL:",
                "Private provider/verifier diagnostic.",
                diagnostic,
            ] {
                assert!(
                    !resend.requests()[0].body.to_string().contains(private),
                    "{text}"
                );
            }
            for absent in [
                "Session trace:",
                "Command log:",
                "Reproduction:",
                "Session models:",
                "cron.log",
            ] {
                assert!(!text.contains(absent), "{text}");
            }
        }
    }
}

fn assert_pre_audit_response_dependency_cause(response: &str, diagnostic: &str) {
    let scenario = Scenario::new();
    let gh = scenario.path("bin/gh");
    fs::remove_file(&gh).unwrap();
    write_executable(&gh, &format!("#!/bin/sh\nprintf '%s\\n' '{response}'\n"));

    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result = scenario.run_with_env(
        &["secure", "email", "me@example.com"],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_fake"),
        ],
    );

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains(diagnostic), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());

    let requests = resend.requests();
    assert_eq!(requests.len(), 1);
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(text.contains("Audit:        not started"), "{text}");
    assert!(
        !requests[0]
            .body
            .to_string()
            .contains("PRIVATE-INVALID-JSON")
    );
    assert!(
        text.contains("Cause:        GitHub dependency failure"),
        "{text}"
    );
}

#[test]
fn malformed_pre_audit_github_response_has_dependency_cause() {
    assert_pre_audit_response_dependency_cause(
        "PRIVATE-INVALID-JSON",
        "gh api repos/acme/widgets/issues?state=open&labels=ready-for-agent&sort=created&direction=desc&per_page=100 returned invalid JSON",
    );
}

#[test]
fn invalid_schema_pre_audit_github_response_has_dependency_cause() {
    assert_pre_audit_response_dependency_cause("{}", "gh api output has no html_url");
}

#[test]
fn an_unavailable_github_cli_has_a_safe_pre_audit_dependency_cause() {
    let scenario = Scenario::new();
    link_git(&scenario);
    fs::remove_file(scenario.path("bin/gh")).unwrap();
    let path = scenario.path("bin");
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let mut env = resend_env(&resend).to_vec();
    env.push(("PATH", path.to_str().unwrap()));
    let result = scenario.run_with_env(&["secure", "email", "me@example.com"], &env);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("could not run gh"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
    let (subject, text) = the_one_notification(&resend);
    assert!(subject.ends_with(": pre-audit failed"), "{subject}");
    assert!(text.contains("Audit:        not started"), "{text}");
    assert!(
        text.contains("Cause:        GitHub dependency failure"),
        "{text}"
    );
    assert!(!text.contains("could not run gh"), "{text}");
    assert!(!text.contains("No such file"), "{text}");
}

#[test]
fn an_interrupted_security_audit_sends_one_notification() {
    let scenario = Scenario::new();
    scenario.agent_does("touch \"$HOME/../started\"\nsleep 60\n");
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result = scenario.run_and_signal_with_env(
        &["secure", "email", "me@example.com"],
        &resend_env(&resend),
        "started",
        "TERM",
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets Security run: interrupted"
    );
    assert!(text.contains("Audit:        interrupted"), "{text}");
    assert!(!text.contains("Cause:"), "{text}");
}

#[test]
fn a_failed_send_keeps_the_security_runs_success_or_failure() {
    for script in [audit_script("[]"), "exit 3".to_string()] {
        let scenario = Scenario::new();
        scenario.agent_does(&script);
        let without_email = scenario.run(&["secure"]);
        scenario.origin_has_commit("main", "new-work.txt", "new work", "Move Base branch");
        let resend = ResendStandIn::replying(500, "upstream exploded");
        let result =
            scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
        assert_eq!(result.code, without_email.code, "{}", result.stderr);
        assert_eq!(result.stdout, without_email.stdout);
        the_one_notification(&resend);
        assert!(
            result
                .stderr
                .contains("warning: could not send the Run notification"),
            "{}",
            result.stderr
        );
    }
}

#[test]
fn notification_checks_precede_security_skip_checks_and_work() {
    for (args, env, cause) in [
        (vec!["secure", "email"], true, "no email address"),
        (
            vec!["secure", "email", "me@example.com"],
            false,
            "no Resend API key",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["ready-for-agent"]);
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let env = if env {
            resend_env(&resend).to_vec()
        } else {
            vec![("THIRDSHIFT_RESEND_URL", resend.url())]
        };
        let result = scenario.run_with_env(&args, &env);
        scenario.assert_rejected_before_any_work(&result, cause);
        assert!(scenario.gh_calls().is_empty());
        assert!(resend.requests().is_empty());
    }
}

#[test]
fn unchanged_base_is_remembered_separately_for_each_branch() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("stable", "main", &["Stable branch"]);
    scenario.agent_does(&audit_script("[]"));

    let main_audit = scenario.run(&["secure", "base", "main"]);
    assert_eq!(main_audit.code, Some(0), "{}", main_audit.stderr);
    std::thread::sleep(std::time::Duration::from_millis(20));

    let stable_audit = scenario.run(&["secure", "base", "stable"]);
    assert_eq!(stable_audit.code, Some(0), "{}", stable_audit.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);

    let unchanged_main = scenario.run(&["secure", "base", "main"]);
    assert_eq!(unchanged_main.code, Some(0), "{}", unchanged_main.stderr);
    assert_eq!(
        scenario.claude_calls().len(),
        2,
        "An unchanged main must skip after stable is audited: {}",
        unchanged_main.stderr
    );
    let activity =
        fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(activity.contains(
        "Security run skipped: Base branch main hasn't changed since the last completed Security audit"
    ));
}

#[test]
fn failed_security_notification_excludes_description_from_killed_verifier() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{}{}",
            audit_script(&json!([finding("private-candidate")]).to_string()),
            support::leaves_running("Private candidate write-up.")
        ),
    );
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}printf '%s\\n' 'Security audit: incomplete' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
            support::leaves_running("Private candidate write-up.")
        ),
    );
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result =
        scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    let (subject, text) = the_one_notification(&resend);
    assert!(subject.ends_with(": audit failed"), "{subject}");
    assert!(text.contains("Audit:        audit failed"), "{text}");
    assert!(
        !resend.requests()[0]
            .body
            .to_string()
            .contains("Private candidate write-up."),
        "finding description leaked to Resend: {text}"
    );
}

#[test]
fn security_notification_omits_private_background_trace() {
    let scenario = Scenario::new();
    scenario.agent_does(&support::leaves_running(
        "Private exploit trace: POST /admin/debug with token",
    ));
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result =
        scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let (_, text) = the_one_notification(&resend);
    assert!(!text.contains("Private exploit trace"), "{text}");
}

#[test]
fn a_reproduction_failure_notifies_recorded_metadata_without_private_evidence() {
    for private in [false, true] {
        for failed_script in [
            reproduction_script("incomplete"),
            "printf '%s\\n' 'Private provider diagnostic.' >&2\nexit 3\n".to_string(),
        ] {
            let scenario = Scenario::new();
            let mut github = scenario.gh_state();
            github["private"] = json!(private);
            scenario.write_gh_state(&github);
            let mut first = finding("first");
            first["title"] = json!("First recorded finding");
            let mut second = finding("second");
            second["title"] = json!("Second recorded finding");
            scenario.agent_does_in_session(
                1,
                &audit_script(&json!([first, second, finding("third")]).to_string()),
            );
            scenario.agent_does_in_session(2, &reproduction_script("reproduced high single"));
            scenario.agent_does_in_session(3, &failed_script);
            let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
            let result = scenario.run_with_env(
                &["secure", "security-fix", "email", "me@example.com"],
                &resend_env(&resend),
            );
            assert_eq!(result.code, Some(1), "{}", result.stderr);
            if failed_script.contains("incomplete") {
                assert!(
                    result
                        .stderr
                        .contains("Security reproduction ended without the final line"),
                    "{}",
                    result.stderr
                );
            }
            let (subject, text) = the_one_notification(&resend);
            assert!(subject.ends_with(": reproduction failed"), "{subject}");
            for line in [
                "Result:       reproduction failed",
                "Audit:        audit complete",
                "Reproduction: 2 failed",
                "Session trace:",
                "security-reproduction-2.logs.jsonl",
                "Command log:",
            ] {
                assert!(text.contains(line), "missing {line}: {text}");
            }
            assert!(text.contains("high: First recorded finding"), "{text}");
            assert!(text.contains("Second recorded finding"), "{text}");
            assert!(!text.contains("high: Second recorded finding"), "{text}");
            let url = if private {
                "https://github.com/acme/widgets/issues/"
            } else {
                "https://github.com/acme/widgets/security/advisories/"
            };
            assert_eq!(text.matches(url).count(), 3, "{text}");
            for evidence in [
                "Private candidate write-up.",
                "Private trace.",
                "Private evidence.",
                "Local command: bounded-fixture",
                "bounded_fixture();",
                "Cause:",
                "Private provider diagnostic.",
            ] {
                assert!(
                    !resend.requests()[0].body.to_string().contains(evidence),
                    "leaked {evidence}: {text}"
                );
            }
            assert_eq!(scenario.claude_calls().len(), 3);
            let state = scenario.gh_state();
            assert!(state["prs"].as_array().unwrap().is_empty());
            if private {
                assert_eq!(state["bodies"].as_object().unwrap().len(), 3);
                assert!(
                    state["bodies"]["8"]
                        .as_str()
                        .unwrap()
                        .contains("Severity: high")
                );
            } else {
                let records = state["advisories"].as_array().unwrap();
                assert_eq!(records.len(), 3);
                assert_eq!(records[0]["severity"], "high");
                assert!(records[1]["severity"].is_null());
                assert!(records[2]["severity"].is_null());
            }
        }
    }
}

#[test]
fn spec_review_short_private_poc_must_not_be_dispatched() {
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    let description = state["advisories"][0]["description"]
        .as_str()
        .unwrap()
        .replace("bounded_fixture();", "bypass_login();");
    state["advisories"][0]["description"] = json!(description);
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(
        1,
        "printf 'Proposed public issue:\\nTitle: Bound accepted input\\nBody:\\nReject oversized input. Proof-of-concept: bypass_login(); Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
    );
    let result = scenario.run(&["secure", "security-fix"]);
    let state = scenario.gh_state();
    assert_eq!(
        result.code,
        Some(1),
        "staged text copying the private proof-of-concept was dispatched: {}",
        result.stderr,
    );
    assert!(
        result.stderr.contains("includes private write-up text"),
        "{}",
        result.stderr
    );
    assert!(
        state["bodies"].get("8").is_none(),
        "rejected staged text must not create a public issue: {}",
        state["bodies"]
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(state["prs"].as_array().unwrap().is_empty());
}

#[test]
fn spec_review_current_day_shift_severity_must_decide_fix_priority() {
    let scenario = with_reproduced_findings(&["critical", "high"]);
    let mut state = scenario.gh_state();
    // This models changing severity in GitHub without rewriting the historical
    // reproduction outcome, which must remain useful private evidence.
    state["advisories"][0]["severity"] = json!("low");
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(1, r#"
case "$FAKE_CLAUDE_PROMPT" in
  *GHSA-finding-1*) record=GHSA-finding-1 ;;
  *GHSA-finding-0*) record=GHSA-finding-0 ;;
  *) exit 1 ;;
esac
printf 'Proposed public issue:\nTitle: Bound accepted input\nBody:\nReject oversized input. Private record: https://github.com/acme/widgets/security/advisories/%s\n' "$record" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    scenario.agent_does_for(8, implement_fix());
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.claude_calls();
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("GHSA-finding-1"),
        "the current high finding must precede the finding regraded low: {prompt}",
    );
}

#[test]
fn a_later_audit_keeps_the_link_to_a_successfully_dispatched_private_fix() {
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    state["private"] = json!(true);
    state["bodies"]["7"] = state["advisories"][0]["description"].clone();
    state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
    scenario.write_gh_state(&state);
    scenario.agent_does_for(
        7,
        &implement_fix()
            .replace("issue-8", "issue-7")
            .replace("#8", "#7"),
    );
    let first = scenario.run(&["secure", "security-fix"]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "OPEN");
    assert!(
        state["bodies"]["7"]
            .as_str()
            .unwrap()
            .contains("Fix Ticket: https://github.com/acme/widgets/issues/7")
    );
    assert_eq!(state["issues"].as_object().unwrap().len(), 1);

    scenario.origin_has_commit(
        "main",
        "other-work.txt",
        "other work",
        "Advance Base branch",
    );
    scenario.agent_does_in_session(2, &audit_script(&json!([finding("finding-0")]).to_string()));
    scenario.agent_does_for_in_session(7, 2, &reproduction_script("reproduced high single"));
    let second = scenario.run(&["secure", "security-fix"]);
    assert_eq!(second.code, Some(0), "{}", second.stderr);
    let state = scenario.gh_state();
    assert_eq!(
        state["issues"].as_object().unwrap().len(),
        1,
        "another fix Ticket was published for a finding whose first fix succeeded: {state}"
    );
    assert!(
        state["bodies"]["7"]
            .as_str()
            .unwrap()
            .contains("Fix Ticket: https://github.com/acme/widgets/issues/7")
    );
}

#[test]
fn spec_followup_common_poc_token_must_not_block_a_terse_fix_ticket() {
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    let description = state["advisories"][0]["description"]
        .as_str()
        .unwrap()
        .replace("bounded_fixture();", "assert (\n    input\n) == 'overflow'");
    state["advisories"][0]["description"] = json!(description);
    scenario.write_gh_state(&state);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, implement_fix());
    let result = scenario.run(&["secure", "security-fix"]);
    let state = scenario.gh_state();
    let ticket = state["bodies"]["8"].as_str().unwrap();
    assert_eq!(
        ticket,
        "Reject oversized input. Private record: https://github.com/acme/widgets/security/advisories/GHSA-finding-0",
    );
    assert!(!ticket.contains("assert (") && !ticket.contains("overflow"));
    assert_eq!(
        result.code,
        Some(0),
        "a valid terse Ticket was rejected because it shares an ordinary token with the private test: {}",
        result.stderr,
    );
    assert_eq!(scenario.claude_calls().len(), 2);
    assert_eq!(state["prs"].as_array().unwrap().len(), 1);
}

#[test]
fn claude_security_progress_ignores_synthetic_api_errors() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        r#"
printf '%s\n' '{{"type":"assistant","message":{{"model":"claude-opus-4-8","content":[]}}}}'
printf '%s\n' '{{"type":"assistant","parent_tool_use_id":null,"is_api_error_message":true,"error":"rate_limit","message":{{"model":"<synthetic>","role":"assistant","content":[{{"type":"text","text":"API Error: Rate limit reached"}}]}}}}'
printf '%s\n' '{{"type":"assistant","message":{{"model":"claude-opus-4-8","content":[]}}}}'
{}
"#,
        audit_script("[]")
    ));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let command_log = security_command_log(&scenario);
    for text in [&result.stderr, &command_log] {
        assert!(!text.contains("Model: <synthetic>"), "{text}");
        assert_eq!(
            text.matches("security-audit: Model: claude-opus-4-8")
                .count(),
            1,
            "{text}"
        );
    }
}

#[test]
fn grok_security_audits_log_the_model_that_answered() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
node -e 'const fs = require("fs"); const calls = JSON.parse(fs.readFileSync(process.env.FAKE_GROK_RECORD, "utf8")); process.stdout.write(calls[calls.length - 1].prompt);' > prompt.txt
output=$(sed -n 's/^Output directory: `\(.*\)`\.$/\1/p' prompt.txt)
test -n "$output"
printf '%s\n' '[]' > "$output/findings.json"
printf '%s\n' '[]' > "$output/coverage-ledger.json"
printf '%s\n' '{"run_status":"complete"}' > "$output/run-metadata.json"
printf '%s\n' '{"type":"assistant","message":{"model":"grok-4.7","content":[]}}'
printf '%s\n' 'Security audit: complete' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = scenario.run(&["secure", "harness", "grok", "model", "grok-4.7"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let command_log = security_command_log(&scenario);
    for text in [&result.stderr, &command_log] {
        assert!(text.contains("security-audit: Model: grok-4.7"), "{text}");
    }
}

#[test]
fn muse_security_audits_log_the_answering_model_from_the_session_record() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        r#"
export FAKE_CLAUDE_PROMPT="$(node -e 'const fs = require("fs"); const calls = JSON.parse(fs.readFileSync(process.env.FAKE_MUSE_RECORD, "utf8")); process.stdout.write(calls[calls.length - 1].prompt);')"
{}
root="$HOME/.local/share/muse/sessions/2026/10/06/fake-muse-1"
mkdir -p "$root"
printf '%s\n' '{{"stream":{{"id":"child"}},"payload":{{"kind":"run","event":{{"kind":"model_completed","model":"child-model"}}}}}}' '{{"stream":{{"id":"fake-muse-1"}},"payload":{{"kind":"run","event":{{"kind":"model_completed","model":"muse-spark-1.3"}}}}}}' > "$root/session.jsonl"
"#,
        audit_script("[]")
    ));
    let result =
        scenario.run_with_env(&["secure", "harness", "muse"], &[("FAKE_MUSE_NO_LOG", "1")]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("security-audit: Model: muse-spark-1.3"),
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("Model: child-model"),
        "{}",
        result.stderr
    );
}

#[test]
fn opencode_security_audits_log_the_answering_model_from_the_export() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        r#"
export FAKE_CLAUDE_PROMPT="$(node -e 'const fs = require("fs"); const calls = JSON.parse(fs.readFileSync(process.env.FAKE_OPENCODE_RECORD, "utf8")); process.stdout.write(calls[calls.length - 1].prompt);')"
{}
"#,
        audit_script("[]")
    ));
    let script = scenario.path("export-model.sh");
    fs::write(&script, format!(
        r#"node -e 'const fs = require("fs"); const p = process.argv[1]; const record = JSON.parse(fs.readFileSync(p, "utf8")); record.info.model = {{id:"title-model",providerID:"title"}}; for (const message of record.messages) {{ if (message.type === "assistant") message.model = {{id:"MiMo-V2.6-Pro",providerID:"primalabs"}}; }} fs.writeFileSync(p, JSON.stringify(record));' "{}"
"#,
        scenario.path("opencode-calls.fake-opencode-1.export.json").display()
    )).unwrap();
    let result = scenario.run_with_env(
        &["secure", "harness", "opencode"],
        &[("FAKE_OPENCODE_EXPORT_SCRIPT", script.to_str().unwrap())],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("security-audit: Model: primalabs/MiMo-V2.6-Pro"),
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("Model: title/"),
        "{}",
        result.stderr
    );
}

#[test]
fn review_failed_fix_before_claim_is_not_retried_by_pickup() {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.agent_does_in_session(
        1,
        &format!(
            "{}gh fake fails 'label create in-progress'\n",
            stage_fix("GHSA-finding-0")
        ),
    );
    scenario.agent_does_for(8, implement_fix());

    let failed = scenario.run(&["secure", "security-fix"]);
    assert_eq!(failed.code, Some(1), "{}", failed.stderr);
    assert!(
        failed.stderr.contains("could not make the Claim on #8"),
        "{}",
        failed.stderr
    );

    let mut state = scenario.gh_state();
    state["failing"] = json!([]);
    scenario.write_gh_state(&state);
    scenario.issue_timeline(
        8,
        &[(support::TimelineEvent::Labelled("ready-for-agent"), 20)],
    );

    let pickup = scenario.run(&["pickup"]);
    assert_eq!(pickup.code, Some(0), "{}", pickup.stderr);
    assert!(
        !pickup.stderr.contains("taking Ready issue #8"),
        "Pickup retried a failed Security fix: {}",
        pickup.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn review_failed_fix_marker_write_error_still_pauses_security() {
    assert_a_failed_fix_record_outage_pauses_security("high");
}

#[test]
fn a_failed_fix_still_pauses_after_its_failure_record_patch_is_rejected() {
    assert_a_failed_fix_record_outage_pauses_security("critical");
}

#[test]
fn a_failed_fix_preserves_private_edits_made_during_its_run_and_still_pauses() {
    for private in [false, true] {
        let scenario = with_reproduced_findings(&["high"]);
        let issue = if private { 7 } else { 8 };
        let mut state = scenario.gh_state();
        let original = state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .to_string();
        if private {
            state["private"] = json!(true);
            state["bodies"]["7"] = json!(original);
            state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
            scenario.write_gh_state(&state);
        } else {
            scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
        }
        let description = format!(
            "{original}\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/{issue}\nFix Run: pending\nDay shift note: keep the input contract.\n"
        );
        let (path, field) = if private {
            ("issues/7", "body")
        } else {
            ("security-advisories/GHSA-finding-0", "description")
        };
        scenario.agent_does_for(issue, &format!(
            "gh api --method PATCH repos/acme/widgets/{path} --input - <<'RECORD'\n{}\nRECORD\nexit 1\n",
            json!({field: description})
        ));
        let failed = scenario.run(&["secure", "security-fix"]);
        assert_eq!(failed.code, Some(1), "{}", failed.stderr);
        let state = scenario.gh_state();
        let description = if private {
            &state["bodies"]["7"]
        } else {
            &state["advisories"][0]["description"]
        }
        .as_str()
        .unwrap();
        assert!(
            description.contains("Day shift note: keep the input contract."),
            "{description}"
        );
        let next = scenario.run(&["secure", "security-fix"]);
        assert_eq!(next.code, Some(0), "{}", next.stderr);
        assert!(
            next.stderr
                .contains(&format!("failed Security fix #{issue} is still open")),
            "{}",
            next.stderr
        );
        assert_eq!(scenario.claude_calls().len(), if private { 1 } else { 2 });
    }
}

#[test]
fn a_changed_fix_link_refuses_completion_without_a_patch() {
    for private in [false, true] {
        let scenario = with_reproduced_findings(&["high"]);
        let dispatched = if private { 7 } else { 8 };
        let mut state = scenario.gh_state();
        let original = state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .to_string();
        if private {
            state["private"] = json!(true);
            state["bodies"]["7"] = json!(original);
            state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
            scenario.write_gh_state(&state);
        } else {
            scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
        }
        let changed = format!(
            "{original}\n<!-- thirdshift:security-fix -->\nFix Ticket: https://github.com/acme/widgets/issues/9\nFix Run: pending\nDay shift changed the dispatched Ticket.\n"
        );
        let (path, field) = if private {
            ("issues/7", "body")
        } else {
            ("security-advisories/GHSA-finding-0", "description")
        };
        scenario.agent_does_for(dispatched, &format!(
            "gh api --method PATCH repos/acme/widgets/{path} --input - <<'RECORD'\n{}\nRECORD\nexit 1\n", json!({field: changed})
        ));
        let failed = scenario.run(&["secure", "security-fix"]);
        assert_eq!(failed.code, Some(1), "{}", failed.stderr);
        assert!(
            failed
                .stderr
                .contains("could not record the failed Security fix's ending"),
            "{}",
            failed.stderr
        );
        let state = scenario.gh_state();
        let description = if private {
            &state["bodies"]["7"]
        } else {
            &state["advisories"][0]["description"]
        };
        assert_eq!(description.as_str().unwrap(), changed);
        let patch_prefix = vec![
            "api".to_string(),
            "--method".to_string(),
            "PATCH".to_string(),
            format!("repos/acme/widgets/{path}"),
        ];
        // Only the pending link and the session's explicit link edit may PATCH.
        assert_eq!(
            scenario
                .gh_calls()
                .iter()
                .filter(|call| call.starts_with(&patch_prefix))
                .count(),
            2
        );
    }
}

#[test]
fn missing_or_malformed_audited_commits_refuse_reproduction_before_worktree_acquisition() {
    for private in [false, true] {
        for (header, expected) in [
            ("", "no audited commit"),
            ("Audited commit: no backticks", "no audited commit"),
            ("Audited commit: `bad`", "invalid audited commit"),
            (
                "Audited commit: `gggggggggggggggggggggggggggggggggggggggg`",
                "invalid audited commit",
            ),
        ] {
            let scenario = Scenario::new();
            let mut state = scenario.gh_state();
            let description = format!("Fingerprint: `invalid-commit`\n{header}\nPrivate evidence.");
            if private {
                state["private"] = json!(true);
                state["bodies"]["7"] = json!(description);
                state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
            } else {
                state["advisories"] = json!([{
                    "ghsa_id": "GHSA-invalid", "state": "draft", "severity": null,
                    "summary": "Unchecked input size", "html_url": "https://github.com/acme/widgets/security/advisories/GHSA-invalid", "description": description
                }]);
            }
            scenario.agent_does_in_session(
                1,
                &audit_script_with_records(
                    &scenario,
                    &json!([finding("invalid-commit")]).to_string(),
                    &state,
                ),
            );
            let failed = scenario.run(&["secure"]);
            assert_eq!(failed.code, Some(1), "{}", failed.stderr);
            assert!(failed.stderr.contains(expected), "{}", failed.stderr);
            assert_eq!(
                scenario.claude_calls().len(),
                1,
                "reproduction session started"
            );
            assert!(
                !failed.stderr.contains("widgets-security-reproduce"),
                "reproduction Worktree acquisition started: {}",
                failed.stderr
            );
            assert_eq!(scenario.entries("work"), vec![REPO]);
            assert_eq!(
                scenario
                    .launch_git(&["worktree", "list", "--porcelain"])
                    .matches("worktree ")
                    .count(),
                1
            );
            assert!(scenario.gh_calls().iter().all(|call| !call.starts_with(&[
                "api".into(),
                "--method".into(),
                "PATCH".into()
            ])));
        }
    }
}

fn assert_a_failed_fix_record_outage_pauses_security(severity: &str) {
    let scenario = with_reproduced_findings(&[severity]);
    scenario.agent_does_in_session(1, &stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(
        8,
        "gh fake fails 'api --method PATCH repos/acme/widgets/security-advisories/GHSA-finding-0'\nexit 1\n",
    );

    let failed = scenario.run(&["secure", "security-fix"]);
    assert_eq!(failed.code, Some(1), "{}", failed.stderr);
    assert!(
        failed
            .stderr
            .contains("could not record the failed Security fix's ending"),
        "{}",
        failed.stderr
    );
    assert_eq!(scenario.gh_state()["issues"]["8"], "OPEN");
    assert_eq!(
        scenario.issue_labels(8),
        vec!["security-fix", "in-progress"]
    );

    let mut state = scenario.gh_state();
    state["failing"] = json!([]);
    scenario.write_gh_state(&state);
    scenario.agent_does(&audit_script("[]"));

    let next = scenario.run(&["secure", "security-fix"]);
    assert_eq!(next.code, Some(0), "{}", next.stderr);
    assert!(
        next.stderr.contains("failed Security fix #8 is still open"),
        "Security resumed with an open failed fix: {}",
        next.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 2);
}

#[test]
fn audit_notifications_keep_record_order_and_reproduce_only_untriaged_records() {
    for private in [false, true] {
        let scenario = Scenario::new();
        let mut state = scenario.gh_state();
        state["private"] = json!(private);
        for (number, fingerprint, title) in [
            (8, "first-bound", "First bound"),
            (9, "third-bound", "Third bound"),
        ] {
            let description = format!("Day shift write-up\nFingerprint: `{fingerprint}`\n");
            if private {
                let number = number.to_string();
                state["issues"][&number] = json!("OPEN");
                state["labels"][&number] = json!(["security-finding"]);
                state["titles"][&number] = json!(title);
                state["bodies"][&number] = json!(description);
            } else {
                if !state["advisories"].is_array() {
                    state["advisories"] = json!([]);
                }
                state["advisories"].as_array_mut().unwrap().push(json!({
                    "ghsa_id": format!("GHSA-{number}"), "description": description,
                    "state": "draft", "severity": "low", "summary": title,
                    "html_url": format!("https://github.com/acme/widgets/security/advisories/GHSA-{number}")
                }));
            }
        }
        scenario.write_gh_state(&state);
        let first = finding("first-bound");
        let mut second = finding("second-bound");
        second["title"] = json!("Second bound");
        let third = finding("third-bound");
        scenario.agent_does(&audit_script(&json!([first, second, third]).to_string()));
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result =
            scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
        assert_eq!(result.code, Some(0), "{private}: {}", result.stderr);
        assert!(
            result
                .stderr
                .contains("1 new finding(s), 2 already recorded"),
            "{}",
            result.stderr
        );
        assert_eq!(
            scenario.claude_calls().len(),
            2,
            "audit and the new finding's reproduction"
        );
        let (_, text) = the_one_notification(&resend);
        for title in ["First bound", "Second bound", "Third bound"] {
            assert_eq!(text.matches(title).count(), 1, "{text}");
        }
        assert!(
            text.find("First bound").unwrap() < text.find("Second bound").unwrap()
                && text.find("Second bound").unwrap() < text.find("Third bound").unwrap(),
            "{text}"
        );
        assert!(!text.contains("Day shift write-up"), "{text}");
        assert!(!text.contains("Private candidate write-up."), "{text}");
    }
}

#[test]
fn historical_fix_links_are_validated_in_both_storage_adapters_before_work() {
    for private in [false, true] {
        for (link, diagnostic) in [
            (
                "malformed",
                "the private record has an invalid fix Ticket link",
            ),
            (
                "https://github.com/other/widgets/issues/8",
                "the private record's fix Ticket belongs to another repository",
            ),
        ] {
            let scenario = with_reproduced_findings(&["high"]);
            let mut state = scenario.gh_state();
            let text = format!(
                "{}\n<!-- thirdshift:security-fix -->\nFix Ticket: {link}\nFix Run: pending\n",
                state["advisories"][0]["description"].as_str().unwrap()
            );
            if private {
                state["private"] = json!(true);
                state["bodies"]["7"] = json!(text);
                state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
            } else {
                state["advisories"][0]["description"] = json!(text);
            }
            scenario.write_gh_state(&state);
            let result = scenario.run(&["secure", "security-fix"]);
            assert_eq!(result.code, Some(1), "{}", result.stderr);
            assert!(result.stderr.contains(diagnostic), "{}", result.stderr);
            assert!(!result.stderr.contains("Private candidate write-up."));
            assert!(scenario.claude_calls().is_empty());
        }
    }
}

#[test]
fn a_security_fix_spec_rejects_a_ticket_outside_its_repository_before_labelling_or_dispatch() {
    let scenario = with_reproduced_findings(&["high"]);
    let mut state = scenario.gh_state();
    state["private"] = json!(true);
    state["bodies"]["7"] = json!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .replace("high single", "high spec")
            .replace("Fix size: single", "Fix size: spec")
    );
    state["labels"]["7"] = json!(["security-finding", "needs-triage"]);
    state["issue_urls"] = json!({"8": "https://github.com/other/widgets/issues/8"});
    scenario.write_gh_state(&state);
    scenario.agent_does_for_in_session(7, 1, r#"
gh issue create --title 'Bound storage input' --body 'Bound storage input. Private record: https://github.com/acme/widgets/issues/7' --label ready-for-agent
gh fake sub-issues 7 '[8]'
printf '%s\n' 'Security fix Spec: https://github.com/acme/widgets/issues/7' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    let result = scenario.run(&["secure", "security-fix"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("a Security fix Spec's Ticket must be in the fix issue's repository"),
        "{}",
        result.stderr
    );
    let state = scenario.gh_state();
    assert_eq!(
        state["labels"]["7"],
        json!(["security-finding", "needs-triage"])
    );
    assert_eq!(state["labels"]["8"], json!(["ready-for-agent"]));
    assert!(state["prs"].as_array().unwrap().is_empty());
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn review_acquisition_failure_is_pre_audit() {
    let scenario = Scenario::new();
    fs::create_dir(scenario.path("work/widgets-security")).unwrap();
    fs::write(
        scenario.path("work/widgets-security/keep.txt"),
        "unowned user work",
    )
    .unwrap();
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result =
        scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
    let (subject, text) = the_one_notification(&resend);
    assert!(
        subject.ends_with(": pre-audit failed"),
        "{subject}\n{text}\n{}",
        result.stderr
    );
    assert!(text.contains("Audit:        not started"), "{text}");
    assert!(
        text.contains("Cause:        pre-audit checks failed"),
        "{text}"
    );
}

#[test]
fn review_malformed_github_response_keeps_dependency_category() {
    let scenario = Scenario::new();
    fs::create_dir(scenario.path("bin/original")).unwrap();
    fs::rename(scenario.path("bin/gh"), scenario.path("bin/original/gh")).unwrap();
    write_executable(
        &scenario.path("bin/gh"),
        r#"#!/bin/sh
if [ "$1" = api ] && [ "$2" = --paginate ]; then
  printf '%s' 'PRIVATE_RESPONSE_NOT_JSON'
  exit 0
fi
exec "$(dirname "$0")/original/gh" "$@"
"#,
    );
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result =
        scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty());
    let (subject, text) = the_one_notification(&resend);
    assert!(subject.ends_with(": pre-audit failed"), "{subject}");
    assert!(text.contains("Audit:        not started"), "{text}");
    assert!(!text.contains("PRIVATE_RESPONSE_NOT_JSON"), "{text}");
    assert!(
        text.contains("Cause:        GitHub dependency failure"),
        "{text}\n{}",
        result.stderr
    );
}

#[test]
fn invalid_parent_issue_url_retains_github_dependency_cause() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    fs::create_dir(scenario.path("bin/original")).unwrap();
    fs::rename(scenario.path("bin/gh"), scenario.path("bin/original/gh")).unwrap();
    write_executable(
        &scenario.path("bin/gh"),
        r#"#!/bin/sh
if [ "$1" = api ] && [ "$2" = graphql ]; then
    printf '%s\n' '{"data":{"repository":{"issue":{"labelled":{"nodes":[],"pageInfo":{"hasNextPage":false}},"parent":{"url":"PRIVATE-INVALID-PARENT"}}}}}'
    exit 0
fi
exec "$(dirname "$0")/original/gh" "$@"
"#,
    );

    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
    let result = scenario.run_with_env(
        &["secure", "email", "me@example.com"],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_fake"),
        ],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("not a GitHub issue URL: PRIVATE-INVALID-PARENT"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
    let requests = resend.requests();
    assert_eq!(requests.len(), 1);
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(text.contains("Audit:        not started"), "{text}");
    assert!(!text.contains("PRIVATE-INVALID-PARENT"), "{text}");
    assert!(
        text.contains("Cause:        GitHub dependency failure"),
        "{text}"
    );
}

#[test]
fn an_audit_only_security_run_does_not_check_the_security_review_harness() {
    let scenario = Scenario::new();
    // A Model Codex's catalog lacks: checking it would fail the run.
    scenario.user_config_is(
        "[security]\nharness = \"codex\"\nreview = true\n\
         [harness.codex]\nmodel = \"no-such-model\"\n",
    );
    scenario.agent_does(&audit_script("[]"));
    let result = scenario.run(&["secure", "harness", "claude"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        scenario.codex_calls().is_empty(),
        "{:?}",
        scenario.codex_calls()
    );
    assert!(
        !result.stderr.contains("no-such-model"),
        "{}",
        result.stderr
    );
}

/// A User config that lets `limit` Commands run on a repository at once,
/// with no settling, so a Pickup run takes an issue as soon as it is Ready.
fn command_limit(limit: usize) -> String {
    format!("[pass]\ncommand_limit = {limit}\n[pickup]\nwait_minutes = 0\n")
}

/// A script that touches `started-<name>` in the scenario root, then waits
/// there until the test touches `release-<name>`, or until the scenario is
/// gone.
fn waits_on(name: &str) -> String {
    format!(
        r#"
root=$(dirname "$FAKE_CLAUDE_RECORD")
touch "$root/started-{name}"
while [ -d "$root" ] && [ ! -e "$root/release-{name}" ]; do sleep 0.05; done
"#
    )
}

fn release(scenario: &Scenario, name: &str) {
    fs::write(scenario.path(&format!("release-{name}")), "").unwrap();
}

/// The Pickup reservation refs on origin, one line each.
fn reservations(scenario: &Scenario) -> String {
    scenario
        .origin_git(&[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads/thirdshift/pickup",
        ])
        .trim_end()
        .to_string()
}

/// `stderr` after the line saying the Security run is starting.
fn after_start(stderr: &str) -> &str {
    support::after_start(stderr, "Security run starting")
}

#[test]
fn at_a_command_limit_of_3_a_security_run_audits_beside_a_pickup_run_and_an_architect_run_in_a_worktree_of_its_own()
 {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    scenario.user_config_is(&command_limit(3));
    scenario.issue_is(7, "OPEN");
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does_for(
        7,
        &format!(
            "{}{}",
            waits_on("7"),
            r#"
echo 7 > issue-7.txt
git add issue-7.txt
git commit -q -m "Work on 7"
gh pr create --base main --head issue-7 --title "Work on 7" --body "Closes #7"
"#
        ),
    );
    // The Security audit records what it found; the Architecture review
    // waits, then publishes its plan.
    scenario.agent_does(&format!(
        r#"
case "$FAKE_CLAUDE_PROMPT" in
*"Output directory: "*)
{audit}
;;
*)
pwd > "$(dirname "$FAKE_CLAUDE_RECORD")/architect-cwd"
{waits}
url=$(gh issue create --title "Deepen the session module" --body "The plan" --label needs-triage)
printf 'Architecture review plan: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
;;
esac
"#,
        audit = audit_script("[]"),
        waits = waits_on("architect"),
    ));
    let pickup = scenario.run_until(&["pickup"], &[], "started-7");
    let architect = scenario.run_until(&["architect", "--plan-only"], &[], "started-architect");
    let reviewing = fs::read_to_string(scenario.path("architect-cwd")).unwrap();

    let audited = scenario.run(&["secure"]);

    assert_eq!(audited.code, Some(0), "{}", audited.stderr);
    assert!(
        audited
            .stderr
            .contains("Security audit recorded 0 new finding(s), 0 already recorded"),
        "{}",
        audited.stderr
    );
    let audit = scenario
        .claude_calls()
        .into_iter()
        .find(|call| {
            call["prompt"]
                .as_str()
                .unwrap()
                .contains("Output directory: ")
        })
        .unwrap();
    assert!(
        audit["cwd"]
            .as_str()
            .unwrap()
            .ends_with("/work/widgets-security"),
        "{audit}"
    );
    assert!(
        reviewing.trim_end().ends_with("/work/widgets-architect"),
        "{reviewing}"
    );
    assert!(
        scenario
            .entries("work")
            .contains(&"widgets-architect".to_string()),
        "the Security run removed the live Architecture review's worktree"
    );
    release(&scenario, "architect");
    let architect = architect.finish();
    assert_eq!(architect.code, Some(0), "{}", architect.stderr);
    assert!(
        architect
            .stdout
            .starts_with("https://github.com/acme/widgets/issues/"),
        "{}",
        architect.stdout
    );
    release(&scenario, "7");
    let pickup = pickup.finish();
    assert_eq!(pickup.code, Some(0), "{}", pickup.stderr);
    assert_eq!(scenario.entries("work"), [REPO]);
}

#[test]
fn a_second_security_run_is_skipped_while_one_dispatches_a_fix_it_reserved_and_never_reads_it_as_failed()
 {
    let scenario = with_reproduced_findings(&["high"]);
    scenario.user_config_is(&command_limit(2));
    scenario.agent_does(&stage_fix("GHSA-finding-0"));
    scenario.agent_does_for(8, &format!("{}{}", waits_on("8"), implement_fix()));
    // Once the fix, #8, is marked ready, and before its Ticket run Claims
    // it, a Pickup run starts. The Ready search passes a Security fix over
    // in any case; the reservation keeps it from any Pickup run all the
    // same.
    let mut state = scenario.gh_state();
    state["on_issue_view"]["8"] = json!({
        "fields": "labels",
        "script": format!(
            "cd \"{}\" && {{ \"{}\" pickup > \"$FAKE_GH_STATE.next.out\" 2> \"$FAKE_GH_STATE.next.err\" || true; }}",
            scenario.launch_dir().display(),
            env!("CARGO_BIN_EXE_thirdshift"),
        ),
    });
    scenario.write_gh_state(&state);
    let fixing = scenario.run_until(&["secure", "security-fix"], &[], "started-8");
    let next = fs::read_to_string(scenario.path("gh-state.json.next.err")).unwrap();
    assert!(
        next.contains(" #8 a Security fix: its Security run owns it\n"),
        "{next}"
    );
    assert!(
        next.ends_with(" no Ready issue on acme/widgets\n"),
        "{next}"
    );
    assert_eq!(reservations(&scenario), "refs/heads/thirdshift/pickup/8");
    let description = scenario.gh_state()["advisories"][0]["description"].clone();
    assert!(
        description
            .as_str()
            .unwrap()
            .ends_with("Fix Run: pending\n"),
        "{description}"
    );
    let (github, calls) = (scenario.gh_state(), scenario.claude_calls().len());

    let skipped = scenario.run(&["secure", "security-fix"]);

    assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    assert_eq!(
        after_start(&skipped.stderr),
        "thirdshift: a Security run is already running on acme/widgets\n"
    );
    assert_eq!(skipped.stdout, "");
    assert_eq!(scenario.gh_state(), github);
    assert_eq!(scenario.claude_calls().len(), calls);
    release(&scenario, "8");
    let fixing = fixing.finish();
    assert_eq!(fixing.code, Some(0), "{}", fixing.stderr);
    assert_eq!(fixing.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(reservations(&scenario), "");
}
