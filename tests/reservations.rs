//! Origin reservation recovery through scheduled Passes and the shared Inbox.
mod support;

use support::Scenario;

const REFERENCES: [&str; 4] = [
    "refs/heads/thirdshift/pickup/7",
    "refs/heads/thirdshift/pickup/ci-red",
    "refs/thirdshift/reservations/8",
    "refs/thirdshift/reservations/release",
];

fn hostname() -> String {
    let output = std::process::Command::new("hostname").output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

fn reserve(scenario: &Scenario, reference: &str, host: &str, pid: u32, started: &str) -> String {
    let tree = scenario.launch_git(&["mktree"]);
    let message = format!(
        "thirdshift Pickup reservation for test\n\n{host}Launch: /old/checkout\nProcess: {pid}\nStarted: {started}"
    );
    let sha = scenario.launch_git(&["commit-tree", tree.trim(), "-m", &message]);
    scenario.launch_git(&[
        "push",
        "-q",
        "origin",
        &format!("{}:{reference}", sha.trim()),
    ]);
    sha.trim().into()
}

#[test]
fn a_pass_releases_dead_local_owners_of_every_reservation_kind_and_logs_the_blocked_work() {
    let scenario = Scenario::new();
    let host = format!("Host: {}\n", hostname());
    let pid = dead_pid();
    let started = chrono::Utc::now().to_rfc3339();
    for reference in REFERENCES {
        reserve(&scenario, reference, &host, pid, &started);
    }

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let activity =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    for reference in REFERENCES {
        assert!(
            scenario
                .launch_git(&["ls-remote", "--refs", "origin", reference])
                .is_empty(),
            "dead owner still holds {reference}: {}",
            result.stderr
        );
        for log in [&result.stderr, &activity] {
            assert!(
                log.contains(&format!("released orphaned reservation {reference}")),
                "{log}"
            );
            assert!(log.contains(&format!("Process: {pid}")), "{log}");
            assert!(log.contains(&format!("Started: {started}")), "{log}");
        }
    }
    assert!(activity.contains("Base branch CI"), "{activity}");
    assert!(activity.contains("Release run"), "{activity}");
}

fn reservation_rows(scenario: &Scenario) -> Vec<serde_json::Value> {
    let result = scenario.run(&["status", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["source"] == "reservation")
        .cloned()
        .collect()
}

#[test]
fn an_old_untestable_reservation_that_skips_work_waits_for_manual_release_and_can_be_dismissed() {
    for host in ["Host: retired-host\n", ""] {
        let scenario = Scenario::new();
        scenario.issue_is(7, "OPEN");
        scenario.issue_labelled(7, &["ready-for-agent"]);
        let started = (chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339();
        let reference = REFERENCES[0];
        let sha = reserve(&scenario, reference, host, dead_pid(), &started);

        let skipped = scenario.run(&["pickup"]);
        assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
        assert!(
            skipped.stderr.contains("reserved by another run"),
            "{}",
            skipped.stderr
        );
        let skipped_again = scenario.run(&["pickup"]);
        assert_eq!(skipped_again.code, Some(0), "{}", skipped_again.stderr);
        let rows = reservation_rows(&scenario);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["group"], "needs you");
        assert_eq!(rows[0]["reservation"]["reference"], reference);
        assert_eq!(rows[0]["reservation"]["sha"], sha);
        assert!(
            rows[0]["release_command"]
                .as_str()
                .unwrap()
                .contains(&format!("--force-with-lease={reference}:{sha}"))
        );
        let selector = rows[0]["selector"].as_str().unwrap();
        let dismissed = scenario.run(&["dismiss", selector, "--json"]);
        assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
        let rows = reservation_rows(&scenario);
        assert_eq!(rows[0]["group"], "resolved");
        let undo = scenario.run(&["dismiss", selector, "--undo", "--json"]);
        assert_eq!(undo.code, Some(0), "{}", undo.stderr);
        assert_eq!(reservation_rows(&scenario)[0]["group"], "needs you");

        if host.is_empty() {
            let replacement = scenario.origin_git(&["rev-parse", "main"]);
            scenario.origin_git(&["update-ref", reference, replacement.trim(), &sha]);
        } else {
            scenario.origin_git(&["update-ref", "-d", reference, &sha]);
        }
        assert!(reservation_rows(&scenario).is_empty());
    }
}

#[test]
fn live_local_foreign_legacy_and_young_owners_are_retained_without_an_inbox_warning() {
    for (host, pid, started) in [
        (
            format!("Host: {}\n", hostname()),
            std::process::id(),
            chrono::Utc::now().to_rfc3339(),
        ),
        (
            "Host: remote\n".into(),
            dead_pid(),
            chrono::Utc::now().to_rfc3339(),
        ),
        (String::new(), dead_pid(), chrono::Utc::now().to_rfc3339()),
    ] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["ready-for-agent"]);
        let sha = reserve(&scenario, REFERENCES[0], &host, pid, &started);
        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            scenario.origin_git(&["rev-parse", REFERENCES[0]]).trim(),
            sha
        );
        assert!(reservation_rows(&scenario).is_empty());
    }
}

#[test]
fn a_reused_pid_and_a_reservation_from_before_boot_are_released() {
    for started in ["2000-01-01 00:00:00 UTC", "2000-01-01T00:00:00Z"] {
        let scenario = Scenario::new();
        reserve(
            &scenario,
            REFERENCES[0],
            &format!("Host: {}\n", hostname()),
            std::process::id(),
            started,
        );
        let result = scenario.run(&["pickup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            scenario
                .launch_git(&["ls-remote", "origin", REFERENCES[0]])
                .is_empty(),
            "{}",
            result.stderr
        );
    }
}

#[test]
fn a_process_started_after_the_reservation_is_pid_reuse_even_during_this_boot() {
    let scenario = Scenario::new();
    let started = (chrono::Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
    let mut replacement = std::process::Command::new("sleep")
        .arg("120")
        .spawn()
        .unwrap();
    reserve(
        &scenario,
        REFERENCES[0],
        &format!("Host: {}\n", hostname()),
        replacement.id(),
        &started,
    );
    let result = scenario.run(&["pickup"]);
    replacement.kill().unwrap();
    replacement.wait().unwrap();
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        scenario
            .launch_git(&["ls-remote", "origin", REFERENCES[0]])
            .is_empty(),
        "{}",
        result.stderr
    );
}

#[test]
fn a_moved_reservation_loses_its_release_lease_and_warns_without_deleting_the_new_owner() {
    let scenario = Scenario::new();
    let reference = REFERENCES[0];
    let old = reserve(
        &scenario,
        reference,
        &format!("Host: {}\n", hostname()),
        dead_pid(),
        &chrono::Utc::now().to_rfc3339(),
    );
    let replacement = scenario.origin_git(&["rev-parse", "main"]);
    let git = std::process::Command::new("which")
        .arg("git")
        .output()
        .unwrap();
    let git = String::from_utf8(git.stdout).unwrap();
    support::write_executable(
        &scenario.path("bin/git"),
        &format!(
            "#!/bin/sh\ncase \"$*\" in\n*--force-with-lease={reference}:{old}*)\n  '{}' --git-dir '{}' update-ref '{reference}' '{}' '{old}' || exit $?\n;;\nesac\nexec '{}' \"$@\"\n",
            git.trim(),
            scenario.origin_dir().display(),
            replacement.trim(),
            git.trim()
        ),
    );
    let result = scenario.run(&["pickup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.origin_git(&["rev-parse", reference]), replacement);
    assert!(
        result
            .stderr
            .contains("warning: could not release orphaned reservation"),
        "{}",
        result.stderr
    );
    let activity =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(
        activity.contains("warning: could not release orphaned reservation"),
        "{activity}"
    );
}

#[test]
fn every_command_limited_pass_and_the_review_author_pass_recovers_before_selecting_work() {
    for args in [
        vec!["fix", "base", "main"],
        vec!["review", "--author", "alice"],
        vec!["triage"],
        vec!["architect"],
        vec!["secure"],
    ] {
        let scenario = Scenario::new();
        if matches!(args[0], "architect" | "secure") {
            scenario.issue_is(9, "OPEN");
            scenario.issue_labelled(9, &["ready-for-agent"]);
        }
        reserve(
            &scenario,
            REFERENCES[0],
            &format!("Host: {}\n", hostname()),
            dead_pid(),
            &chrono::Utc::now().to_rfc3339(),
        );
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(0), "{args:?}: {}", result.stderr);
        assert!(
            scenario
                .launch_git(&["ls-remote", "origin", REFERENCES[0]])
                .is_empty(),
            "{args:?}: {}",
            result.stderr
        );
    }
}
