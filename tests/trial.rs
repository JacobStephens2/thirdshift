//! Trials use the Run's session machinery without creating a Command.

mod support;

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use support::{Scenario, leaves_running};

fn request(scenario: &Scenario) -> PathBuf {
    fs::write(
        scenario.launch_dir().join(".thirdshift-issue.md"),
        "# Fixture Spec\n\nReview the fixture change.\n",
    )
    .unwrap();
    let path = scenario.path("request.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "version": 1,
            "repository": scenario.launch_dir(),
            "session_kind": "spec-review",
            "inputs": {
                "issue_url": scenario.issue_url(7),
                "base": "main",
                "branch": "issue-7",
                "pr_url": "https://github.com/acme/widgets/pull/1"
            },
            "harness": "claude",
            "model": "opus",
            "effort": "high",
            "output_dir": scenario.path("trial-output")
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn write_reports() -> &'static str {
    r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
test -n "$reports"
printf 'Standards fixture report\n' > "$reports/standards.md"
printf 'Spec fixture report\n' > "$reports/spec.md"
"#
}

#[test]
fn a_spec_review_trial_prints_one_complete_line_and_keeps_artifacts_outside_factory_logs() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    scenario.agent_does(write_reports());

    let result = scenario.run(&["--trial", request.to_str().unwrap()]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.lines().count(), 1, "{}", result.stdout);
    let line: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(line["version"], 1);
    assert_eq!(line["outcome"], "succeeded");
    assert_eq!(line["void_cause"], Value::Null);
    assert_eq!(line["cause"], Value::Null);
    assert_eq!(line["session_id"], "fake-session-1");
    assert!(line["timings"]["total_ms"].is_u64());
    assert!(line["timings"]["session_ms"].is_u64());
    assert!(line["timings"]["total_ms"].as_u64() >= line["timings"]["session_ms"].as_u64());
    for field in ["log", "model_evidence"] {
        let path = PathBuf::from(line[field].as_str().unwrap());
        assert!(path.starts_with(scenario.path("trial-output")));
        assert!(path.is_file(), "{}", path.display());
    }
    for (axis, expected) in [
        ("standards", "Standards fixture report\n"),
        ("spec", "Spec fixture report\n"),
    ] {
        let path = PathBuf::from(line["reports"][axis].as_str().unwrap());
        assert!(path.starts_with(scenario.path("trial-output")));
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
    }
    let models: Value =
        serde_json::from_slice(&fs::read(line["model_evidence"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(models["command_log"], Value::Null);
    assert_eq!(models["requested"], "opus");
    assert_eq!(models["effort"], "high");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["cwd"], scenario.launch_dir().to_str().unwrap());
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(prompt.contains("Every Ticket of the Spec"));
    assert!(prompt.contains("using main as the fixed point"));
    assert!(prompt.contains(".thirdshift-issue.md"));
    scenario.assert_every_session_found_the_factory_skills();
    assert!(!scenario.path("home/.thirdshift/logs").exists());
    assert!(scenario.gh_calls().is_empty());
}

#[test]
fn an_unknown_request_version_is_refused_before_decoding_its_fields_or_starting_work() {
    let scenario = Scenario::new();
    let request = scenario.path("request.json");
    fs::write(&request, r#"{"version":99,"session_kind":"future-kind"}"#).unwrap();

    let result = scenario.run(&["--trial", request.to_str().unwrap()]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stdout.is_empty());
    assert!(
        result
            .stderr
            .contains("unsupported Trial request version 99; supported version: 1"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert!(!scenario.path("trial-output").exists());
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn trial_output_cannot_use_factory_logs_or_overwrite_an_earlier_trial() {
    for case in ["default", "configured", "symlink", "occupied"] {
        let scenario = Scenario::new();
        let request = request(&scenario);
        let output = match case {
            "default" => scenario.path("home/.thirdshift/logs/trial"),
            "configured" => {
                scenario.user_config_is(&format!(
                    "[logs]\ndir = {:?}\n",
                    scenario.path("factory-logs")
                ));
                scenario.path("factory-logs/trial")
            }
            "symlink" => {
                fs::create_dir_all(scenario.path("home/.thirdshift/logs")).unwrap();
                std::os::unix::fs::symlink(
                    scenario.path("home/.thirdshift/logs"),
                    scenario.path("linked-logs"),
                )
                .unwrap();
                scenario.path("linked-logs/trial")
            }
            "occupied" => {
                fs::create_dir(scenario.path("trial-output")).unwrap();
                fs::write(scenario.path("trial-output/previous-result"), "keep me").unwrap();
                scenario.path("trial-output")
            }
            _ => unreachable!(),
        };
        let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
        document["output_dir"] = json!(output);
        fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();

        let result = scenario.run(&["--trial", request.to_str().unwrap()]);

        assert_eq!(result.code, Some(1), "{case}: {}", result.stderr);
        assert!(result.stdout.is_empty(), "{case}: {}", result.stdout);
        assert!(scenario.claude_calls().is_empty(), "{case}");
        if case == "occupied" {
            assert!(result.stderr.contains("must be empty"), "{}", result.stderr);
            assert_eq!(
                fs::read_to_string(output.join("previous-result")).unwrap(),
                "keep me"
            );
        } else {
            assert!(
                result.stderr.contains("outside factory logs"),
                "{}",
                result.stderr
            );
            assert!(!output.exists(), "created {}", output.display());
        }
    }
}

#[test]
fn a_failed_trial_keeps_its_session_id_diagnostic_and_model_evidence() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    scenario.agent_does(
        r#"echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["review failed"]}' >> "$FAKE_CLAUDE_AFTER_RESULT""#,
    );

    let result = scenario.run(&["--trial", request.to_str().unwrap()]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.lines().count(), 1);
    let line: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(line["outcome"], "failed");
    assert_eq!(line["void_cause"], Value::Null);
    assert_eq!(line["session_id"], "fake-session-1");
    assert!(line["cause"].as_str().unwrap().contains("review failed"));
    assert!(PathBuf::from(line["log"].as_str().unwrap()).is_file());
    assert!(PathBuf::from(line["model_evidence"].as_str().unwrap()).is_file());
    assert_eq!(line["reports"], json!({"standards": null, "spec": null}));
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn unavailable_sessions_and_provider_limits_are_void_trials() {
    for (script, expected, diagnostic) in [
        ("exit 3".to_string(), "session_unavailable", "claude exited 3"),
        (
            r#"echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["quota exceeded"]}' >> "$FAKE_CLAUDE_AFTER_RESULT""#.to_string(),
            "usage_limit",
            "quota exceeded",
        ),
        (
            r#"echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["rate limit reached"]}' >> "$FAKE_CLAUDE_AFTER_RESULT""#.to_string(),
            "rate_limit",
            "rate limit reached",
        ),
    ] {
        let scenario = Scenario::new();
        let request = request(&scenario);
        scenario.agent_does(&script);

        let result = scenario.run(&["--trial", request.to_str().unwrap()]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(result.stdout.lines().count(), 1);
        let line: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(line["outcome"], "void", "{expected}: {line}");
        assert_eq!(line["void_cause"], expected);
        assert_eq!(line["session_id"], "fake-session-1");
        assert!(line["cause"].as_str().unwrap().contains(diagnostic), "{line}");
        assert_eq!(scenario.claude_calls().len(), 1);
        assert!(!scenario.path("home/.thirdshift/logs").exists());
    }
}

#[test]
fn a_trial_clears_inherited_ownership_and_resumes_killed_work_only_once_with_its_reports() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    scenario.agent_does(&format!(
        "test -z \"${{THIRDSHIFT_LOG_OWNER+x}}\"\n{}{}",
        write_reports(),
        leaves_running("cargo test"),
    ));

    let result = scenario.run_with_env(
        &["--trial", request.to_str().unwrap()],
        &[("THIRDSHIFT_LOG_OWNER", "stale parent ownership")],
    );

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let line: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(line["outcome"], "succeeded", "{line}");
    assert_eq!(line["session_id"], "fake-session-2");
    assert!(
        line["log"]
            .as_str()
            .unwrap()
            .ends_with("-spec-review-resume.jsonl")
    );
    assert!(result.stderr.contains("resuming it once"));
    assert!(
        result
            .stderr
            .contains("carrying on, as it may have been abandoned")
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    let argv = calls[1]["argv"].as_array().unwrap();
    assert!(
        argv.windows(2)
            .any(|pair| pair == [json!("--resume"), json!("fake-session-1")])
    );
    for call in &calls {
        let argv = call["argv"].as_array().unwrap();
        for pair in [["--model", "opus"], ["--effort", "high"]] {
            assert!(
                argv.windows(2)
                    .any(|args| args == [json!(pair[0]), json!(pair[1])])
            );
        }
    }
    for axis in ["standards", "spec"] {
        assert!(PathBuf::from(line["reports"][axis].as_str().unwrap()).is_file());
        assert!(
            scenario
                .path(&format!("trial-output/trial-spec-review.{axis}"))
                .is_file()
        );
    }
    assert!(
        calls[1]["prompt"]
            .as_str()
            .unwrap()
            .contains("in the foreground")
    );
    scenario.assert_every_session_found_the_factory_skills();
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn early_refusals_and_context_limits_fail_while_login_and_provider_failures_are_void() {
    for (diagnostic, expected) in [
        ("context window exceeded", "failed"),
        ("request refused", "failed"),
        ("not logged in", "void"),
        ("provider server error", "void"),
    ] {
        let scenario = Scenario::new();
        let request = request(&scenario);
        scenario.agent_does(&format!(
            "cat >> \"$FAKE_CLAUDE_AFTER_RESULT\" <<'TRIAL_ERROR'\n{}\nTRIAL_ERROR\nexit 3\n",
            json!({"type":"result", "subtype":"error_during_execution", "is_error":true, "errors":[diagnostic]}),
        ));

        let result = scenario.run(&["--trial", request.to_str().unwrap()]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let line: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(line["outcome"], expected, "{diagnostic}: {line}");
        assert_eq!(
            line["void_cause"],
            if expected == "void" {
                json!("session_unavailable")
            } else {
                Value::Null
            },
        );
        assert!(
            line["cause"].as_str().unwrap().contains(diagnostic),
            "{line}"
        );
    }
}

#[test]
fn a_codex_trial_uses_its_request_defaults_and_keeps_the_prepared_issue_out_of_git() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    scenario.user_config_is(
        "[harness]\ndefault = \"claude\"\n[harness.codex]\nmodel = \"wrong-default\"\neffort = \"low\"\n",
    );
    let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    document["harness"] = json!("codex");
    document["model"] = Value::Null;
    document["effort"] = Value::Null;
    fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
    scenario.agent_does(&format!(
        "git check-ignore -q .thirdshift-issue.md\n{}",
        write_reports()
    ));

    let result = scenario.run(&["--trial", request.to_str().unwrap()]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let line: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(line["outcome"], "succeeded", "{line}");
    assert_eq!(line["session_id"], "fake-thread-1");
    let models: Value =
        serde_json::from_slice(&fs::read(line["model_evidence"].as_str().unwrap()).unwrap())
            .unwrap();
    assert_eq!(models["harness"], "codex");
    assert_eq!(models["requested"], Value::Null);
    assert_eq!(models["effort"], Value::Null);
    assert_eq!(scenario.codex_calls().len(), 1);
    assert!(scenario.claude_calls().is_empty());
    scenario.assert_every_codex_session_found_the_factory_skills();
    assert!(PathBuf::from(line["reports"]["spec"].as_str().unwrap()).is_file());
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn an_interrupted_trial_emits_a_void_result_after_stopping_its_harness() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    scenario.agent_does(&format!(
        "touch '{}'\nexec sleep 60\n",
        scenario.path("started").display()
    ));

    let result = scenario.run_and_signal(&["--trial", request.to_str().unwrap()], "started", "INT");

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.lines().count(), 1);
    let line: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(line["outcome"], "void");
    assert_eq!(line["void_cause"], "interrupted");
    assert!(line["cause"].as_str().unwrap().contains("interrupted"));
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn factory_skill_temporary_files_are_removed_after_success_or_failure() {
    for script in ["true", "exit 3"] {
        let scenario = Scenario::new();
        let request = request(&scenario);
        scenario.agent_does(script);

        let result = scenario.run(&["--trial", request.to_str().unwrap()]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            scenario.entries("tmp").is_empty(),
            "{}: {:?}",
            script,
            scenario.entries("tmp")
        );
    }
}
