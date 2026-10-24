//! Trials use the Run's session machinery without creating a Command.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{RunResult, Scenario, leaves_running, scope_available};

/// Where a sandboxed Trial's fake Harness scripts and records live: inside the
/// prepared repository, the one path the sandbox keeps.
const TRIAL_FAKES: &str = ".trial-fakes";

/// Whether the Trial sandbox can run here.
fn bwrap_present() -> bool {
    std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Run `--trial` the way production does: inside the Trial sandbox, where it
/// runs. The fake Harness's scripts and records move into the prepared
/// repository, the one path the sandbox keeps, so the session stays
/// observable. Where bwrap cannot run, the Trial runs without its sandbox, as
/// it does with `THIRDSHIFT_TRIAL_NO_SANDBOX`, so the test still exercises
/// the Trial itself.
fn run_trial(scenario: &Scenario, request: &Path, extra: &[(&str, &str)]) -> RunResult {
    let args = ["--trial", request.to_str().unwrap()];
    if !bwrap_present() {
        let mut env = vec![("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")];
        env.extend_from_slice(extra);
        return scenario.run_with_env(&args, &env);
    }
    let dir = scenario.launch_dir().join(TRIAL_FAKES);
    fs::create_dir_all(&dir).unwrap();
    for entry in fs::read_dir(scenario.path("")).unwrap() {
        let entry = entry.unwrap();
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("claude-script.sh"))
        {
            fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
        }
    }
    let names = [
        "FAKE_CLAUDE_SCRIPT",
        "FAKE_CLAUDE_RECORD",
        "FAKE_CODEX_RECORD",
        "FAKE_AGY_RECORD",
        "FAKE_AGY_CHECK_RECORD",
        "FAKE_GROK_RECORD",
        "FAKE_MUSE_RECORD",
        "FAKE_OPENCODE_RECORD",
    ];
    let files = [
        "claude-script.sh",
        "claude-calls.json",
        "codex-calls.json",
        "agy-calls.json",
        "agy-checks.json",
        "grok-calls.json",
        "muse-calls.json",
        "opencode-calls.json",
    ];
    let paths = files
        .into_iter()
        .map(|name| dir.join(name).to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let mut env: Vec<(&str, &str)> = names
        .into_iter()
        .zip(paths.iter().map(String::as_str))
        .collect();
    env.extend_from_slice(extra);
    scenario.run_with_env(&args, &env)
}

/// The fake Harness calls a sandboxed Trial recorded, beside the scripts it
/// ran. Without bwrap the Trial runs unsandboxed, recording at fixed paths.
fn trial_calls(scenario: &Scenario, file: &str) -> Vec<Value> {
    let redirected = scenario.launch_dir().join(TRIAL_FAKES).join(file);
    if redirected.exists() {
        return serde_json::from_str(&fs::read_to_string(redirected).unwrap()).unwrap();
    }
    serde_json::from_str(&fs::read_to_string(scenario.path(file)).unwrap_or_default())
        .unwrap_or_default()
}

/// Assert every sandboxed call found every Factory skill in its worktree.
fn assert_trial_sessions_found_the_factory_skills(calls: Vec<Value>) {
    assert!(!calls.is_empty(), "no session ran");
    for call in calls {
        for skill in support::FACTORY_SKILLS {
            let skill_md = call["skill_files"][format!("{skill}/SKILL.md")].as_str();
            assert!(
                skill_md.is_some_and(|text| text.contains(&format!("name: {skill}\n"))),
                "the session in {} with the prompt {} found no {skill}",
                call["cwd"],
                call["prompt"]
            );
        }
    }
}

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
fn a_code_review_trial_reviews_the_prepared_diff_and_keeps_both_reports() {
    let scenario = Scenario::new();
    scenario.launch_git(&["checkout", "-b", "issue-7"]);
    scenario.commit_locally("counter.txt", "off by one\n", "Reviewed change");
    let request = request(&scenario);
    let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    document["session_kind"] = json!("code-review");
    document["harness"] = json!("codex");
    fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
    scenario.agent_does(write_reports());

    let result = run_trial(&scenario, &request, &[]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "succeeded");
    assert!(
        output["log"]
            .as_str()
            .unwrap()
            .ends_with("trial-code-review.jsonl")
    );
    for (axis, expected) in [
        ("standards", "Standards fixture report\n"),
        ("spec", "Spec fixture report\n"),
    ] {
        let path = PathBuf::from(output["reports"][axis].as_str().unwrap());
        assert!(path.starts_with(scenario.path("trial-output")));
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
    }
    let calls = trial_calls(&scenario, "codex-calls.json");
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with("$thirdshift-code-review main"),
        "{prompt}"
    );
    assert!(prompt.contains("using main as the fixed point"));
    assert!(prompt.contains(".thirdshift-issue.md"));
    assert!(prompt.contains("review-only"));
    assert!(prompt.contains("Do not fix"));
    assert!(prompt.contains("do not commit, push"));
    assert!(!prompt.contains("Address the Standards and Spec findings"));
    assert!(!prompt.contains("Update PR"));
    assert!(prompt.contains("fork_turns: \"none\""), "{prompt}");
    assert!(prompt.contains("only its own task and necessary evidence"));
    let argv = calls[0]["argv"].to_string();
    assert!(!argv.contains("agents.max_threads"), "{argv}");
    assert_trial_sessions_found_the_factory_skills(trial_calls(&scenario, "codex-calls.json"));
    assert!(scenario.gh_calls().is_empty());
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn a_spec_review_trial_keeps_reports_without_a_pr_service_or_factory_logs() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    // The sandbox cannot reach GitHub or the scenario's fake gh. Keep the
    // same unavailable-PR behavior when run_trial falls back without bwrap.
    scenario.gh_fails("pr view");
    scenario.agent_does(&format!(
        r#"
if gh pr view https://github.com/acme/widgets/pull/1 --json state >/dev/null 2>&1; then
    echo 'unexpected PR service' >&2
    exit 1
fi
{}
"#,
        write_reports()
    ));

    let result = run_trial(&scenario, &request, &[]);

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
    let calls = trial_calls(&scenario, "claude-calls.json");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["cwd"], scenario.launch_dir().to_str().unwrap());
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(prompt.contains("Every Ticket of the Spec"));
    assert!(prompt.contains("using main as the fixed point"));
    assert!(prompt.contains(".thirdshift-issue.md"));
    assert!(prompt.contains("Update PR https://github.com/acme/widgets/pull/1"));
    assert_trial_sessions_found_the_factory_skills(trial_calls(&scenario, "claude-calls.json"));
    assert!(!scenario.path("home/.thirdshift/logs").exists());
    assert!(
        scenario
            .gh_calls()
            .iter()
            .all(|call| call.starts_with(&["pr".to_owned(), "view".to_owned()]))
    );
}

#[test]
fn an_unknown_request_version_is_refused_before_decoding_its_fields_or_starting_work() {
    let scenario = Scenario::new();
    let request = scenario.path("request.json");
    fs::write(&request, r#"{"version":99,"session_kind":"future-kind"}"#).unwrap();

    let result = run_trial(&scenario, &request, &[]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stdout.is_empty());
    assert!(
        result
            .stderr
            .contains("unsupported Trial request version 99; supported version: 1"),
        "{}",
        result.stderr
    );
    assert!(trial_calls(&scenario, "claude-calls.json").is_empty());
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

        let result = run_trial(&scenario, &request, &[]);

        assert_eq!(result.code, Some(1), "{case}: {}", result.stderr);
        assert!(result.stdout.is_empty(), "{case}: {}", result.stdout);
        assert!(
            trial_calls(&scenario, "claude-calls.json").is_empty(),
            "{case}"
        );
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

    let result = run_trial(&scenario, &request, &[]);

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
fn a_live_plan_limit_cuts_short_a_trial_and_keeps_its_evidence() {
    for (harness, event, session_id) in [
        (
            "claude",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"seven_day"}}"#,
            "fake-session-1",
        ),
        (
            "claude",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"five_hour"}}"#,
            "fake-session-1",
        ),
        (
            "codex",
            r#"{"type":"token_count","rate_limits":{"primary":{"used_percent":85}}}"#,
            "fake-thread-1",
        ),
    ] {
        let scenario = Scenario::new();
        let request = request(&scenario);
        let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
        document["harness"] = json!(harness);
        fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
        let escaped = scenario.launch_dir().join("continued-past-warning");
        scenario.agent_does(&format!(
            "echo '{event}'\nsleep 0.5\ntouch '{}'\n",
            escaped.display()
        ));

        let result = run_trial(&scenario, &request, &[]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let output: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["outcome"], "void", "{harness}: {output}");
        assert_eq!(output["void_cause"], "usage_limit", "{output}");
        assert_eq!(output["session_id"], session_id);
        assert!(output["cause"].as_str().unwrap().contains("plan-limit"));
        assert!(PathBuf::from(output["model_evidence"].as_str().unwrap()).is_file());
        assert!(
            fs::read_to_string(output["log"].as_str().unwrap())
                .unwrap()
                .contains(event)
        );
        assert!(
            !escaped.exists(),
            "{harness} kept working after its warning"
        );
    }
}

#[test]
fn sandboxed_codex_native_limits_use_the_fresh_home_without_an_override() {
    if !bwrap_present() {
        println!("SKIPPED (fresh-home Codex quota): bwrap unavailable");
        return;
    }
    let scenario = Scenario::new();
    let request = request(&scenario);
    let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    document["harness"] = json!("codex");
    fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
    let escaped = scenario.launch_dir().join("continued-after-native-warning");
    scenario.agent_does(&format!(r#"
mkdir -p "$HOME/.codex/sessions/2026/10/23"
native="$HOME/.codex/sessions/2026/10/23/rollout-fixture-fake-thread-1.jsonl"
printf '{{"type":"session_meta","payload":{{"id":"fake-thread-1","cwd":"%s"}}}}\n' "$PWD" > "$native"
sleep 1.1
timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
printf '{{"timestamp":"%s","type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"secondary":{{"used_percent":85}}}}}}}}\n' "$timestamp" >> "$native"
sleep 0.5
touch '{}'
"#, escaped.display()));
    let result = run_trial(&scenario, &request, &[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(
        output["outcome"], "void",
        "native quota warning was missed: {output}"
    );
    assert_eq!(output["void_cause"], "usage_limit");
    assert!(
        !escaped.exists(),
        "Codex continued after its native warning"
    );
}

#[test]
fn codex_quota_reports_are_read_live_from_the_current_sessions_native_file() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    document["harness"] = json!("codex");
    fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
    let escaped = scenario.launch_dir().join("continued-after-native-warning");
    scenario.agent_does(&format!(r#"
mkdir -p "$HOME/.codex/sessions/2026/10/23"
native="$HOME/.codex/sessions/2026/10/23/rollout-fixture-fake-thread-1.jsonl"
printf '{{"type":"session_meta","payload":{{"id":"fake-thread-1","cwd":"%s"}}}}\n' "$PWD" > "$native"
printf '{{"timestamp":"2000-01-01T00:00:00Z","type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"primary":{{"used_percent":99}}}}}}}}\n' >> "$native"
sleep 1.1
touch '{}'
timestamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)
printf '{{"timestamp":"%s","type":"event_msg","payload":{{"type":"token_count","rate_limits":{{"secondary":{{"used_percent":85}}}}}}}}\n' "$timestamp" >> "$native"
sleep 0.5
touch '{}'
"#, scenario.launch_dir().join("ignored-old-native-warning").display(), escaped.display()));

    // The native session file lives beside the login: sandboxed, point
    // CODEX_HOME at the Trial's fresh home, which the sandbox keeps on both
    // sides of its bind. Without bwrap the Trial runs unsandboxed and the
    // fresh home is never made, so no override: the agent and its reader
    // share the scenario home.
    let codex_home = scenario
        .path("trial-output/home/.codex")
        .to_str()
        .unwrap()
        .to_owned();
    let mut extra = Vec::new();
    if bwrap_present() {
        extra.push(("CODEX_HOME", codex_home.as_str()));
    }
    let result = run_trial(&scenario, &request, &extra);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "void", "{output}");
    assert_eq!(output["void_cause"], "usage_limit");
    assert_eq!(output["session_id"], "fake-thread-1");
    assert!(
        scenario
            .launch_dir()
            .join("ignored-old-native-warning")
            .exists(),
        "an old report stopped the new session"
    );
    assert!(
        !escaped.exists(),
        "the native warning was not read while stdout was idle"
    );
    assert!(
        fs::read_to_string(output["log"].as_str().unwrap())
            .unwrap()
            .contains("used_percent")
    );
}

#[test]
fn unavailable_sessions_and_provider_limits_are_void_trials() {
    for (script, expected, diagnostic) in [
        (
            r#"echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["connection refused"]}' >> "$FAKE_CLAUDE_AFTER_RESULT""#.to_string(),
            "session_unavailable",
            "connection refused",
        ),
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

        let result = run_trial(&scenario, &request, &[]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(result.stdout.lines().count(), 1);
        let line: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(line["outcome"], "void", "{expected}: {line}");
        assert_eq!(line["void_cause"], expected);
        assert_eq!(line["session_id"], "fake-session-1");
        assert!(line["cause"].as_str().unwrap().contains(diagnostic), "{line}");
        assert_eq!(trial_calls(&scenario, "claude-calls.json").len(), 1);
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

    let result = run_trial(
        &scenario,
        &request,
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
    let calls = trial_calls(&scenario, "claude-calls.json");
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
    assert_trial_sessions_found_the_factory_skills(trial_calls(&scenario, "claude-calls.json"));
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

#[test]
fn only_external_failures_are_void_and_provider_server_errors_require_no_work() {
    for (diagnostic, turns, expected, cause) in [
        ("", 0, "failed", Value::Null),
        ("context window exceeded", 0, "failed", Value::Null),
        ("request refused", 0, "failed", Value::Null),
        ("JavaScript heap out of memory", 0, "failed", Value::Null),
        ("not logged in", 0, "void", json!("session_unavailable")),
        ("login expired", 1, "void", json!("session_unavailable")),
        // A Harness that ignores the sandbox's proxy finds no route: DNS
        // fails before any connection, and its Trial is void.
        (
            "getaddrinfo EAI_AGAIN api.anthropic.com",
            0,
            "void",
            json!("session_unavailable"),
        ),
        (
            "Temporary failure in name resolution",
            0,
            "void",
            json!("session_unavailable"),
        ),
        (
            "Could not resolve host: api.anthropic.com",
            0,
            "void",
            json!("session_unavailable"),
        ),
        (
            "provider server error",
            0,
            "void",
            json!("session_unavailable"),
        ),
        ("provider server error", 1, "failed", Value::Null),
        (
            "sandbox failed to start",
            0,
            "void",
            json!("sandbox_unavailable"),
        ),
        ("sandbox failed to start", 1, "failed", Value::Null),
    ] {
        let scenario = Scenario::new();
        let request = request(&scenario);
        scenario.agent_does(&format!(
            "cat >> \"$FAKE_CLAUDE_AFTER_RESULT\" <<'TRIAL_ERROR'\n{}\nTRIAL_ERROR\nexit 3\n",
            json!({"type":"result", "subtype":"error_during_execution", "is_error":true, "errors":[diagnostic], "num_turns":turns}),
        ));

        let result = run_trial(&scenario, &request, &[]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let line: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(line["outcome"], expected, "{diagnostic}: {line}");
        assert_eq!(line["void_cause"], cause, "{diagnostic}: {line}");
        assert!(
            line["cause"].as_str().unwrap().contains(diagnostic),
            "{line}"
        );
    }
}

#[test]
fn a_server_error_on_resume_after_trial_work_is_failed() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    scenario.agent_does_in_session(1, &format!(
        "{}\n{}\n{}",
        write_reports(),
        r#"echo '{"type":"result","subtype":"success","is_error":false,"num_turns":1}' >> "$FAKE_CLAUDE_AFTER_RESULT""#,
        leaves_running("cargo test"),
    ));
    scenario.agent_does_in_session(2, r#"
echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["provider server error"],"num_turns":0}' >> "$FAKE_CLAUDE_AFTER_RESULT"
exit 3
"#);
    let run = run_trial(&scenario, &request, &[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        trial_calls(&scenario, "claude-calls.json").len(),
        2,
        "{}",
        run.stderr
    );
    let result: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(
        result["outcome"], "failed",
        "work before the Resume must count: {result}"
    );
    assert_eq!(result["void_cause"], Value::Null);
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

    let result = run_trial(&scenario, &request, &[]);

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
    assert_eq!(trial_calls(&scenario, "codex-calls.json").len(), 1);
    assert!(trial_calls(&scenario, "claude-calls.json").is_empty());
    assert_trial_sessions_found_the_factory_skills(trial_calls(&scenario, "codex-calls.json"));
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

    // The signal watch reads the scenario root, which the sandbox hides: this
    // interruption runs without the sandbox, as the Experiment suite does.
    let result = scenario.run_and_signal_with_env(
        &["--trial", request.to_str().unwrap()],
        &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")],
        "started",
        "INT",
    );

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

        let result = run_trial(&scenario, &request, &[]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            scenario.entries("tmp").is_empty(),
            "{}: {:?}",
            script,
            scenario.entries("tmp")
        );
    }
}

#[test]
fn an_interrupted_trial_keeps_an_already_observed_session_id() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    let log = scenario.path("trial-output/trial-spec-review.jsonl");
    scenario.agent_does(&format!(
        "until grep -q 'fake-session-1' '{}'; do sleep 0.01; done\n\
         touch '{}'\n\
         exec sleep 60\n",
        log.display(),
        scenario.path("session-id-observed").display()
    ));

    // As above: the session-id watch lives outside the sandbox.
    let result = scenario.run_and_signal_with_env(
        &["--trial", request.to_str().unwrap()],
        &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")],
        "session-id-observed",
        "INT",
    );

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let line: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(line["outcome"], "void");
    assert_eq!(line["void_cause"], "interrupted");
    assert!(fs::read_to_string(&log).unwrap().contains("fake-session-1"));
    assert_eq!(line["session_id"], "fake-session-1", "{line}");
}

#[test]
fn a_sandboxed_trial_hides_unbound_paths_and_keeps_its_working_directory() {
    if !bwrap_present() {
        println!("SKIPPED (sandboxed Trial): bwrap is not on PATH");
        return;
    }
    let scenario = Scenario::new();
    scenario.launch_git(&["checkout", "-b", "issue-7"]);
    scenario.commit_locally("counter.txt", "off by one\n", "Reviewed change");
    let request = request(&scenario);
    let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    document["session_kind"] = json!("code-review");
    document["harness"] = json!("codex");
    fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
    let beyond = scenario.path("beyond-the-sandbox");
    fs::write(&beyond, "host only\n").unwrap();
    scenario.agent_does(&format!(
        "test ! -e '{}' || exit 41\ntest \"$(pwd)\" = '{}'\n{}",
        beyond.display(),
        scenario.launch_dir().display(),
        write_reports(),
    ));

    let result = run_trial(&scenario, &request, &[]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "succeeded", "{output}");
    let calls = trial_calls(&scenario, "codex-calls.json");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["cwd"], scenario.launch_dir().to_str().unwrap());
    assert_trial_sessions_found_the_factory_skills(calls);
    assert!(!scenario.path("home/.thirdshift/logs").exists());
}

/// Supply the relay dependency so these fixtures reach the bwrap failure
/// even on hosts without socat. The failing launcher never runs the relay.
fn failing_bwrap(scenario: &Scenario, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    for (name, script) in [("bwrap", script), ("socat", "#!/bin/sh\nexit 99\n")] {
        let path = scenario.path(&format!("bin/{name}"));
        fs::write(&path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn a_bwrap_startup_failure_makes_a_trial_void() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    failing_bwrap(
        &scenario,
        "#!/bin/sh\necho 'bwrap: Creating new namespace failed: Operation not permitted' >&2\nexit 1\n",
    );
    let result = scenario.run(&["--trial", request.to_str().unwrap()]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "void", "{output}");
    assert_eq!(output["void_cause"], "sandbox_unavailable", "{output}");
    assert!(
        output["cause"]
            .as_str()
            .unwrap()
            .contains("Creating new namespace failed"),
        "{output}"
    );
}

#[test]
fn a_bwrap_failure_after_its_startup_check_still_makes_a_trial_void() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    failing_bwrap(
        &scenario,
        "#!/bin/sh\ncase \"$*\" in *'/bin/true') exit 0;; esac\necho 'bwrap: Creating new namespace failed after probe' >&2\nexit 1\n",
    );
    let result = scenario.run(&["--trial", request.to_str().unwrap()]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "void", "{output}");
    assert_eq!(output["void_cause"], "sandbox_unavailable", "{output}");
    assert!(
        output["cause"].as_str().unwrap().contains("after probe"),
        "{output}"
    );
}

#[test]
fn an_unlaunchable_passthrough_harness_is_a_void_trial() {
    use std::os::unix::fs::PermissionsExt;
    let scenario = Scenario::new();
    let request = request(&scenario);
    let claude = scenario.path("bin/claude");
    fs::remove_file(&claude).unwrap();
    fs::write(&claude, b"not an executable format\n").unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    let result = scenario.run_with_env(
        &["--trial", request.to_str().unwrap()],
        &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "void", "{output}");
    assert_eq!(output["void_cause"], "session_unavailable", "{output}");
}

#[test]
fn an_unstartable_passthrough_harness_makes_a_trial_void() {
    use std::os::unix::fs::PermissionsExt;
    let scenario = Scenario::new();
    let request = request(&scenario);
    let binary = scenario.path("bin/claude");
    fs::remove_file(&binary).unwrap();
    fs::write(&binary, "#!/thirdshift-missing-interpreter\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let git = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(git.status.success());
    let git = String::from_utf8(git.stdout).unwrap();
    std::os::unix::fs::symlink(git.trim(), scenario.path("bin/git")).unwrap();
    let bin = scenario.path("bin");
    let result = scenario.run_with_env(
        &["--trial", request.to_str().unwrap()],
        &[
            ("THIRDSHIFT_TRIAL_NO_SANDBOX", "1"),
            ("PATH", bin.to_str().unwrap()),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "void", "{output}");
    assert_eq!(output["void_cause"], "session_unavailable", "{output}");
}

#[test]
fn a_trial_without_its_sandbox_runs_its_session_directly() {
    let scenario = Scenario::new();
    let request = request(&scenario);
    scenario.agent_does(write_reports());

    let result = run_trial(&scenario, &request, &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "succeeded", "{output}");
    assert_eq!(output["session_id"], "fake-session-1");
}

/// Whether a process can read its own `oom_score_adj` here (Linux).
fn oom_score_readable() -> bool {
    std::fs::read_to_string("/proc/self/oom_score_adj").is_ok()
}

#[test]
fn trials_raise_their_own_oom_score_adj() {
    if !oom_score_readable() {
        println!("SKIPPED (trials raise their own oom_score_adj): no /proc/self/oom_score_adj");
        return;
    }
    let scenario = Scenario::new();
    let request = request(&scenario);
    scenario.agent_does(&format!(
        "{}\ncat /proc/self/oom_score_adj >> \"$FAKE_CLAUDE_RECORD.adj\"\n",
        write_reports()
    ));

    let result = run_trial(&scenario, &request, &[]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["outcome"], "succeeded", "{output}");
    // The fake Harness inherits the Trial's raised score through the sandbox.
    let redirected = scenario
        .launch_dir()
        .join(TRIAL_FAKES)
        .join("claude-calls.json.adj");
    let path = if redirected.exists() {
        redirected
    } else {
        scenario.path("claude-calls.json.adj")
    };
    assert_eq!(
        fs::read_to_string(&path).unwrap().trim(),
        "1000",
        "the Trial's session kept its raised oom_score_adj"
    );
}

#[test]
fn oom_exec_raises_its_score_and_becomes_its_program() {
    if !oom_score_readable() {
        println!("SKIPPED (oom_exec raises its score): no /proc/self/oom_score_adj");
        return;
    }
    let scenario = Scenario::new();
    let result = scenario.run(&["--oom-exec", "sh", "-c", "cat /proc/self/oom_score_adj"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.trim(), "1000");
    let echoed = scenario.run(&["--oom-exec", "echo", "hello-scope"]);
    assert_eq!(echoed.code, Some(0), "{}", echoed.stderr);
    assert_eq!(echoed.stdout, "hello-scope\n");
    let missing = scenario.run(&["--oom-exec", "/nonexistent-program"]);
    assert_eq!(missing.code, Some(1), "{}", missing.stderr);
    assert!(
        missing
            .stderr
            .contains("could not run /nonexistent-program"),
        "{}",
        missing.stderr
    );
    let usage = scenario.run(&["--oom-exec"]);
    assert_eq!(usage.code, Some(2), "{}", usage.stderr);
}

#[test]
fn a_scoped_hog_leaves_journal_oom_evidence() {
    if !scope_available() {
        println!("SKIPPED (a scoped hog leaves journal oom evidence): no systemd user manager");
        return;
    }
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("SKIPPED (a scoped hog leaves journal oom evidence): no python3");
        return;
    }
    let scenario = Scenario::new();
    let hog = scenario.path("hog.py");
    fs::write(&hog, "a=[bytearray(1000000) for _ in range(2000)]\n").unwrap();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let unit = format!("thirdshift-test-{}-{nanos}.scope", std::process::id());
    let binary = env!("CARGO_BIN_EXE_thirdshift");
    let status = std::process::Command::new("systemd-run")
        .args([
            "--user",
            "--scope",
            "-p",
            "MemoryHigh=50M",
            "-p",
            "MemoryMax=50M",
            "-p",
            "MemorySwapMax=0",
            &format!("--unit={unit}"),
            "--",
            binary,
            "--oom-exec",
            "python3",
        ])
        .arg(&hog)
        .status()
        .unwrap();
    assert!(!status.success(), "the hog must die under its cap");
    // The manager's `oom-kill` result survives the scope itself: it is the
    // outer launcher's evidence for a `memory_cap` Void.
    let mut found = false;
    for _ in 0..6 {
        let journal = std::process::Command::new("journalctl")
            .args(["--user", "-u", &unit, "--no-pager", "-o", "cat"])
            .output()
            .unwrap();
        if String::from_utf8_lossy(&journal.stdout)
            .lines()
            .any(|line| line.contains("Failed with result 'oom-kill'"))
        {
            found = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    assert!(found, "the scope's journal must record its oom-kill");
}
