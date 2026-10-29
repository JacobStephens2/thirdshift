// Exercise the public Pickup command against GitHub.
use super::*;
use serde_json::json;

fn failed_push(scenario: &Scenario) {
    let mut state = scenario.gh_state();
    state["runs"] = json!([{
        "id": 900, "head_branch": "main", "event": "push",
        "head_sha": "badc0ffee", "html_url": "https://github.com/acme/widgets/actions/runs/900",
        "status": "completed", "conclusion": "failure", "updated_at": "2020-01-01T00:00:00Z",
        "run_attempt": 2,
        "jobs": [
            {"id": 901, "name": "Rust (Linux)", "conclusion": "failure"},
            {"id": 902, "name": "site", "conclusion": "success"}
        ]
    }]);
    let mut previous = state["runs"][0].clone();
    previous["run_attempt"] = json!(1);
    state["runs"][0]["previous_attempt"] = previous;
    state["job_logs"] = json!({"901": "2026-10-25T10:00:00Z test pickup::tests::regression ... FAILED\n2026-10-25T10:00:00Z FAIL [ 0.1s] thirdshift::pickup_run another_regression\n"});
    scenario.write_gh_state(&state);
}

#[test]
fn first_failed_attempt_is_rerun_immediately_without_filing() {
    let scenario = Scenario::new();
    failed_push(&scenario);
    let mut state = scenario.gh_state();
    state["runs"][0]["run_attempt"] = json!(1);
    state["runs"][0]["updated_at"] = json!(chrono::Utc::now().to_rfc3339());
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_calls_of("run", "rerun").len(), 1);
    assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
    assert!(result.stderr.contains("900"), "{}", result.stderr);
    assert!(result.stderr.contains("Rust (Linux)"), "{}", result.stderr);
    assert!(result.stderr.contains("re-running"), "{}", result.stderr);
    let again = scenario.run(&["pickup"]);
    assert_eq!(again.code, Some(0), "{}", again.stderr);
    assert_eq!(scenario.gh_calls_of("run", "rerun").len(), 1);
}

fn passed_rerun(scenario: &Scenario) {
    failed_push(scenario);
    let mut state = scenario.gh_state();
    state["runs"][0]["conclusion"] = json!("success");
    state["runs"][0]["updated_at"] = json!(chrono::Utc::now().to_rfc3339());
    state["runs"][0]["jobs"] = json!([
        {"id": 903, "name": "Rust (Linux)", "conclusion": "success"}
    ]);
    scenario.write_gh_state(&state);
}

#[test]
fn a_green_rerun_files_one_flaky_ticket_even_after_the_ticket_closes() {
    let scenario = Scenario::new();
    passed_rerun(&scenario);
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["titles"]["8"], "Flaky CI on main: Rust (Linux)");
    assert_eq!(
        scenario.issue_labels(8),
        ["bug", "flaky-test", READY_FOR_AGENT]
    );
    let body = state["bodies"]["8"].as_str().unwrap();
    for evidence in [
        "badc0ffee",
        "pickup::tests::regression",
        "another_regression",
        "/900/attempts/1",
        "/900/attempts/2",
        "same commit passed on re-run",
    ] {
        assert!(body.contains(evidence), "{body}");
    }
    for closed in [false, true] {
        if closed {
            scenario.issue_is(8, "CLOSED");
        }
        let again = scenario.run(&["pickup"]);
        assert_eq!(again.code, Some(0), "{}", again.stderr);
        assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 2);
    }
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
}

#[test]
fn an_open_flaky_ticket_naming_the_same_test_suppresses_a_new_report() {
    for open in [true, false] {
        let scenario = Scenario::new();
        passed_rerun(&scenario);
        let mut state = scenario.gh_state();
        state["titles"]["7"] = json!("Flaky CI on release: macOS");
        state["bodies"]["7"] =
            json!("A different commit failed pickup::tests::regression on macOS.");
        state["issues"]["7"] = json!(if open { "OPEN" } else { "CLOSED" });
        scenario.write_gh_state(&state);
        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            scenario.gh_state()["issues"].as_object().unwrap().len(),
            if open { 1 } else { 2 }
        );
    }
}

#[test]
fn pickup_files_one_ci_red_ticket_with_evidence() {
    let scenario = Scenario::new();
    failed_push(&scenario);
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(
        state["titles"]["8"], "CI red on main: Rust (Linux)",
        "{state}"
    );
    assert_eq!(scenario.issue_labels(8), ["bug", READY_FOR_AGENT]);
    let body = state["bodies"]["8"].as_str().unwrap();
    for evidence in [
        "badc0ffee",
        "Rust (Linux)",
        "pickup::tests::regression",
        "another_regression",
        "https://github.com/acme/widgets/actions/runs/900",
        "https://github.com/acme/widgets/actions/runs/900/attempts/1",
        "https://github.com/acme/widgets/actions/runs/900/attempts/2",
    ] {
        assert!(body.contains(evidence), "{body}");
    }
    assert!(!body.contains("- site"), "{body}");
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 2);
    assert!(
        scenario
            .origin_git(&["for-each-ref", "refs/heads/thirdshift/pickup"])
            .is_empty()
    );
}

#[test]
fn pickup_ignores_green_pending_and_young_latest_push_runs() {
    for (status, conclusion, updated) in [
        ("completed", "success", "2020-01-01T00:00:00Z".to_string()),
        ("in_progress", "", "2020-01-01T00:00:00Z".to_string()),
        ("queued", "", "2020-01-01T00:00:00Z".to_string()),
        ("completed", "failure", chrono::Utc::now().to_rfc3339()),
        (
            "completed",
            "failure",
            (chrono::Utc::now() - chrono::Duration::minutes(29)).to_rfc3339(),
        ),
    ] {
        let scenario = Scenario::new();
        failed_push(&scenario);
        let mut state = scenario.gh_state();
        let mut latest = state["runs"][0].clone();
        latest["id"] = json!(910);
        latest["status"] = json!(status);
        latest["conclusion"] = if conclusion.is_empty() {
            json!(null)
        } else {
            json!(conclusion)
        };
        if conclusion == "success" {
            latest["run_attempt"] = json!(1);
        }
        latest["updated_at"] = json!(updated);
        state["runs"].as_array_mut().unwrap().push(latest);
        scenario.write_gh_state(&state);
        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            scenario.gh_state()["issues"].as_object().unwrap().len(),
            1,
            "{status} {conclusion}"
        );
    }
}

#[test]
fn a_similarly_named_test_does_not_suppress_a_flaky_report() {
    let scenario = Scenario::new();
    passed_rerun(&scenario);
    let mut state = scenario.gh_state();
    state["titles"]["7"] = json!("Flaky CI on main: Rust");
    state["bodies"]["7"] =
        json!("Failures: pickup::tests::regression_extra and another_regression_extra");
    scenario.write_gh_state(&state);
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["titles"]["8"],
        "Flaky CI on main: Rust (Linux)"
    );
}

#[test]
fn a_renamed_flaky_ticket_still_covers_its_recorded_test() {
    let scenario = Scenario::new();
    passed_rerun(&scenario);
    let mut state = scenario.gh_state();
    state["titles"]["7"] = json!("Remove scheduling dependence from pickup::tests::regression");
    scenario.write_gh_state(&state);
    scenario.issue_labelled(7, &["FlAkY-TeSt"]);
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
}

#[test]
fn node_spec_flakes_keep_evidence_and_deduplicate_by_test() {
    for existing in [false, true] {
        let scenario = Scenario::new();
        passed_rerun(&scenario);
        let test = "Command details opens with the Inbox subject and full title above its actions";
        let mut state = scenario.gh_state();
        state["runs"][0]["previous_attempt"]["jobs"][0]["name"] = json!("console-test");
        state["runs"][0]["jobs"][0]["name"] = json!("console-test");
        state["job_logs"]["901"] = json!(format!("2026-10-29T10:00:00Z ✖ {test} (123.45ms)\n"));
        if existing {
            state["titles"]["7"] = json!("Flaky CI on main: console-test");
            state["bodies"]["7"] = json!(format!("An earlier commit failed {test}."));
        }
        scenario.write_gh_state(&state);

        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let state = scenario.gh_state();
        if existing {
            assert_eq!(state["issues"].as_object().unwrap().len(), 1, "{state}");
        } else {
            let body = state["bodies"]["8"].as_str().unwrap();
            assert!(body.contains(test), "{body}");
        }
    }
}

#[test]
fn node_failure_summary_does_not_make_different_tests_duplicates() {
    let scenario = Scenario::new();
    passed_rerun(&scenario);
    let mut state = scenario.gh_state();
    state["job_logs"]["901"] = json!("✖ a different test (123.45ms)\n✖ failing tests:\n");
    state["titles"]["7"] = json!("Flaky CI on main: console-test");
    state["bodies"]["7"] = json!("✖ the original test (12.34ms)\n✖ failing tests:\n");
    scenario.write_gh_state(&state);
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["titles"]["8"],
        "Flaky CI on main: Rust (Linux)"
    );
}

#[test]
fn dotted_node_test_names_do_not_cover_a_different_test() {
    let scenario = Scenario::new();
    passed_rerun(&scenario);
    let mut state = scenario.gh_state();
    state["job_logs"]["901"] = json!("✖ render (123.45ms)\n");
    state["titles"]["7"] = json!("Flaky CI on main: console-test");
    state["bodies"]["7"] = json!("✖ render.details (12.34ms)\n");
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["titles"]["8"],
        "Flaky CI on main: Rust (Linux)"
    );
}

#[test]
fn pickup_reads_origin_and_the_named_base_and_can_take_the_ticket_normally() {
    let scenario = Scenario::new();
    failed_push(&scenario);
    scenario.origin_has_branch("develop", "main", &["Develop"]);
    let mut state = scenario.gh_state();
    state["runs"][0]["head_branch"] = json!("develop");
    // Newer runs on other branches or events must not hide the Base failure.
    let mut unrelated = state["runs"][0].clone();
    unrelated["id"] = json!(999);
    unrelated["conclusion"] = json!("success");
    unrelated["event"] = json!("pull_request");
    state["runs"].as_array_mut().unwrap().push(unrelated);
    scenario.write_gh_state(&state);
    scenario.user_config_is("[pickup]\nwait_minutes = 0\n");
    scenario.agent_does_for(8, &agent_fixes_bug(8, "develop"));
    let result = scenario.run(&["pickup", "base", "develop"]);
    let pr = pr_from(&scenario, "issue-8");
    assert_ended_with_pr(&result, &pr, "ready for review");
    assert_eq!(pr["base"], "develop");
    assert_eq!(
        scenario.gh_state()["titles"]["8"],
        "CI red on develop: Rust (Linux)"
    );
    assert_eq!(scenario.issue_labels(8), ["bug", IN_PROGRESS]);
}

#[test]
fn pickup_deduplicates_across_bases_and_labels_but_closed_tickets_allow_filing() {
    for open in [true, false] {
        let scenario = Scenario::new();
        failed_push(&scenario);
        let mut state = scenario.gh_state();
        state["titles"] = json!({"7": "CI red on release: old failure"});
        state["issues"]["7"] = json!(if open { "OPEN" } else { "CLOSED" });
        // No bug/Ready label: an existing Ticket can be taken or relabelled.
        scenario.write_gh_state(&state);
        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            scenario.gh_state()["issues"].as_object().unwrap().len(),
            if open { 1 } else { 2 }
        );
    }
}

#[test]
fn a_needs_info_ci_red_ticket_does_not_block_a_new_failure() {
    let scenario = Scenario::new();
    failed_push(&scenario);
    let mut state = scenario.gh_state();
    state["titles"]["7"] = json!("CI red on main: an older flake");
    state["bodies"]["7"] = json!("Failing commit: `oldercommit`");
    scenario.write_gh_state(&state);
    scenario.issue_labelled(7, &["bug", "NeEdS-InFo"]);
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.gh_state()["titles"]["8"],
        "CI red on main: Rust (Linux)"
    );
}

#[test]
fn an_open_ci_red_ticket_gets_new_commit_evidence_only_once() {
    for different in [true, false] {
        let scenario = Scenario::new();
        failed_push(&scenario);
        let mut state = scenario.gh_state();
        state["titles"]["7"] = json!("CI red on release: old failure");
        state["bodies"]["7"] = json!(format!(
            "Failing commit: `{}`",
            if different { "oldcommit" } else { "badc0ffee" }
        ));
        scenario.write_gh_state(&state);
        for _ in 0..2 {
            let result = scenario.run(&["pickup"]);
            assert_eq!(result.code, Some(0), "{}", result.stderr);
        }
        let state = scenario.gh_state();
        assert_eq!(state["issues"].as_object().unwrap().len(), 1);
        let comments = state["comments"]["7"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(comments.len(), usize::from(different), "{state}");
        if different {
            let body = comments[0].as_str().unwrap();
            for evidence in [
                "badc0ffee",
                "/900/attempts/2",
                "pickup::tests::regression",
                "another_regression",
            ] {
                assert!(body.contains(evidence), "{body}");
            }
        }
    }
}

#[test]
fn refused_or_impossible_first_reruns_warn_and_fall_back_to_filing() {
    for no_jobs in [false, true] {
        let scenario = Scenario::new();
        failed_push(&scenario);
        let mut state = scenario.gh_state();
        state["runs"][0]["run_attempt"] = json!(1);
        state["runs"][0]["updated_at"] = json!(chrono::Utc::now().to_rfc3339());
        if no_jobs {
            state["runs"][0]["jobs"] = json!([]);
        }
        scenario.write_gh_state(&state);
        if !no_jobs {
            scenario.gh_fails("run rerun");
        }
        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let state = scenario.gh_state();
        assert!(
            state["titles"]["8"]
                .as_str()
                .unwrap()
                .starts_with("CI red on main:"),
            "{state}"
        );
        assert!(result.stderr.contains("falling back"), "{}", result.stderr);
        assert!(result.stderr.contains("warning"), "{}", result.stderr);
    }
}

#[test]
fn unreadable_ci_files_nothing_and_does_not_stop_ready_dispatch() {
    for failure in [
        "api repos/acme/widgets/actions/runs?branch=main&event=push&per_page=1",
        "api repos/acme/widgets/actions/runs/900/attempts/1",
        "api --paginate --jq .jobs[] repos/acme/widgets/actions/runs/900/attempts/2/jobs?per_page=100",
        "api repos/acme/widgets/actions/jobs/901/logs",
        "api --paginate --jq .[] repos/acme/widgets/issues?state=open&sort=created&direction=desc&per_page=100",
    ] {
        let scenario = Scenario::new();
        failed_push(&scenario);
        ready_issue(&scenario, 7, &[]);
        scenario.agent_does_for(7, &agent_opens_pr(7, "main"));
        scenario.gh_fails(failure);
        let result = scenario.run(&["pickup"]);
        assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
        assert!(
            result.stderr.contains("could not file CI-red Ticket"),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
    }
}

#[test]
fn an_occupied_ci_reservation_skips_filing_and_keeps_its_owners_ref() {
    let scenario = Scenario::new();
    failed_push(&scenario);
    scenario.origin_git(&[
        "update-ref",
        "refs/heads/thirdshift/pickup/ci-red",
        "refs/heads/main",
    ]);
    let owner = scenario.origin_git(&["rev-parse", "refs/heads/thirdshift/pickup/ci-red"]);
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));
    let result = scenario.run(&["pickup"]);
    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
    assert_eq!(
        scenario.origin_git(&["rev-parse", "refs/heads/thirdshift/pickup/ci-red"]),
        owner
    );
}

#[test]
fn an_occupied_ci_reservation_also_prevents_reruns_and_comments() {
    for first_attempt in [true, false] {
        let scenario = Scenario::new();
        failed_push(&scenario);
        let mut state = scenario.gh_state();
        state["runs"][0]["run_attempt"] = json!(if first_attempt { 1 } else { 2 });
        state["titles"]["7"] = json!("CI red on main: previous failure");
        state["bodies"]["7"] = json!("Failing commit: `oldcommit`");
        scenario.write_gh_state(&state);
        scenario.origin_git(&[
            "update-ref",
            "refs/heads/thirdshift/pickup/ci-red",
            "refs/heads/main",
        ]);
        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(scenario.gh_calls_of("run", "rerun").is_empty());
        assert!(scenario.gh_calls_of("issue", "comment").is_empty());
        assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
    }
}
