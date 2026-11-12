//! Base branch maintenance through the public Fix Pass.
mod support;

use serde_json::json;
use support::Scenario;

const CI_RESERVATION: &str = "refs/heads/thirdshift/pickup/ci-red";
const PASS: &[&str] = &["fix", "base", "main", "no-merge", "no-base-fix", "no-email"];

fn conflicting_pr(scenario: &Scenario) -> String {
    scenario.origin_has_branch("conflict", "main", &["Feature"]);
    scenario.origin_has_commit("conflict", "shared.txt", "feature", "Feature content");
    scenario.origin_has_commit("main", "shared.txt", "base", "Base content");
    let url = scenario.github_has_pr("conflict", "main", "OPEN");
    let mut state = scenario.gh_state();
    state["prs"][0]["mergeable"] = "CONFLICTING".into();
    scenario.write_gh_state(&state);
    scenario.agent_does(
        r#"
test -n "$(git ls-remote origin refs/thirdshift/reservations/1)"
case "$(gh api --paginate 'repos/acme/widgets/issues/1/labels?per_page=100')" in
  *in-progress*) ;;
  *) exit 1 ;;
esac
echo resolved > shared.txt
git add shared.txt
git commit -qm 'Resolve base conflict'
gh fake pr conflict mergeable '"MERGEABLE"'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );
    url
}

fn assert_repaired(scenario: &Scenario, result: &support::RunResult, url: &str) {
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1, "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains(&format!("PR {url} is ready for review")),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario.origin_file("conflict", "shared.txt").as_deref(),
        Some("resolved\n")
    );
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["isDraft"], false, "{state}");
    assert!(
        state["prs"][0]["labels"].as_array().unwrap().is_empty(),
        "{state}"
    );
    assert!(
        scenario
            .launch_git(&["ls-remote", "origin", "refs/thirdshift/reservations/*"])
            .is_empty()
    );
}

fn assert_maintenance_skipped(scenario: &Scenario, result: &support::RunResult, reason: &str) {
    let message = format!("Base branch CI step skipped: {reason}");
    assert!(result.stderr.contains(&message), "{}", result.stderr);
    let activity =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(activity.contains(&message), "{activity}");
    assert!(
        !activity.contains(&format!("Fix run skipped: {reason}")),
        "{activity}"
    );
    assert!(
        !result
            .stderr
            .contains(&format!("Fix run skipped: {reason}")),
        "{}",
        result.stderr
    );
}

fn assert_ci_reservation_released(scenario: &Scenario) {
    assert!(
        scenario
            .launch_git(&["ls-remote", "origin", CI_RESERVATION])
            .is_empty()
    );
}

#[test]
fn held_ci_reservation_still_repairs_a_conflicting_pr() {
    let scenario = Scenario::new();
    let url = conflicting_pr(&scenario);
    let owner = scenario.origin_git(&["rev-parse", "main"]);
    scenario.origin_git(&["update-ref", CI_RESERVATION, owner.trim()]);
    let result = scenario.run(PASS);
    assert_repaired(&scenario, &result, &url);
    assert_maintenance_skipped(&scenario, &result, "Base branch CI is reserved on origin");
    assert_eq!(scenario.origin_git(&["rev-parse", CI_RESERVATION]), owner);
    assert!(
        scenario
            .gh_calls_of(
                "api",
                "repos/acme/widgets/actions/runs?branch=main&event=push&per_page=1"
            )
            .is_empty()
    );
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
}

#[test]
fn held_ci_reservation_records_maintenance_skip_even_without_an_eligible_pr() {
    let scenario = Scenario::new();
    let owner = scenario.origin_git(&["rev-parse", "main"]);
    scenario.origin_git(&["update-ref", CI_RESERVATION, owner.trim()]);
    for _ in 0..2 {
        let result = scenario.run(PASS);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            result.stderr.contains("no eligible pull request"),
            "{}",
            result.stderr
        );
        assert_maintenance_skipped(&scenario, &result, "Base branch CI is reserved on origin");
    }
    let activity =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert_eq!(activity.lines().count(), 2, "{activity}");
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(scenario.origin_git(&["rev-parse", CI_RESERVATION]), owner);
}

#[test]
fn pending_base_ci_still_repairs_a_conflicting_pr() {
    for workflow in [false, true] {
        let scenario = Scenario::new();
        let url = conflicting_pr(&scenario);
        let tip = scenario.origin_git(&["rev-parse", "main"]);
        let mut state = scenario.gh_state();
        if workflow {
            state["runs"] = json!([{
                "id":900,"head_branch":"main","event":"push","head_sha":tip.trim(),
                "html_url":"https://github.com/acme/widgets/actions/runs/900",
                "status":"in_progress","run_attempt":2,"conclusion":null,
                "updated_at":"2020-01-01T00:00:00Z"
            }]);
        } else {
            state["checks"][tip.trim()] = json!([{
                "name":"external","conclusion":"success","pending_polls":1000
            }]);
        }
        scenario.write_gh_state(&state);
        let result = scenario.run(PASS);
        assert_repaired(&scenario, &result, &url);
        assert_maintenance_skipped(&scenario, &result, "CI on main is pending or re-running");
        assert_ci_reservation_released(&scenario);
        assert!(scenario.gh_calls_of("run", "rerun").is_empty());
        assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
    }
}

fn failed_base(scenario: &Scenario, attempt: u64) -> String {
    let head = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut state = scenario.gh_state();
    state["checks"][&head] = json!([{
        "id":901, "name":"test", "conclusion":"failure", "url":"https://github.com/acme/widgets/actions/runs/900/job/901"
    }]);
    state["runs"] = json!([{
        "id":900, "head_branch":"main", "event":"push", "head_sha":head,
        "html_url":"https://github.com/acme/widgets/actions/runs/900",
        "status":"completed", "conclusion":"failure", "updated_at":"2020-01-01T00:00:00Z",
        "run_attempt":attempt, "jobs":[{"id":901,"name":"test","conclusion":"failure"}]
    }]);
    scenario.write_gh_state(&state);
    head
}

#[test]
fn first_failed_base_attempt_is_rerun_and_the_pass_repairs_a_conflicting_pr() {
    let scenario = Scenario::new();
    let url = conflicting_pr(&scenario);
    failed_base(&scenario, 1);
    let result = scenario.run(PASS);
    assert_repaired(&scenario, &result, &url);
    assert_eq!(
        scenario.gh_calls_of("run", "rerun").len(),
        1,
        "{}",
        result.stderr
    );
    assert!(result.stderr.contains("re-running"), "{}", result.stderr);
    assert_maintenance_skipped(&scenario, &result, "CI on main is pending or re-running");
    assert_ci_reservation_released(&scenario);
    assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
    let again = scenario.run(PASS);
    assert_eq!(again.code, Some(0), "{}", again.stderr);
    assert_eq!(scenario.gh_calls_of("run", "rerun").len(), 1);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_maintenance_skipped(&scenario, &again, "CI on main is pending or re-running");
}

const FIX: &str = r#"
echo fixed > ci-fix.txt
git add ci-fix.txt
git commit -qm 'Fix base CI'
"#;

#[test]
fn unchanged_base_ci_fix_publishes_nothing_and_is_not_repeated() {
    let scenario = Scenario::new();
    let head = failed_base(&scenario, 2);
    scenario
        .agent_does("mkdir -p .validation-logs\nprintf 'test output' > .validation-logs/check.log");
    let result = scenario.run(PASS);
    assert!(
        scenario.gh_calls_of("pr", "create").is_empty(),
        "A CI-fix session that committed nothing tried to open a PR: {}",
        result.stderr
    );
    let branch = format!("fix-main-{}", &head[..7]);
    assert!(
        scenario
            .launch_git(&["ls-remote", "origin", &format!("refs/heads/{branch}")])
            .is_empty(),
        "An unchanged repair branch was pushed"
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let reason = format!("CI-fix found nothing to change on main at {head}");
    assert!(result.stderr.contains(&reason), "{}", result.stderr);
    let activity =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(activity.contains(&reason), "{activity}");
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    assert_ci_reservation_released(&scenario);
    scenario.assert_cleaned_up(&branch);

    let again = scenario.run(PASS);
    assert_eq!(again.code, Some(0), "{}", again.stderr);
    assert_eq!(scenario.claude_calls().len(), 1, "{}", again.stderr);
    assert!(scenario.gh_calls_of("pr", "create").is_empty());
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    assert_ci_reservation_released(&scenario);
}

#[test]
fn unchanged_base_attempt_does_not_rerun_after_refusal() {
    let scenario = Scenario::new();
    let head = failed_base(&scenario, 1);
    scenario.gh_fails("run rerun");
    scenario.agent_does("true");
    let first = scenario.run(PASS);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    assert!(
        first
            .stderr
            .contains(&format!("CI-fix found nothing to change on main at {head}")),
        "{}",
        first.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(scenario.gh_calls_of("run", "rerun").len(), 1);
    assert!(scenario.gh_calls_of("pr", "create").is_empty());

    let mut state = scenario.gh_state();
    state["failing"] = json!([]);
    scenario.write_gh_state(&state);
    let second = scenario.run(PASS);
    assert_eq!(second.code, Some(0), "{}", second.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(
        scenario.gh_calls_of("run", "rerun").len(),
        1,
        "A later Pass re-ran checks after the unchanged repair: {}",
        second.stderr
    );
}

#[test]
fn unchanged_base_attempt_is_scoped_to_the_base_and_full_commit() {
    let scenario = Scenario::new();
    let head = failed_base(&scenario, 2);
    scenario.origin_has_branch("release", "main", &[]);
    scenario.agent_does("true");
    let first = scenario.run(PASS);
    assert_eq!(first.code, Some(0), "{}", first.stderr);

    let mut state = scenario.gh_state();
    state["runs"][0]["head_branch"] = "release".into();
    scenario.write_gh_state(&state);
    let release = scenario.run(&["fix", "base", "release", "no-merge", "no-email"]);
    assert_eq!(release.code, Some(0), "{}", release.stderr);
    assert_eq!(scenario.claude_calls().len(), 2, "{}", release.stderr);
    assert!(
        release.stderr.contains(&format!(
            "CI-fix found nothing to change on release at {head}"
        )),
        "{}",
        release.stderr
    );

    scenario.origin_has_commit("main", "next.txt", "next", "New Base branch commit");
    let next = failed_base(&scenario, 2);
    assert_ne!(next, head);
    scenario.agent_does(FIX);
    let changed = scenario.run(PASS);
    assert_eq!(changed.code, Some(0), "{}", changed.stderr);
    assert_eq!(scenario.claude_calls().len(), 3, "{}", changed.stderr);
    assert_eq!(
        scenario.gh_state()["prs"][0]["head"],
        format!("fix-main-{}", &next[..7])
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    assert_ci_reservation_released(&scenario);
}

#[test]
fn base_ci_notification_keeps_the_authored_pr_title() {
    let scenario = Scenario::new();
    failed_base(&scenario, 2);
    scenario.agent_does(FIX);
    let resend = support::resend::ResendStandIn::replying(200, r#"{"id":"accepted"}"#);
    let result = scenario.run_with_env(
        &["fix", "no-merge", "email", "me@example.com"],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_titles"),
        ],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request = &requests[0];
    assert_eq!(
        request.body["subject"],
        "[thirdshift] acme/widgets Fix run #1 Fix CI on main: test: ready for review"
    );
    let text = request.body["text"].as_str().unwrap();
    assert!(text.contains("Pull request: https://github.com/acme/widgets/pull/1\nTitle:        Fix CI on main: test\n"), "{text}");
    support::resend::assert_saved_notification(&scenario, "fix", request);
}

#[test]
fn persistent_base_failure_opens_a_ready_pr_at_the_origin_tip() {
    let scenario = Scenario::new();
    let head = failed_base(&scenario, 2);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    let prs = state["prs"].as_array().unwrap();
    assert_eq!(prs.len(), 1, "{}", result.stderr);
    let branch = format!("fix-main-{}", &head[..7]);
    assert_eq!(prs[0]["head"], branch);
    assert_eq!(prs[0]["base"], "main");
    assert_eq!(prs[0]["title"], "Fix CI on main: test");
    assert_eq!(prs[0]["state"], "OPEN");
    assert_eq!(prs[0]["isDraft"], false);
    assert_eq!(
        scenario
            .origin_git(&["rev-parse", &format!("{branch}~1")])
            .trim(),
        head
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1);
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("https://github.com/acme/widgets/actions/runs/900/job/901"),
        "{prompt}"
    );
    assert!(prompt.contains("every red check"), "{prompt}");
    let (_, notification) = support::resend::saved_notification(&scenario, "fix");
    assert!(notification.contains("Fix run"), "{notification}");
    let again = scenario.run(&["fix", "no-email"]);
    assert_eq!(again.code, Some(0), "{}", again.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
}

#[test]
fn delivery_repairs_checks_also_red_on_base_and_self_merges_with_issue_cleanup() {
    let scenario = Scenario::new();
    failed_base(&scenario, 2);
    let mut state = scenario.gh_state();
    state["titles"]["7"] = "CI red on main".into();
    state["issues"]["8"] = "OPEN".into();
    state["titles"]["8"] = "CI red on main: test".into();
    state["labels"]["8"] = json!(["base-fix"]);
    state["issues"]["9"] = "OPEN".into();
    state["titles"]["9"] = "CI red on mainline".into();
    scenario.write_gh_state(&state);
    scenario.user_config_is("[merge]\nalways = true\n");
    scenario.agent_does_in_session(1, &format!("{FIX}\ngh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\":\"test\",\"conclusion\":\"failure\",\"url\":\"https://ci.example/test\"}}]'"));
    scenario.agent_does_in_session(2, "echo repaired >> ci-fix.txt\ngit add ci-fix.txt\ngit commit -qm 'Repair inherited-named check'");
    let result = scenario.run(&["fix", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], "MERGED", "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2, "{}", result.stderr);
    assert_eq!(state["issues"]["7"], "CLOSED", "{}", result.stderr);
    assert_eq!(state["issues"]["8"], "CLOSED", "{}", result.stderr);
    assert_eq!(state["issues"]["9"], "OPEN");
    assert!(
        scenario.claude_calls()[1]["prompt"]
            .as_str()
            .unwrap()
            .contains("every red check")
    );
}

#[test]
fn existing_repair_prs_in_all_states_allow_other_pr_repairs() {
    for (status, draft, labels) in [
        ("OPEN", false, json!([])),
        ("OPEN", true, json!([])),
        ("OPEN", false, json!(["in-progress"])),
        ("CLOSED", false, json!([])),
        ("MERGED", false, json!([])),
    ] {
        let scenario = Scenario::new();
        let url = conflicting_pr(&scenario);
        let head = failed_base(&scenario, 2);
        let branch = format!("fix-main-{}", &head[..7]);
        scenario.origin_has_branch(&branch, "main", &["Attempted CI fix"]);
        scenario.github_has_pr(&branch, "main", status);
        let mut state = scenario.gh_state();
        state["prs"][1]["isDraft"] = draft.into();
        state["prs"][1]["labels"] = labels;
        scenario.write_gh_state(&state);
        let result = scenario.run(PASS);
        assert_repaired(&scenario, &result, &url);
        assert_maintenance_skipped(
            &scenario,
            &result,
            &format!("a repair branch or pull request from {branch} already exists"),
        );
        assert_ci_reservation_released(&scenario);
        assert!(scenario.gh_calls_of("run", "rerun").is_empty());
        assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 2);
    }
}

#[test]
fn existing_repair_branch_without_a_pr_still_allows_a_conflicting_pr() {
    let scenario = Scenario::new();
    let url = conflicting_pr(&scenario);
    let head = failed_base(&scenario, 2);
    let branch = format!("fix-main-{}", &head[..7]);
    scenario.origin_has_branch(&branch, "main", &["Attempted CI fix"]);
    let owner = scenario.origin_git(&["rev-parse", &branch]);
    let result = scenario.run(PASS);
    assert_repaired(&scenario, &result, &url);
    assert_maintenance_skipped(
        &scenario,
        &result,
        &format!("a repair branch or pull request from {branch} already exists"),
    );
    assert_ci_reservation_released(&scenario);
    assert_eq!(scenario.origin_git(&["rev-parse", &branch]), owner);
    assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
}

#[test]
fn author_scoped_pass_leaves_base_ci_alone() {
    let scenario = Scenario::new();
    let url = conflicting_pr(&scenario);
    failed_base(&scenario, 1);
    let mut state = scenario.gh_state();
    state["prs"][0]["author"] = "acme".into();
    scenario.write_gh_state(&state);
    let owner = scenario.origin_git(&["rev-parse", "main"]);
    scenario.origin_git(&["update-ref", CI_RESERVATION, owner.trim()]);
    let mut args = PASS.to_vec();
    args.extend(["--author", "acme"]);
    let result = scenario.run(&args);
    assert_repaired(&scenario, &result, &url);
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    assert!(
        scenario
            .gh_calls_of(
                "api",
                "repos/acme/widgets/actions/runs?branch=main&event=push&per_page=1"
            )
            .is_empty()
    );
    assert!(
        !result.stderr.contains("Base branch CI step skipped"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.origin_git(&["rev-parse", CI_RESERVATION]), owner);
}

#[test]
fn persistent_red_ci_repairs_base_before_a_conflicting_pr() {
    let scenario = Scenario::new();
    conflicting_pr(&scenario);
    let tip = failed_base(&scenario, 2);
    scenario.agent_does(FIX);
    let result = scenario.run(PASS);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1, "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"].as_array().unwrap().len(), 2, "{state}");
    assert_eq!(state["prs"][1]["head"], format!("fix-main-{}", &tip[..7]));
    assert_eq!(state["prs"][1]["isDraft"], false, "{state}");
    assert_eq!(
        scenario.origin_file("conflict", "shared.txt").as_deref(),
        Some("feature")
    );
    assert_ci_reservation_released(&scenario);
}

#[test]
fn base_ci_read_errors_still_fail_before_pr_selection() {
    let scenario = Scenario::new();
    conflicting_pr(&scenario);
    let mut state = scenario.gh_state();
    state["failing"] =
        json!(["api repos/acme/widgets/actions/runs?branch=main&event=push&per_page=1"]);
    state["failing_stderr"] = "Base CI evidence unavailable".into();
    scenario.write_gh_state(&state);
    let result = scenario.run(PASS);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("Base CI evidence unavailable"),
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("Base branch CI step skipped"),
        "{}",
        result.stderr
    );
    assert!(scenario.gh_calls_of("pr", "list").is_empty());
    assert!(scenario.claude_calls().is_empty());
    assert_ci_reservation_released(&scenario);
}

#[test]
fn non_rerunnable_tip_checks_ignore_an_old_failed_push_attempt() {
    let scenario = Scenario::new();
    failed_base(&scenario, 1);
    let mut state = scenario.gh_state();
    state["runs"][0]["head_sha"] = "old-commit".into();
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    state["checks"][tip.trim()] =
        json!([{"name":"external", "conclusion":"failure", "url":"https://ci.example/failed"}]);
    scenario.write_gh_state(&state);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    assert_eq!(
        scenario.gh_state()["prs"][0]["title"],
        "Fix CI on main: external"
    );
}

#[test]
fn flags_override_merge_and_email_config_for_base_repairs() {
    use support::resend::ResendStandIn;
    for (flags, config, merged, emails) in [
        (
            vec!["no-merge", "email", "operator@example.com"],
            "[merge]\nalways=true\n",
            false,
            1,
        ),
        (
            vec!["merge", "email-unless-merged", "operator@example.com"],
            "",
            true,
            0,
        ),
        (
            vec!["no-merge", "no-email"],
            "[merge]\nalways=true\n[email]\nalways=true\nto='operator@example.com'\n",
            false,
            0,
        ),
        (
            vec!["no-merge"],
            "[email]\nunless_merged=true\nto='operator@example.com'\n",
            false,
            1,
        ),
    ] {
        let scenario = Scenario::new();
        failed_base(&scenario, 2);
        scenario.agent_does(FIX);
        scenario.user_config_is(config);
        let resend = ResendStandIn::replying(200, r#"{"id":"sent"}"#);
        let mut args = vec!["fix"];
        args.extend(flags);
        let result = scenario.run_with_env(
            &args,
            &[
                ("THIRDSHIFT_RESEND_URL", resend.url()),
                ("RESEND_API_KEY", "re_test_123"),
            ],
        );
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            scenario.gh_state()["prs"][0]["state"],
            if merged { "MERGED" } else { "OPEN" }
        );
        let requests = resend.requests();
        assert_eq!(requests.len(), emails, "{}", result.stderr);
        if let Some(request) = requests.first() {
            assert!(
                request.body["text"]
                    .as_str()
                    .unwrap()
                    .contains("https://github.com/acme/widgets/pull/1")
            );
        }
    }
}

#[test]
fn named_base_with_no_rerunnable_jobs_repairs_its_origin_tip() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("release", "main", &["Release tip"]);
    failed_base(&scenario, 1);
    let tip = scenario.origin_git(&["rev-parse", "release"]);
    let mut state = scenario.gh_state();
    state["checks"] = json!({});
    state["runs"][0]["head_branch"] = "release".into();
    state["runs"][0]["head_sha"] = tip.trim().into();
    state["runs"][0]["jobs"] = json!([]);
    scenario.write_gh_state(&state);
    scenario.agent_waits_for_release(FIX);
    let held = scenario.run_until(&["fix", "base", "release", "no-email"], &[], "started");
    let status = scenario.run(&["status", "--local", "--json"]);
    let status: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
    let working = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["kind"] == "Fix run")
        .unwrap();
    assert_eq!(working["base_branch"], "release");
    assert!(working["pull_request"].is_null());
    std::fs::write(scenario.path("release"), "go").unwrap();
    let result = held.finish();
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["base"], "release");
    assert_eq!(state["prs"][0]["title"], "Fix CI on release: push workflow");
    assert_eq!(
        state["prs"][0]["head"],
        format!("fix-release-{}", &tip[..7])
    );
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    let status = scenario.run(&["status", "--local", "--json"]);
    let status: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
    let repaired = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["kind"] == "Fix run")
        .unwrap();
    assert_eq!(repaired["base_branch"], "release");
    assert_eq!(repaired["pull_request"], state["prs"][0]["url"]);
}

/// A commit status or external check can be posted by a principal who cannot
/// close issues. Merging a pull request into the default branch closes issues
/// named by `fixes #15` or `resolves acme/widgets#8` in the description
/// (any case, optional colon). The repair description must not carry that
/// reference through from the check name or URL.
#[test]
fn external_check_text_cannot_close_issues_from_the_repair_pr_body() {
    let scenario = Scenario::new();
    failed_base(&scenario, 1);
    let mut state = scenario.gh_state();
    state["runs"][0]["head_sha"] = "old-commit".into();
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    state["checks"][tip.trim()] = json!([
        {
            "name": "Fixes: #15\nresolves acme/widgets#8",
            "conclusion": "failure",
            "url": "https://ci.example/failed"
        },
        {
            "name": "gate",
            "conclusion": "failure",
            "url": "https://ci.example/run closes #4"
        }
    ]);
    scenario.write_gh_state(&state);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let body = scenario.gh_state()["prs"][0]["body"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        !description_closes_an_issue(&body),
        "repair PR body still has a GitHub closing reference:\n{body}"
    );
    assert!(body.contains("Fixes: \\#15"), "{body}");
    assert!(body.contains("resolves acme/widgets\\#8"), "{body}");
    assert!(body.contains("closes \\#4"), "{body}");
    assert!(body.contains("https://ci.example/failed"), "{body}");
    assert!(!body.contains("Fixes: #15"), "{body}");
    assert!(!body.contains("resolves acme/widgets#8"), "{body}");
    assert!(!body.contains("closes #4"), "{body}");
}

/// The same check text lands in the pull request title. `gh pr merge --merge`
/// sends no subject or body, and every documented merge-commit message format
/// includes the title. A keyword in that commit closes issues when the commit
/// reaches the default branch, so the title has to break the reference too.
#[test]
fn external_check_text_cannot_close_issues_from_the_repair_pr_title() {
    let scenario = Scenario::new();
    failed_base(&scenario, 1);
    let mut state = scenario.gh_state();
    state["runs"][0]["head_sha"] = "old-commit".into();
    let tip = scenario.origin_git(&["rev-parse", "main"]);
    state["checks"][tip.trim()] = json!([
        {
            "name": "Fixes: #15\nresolves acme/widgets#8",
            "conclusion": "failure",
            "url": "https://ci.example/failed"
        },
        {
            "name": "gate",
            "conclusion": "failure",
            "url": "https://ci.example/run closes #4"
        }
    ]);
    scenario.write_gh_state(&state);
    scenario.agent_does(FIX);
    let result = scenario.run(&["fix", "no-merge", "no-email"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let title = scenario.gh_state()["prs"][0]["title"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        !description_closes_an_issue(&title),
        "repair PR title still has a GitHub closing reference:\n{title}"
    );
    assert!(title.contains("Fixes: \\#15"), "{title}");
    assert!(title.contains("resolves acme/widgets\\#8"), "{title}");
    assert!(!title.contains('\n'), "{title}");
}

/// GitHub's closing-keyword syntax in a pull request description: a keyword
/// (`close`, `fix`, `resolve`, and their inflections), optional colon, then
/// `#n` or `owner/repo#n`. Case-insensitive. Whitespace, including a newline
/// broken out of a list item, separates the keyword from the reference.
fn description_closes_an_issue(text: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "resolves", "resolved", "resolve", "closes", "closed", "close", "fixes", "fixed", "fix",
    ];
    let mut i = 0;
    while i < text.len() {
        if closing_hash(text, i, KEYWORDS).is_some() {
            return true;
        }
        i += text[i..].chars().next().unwrap().len_utf8();
    }
    false
}

fn closing_hash(text: &str, i: usize, keywords: &[&str]) -> Option<usize> {
    if i > 0
        && text[..i]
            .chars()
            .next_back()
            .unwrap()
            .is_ascii_alphanumeric()
    {
        return None;
    }
    let tail = &text[i..];
    let keyword = keywords.iter().find(|keyword| {
        let Some(slice) = tail.get(..keyword.len()) else {
            return false;
        };
        slice.eq_ignore_ascii_case(keyword)
            && tail[keyword.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_ascii_alphanumeric())
    })?;
    let mut rest = &tail[keyword.len()..];
    let spaced = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{00a0}');
    let saw_space = spaced.len() != rest.len();
    rest = spaced;
    let mut saw_colon = false;
    if let Some(after) = rest.strip_prefix(':') {
        saw_colon = true;
        rest = after.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{00a0}');
    }
    if !saw_colon && !saw_space {
        return None;
    }
    let hash =
        if rest.starts_with('#') && rest[1..].chars().next().is_some_and(|c| c.is_ascii_digit()) {
            0
        } else {
            let owner = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'))?;
            if owner == 0 || !rest[owner..].starts_with('/') {
                return None;
            }
            let repo_src = &rest[owner + 1..];
            let repo = repo_src
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'))
                .unwrap_or(repo_src.len());
            if repo == 0 || !repo_src[repo..].starts_with('#') {
                return None;
            }
            if !repo_src[repo + 1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
            {
                return None;
            }
            owner + 1 + repo
        };
    Some(text.len() - tail.len() + (tail.len() - rest.len()) + hash)
}
