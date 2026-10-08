mod support;

use std::fs;

use serde_json::{Value, json};
use support::resend::ResendStandIn;
use support::{REPO, Scenario};

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
    let logs = scenario.entries("home/.thirdshift/logs/acme/widgets/commands/secure");
    fs::read_to_string(scenario.path(&format!(
        "home/.thirdshift/logs/acme/widgets/commands/secure/{}",
        logs[0]
    )))
    .unwrap()
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
        let scenario = Scenario::new();
        scenario.agent_does_in_session(
            1,
            &audit_script(&json!([finding("first"), finding("second")]).to_string()),
        );
        scenario.agent_does_in_session(2, script);
        scenario.agent_does_in_session(3, "exit 99\n");
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result = scenario.run_with_env(
            &["secure", "harness", harness, "email", "me@example.com"],
            &resend_env(&resend),
        );
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(cause), "{}", result.stderr);
        let (_, text) = the_one_notification(&resend);
        assert!(text.contains(cause), "{text}");
        assert!(!text.contains("Private refusal evidence."), "{text}");
        let state = scenario.gh_state();
        for record in state["advisories"].as_array().unwrap() {
            assert!(record["severity"].is_null());
            assert!(
                !record["description"]
                    .as_str()
                    .unwrap()
                    .contains("## Reproduction")
            );
        }
        assert_eq!(
            scenario.claude_calls().len() + scenario.codex_calls().len(),
            2
        );
        assert_eq!(scenario.entries("work"), vec![REPO]);
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
                result.stderr.contains("security-reproduction-1.jsonl"),
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

/// A finding recorded during the audit is first seen after the waiting gate.
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
    let commands = scenario.entries("home/.thirdshift/logs/acme/widgets/commands/secure");
    assert_two_quiet_skips_send_no_email(&scenario);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(
        scenario.entries("home/.thirdshift/logs/acme/widgets/commands/secure"),
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
fn waiting_findings_skip_quietly_once_and_triage_allows_the_next_audit() {
    for triage in [
        "closed",
        "published",
        "severity",
        "private-closed",
        "private-labelled",
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is("[activity]\nquiet_skips = true\n");
        scenario.agent_does(&audit_script("[]"));
        let mut github = scenario.gh_state();
        let private = triage.starts_with("private-");
        if private {
            github["private"] = json!(true);
            github["labels"]["7"] = json!(["security-finding", "Needs-Triage"]);
        } else {
            github["advisories"] = json!([{"state":"draft", "severity":null, "summary":"Private title", "description":"Private evidence"}]);
        }
        scenario.write_gh_state(&github);
        assert_two_quiet_skips_send_no_email(&scenario);
        assert!(scenario.claude_calls().is_empty());
        assert!(
            !scenario
                .path("home/.thirdshift/logs/acme/widgets/commands")
                .exists()
        );
        assert!(
            !scenario
                .path("home/.thirdshift/logs/acme/widgets/audits")
                .exists()
        );
        let log =
            fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
                .unwrap();
        assert_eq!(log.lines().count(), 1, "{log}");
        assert!(
            log.contains("Security run skipped: a Security finding is waiting for the Day shift")
        );
        assert!(!log.contains("Private"));
        assert_eq!(scenario.gh_state(), github);
        match triage {
            "private-closed" => github["issues"]["7"] = json!("CLOSED"),
            "private-labelled" => github["labels"]["7"] = json!(["security-finding"]),
            "severity" => github["advisories"][0]["severity"] = json!("high"),
            state => github["advisories"][0]["state"] = json!(state),
        }
        scenario.write_gh_state(&github);
        let next = scenario.run(&["secure"]);
        assert_eq!(next.code, Some(0), "{triage}: {}", next.stderr);
        assert_eq!(scenario.claude_calls().len(), 1, "{triage}");
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
    }
    let scenario = Scenario::new();
    scenario.user_config_is("[security]\nharness = \"\"\n");
    scenario.agent_does(&audit_script("[]"));
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(scenario.codex_calls().is_empty());
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
fn another_pass_holds_the_security_run_and_its_skip_is_logged() {
    let scenario = Scenario::new();
    let waiting = format!(
        "touch \"$HOME/../started\"\nwhile [ ! -f \"$HOME/../release\" ]; do sleep 0.05; done\n{}",
        audit_script("[]")
    );
    scenario.agent_does(&waiting);
    let first = scenario.run_until(&["secure"], &[], "started");
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
    }
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(
        scenario
            .entries("home/.thirdshift/logs/acme/widgets/commands/secure")
            .len(),
        1
    );
    let log = fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
        .unwrap();
    assert!(log.contains("Security run skipped: another Pass is already running"));
}

#[test]
fn missing_node_fails_before_any_session_and_names_the_prerequisite() {
    let scenario = Scenario::new();
    let git = std::process::Command::new("which")
        .arg("git")
        .output()
        .unwrap();
    let git = String::from_utf8(git.stdout).unwrap();
    std::os::unix::fs::symlink(git.trim(), scenario.path("bin/git")).unwrap();
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
        assert!(result.stderr.contains("session log:"));
        assert!(scenario.gh_state()["advisories"].is_null());
        assert_eq!(scenario.entries("work"), vec![REPO]);
        let log =
            fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
                .unwrap();
        assert!(log.contains("Security run started"));
        assert!(log.contains("Security run ended: failed:"), "{log}");
        let logs = scenario.entries("home/.thirdshift/logs/acme/widgets/commands/secure");
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
    for (script, failing_call, cause) in [
        (
            audit_script("[]").replace("Security audit: complete", "Security audit: incomplete"),
            None,
            "ended incomplete",
        ),
        (
            audit_script("[{}]"),
            None,
            "validator validate-findings.cjs",
        ),
        (
            audit_script(&json!([finding("new")]).to_string()),
            Some("api --method POST"),
            "creating a draft repository security advisory failed",
        ),
        (audit_script("[]"), Some("issue list"), "gh issue list"),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&script);
        if let Some(call) = failing_call {
            if call == "api --method POST" {
                // The API consumes the request before rejecting it, avoiding
                // a race between writing stdin and an early fake gh exit.
                let mut github = scenario.gh_state();
                github["advisory_create_fails_after"] = json!(0);
                scenario.write_gh_state(&github);
            } else {
                scenario.gh_fails(call);
            }
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
        assert!(
            result.stderr.contains(cause),
            "expected {cause}: {}",
            result.stderr
        );
        assert!(!text.contains("Cause:"), "{text}");
        assert!(!text.contains("Private candidate write-up."), "{text}");
    }
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
        let scenario = Scenario::new();
        let mut github = scenario.gh_state();
        github["private"] = json!(private);
        scenario.write_gh_state(&github);
        let mut first = finding("first");
        first["title"] = json!("First recorded finding");
        let mut second = finding("second");
        second["title"] = json!("Second recorded finding");
        scenario.agent_does_in_session(1, &audit_script(&json!([first, second]).to_string()));
        scenario.agent_does_in_session(2, &reproduction_script("reproduced high single"));
        scenario.agent_does_in_session(3, &reproduction_script("incomplete"));
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result =
            scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result
                .stderr
                .contains("Security reproduction ended without the final line"),
            "{}",
            result.stderr
        );
        let (subject, text) = the_one_notification(&resend);
        assert!(subject.ends_with(": audit failed"), "{subject}");
        assert!(text.contains("high: First recorded finding"), "{text}");
        assert!(text.contains("Second recorded finding"), "{text}");
        assert!(!text.contains("high: Second recorded finding"), "{text}");
        let url = if private {
            "https://github.com/acme/widgets/issues/"
        } else {
            "https://github.com/acme/widgets/security/advisories/"
        };
        assert_eq!(text.matches(url).count(), 2, "{text}");
        for evidence in [
            "Private candidate write-up.",
            "Private trace.",
            "Private evidence.",
            "Local command: bounded-fixture",
            "bounded_fixture();",
            "Cause:",
        ] {
            assert!(
                !resend.requests()[0].body.to_string().contains(evidence),
                "leaked {evidence}: {text}"
            );
        }
        assert_eq!(scenario.claude_calls().len(), 3);
    }
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
