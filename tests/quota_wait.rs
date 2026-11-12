mod support;

use serde_json::json;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use support::Scenario;

const COMMIT: &str =
    "echo feature > feature.txt\ngit add feature.txt\ngit commit -qm 'Add feature'\n";
const PR: &str =
    "gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7' --draft\n";

#[test]
fn delivery_write_waits_for_primary_quota_and_notifies_once() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{COMMIT}{PR}"));
    let reset = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3;
    let mut state = scenario.gh_state();
    state["fail_next"] = json!({"call":"pr ready", "times":1,
        "error":"GraphQL: API rate limit exceeded (HTTP 403)"});
    state["rate_limits"] = json!({"graphql":{"remaining":0,"reset":reset}});
    scenario.write_gh_state(&state);

    let result = scenario.run_with_env(
        &[&scenario.issue_url(7)],
        &[("THIRDSHIFT_QUOTA_MINUTE_MS", "1")],
    );

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("Quota wait until"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.gh_calls_of("pr", "ready").len(), 2);
    let (_, notification) = support::resend::saved_notification(&scenario, "issue");
    assert_eq!(
        notification
            .matches("for the GitHub quota (graphql)")
            .count(),
        1,
        "{notification}"
    );
    assert!(
        !scenario.gh_state()["labels"]["7"]
            .as_array()
            .unwrap()
            .contains(&json!("agent-failed"))
    );
}

#[test]
fn ci_watch_waits_until_reset_then_delivers_without_a_repair() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(r#"{COMMIT}{PR}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"success","url":"https://ci.example/test"}}]'
python3 - <<'PY'
import json, os, time
path = os.environ['FAKE_GH_STATE']
with open(path) as f: state = json.load(f)
state['fail_next'] = {{'call':'api --paginate --jq .check_runs[]','times':1,'error':'API rate limit exceeded (HTTP 403)'}}
state['rate_limits'] = {{'core':{{'remaining':0,'reset':int(time.time())+3}}}}
with open(path, 'w') as f: json.dump(state, f)
PY
"#));
    let result = scenario.run_with_env(
        &[&scenario.issue_url(7)],
        &[("THIRDSHIFT_QUOTA_MINUTE_MS", "1")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stderr.contains("CI passed"), "{}", result.stderr);
    assert!(
        result.stderr.contains("Quota wait until"),
        "{}",
        result.stderr
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(
        now >= scenario.gh_state()["rate_limits"]["core"]["reset"]
            .as_u64()
            .unwrap()
    );
    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "quota wait started a Repair"
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    let (_, text) = support::resend::saved_notification(&scenario, "issue");
    assert_eq!(
        text.matches("for the GitHub quota (core)").count(),
        1,
        "{text}"
    );
    assert!(
        !scenario
            .issue_labels(7)
            .iter()
            .any(|label| label == "agent-failed")
    );
}

#[test]
fn secondary_limit_stops_after_five_retries_without_a_quota_wait() {
    for failures in [1, 6] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!("{COMMIT}{PR}"));
        let mut state = scenario.gh_state();
        state["fail_next"] = json!({"call":"pr ready", "times":failures,
            "error":"GraphQL: secondary rate limit exceeded (HTTP 429)"});
        scenario.write_gh_state(&state);
        let result = scenario.run_with_env(
            &[&scenario.issue_url(7)],
            &[("THIRDSHIFT_QUOTA_MINUTE_MS", "10")],
        );
        assert_eq!(
            result.code,
            Some(if failures == 1 { 0 } else { 1 }),
            "{}",
            result.stderr
        );
        assert_eq!(
            scenario.gh_calls_of("pr", "ready").len(),
            failures.min(5) + 1
        );
        assert!(
            !result.stderr.contains("Quota wait until"),
            "{}",
            result.stderr
        );
        let (_, text) = support::resend::saved_notification(&scenario, "issue");
        assert!(!text.contains("for the GitHub quota"), "{text}");
        assert!(!scenario.path("home/.thirdshift/github-quota.json").exists());
    }
}

#[test]
fn a_primary_wait_beyond_the_cap_returns_the_original_github_failure() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{COMMIT}{PR}"));
    let mut state = scenario.gh_state();
    state["fail_next"] = json!({"call":"pr ready", "times":1,
        "error":"GraphQL: API rate limit exceeded (HTTP 403)"});
    state["rate_limits"] = json!({"graphql":{"remaining":0,"reset":4102444800u64}});
    scenario.write_gh_state(&state);
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains(
            "gh pr ready 1 --repo acme/widgets failed: GraphQL: API rate limit exceeded (HTTP 403)"
        ),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.gh_calls_of("pr", "ready").len(), 1);
    let (_, text) = support::resend::saved_notification(&scenario, "issue");
    assert!(!text.contains("for the GitHub quota"), "{text}");
}

fn wait_for_quota_stage(scenario: &Scenario) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/issue")
            ["quota_wait_until"].is_string() { return; }
        assert!(
            Instant::now() < deadline,
            "Command never entered Quota wait"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn stop_cancels_an_ordinary_wait_but_claim_ending_waits_and_restores_its_stage() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(&format!(
        "{COMMIT}{PR}touch '{}'\n",
        scenario.path("started").display()
    ));
    let mut state = scenario.gh_state();
    state["fail_next"] = json!({"call":"pr ready", "times":1,
        "error":"GraphQL: API rate limit exceeded (HTTP 403)"});
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    state["rate_limits"] = json!({"graphql":{"remaining":0,"reset":reset}});
    scenario.write_gh_state(&state);
    let held = scenario.run_until(
        &[&scenario.issue_url(7)],
        &[("THIRDSHIFT_QUOTA_MINUTE_MS", "1")],
        "started",
    );
    wait_for_quota_stage(&scenario);
    for args in [vec!["status"], vec!["show", "widgets#7"]] {
        let shown = scenario.run(&args);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        assert!(
            shown.stdout.contains("Quota wait until"),
            "{}",
            shown.stdout
        );
    }
    let mut state = scenario.gh_state();
    state["fail_next"] = json!({"call":"api --method PUT", "times":1,
        "error":"API rate limit exceeded (HTTP 403)"});
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3;
    state["rate_limits"]["core"] = json!({"remaining":0,"reset":reset});
    scenario.write_gh_state(&state);
    let stopped_at = Instant::now();
    let stopped = scenario.run(&["stop", "widgets#7"]);
    let result = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        stopped_at.elapsed() < Duration::from_secs(30),
        "ordinary hour wait survived Stop"
    );
    assert!(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            >= reset
    );
    assert_eq!(scenario.gh_state()["fail_next"]["times"], 0);
    assert!(
        scenario
            .issue_labels(7)
            .contains(&"needs-triage".to_string())
    );
    assert!(
        !scenario
            .issue_labels(7)
            .contains(&"agent-failed".to_string())
    );
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/issue");
    assert!(record["quota_wait_until"].is_null(), "{record}");
    let (_, text) = support::resend::saved_notification(&scenario, "issue");
    assert_eq!(
        text.matches("for the GitHub quota (core, graphql)").count(),
        1,
        "{text}"
    );
}

#[test]
fn a_child_tickets_quota_wait_belongs_to_its_spec_commands_notification() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[])]);
    scenario.agent_does_for(21, &format!("{COMMIT}gh pr create --base issue-20 --head issue-21 --title Ticket --body 'Closes #21' --draft\n"));
    let mut state = scenario.gh_state();
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3;
    state["fail_next"] = json!({"call":"pr ready", "times":1,
        "error":"GraphQL: API rate limit exceeded (HTTP 403)"});
    state["rate_limits"] = json!({"graphql":{"remaining":0,"reset":reset}});
    scenario.write_gh_state(&state);
    let result = scenario.run_with_env(
        &[&scenario.issue_url(20)],
        &[("THIRDSHIFT_QUOTA_MINUTE_MS", "1")],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let (_, text) = support::resend::saved_notification(&scenario, "issue");
    assert_eq!(
        text.matches("for the GitHub quota (graphql)").count(),
        1,
        "{text}"
    );
    let sidecars = scenario
        .entries("home/.thirdshift/logs/acme/widgets/commands/issue")
        .into_iter()
        .filter(|name| name.ends_with(".quota-wait.json"))
        .collect::<Vec<_>>();
    assert_eq!(sidecars.len(), 1, "{sidecars:?}");
    assert!(sidecars[0].starts_with("20-"), "{sidecars:?}");
}

#[test]
fn preflight_quota_wait_is_visible_and_can_be_stopped() {
    use std::{fs, process::Stdio};
    let scenario = Scenario::new();
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let mut state = scenario.gh_state();
    state["fail_next"] = json!({"call":"api graphql", "times":1,
        "error":"GraphQL: API rate limit exceeded (HTTP 403)"});
    state["rate_limits"] = json!({"graphql":{"remaining":0,"reset":reset}});
    scenario.write_gh_state(&state);
    let stderr_path = scenario.path("preflight.stderr");
    let owned = support::child_lifetime::OwnedRuns::new();
    let mut command = scenario.command(&[&scenario.issue_url(7)]);
    command
        .env("THIRDSHIFT_QUOTA_MINUTE_MS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&stderr_path).unwrap());
    let mut child = owned.spawn(&mut command);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let stderr = fs::read_to_string(&stderr_path).unwrap();
        if stderr.contains("Quota wait until") {
            break;
        }
        assert!(
            child.try_wait().is_none() && Instant::now() < deadline,
            "{stderr}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    for args in [vec!["status"], vec!["show", "widgets#7"]] {
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            result.stdout.contains("Quota wait until"),
            "{}",
            result.stdout
        );
    }
    let stopped = scenario.run(&["stop", "widgets#7"]);
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert!(!child.wait().success());
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn a_pass_that_waited_before_skipping_finishes_its_command_record() {
    let scenario = Scenario::new();
    let reset = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 2;
    let mut state = scenario.gh_state();
    state["fail_next"] = json!({"call":"api --paginate --jq", "times":1,
        "error":"API rate limit exceeded (HTTP 403)"});
    // Admission sees enough quota; the Sweep's subsequent read exhausts it.
    state["next_rate_limits"] = json!({"core":{"remaining":0,"reset":reset}});
    scenario.write_gh_state(&state);
    let result = scenario.run_with_env(&["pickup"], &[("THIRDSHIFT_QUOTA_MINUTE_MS", "1")]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("Quota wait until"),
        "{}",
        result.stderr
    );
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/pickup");
    assert_eq!(record["outcome"], "skipped", "{record}");
    assert!(record["ended_at"].is_string(), "{record}");
    let shown = scenario.run(&["status"]);
    assert!(!shown.stdout.contains("Died"), "{}", shown.stdout);
}

#[test]
fn a_rate_limit_whose_probe_shows_the_reserve_retries_without_a_quota_wait() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{COMMIT}{PR}"));
    let mut state = scenario.gh_state();
    state["fail_next"] = json!({"call":"pr ready", "times":1,
        "error":"GraphQL: API rate limit exceeded (HTTP 403)"});
    // The window reset between the failed request and the probe: a full
    // quota whose next reset is far beyond any Command's wait cap.
    state["rate_limits"] = json!({"graphql":{"remaining":4995,"reset":4102444800u64}});
    scenario.write_gh_state(&state);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        !result.stderr.contains("Quota wait until"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.gh_calls_of("pr", "ready").len(), 2);
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    let path = scenario.path("home/.thirdshift/github-quota.json");
    if path.exists() {
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(
            record["waits"].as_array().is_none_or(Vec::is_empty),
            "{record}"
        );
    }
}
