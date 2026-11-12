//! Every Pass shares GitHub quota admission before its first GitHub read.
mod support;

use serde_json::json;
use support::Scenario;
use support::resend::ResendStandIn;

fn commands() -> Vec<Vec<&'static str>> {
    vec![
        vec!["pickup"],
        vec!["architect"],
        vec!["secure"],
        vec!["triage"],
        vec!["review", "--author", "factory"],
        vec!["fix"],
        vec!["process", "notes"],
    ]
}

fn setup(scenario: &Scenario) {
    let dir = scenario.path("home/.thirdshift/processes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("notes.toml"),
        "version = 1\ndescription = \"Notes\"\n[[steps]]\nname = \"notes\"\nskill = \"code-review\"\n",
    )
    .unwrap();
}

fn edit_quota_record(scenario: &Scenario, change: impl FnOnce(&mut serde_json::Value)) {
    let path = scenario.path("home/.thirdshift/github-quota.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    change(&mut record);
    std::fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
}

fn expire_wait(scenario: &Scenario) {
    edit_quota_record(scenario, |record| {
        for wait in record["waits"].as_array_mut().unwrap() {
            wait["reset_at"] = json!("2000-01-01T00:00:00Z");
        }
    });
}

#[test]
fn status_shows_the_machine_quota_wait_beside_pass_starts_until_reset() {
    let scenario = Scenario::new();
    let mut gh = scenario.gh_state();
    gh["rate_limits"]["graphql"] = json!({
        "remaining": 42,
        "reset": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
    });
    scenario.write_gh_state(&gh);
    assert_eq!(scenario.run(&["pickup"]).code, Some(0));
    let before = scenario.gh_calls();
    let status = scenario.run(&["status"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    assert!(
        status.stdout.contains("Pickup last fired"),
        "{}",
        status.stdout
    );
    assert!(
        status
            .stdout
            .lines()
            .any(|line| line.starts_with("GitHub Quota wait until")),
        "{}",
        status.stdout
    );
    assert!(
        status.stdout.contains("graphql: 42 left"),
        "{}",
        status.stdout
    );
    assert_eq!(scenario.gh_calls(), before);
    expire_wait(&scenario);
    assert!(
        !scenario
            .run(&["status"])
            .stdout
            .lines()
            .any(|line| line.starts_with("GitHub Quota wait until"))
    );
}

#[test]
fn each_pass_checks_live_quota_before_work_and_admits_exactly_500() {
    for resource in ["core", "graphql"] {
        for args in commands() {
            let scenario = Scenario::new();
            setup(&scenario);
            let reset = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
            let mut gh = scenario.gh_state();
            gh["rate_limits"][resource] = json!({"remaining": 499, "reset": reset});
            scenario.write_gh_state(&gh);
            let output = scenario.run(&args);
            assert_eq!(output.code, Some(0), "{args:?}: {}", output.stderr);
            assert!(output.stdout.is_empty(), "{args:?}: {}", output.stdout);
            assert!(
                output.stderr.contains("GitHub Quota wait until"),
                "{args:?}: {}",
                output.stderr
            );
            assert!(scenario.gh_calls().iter().all(|call| {
                call.starts_with(&["api".into(), "rate_limit".into()])
                    || call.starts_with(&["api".into(), "user".into()])
            }));
            assert!(scenario.claude_calls().is_empty());
            assert!(
                !scenario
                    .path("home/.thirdshift/command-slots/acme/widgets/0.slot")
                    .exists()
            );
            expire_wait(&scenario);
            gh["rate_limits"][resource]["remaining"] = json!(500);
            scenario.write_gh_state(&gh);
            let calls = scenario.gh_calls().len();
            let output = scenario.run(&args);
            assert!(
                !output.stderr.contains("GitHub Quota wait until"),
                "{args:?}: {}",
                output.stderr
            );
            assert!(
                scenario.gh_calls()[calls..]
                    .iter()
                    .any(|call| { !call.starts_with(&["api".into(), "rate_limit".into()]) })
                    || !scenario.claude_calls().is_empty(),
                "no work after reset: {args:?}"
            );
        }
    }
}

#[test]
fn a_non_quota_probe_failure_admits_every_pass_but_exhaustion_records_a_wait() {
    for args in commands() {
        let scenario = Scenario::new();
        setup(&scenario);
        scenario.gh_fails("api rate_limit");
        let output = scenario.run(&args);
        assert!(
            !output.stderr.contains("GitHub Quota wait until"),
            "{args:?}: {}",
            output.stderr
        );
        assert!(
            scenario
                .gh_calls()
                .iter()
                .any(|call| { !call.starts_with(&["api".into(), "rate_limit".into()]) })
                || !scenario.claude_calls().is_empty(),
            "{args:?}"
        );
    }
    for diagnostic in [
        "HTTP 403: API rate limit exceeded for factory",
        "GraphQL: API rate limit already exceeded (RATE_LIMITED)",
    ] {
        let scenario = Scenario::new();
        let mut gh = scenario.gh_state();
        gh["failing"] = json!(["api rate_limit"]);
        gh["failing_stderr"] = json!(diagnostic);
        scenario.write_gh_state(&gh);
        let skipped = scenario.run(&["pickup"]);
        assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
        assert!(skipped.stdout.is_empty());
        assert!(
            skipped.stderr.contains("GitHub Quota wait until"),
            "{}",
            skipped.stderr
        );
        let before = scenario.gh_calls();
        assert_eq!(scenario.run(&["pickup"]).code, Some(0));
        assert_eq!(scenario.gh_calls(), before);
        assert!(
            !scenario
                .path("home/.thirdshift/command-slots/acme/widgets/0.slot")
                .exists()
        );
    }
}

#[test]
fn exhausted_quotas_skip_even_when_login_cannot_be_probed() {
    let scenario = Scenario::new();
    let reset = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
    let mut state = scenario.gh_state();
    state["rate_limits"] = json!({
        "core": {"remaining": 0, "reset": reset},
        "graphql": {"remaining": 0, "reset": reset}
    });
    state["failing"] = json!(["api user", "api graphql", "auth status"]);
    state["failing_stderr"] = json!("API rate limit exceeded (HTTP 403)");
    scenario.write_gh_state(&state);
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("GitHub Quota wait until"),
        "{}",
        result.stderr
    );
    assert!(
        !scenario
            .path("home/.thirdshift/command-slots/acme/widgets/0.slot")
            .exists()
    );
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn unchanged_quota_wait_deduplicates_alternating_custom_processes() {
    for names in [["notes", "other-notes"], ["notes", "notes daily"]] {
        let scenario = Scenario::new();
        setup(&scenario);
        std::fs::copy(
            scenario.path("home/.thirdshift/processes/notes.toml"),
            scenario.path(&format!("home/.thirdshift/processes/{}.toml", names[1])),
        )
        .unwrap();
        let mut gh = scenario.gh_state();
        gh["rate_limits"]["core"] = json!({
            "remaining": 499,
            "reset": (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp(),
        });
        scenario.write_gh_state(&gh);
        for _ in 0..3 {
            for name in names {
                let output = scenario.run(&["process", name]);
                assert_eq!(output.code, Some(0), "{name}: {}", output.stderr);
                assert!(output.stdout.is_empty(), "{name}: {}", output.stdout);
            }
        }
        let activity = std::fs::read_to_string(
            scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"),
        )
        .unwrap();
        for name in names {
            let prefix = format!("Process {name} skipped: GitHub Quota wait until");
            assert_eq!(
                activity
                    .lines()
                    .filter(|line| line.contains(&prefix))
                    .count(),
                1,
                "unchanged quota reason repeated for {name}:\n{activity}"
            );
        }
    }
}

#[test]
fn every_pass_skips_below_either_reserve_and_reuses_the_wait_without_notifications() {
    for resource in ["core", "graphql"] {
        let scenario = Scenario::new();
        setup(&scenario);
        let reset = chrono::Utc::now() + chrono::Duration::hours(1);
        let mut gh = scenario.gh_state();
        gh["rate_limits"][resource] = json!({"remaining": 499, "reset": reset.timestamp()});
        // A healthy subsequent observation must not bypass the recorded wait.
        gh["next_rate_limits"] = json!({});
        scenario.write_gh_state(&gh);
        let resend = ResendStandIn::replying(200, "{}");
        scenario.user_config_is("[email]\nalways = true\nto = \"operator@example.test\"\n");
        for _ in 0..3 {
            for args in commands() {
                let output = scenario.run_with_env(
                    &args,
                    &[
                        ("RESEND_API_KEY", "re_test_123"),
                        ("THIRDSHIFT_RESEND_URL", resend.url()),
                    ],
                );
                assert_eq!(output.code, Some(0), "{args:?}: {}", output.stderr);
                assert!(output.stdout.is_empty(), "{args:?}: {}", output.stdout);
            }
        }
        let calls = scenario.gh_calls();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with(&["api".into(), "rate_limit".into()]))
                .count(),
            1,
            "{calls:?}"
        );
        assert!(
            calls.iter().all(
                |call| call.starts_with(&["api".into(), "rate_limit".into()])
                    || call.starts_with(&["api".into(), "user".into()])
            ),
            "GitHub work after skip: {calls:?}"
        );
        assert!(scenario.claude_calls().is_empty());
        assert!(resend.requests().is_empty());
        let activity = std::fs::read_to_string(
            scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"),
        )
        .unwrap();
        assert_eq!(activity.lines().count(), 7, "{activity}");
        assert!(
            activity
                .lines()
                .all(|line| line.contains("skipped: GitHub Quota wait until")
                    && line.contains(&format!("({resource}: 499 left)"))),
            "{activity}"
        );
        assert!(
            !scenario
                .path("home/.thirdshift/command-slots/acme/widgets/0.slot")
                .exists()
        );
    }
}

#[test]
fn a_stored_wait_with_the_reserve_available_does_not_skip_a_pass() {
    let scenario = Scenario::new();
    // A first Pass caches the account beside an empty wait list.
    assert_eq!(scenario.run(&["pickup"]).code, Some(0));
    // A wait recorded after the window had already reset, as by v0.30.0.
    edit_quota_record(&scenario, |record| {
        let account = record["identities"][0]["account"].clone();
        record["waits"] = json!([{
            "host": account["host"],
            "login": account["login"],
            "resource": "graphql",
            "remaining": 4995,
            "reset_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        }]);
    });
    let calls = scenario.gh_calls().len();
    // `status` reads only the stored record, so it goes first: Pickup's own
    // probe would clear the wait.
    for args in [vec!["status"], vec!["pickup"]] {
        let output = scenario.run(&args);
        assert_eq!(output.code, Some(0), "{args:?}: {}", output.stderr);
        let shown = format!("{}{}", output.stdout, output.stderr);
        assert!(
            !shown.contains("GitHub Quota wait until") && !shown.contains("Inbox reads paused"),
            "{args:?}: {shown}"
        );
    }
    assert!(
        scenario.gh_calls()[calls..]
            .iter()
            .any(|call| !call.starts_with(&["api".into(), "rate_limit".into()])),
        "Pickup did no GitHub work"
    );
}
