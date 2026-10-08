mod support;

use std::fs;

use serde_json::{Value, json};
use support::{REPO, Scenario};

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
            audit_script("[]")
        ),
    );
    scenario
        .agent_does("printf '%s\\n' 'Security audit: complete' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n");
    let result = scenario.run(&["secure", "harness", "codex"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.codex_calls();
    assert_eq!(calls.len(), 2);
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
    let resumed = calls[1]["argv"].as_array().unwrap();
    assert!(
        resumed
            .windows(2)
            .any(|pair| pair == [json!("resume"), json!("fake-thread-1")])
    );
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
    let second = scenario.run(&["secure"]);
    let pickup = scenario.run(&["pickup"]);
    fs::write(scenario.path("release"), "").unwrap();
    let first = first.finish();
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
