use super::{Console, configure, record, section};
use serde_json::Value;
use std::path::Path;
use std::{fs, process::Command};

/// A Triage URL that fails the origin check is still recorded. The compact
/// header must not lend the launch repository's name to that other issue.
#[test]
fn a_rejected_foreign_triage_issue_is_not_linked_under_the_local_repository() {
    let scenario = super::support::Scenario::new();
    scenario.user_config_is("[console]\nport = 0\n");
    let foreign = "https://github.com/attacker/phish/issues/4";
    let result = scenario.run(&["triage", foreign]);
    assert_ne!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("origin mismatch"),
        "{}",
        result.stderr
    );
    let console = Console::start(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        &scenario.path("home"),
    );
    for page in ["/?section=overview", "/?section=needs-you"] {
        let html = console.get(page).text().unwrap();
        let heads: Vec<_> = html
            .split("<div class=\"row-head\">")
            .skip(1)
            .map(|head| head.split("</div>").next().unwrap())
            .collect();
        assert!(
            heads.iter().any(|head| head.contains("acme/widgets#4")),
            "compact rows lost the recorded subject on {page}"
        );
        for head in &heads {
            assert!(
                !head.contains("href=\"https://github.com/attacker/phish/issues/4\""),
                "header lends the local repository to {foreign}: {head}"
            );
        }
        let mut saw_heading = false;
        for peek in html.split("data-peek=\"").skip(1) {
            let path = peek.split('"').next().unwrap();
            let details = console.get(path).text().unwrap();
            let heading = details
                .split("class=\"peek-heading\"")
                .nth(1)
                .unwrap_or("")
                .split("</h2>")
                .next()
                .unwrap_or("");
            if heading.contains("acme/widgets#4") {
                saw_heading = true;
            }
            assert!(
                !heading.contains("href=\"https://github.com/attacker/phish/issues/4\""),
                "details lend the local repository to {foreign}: {heading}"
            );
        }
        assert!(saw_heading, "details lost the recorded subject on {page}");
    }
}

#[test]
fn needs_you_review_and_triage_headers_link_targets_locally_and_across_fleet() {
    use serde_json::json;
    use std::time::{Duration, Instant};

    let owner = tempfile::tempdir().unwrap();
    let caller = tempfile::tempdir().unwrap();
    configure(owner.path());
    configure(caller.path());
    let cases = [
        (
            "Review run",
            false,
            Some("https://github.com/acme/widgets/pull/101"),
            Some("pull/101"),
        ),
        (
            "Review run",
            true,
            Some("https://github.com/acme/widgets/pull/102"),
            Some("pull/102"),
        ),
        ("Triage run", true, None, Some("issues/3")),
        (
            "Triage run",
            true,
            Some("https://github.com/acme/widgets/pull/104"),
            Some("pull/104"),
        ),
        ("Triage run", false, None, None),
        ("Review run", true, None, Some("issues/6")),
        (
            "Review run",
            true,
            Some("https://github.com/other/repo/pull/107"),
            Some("issues/7"),
        ),
        (
            "Triage run",
            true,
            Some("https://evil.example/pull/108"),
            Some("issues/8"),
        ),
    ];
    for (index, (kind, issue, pr, _)) in cases.iter().enumerate() {
        let number = index as u64 + 1;
        let mut fixture = record(owner.path(), number, Some("failed"), 0);
        fixture["kind"] = json!(kind);
        fixture["name"] = json!(format!("target-fixture-{number}"));
        fixture["cause"] = json!(format!("Target failure {number}"));
        fixture["tickets"] = json!([]);
        if !issue {
            fixture["issue"] = Value::Null;
        }
        fixture["pull_request"] = json!(pr);
        super::write_record(owner.path(), number, &fixture);
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let local = Console::start(binary, owner.path());
    let token = super::tokens::create(owner.path(), "caller", &["--scopes", "read"]);
    assert!(
        super::fleet::add(caller.path(), "owner", &local.address, &token)
            .status
            .success()
    );
    let fleet = Console::start(binary, caller.path());
    for console in [&local, &fleet] {
        let _events = console.get("/events");
        for page in ["/?section=overview", "/?section=needs-you"] {
            let deadline = Instant::now() + Duration::from_secs(30);
            let html = loop {
                let html = console.get(page).text().unwrap();
                if html.contains("target-fixture-5") {
                    break html;
                }
                assert!(Instant::now() < deadline, "{html}");
                std::thread::sleep(Duration::from_millis(25));
            };
            let heads: Vec<_> = section(&html, "needs you")
                .split("<div class=\"row-head\">")
                .skip(1)
                .map(|head| head.split("</div>").next().unwrap())
                .collect();
            assert_eq!(heads.len(), cases.len(), "{html}");
            for (index, (_, _, _, target)) in cases.iter().enumerate() {
                if let Some(target) = target {
                    let url = format!("href=\"https://github.com/acme/widgets/{target}\"");
                    let head = heads
                        .iter()
                        .find(|head| head.contains(&url))
                        .unwrap_or_else(|| panic!("header target {url} missing: {heads:#?}"));
                    if target.starts_with("pull/") {
                        assert!(
                            !head.contains(&format!("/issues/{}\"", index + 1)),
                            "{head}"
                        );
                    }
                } else {
                    let head = heads
                        .iter()
                        .find(|head| head.contains("target-fixture-5"))
                        .unwrap();
                    assert!(head.contains("acme/widgets"), "{head}");
                    assert!(!head.contains("href="), "{head}");
                }
            }
            assert!(
                !heads
                    .iter()
                    .any(|head| head.contains("evil.example") || head.contains("other/repo")),
                "{heads:#?}"
            );
        }
    }
}

#[test]
fn suspended_time_health_agrees_in_status_and_the_console_inbox() {
    use super::watching::live_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let now = chrono::Local::now();
    struct Case {
        number: u64,
        stage: &'static str,
        stage_minutes: i64,
        quiet_minutes: u64,
        suspended_minutes: i64,
        resumed_minutes_ago: i64,
        group: &'static str,
        state: &'static str,
        health: &'static str,
    }
    let cases = [
        Case {
            number: 1,
            stage: "implement",
            stage_minutes: 65,
            quiet_minutes: 0,
            suspended_minutes: 60,
            resumed_minutes_ago: 5,
            group: "working",
            state: "live",
            health: "ok",
        },
        Case {
            number: 2,
            stage: "implement",
            stage_minutes: 65,
            quiet_minutes: 0,
            suspended_minutes: 0,
            resumed_minutes_ago: 0,
            group: "needs you",
            state: "Slow",
            health: "slow",
        },
        Case {
            number: 3,
            stage: "implement",
            stage_minutes: 75,
            quiet_minutes: 0,
            suspended_minutes: 60,
            resumed_minutes_ago: 5,
            group: "working",
            state: "live",
            health: "slow",
        },
        Case {
            number: 4,
            stage: "Code review",
            stage_minutes: 180,
            quiet_minutes: 65,
            suspended_minutes: 60,
            resumed_minutes_ago: 5,
            group: "working",
            state: "live",
            health: "ok",
        },
        Case {
            number: 5,
            stage: "Code review",
            stage_minutes: 180,
            quiet_minutes: 65,
            suspended_minutes: 0,
            resumed_minutes_ago: 0,
            group: "needs you",
            state: "Silent",
            health: "silent",
        },
        Case {
            number: 6,
            stage: "Code review",
            stage_minutes: 180,
            quiet_minutes: 74,
            suspended_minutes: 60,
            resumed_minutes_ago: 5,
            group: "needs you",
            state: "Silent",
            health: "silent",
        },
        Case {
            number: 7,
            stage: "Code review",
            stage_minutes: 180,
            quiet_minutes: 20,
            suspended_minutes: 60,
            resumed_minutes_ago: 25,
            group: "needs you",
            state: "Silent",
            health: "silent",
        },
        Case {
            number: 8,
            stage: "CI watch",
            stage_minutes: 65,
            quiet_minutes: 65,
            suspended_minutes: 60,
            resumed_minutes_ago: 5,
            group: "landing",
            state: "Landing",
            health: "ok",
        },
    ];
    for case in &cases {
        let mut fixture = live_record(home.path(), case.number, std::process::id());
        let began = (now - chrono::Duration::minutes(case.stage_minutes)).to_rfc3339();
        fixture["kind"] = json!("Run");
        fixture["tickets"] = json!([]);
        fixture["started_at"] = json!(began);
        fixture["stages"] = json!([{
            "name": case.stage, "began_at": began, "session_log": null
        }]);
        if case.suspended_minutes > 0 {
            fixture["stages"][0]["suspended_seconds"] = json!(case.suspended_minutes * 60);
            fixture["stages"][0]["suspensions"] = json!([{
                "resumed_at": (now - chrono::Duration::minutes(case.resumed_minutes_ago)).to_rfc3339(),
                "seconds": case.suspended_minutes * 60
            }]);
        }
        super::write_record(home.path(), case.number, &fixture);
        let log = home.path().join(format!(
            ".thirdshift/logs/acme/widgets/commands/issue/{}-fixture.log",
            case.number
        ));
        fs::write(&log, "working\n").unwrap();
        fs::File::open(log)
            .unwrap()
            .set_modified(
                std::time::SystemTime::now()
                    - std::time::Duration::from_secs(case.quiet_minutes * 60),
            )
            .unwrap();
    }
    for number in 20..23 {
        let mut fixture = record(home.path(), number, Some("merged"), 0);
        let end = now - chrono::Duration::hours(4);
        fixture["started_at"] = json!((end - chrono::Duration::minutes(10)).to_rfc3339());
        fixture["ended_at"] = json!(end.to_rfc3339());
        fixture["stages"][0]["began_at"] = fixture["started_at"].clone();
        super::write_record(home.path(), number, &fixture);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let status = super::api::cli(home.path(), &["status", "--json"]);
    assert_eq!(
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["state"] == "live")
            .count(),
        cases.len()
    );
    for case in &cases {
        let subject = format!(
            "href=\"https://github.com/acme/widgets/issues/{}\"",
            case.number
        );
        let row = section(&html, case.group)
            .split("<li class=\"command\"")
            .find(|row| row.contains(&subject))
            .unwrap_or_else(|| panic!("{html}"));
        assert!(
            row.contains(&format!("data-state=\"{}\"", case.state)),
            "{row}"
        );
        assert_eq!(
            row.contains("data-dismiss>"),
            case.group == "needs you",
            "{row}"
        );
        let command = status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["issue"]["number"] == case.number)
            .unwrap();
        assert_eq!(command["group"], case.group, "{command}");
        assert_eq!(command["health"]["state"], case.health, "{command}");
    }
}

#[test]
fn overview_ticket_progress_counts_recorded_states_and_omits_missing_accounting() {
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 1, Some("failed"), 0);
    fixture["tickets"] = ["merged", "merged", "running", "failed", "waiting", "not started"]
        .into_iter()
        .enumerate()
        .map(|(index, state)| json!({
            "issue": {"number": index + 10, "url": format!("https://github.com/acme/widgets/issues/{}", index + 10), "title": "Ticket"},
            "state": state, "blockers": []
        }))
        .collect();
    // Merged accounting includes Tickets closed before any child Run existed.
    super::write_record(home.path(), 1, &fixture);
    for attempt in [20, 21] {
        let mut child = record(home.path(), attempt, Some("merged"), 0);
        child["kind"] = json!("Run");
        child["issue"]["number"] = json!(12);
        child["issue"]["url"] = json!("https://github.com/acme/widgets/issues/12");
        child["tickets"] = json!([]);
        child["parent_record"] = json!(
            home.path()
                .join(".thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json")
        );
        super::write_record(home.path(), attempt, &child);
    }
    let mut empty = record(home.path(), 2, Some("failed"), 0);
    empty["tickets"] = json!([]);
    super::write_record(home.path(), 2, &empty);
    let mut legacy = record(home.path(), 3, Some("failed"), 0);
    legacy.as_object_mut().unwrap().remove("tickets");
    super::write_record(home.path(), 3, &legacy);
    let mut ticket = record(home.path(), 4, Some("failed"), 0);
    ticket["kind"] = json!("Run");
    ticket["tickets"] = json!([]);
    super::write_record(home.path(), 4, &ticket);

    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/?section=overview").text().unwrap();
    let heads: Vec<_> = section(&html, "needs you")
        .split("<div class=\"row-head\">")
        .skip(1)
        .map(|head| head.split("</div>").next().unwrap())
        .collect();
    let spec = heads
        .iter()
        .find(|head| head.contains("issues/1\""))
        .unwrap();
    assert!(spec.contains("2 / 6 Tickets complete"), "{spec}");
    assert_eq!(
        html.matches("class=\"ticket-progress\"").count(),
        1,
        "{html}"
    );
    assert!(!html.contains("0 / 0"), "{html}");
}

#[test]
fn ready_rows_prefer_pull_request_subjects_and_keep_issue_and_command_fallbacks() {
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    for number in 1..=4 {
        let mut fixture = record(home.path(), number, Some("ready for review"), 0);
        fixture["name"] = json!(format!("review-fixture-{number}"));
        fixture["tickets"] = json!([]);
        if number == 1 || number == 4 {
            fixture["issue"] = Value::Null;
        }
        fixture["pull_request"] = if number <= 2 {
            json!(format!(
                "https://github.com/acme/widgets/pull/{}",
                number + 10
            ))
        } else {
            Value::Null
        };
        super::write_record(home.path(), number, &fixture);
    }
    fs::write(
        home.path().join(".thirdshift/logs/github-inbox.json"),
        serde_json::to_vec(&json!({
            "version": 1,
            "subjects": {
                "https://github.com/acme/widgets/pull/11": {"state": "Open", "title": "Review <script>unsafe()</script> & ship", "merged_at": null, "closed_at": null},
                "https://github.com/acme/widgets/pull/12": {"state": "Open", "title": "A distinct pull request title", "merged_at": null, "closed_at": null}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let ready = section(&html, "ready for review");
    let heads: Vec<_> = ready
        .split("<div class=\"row-head\">")
        .skip(1)
        .map(|head| head.split("</div>").next().unwrap())
        .collect();
    assert_eq!(heads.len(), 2, "{ready}");
    for (number, title) in [
        (
            11,
            "Review &#60;script&#62;unsafe()&#60;/script&#62; &#38; ship",
        ),
        (12, "A distinct pull request title"),
    ] {
        let head = heads
            .iter()
            .find(|head| {
                head.contains(&format!(
                    "href=\"https://github.com/acme/widgets/pull/{number}\""
                ))
            })
            .unwrap_or_else(|| panic!("primary PR link {number} missing: {ready}"));
        assert!(head.contains(title), "{head}");
        assert!(
            !head.contains("Widgets"),
            "PR must win over issue title: {head}"
        );
    }
    let done = section(&html, "done");
    assert!(
        done.contains("href=\"https://github.com/acme/widgets/issues/3\"")
            && done.contains("Widgets &#60;script&#62;"),
        "{done}"
    );
    assert!(done.contains("review-fixture-4 on acme/widgets"), "{done}");
    assert!(!ready.contains("<script>unsafe()"), "{ready}");
}

#[test]
fn a_live_review_run_links_its_pull_request_in_working() {
    let scenario = super::support::Scenario::new();
    scenario.user_config_is("[console]\nport = 0\n");
    scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
    let url = scenario.github_has_pr("colleague", "main", "OPEN");
    scenario.agent_waits_for_release("printf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let _held = super::HeldFixture {
        run: Some(scenario.run_until(&["review", &url, "--recommend-only"], &[], "started")),
        release: scenario.path("release"),
    };
    let console = Console::start(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        &scenario.path("home"),
    );

    let html = console.get("/").text().unwrap();
    let working = section(&html, "working");

    assert!(working.contains("data-live=\"true\""), "{working}");
    assert!(working.contains("class=\"working-target\""), "{working}");
    assert!(working.contains("PR #1</a>"), "{working}");
    assert!(working.contains(&format!("href=\"{url}\"")), "{working}");
}

#[test]
fn triage_and_review_commands_render_pages_and_cli_selectors() {
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let mut commands = Vec::new();
    for (number, prefix, kind, subject, session_kind) in [
        (1, "triage", "Triage run", "triage", "triage"),
        (2, "review", "Review run", "process-step-1", "code-review"),
    ] {
        let mut fixture = record(home.path(), number, Some("failed"), 0);
        let name = format!("{prefix}-20261019T110000-0400");
        fixture["kind"] = json!(kind);
        fixture["name"] = json!(name);
        fixture["issue"] = Value::Null;
        fixture["tickets"] = json!([]);
        let folder = home
            .path()
            .join(".thirdshift/logs/acme/widgets/commands")
            .join(prefix);
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("20261019T110000-0400~owner.command.json");
        fs::write(&path, serde_json::to_vec(&fixture).unwrap()).unwrap();
        fs::remove_file(home.path().join(format!(
            ".thirdshift/logs/acme/widgets/commands/issue/{number}-fixture.command.json"
        )))
        .unwrap();
        fs::write(
            folder.join("20261019T110000-0400~owner.log"),
            "Command output\n",
        )
        .unwrap();
        for suffix in ["", "-resume"] {
            super::support::session_trace::write(
                folder.join(format!(
                    "{subject}-20261019T110000-0400~owner-{session_kind}{suffix}.logs.jsonl"
                )),
                "Session output\n",
            );
        }
        commands.push((name, path));
    }
    let console = Console::start(binary, home.path());
    for page in ["/?page=inbox", "/?page=history", "/?page=logs"] {
        let response = console.get(page);
        assert_eq!(response.status(), 200, "{page}");
        let html = response.text().unwrap();
        for (name, _) in &commands {
            assert!(html.contains(name), "{page}: {html}");
        }
    }
    let run = |args: &[&str]| {
        let output = Command::new(binary)
            .args(args)
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["logs", "--json"]);
    for (name, path) in &commands {
        run(&["show", name, "--json"]);
        run(&["show", path.to_str().unwrap(), "--json"]);
        run(&["logs", name]);
        let session = if name.starts_with("triage-") {
            "triage"
        } else {
            "code-review"
        };
        run(&["logs", name, "--session", session]);
        let output = Command::new(binary)
            .args(["stop", name, "--json"])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Command ended"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(binary)
            .args(["dismiss", name, "--json"])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn paused_command_rows_and_details_offer_the_resume_command_and_reset() {
    use serde_json::json;
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 7, Some("paused"), 0);
    fixture["kind"] = json!("Scoring run");
    fixture["cause"] = json!(
        "Command paused at a plan-limit warning: Claude seven-day window at 86%, resets Fri Oct 30 22:00 UTC; run the Command again to resume"
    );
    fixture["resume_command"] = json!("thirdshift score");
    fixture["limit"] = json!({"harness":"claude","window":"seven_day","utilization":0.86,"resets_at":"2026-10-30T22:00:00Z"});
    super::write_record(home.path(), 7, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let inbox = console.get("/").text().unwrap();
    let needs = section(&inbox, "needs you");
    assert!(needs.contains(">Paused</span>"), "{inbox}");
    assert!(needs.contains("resets Fri Oct 30 22:00"), "{needs}");
    assert!(
        needs.contains("data-copy-cli=\"thirdshift score\""),
        "{needs}"
    );
    let details = console.get("/peek?command=widgets%237").text().unwrap();
    assert!(details.contains("Paused:"), "{details}");
    assert!(!details.contains("Failed:"), "{details}");
    assert!(details.contains("resets Fri Oct 30 22:00"), "{details}");
    assert!(
        details.contains("data-copy-cli=\"thirdshift score\""),
        "{details}"
    );
}

#[test]
fn ended_rows_distinguish_finish_age_from_run_duration() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let end = chrono::Utc::now() - chrono::Duration::minutes(52);
    let mut fixture = record(home.path(), 7, Some("merged"), 0);
    fixture["started_at"] = json!((end - chrono::Duration::minutes(140)).to_rfc3339());
    fixture["ended_at"] = json!(end.to_rfc3339());
    write_record(home.path(), 7, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    assert!(html.contains("52 min ago"), "{html}");
    assert!(
        html.contains(&format!("<time datetime=\"{}\">Ended ", end.to_rfc3339())),
        "{html}"
    );
    assert!(
        html.contains(&format!(
            "Ended {}",
            super::support::local_minute(&end.to_rfc3339())
        )),
        "{html}"
    );
    assert!(!html.contains("2 hr 20 min"), "{html}");
}

#[test]
fn unavailable_finish_times_are_not_invented_from_other_observations() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    for (number, outcome, end) in [
        (1, Some("merged"), Value::Null),
        (2, Some("failed"), json!("not a timestamp")),
        (3, None, Value::Null),
    ] {
        let mut fixture = record(home.path(), number, outcome, 0);
        fixture["ended_at"] = end;
        write_record(home.path(), number, &fixture);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for page in ["/", "/?page=history"] {
        let html = console.get(page).text().unwrap();
        assert_eq!(html.matches("End time unknown").count(), 3, "{html}");
        assert!(!html.contains("<time datetime="), "{html}");
        assert!(!html.contains("ago</span>"), "{html}");
    }
}

#[test]
fn finish_instants_survive_offsets_and_nested_history() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut outer = record(home.path(), 7, Some("merged"), 0);
    outer["kind"] = json!("Pickup run");
    outer["ended_at"] = json!("2026-10-23T19:03:18.964097412-04:00");
    let mut child = outer.clone();
    child["kind"] = json!("Run");
    child["name"] = json!("acme/widgets#8");
    child["issue"]["number"] = json!(8);
    child["ended_at"] = json!("2026-10-23T19:03:17.705023444-04:00");
    outer["dispatched"] = json!([child]);
    write_record(home.path(), 7, &outer);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/?page=history").text().unwrap();
    let outer_end = "2026-10-23T19:03:18.964097412-04:00";
    assert!(
        html.contains(&format!(
            "<time datetime=\"{outer_end}\">Ended {}",
            super::support::local_minute(outer_end)
        )),
        "{html}"
    );
    let nested = html.split("<ul class=\"children\">").nth(1).unwrap();
    let child_end = "2026-10-23T19:03:17.705023444-04:00";
    assert!(
        nested.contains(&format!(
            "<time datetime=\"{child_end}\">Ended {}",
            super::support::local_minute(child_end)
        )),
        "{nested}"
    );
    assert!(!nested.contains(outer_end), "{nested}");
}

#[test]
fn shared_ended_states_show_each_commands_finish() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let now = chrono::Utc::now();
    for (number, outcome, days) in [
        (1, "failed", 0),
        (2, "ready for review", 0),
        (3, "merged", 0),
        (4, "merged", 2),
    ] {
        // A slightly future finish (clock skew) must clamp to a recent zero age.
        let end = (now - chrono::Duration::days(days)
            + chrono::Duration::seconds(if days == 0 { 30 } else { 0 }))
        .to_rfc3339();
        let mut fixture = record(home.path(), number, Some(outcome), days);
        fixture["ended_at"] = json!(end);
        write_record(home.path(), number, &fixture);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    for group in ["needs you", "ready for review", "done", "resolved"] {
        let section = section(&html, group);
        assert!(section.contains("<time datetime="), "{group}: {section}");
        assert!(section.contains("</time> · </span>"), "{group}: {section}");
        assert!(section.contains(" ago</span>"), "{group}: {section}");
    }
    assert!(html.contains("0 sec ago"), "{html}");
    assert!(html.contains("48 hr 0 min ago"), "{html}");
}

#[test]
fn retained_activity_finishes_show_dates_and_missing_finishes_stay_unknown() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let root = home.path().join(".thirdshift/logs/acme/widgets");
    fs::create_dir_all(root.join("commands/issue")).unwrap();
    fs::write(
        root.join("commands/issue/8-20260101T230000-0400.log"),
        "Old Command output",
    )
    .unwrap();
    fs::write(root.join("activity.log"), "2026-01-01 23:00:00 Run #8 started: commands/issue/8-20260101T230000-0400.log\n2026-01-01 23:59:00 Run #8 ended: merged\n").unwrap();
    let session = root.join("commands/issue/9-20260102T120000+0000-implement.logs.jsonl");
    super::support::session_trace::write(&session, "Session without a Command record");
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/?page=history").text().unwrap();
    assert!(
        html.contains(&format!(
            "<time datetime=\"2026-01-01T23:59:00-04:00\">Ended {}",
            super::support::local_minute("2026-01-01T23:59:00-04:00")
        )),
        "{html}"
    );
    assert_eq!(html.matches("<time datetime=").count(), 1, "{html}");
    assert_eq!(html.matches("End time unknown").count(), 1, "{html}");
}

#[test]
fn live_rows_keep_start_age_without_claiming_to_have_ended() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = super::watching::live_record(home.path(), 7, std::process::id());
    fixture["started_at"] =
        serde_json::json!((chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339());
    super::write_record(home.path(), 7, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    assert!(html.contains("data-live=\"true\""), "{html}");
    assert!(html.contains("<span class=\"age\">5 min</span>"), "{html}");
    assert!(!html.contains("<time datetime="), "{html}");
    assert!(!html.contains("End time unknown"), "{html}");
    assert!(!html.contains("ago</span>"), "{html}");
}

#[test]
fn idle_inbox_explains_missing_activity_before_the_folded_sections() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let notice = html
        .split("aria-label=\"No running Commands\"")
        .nth(1)
        .expect(&html);
    assert!(
        notice.contains("No Commands are running on this machine."),
        "{html}"
    );
    assert!(
        notice.contains("No Pass is currently recorded as skipped."),
        "{html}"
    );
    let notice = notice.split("</section>").next().unwrap();
    assert!(!notice.contains("scheduler's output"), "{notice}");
    assert!(html.find("No running Commands").unwrap() < html.find("id=\"group-1\"").unwrap());
    for page in ["history", "security"] {
        let html = console.get(&format!("/?page={page}")).text().unwrap();
        assert!(!html.contains("No running Commands"), "{page}: {html}");
    }

    // Ended work does not make the machine active.
    record(home.path(), 1, Some("merged"), 0);
    let html = console.get("/").text().unwrap();
    assert!(html.contains("No running Commands"), "{html}");
}

#[test]
fn idle_inbox_surfaces_recorded_skip_reasons_and_waiting_work() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 1, Some("failed"), 0);
    record(home.path(), 2, Some("ready for review"), 0);
    let activity = home
        .path()
        .join(".thirdshift/logs/acme/widgets/activity.log");
    fs::write(&activity, "2026-01-01 12:00:00 Pickup run skipped: Claim limit reached <script>unsafe</script>\n2026-01-01 12:01:00 Architect run skipped: plan still open https://github.com/acme/widgets/issues/7\n").unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/").text().unwrap();
    let notice = html
        .split("aria-label=\"No running Commands\"")
        .nth(1)
        .expect(&html)
        .split("</section>")
        .next()
        .unwrap();
    assert!(notice.contains("Last recorded skipped Passes"), "{notice}");
    assert!(
        notice.contains("<li>2026-01-01 12:00 · <span>Pickup run idle on acme/widgets:"),
        "{notice}"
    );
    assert!(
        notice.contains("<li>2026-01-01 12:01 · <span>Architect run idle on acme/widgets:"),
        "{notice}"
    );
    let status = Command::new(binary)
        .args(["status", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["idle_passes"].as_array().unwrap().len(), 2);
    assert!(
        status["idle_passes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|pass| {
                pass["kind"] == "Pickup run" && pass["recorded_at"] == "2026-01-01T12:00:00"
            })
    );
    assert!(notice.contains("Pickup run idle on acme/widgets: Claim limit reached &#60;script&#62;unsafe&#60;/script&#62;"), "{notice}");
    assert!(
        notice.contains("Architect run idle on acme/widgets: plan still open"),
        "{notice}"
    );
    assert!(
        notice.contains("href=\"https://github.com/acme/widgets/issues/7\""),
        "{notice}"
    );
    assert!(!notice.contains("<script>unsafe</script>"));
    assert!(!notice.contains("No Pass is currently recorded as skipped."));
    // Needs you is its own folder, with its count; the notice doesn't repeat it.
    assert!(!notice.contains("Ended problems"), "{notice}");
    assert!(
        notice.contains("href=\"/?section=ready-for-review\""),
        "{notice}"
    );
    assert!(
        !html.contains("aria-label=\"Idle Passes\""),
        "skip reasons should appear only once"
    );
}

#[test]
fn idle_inbox_does_not_deny_skips_recorded_before_a_later_run() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let repo = home.path().join(".thirdshift/logs/acme/widgets");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("activity.log"), "2026-01-01 12:00:00 Pickup run skipped: no Ready issue on acme/widgets\n2026-01-01 12:01:00 Pickup run #7 started: commands/pickup/7-fixture.log, on codex\n2026-01-01 12:02:00 Pickup run #7 ended: merged\n").unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    assert!(
        html.contains("No Pass is currently recorded as skipped."),
        "{html}"
    );
    assert!(
        !html.contains("No skipped Pass reason has been recorded."),
        "{html}"
    );
}

#[test]
fn live_commands_never_show_the_idle_notice_even_in_needs_you_or_hidden_sections() {
    use super::{HeldFixture, support};
    use serde_json::json;
    use std::time::{Duration, SystemTime};

    let scenario = support::Scenario::new();
    scenario.user_config_is("[console]\nport = 0\nhidden_sections = [\"working\", \"landing\"]\n");
    scenario.agent_does(&format!(
        "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\nexit 1\n",
        scenario.path("started").display(),
        scenario.path("release").display()
    ));
    let _held = HeldFixture {
        run: Some(scenario.run_until(&[&scenario.issue_url(7)], &[], "started")),
        release: scenario.path("release"),
    };
    let dir = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    let path = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".command.json"))
        .unwrap();
    let mut fixture: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let console = Console::start(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        &scenario.path("home"),
    );
    for stage in ["implement", "CI watch"] {
        fixture["stages"][0]["name"] = json!(stage);
        fs::write(&path, serde_json::to_vec(&fixture).unwrap()).unwrap();
        let html = console.get("/").text().unwrap();
        assert!(html.contains("data-live=\"true\""), "{html}");
        assert!(!html.contains("No running Commands"), "{stage}: {html}");
    }
    // Silence moves the live Command to Needs you, leaving both active groups empty.
    fixture["stages"][0]["began_at"] =
        json!((chrono::Local::now() - chrono::Duration::minutes(20)).to_rfc3339());
    fs::write(&path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    for file in fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "log" || ext == "jsonl")
        })
    {
        fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(20 * 60)),
            )
            .unwrap();
    }
    let html = console.get("/").text().unwrap();
    assert!(html.contains("data-state=\"Silent\""), "{html}");
    assert!(!html.contains("No running Commands"), "{html}");
}

#[test]
fn landing_ready_and_stopping_rows_carry_their_state_in_the_console_and_status() {
    use super::watching::{FixtureProcess, live_record};
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let work = FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
    let pid = work.0.id();
    for (number, stage) in [(1, "CI watch"), (2, "implement"), (3, "implement")] {
        let mut fixture = live_record(home.path(), number, pid);
        fixture["stages"][0]["name"] = json!(stage);
        write_record(home.path(), number, &fixture);
    }
    record(home.path(), 4, Some("ready for review"), 0);
    record(home.path(), 5, Some("failed"), 0);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::write(
        dir.join("3-fixture.command.stop.json"),
        r#"{"who":"jacob","requeue":false}"#,
    )
    .unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/").text().unwrap();
    let status = |args: &[&str]| {
        let output = Command::new(binary)
            .args(args)
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let json: Value = serde_json::from_str(&status(&["status", "--json"])).unwrap();
    let text = status(&["status"]);
    for (number, group, state, live, stopping) in [
        (1, "landing", "Landing", true, false),
        (2, "working", "live", true, false),
        (3, "working", "Stopping", true, true),
        (4, "ready for review", "Ready", false, false),
        (5, "needs you", "Failed", false, false),
    ] {
        let subject_link = if group == "ready for review" {
            "href=\"https://github.com/acme/widgets/pull/10\"".into()
        } else {
            format!("href=\"https://github.com/acme/widgets/issues/{number}\"")
        };
        let row = html
            .split("<li class=\"command\"")
            .find(|row| row.contains(&subject_link))
            .expect(&html);
        assert!(row.contains(&format!("data-live=\"{live}\"")), "{row}");
        assert!(row.contains(&format!("data-state=\"{state}\"")), "{row}");
        assert!(
            row.contains(&format!("<span class=\"state\">{state}</span>")),
            "{row}"
        );
        let section = section(&html, group);
        assert!(section.contains(&subject_link), "{section}");
        let item = json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["issue"]["number"] == number)
            .unwrap();
        assert_eq!(item["group"], group, "{item}");
        assert_eq!(item["stopping"], stopping, "{item}");
        assert!(
            text.contains(&format!(
                "Spec run acme/widgets#{number} Widgets <script>alert(1)</script>"
            )) && text
                .lines()
                .any(|line| line.contains(&format!("acme/widgets#{number} "))
                    && line.contains(&format!(" · {state} · "))),
            "{text}"
        );
    }
}

#[test]
fn taken_up_rows_say_why_they_left_on_the_console_row() {
    use super::write_record;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 40, Some("failed"), 10);
    record(home.path(), 41, Some("merged"), 0);
    // The later Command took up issue 40, so the ten-day-old failure stays
    // in Resolved instead of going straight to History.
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/41-fixture.command.json");
    let mut later: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    later["issue"] = serde_json::json!({
        "number": 40,
        "url": "https://github.com/acme/widgets/issues/40",
        "title": "Widgets",
    });
    write_record(home.path(), 41, &later);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/").text().unwrap();
    let resolved = section(&html, "resolved");
    let issue_link = "href=\"https://github.com/acme/widgets/issues/40\"";
    assert!(resolved.contains(issue_link), "{resolved}");
    let row = resolved
        .split("<li class=\"command\"")
        .find(|row| row.contains(issue_link))
        .expect(resolved);
    assert!(
        row.contains("<p class=\"left\">left needs you: taken up by a later Command started at"),
        "{row}"
    );
    // The Console and status agree on the row's group and its reason.
    let status = Command::new(binary)
        .args(["status", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    let item = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["issue"]["number"] == 40 && item["group"] == "resolved")
        .unwrap();
    assert_eq!(item["left_reason"], "taken up by a later Command", "{item}");
}

#[test]
fn the_inbox_and_status_count_each_active_outer_command_once_whatever_its_group() {
    use super::watching::{FixtureProcess, live_record};
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".thirdshift")).unwrap();
    fs::write(
        home.path().join(".thirdshift/config.toml"),
        "[console]\nport = 0\nhidden_sections = [\"needs-you\", \"landing\", \"working\"]\n",
    )
    .unwrap();
    let mut work = FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
    let pid = work.0.id();
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    // A Spec run with two parallel Tickets, one of them Silent: Needs you.
    let spec = live_record(home.path(), 1, pid);
    write_record(home.path(), 1, &spec);
    let silent = (chrono::Local::now() - chrono::Duration::minutes(20)).to_rfc3339();
    for (number, began) in [(11, spec["started_at"].clone()), (12, json!(silent))] {
        let mut ticket = live_record(home.path(), number, pid);
        ticket["kind"] = json!("Run");
        ticket["tickets"] = json!([]);
        ticket["stages"][0]["began_at"] = began;
        ticket["parent_record"] = json!(dir.join("1-fixture.command.json"));
        fs::write(
            dir.join(format!("1-fixture.command.issue-{number}.child.json")),
            serde_json::to_vec(&ticket).unwrap(),
        )
        .unwrap();
        fs::remove_file(dir.join(format!("{number}-fixture.command.json"))).unwrap();
    }
    // An unrelated Landing Ticket run and a Stopping one.
    for (number, stage) in [(2, "CI watch"), (3, "implement")] {
        let mut run = live_record(home.path(), number, pid);
        run["kind"] = json!("Run");
        run["stages"][0]["name"] = json!(stage);
        write_record(home.path(), number, &run);
    }
    fs::write(
        dir.join("3-fixture.command.stop.json"),
        r#"{"who":"jacob","requeue":false}"#,
    )
    .unwrap();
    // Ended work, collapsed failures, ready review and a death add nothing.
    for number in [4, 5] {
        let mut failed = record(home.path(), number, Some("failed"), 0);
        failed["cause"] = json!("the same cause");
        write_record(home.path(), number, &failed);
    }
    record(home.path(), 6, Some("ready for review"), 0);
    let mut died = record(home.path(), 7, None, 0);
    died["process_id"] = json!(pid);
    write_record(home.path(), 7, &died);

    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let status = |args: &[&str]| {
        let output = Command::new(binary)
            .args(args)
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    };
    let summary = |html: &str| -> String {
        let after = html.split("class=\"active-runs\"").nth(1).expect(html);
        assert!(
            html.find("class=\"active-runs\"").unwrap() < html.find("id=\"group-1\"").unwrap(),
            "{html}"
        );
        after.split("</p>").next().unwrap().to_owned()
    };
    let html = console.get("/").text().unwrap();
    let json: Value = serde_json::from_str(&status(&["status", "--json"])).unwrap();
    assert_eq!(json["active_runs"], 3, "{json}");
    let groups: Vec<_> = json["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["issue"]["number"].as_u64().unwrap(),
                item["group"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(groups.contains(&(1, "needs you")), "{groups:?}");
    assert!(groups.contains(&(2, "landing")), "{groups:?}");
    assert!(summary(&html).contains("3 active runs"), "{html}");
    assert!(summary(&html).contains("Tickets"), "{html}");
    assert!(!html.contains("No running Commands"), "{html}");
    assert!(status(&["status"]).contains("3 active runs"));

    for number in [2, 3] {
        let mut ended = record(home.path(), number, Some("merged"), 0);
        ended["kind"] = json!("Run");
        write_record(home.path(), number, &ended);
    }
    let html = console.get("/").text().unwrap();
    assert!(summary(&html).contains("1 active run<"), "{html}");
    assert!(status(&["status"]).contains("1 active run\n"));

    work.0.kill().unwrap();
    work.0.wait().unwrap();
    let html = console.get("/").text().unwrap();
    assert!(summary(&html).contains("0 active runs"), "{html}");
    assert!(html.contains("No running Commands"), "{html}");
    let json: Value = serde_json::from_str(&status(&["status", "--json"])).unwrap();
    assert_eq!(json["active_runs"], 0);
}

#[test]
fn overview_shows_current_groups_and_section_urls_keep_their_filters() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 1, Some("failed"), 0);
    record(home.path(), 2, Some("merged"), 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let shown = |html: &str| -> Vec<String> {
        html.split("<details id=\"group-")
            .skip(1)
            .filter(|section| !section.split('>').next().unwrap().contains(" hidden"))
            .map(|section| {
                section
                    .split("data-group=\"")
                    .nth(1)
                    .unwrap()
                    .split('"')
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };

    // With no choice, Overview shows every group, even empty ones.
    let html = console.get("/").text().unwrap();
    let folders = html
        .split("aria-label=\"Inbox sections\"")
        .nth(1)
        .expect(&html)
        .split("</nav>")
        .next()
        .unwrap();
    for (group, slug) in [
        ("overview", "overview"),
        ("needs you", "needs-you"),
        ("ready for review", "ready-for-review"),
        ("landing", "landing"),
        ("working", "working"),
        ("done", "done"),
        ("resolved", "resolved"),
    ] {
        assert!(
            folders.contains(&format!("href=\"/?section={slug}\"")),
            "{group}: {folders}"
        );
    }
    assert!(
        folders
            .contains("href=\"/?section=overview\" data-group=\"overview\" aria-current=\"page\""),
        "{folders}"
    );
    let overview = [
        "needs you",
        "ready for review",
        "landing",
        "working",
        "done",
        "resolved",
    ];
    assert_eq!(shown(&html), overview, "{html}");
    let opens = |html: &str, group: &str| {
        section(html, group)
            .split('>')
            .next()
            .unwrap()
            .ends_with(" open")
    };
    // Done and Resolved follow Working, folded closed even without JavaScript.
    for (group, title, open) in [
        ("needs you", "Needs you", true),
        ("ready for review", "Ready for review", true),
        ("landing", "Landing", true),
        ("working", "Working", true),
        ("done", "Done", false),
        ("resolved", "Resolved", false),
    ] {
        assert!(
            section(&html, group).contains(&format!("<summary><h3>{title} ")),
            "{html}"
        );
        assert_eq!(opens(&html, group), open, "{group}: {html}");
    }
    assert!(
        section(&html, "resolved").contains("<p class=\"empty\">Nothing here.</p>"),
        "{html}"
    );

    // A section's own folder opens it.
    for group in ["done", "resolved"] {
        let html = console.get(&format!("/?section={group}")).text().unwrap();
        assert_eq!(shown(&html), [group], "{html}");
        assert!(opens(&html, group), "{html}");
    }
    let html = console.get("/?section=done").text().unwrap();
    assert!(
        html.contains("href=\"/?section=done\" data-group=\"done\" aria-current=\"page\""),
        "{html}"
    );

    // An unknown section falls back to the default.
    assert_eq!(
        shown(&console.get("/?section=nonsense").text().unwrap()),
        overview
    );

    // A section hidden with Sections keeps a hidden folder, for Sections to
    // show again, and is not shown when asked for.
    fs::write(
        home.path().join(".thirdshift/config.toml"),
        "[console]\nport = 0\nhidden_sections = [\"needs-you\"]\n",
    )
    .unwrap();
    let html = console.get("/?section=needs-you").text().unwrap();
    assert!(
        html.contains("href=\"/?section=needs-you\" data-group=\"needs you\" hidden>"),
        "{html}"
    );
    assert_eq!(
        shown(&html),
        ["ready for review", "landing", "working", "done", "resolved"],
        "{html}"
    );
}

#[test]
fn idle_inbox_folds_last_fired_passes_closed_on_load() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let root = home.path().join(".thirdshift/logs/acme/widgets");
    fs::create_dir_all(&root).unwrap();
    let fired = "2026-10-29T12:34:56-04:00";
    fs::write(root.join("pickup.last-fired"), fired).unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let notice = html
        .split("aria-label=\"No running Commands\"")
        .nth(1)
        .expect(&html)
        .split("</section>")
        .next()
        .unwrap();
    let starts = notice
        .split("<details id=\"pass-starts\"><summary>Last fired Passes:</summary>")
        .nth(1)
        .expect(notice)
        .split("</details>")
        .next()
        .unwrap();
    assert!(starts.contains("Pickup last fired"), "{starts}");
    assert!(starts.contains("on acme/widgets"), "{starts}");
    assert!(
        starts.contains(&format!(
            "<li><time datetime=\"{fired}\">{}",
            super::support::local_minute(fired)
        )),
        "{starts}"
    );
    assert!(starts.contains("</time> · Pickup last fired"), "{starts}");
}

#[test]
fn one_line_rows_carry_a_gist_of_outcome_and_what_a_title_less_pass_did() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut pass = record(home.path(), 1, Some("failed"), 0);
    pass["kind"] = json!("Pickup run");
    pass["name"] = json!("pickup-fixture");
    pass["issue"] = Value::Null;
    let mut dispatched = record(home.path(), 1, Some("failed"), 0);
    dispatched["kind"] = json!("Run");
    dispatched["name"] = json!("acme/widgets#48");
    dispatched["issue"]["number"] = json!(48);
    dispatched["issue"]["title"] = json!("Finish a readback");
    pass["dispatched"] = json!([dispatched]);
    write_record(home.path(), 1, &pass);
    let mut quiet = record(home.path(), 2, Some("failed"), 0);
    quiet["kind"] = json!("Pickup run");
    quiet["name"] = json!("pickup-quiet");
    quiet["issue"] = Value::Null;
    quiet["cause"] = json!("codex exited 1");
    write_record(home.path(), 2, &quiet);
    record(home.path(), 3, Some("failed"), 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let heads: Vec<&str> = section(&html, "needs you")
        .split("<div class=\"row-head\">")
        .skip(1)
        .map(|head| head.split("</div>").next().unwrap())
        .collect();
    let head = |subject: &str| {
        *heads
            .iter()
            .find(|head| head.contains(subject))
            .unwrap_or_else(|| panic!("{subject}: {heads:#?}"))
    };
    assert!(
        head("pickup-fixture on")
            .contains("<span class=\"gist\" title=\"failed · #48 Finish a readback\">failed · #48 Finish a readback</span>"),
        "{heads:#?}"
    );
    assert!(
        head("pickup-quiet on").contains(">failed · codex exited 1</span>"),
        "{heads:#?}"
    );
    assert!(
        head("acme/widgets#3").contains("<span class=\"gist\" title=\"failed\">failed</span>"),
        "{heads:#?}"
    );
}

#[test]
fn a_pause_pickup_retakes_sits_in_done_beside_the_plan_usage_banner() {
    use serde_json::json;
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut retaken = record(home.path(), 7, Some("paused"), 0);
    retaken["kind"] = json!("Run");
    retaken["handed_back_for_pickup"] = json!(true);
    retaken["cause"] = json!("no allowed route has available Plan usage: claude five-hour window");
    super::write_record(home.path(), 7, &retaken);
    let mut waiting = record(home.path(), 8, Some("paused"), 0);
    waiting["kind"] = json!("Run");
    super::write_record(home.path(), 8, &waiting);
    // The default Harness is installed, and held until its reset.
    let bin = home.path().join("console-fixture-bin");
    fs::create_dir_all(&bin).unwrap();
    super::support::write_executable(&bin.join("claude"), "#!/bin/sh\nexit 0\n");
    let now = chrono::Utc::now().to_rfc3339();
    fs::write(
        home.path().join(".thirdshift/plan-usage.json"),
        json!({"harnesses": {"claude": {"observed_at": now,
            "cooldown": {"since": now, "until": "2096-10-02T07:06:40Z", "window": "five_hour"}}}})
        .to_string(),
    )
    .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let inbox = console.get("/").text().unwrap();
    let needs = section(&inbox, "needs you");
    assert!(needs.contains("widgets#8"), "{needs}");
    assert!(!needs.contains("widgets#7"), "{needs}");
    assert!(section(&inbox, "done").contains("widgets#7"), "{inbox}");
    assert!(
        inbox.contains(
            "<p class=\"plan-usage-exhausted\" role=\"status\">Plan usage: every configured route \
             is unavailable, so scheduled Passes skip: claude in cooldown (five_hour) until \
             2096-10-02T07:06:40Z; earliest reset 2096-10-02T07:06:40Z</p>"
        ),
        "{inbox}"
    );
}
