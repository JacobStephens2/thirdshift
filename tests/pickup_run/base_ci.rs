// Exercise the public Pickup command against GitHub.
use super::*;
use serde_json::json;

fn failed_push(scenario: &Scenario) {
    let mut state = scenario.gh_state();
    state["runs"] = json!([{
        "id": 900, "head_branch": "main", "event": "push",
        "head_sha": "badc0ffee", "html_url": "https://github.com/acme/widgets/actions/runs/900",
        "status": "completed", "conclusion": "failure", "updated_at": "2020-01-01T00:00:00Z",
        "jobs": [
            {"id": 901, "name": "Rust (Linux)", "conclusion": "failure"},
            {"id": 902, "name": "site", "conclusion": "success"}
        ]
    }]);
    state["job_logs"] = json!({"901": "2026-10-25T10:00:00Z test pickup::tests::regression ... FAILED\n2026-10-25T10:00:00Z FAIL [ 0.1s] thirdshift::pickup_run another_regression\n"});
    scenario.write_gh_state(&state);
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
    ] {
        assert!(body.contains(evidence), "{body}");
    }
    assert!(!body.contains("- site"), "{body}");
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
fn unreadable_ci_files_nothing_and_does_not_stop_ready_dispatch() {
    for failure in [
        "api repos/acme/widgets/actions/runs?branch=main&event=push&per_page=1",
        "api --paginate --jq .jobs[] repos/acme/widgets/actions/runs/900/jobs?per_page=100",
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
