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

#[test]
fn the_notification_lists_each_recorded_finding_without_its_private_write_up() {
    let scenario = Scenario::new();
    let mut github = scenario.gh_state();
    github["advisories"] = json!([{
        "state": "draft", "severity": "high", "summary": "Existing finding",
        "html_url": "https://github.com/acme/widgets/security/advisories/GHSA-existing",
        "description": "Fingerprint: `existing`\nPrivate existing write-up."
    }]);
    scenario.write_gh_state(&github);
    scenario.agent_does(&audit_script(
        &json!([finding("existing"), finding("new")]).to_string(),
    ));
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
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);
        let result =
            scenario.run_with_env(&["secure", "email", "me@example.com"], &resend_env(&resend));
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let (_, text) = the_one_notification(&resend);
        for metadata in [
            "Security audit recorded 1 new finding(s), 1 already recorded",
            "Existing finding",
            "https://github.com/acme/widgets/issues/8",
            "Unchecked input size",
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
        assert_eq!(state["bodies"][number], expected);
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
output=$(sed -n 's/^Output directory: `\(.*\)`\.$/\1/p' prompt.txt)
test -n "$output"
printf '%s\n' '{findings}' > "$output/findings.json"
printf '%s\n' '[]' > "$output/coverage-ledger.json"
printf '%s\n' '{{"run_status":"complete"}}' > "$output/run-metadata.json"
printf '%s\n' 'Security audit: complete' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
    )
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
        }
        scenario.write_gh_state(&github);
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
        assert!(api_calls.iter().all(|call| call[2] == "POST"));
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
