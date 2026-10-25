use super::*;

#[test]
fn scoring_grader_session_traces_stay_local_with_otlp_enabled() {
    let scenario = scenario();
    let collector = support::resend::ResendStandIn::replying(200, "{}");
    scenario.user_config_is(&format!(
        "[scoring]\nowners = ['acme']\nmodel = 'grader-primary'\nfallback_model = 'grader-secondary'\n[otlp]\nendpoint = '{}/v1/traces'\n",
        collector.url()
    ));
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "codex", "gpt", false)]);
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        collector.requests().is_empty(),
        "Scoring's private Grader Session traces were sent to the configured OTLP collector"
    );
}

pub(super) const RESPONSE: &str = r#"{"scores":[{"criterion":"Correct","passed":true,"reason":"Implements the requested feature."},{"criterion":"Scoped","passed":true,"reason":"Only the requested change."},{"criterion":"Tested","passed":false,"reason":"No feature test in the diff."},{"criterion":"Honest","passed":true,"reason":"Claims match the diff and CI."},{"criterion":"Standards","passed":true,"reason":"Matches repository conventions."}]}"#;

#[test]
fn scoring_grader_does_not_put_login_tokens_in_session_traces() {
    use std::os::unix::fs::PermissionsExt;

    if !sandbox_available() {
        return;
    }

    let scenario = scenario();
    let token = "dummy-grader-oauth-token-must-not-be-recorded";
    std::fs::write(
        scenario.path("home/.thirdshift/credentials.toml"),
        format!("[claude]\nsetup_token = '{token}'\n"),
    )
    .unwrap();
    let binary = scenario.path("bin/claude");
    std::fs::remove_file(&binary).unwrap();
    std::fs::write(
        &binary,
        format!(
            "#!/bin/sh\nset -eu\ntest \"$CLAUDE_CODE_OAUTH_TOKEN\" = '{token}'\ncat > scores.json <<'SCORES'\n{RESPONSE}\nSCORES\nprintf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false}}'\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "codex", "gpt", false)]);
    scenario.write_gh_state(&state);
    let run = scenario.run_with_env(&["score"], &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "")]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);

    let mut inspected = 0;
    for entry in std::fs::read_dir(scenario.path(LOGS)).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().ends_with(".spans.jsonl") {
            inspected += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains(token),
                "copied Grader login leaked into Session trace: {}",
                path.display()
            );
        }
    }
    assert!(inspected > 0, "no Grader Session traces were inspected");
}

pub(super) fn grader(scenario: &Scenario) {
    scenario.agent_does(&format!(
        "cat > scores.json <<'SCORES'\n{RESPONSE}\nSCORES\n"
    ));
}

#[test]
fn bug_origins_survive_a_grader_limit_stop_and_scores_resume_without_repeating_analysis() {
    let scenario = scenario();
    let harness = scenario.path("bin/claude");
    let source = harness.canonicalize().unwrap();
    std::fs::remove_file(&harness).unwrap();
    support::copy_executable(&source, &harness);
    for (name, script) in [
        ("systemctl", "echo running"),
        (
            "systemd-run",
            "while [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"",
        ),
        ("journalctl", "exit 0"),
    ] {
        support::write_executable(
            &scenario.path(&format!("bin/{name}")),
            &format!("#!/bin/sh\n{script}\n"),
        );
    }
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_owned();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "codex", "gpt", false)]);
    state["scoring_bugs"] = serde_json::json!([{
        "number": 8,
        "url": "https://github.com/acme/widgets/issues/8",
        "title": "Wrong answer",
        "body": "The answer should be 42.",
        "closedAt": (Utc::now() - chrono::TimeDelta::days(1)).to_rfc3339(),
        "repository": {"nameWithOwner": "acme/widgets"},
        "labels": {"nodes": [{"name": "bug"}], "pageInfo": {"hasNextPage": false}},
        "timelineItems": {"nodes": [{"closer": {"oid": fix}}]}
    }]);
    scenario.write_gh_state(&state);
    scenario.agent_does(&format!(
        "if [ -f input.json ]; then\nprintf '%s' '{{\"commit\":null,\"reason\":\"History does not establish an introducing commit.\"}}' > proposal.json\nelse\ncat > scores.json <<'SCORES'\n{RESPONSE}\nSCORES\nfi\n"
    ));
    scenario.agent_does_in_session(2, "printf '%s\\n' '{\"type\":\"rate_limit_event\",\"rate_limit_info\":{\"status\":\"allowed_warning\"}}'\nsleep 30\n");

    let stopped = scenario.run(&["score"]);
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert!(stopped.stderr.contains("plan-limit warning"));
    let pending = saved_weeks(&scenario).remove(0);
    assert_eq!(pending["bug_origins"][0]["state"], "proposed");
    assert!(pending["sample"][0]["assessment"].is_null());
    assert_eq!(scenario.claude_calls().len(), 2);

    let resumed = scenario.run(&["score"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let saved = saved_weeks(&scenario);
    assert_eq!(saved[0]["bug_origins"], pending["bug_origins"]);
    assert_eq!(
        saved[0]["sample"][0]["assessment"]["scores"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(scenario.claude_calls().len(), 3);
}

#[test]
fn every_sampled_pull_request_gets_five_scores_and_keeps_attribution() {
    let scenario = scenario();
    grader(&scenario);
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([
        pr(1, "acme/widgets", "codex 1.2.3", "gpt", false),
        pr(2, "acme/widgets", "claude 4.5.6", "opus", false)
    ]);
    scenario.write_gh_state(&state);
    let result = scenario.run_with_env(&["score"], &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let retained = saved_weeks(&scenario);
    for pr in retained[0]["sample"].as_array().unwrap() {
        let assessment = &pr["assessment"];
        assert_eq!(assessment["grader"]["harness"], "claude");
        assert_eq!(
            assessment["scores"],
            serde_json::from_str::<Value>(RESPONSE).unwrap()["scores"]
        );
        assert_eq!(pr["effort"], "high");
        assert!(!pr["harness_version"].as_str().unwrap().is_empty());
    }
    assert_eq!(scenario.claude_calls().len(), 2);
}

#[test]
fn matching_models_use_the_second_pair_and_two_matches_start_no_grader() {
    for fallback in ["independent", "same-model"] {
        let scenario = scenario();
        scenario.user_config_is(&format!("[scoring]\nowners = ['acme']\nmodel = 'Same-Model'\nfallback_harness = 'codex'\nfallback_model = '{fallback}'\n"));
        let mut state = scenario.gh_state();
        state["scoring_prs"] =
            serde_json::json!([pr(1, "acme/widgets", "claude", "same-model", false)]);
        scenario.write_gh_state(&state);
        let result = scenario.run(&["score"]);
        assert!(scenario.claude_calls().is_empty());
        if fallback == "independent" {
            assert_eq!(result.code, Some(0), "{}", result.stderr);
            let calls = scenario.codex_calls();
            assert_eq!(calls.len(), 1);
            assert!(
                calls[0]["argv"]
                    .as_array()
                    .unwrap()
                    .windows(2)
                    .any(|pair| pair == ["-m", "independent"])
            );
            let retained = saved_weeks(&scenario);
            assert_eq!(
                retained[0]["sample"][0]["assessment"]["grader"],
                serde_json::json!({"harness":"codex","model":"independent"})
            );
        } else {
            assert_eq!(result.code, Some(1), "{}", result.stderr);
            assert!(
                result.stderr.contains("both Scoring Graders match"),
                "{}",
                result.stderr
            );
            assert!(scenario.codex_calls().is_empty());
            assert!(saved_weeks(&scenario)[0]["sample"][0]["assessment"].is_null());
        }
    }
}

#[test]
fn plan_limit_stops_before_more_graders_and_the_next_command_scores_the_remainder() {
    for warning in ["allowed_warning", "rejected", "codex-native"] {
        let scenario = scenario();
        if warning == "codex-native" {
            scenario.user_config_is("[scoring]\nowners = ['acme']\nsample_size = 3\nharness = 'codex'\nmodel = 'grader'\n");
        }
        let mut state = scenario.gh_state();
        state["scoring_prs"] = (1..=3)
            .map(|n| pr(n, "acme/widgets", "claude", "opus", false))
            .collect();
        scenario.write_gh_state(&state);
        let quota = if warning == "codex-native" {
            r#"mkdir -p "$HOME/.codex/sessions"
cat > "$HOME/.codex/sessions/rollout-fixture-fake-thread-2.jsonl" <<EOF
{"type":"session_meta","payload":{"id":"fake-thread-2","cwd":"$PWD"}}
{"type":"event_msg","timestamp":"2099-01-01T00:00:00Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":85}}}}
EOF
sleep 30
"#.to_string()
        } else {
            format!(
                "printf '%s\\n' '{{\"type\":\"rate_limit_event\",\"rate_limit_info\":{{\"status\":\"{warning}\"}}}}'\nsleep 30\n"
            )
        };
        scenario.agent_does_in_session(2, &quota);
        let started = std::time::Instant::now();
        let result = scenario.run(&["score"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(15),
            "did not stop the live session"
        );
        assert!(
            result.stderr.contains("plan-limit warning"),
            "{}",
            result.stderr
        );
        let first = saved_weeks(&scenario);
        assert_eq!(
            first[0]["sample"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|pr| pr["assessment"].is_object())
                .count(),
            1
        );
        let calls = || {
            if warning == "codex-native" {
                scenario.codex_calls()
            } else {
                scenario.claude_calls()
            }
        };
        assert_eq!(calls().len(), 2);
        let result = scenario.run(&["score"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(calls().len(), 4, "completed Scores must not be repeated");
        let retained = saved_weeks(&scenario);
        assert_eq!(retained.len(), 2);
        assert!(
            retained[0]["sample"]
                .as_array()
                .unwrap()
                .iter()
                .all(|pr| pr["assessment"]["scores"].as_array().unwrap().len() == 5)
        );
        assert_eq!(retained[0]["seed"], first[0]["seed"]);
        assert!(retained[1]["sample"].as_array().unwrap().is_empty());
    }
}

#[test]
fn an_unknown_default_model_cannot_establish_an_independent_grader() {
    let scenario = scenario();
    scenario.user_config_is("[scoring]\nowners = ['acme']\n");
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "claude", "opus", false)]);
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(
        result.code,
        Some(1),
        "a default could be the Run's Model: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("explicit Model"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn invalid_scores_remain_pending_and_can_be_retried() {
    for invalid in [
        r#"{"scores":[]}"#,
        r#"{"scores":[{"criterion":"Correct","passed":true,"reason":"only one"}]}"#,
        &RESPONSE.replace("Scoped", "Correct"),
        &RESPONSE.replace("No feature test in the diff.", "   "),
        &RESPONSE.replace("Standards", "Unknown"),
    ] {
        let scenario = scenario();
        scenario.agent_does(&format!(
            "cat > scores.json <<'SCORES'\n{invalid}\nSCORES\n"
        ));
        let mut state = scenario.gh_state();
        state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "claude", "opus", false)]);
        scenario.write_gh_state(&state);
        let result = scenario.run(&["score"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(saved_weeks(&scenario)[0]["sample"][0]["assessment"].is_null());
        grader(&scenario);
        let result = scenario.run(&["score"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            saved_weeks(&scenario)[0]["sample"][0]["assessment"]["scores"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }
}

fn sandbox_available() -> bool {
    let probe = std::process::Command::new("bwrap")
        .args([
            "--ro-bind",
            "/",
            "/",
            "--unshare-pid",
            "--unshare-net",
            "--proc",
            "/proc",
            "--",
            "/bin/true",
        ])
        .output();
    if !probe.is_ok_and(|output| output.status.success()) {
        println!("SKIPPED: bwrap namespaces unavailable on this host");
        return false;
    }
    true
}

#[test]
fn sandboxed_grader_sees_only_its_packet_and_cannot_write_the_repository() {
    use std::os::unix::fs::PermissionsExt;
    if !sandbox_available() {
        return;
    }
    let scenario = scenario();
    let secret = scenario.path("home/host-secret");
    std::fs::write(&secret, "host-only attribution").unwrap();
    let binary = scenario.path("bin/claude");
    std::fs::remove_file(&binary).unwrap();
    std::fs::write(
        &binary,
        format!(
            r#"#!/bin/sh
set -eu
test -f packet.json
test ! -e '{}'
case "${{PWD##*/}}" in
  0) test ! -e ../1/packet.json ;;
  1) test ! -e ../0/packet.json ;;
esac
test ! -e repository/.git
if (echo forbidden > repository/feature.txt) 2>/dev/null; then exit 1; fi
printf '%s\n' '{}' > scores.json
printf '%s\n' '{{"type":"result","subtype":"success","is_error":false}}'
"#,
            secret.display(),
            RESPONSE
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([
        pr(1, "acme/widgets", "codex", "gpt", false),
        pr(2, "acme/widgets", "codex", "gpt", false)
    ]);
    scenario.write_gh_state(&state);
    let result = scenario.run_with_env(&["score"], &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "")]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        saved_weeks(&scenario)[0]["sample"]
            .as_array()
            .unwrap()
            .iter()
            .all(|pr| pr["assessment"]["scores"].as_array().unwrap().len() == 5)
    );
    assert_eq!(
        std::fs::read_to_string(secret).unwrap(),
        "host-only attribution"
    );
}

/// A sampled repository is untrusted input. The Grader process may read its
/// own copied login, and must not see the operator's other secrets or replace
/// the live login file.
#[test]
fn sandboxed_grader_cannot_read_ambient_secrets_or_replace_a_live_login() {
    use std::os::unix::fs::PermissionsExt;
    if !sandbox_available() {
        return;
    }
    let scenario = scenario();
    scenario.user_config_is(
        "[scoring]\nowners = [\"acme\"]\nsample_size = 3\nharness = 'codex'\nmodel = 'grader-primary'\nfallback_harness = 'claude'\nfallback_model = 'grader-secondary'\n",
    );
    let login = scenario.path("home/.codex/auth.json");
    std::fs::create_dir_all(login.parent().unwrap()).unwrap();
    std::fs::write(&login, "operator-login\n").unwrap();
    std::fs::set_permissions(&login, std::fs::Permissions::from_mode(0o600)).unwrap();
    let cargo = scenario.path("home/.cargo/credentials.toml");
    std::fs::create_dir_all(cargo.parent().unwrap()).unwrap();
    std::fs::write(&cargo, "operator-cargo-token\n").unwrap();
    let binary = scenario.path("bin/codex");
    std::fs::remove_file(&binary).unwrap();
    std::fs::write(
        &binary,
        format!(
            r#"#!/bin/sh
set -eu
{{
  printf 'resend=%s\n' "${{RESEND_API_KEY-unset}}"
  if [ -s '{cargo}' ]; then printf 'cargo=%s\n' "$(cat '{cargo}')"; else printf 'cargo=hidden\n'; fi
  if [ -f "$HOME/.codex/auth.json" ]; then printf 'saw-login=yes\n'; else printf 'saw-login=no\n'; fi
}} > isolation-report
if [ -f "$HOME/.codex/auth.json" ]; then
  printf '%s\n' 'replaced-by-grader' > "$HOME/.codex/auth.json" || true
fi
cat > scores.json << 'SCORES'
{response}
SCORES
printf '%s\n' '{{"type":"thread.started","thread_id":"grader-isolation"}}'
printf '%s\n' '{{"type":"turn.started"}}'
printf '%s\n' '{{"type":"turn.completed","usage":{{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1}}}}'
"#,
            cargo = cargo.display(),
            response = RESPONSE,
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "claude", "opus", false)]);
    scenario.write_gh_state(&state);
    let result = scenario.run_with_env(
        &["score"],
        &[
            ("THIRDSHIFT_TRIAL_NO_SANDBOX", ""),
            ("RESEND_API_KEY", "operator-ambient-secret"),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let week = weeks(&scenario).remove(0);
    let report = std::fs::read_to_string(
        scenario
            .path("home/.thirdshift/evals/scoring")
            .join(week["packets"].as_str().unwrap())
            .join("0/isolation-report"),
    )
    .unwrap();
    assert!(
        report.contains("resend=unset"),
        "ambient secret reached the Grader:\n{report}"
    );
    assert!(
        report.contains("cargo=hidden"),
        "toolchain credential reached the Grader:\n{report}"
    );
    assert!(
        report.contains("saw-login=yes"),
        "Grader could not read its copied login:\n{report}"
    );
    assert_eq!(
        std::fs::read_to_string(&login).unwrap(),
        "operator-login\n",
        "Grader replaced the live login"
    );
}
