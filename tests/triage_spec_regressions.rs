//! Whole-Spec regressions for Report bases and private finding notifications.
mod support;

use serde_json::json;
use support::Scenario;

fn recommend(scenario: &Scenario, recommendation: &serde_json::Value) {
    let serialized = recommendation.to_string().replace('\'', "'\\''");
    scenario.agent_does(&format!(
        "printf '%s\\n' 'Triage recommendation: {serialized}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\""
    ));
}

#[test]
fn report_modes_honor_the_requested_base_branch() {
    for mode in ["stdin", "file"] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["bug"]);
        scenario.origin_has_branch("investigate", "main", &[]);
        scenario.origin_has_commit("investigate", "base-marker", "selected base", "Base");
        recommend(
            &scenario,
            &json!({
                "category": "bug",
                "state": "ready-for-agent",
                "issues": [{"title": "Handle an empty cache", "body": "## Agent Brief\n\nReproduce with an empty cache."}],
                "investigation": "The cache read assumes an entry exists."
            }),
        );
        let script = std::fs::read_to_string(scenario.path("claude-script.sh")).unwrap();
        scenario.agent_does(&format!(
            "cat base-marker > '{}'\n{script}",
            scenario.path("selected-base").display()
        ));
        let report = "Investigate cache misses.";
        let result = if mode == "stdin" {
            scenario.run_with_input(
                &["triage", "base", "investigate", "--recommend-only"],
                report,
            )
        } else {
            let path = scenario.path("report.txt");
            std::fs::write(&path, report).unwrap();
            scenario.run(&[
                "triage",
                "--file",
                path.to_str().unwrap(),
                "base",
                "investigate",
                "--recommend-only",
            ])
        };
        assert_eq!(result.code, Some(0), "{mode}: {}", result.stderr);
        assert_eq!(
            std::fs::read_to_string(scenario.path("selected-base")).unwrap(),
            "selected base",
            "{mode}"
        );
        let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["issues"][0]["title"], "Handle an empty cache");
        assert_eq!(scenario.claude_calls().len(), 1, "{mode}");
        assert!(scenario.gh_calls_of("issue", "create").is_empty());
    }
}

#[test]
fn private_security_finding_triage_does_not_email_its_write_up() {
    for mode in ["issue", "pass"] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["bug", "security-finding"]);
        scenario.issue_labelled(7, &["security-finding", "needs-triage"]);
        scenario.issue_titled(7, "Private finding");
        let secret = "PRIVATE_TRIAGE_FINDING_WRITE_UP_1695";
        let questionnaire_secret = "PRIVATE_TRIAGE_QUESTIONNAIRE_EVIDENCE_1695";
        let mut state = scenario.gh_state();
        state["private"] = json!(true);
        state["bodies"]["7"] = json!(format!("Private vulnerability evidence: {secret}"));
        scenario.write_gh_state(&state);
        recommend(
            &scenario,
            &json!({
                "category": "bug",
                "state": "needs-info",
                "issues": [{
                    "title": "Private finding",
                    "body": "Investigated the private finding; evidence stays on this private issue.",
                    "questionnaire": format!("Does {questionnaire_secret} affect the second tenant?")
                }],
                "investigation": format!("Confirmed the private vulnerability: {secret}")
            }),
        );
        let resend = support::resend::ResendStandIn::replying(200, r#"{"id":"private-triage"}"#);
        let args = if mode == "issue" {
            vec!["triage", "7", "email", "day@example.com"]
        } else {
            vec!["triage", "email", "day@example.com"]
        };
        let result = scenario.run_with_env(
            &args,
            &[
                ("RESEND_API_KEY", "test-key"),
                ("THIRDSHIFT_RESEND_URL", resend.url()),
            ],
        );
        assert_eq!(result.code, Some(0), "{mode}: {}", result.stderr);
        assert!(result.stdout.contains(secret), "{mode}: {}", result.stdout);
        assert!(
            result.stdout.contains(questionnaire_secret),
            "{mode}: {}",
            result.stdout
        );
        let requests = resend.requests();
        assert_eq!(requests.len(), 1, "{mode}: {requests:?}");
        let (_, notification) = support::resend::saved_notification(&scenario, "triage");
        assert!(
            notification.contains(&scenario.issue_url(7)),
            "{mode}: {notification}"
        );
        assert!(
            notification.contains("Private finding"),
            "{mode}: {notification}"
        );
        for private in [secret, questionnaire_secret] {
            assert!(
                !notification.contains(private),
                "{mode}: leaked {private}: {notification}"
            );
            assert!(
                !requests[0].body.to_string().contains(private),
                "{mode}: leaked {private} to Resend"
            );
        }
        support::resend::assert_saved_notification(&scenario, "triage", &requests[0]);
    }
}

#[test]
fn failed_private_finding_triage_keeps_diagnostics_out_of_email() {
    for (mode, failure) in [
        ("issue", "recommendation"),
        ("pass", "recommendation"),
        ("issue", "harness"),
        ("pass", "harness"),
    ] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["bug", "security-finding"]);
        scenario.issue_labelled(7, &["security-finding", "needs-triage"]);
        scenario.issue_titled(7, "Private finding");
        let secret = "PRIVATE_FAILED_TRIAGE_EVIDENCE_1695";
        if failure == "harness" {
            scenario.agent_does(&format!("printf '%s\\n' '{secret}' >&2\nexit 1"));
        } else {
            recommend(
                &scenario,
                &json!({
                    "category": "bug", "state": "ready-for-human", "issues": secret,
                    "investigation": "Private evidence"
                }),
            );
        }
        let resend =
            support::resend::ResendStandIn::replying(200, r#"{"id":"failed-private-triage"}"#);
        let args = if mode == "issue" {
            vec!["triage", "7", "email", "day@example.com"]
        } else {
            vec!["triage", "email", "day@example.com"]
        };
        let result = scenario.run_with_env(
            &args,
            &[
                ("RESEND_API_KEY", "test-key"),
                ("THIRDSHIFT_RESEND_URL", resend.url()),
            ],
        );
        assert_ne!(result.code, Some(0), "{mode}: {}", result.stderr);
        assert!(result.stderr.contains(secret), "{mode}: {}", result.stderr);
        let requests = resend.requests();
        assert_eq!(requests.len(), 1, "{mode}: {requests:?}");
        let (_, notification) = support::resend::saved_notification(&scenario, "triage");
        assert!(!notification.contains(secret), "{mode}: {notification}");
        assert!(
            !requests[0].body.to_string().contains(secret),
            "{mode}: {requests:?}"
        );
        assert!(
            notification.contains("Private finding"),
            "{mode}: {notification}"
        );
        assert!(
            notification.contains(&scenario.issue_url(7)),
            "{mode}: {notification}"
        );
        support::resend::assert_saved_notification(&scenario, "triage", &requests[0]);
    }
}
