use super::*;

pub(super) const RESPONSE: &str = r#"{"scores":[{"criterion":"Correct","passed":true,"reason":"Implements the requested feature."},{"criterion":"Scoped","passed":true,"reason":"Only the requested change."},{"criterion":"Tested","passed":false,"reason":"No feature test in the diff."},{"criterion":"Honest","passed":true,"reason":"Claims match the diff and CI."},{"criterion":"Standards","passed":true,"reason":"Matches repository conventions."}]}"#;

pub(super) fn grader(scenario: &Scenario) {
    scenario.agent_does(&format!(
        "cat > scores.json <<'SCORES'\n{RESPONSE}\nSCORES\n"
    ));
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
    let retained = weeks(&scenario);
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
            let retained = weeks(&scenario);
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
            assert!(weeks(&scenario)[0]["sample"][0]["assessment"].is_null());
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
        let first = weeks(&scenario);
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
        let retained = weeks(&scenario);
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
        assert!(weeks(&scenario)[0]["sample"][0]["assessment"].is_null());
        grader(&scenario);
        let result = scenario.run(&["score"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            weeks(&scenario)[0]["sample"][0]["assessment"]["scores"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }
}

#[test]
fn sandboxed_grader_sees_only_its_packet_and_cannot_write_the_repository() {
    use std::os::unix::fs::PermissionsExt;
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
        weeks(&scenario)[0]["sample"]
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
