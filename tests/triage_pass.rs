//! Unattended Triage Passes select one issue through the real CLI.
mod support;

use serde_json::json;
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/acme/widgets";

const RECOMMEND: &str = r###"printf '%s\n' 'Triage recommendation: {"category":"bug","state":"ready-for-agent","issues":[{"title":"Handle an empty cache","body":"## Agent Brief\n\nReproduce with an empty cache."}],"investigation":"The cache read assumes an entry exists."}' > "$FAKE_CLAUDE_FINAL_MESSAGE""###;

#[test]
fn recommendation_only_pass_reserves_without_claiming_or_publishing_and_notifies() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["bug"]);
    scenario.issue_labelled(7, &["needs-triage"]);
    scenario.user_config_is("[pass]\ncommand_limit = 2\n");
    scenario.agent_does(&format!(
        "{}{RECOMMEND}",
        support::concurrency::waits_on("recommend")
    ));
    let before = scenario.gh_state();
    let resend = support::resend::ResendStandIn::replying(200, r#"{"id":"triage-pass"}"#);
    let held = scenario.run_until(
        &["triage", "--recommend-only", "email", "day@example.com"],
        &[
            ("RESEND_API_KEY", "test-key"),
            ("THIRDSHIFT_RESEND_URL", resend.url()),
        ],
        "started-recommend",
    );
    let reserved = support::concurrency::reservations(&scenario);
    let competing = scenario.run_with_env(
        &["triage", "email", "day@example.com"],
        &[
            ("RESEND_API_KEY", "test-key"),
            ("THIRDSHIFT_RESEND_URL", resend.url()),
        ],
    );
    support::concurrency::release(&scenario, "recommend");
    let result = held.finish();
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(reserved, "refs/heads/thirdshift/pickup/7");
    assert_eq!(competing.code, Some(0), "{}", competing.stderr);
    assert!(
        competing.stderr.contains("reserved by another run"),
        "{}",
        competing.stderr
    );
    assert_eq!(scenario.gh_state(), before);
    assert!(support::concurrency::reservations(&scenario).is_empty());
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert!(
        requests[0].body["text"]
            .as_str()
            .unwrap()
            .contains(&scenario.issue_url(7))
    );
    support::resend::assert_saved_notification(&scenario, "triage", &requests[0]);
}

#[test]
fn a_failed_triage_counts_until_the_day_shift_retries_takes_over_or_closes_it() {
    for action in ["retry", "take over", "close"] {
        let scenario = Scenario::new();
        scenario.user_config_is("[pickup]\nfailure_limit = 1\n");
        scenario.issue_labelled(7, &["needs-triage"]);
        scenario.agent_does("exit 1");
        let failed = scenario.run(&["triage"]);
        assert_ne!(failed.code, Some(0));
        assert_eq!(
            scenario.issue_labels(7),
            ["ready-for-human", "agent-failed"]
        );
        let waiting = scenario.run(&["triage"]);
        assert_eq!(waiting.code, Some(0), "{}", waiting.stderr);
        assert!(
            waiting.stderr.contains("at the Failure limit"),
            "{}",
            waiting.stderr
        );
        assert_eq!(
            scenario.issue_labels(7),
            ["ready-for-human", "agent-failed"]
        );
        // Removing ready-for-human alone does not acknowledge the failure.
        scenario.issue_labelled(7, &["agent-failed"]);
        let waiting = scenario.run(&["triage"]);
        assert_eq!(waiting.code, Some(0), "{}", waiting.stderr);
        assert!(
            waiting.stderr.contains("at the Failure limit"),
            "{}",
            waiting.stderr
        );
        assert_eq!(scenario.issue_labels(7), ["agent-failed"]);
        match action {
            "retry" => scenario.issue_labelled(7, &["ready-for-agent", "agent-failed"]),
            "take over" => scenario.issue_labelled(7, &["needs-info"]),
            "close" => scenario.issue_is(7, "CLOSED"),
            _ => unreachable!(),
        }
        let handled = scenario.run(&["triage"]);
        assert_eq!(handled.code, Some(0), "{}", handled.stderr);
        assert!(
            !handled.stderr.contains("at the Failure limit"),
            "{}",
            handled.stderr
        );
        assert!(
            !scenario
                .issue_labels(7)
                .iter()
                .any(|label| label == "agent-failed")
        );
    }
}

#[test]
fn report_mode_rejects_a_missing_base_branch_before_starting_a_session() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["bug"]);
    scenario.agent_does(RECOMMEND);
    let result = scenario.run_with_input(
        &["triage", "base", "missing", "--recommend-only"],
        "Investigate cache misses.",
    );
    assert_ne!(
        result.code,
        Some(0),
        "the selected base must be validated: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("base branch missing does not exist"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn eof_selects_an_issue_and_triages_it_on_the_selected_base() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["bug"]);
    scenario.issue_labelled(7, &["needs-triage"]);
    scenario.origin_has_branch("investigate", "main", &[]);
    scenario.origin_has_commit("investigate", "base-marker", "selected base", "Base");
    scenario.agent_does(&format!(
        "cat base-marker > '{}'\n{RECOMMEND}",
        scenario.path("selected-base").display()
    ));
    let result = scenario.run(&["triage", "base", "investigate"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.issue_labels(7), ["ready-for-human", "bug"]);
    assert_eq!(
        std::fs::read_to_string(scenario.path("selected-base")).unwrap(),
        "selected base"
    );
    assert!(support::concurrency::reservations(&scenario).is_empty());
    let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(output["issues"][0]["url"], scenario.issue_url(7));
    let (_, notification) = support::resend::saved_notification(&scenario, "triage");
    assert!(
        notification.contains(&scenario.issue_url(7)),
        "{notification}"
    );
    let activity = std::fs::read_to_string(scenario.path(&format!("{LOGS}/activity.log"))).unwrap();
    assert!(activity.contains("Triage run started"), "{activity}");
    assert!(activity.contains("Triage run ended"), "{activity}");
    assert!(
        !scenario
            .log_files(&format!("{LOGS}/commands/triage"), "log")
            .is_empty()
    );
}

#[test]
fn empty_stdin_and_a_terminal_skip_without_work_and_deduplicate_quiet_activity() {
    for quiet in [false, true] {
        let scenario = Scenario::new();
        scenario.user_config_is(&format!("[activity]\nquiet_skips = {quiet}\n"));
        for text in ["", " \n\t"] {
            let result = scenario.run_with_input(&["triage"], text);
            assert_eq!(result.code, Some(0), "{}", result.stderr);
            assert!(result.stdout.is_empty());
            if quiet {
                assert!(result.stderr.is_empty(), "{}", result.stderr);
            } else {
                assert!(
                    result.stderr.contains("no eligible needs-triage issue"),
                    "{}",
                    result.stderr
                );
            }
        }
        let terminal = scenario.run_on_terminal(&["triage"], &[], &[]);
        assert_eq!(terminal.code, Some(0), "{}", terminal.stderr);
        assert!(terminal.stdout.is_empty());
        if quiet {
            assert!(terminal.stderr.is_empty(), "{}", terminal.stderr);
        }
        let activity =
            std::fs::read_to_string(scenario.path(&format!("{LOGS}/activity.log"))).unwrap();
        assert_eq!(
            activity
                .lines()
                .filter(|line| line.contains("Triage run skipped:"))
                .count(),
            1,
            "{activity}"
        );
        assert!(scenario.claude_calls().is_empty());
        assert!(!scenario.path(&format!("{LOGS}/commands/triage")).exists());
        assert!(
            scenario
                .path(&format!("{LOGS}/triage.last-fired"))
                .is_file()
        );
        let status = scenario.run(&["status", "--json"]);
        assert_eq!(status.code, Some(0), "{}", status.stderr);
        let status: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
        assert!(
            status["idle_passes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|pass| pass["kind"] == "Triage run"),
            "{status}"
        );
    }
}

#[test]
fn pass_filters_conflicting_states_claims_failures_parent_specs_and_untrusted_text() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["bug"]);
    scenario.user_config_is("[pickup]\nfailure_limit = 20\n");
    for (number, label) in [
        (1, "ready-for-agent"),
        (2, "ready-for-human"),
        (3, "needs-info"),
        (4, "wontfix"),
        (5, "in-progress"),
        (6, "agent-failed"),
    ] {
        scenario.issue_is(number, "OPEN");
        scenario.issue_labelled(number, &["needs-triage", label]);
    }
    scenario.issue_labelled(7, &["needs-triage"]);
    scenario.spec_has_tickets(7, &[(99, &[])]);
    for number in 8..=11 {
        scenario.issue_is(number, "OPEN");
        scenario.issue_labelled(number, &["needs-triage"]);
    }
    let mut state = scenario.gh_state();
    state["author_associations"]["8"] = json!("NONE");
    for (number, kind) in [(9, "body"), (10, "title")] {
        state["edits"][number.to_string()] =
            json!([{"kind":kind,"by":"stranger","at":"2019-01-01T00:00:00Z"}]);
    }
    scenario.write_gh_state(&state);
    scenario.agent_does(RECOMMEND);
    let result = scenario.run(&["triage", "--allow-ready-for-agent"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.issue_labels(11), ["ready-for-agent", "bug"]);
    for (number, reason) in [
        (1, "ready-for-agent"),
        (2, "ready-for-human"),
        (3, "needs-info"),
        (4, "wontfix"),
        (5, "in-progress"),
        (6, "agent-failed"),
        (7, "parent Spec"),
        (8, "Trusted account"),
        (9, "body"),
        (10, "title"),
    ] {
        assert!(
            result
                .stderr
                .lines()
                .any(|line| line.contains(&format!("#{number} ")) && line.contains(reason)),
            "{}",
            result.stderr
        );
    }
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn pass_uses_the_configured_issue_order() {
    for (order, selected) in [("lowest-first", 7), ("highest-first", 9)] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["bug"]);
        scenario.user_config_is(&format!("[pickup]\norder = '{order}'\n"));
        for number in [7, 9] {
            scenario.issue_is(number, "OPEN");
            scenario.issue_labelled(number, &["needs-triage"]);
        }
        scenario.agent_does(RECOMMEND);
        let result = scenario.run(&["triage"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let output: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(output["issues"][0]["url"], scenario.issue_url(selected));
        assert_eq!(
            scenario.issue_labels(if selected == 7 { 9 } else { 7 }),
            ["needs-triage"]
        );
    }
}

#[test]
fn pass_sweeps_and_clears_handed_back_failures_before_checking_the_failure_limit() {
    let scenario = Scenario::new();
    scenario.user_config_is("[pickup]\nfailure_limit = 1\n");
    scenario.issue_labelled(7, &["needs-triage"]);
    for number in 8..=11 {
        scenario.issue_is(number, "OPEN");
    }
    scenario.issue_is(8, "CLOSED");
    scenario.issue_labelled(8, &["bug", "in-progress", "agent-failed"]);
    scenario.issue_labelled(9, &["bug", "ready-for-agent", "agent-failed"]);
    scenario.issue_labelled(10, &["bug", "needs-info", "agent-failed"]);
    scenario.issue_labelled(11, &["bug", "in-progress", "agent-failed"]);
    let result = scenario.run(&["triage"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.is_empty());
    assert!(
        result.stderr.contains("at the Failure limit"),
        "{}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains("#10 and #11 are labelled agent-failed"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(8), ["bug"]);
    assert_eq!(scenario.issue_labels(9), ["bug", "ready-for-agent"]);
    assert_eq!(
        scenario.issue_labels(10),
        ["bug", "needs-info", "agent-failed"]
    );
    assert_eq!(scenario.issue_labels(11), ["bug", "agent-failed"]);
    assert!(scenario.claude_calls().is_empty());
    assert!(!scenario.path(&format!("{LOGS}/commands/triage")).exists());
}

#[test]
fn command_limit_skips_before_sweep_but_another_slot_admits_a_triage_pass() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["bug"]);
    scenario.issue_labelled(7, &["needs-triage"]);
    scenario.issue_is(8, "OPEN");
    scenario.issue_labelled(8, &["needs-triage"]);
    scenario.issue_is(9, "CLOSED");
    scenario.issue_labelled(9, &["in-progress", "agent-failed"]);
    scenario.agent_does(RECOMMEND);
    scenario.agent_does_in_session(
        1,
        &format!("{}{RECOMMEND}", support::concurrency::waits_on("holder")),
    );
    let holder = scenario.run_until(&["triage", "7"], &[], "started-holder");
    let skipped = scenario.run(&["triage"]);
    let unswept = scenario.issue_labels(9);
    scenario.user_config_is("[pass]\ncommand_limit = 2\n");
    let admitted = scenario.run(&["triage"]);
    support::concurrency::release(&scenario, "holder");
    let held = holder.finish();
    assert_eq!(held.code, Some(0), "{}", held.stderr);
    assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    assert!(
        skipped.stderr.contains("at the Command limit"),
        "{}",
        skipped.stderr
    );
    assert!(skipped.stdout.is_empty());
    assert_eq!(unswept, ["in-progress", "agent-failed"]);
    assert_eq!(admitted.code, Some(0), "{}", admitted.stderr);
    assert_eq!(scenario.issue_labels(8), ["ready-for-human", "bug"]);
    assert!(scenario.issue_labels(9).is_empty());
    assert!(support::concurrency::reservations(&scenario).is_empty());
}

#[test]
fn reserved_or_stale_candidates_give_way_to_the_next_issue() {
    for stale in [
        "reserved", "closed", "state", "claimed", "author", "edit", "spec",
    ] {
        let scenario = Scenario::new();
        scenario.repo_has_labels(&["bug"]);
        for number in [7, 8] {
            scenario.issue_is(number, "OPEN");
            scenario.issue_labelled(number, &["needs-triage"]);
        }
        if stale == "reserved" {
            let token = scenario.origin_git(&["rev-parse", "main"]);
            scenario.origin_git(&["update-ref", "refs/heads/thirdshift/pickup/7", token.trim()]);
        } else {
            let edit = match stale {
                "closed" => "s['issues']['7']='CLOSED'",
                "state" => "s['labels']['7']=['ready-for-human']",
                "claimed" => "s['labels']['7']=['needs-triage','in-progress']",
                "author" => "s.setdefault('author_associations',{})['7']='NONE'",
                "edit" => {
                    "s.setdefault('edits',{})['7']=[dict(kind='body',by='stranger',at='2020-01-01T00:00:00Z')]"
                }
                "spec" => "s.setdefault('sub_issues',{})['7']=[8]",
                _ => unreachable!(),
            };
            let mut state = scenario.gh_state();
            state["on_issue_view"]["7"] = json!({"fields":"title,state,labels,createdAt","script":format!("python3 - <<'PY'\nimport json,os\np=os.environ['FAKE_GH_STATE']\ns=json.load(open(p))\n{edit}\njson.dump(s,open(p,'w'))\nPY")});
            scenario.write_gh_state(&state);
        }
        scenario.agent_does(RECOMMEND);
        let result = scenario.run(&["triage"]);
        assert_eq!(result.code, Some(0), "{stale}: {}", result.stderr);
        assert_eq!(
            scenario.issue_labels(8),
            ["ready-for-human", "bug"],
            "{stale}"
        );
        assert!(
            result.stderr.contains(if stale == "reserved" {
                "reserved by another run"
            } else {
                "selection went stale"
            }),
            "{stale}: {}",
            result.stderr
        );
        if stale == "reserved" {
            assert_eq!(
                support::concurrency::reservations(&scenario),
                "refs/heads/thirdshift/pickup/7"
            );
        } else {
            assert!(
                support::concurrency::reservations(&scenario).is_empty(),
                "{stale}"
            );
        }
    }
}
