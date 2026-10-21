mod support;

use serde_json::json;
use support::Scenario;
use support::resend::ResendStandIn;

const OPENS_PR: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'
"#;

fn old_finding() -> serde_json::Value {
    json!({
        "fingerprint": "old-input-bound",
        "title": "Old unchecked bound",
        "description": "Private old vulnerability evidence.",
        "proof_of_concept": {
            "test": "assert_bounded_input();",
            "command": "cargo test bounded_input",
            "head_exit_code": 101,
            "merge_base_exit_code": 101,
            "notes": "The bounded local fixture fails at both commits. Low likelihood and low impact; one session can add a bound.",
            "severity": "low",
            "fix_size": "single"
        }
    })
}

fn review_script(old: &serde_json::Value, introduced: &[&str]) -> String {
    format!(
        r#"
printf '%s' "$FAKE_CLAUDE_PROMPT" > review-prompt.txt
report=$(sed -n 's/^Private report file: `\(.*\)`\.$/\1/p' review-prompt.txt)
test -n "$report"
printf '%s\n' '{old}' > "$report"
printf '%s\n' 'Security review: {outcome}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        outcome = json!({
            "unaddressed_count": introduced.len(),
            "findings": introduced,
            "pre_existing_count": old.as_array().unwrap().len()
        })
    )
}

fn assert_old_finding_stays_private(body: &str, result: &support::RunResult) {
    for private in [
        "Old unchecked bound",
        "old-input-bound",
        "Private old vulnerability evidence.",
        "assert_bounded_input();",
    ] {
        assert!(!body.contains(private));
        assert!(!result.stderr.contains(private));
        assert!(!result.stdout.contains(private));
    }
}

#[test]
fn explicit_incomplete_review_surfaces_its_reason_and_records_old_findings_privately() {
    for old in [json!([]), json!([old_finding()])] {
        let scenario = Scenario::new();
        scenario.agent_does_in_session(1, OPENS_PR);
        let script = format!(
            "{}\nprintf '%s\\n' 'Security review: incomplete: required sandbox isolation is unavailable' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
            review_script(&old, &[])
        );
        scenario.agent_does_in_session(2, &script);
        let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let reason = "Security review incomplete: required sandbox isolation is unavailable";
        assert!(result.stderr.contains(reason), "{}", result.stderr);
        let state = scenario.gh_state();
        let pr = &state["prs"][0];
        assert_eq!(pr["state"], "OPEN");
        assert_eq!(pr["isDraft"], false);
        assert!(pr["body"].as_str().unwrap().contains(reason));
        if !old.as_array().unwrap().is_empty() {
            assert_eq!(state["advisories"].as_array().unwrap().len(), 1);
            assert_eq!(state["advisories"][0]["severity"], "low");
        }
        assert_old_finding_stays_private(pr["body"].as_str().unwrap(), &result);
    }
}

#[test]
fn explicit_incomplete_reason_is_capped_without_breaking_unicode() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    let reason = format!("{}discarded suffix", "λ".repeat(510));
    scenario.agent_does_in_session(2, &format!(
        "{}\nprintf '%s\\n' 'Security review: incomplete: {reason}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
        review_script(&json!([]), &[])
    ));
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "no-merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let expected = format!("Security review incomplete: {}", "λ".repeat(500));
    assert!(result.stderr.contains(&expected), "{}", result.stderr);
    let state = scenario.gh_state();
    let body = state["prs"][0]["body"].as_str().unwrap();
    assert!(body.contains(&format!("- {expected}\n")), "{body}");
    assert!(!body.contains(&"λ".repeat(501)));
    assert!(!result.stderr.contains(&"λ".repeat(501)));
    assert!(!body.contains("discarded suffix"));
    assert!(!result.stderr.contains("discarded suffix"));
}

#[test]
fn explicit_incomplete_reviews_still_validate_the_entire_private_report() {
    let mut invalid = old_finding();
    invalid["proof_of_concept"]["merge_base_exit_code"] = json!(0);
    let valid = review_script(&json!([old_finding()]), &[]);
    for script in [
        format!("{valid}\nrm \"$report\"\n"),
        format!("{valid}\nprintf '%s' 'Private old vulnerability evidence.' > \"$report\"\n"),
        review_script(&json!([old_finding(), invalid]), &[]),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_in_session(1, OPENS_PR);
        scenario.agent_does_in_session(2, &format!(
            "{script}\nprintf '%s\\n' 'Security review: incomplete: blocked validation' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n"
        ));
        let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let state = scenario.gh_state();
        assert!(state["advisories"].is_null());
        let body = state["prs"][0]["body"].as_str().unwrap();
        let generic = "Security review incomplete: session failed, ended early or omitted a valid final line; see Session log";
        assert!(body.contains(generic), "{body}");
        assert!(result.stderr.contains(generic), "{}", result.stderr);
        assert_old_finding_stays_private(body, &result);
    }
}

#[test]
fn an_old_finding_is_recorded_privately_without_holding_self_merge() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(2, &review_script(&json!([old_finding()]), &[]));
    let result = scenario.run(&[
        &scenario.issue_url(7),
        "security-review",
        "merge",
        "security-fix",
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let records = state["advisories"]
        .as_array()
        .expect("private draft advisory");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["state"], "draft");
    assert_eq!(records[0]["severity"], "low");
    let description = records[0]["description"].as_str().unwrap();
    assert!(description.contains("Fingerprint: `old-input-bound`"));
    assert!(description.contains("Private old vulnerability evidence."));
    assert!(description.contains("assert_bounded_input();"));
    assert!(description.contains(scenario.launch_git(&["rev-parse", "main"]).trim()));
    assert_eq!(state["prs"][0]["state"], "MERGED");
    assert_eq!(
        scenario.claude_calls().len(),
        2,
        "old findings belong to a Security run"
    );
    for private in [
        "Old unchecked bound",
        "old-input-bound",
        "Private old vulnerability evidence.",
        "assert_bounded_input();",
    ] {
        assert!(!state["prs"][0]["body"].as_str().unwrap().contains(private));
        assert!(!result.stderr.contains(private));
        assert!(!result.stdout.contains(private));
    }
    let reports = scenario.entries("home/.thirdshift/logs/acme/widgets/security-reviews");
    assert_eq!(reports.len(), 1);
    let kept: serde_json::Value = serde_json::from_slice(
        &std::fs::read(scenario.path(&format!(
            "home/.thirdshift/logs/acme/widgets/security-reviews/{}/pre-existing.json",
            reports[0]
        )))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(kept, json!([old_finding()]));
}

#[test]
fn mixed_old_and_introduced_findings_keep_the_introduced_hold_and_old_details_private() {
    for harness in ["claude", "codex"] {
        let scenario = Scenario::new();
        scenario.agent_does_in_session(1, OPENS_PR);
        scenario.agent_does_in_session(
            2,
            &review_script(&json!([old_finding()]), &["Introduced cross-tenant read"]),
        );
        let result = scenario.run(&[
            &scenario.issue_url(7),
            "security-review",
            "merge",
            "harness",
            harness,
        ]);
        assert_eq!(result.code, Some(1), "{harness}: {}", result.stderr);
        let state = scenario.gh_state();
        assert_eq!(state["advisories"].as_array().unwrap().len(), 1);
        assert_eq!(state["prs"][0]["state"], "OPEN");
        assert_eq!(state["prs"][0]["isDraft"], false);
        let body = state["prs"][0]["body"].as_str().unwrap();
        assert!(body.contains("Introduced cross-tenant read"));
        assert!(result.stderr.contains("Introduced cross-tenant read"));
        for private in [
            "Old unchecked bound",
            "old-input-bound",
            "Private old vulnerability evidence.",
            "assert_bounded_input();",
        ] {
            assert!(!body.contains(private));
            assert!(!result.stderr.contains(private));
        }
    }
}

#[test]
fn repeated_fingerprints_reuse_private_records_and_preserve_day_shift_grades() {
    for (private, record_state) in [
        (false, "draft"),
        (false, "published"),
        (false, "closed"),
        (true, "OPEN"),
        (true, "CLOSED"),
    ] {
        let scenario = Scenario::new();
        let mut state = scenario.gh_state();
        state["private"] = json!(private);
        scenario.write_gh_state(&state);
        scenario.agent_does_in_session(1, OPENS_PR);
        // Duplicates within one report must match just as later reviews do.
        scenario.agent_does_in_session(
            2,
            &review_script(&json!([old_finding(), old_finding()]), &[]),
        );
        let first = scenario.run(&[&scenario.issue_url(7), "security-review", "no-merge"]);
        assert_eq!(
            first.code,
            Some(0),
            "{private} {record_state}: {}",
            first.stderr
        );
        let mut state = scenario.gh_state();
        if private {
            assert!(state["advisories"].is_null());
            assert_eq!(state["issues"].as_object().unwrap().len(), 2);
            assert_eq!(
                state["labels"]["8"],
                json!(["security-finding", "needs-triage"])
            );
            assert!(
                state["bodies"]["8"]
                    .as_str()
                    .unwrap()
                    .contains("Private old vulnerability evidence.")
            );
            state["issues"]["8"] = json!(record_state);
            state["labels"]["8"] = json!(["security-finding"]);
        } else {
            assert_eq!(state["advisories"].as_array().unwrap().len(), 1);
            state["advisories"][0]["state"] = json!(record_state);
            state["advisories"][0]["severity"] = json!("low");
        }
        scenario.write_gh_state(&state);
        scenario.agent_does_in_session(3, "true");
        let mut repeated = old_finding();
        repeated["description"] = json!("A later review must not replace the grade or write-up.");
        scenario.agent_does_in_session(4, &review_script(&json!([repeated]), &[]));
        let second = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
        assert_eq!(
            second.code,
            Some(0),
            "{private} {record_state}: {}",
            second.stderr
        );
        let after = scenario.gh_state();
        assert_eq!(after["prs"][0]["state"], "MERGED");
        if private {
            assert_eq!(after["issues"]["8"], state["issues"]["8"]);
            assert_eq!(after["labels"]["8"], state["labels"]["8"]);
            assert_eq!(after["bodies"]["8"], state["bodies"]["8"]);
            assert_eq!(after["issues"].as_object().unwrap().len(), 2);
        } else {
            assert_eq!(after["advisories"], state["advisories"]);
        }
        let body = after["prs"][0]["body"].as_str().unwrap();
        assert!(!body.contains("Old unchecked bound"));
        assert!(!body.contains("old-input-bound"));
    }
}

#[test]
fn incomplete_private_reports_hold_self_merge_without_publishing_report_diagnostics() {
    let valid = review_script(&json!([old_finding()]), &[]);
    let mut passes_at_base = old_finding();
    passes_at_base["proof_of_concept"]["merge_base_exit_code"] = json!(0);
    let mut no_test = old_finding();
    no_test["proof_of_concept"]["test"] = json!("");
    for script in [
        valid.replace("\"pre_existing_count\":1", "\"pre_existing_count\":0"),
        format!("{valid}\nrm \"$report\"\n"),
        format!("{valid}\nprintf '%s' 'Private old vulnerability evidence.' > \"$report\"\n"),
        review_script(&json!([passes_at_base]), &[]),
        review_script(&json!([no_test]), &[]),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_in_session(1, OPENS_PR);
        scenario.agent_does_in_session(2, &script);
        let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let state = scenario.gh_state();
        assert!(state["advisories"].is_null());
        assert_eq!(state["prs"][0]["state"], "OPEN");
        assert_eq!(state["prs"][0]["isDraft"], false);
        let body = state["prs"][0]["body"].as_str().unwrap();
        assert!(body.contains("Security review incomplete"));
        assert!(!body.contains("Private old vulnerability evidence."));
        assert!(
            !result
                .stderr
                .contains("Private old vulnerability evidence.")
        );
    }
}

#[test]
fn review_uses_the_published_base_when_the_local_base_is_stale() {
    let scenario = Scenario::new();
    scenario.origin_has_commit(
        "main",
        "base-new.txt",
        "base change",
        "Advance the Base branch",
    );
    let base = scenario.origin_git(&["rev-parse", "main"]);
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(
        2,
        &format!(
            r#"
printf '%s' "$FAKE_CLAUDE_PROMPT" > review-prompt.txt
merge_base=$(sed -n 's/^Merge base commit: `\(.*\)`\.$/\1/p' review-prompt.txt)
test "$merge_base" = "$(git rev-parse origin/main)"
{}
"#,
            review_script(&json!([old_finding()]), &[])
        ),
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert!(
        state["advisories"][0]["description"]
            .as_str()
            .unwrap()
            .contains(base.trim())
    );
    assert_eq!(state["prs"][0]["state"], "MERGED");
}

#[test]
fn command_words_override_the_config_and_review_is_off_by_default() {
    for (config, word, enabled) in [
        ("", None, false),
        ("[security]\nreview = true\n", None, true),
        (
            "[security]\nreview = true\n",
            Some("no-security-review"),
            false,
        ),
        (
            "[security]\nreview = false\n",
            Some("security-review"),
            true,
        ),
    ] {
        let scenario = Scenario::new();
        scenario.user_config_is(config);
        scenario.agent_does_in_session(1, OPENS_PR);
        scenario.agent_does_in_session(2, &review_script(&json!([]), &[]));
        let url = scenario.issue_url(7);
        let mut args = vec![url.as_str(), "merge"];
        args.extend(word);
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{config} {word:?}: {}", result.stderr);
        assert_eq!(scenario.claude_calls().len(), if enabled { 2 } else { 1 });
        assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    }
}

#[test]
fn invalid_or_missing_final_lines_hold_self_merge_but_report_only_runs_continue() {
    for final_message in ["", "Security review: not JSON"] {
        for merge in [true, false] {
            let scenario = Scenario::new();
            scenario.agent_does_in_session(1, OPENS_PR);
            // Write literal data as a file through the existing fixture.
            std::fs::write(scenario.path("review-final.txt"), final_message).unwrap();
            scenario.agent_does_in_session(2, &format!(
                "{}\n{}",
                review_script(&json!([]), &[]),
                r#"cat "$(dirname "$FAKE_CLAUDE_SCRIPT")/review-final.txt" > "$FAKE_CLAUDE_FINAL_MESSAGE""#
            ));
            let result = scenario.run(&[
                &scenario.issue_url(7),
                "security-review",
                if merge { "merge" } else { "no-merge" },
            ]);
            assert_eq!(
                result.code,
                Some(if merge { 1 } else { 0 }),
                "{final_message}: {}",
                result.stderr
            );
            assert!(
                result.stderr.contains("Security review incomplete: session failed, ended early or omitted a valid final line; see Session log"),
                "{}",
                result.stderr
            );
            let pr = &scenario.gh_state()["prs"][0];
            assert_eq!(pr["state"], "OPEN");
            assert_eq!(pr["isDraft"], false);
            assert!(
                pr["body"]
                    .as_str()
                    .unwrap()
                    .contains("Security review incomplete: session failed, ended early or omitted a valid final line; see Session log")
            );
        }
    }
}

#[test]
fn safeguard_refusals_hold_self_merge_without_publishing_private_diagnostics() {
    for (harness, script, cause) in [
        (
            "claude",
            r#"printf '%s\n' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
            "Claude Code's [cyber] safeguard refusal",
        ),
        (
            "codex",
            r#"printf '%s\n' 'Cybersecurity safeguard refused: Private refusal evidence' > "$FAKE_CODEX_ERROR"
exit 1"#,
            "Codex's cybersecurity safeguard refusal",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_in_session(1, OPENS_PR);
        scenario.agent_does_in_session(2, script);
        let result = scenario.run(&[
            &scenario.issue_url(7),
            "security-review",
            "merge",
            "harness",
            harness,
        ]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(cause), "{}", result.stderr);
        let pr = &scenario.gh_state()["prs"][0];
        assert_eq!(pr["state"], "OPEN");
        assert_eq!(pr["isDraft"], false);
        let body = pr["body"].as_str().unwrap();
        assert!(body.contains(cause), "{body}");
        assert!(!body.contains("Private refusal evidence"));
    }
}

#[test]
fn base_fixes_inherit_the_security_review_choice() {
    let scenario = Scenario::new();
    scenario.agent_does_for_in_session(
        7,
        1,
        &format!(
            r#"{OPENS_PR}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure"}}]'
gh fake checks "$(git rev-parse origin/main)" '[{{"name":"test","conclusion":"failure"}}]'
"#
        ),
    );
    scenario.agent_does_for_in_session(7, 2, &review_script(&json!([]), &[]));
    scenario.agent_does_for_in_session(
        8,
        1,
        r#"
echo fixed > ci-fix.txt
git add ci-fix.txt
git commit -q -m 'Fix base CI'
gh pr create --base main --head issue-8 --title 'Fix base CI' --body 'Closes #8'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );
    scenario.agent_does_for_in_session(8, 2, &review_script(&json!([]), &[]));
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "base-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 4);
    // The Base fix's own Delivery runs the Security review its Command asked
    // for, on its own branch.
    assert_eq!(calls[3]["branch"], "issue-8");
    let prompt = calls[3]["prompt"].as_str().unwrap();
    assert!(prompt.contains("guidance mode"), "{prompt}");
    assert!(
        prompt.contains("branch issue-8 against Base branch main"),
        "{prompt}"
    );
    assert_eq!(scenario.gh_state()["prs"][1]["state"], "MERGED");
}

#[test]
fn an_unaddressed_introduced_finding_holds_self_merge_with_the_pr_ready() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(2, &review_script(&json!([]), &["Cross-tenant read"]));
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("Cross-tenant read"),
        "{}",
        result.stderr
    );
    let pr = &scenario.gh_state()["prs"][0];
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], false);
    assert!(pr["body"].as_str().unwrap().contains("### Security"));
    assert!(pr["body"].as_str().unwrap().contains("Cross-tenant read"));
}

#[test]
fn enabled_review_fixes_reach_origin_before_delivery_finishes() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(
        2,
        &format!(
            r#"
echo checked > security-fix.txt
git add security-fix.txt
git commit -q -m 'Fix reproduced finding'
{clean}
"#,
            clean = review_script(&json!([]), &[])
        ),
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario
            .origin_file("issue-7", "security-fix.txt")
            .as_deref(),
        Some("checked\n")
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert!(prompt.contains("guidance mode"));
    assert!(prompt.contains("Do not delegate"));
    assert!(prompt.contains(&format!(
        "git diff {}...HEAD",
        scenario.launch_git(&["rev-parse", "main"]).trim()
    )));
}

// Regression A.
#[test]
fn review_regression_a_security_hold_still_repairs_red_ci() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{OPENS_PR}\n{}",
            r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name":"test","conclusion":"success"}]'
"#
        ),
    );
    scenario.agent_does_in_session(2, &review_script(&json!([]), &["Cross-tenant read"]));
    scenario.agent_does_in_session(
        3,
        r#"
echo fixed > ci-fixed.txt
git add ci-fixed.txt
git commit -q -m 'Fix branch CI'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("Cross-tenant read"),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario.claude_calls().len(),
        3,
        "The held Merge run should repair red CI before handing over its PR: {}",
        result.stderr
    );
    assert_eq!(
        scenario.origin_file("issue-7", "ci-fixed.txt").as_deref(),
        Some("fixed\n")
    );
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "OPEN");
    assert_eq!(state["prs"][0]["isDraft"], false);
}

#[test]
fn a_refused_security_review_followed_by_a_repair_reports_the_reviews_session_log() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{OPENS_PR}\n{}",
            r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name":"test","conclusion":"success"}]'
"#
        ),
    );
    scenario.agent_does_in_session(
        2,
        r#"printf '%s\n' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE""#,
    );
    scenario.agent_does_in_session(
        3,
        r#"
echo fixed > ci-fixed.txt
git add ci-fixed.txt
git commit -q -m 'Fix branch CI'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );
    let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
    let result = scenario.run_with_env(
        &[
            &scenario.issue_url(7),
            "security-review",
            "merge",
            "email",
            "you@example.com",
        ],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test"),
        ],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("Security review refused"),
        "{}",
        result.stderr
    );
    // The Repair ran after the review, so it is the most recent session.
    assert!(
        result.stderr.contains("-repair-1.jsonl"),
        "{}",
        result.stderr
    );
    let reported = result
        .stderr
        .lines()
        .find_map(|line| line.split_once("session log: ").map(|(_, log)| log))
        .unwrap_or_else(|| panic!("no session log line: {}", result.stderr));
    assert!(
        reported.ends_with("-security-review.jsonl"),
        "{}",
        result.stderr
    );
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(
        text.contains(&format!("Session log:  {reported}\n")),
        "{text}"
    );
}

#[test]
fn a_failing_repair_after_a_security_review_reports_the_repairs_session_log() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{OPENS_PR}\n{}",
            r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name":"test","conclusion":"success"}]'
"#
        ),
    );
    scenario.agent_does_in_session(2, &review_script(&json!([]), &[]));
    scenario.agent_does_in_session(3, "exit 3\n");
    let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
    let result = scenario.run_with_env(
        &[
            &scenario.issue_url(7),
            "security-review",
            "merge",
            "email",
            "you@example.com",
        ],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test"),
        ],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let reported = result
        .stderr
        .lines()
        .find_map(|line| line.split_once("session log: ").map(|(_, log)| log))
        .unwrap_or_else(|| panic!("no session log line: {}", result.stderr));
    assert!(reported.ends_with("-repair-1.jsonl"), "{}", result.stderr);
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(
        text.contains(&format!("Session log:  {reported}\n")),
        "{text}"
    );
}

// Regression B.
#[test]
fn review_regression_b_body_write_failure_preserves_refusal_and_readiness() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(
        2,
        r#"
gh fake fails 'api --method PATCH repos/acme/widgets/pulls/1'
printf '%s\n' 'API Error: [cyber] Private refusal evidence' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "OPEN");
    assert_eq!(
        state["prs"][0]["isDraft"], false,
        "A refused review must leave the already-ready PR ready when only its summary PATCH fails: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("Security review refused"),
        "The failure must retain the Security refusal cause: {}",
        result.stderr
    );
}

// Regression C.
#[test]
fn review_regression_c_continuation_replaces_the_previous_security_outcome() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(2, &review_script(&json!([]), &["Cross-tenant read"]));
    let first = scenario.run(&[&scenario.issue_url(7), "security-review", "no-merge"]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    assert!(
        scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Cross-tenant read")
    );
    scenario.agent_does_in_session(
        3,
        r#"
echo fixed > feature.txt
git add feature.txt
git commit -q -m 'Fix the previously reported finding'
"#,
    );
    scenario.agent_does_in_session(4, &review_script(&json!([]), &[]));
    let second = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(second.code, Some(0), "{}", second.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "MERGED");
    let body = state["prs"][0]["body"].as_str().unwrap();
    assert!(
        !body.contains("Cross-tenant read"),
        "The fixed finding remains falsely listed as unaddressed after a clean review: {body}"
    );
    assert_eq!(
        body.matches("### Security review outcome").count(),
        1,
        "{body}"
    );
}

#[test]
fn review_old_finding_can_be_taken_by_next_security_run() {
    for private in [false, true] {
        let scenario = Scenario::new();
        let mut state = scenario.gh_state();
        state["private"] = json!(private);
        scenario.write_gh_state(&state);
        scenario.agent_does_in_session(1, OPENS_PR);
        scenario.agent_does_in_session(2, &review_script(&json!([old_finding()]), &[]));
        let first = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
        assert_eq!(first.code, Some(0), "{}", first.stderr);
        let before = scenario.claude_calls().len();
        if !private {
            let record = scenario.gh_state()["advisories"][0]["html_url"]
                .as_str()
                .unwrap()
                .to_string();
            scenario.agent_does_in_session(3, &format!(r#"
printf 'Proposed public issue:\nTitle: Bound input\nBody:\nAdd a bound. Private record: {record}\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#));
        }
        scenario.agent_does_for(
            8,
            r#"
echo bounded > bounded.txt
git add bounded.txt
git commit -q -m 'Bound input'
gh pr create --base main --head issue-8 --title 'Bound input' --body 'Closes #8'
"#,
        );
        let next = scenario.run(&["secure", "base", "main", "security-fix"]);
        assert!(
            scenario.claude_calls().len() > before,
            "Security run must take the reproduced review finding when fixing is allowed; private={private}, stderr={}",
            next.stderr
        );
        assert_eq!(next.code, Some(0), "{private}: {}", next.stderr);
        let state = scenario.gh_state();
        assert_eq!(state["prs"][1]["head"], "issue-8");
        assert_eq!(state["prs"][1]["isDraft"], false);
        assert_eq!(
            scenario.origin_file("issue-8", "bounded.txt").as_deref(),
            Some("bounded\n")
        );
        assert!(
            !state["prs"][1]["body"]
                .as_str()
                .unwrap()
                .contains("Private old vulnerability evidence.")
        );
    }
}

#[test]
fn switched_checkout_is_rejected_before_private_recording() {
    for before_review in [true, false] {
        let scenario = Scenario::new();
        let switch = "\ngit switch -c replacement-branch\n";
        scenario.agent_does_in_session(
            1,
            &format!("{OPENS_PR}{}", if before_review { switch } else { "" }),
        );
        scenario.agent_does_in_session(
            2,
            &format!(
                "{}{}",
                review_script(&json!([old_finding()]), &[]),
                if before_review { "" } else { switch }
            ),
        );
        let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let records = scenario.gh_state()["advisories"].clone();
        assert!(
            records.is_null() || records.as_array().is_some_and(|records| records.is_empty()),
            "A changed checkout must be rejected before private recording: {records}; {}",
            result.stderr
        );
        assert_eq!(
            scenario.claude_calls().len(),
            if before_review { 1 } else { 2 },
            "A checkout replaced by the opening session must not start a Security review"
        );
    }
}

#[test]
fn review_merge_base_uses_remote_ref_when_local_tag_shadows_it() {
    let scenario = Scenario::new();
    scenario.launch_git(&["tag", "origin/main"]);
    scenario.origin_has_commit("main", "base-new.txt", "base change", "Advance Base branch");
    let published_base = scenario.origin_git(&["rev-parse", "refs/heads/main"]);
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(2, &review_script(&json!([]), &[]));
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "no-merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.claude_calls();
    let prompt = calls[1]["prompt"].as_str().unwrap();
    let expected = format!("Merge base commit: `{}`.", published_base.trim());
    assert!(
        prompt.contains(&expected),
        "Security review used a local tag instead of published Base branch merge base; expected {expected}, got {prompt}"
    );
}

#[test]
fn guidance_security_review_on_codex_does_not_request_delegated_auditors() {
    for resumed in [false, true] {
        let scenario = Scenario::new();
        scenario.agent_does_in_session(1, OPENS_PR);
        let mut review = review_script(&json!([]), &[]);
        if resumed {
            review.push_str(
                r#"printf '%s\n' '{"type":"item.started","item":{"id":"pending","type":"command_execution","command":"bounded-fixture","status":"in_progress"}}'
"#,
            );
            scenario.agent_does_in_session(
                3,
                r#"printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
            );
        }
        scenario.agent_does_in_session(2, &review);
        let result = scenario.run(&[
            &scenario.issue_url(7),
            "security-review",
            "harness",
            "codex",
        ]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let calls = scenario.codex_calls();
        assert_eq!(calls.len(), if resumed { 3 } else { 2 });
        assert!(
            calls[1]["prompt"]
                .as_str()
                .unwrap()
                .contains("Use one session. Do not delegate auditors")
        );
        for call in &calls[1..] {
            let prompt = call["prompt"].as_str().unwrap();
            assert!(
                !prompt.contains("Start fresh sub-agents"),
                "The chosen guidance review and its Resume must not request delegated auditors: {prompt}"
            );
            let args = call["argv"].as_array().unwrap();
            assert!(args.windows(2).any(|pair| pair
                == [
                    json!("-c"),
                    json!("agents.max_concurrent_threads_per_session=8")
                ]));
        }
    }
}

#[test]
fn reproduced_old_review_finding_sets_the_private_advisory_severity() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, OPENS_PR);
    scenario.agent_does_in_session(2, &review_script(&json!([old_finding()]), &[]));
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "MERGED");
    assert_eq!(state["advisories"][0]["state"], "draft");
    assert_eq!(
        state["advisories"][0]["severity"], "low",
        "The validated old finding already has a failing proof of concept and scored severity; record it as a Security run does"
    );
}

#[test]
fn an_untrusted_edit_before_the_security_review_fails_the_run() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!("{OPENS_PR}gh fake edits 7 '[{{\"kind\": \"body\", \"by\": \"outsider\", \"at\": \"2999-01-01T00:00:00Z\"}}]'\n"),
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "no-merge"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("#7 body edited since the Run started by an account that is not a Trusted account; starting no further session"),
        "{}", result.stderr
    );
    assert!(
        !result.stderr.contains("Security review incomplete"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

/// A User config whose sessions run on Codex but whose Security reviews run
/// on Claude Code, with its own Model and Effort.
const CODEX_WITH_A_CLAUDE_SECURITY_HARNESS: &str = r#"
[harness]
default = "codex"

[harness.claude]
model = "claude-opus-5-5"
effort = "high"

[security]
harness = "claude"
"#;

/// The script for every session of issue `issue`, whichever Harness runs
/// it: the Security review's, which its prompt names, reports nothing;
/// any other runs `other`.
fn reviewed_on_any_harness(other: &str) -> String {
    format!(
        r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
{review}
else
{other}
fi
"#,
        review = review_script(&json!([]), &[])
    )
}

/// The arguments of `call`.
fn argv(call: &serde_json::Value) -> Vec<String> {
    call["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_string())
        .collect()
}

/// The prompts of every Claude Code agent session, leaving out any test
/// call checking its Model.
fn claude_session_prompts(scenario: &Scenario) -> Vec<String> {
    scenario
        .claude_calls()
        .iter()
        .filter(|call| argv(call).iter().any(|arg| arg == "--output-format"))
        .map(|call| call["prompt"].as_str().unwrap().to_string())
        .collect()
}

/// Every `.models.json` under the scenario's Command logs, by file name.
fn session_models(scenario: &Scenario) -> Vec<(String, serde_json::Value)> {
    let dir = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    let mut models: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(".models.json"))
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let record = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            (name, record)
        })
        .collect();
    models.sort_by(|a, b| a.0.cmp(&b.0));
    models
}

#[test]
fn a_runs_security_review_runs_on_the_security_harness_and_its_other_sessions_on_the_commands() {
    let scenario = Scenario::new();
    scenario.user_config_is(CODEX_WITH_A_CLAUDE_SECURITY_HARNESS);
    // The implement session leaves CI red, and the Repair, which comes
    // after the Security review, turns it green.
    scenario.agent_does_for(
        7,
        &reviewed_on_any_harness(&format!(
            r#"
if [ -f feature.txt ]; then
echo fixed > ci-fix.txt
git add ci-fix.txt
git commit -q -m 'Fix CI'
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"success"}}]'
else
{OPENS_PR}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure"}}]'
fi
"#
        )),
    );
    let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
    let result = scenario.run_with_env(
        &[
            &scenario.issue_url(7),
            "security-review",
            "email",
            "you@example.com",
        ],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test"),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);

    let codex = scenario.codex_calls();
    assert_eq!(codex.len(), 2, "{codex:?}");
    for call in &codex {
        assert!(!call["prompt"].as_str().unwrap().contains("guidance mode"));
    }
    assert!(
        codex[1]["prompt"].as_str().unwrap().contains("CI"),
        "{}",
        codex[1]["prompt"]
    );
    let claude: Vec<_> = scenario
        .claude_calls()
        .iter()
        .map(argv)
        .filter(|args| args.iter().any(|arg| arg == "--output-format"))
        .collect();
    assert_eq!(claude.len(), 1, "{claude:?}");
    let args = &claude[0];
    let at = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .map(|at| &args[at + 1])
    };
    assert_eq!(at("--model").map(String::as_str), Some("claude-opus-5-5"));
    assert_eq!(at("--effort").map(String::as_str), Some("high"));
    assert!(claude_session_prompts(&scenario)[0].contains("guidance mode"));

    let models = session_models(&scenario);
    let harness_of = |kind: &str| {
        models
            .iter()
            .find(|(name, _)| name.ends_with(&format!("-{kind}.models.json")))
            .map(|(_, record)| record["harness"].as_str().unwrap().to_string())
    };
    assert_eq!(
        harness_of("implement").as_deref(),
        Some("codex"),
        "{models:?}"
    );
    assert_eq!(
        harness_of("security-review").as_deref(),
        Some("claude"),
        "{models:?}"
    );
    assert!(
        result.stderr.contains(
            "security-review: on claude · claude-opus-5-5 · high, chosen by security.harness"
        ),
        "{}",
        result.stderr
    );
    // The Run notification's session models name each session's Harness.
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let text = requests[0].body["text"].as_str().unwrap().to_string();
    let sessions: Vec<_> = text
        .lines()
        .skip_while(|line| !line.starts_with("Session models:"))
        .skip(1)
        .skip_while(|line| !line.starts_with("- "))
        .take_while(|line| line.starts_with("- "))
        .collect();
    assert_eq!(sessions.len(), 3, "{text}");
    for line in &sessions {
        let on = if line.starts_with("- security-review:") {
            "claude"
        } else {
            "codex"
        };
        assert!(line.contains(&format!(": {on} · ")), "{text}");
    }
    assert!(
        sessions
            .iter()
            .any(|line| line.starts_with("- security-review: claude · claude-opus-5-5")),
        "{text}"
    );
    // The line that says what the Command built with keeps the Command's.
    let body = scenario.gh_state()["prs"][0]["body"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(body.contains("Built with codex"), "{body}");
}

#[test]
fn a_commands_harness_word_does_not_choose_the_security_reviews_harness() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        r#"
[harness.claude]
model = "claude-opus-5-5"

[security]
harness = "claude"
"#,
    );
    scenario.agent_does_for(7, &reviewed_on_any_harness(OPENS_PR));
    let result = scenario.run(&[
        &scenario.issue_url(7),
        "security-review",
        "harness",
        "codex",
        "effort",
        "max",
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let codex = scenario.codex_calls();
    assert_eq!(codex.len(), 1, "{codex:?}");
    let prompts = claude_session_prompts(&scenario);
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert!(prompts[0].contains("guidance mode"));
    let models = session_models(&scenario);
    let review = models
        .iter()
        .find(|(name, _)| name.ends_with("-security-review.models.json"))
        .map(|(_, record)| record.clone())
        .unwrap();
    // Its Model and Effort are [harness.claude]'s, not the command's words.
    assert_eq!(review["harness"], "claude");
    assert_eq!(review["requested"], "claude-opus-5-5");
    assert_eq!(review["effort"], serde_json::Value::Null);
}

#[test]
fn a_base_fix_runs_its_security_review_on_the_security_harness() {
    let scenario = Scenario::new();
    scenario.user_config_is(CODEX_WITH_A_CLAUDE_SECURITY_HARNESS);
    scenario.agent_does_for(
        7,
        &reviewed_on_any_harness(&format!(
            r#"{OPENS_PR}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure"}}]'
gh fake checks "$(git rev-parse origin/main)" '[{{"name":"test","conclusion":"failure"}}]'
"#
        )),
    );
    scenario.agent_does_for(
        8,
        &reviewed_on_any_harness(
            r#"
echo fixed > ci-fix.txt
git add ci-fix.txt
git commit -q -m 'Fix base CI'
gh pr create --base main --head issue-8 --title 'Fix base CI' --body 'Closes #8'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
        ),
    );
    let result = scenario.run(&[&scenario.issue_url(7), "security-review", "base-fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let codex: Vec<_> = scenario
        .codex_calls()
        .iter()
        .map(|call| call["prompt"].as_str().unwrap().to_string())
        .collect();
    assert!(codex.iter().all(|prompt| !prompt.contains("guidance mode")));
    let reviews = claude_session_prompts(&scenario);
    assert_eq!(reviews.len(), 2, "{reviews:?}");
    assert!(
        reviews[1].contains("branch issue-8 against Base branch main"),
        "{}",
        reviews[1]
    );
    let claude: Vec<_> = scenario
        .claude_calls()
        .iter()
        .map(argv)
        .filter(|args| args.iter().any(|arg| arg == "--output-format"))
        .collect();
    for args in &claude {
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--model", "claude-opus-5-5"]),
            "{args:?}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["--effort", "high"]),
            "{args:?}"
        );
    }
    assert_eq!(scenario.gh_state()["prs"][1]["state"], "MERGED");
}

#[test]
fn a_security_harness_that_cannot_run_fails_the_run_before_any_work() {
    let scenario = Scenario::new();
    scenario.user_config_is(CODEX_WITH_A_CLAUDE_SECURITY_HARNESS);
    scenario.agent_does_for(7, OPENS_PR);
    // Claude refuses the test call on its Model.
    scenario.agent_does("echo \"There's an issue with the selected model.\"\nexit 1\n");
    let result = scenario.run(&[&scenario.issue_url(7), "security-review"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("the Security review's Harness claude, chosen by security.harness in the User config, can't run"),
        "{}",
        result.stderr
    );
    assert!(scenario.codex_calls().is_empty());
    assert!(claude_session_prompts(&scenario).is_empty());
    assert!(scenario.issue_labels(7).is_empty(), "the Claim was made");

    // With no Security review asked, the security Harness is not checked.
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
}

#[test]
fn a_continuation_from_the_recovery_command_runs_its_security_review_on_the_security_harness_again()
{
    let scenario = Scenario::new();
    scenario.user_config_is(CODEX_WITH_A_CLAUDE_SECURITY_HARNESS);
    // Each Harness counts its own sessions: the first on each is the first
    // Run's, the second the Continuation's.
    scenario.agent_does_for_in_session(
        7,
        1,
        &format!(
            r#"
if printf '%s' "$FAKE_CLAUDE_PROMPT" | grep -q 'guidance mode'; then
{review}
else
{OPENS_PR}
fi
"#,
            review = review_script(&json!([]), &["Cross-tenant read"])
        ),
    );
    scenario.agent_does_for_in_session(7, 2, &reviewed_on_any_harness("true"));
    let first = scenario.run(&[&scenario.issue_url(7), "security-review", "merge"]);
    assert_eq!(first.code, Some(1), "{}", first.stderr);
    let recovery = first
        .stderr
        .lines()
        .find_map(|line| line.strip_prefix("thirdshift: Recovery: thirdshift "))
        .unwrap_or_else(|| panic!("no Recovery command: {}", first.stderr))
        .to_string();
    // It names the Command's Harness, which must not move the review.
    assert!(recovery.ends_with(" harness codex"), "{recovery}");
    let words: Vec<&str> = recovery.split_whitespace().collect();

    let second = scenario.run(&words);

    assert_eq!(second.code, Some(0), "{recovery}: {}", second.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    let codex = scenario.codex_calls();
    assert_eq!(codex.len(), 2, "{codex:?}");
    for call in &codex {
        assert!(!call["prompt"].as_str().unwrap().contains("guidance mode"));
    }
    let reviews = claude_session_prompts(&scenario);
    assert_eq!(reviews.len(), 2, "{reviews:?}");
    assert!(
        reviews
            .iter()
            .all(|prompt| prompt.contains("guidance mode"))
    );
}
