//! Exhausted Plan usage never fails an issue: when every route a Pass may
//! take is unavailable it skips without sleeping, writing one Activity log
//! line per reason and a status banner, and a live factory Command whose
//! routes are all unavailable ends as a Paused Command, releasing its Claim
//! without `agent-failed` and putting `ready-for-agent` back for Pickup.
//! Only structured Harness wire frames count, never agent-written text.

mod support;

use serde_json::{Value, json};
use support::Scenario;
use support::resend::ResendStandIn;

/// A future Unix time, so a recorded reset is still ahead.
const RESET: i64 = 4_000_000_000;

/// The same reset as Plan usage saves it.
const RESET_AT: &str = "2096-10-02T07:06:40Z";

/// Work begins: the agent commits, then says something before it stops.
const COMMITS: &str = r#"
echo kept > kept.txt
git add kept.txt
git commit -q -m "Keep this"
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"working"}]}}'
"#;

/// Claude Code's own structured refusal of included work.
fn claude_limit() -> String {
    json!({"type": "rate_limit_event", "rate_limit_info": {"status": "rejected",
        "rateLimitType": "five_hour", "resetsAt": RESET, "utilization": 1.0}})
    .to_string()
}

/// The agent commits work, then Claude Code refuses included work.
fn stops_at_the_limit() -> String {
    format!("{COMMITS}echo '{}'\nexit 1\n", claude_limit())
}

/// Claude Code first, then Codex.
const ROUTES: &str = "[harness]\nroutes = [\"claude\", \"codex\"]\n";

/// Save `harnesses` as the machine's Plan usage snapshot.
fn plan_usage_is(scenario: &Scenario, harnesses: Value) {
    let path = scenario.path("home/.thirdshift/plan-usage.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, json!({ "harnesses": harnesses }).to_string()).unwrap();
}

/// A Harness held until `RESET_AT` after a refusal in its five-hour window,
/// freshly observed so routing reads nothing more.
fn in_cooldown() -> Value {
    let now = chrono::Utc::now().to_rfc3339();
    json!({"observed_at": now, "read_at": now,
        "cooldown": {"since": now, "until": RESET_AT, "window": "five_hour"}})
}

/// Both routes held by Plan usage.
fn every_route_in_cooldown(scenario: &Scenario) {
    plan_usage_is(
        scenario,
        json!({"claude": in_cooldown(), "codex": in_cooldown()}),
    );
}

fn activity(scenario: &Scenario) -> String {
    std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
        .unwrap_or_default()
}

/// Claude Code's agent sessions, without its sign-in checks.
fn claude_sessions(scenario: &Scenario) -> usize {
    scenario
        .claude_calls()
        .into_iter()
        .filter(|call| {
            call["argv"]
                .as_array()
                .is_some_and(|argv| argv.iter().any(|arg| arg == "--output-format"))
        })
        .count()
}

fn issue_record(scenario: &Scenario) -> Value {
    scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/issue")
}

/// Where the Inbox puts the one Command on issue `number`, and its kind.
fn inbox_row(scenario: &Scenario, number: u64) -> (String, Value) {
    let output = scenario.run(&["status", "--json", "--local"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["issue"]["number"] == number)
        .unwrap_or_else(|| panic!("no row for #{number}: {status:#}"))
        .clone();
    (
        row["group"].as_str().unwrap().to_string(),
        row["needs_you_kind"].clone(),
    )
}

/// A Custom process named `name`, of one review Step.
fn process_named(scenario: &Scenario, name: &str) {
    let dir = scenario.path("home/.thirdshift/processes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.toml")),
        "version = 1\ndescription = \"Notes\"\n[[steps]]\nname = \"notes\"\nskill = \"code-review\"\n",
    )
    .unwrap();
}

#[test]
fn passes_skip_without_work_while_every_route_is_unavailable_and_log_each_reason_once() {
    let scenario = Scenario::new();
    scenario.user_config_is(ROUTES);
    every_route_in_cooldown(&scenario);
    scenario.issue_labelled(7, &["ready-for-agent"]);

    for _ in 0..3 {
        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(result.stdout.is_empty(), "{}", result.stdout);
    }

    assert_eq!(claude_sessions(&scenario), 0);
    assert!(scenario.codex_calls().is_empty());
    assert_eq!(scenario.issue_labels(7), ["ready-for-agent"]);
    // Skipped before taking a Command-limit slot or reading the issues.
    assert!(
        !scenario
            .path("home/.thirdshift/command-slots/acme/widgets/0.slot")
            .exists()
    );
    assert!(
        scenario
            .gh_calls()
            .iter()
            .all(|call| call.starts_with(&["api".into(), "rate_limit".into()])
                || call.starts_with(&["api".into(), "user".into()])),
        "{:?}",
        scenario.gh_calls()
    );
    let reason = format!(
        "Pickup run skipped: no allowed route has available Plan usage: claude in cooldown \
         (five_hour) until {RESET_AT}; codex in cooldown (five_hour) until {RESET_AT}"
    );
    let log = activity(&scenario);
    assert_eq!(
        log.lines().filter(|line| line.ends_with(&reason)).count(),
        1,
        "{log}"
    );

    // `thirdshift status` names the held routes and the earliest reset.
    let status = scenario.run(&["status", "--local"]);
    assert!(
        status.stdout.contains(&format!(
            "Plan usage: every configured route is unavailable, so scheduled Passes skip: claude \
             in cooldown (five_hour) until {RESET_AT}; codex in cooldown (five_hour) until \
             {RESET_AT}; earliest reset {RESET_AT}"
        )),
        "{}",
        status.stdout
    );

    // Once a route frees up, the Pass runs and the banner goes.
    plan_usage_is(&scenario, json!({"claude": in_cooldown()}));
    scenario.agent_does(support::AGENT_COMMITS_AND_OPENS_PR);
    let ran = scenario.run(&["pickup"]);
    assert_eq!(ran.code, Some(0), "{}", ran.stderr);
    assert_eq!(scenario.codex_calls().len(), 1, "{}", ran.stderr);
    assert!(
        !scenario
            .run(&["status", "--local"])
            .stdout
            .contains("Plan usage: every configured route")
    );
    // A later skip for the same reason is written again: it changed since.
    every_route_in_cooldown(&scenario);
    assert_eq!(scenario.run(&["pickup"]).code, Some(0));
    let log = activity(&scenario);
    assert_eq!(
        log.lines().filter(|line| line.ends_with(&reason)).count(),
        2,
        "{log}"
    );
}

#[test]
fn named_custom_processes_each_log_an_unchanged_plan_usage_skip_once() {
    let scenario = Scenario::new();
    scenario.user_config_is("[harness]\ndefault = \"claude\"\n");
    plan_usage_is(&scenario, json!({"claude": in_cooldown()}));
    for name in ["notes", "other-notes"] {
        process_named(&scenario, name);
    }

    for _ in 0..3 {
        for name in ["notes", "other-notes"] {
            let result = scenario.run(&["process", name]);
            assert_eq!(result.code, Some(0), "{name}: {}", result.stderr);
            assert!(result.stdout.is_empty(), "{name}: {}", result.stdout);
        }
    }

    assert_eq!(claude_sessions(&scenario), 0);
    let log = activity(&scenario);
    for name in ["notes", "other-notes"] {
        let line = format!(
            "Process {name} skipped: no allowed route has available Plan usage: claude in \
             cooldown (five_hour) until {RESET_AT}"
        );
        assert_eq!(
            log.lines().filter(|logged| logged.ends_with(&line)).count(),
            1,
            "{name}:\n{log}"
        );
    }
}

#[test]
fn a_run_whose_only_route_stops_at_its_limit_pauses_and_hands_its_issue_back_for_pickup() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.agent_does(&stops_at_the_limit());

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(75), "{}", result.stderr);
    assert_eq!(
        scenario.issue_labels(7),
        ["bug", "ready-for-agent"],
        "{}",
        result.stderr
    );
    assert!(scenario.issue_comments(7).is_empty());
    assert!(
        result.stderr.contains(
            "releasing the Claim on #7: labelling it ready-for-agent, in place of in-progress, \
             without agent-failed: no allowed route has available Plan usage"
        ),
        "{}",
        result.stderr
    );
    // Work already on origin stays there for the next Continuation.
    assert!(
        scenario
            .origin_log("issue-7")
            .unwrap()
            .contains(&"Earlier work".to_string())
    );
    let record = issue_record(&scenario);
    assert_eq!(record["outcome"], "paused", "{record:#}");
    assert_eq!(record["limit"]["harness"], "claude", "{record:#}");
    assert_eq!(record["limit"]["window"], "five_hour", "{record:#}");
    assert_eq!(record["limit"]["utilization"], 1.0, "{record:#}");
    assert_eq!(record["limit"]["resets_at"], RESET_AT, "{record:#}");
    assert!(
        record["resume_command"]
            .as_str()
            .is_some_and(|command| command.contains(&scenario.issue_url(7))),
        "{record:#}"
    );
    assert_eq!(record["retaken_by_pickup"], true, "{record:#}");
    assert!(record["failure_category"].is_null(), "{record:#}");
    assert!(
        activity(&scenario)
            .contains("ended: paused: no allowed route has available Plan usage: claude"),
        "{}",
        activity(&scenario)
    );
    // Pickup retakes it, so it waits on no one.
    assert_eq!(inbox_row(&scenario, 7).0, "done");
}

#[test]
fn a_run_with_every_route_in_cooldown_pauses_before_any_session() {
    let scenario = Scenario::new();
    scenario.user_config_is(ROUTES);
    every_route_in_cooldown(&scenario);
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(support::AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(75), "{}", result.stderr);
    assert_eq!(claude_sessions(&scenario), 0);
    assert!(scenario.codex_calls().is_empty());
    assert!(
        result
            .stderr
            .contains("implement: not started: no allowed route has available Plan usage"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(7), ["ready-for-agent"]);
    let record = issue_record(&scenario);
    assert_eq!(record["outcome"], "paused", "{record:#}");
    assert_eq!(record["limit"]["harness"], "claude", "{record:#}");
    assert_eq!(record["limit"]["resets_at"], RESET_AT, "{record:#}");
}

#[test]
fn a_paused_run_on_an_issue_edited_by_an_untrusted_account_is_left_for_triage() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    let mut state = scenario.gh_state();
    state["trust_events"] = json!({"7": [
        {"created_at": "2020-01-04T00:00:00Z", "event": "labeled",
         "label": {"name": "ready-for-agent"}, "actor": {"login": "acme"}}
    ]});
    state["edits"] = json!({"7": [
        {"kind": "body", "by": "outsider", "at": "2020-01-05T00:00:00Z"}
    ]});
    scenario.write_gh_state(&state);
    scenario.agent_does(&stops_at_the_limit());

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(75), "{}", result.stderr);
    assert_eq!(scenario.issue_labels(7), ["needs-triage"], "{}", result.stderr);
    let record = issue_record(&scenario);
    assert!(record["retaken_by_pickup"].is_null(), "{record:#}");
    // No Pickup run retakes it, so it waits in Needs you.
    let (group, kind) = inbox_row(&scenario, 7);
    assert_eq!(group, "needs you");
    assert_eq!(kind, "paused");
}

#[test]
fn agent_written_limit_text_is_an_ordinary_failure() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.agent_does(&format!(
        "echo '{}'\nexit 1\n",
        json!({"type": "assistant", "message": {"content": [{"type": "text",
            "text": "no allowed route has available Plan usage; run the Command again to continue once Plan usage allows"}]}})
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(scenario.issue_labels(7), ["agent-failed"]);
    let record = issue_record(&scenario);
    assert_eq!(record["outcome"], "failed", "{record:#}");
}

#[test]
fn a_pause_pickup_retakes_is_saved_but_not_emailed_when_only_unmerged_runs_are() {
    let scenario = Scenario::new();
    scenario.user_config_is("[email]\nunless_merged = true\nto = \"operator@example.test\"\n");
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(&stops_at_the_limit());
    let resend = ResendStandIn::replying(200, r#"{"id":"accepted"}"#);

    let result = scenario.run_with_env(
        &[&scenario.issue_url(7)],
        &[
            ("RESEND_API_KEY", "re_test_123"),
            ("THIRDSHIFT_RESEND_URL", resend.url()),
        ],
    );

    assert_eq!(result.code, Some(75), "{}", result.stderr);
    assert!(resend.requests().is_empty(), "{:?}", resend.requests());
    let saved = scenario.log_files(
        "home/.thirdshift/logs/acme/widgets/commands/issue",
        "txt",
    );
    assert!(
        saved
            .iter()
            .any(|name| name.ends_with(".notification.txt")),
        "{saved:?}"
    );
}

#[test]
fn a_pause_no_pickup_retakes_waits_in_needs_you_and_is_emailed() {
    let scenario = Scenario::new();
    scenario.user_config_is("[email]\nunless_merged = true\nto = \"operator@example.test\"\n");
    // Hand-started on an issue that was never Ready: nothing to put back.
    scenario.issue_labelled(7, &["bug"]);
    scenario.agent_does(&stops_at_the_limit());
    let resend = ResendStandIn::replying(200, r#"{"id":"accepted"}"#);

    let result = scenario.run_with_env(
        &[&scenario.issue_url(7)],
        &[
            ("RESEND_API_KEY", "re_test_123"),
            ("THIRDSHIFT_RESEND_URL", resend.url()),
        ],
    );

    assert_eq!(result.code, Some(75), "{}", result.stderr);
    assert_eq!(scenario.issue_labels(7), ["bug"]);
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(
        requests[0].body["subject"]
            .as_str()
            .is_some_and(|subject| subject.ends_with(": paused")),
        "{:?}",
        requests[0].body
    );
    let (group, kind) = inbox_row(&scenario, 7);
    assert_eq!(group, "needs you");
    assert_eq!(kind, "paused");
}

#[test]
fn a_spec_run_whose_ticket_pauses_starts_no_queued_ticket_and_pauses() {
    let scenario = Scenario::new();
    scenario.issue_labelled(20, &["ready-for-agent"]);
    scenario.spec_has_tickets(20, &[(21, &[]), (22, &[])]);
    scenario.agent_does_for(21, &stops_at_the_limit());
    scenario.agent_does_for(22, support::AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&["parallel", "1", &scenario.issue_url(20)]);

    assert_eq!(result.code, Some(75), "{}", result.stderr);
    // #22 was queued behind #21, and never started.
    let prompts: Vec<String> = scenario
        .claude_calls()
        .iter()
        .filter_map(|call| call["prompt"].as_str().map(str::to_string))
        .collect();
    assert!(
        prompts.iter().all(|prompt| !prompt.contains("/issues/22")),
        "{prompts:?}"
    );
    assert!(
        result
            .stderr
            .contains("#21 paused: starting no more Tickets until Plan usage allows"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(20), ["ready-for-agent"]);
    assert!(
        !scenario.issue_labels(21).contains(&"agent-failed".to_string()),
        "{:?}",
        scenario.issue_labels(21)
    );
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/issue");
    assert_eq!(record["outcome"], "paused", "{record:#}");
}

#[test]
fn a_custom_process_whose_step_stops_at_its_limit_pauses() {
    let scenario = Scenario::new();
    process_named(&scenario, "notes");
    scenario.agent_does(&format!(
        "echo '{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"working\"}}]}}}}'\necho '{}'\nexit 1\n",
        claude_limit()
    ));

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(75), "{}", result.stderr);
    assert!(
        activity(&scenario).contains("Process notes ended: paused"),
        "{}",
        activity(&scenario)
    );
}
