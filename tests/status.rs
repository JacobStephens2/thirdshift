mod support;

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";
const OPENS_PR: &str = "echo work > work.txt\ngit add work.txt\ngit commit -qm 'Make widgets'\ngh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'\n";

#[test]
fn fake_gh_call_reads_wait_for_an_in_progress_record_write() {
    let scenario = Scenario::new();
    let lock = fs::File::create(scenario.path("gh-state.json.lock")).unwrap();
    lock.lock().unwrap();
    fs::write(scenario.path("gh-calls.json"), "").unwrap();
    std::thread::scope(|scope| {
        let scenario = &scenario;
        let (started, ready) = std::sync::mpsc::channel();
        let (finished, answer) = std::sync::mpsc::channel();
        scope.spawn(move || {
            started.send(()).unwrap();
            finished
                .send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || scenario.gh_calls(),
                )))
                .unwrap();
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let early = answer.recv_timeout(std::time::Duration::from_millis(100));
        fs::write(scenario.path("gh-calls.json"), r#"[["api","rate_limit"]]"#).unwrap();
        drop(lock);
        let result = early.unwrap_or_else(|_| {
            answer
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        });
        assert_eq!(
            result.expect("read a truncated call record during its write"),
            vec![vec!["api".to_string(), "rate_limit".to_string()]]
        );
    });
}

#[test]
fn ticket_run_display_preserves_the_stored_command_kind() {
    let scenario = Scenario::new();
    let now = chrono::Local::now().to_rfc3339();
    write_record(&scenario, 7, &now, Some(&now));
    let path = scenario.path(LOGS).join("7-fixture.command.json");
    let original = fs::read(&path).unwrap();
    let text = scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout.contains("Ticket run acme/widgets#7 Fixture"),
        "{}",
        text.stdout
    );
    let json = scenario.run(&["status", "--json"]);
    assert_eq!(json.code, Some(0), "{}", json.stderr);
    let status: Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(status["commands"][0]["kind"], "Run");
    let shown = scenario.run(&["show", path.to_str().unwrap()]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    assert!(shown.stdout.starts_with("Ticket run "), "{}", shown.stdout);
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn status_shows_each_idle_pass_until_its_own_kind_starts_work() {
    let scenario = Scenario::new();
    scenario.user_config_is("[activity]\nquiet_skips = true\n");
    for _ in 0..2 {
        let skipped = scenario.run(&["pickup"]);
        assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    }
    scenario.issue_titled(7, "Make widgets");
    scenario.issue_labelled(7, &["ready-for-agent"]);
    for kind in ["architect", "secure"] {
        let skipped = scenario.run(&[kind]);
        assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    }
    let result = scenario.run(&["status", "--json"]);
    let text = scenario.run(&["status"]);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["commands"], serde_json::json!([]));
    let idle = status["idle_passes"].as_array().expect(&result.stdout);
    assert_eq!(idle.len(), 3, "{status}");
    assert!(
        idle.iter().any(|pass| pass["kind"] == "Pickup run"
            && pass["reason"] == "no Ready issue on acme/widgets")
    );
    for kind in ["Architect run", "Security run"] {
        let reason =
            "Ready issue #7 \"Make widgets\" goes first: https://github.com/acme/widgets/issues/7";
        assert!(
            idle.iter().any(|pass| pass["repository"] == "acme/widgets"
                && pass["kind"] == kind
                && pass["reason"] == reason),
            "{status}"
        );
        assert!(
            text.stdout
                .contains(&format!("{kind} idle on acme/widgets: {reason}")),
            "{}",
            text.stdout
        );
    }
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&["pickup"]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let result = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    let idle = status["idle_passes"].as_array().unwrap();
    assert_eq!(idle.len(), 2, "{status}");
    assert!(!idle.iter().any(|pass| pass["kind"] == "Pickup run"));
    for flags in [
        ["status", "--json"].as_slice(),
        &["status", "--all", "--json"],
    ] {
        let result = scenario.run(flags);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let status: Value = serde_json::from_str(&result.stdout).unwrap();
        let commands = status["commands"].as_array().unwrap();
        assert_eq!(commands.len(), 2, "{status}");
        let pass = commands
            .iter()
            .find(|command| command["kind"] == "Pickup run")
            .unwrap();
        assert_eq!(pass["kind"], "Pickup run");
        let dispatched = &pass["dispatched"][0];
        assert_eq!(dispatched["issue"]["number"], 7);
        assert_eq!(dispatched["command_log"], pass["command_log"]);
        let ready = commands
            .iter()
            .find(|command| command["kind"] == "Run")
            .unwrap();
        assert_eq!(ready["group"], "ready for review");
        assert_eq!(ready["issue"]["number"], 7);
        assert_eq!(ready["command_log"], dispatched["command_log"]);
        for command in [pass, dispatched] {
            assert_eq!(command["source"], "command_record");
            assert_eq!(command["state"], "ended");
            assert_eq!(command["outcome"], "ready for review");
            assert!(
                chrono::DateTime::parse_from_rfc3339(command["ended_at"].as_str().unwrap()).is_ok()
            );
        }
    }
    let text = scenario.run(&["status"]);
    let dispatched = text
        .stdout
        .lines()
        .find(|line| line.starts_with("  Ticket run acme/widgets#7"))
        .expect(&text.stdout);
    assert!(dispatched.ends_with("ready for review"), "{dispatched}");
}

#[test]
fn status_lists_a_live_run_and_keeps_its_outcome_after_it_ends() {
    let scenario = Scenario::new();
    scenario.issue_titled(7, "Make widgets");
    scenario.agent_does_for(
        7,
        &format!(
            r#"touch '{started}'
while ! test -f '{release}'; do sleep 0.02; done
echo work > work.txt
git add work.txt
git commit -qm 'Make widgets'
gh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'
"#,
            started = scenario.path("started").display(),
            release = scenario.path("release").display()
        ),
    );
    let held = scenario.run_until(
        &[&scenario.issue_url(7), "model", "opus", "effort", "high"],
        &[],
        "started",
    );
    let recent = chrono::Local::now().to_rfc3339();
    write_record(&scenario, 8, &recent, Some(&recent));
    let result = scenario.run(&["status", "--json"]);
    let text = scenario.run(&["status"]);
    fs::remove_file(scenario.path(LOGS).join("8-fixture.command.json")).unwrap();
    fs::write(scenario.path("release"), "").unwrap();
    let ended = held.finish();
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["version"], 1);
    assert_eq!(status["commands"].as_array().unwrap().len(), 2);
    assert_eq!(status["commands"][1]["state"], "ended");
    assert!(!status["hostname"].as_str().unwrap().is_empty());
    let command = &status["commands"][0];
    assert_eq!(command["issue"]["number"], 7);
    assert_eq!(command["issue"]["title"], "Make widgets");
    assert_eq!(command["harness"], "claude");
    assert_eq!(command["model"], "opus");
    assert_eq!(command["effort"], "high");
    assert_eq!(command["state"], "live");
    assert_eq!(command["stage"]["name"], "implement");
    assert!(command["stage"]["age_seconds"].is_number());
    assert!(
        command["session_log"]
            .as_str()
            .unwrap()
            .ends_with("-implement.logs.jsonl")
    );
    assert!(text.stdout.contains("Make widgets"), "{}", text.stdout);
    assert!(text.stdout.contains("implement ·"), "{}", text.stdout);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let result = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["commands"][0]["state"], "ended");
    assert_eq!(status["commands"][0]["source"], "command_record");
    assert_eq!(status["commands"][0]["outcome"], "ready for review");
    assert!(status["commands"][0]["ended_at"].is_string());
    let record = scenario.command_record(LOGS);
    assert_eq!(record["outcome"], "ready for review");
    assert_eq!(record["cause"], Value::Null);
    assert_eq!(record["issue_branch"], "issue-7");
    assert_eq!(
        record["pull_request"],
        "https://github.com/acme/widgets/pull/1"
    );
}

#[test]
fn a_live_pickup_run_names_its_issue_and_the_run_it_dispatched() {
    let scenario = Scenario::new();
    scenario.issue_titled(7, "Make widgets");
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(&format!(
        "touch '{started}'\nwhile ! test -f '{release}'; do sleep 0.02; done\n{OPENS_PR}",
        started = scenario.path("started").display(),
        release = scenario.path("release").display()
    ));
    let held = scenario.run_until(&["pickup"], &[], "started");
    let json = scenario.run(&["status", "--json"]);
    let text = scenario.run(&["status"]);
    let status: Value = serde_json::from_str(&json.stdout).unwrap();
    let name = status["commands"][0]["name"].as_str().unwrap().to_string();
    let shown = scenario.run(&["show", &name]);
    fs::write(scenario.path("release"), "").unwrap();
    let ended = held.finish();
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let pickup = &status["commands"][0];
    assert_eq!(pickup["kind"], "Pickup run", "{status:#}");
    assert_eq!(pickup["state"], "live");
    assert_eq!(pickup["issue"]["title"], "Make widgets");
    assert_eq!(pickup["stage"]["name"], "dispatched Run");
    assert_eq!(pickup["stages"][0]["name"], "dispatched Run");
    assert!(
        chrono::DateTime::parse_from_rfc3339(pickup["stage"]["began_at"].as_str().unwrap()).is_ok()
    );
    assert!(pickup["stage"]["age_seconds"].is_number());
    assert_eq!(pickup["dispatched"][0]["stage"]["name"], "implement");
    let row = text
        .stdout
        .lines()
        .find(|line| line.starts_with("Pickup run "))
        .expect(&text.stdout);
    assert!(
        row.starts_with("Pickup run acme/widgets#7 Make widgets · "),
        "{row}"
    );
    assert!(row.contains(" · dispatched Ticket run · "), "{row}");
    assert!(
        text.stdout
            .contains("  Ticket run acme/widgets#7 Make widgets · "),
        "{}",
        text.stdout
    );
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    assert!(
        shown
            .stdout
            .starts_with(&format!("Pickup run {name} · dispatched Ticket run · ")),
        "{}",
        shown.stdout
    );
    assert!(
        shown.stdout.lines().any(|line| line == "Make widgets"),
        "{}",
        shown.stdout
    );
}

const PICKUP_LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/pickup";

/// The status line of a Pickup run whose record `change` rewrites, on a
/// Pickup run that took #7 and ended.
fn pickup_row_after(change: impl FnOnce(&mut Value)) -> String {
    let scenario = Scenario::new();
    scenario.issue_titled(7, "Make widgets");
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&["pickup"]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let path = fs::read_dir(scenario.path(PICKUP_LOGS))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".command.json"))
        .unwrap();
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    change(&mut record);
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let text = scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    text.stdout
        .lines()
        .find(|line| line.starts_with("Pickup run "))
        .expect(&text.stdout)
        .to_string()
}

/// Make an ended record read as a Command that died without an outcome.
fn died(record: &mut Value) {
    record["outcome"] = Value::Null;
    record["ended_at"] = Value::Null;
    record["process_start"] = "gone".into();
}

#[test]
fn an_ended_pickup_run_shows_its_outcome_and_one_that_died_names_its_dispatch() {
    let ended = pickup_row_after(|_| {});
    assert!(
        ended.starts_with("Pickup run acme/widgets#7 Make widgets · "),
        "{ended}"
    );
    assert!(ended.ends_with(" · ready for review"), "{ended}");
    let dead = pickup_row_after(died);
    assert!(dead.ends_with(" · died at dispatched Ticket run"), "{dead}");
}

#[test]
fn a_pickup_record_from_before_dispatch_stages_and_titles_reads_as_it_did() {
    let before = |record: &mut Value| {
        record["stages"] = serde_json::json!([]);
        record["issue"]["title"] = Value::Null;
    };
    let ended = pickup_row_after(before);
    assert!(
        ended.starts_with("Pickup run acme/widgets#7 (title unavailable) · "),
        "{ended}"
    );
    assert!(ended.ends_with(" · ready for review"), "{ended}");
    let dead = pickup_row_after(|record| {
        before(record);
        died(record);
    });
    assert!(dead.ends_with(" · died before first stage"), "{dead}");
}

#[test]
fn a_merge_run_retains_delivery_stages_and_their_start_times() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&["merge", &scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let record = scenario.command_record(LOGS);
    let stages = record["stages"].as_array().unwrap();
    let names: Vec<_> = stages
        .iter()
        .map(|stage| stage["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "implement",
            "pull request",
            "CI watch",
            "Self-merge",
            "cleanup"
        ]
    );
    for stage in stages {
        assert!(chrono::DateTime::parse_from_rfc3339(stage["began_at"].as_str().unwrap()).is_ok());
        assert!(
            stage["session_log"]
                .as_str()
                .unwrap()
                .ends_with("-implement.logs.jsonl")
        );
    }
    assert_eq!(record["outcome"], "merged");
    assert_eq!(record["issue_branch"], Value::Null);
}

#[test]
fn a_failed_run_retains_its_cause_and_does_not_stay_live() {
    let scenario = Scenario::new();
    scenario.agent_does("exit 3\n");
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    let record = scenario.command_record(LOGS);
    assert_eq!(record["outcome"], "failed");
    assert!(
        record["cause"]
            .as_str()
            .unwrap()
            .contains("claude exited 3")
    );
    assert_eq!(record["issue_branch"], Value::Null);
    assert_eq!(record["pull_request"], Value::Null);
    let result = scenario.run(&["status", "--json"]);
    assert_eq!(
        serde_json::from_str::<Value>(&result.stdout).unwrap()["commands"][0]["state"],
        "ended"
    );
}

fn write_record(scenario: &Scenario, number: u32, started: &str, ended: Option<&str>) {
    fs::create_dir_all(scenario.path(LOGS)).unwrap();
    let record = serde_json::json!({
        "version": 1, "kind": "Run", "repository": "acme/widgets",
        "issue": {"number": number, "url": scenario.issue_url(number), "title": "Fixture"},
        "hostname": "factory", "process_id": 0, "process_start": "gone",
        "harness": "claude", "model": null, "effort": null,
        "started_at": started, "ended_at": ended, "stages": [], "session_log": null,
        "outcome": ended.map(|_| "ready for review"), "cause": null,
        "issue_branch": null, "pull_request": null
    });
    fs::write(
        scenario
            .path(LOGS)
            .join(format!("{number}-fixture.command.json")),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

fn change_record(scenario: &Scenario, number: u32, change: impl FnOnce(&mut Value)) {
    let path = scenario
        .path(LOGS)
        .join(format!("{number}-fixture.command.json"));
    let mut record = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    change(&mut record);
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
}

#[test]
fn inbox_groups_ended_commands_and_adds_only_old_history_with_all() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    for (number, days, outcome) in [
        (7, 0, "failed"),
        (8, 0, "ready for review"),
        (9, 0, "merged"),
        (10, 2, "merged"),
        (11, 9, "failed"),
    ] {
        let time = (now - chrono::Duration::days(days)).to_rfc3339();
        write_record(&scenario, number, &time, Some(&time));
        change_record(&scenario, number, |record| {
            record["outcome"] = outcome.into();
            if number == 8 {
                record["pull_request"] = "https://github.com/acme/widgets/pull/8".into();
            }
        });
    }
    for (flags, expected) in [
        (
            vec!["status", "--json"],
            vec![
                (7, "needs you"),
                (8, "ready for review"),
                (9, "done"),
                (10, "resolved"),
            ],
        ),
        (
            vec!["status", "--all", "--json"],
            vec![
                (7, "needs you"),
                (8, "ready for review"),
                (9, "done"),
                (10, "resolved"),
                (11, "history"),
            ],
        ),
    ] {
        let output = scenario.run(&flags);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        let rows: Vec<_> = status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["issue"]["number"].as_u64().unwrap(),
                    row["group"].as_str().unwrap_or("missing"),
                )
            })
            .collect();
        assert_eq!(rows, expected, "{status}");
        assert_eq!(status["commands"][0]["needs_you_kind"], "failed");
    }
    let text = scenario.run(&["status", "--all"]);
    assert!(
        text.stdout
            .contains("https://github.com/acme/widgets/pull/8"),
        "{}",
        text.stdout
    );
    let headings: Vec<_> = text
        .stdout
        .lines()
        .filter(|line| line.ends_with(':'))
        .collect();
    assert_eq!(
        headings,
        [
            "Needs you:",
            "Ready for review:",
            "Done:",
            "Resolved:",
            "History:"
        ]
    );
}

#[test]
fn ready_commands_leave_ready_for_review_on_a_later_local_start_or_the_configured_age() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    // Record 7 is older than inbox.days; record 9 replaced record 8 on issue 8;
    // record 10 still waits.
    for (number, issue, hours) in [(7, 7, 8 * 24 + 1), (8, 8, 2), (9, 8, 1), (10, 10, 1)] {
        let time = (now - chrono::Duration::hours(hours)).to_rfc3339();
        write_record(&scenario, number, &time, Some(&time));
        change_record(&scenario, number, |record| {
            record["issue"]["number"] = issue.into();
            if number == 9 {
                record["outcome"] = "merged".into();
            } else {
                record["pull_request"] =
                    format!("https://github.com/acme/widgets/pull/{number}").into();
            }
        });
    }
    let groups = |flags: &[&str]| {
        let output = scenario.run(flags);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["pull_request"].as_str().unwrap_or("none").to_owned(),
                    row["group"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let pull = |number: u32| format!("https://github.com/acme/widgets/pull/{number}");
    assert_eq!(
        groups(&["status", "--json"]),
        [
            (pull(10), "ready for review".into()),
            ("none".into(), "done".into()),
            (pull(8), "done".into()),
        ]
    );
    assert_eq!(
        groups(&["status", "--all", "--json"]).last(),
        Some(&(pull(7), "history".into()))
    );
}

#[test]
fn inbox_clears_ended_problems_on_a_later_local_start_or_the_configured_age() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    for (number, days) in [(7, 2), (8, 1), (9, 8), (10, 6), (11, 6), (12, 0)] {
        let time = (now - chrono::Duration::days(days) - chrono::Duration::hours(1)).to_rfc3339();
        write_record(
            &scenario,
            number,
            &time,
            (number != 9).then_some(time.as_str()),
        );
        change_record(&scenario, number, |record| {
            if [7, 10, 12].contains(&number) {
                record["outcome"] = "failed".into();
            }
            if number == 8 {
                record["issue"] = serde_json::json!({"number": 7, "url": scenario.issue_url(7), "title": "Retry"});
                record["outcome"] = "merged".into();
            }
        });
    }
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    let problems: Vec<_> = commands
        .iter()
        .filter(|row| row["group"] == "needs you")
        .map(|row| row["issue"]["number"].as_u64().unwrap())
        .collect();
    assert_eq!(problems, [12, 10]);
    assert!(!commands.iter().any(|row| row["issue"]["number"] == 9));
    assert!(commands.iter().any(|row| row["issue"]["title"] == "Fixture"
        && row["issue"]["number"] == 7
        && row["group"] == "resolved"));
    scenario.user_config_is("[inbox]\ndays = 1\n");
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 1, "{status}");
    assert_eq!(commands[0]["issue"]["number"], 12);
    assert_eq!(commands[0]["group"], "needs you");
    assert_eq!(graphql_calls(&scenario).len(), 1);
    assert!(
        scenario
            .gh_calls()
            .iter()
            .all(|call| call[0] == "api" && matches!(call[1].as_str(), "graphql" | "rate_limit"))
    );
}

#[test]
fn resolved_counts_superseded_rows_from_the_later_command_start() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    // Second precision: the Inbox reprints the later start from its parsed
    // time, which drops fractional seconds.
    let stamp = |time: chrono::DateTime<chrono::Local>| {
        time.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
    };
    let days_ago = |days: i64| stamp(now - chrono::Duration::days(days));
    let old = days_ago(10);
    let later = stamp(now - chrono::Duration::hours(12));
    // Record 20 failed on issue 20 ten days ago; record 30 took issue 20
    // up twelve hours ago. Record 21 was ready on issue 21 ten days ago;
    // record 31 took issue 21 up twelve hours ago. Record 22 merged on
    // issue 22 ten days ago and record 32 took issue 22 up: it never
    // waited, so it still goes by when it ended. Record 23 failed ten days
    // ago with no later Command: it aged out. Record 24 merged two days
    // ago: never waited, still in view by its own end.
    for (number, issue, start, end) in [
        (20, 20, old.clone(), old.clone()),
        (21, 21, old.clone(), old.clone()),
        (22, 22, old.clone(), old.clone()),
        (23, 23, old.clone(), old.clone()),
        (24, 24, days_ago(2), days_ago(2)),
        (30, 20, later.clone(), later.clone()),
        (31, 21, later.clone(), later.clone()),
        (32, 22, later.clone(), later.clone()),
    ] {
        write_record(&scenario, number, &start, Some(&end));
        change_record(&scenario, number, |record| {
            record["issue"] = serde_json::json!({
                "number": issue,
                "url": scenario.issue_url(issue),
                "title": "Fixture",
            });
            match number {
                20 | 23 => record["outcome"] = "failed".into(),
                21 => {
                    record["outcome"] = "ready for review".into();
                    record["pull_request"] =
                        format!("https://github.com/acme/widgets/pull/{number}").into();
                }
                _ => record["outcome"] = "merged".into(),
            }
        });
    }
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    let rows: Vec<_> = commands
        .iter()
        .map(|row| {
            (
                row["issue"]["number"].as_u64().unwrap(),
                row["group"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    // The superseded failure and ready row stay in Resolved; the later
    // Commands are in Done; the never-waited superseded row and the aged
    // out failure are only in History.
    assert!(rows.contains(&(20, "resolved".to_string())), "{status}");
    assert!(rows.contains(&(21, "resolved".to_string())), "{status}");
    assert!(rows.contains(&(24, "resolved".to_string())), "{status}");
    assert!(rows.contains(&(20, "done".to_string())), "{status}");
    assert!(!rows.contains(&(22, "resolved".to_string())), "{status}");
    assert!(!rows.contains(&(23, "resolved".to_string())), "{status}");
    let row_in = |issue: u64, group: &str| {
        commands
            .iter()
            .find(|row| row["issue"]["number"] == issue && row["group"] == group)
            .unwrap()
            .clone()
    };
    for (issue, from) in [(20, "needs you"), (21, "ready for review")] {
        let row = row_in(issue, "resolved");
        assert_eq!(row["left_reason"], "taken up by a later Command", "{row}");
        assert_eq!(row["left_from"], from, "{row}");
        assert_eq!(row["left_at"], later.as_str(), "{row}");
    }
    let unmoved = row_in(24, "resolved");
    assert!(
        unmoved.get("left_reason").is_none_or(Value::is_null),
        "{unmoved}"
    );
    let all = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(all.code, Some(0), "{}", all.stderr);
    let status: Value = serde_json::from_str(&all.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    for issue in [22, 23] {
        let row = commands
            .iter()
            .find(|row| row["issue"]["number"] == issue && row["started_at"] == old.as_str())
            .unwrap();
        assert_eq!(row["group"], "history", "{row}");
        assert!(row.get("left_reason").is_none_or(Value::is_null), "{row}");
    }
    let text = scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout
            .contains("left needs you: taken up by a later Command started at"),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout
            .contains("left ready for review: taken up by a later Command started at"),
        "{}",
        text.stdout
    );
}

#[test]
fn a_row_superseded_while_still_live_carries_no_left_reason() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    let stamp = |time: chrono::DateTime<chrono::Local>| {
        time.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
    };
    let days_ago = |days: i64| stamp(now - chrono::Duration::days(days));
    // Record 50 started twelve days ago and ended failed two days ago; record
    // 51 started on issue 50 ten days ago, while 50 was still running, so 50
    // never waited in Needs you.
    write_record(&scenario, 50, &days_ago(12), Some(&days_ago(2)));
    change_record(&scenario, 50, |record| {
        record["outcome"] = "failed".into();
    });
    write_record(&scenario, 51, &days_ago(10), Some(&days_ago(10)));
    change_record(&scenario, 51, |record| {
        record["issue"] = serde_json::json!({
            "number": 50,
            "url": scenario.issue_url(50),
            "title": "Fixture",
        });
        record["outcome"] = "merged".into();
    });
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["issue"]["number"] == 50 && row["started_at"] == days_ago(12).as_str())
        .unwrap()
        .clone();
    assert_eq!(row["group"], "resolved", "{row}");
    assert!(row.get("left_reason").is_none_or(Value::is_null), "{row}");
}

#[test]
fn inbox_puts_live_problems_first_and_points_to_silent_landing_without_duplicating_it() {
    let scenario = Scenario::new();
    scenario.issue_titled(99, "Held fixture");
    scenario.issue_is(99, "OPEN");
    scenario.agent_does(&format!(
        "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\nexit 1\n",
        scenario.path("started").display(),
        scenario.path("release").display()
    ));
    let held = scenario.run_until(&[&scenario.issue_url(99)], &[], "started");
    let live = scenario.command_record(LOGS);
    let original = fs::read_dir(scenario.path(LOGS))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".command.json"))
        .unwrap();
    fs::remove_file(original).unwrap();
    let now = chrono::Local::now();
    for (number, age, stage) in [
        (7, 3600, "CI watch"),
        (8, 120, "implement"),
        (9, 1, "Self-merge"),
        (10, 1, "implement"),
    ] {
        let began = (now - chrono::Duration::seconds(age)).to_rfc3339();
        write_record(&scenario, number, &began, None);
        change_record(&scenario, number, |record| {
            record["process_id"] = live["process_id"].clone();
            record["process_start"] = live["process_start"].clone();
            record["stages"] =
                serde_json::json!([{"name": stage, "began_at": began, "session_log": null}]);
        });
    }
    for number in 20..23 {
        let ended = (now - chrono::Duration::hours(2)).to_rfc3339();
        let began = (now - chrono::Duration::hours(2) - chrono::Duration::minutes(1)).to_rfc3339();
        write_record(&scenario, number, &began, Some(&ended));
        change_record(&scenario, number, |record| {
            record["outcome"] = "merged".into();
            record["stages"] =
                serde_json::json!([{"name": "implement", "began_at": began, "session_log": null}]);
        });
    }
    let recent = now.to_rfc3339();
    write_record(&scenario, 11, &recent, None);
    write_record(&scenario, 12, &recent, Some(&recent));
    change_record(&scenario, 12, |record| record["outcome"] = "failed".into());
    for number in [7, 8] {
        github_issue_is(&scenario, number, "CLOSED", Some(&recent));
    }
    let output = scenario.run(&["status", "--json"]);
    let text = scenario.run(&["status"]);
    let reads_while_live = graphql_calls(&scenario);
    fs::write(scenario.path("release"), "").unwrap();
    held.finish();
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    let first: Vec<_> = commands
        .iter()
        .take(6)
        .map(|row| {
            (
                row["issue"]["number"].as_u64().unwrap(),
                row["group"].as_str().unwrap(),
                row["needs_you_kind"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        first,
        [
            (7, "needs you", Some("silent")),
            (8, "needs you", Some("slow")),
            (11, "needs you", Some("died")),
            (12, "needs you", Some("failed")),
            (9, "landing", None),
            (10, "working", None),
        ]
    );
    assert_eq!(commands[0]["landing"], true);
    assert_eq!(
        commands
            .iter()
            .filter(|row| row["issue"]["number"] == 7)
            .count(),
        1
    );
    assert!(
        text.stdout
            .contains("acme/widgets#7 · Silent · see Needs you"),
        "{}",
        text.stdout
    );
    assert_eq!(
        text.stdout
            .matches("Ticket run acme/widgets#7 Fixture")
            .count(),
        1,
        "{}",
        text.stdout
    );
    assert!(
        text.stdout
            .contains("Ticket run acme/widgets#10 Fixture · live ·"),
        "{}",
        text.stdout
    );
    for number in [7, 8] {
        assert!(
            reads_while_live
                .iter()
                .flatten()
                .any(|arg| arg.contains(&format!("issue(number: {number})"))),
            "{reads_while_live:?}"
        );
    }
    // The same closed-issue facts apply once these live Commands end.
    let output = scenario.run(&["status", "--json"]);
    let ended: Value = serde_json::from_str(&output.stdout).unwrap();
    for number in [7, 8] {
        let row = ended["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["issue"]["number"] == number)
            .unwrap();
        assert_eq!(row["group"], "resolved", "{row}");
        assert_eq!(row["left_reason"], "issue closed", "{row}");
    }
}

#[test]
fn inbox_distinguishes_triage_stops_and_waiting_findings_from_completed_work() {
    let scenario = Scenario::new();
    scenario.waiting_security_finding();
    scenario.issue_labelled(7, &["needs-triage"]);
    let time = chrono::Local::now().to_rfc3339();
    for number in 7..12 {
        write_record(&scenario, number, &time, Some(&time));
        change_record(&scenario, number, |record| {
            record["outcome"] = if number < 9 {
                "interrupted"
            } else {
                "findings recorded"
            }
            .into();
            record["security_findings_waiting"] = (number == 9).into();
            if number >= 9 {
                record["kind"] = "Security run".into();
            }
        });
        if number < 9 {
            let path = scenario
                .path(LOGS)
                .join(format!("{number}-fixture.command.stop.json"));
            fs::write(
                path,
                serde_json::to_vec(&serde_json::json!({"who": "jacob", "requeue": number == 8}))
                    .unwrap(),
            )
            .unwrap();
        }
    }
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    for (number, group, kind) in [
        (7, "needs you", Some("triage")),
        (8, "done", None),
        (9, "needs you", Some("finding")),
        (10, "done", None),
        (11, "done", None),
    ] {
        let row = commands
            .iter()
            .find(|row| row["issue"]["number"] == number)
            .unwrap();
        assert_eq!(row["group"], group, "{row}");
        assert_eq!(row["needs_you_kind"].as_str(), kind, "{row}");
    }
}

#[test]
fn inbox_collapses_only_failures_with_the_same_known_cause_and_keeps_their_commands() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    for (number, cause) in [
        (7, Some("base check failed")),
        (8, Some("base check failed")),
        (9, Some("agent failed")),
        (10, None),
        (11, None),
    ] {
        let time = (now - chrono::Duration::minutes(i64::from(number))).to_rfc3339();
        write_record(&scenario, number, &time, Some(&time));
        change_record(&scenario, number, |record| {
            record["outcome"] = "failed".into();
            record["cause"] = cause.into();
        });
    }
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 4, "{status}");
    assert_eq!(commands[0]["issue"]["number"], 7);
    assert_eq!(commands[0]["count"], 2);
    assert_eq!(commands[0]["collapsed"][0]["issue"]["number"], 8);
    assert_eq!(commands[0]["collapsed"][0]["group"], "needs you");
    for row in &commands[1..] {
        assert_eq!(row["count"], 1);
    }
    let text = scenario.run(&["status"]);
    assert!(
        text.stdout
            .contains("2 Commands with this cause: base check failed"),
        "{}",
        text.stdout
    );
    assert!(
        !text.stdout.contains("Ticket run acme/widgets#8 Fixture"),
        "{}",
        text.stdout
    );
}

#[test]
fn inbox_accepts_a_retention_period_beyond_the_representable_calendar() {
    let scenario = Scenario::new();
    scenario.user_config_is("[inbox]\ndays = 1000000000\n");
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"], serde_json::json!([]));
}

#[test]
fn tokenless_sessions_require_explicit_evidence_to_belong_to_an_owned_command() {
    for evidence in ["none", "record", "models"] {
        let scenario = Scenario::new();
        write_record(
            &scenario,
            7,
            "2026-10-03T12:00:01-04:00",
            Some("2026-10-03T13:00:00-04:00"),
        );
        let folder = scenario.path(LOGS);
        let record = folder.join("7-20261003T120000-0400~ownerA.command.json");
        fs::rename(folder.join("7-fixture.command.json"), &record).unwrap();
        let log = folder.join("7-20261003T120000-0400~ownerA.log");
        fs::write(&log, "owned Command output\n").unwrap();
        let session = folder.join("7-20261003T120000-0400-implement.logs.jsonl");
        support::session_trace::write(&session, "tokenless Session\n");
        match evidence {
            "record" => {
                let mut facts: Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
                facts["session_log"] = serde_json::json!(session);
                fs::write(&record, serde_json::to_vec(&facts).unwrap()).unwrap();
            }
            "models" => fs::write(
                session.with_file_name("7-20261003T120000-0400-implement.models.json"),
                serde_json::json!({"command_log": log}).to_string(),
            )
            .unwrap(),
            _ => {}
        }
        let status = scenario.run(&["status", "--all", "--json"]);
        assert_eq!(status.code, Some(0), "{}", status.stderr);
        let status: Value = serde_json::from_str(&status.stdout).unwrap();
        assert_eq!(
            status["commands"].as_array().unwrap().len(),
            if evidence == "none" { 2 } else { 1 },
            "{evidence}: {status}"
        );
        // The later record start keeps selection independent of equal-stamp
        // ordering while we test the inventory of the owned Command.
        let shown = scenario.run(&["show", "widgets#7", "--json"]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
        assert_eq!(shown["command"]["source"], "command_record");
        assert_eq!(
            shown["command"]["session_logs"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(session)),
            evidence != "none",
            "{evidence}: {shown}"
        );
    }
}

#[test]
fn status_shows_recent_outcomes_before_resolved_history() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    let recent = (now - chrono::Duration::hours(1)).to_rfc3339();
    let old = (now - chrono::Duration::days(2)).to_rfc3339();
    // The recent completion began before the old one: group by completion,
    // so Done precedes Resolved even when it started earlier.
    let earliest = (now - chrono::Duration::days(3)).to_rfc3339();
    write_record(&scenario, 7, &earliest, Some(&recent));
    write_record(&scenario, 8, &old, Some(&old));
    let result = scenario.run(&["status", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["commands"].as_array().unwrap().len(), 2);
    assert_eq!(status["commands"][0]["issue"]["number"], 7);
    assert_eq!(status["commands"][0]["outcome"], "ready for review");
    let text = scenario.run(&["status"]);
    assert!(text.stdout.contains("ready for review"), "{}", text.stdout);
    for flags in [["status", "--all", "--json"], ["status", "--json", "--all"]] {
        let result = scenario.run(&flags);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let status: Value = serde_json::from_str(&result.stdout).unwrap();
        let issues: Vec<_> = status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["issue"]["number"].as_u64().unwrap())
            .collect();
        assert_eq!(issues, [7, 8]);
    }
}

#[test]
fn status_keeps_completed_ticket_trees_with_recent_and_all_history() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    let old = (now - chrono::Duration::days(2)).to_rfc3339();
    let recent = (now - chrono::Duration::hours(1)).to_rfc3339();
    for (number, ended) in [(7, &old), (8, &recent)] {
        write_record(&scenario, number, &old, Some(ended));
        let path = scenario
            .path(LOGS)
            .join(format!("{number}-fixture.command.json"));
        let mut spec: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        spec["kind"] = "Spec run".into();
        spec["tickets"] = serde_json::json!([{
            "issue": {"number": 21, "url": scenario.issue_url(21), "title": "Ticket"},
            "state": "merged", "blockers": []
        }]);
        let mut child = spec.clone();
        child["kind"] = "Run".into();
        child["issue"] = spec["tickets"][0]["issue"].clone();
        child["tickets"] = serde_json::json!([]);
        child["parent_record"] = path.to_str().unwrap().into();
        child["ended_at"] = old.clone().into();
        child["outcome"] = "merged".into();
        child["stages"] = serde_json::json!([{
            "name": "cleanup", "began_at": old, "session_log": null
        }]);
        fs::write(&path, serde_json::to_vec(&spec).unwrap()).unwrap();
        fs::write(
            path.with_extension("issue-21.child.json"),
            serde_json::to_vec(&child).unwrap(),
        )
        .unwrap();
    }
    for (flags, expected) in [
        (vec!["status", "--json"], 2),
        (vec!["status", "--all", "--json"], 2),
    ] {
        let result = scenario.run(&flags);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let status: Value = serde_json::from_str(&result.stdout).unwrap();
        let commands = status["commands"].as_array().unwrap();
        assert_eq!(commands.len(), expected, "{status}");
        for spec in commands {
            assert_eq!(spec["state"], "ended");
            assert_eq!(spec["tickets"][0]["state"], "merged");
            let children = spec["children"].as_array().unwrap();
            assert_eq!(children.len(), 1, "{spec}");
            assert_eq!(children[0]["issue"]["number"], 21);
            assert_eq!(children[0]["state"], "ended");
            assert_eq!(children[0]["stage"]["name"], "cleanup");
            assert_eq!(children[0]["command_log"], spec["command_log"]);
        }
    }
    let text = scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout.contains("Ticket #21 · merged"),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout.contains("  Ticket run acme/widgets#21 Ticket"),
        "{}",
        text.stdout
    );
}

fn fixture_log(scenario: &Scenario, relative: &str, contents: &str) {
    let path = scenario
        .path("home/.thirdshift/logs/acme/widgets")
        .join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    if relative.ends_with(".logs.jsonl") {
        support::session_trace::write(path, contents);
    } else {
        fs::write(path, contents).unwrap();
    }
}

#[test]
fn status_reads_activity_starts_from_before_harness_metadata() {
    let scenario = Scenario::new();
    fixture_log(
        &scenario,
        "activity.log",
        concat!(
            "2026-10-04 12:00:00 Run #7 started: commands/issue/7-20261004T120000-0400.log\n",
            "2026-10-04 13:00:00 Run #7 ended: PR https://github.com/acme/widgets/pull/1 is ready for review\n"
        ),
    );
    fixture_log(
        &scenario,
        "commands/issue/7-20261004T120000-0400.log",
        "thirdshift: 13:00:00 PR https://github.com/acme/widgets/pull/1 is ready for review\n",
    );
    fixture_log(
        &scenario,
        "commands/issue/7-20261004T120000-0400-implement.logs.jsonl",
        "{}\n",
    );
    let result = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["commands"].as_array().unwrap().len(), 1);
    let command = &status["commands"][0];
    assert_eq!(command["source"], "activity_log");
    assert_eq!(command["started_at"], "2026-10-04T12:00:00-04:00");
    assert_eq!(command["ended_at"], "2026-10-04T13:00:00-04:00");
    assert_eq!(
        command["outcome"],
        "PR https://github.com/acme/widgets/pull/1 is ready for review"
    );
    assert_eq!(command["harness"], Value::Null);
    assert_eq!(command["model"], Value::Null);
    assert_eq!(command["effort"], Value::Null);
    assert_eq!(command["kind"], "Run");
    let text = scenario.run(&["status", "--all"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(text.stdout.contains("Ticket run"), "{}", text.stdout);
    assert!(!text.stdout.contains(" Run · "), "{}", text.stdout);
}

#[test]
fn status_matches_overlapping_failures_using_command_log_endings() {
    let scenario = Scenario::new();
    let first = chrono::Local::now() - chrono::Duration::hours(1);
    let second = first + chrono::Duration::minutes(1);
    let second_end = first + chrono::Duration::minutes(2);
    let first_end = first + chrono::Duration::minutes(3);
    let first_log = format!("commands/issue/7-{}.log", first.format("%Y%m%dT%H%M%S%z"));
    let second_log = format!("commands/issue/7-{}.log", second.format("%Y%m%dT%H%M%S%z"));
    // Identical causes require the Command log's ending time to distinguish
    // them. Recovery details are part of the Activity summary only.
    fixture_log(
        &scenario,
        "activity.log",
        &format!(
            "{} Run #7 started: {first_log}, on claude · default model · default effort\n{} Run #7 started: {second_log}, on claude · default model · default effort\n{} Run #7 ended: failed: claude exited 3; Stopped at: Implementation\n{} Run #7 ended: failed: claude exited 3\n",
            first.format("%Y-%m-%d %H:%M:%S"),
            second.format("%Y-%m-%d %H:%M:%S"),
            second_end.format("%Y-%m-%d %H:%M:%S"),
            first_end.format("%Y-%m-%d %H:%M:%S")
        ),
    );
    fixture_log(
        &scenario,
        &first_log,
        &format!(
            "thirdshift: {} claude exited 3\n",
            first_end.format("%H:%M:%S")
        ),
    );
    fixture_log(
        &scenario,
        &second_log,
        &format!(
            "thirdshift: {} Stopped at: Implementation\nthirdshift: {} claude exited 3\n",
            second_end.format("%H:%M:%S"),
            second_end.format("%H:%M:%S")
        ),
    );
    for flags in [vec!["status", "--all", "--json"], vec!["status", "--json"]] {
        let result = scenario.run(&flags);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let status: Value = serde_json::from_str(&result.stdout).unwrap();
        let commands = status["commands"].as_array().unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0]["outcome"],
            "failed: claude exited 3; Stopped at: Implementation"
        );
        assert_eq!(commands[1]["outcome"], "failed: claude exited 3");
        for command in commands {
            assert_eq!(command["source"], "activity_log");
            assert_eq!(command["state"], "ended");
            assert!(command["ended_at"].is_string());
        }
        // Activity times have second precision.
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(commands[0]["ended_at"].as_str().unwrap())
                .unwrap()
                .timestamp(),
            second_end.timestamp()
        );
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(commands[1]["ended_at"].as_str().unwrap())
                .unwrap()
                .timestamp(),
            first_end.timestamp()
        );
    }
}

#[test]
fn status_reads_activity_history_and_command_logs_without_stages() {
    let scenario = Scenario::new();
    fixture_log(
        &scenario,
        "activity.log",
        concat!(
            "2026-10-01 12:00:00 Run #7 started: commands/issue/7-20261001T120000-0400.log, on claude · opus · high\n",
            "2026-10-02 12:00:00 Run #7 started: commands/issue/7-20261002T120000-0400.log, on codex · default model · default effort\n",
            "2026-10-02 13:00:00 Run #7 ended: PR https://github.com/acme/widgets/pull/2 is ready for review\n",
            "2026-10-03 12:00:00 Run #7 ended: failed: claude exited 3\n",
            "2026-10-04 12:00:00 Security run started: commands/secure/20261004T120000-0400.log, on claude · default model · default effort\n",
            "2026-10-04 12:30:00 Security run ended: findings recorded\n",
            "2026-10-05 12:00:00 Pickup run skipped: no Ready issue on acme/widgets\n"
        ),
    );
    fixture_log(
        &scenario,
        "commands/issue/7-20261001T120000-0400.log",
        "thirdshift: 12:00:00 failed: claude exited 3\n",
    );
    fixture_log(
        &scenario,
        "commands/issue/7-20261002T120000-0400.log",
        "thirdshift: 13:00:00 PR https://github.com/acme/widgets/pull/2 is ready for review\n",
    );
    fixture_log(
        &scenario,
        "commands/secure/20261004T120000-0400.log",
        "thirdshift: 12:30:00 findings recorded\n",
    );
    // Keep the dated fixture inside the Inbox period, so Ready for review holds it.
    scenario.user_config_is("[inbox]\ndays = 36500\n");
    let result = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0]["group"], "ready for review");
    assert_eq!(commands[1]["kind"], "Security run");
    assert_eq!(commands[1]["issue"], Value::Null);
    assert_eq!(commands[1]["outcome"], "findings recorded");
    assert_eq!(commands[0]["started_at"], "2026-10-02T12:00:00-04:00");
    assert_eq!(commands[0]["ended_at"], "2026-10-02T13:00:00-04:00");
    assert_eq!(
        commands[0]["outcome"],
        "PR https://github.com/acme/widgets/pull/2 is ready for review"
    );
    assert_eq!(commands[2]["ended_at"], "2026-10-03T12:00:00-04:00");
    assert_eq!(commands[2]["outcome"], "failed: claude exited 3");
    assert_eq!(commands[2]["harness"], "claude");
    assert_eq!(commands[2]["model"], "opus");
    for command in commands {
        assert_eq!(command["source"], "activity_log");
        assert_eq!(command["state"], "ended");
        assert_eq!(command["stages"], serde_json::json!([]));
        assert_eq!(command["stage"], Value::Null);
        assert!(command["command_log"].as_str().unwrap().ends_with(".log"));
    }
    let text = scenario.run(&["status", "--all"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(text.stdout.contains("findings recorded"), "{}", text.stdout);
    assert!(
        text.stdout.contains("2026-10-02T13:00:00-04:00"),
        "{}",
        text.stdout
    );
}

#[test]
fn status_fills_gaps_by_source_and_groups_only_current_layout_sessions() {
    let scenario = Scenario::new();
    write_record(
        &scenario,
        7,
        "2026-10-01T12:00:00-04:00",
        Some("2026-10-01T13:00:00-04:00"),
    );
    fs::rename(
        scenario.path(LOGS).join("7-fixture.command.json"),
        scenario
            .path(LOGS)
            .join("7-20261001T120000-0400.123.command.json"),
    )
    .unwrap();
    fixture_log(
        &scenario,
        "activity.log",
        concat!(
            "2026-10-01 12:00:00 Run #7 started: commands/issue/7-20261001T120000-0400.log, on claude · default model · default effort\n",
            "2026-10-01 13:00:00 Run #7 ended: PR https://github.com/acme/widgets/pull/1 is ready for review\n",
            "2026-10-02 12:00:00 Spec run #8 started: commands/issue/8-20261002T120000-0400.log, on claude · default model · default effort\n",
            "2026-10-02 13:00:00 Spec run #8 ended: failed: Ticket #10 failed\n"
        ),
    );
    for name in [
        "7-20261001T120000-0400-implement.logs.jsonl",
        "8-20261002T120000-0400-spec-review.logs.jsonl",
        "10-20261002T120000-0400-implement.logs.jsonl", // Ticket belongs to Spec #8.
        "9-20260901T120000-0400-implement.logs.jsonl",
        "9-20260901T120000-0400-implement-resume.logs.jsonl",
        "9-20260901T120000-0400-repair-1.logs.jsonl",
        "9-20260902T120000-0400-implement.logs.jsonl",
    ] {
        fixture_log(&scenario, &format!("commands/issue/{name}"), "{}\n");
    }
    fixture_log(
        &scenario,
        "commands/issue/9-20260901T120000-0400-implement.models.json",
        "{}",
    );
    fixture_log(&scenario, "commands/issue/bad-name.jsonl", "{}");
    for relative in [
        "sessions/widgets-4-20260801T120000-0400-implement.logs.jsonl",
        "commands/issue/5-20260801T120000-0400-implement.logs.jsonl",
    ] {
        let path = scenario.path("home/.thirdshift/logs").join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "{}\n").unwrap();
    }
    let result = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 4, "{commands:?}");
    assert_eq!(commands[0]["source"], "activity_log");
    assert_eq!(commands[0]["issue"]["number"], 8);
    assert_eq!(commands[1]["source"], "command_record");
    assert_eq!(commands[1]["outcome"], "ready for review");
    assert_eq!(commands[2]["started_at"], "2026-09-02T12:00:00-04:00");
    assert_eq!(commands[3]["session_logs"].as_array().unwrap().len(), 3);
    for command in &commands[2..] {
        assert_eq!(command["source"], "session_log");
        assert_eq!(command["issue"]["number"], 9);
        assert_eq!(command["outcome"], Value::Null);
        assert_eq!(command["ended_at"], Value::Null);
        assert_eq!(command["state"], "unknown");
    }
    let recent = scenario.run(&["status", "--json"]);
    assert_eq!(recent.code, Some(0), "{}", recent.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&recent.stdout).unwrap()["commands"],
        serde_json::json!([])
    );
}

#[test]
fn status_recovers_a_real_commands_logs_after_newer_sources_are_removed() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    for name in scenario
        .entries(LOGS)
        .into_iter()
        .filter(|name| name.ends_with(".command.json"))
    {
        fs::remove_file(scenario.path(LOGS).join(name)).unwrap();
    }
    let result = scenario.run(&["status", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["commands"].as_array().unwrap().len(), 1);
    assert_eq!(status["commands"][0]["source"], "activity_log");
    assert_eq!(
        status["commands"][0]["outcome"],
        "PR https://github.com/acme/widgets/pull/1 is ready for review"
    );
    fs::remove_file(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log")).unwrap();
    let result = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["commands"].as_array().unwrap().len(), 1);
    assert_eq!(status["commands"][0]["source"], "session_log");
    assert_eq!(status["commands"][0]["outcome"], Value::Null);
}

#[test]
fn failed_acquisition_does_not_claim_a_branch_was_left() {
    let scenario = Scenario::new();
    fs::create_dir(scenario.path("work/widgets-issue-7")).unwrap();
    fs::write(scenario.path("work/widgets-issue-7/manual.txt"), "keep me").unwrap();
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    assert_eq!(scenario.origin_log("issue-7"), None);
    let local = std::process::Command::new("git")
        .arg("-C")
        .arg(scenario.launch_dir())
        .args(["show-ref", "--verify", "refs/heads/issue-7"])
        .output()
        .unwrap();
    assert!(!local.status.success());
    assert_eq!(
        scenario.command_record(LOGS)["issue_branch"],
        Value::Null,
        "{}",
        ended.stderr
    );
}

#[test]
fn refused_remote_deletion_keeps_branch_in_record() {
    let scenario = Scenario::new();
    scenario.repo_has_hook(&scenario.origin_dir(), "pre-receive",
        "#!/bin/sh\nwhile read old new ref; do\n  if [ \"$new\" = 0000000000000000000000000000000000000000 ]; then exit 1; fi\ndone\n");
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&["merge", &scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    assert!(scenario.origin_log("issue-7").is_some(), "{}", ended.stderr);
    assert_eq!(
        scenario.command_record(LOGS)["issue_branch"],
        "issue-7",
        "{}",
        ended.stderr
    );
}

#[test]
fn failed_continuation_acquisition_keeps_existing_remote_branch() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-7", "main", &["Prior work"]);
    fs::create_dir(scenario.path("work/widgets-issue-7")).unwrap();
    fs::write(scenario.path("work/widgets-issue-7/manual.txt"), "keep me").unwrap();
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    assert!(scenario.origin_log("issue-7").is_some(), "{}", ended.stderr);
    assert_eq!(
        scenario.command_record(LOGS)["issue_branch"],
        "issue-7",
        "{}",
        ended.stderr
    );
}

#[test]
fn a_killed_run_shows_as_died_at_its_last_stage() {
    let scenario = Scenario::new();
    scenario.agent_does_for(
        7,
        &format!(
            "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\n",
            scenario.path("started").display(),
            scenario.path("release").display()
        ),
    );
    let mut held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
    held.kill();
    let result = scenario.run(&["status", "--json"]);
    let text = scenario.run(&["status"]);
    fs::write(scenario.path("release"), "").unwrap();
    held.finish();
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["commands"][0]["state"], "died");
    assert_eq!(status["commands"][0]["stage"]["name"], "implement");
    assert!(text.stdout.contains("died at implement"), "{}", text.stdout);
    assert_eq!(scenario.command_record(LOGS)["outcome"], Value::Null);
}

#[test]
fn status_reads_all_repositories_under_configured_logs_and_ignores_unknown_stages() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let mut record = scenario.command_record(LOGS);
    record["outcome"] = Value::Null;
    record["process_start"] = "different process start".into();
    record["stages"] = serde_json::json!([
        {"name": "CI watch", "began_at": (chrono::Local::now() - chrono::Duration::minutes(52)).to_rfc3339(), "session_log": null},
        {"name": "future stage", "began_at": chrono::Local::now().to_rfc3339(), "session_log": null}
    ]);
    let root = scenario.path("custom-logs");
    for (owner, repo, issue) in [("acme", "widgets", 7), ("other", "gadgets", 9)] {
        let folder = root.join(owner).join(repo).join("commands/issue");
        fs::create_dir_all(&folder).unwrap();
        record["repository"] = format!("{owner}/{repo}").into();
        record["issue"]["number"] = issue.into();
        fs::write(
            folder.join("fixture.command.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }
    scenario.user_config_is(&format!("[logs]\ndir = '{}'\n", root.display()));
    let result = scenario.run_from(&scenario.path("home"), &["status", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0]["repository"], "acme/widgets");
    assert_eq!(commands[1]["repository"], "other/gadgets");
    for command in commands {
        assert_eq!(command["state"], "died");
        assert_eq!(command["stage"]["name"], "CI watch");
        assert!(command["stage"]["age_seconds"].as_i64().unwrap() >= 3120);
    }
}

#[test]
fn status_names_a_foreign_blocker_without_making_a_ticket_look_self_blocked() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let mut record = scenario.command_record(LOGS);
    record["kind"] = serde_json::json!("Spec run");
    record["outcome"] = Value::Null;
    record["tickets"] = serde_json::json!([{
        "issue": {
            "number": 21,
            "url": "https://github.com/acme/widgets/issues/21",
            "title": null
        },
        "state": "waiting",
        "blockers": [{
            "number": 21,
            "url": "https://github.com/other/widgets/issues/21",
            "title": null
        }]
    }]);
    let name = scenario
        .entries(LOGS)
        .into_iter()
        .find(|name| name.ends_with(".command.json"))
        .unwrap();
    fs::write(
        scenario.path(LOGS).join(name),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let text = scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout
            .contains("https://github.com/other/widgets/issues/21")
            || text.stdout.contains("other/widgets#21"),
        "{}",
        text.stdout
    );
}

#[test]
fn status_keeps_children_under_their_own_security_dispatch() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let mut pass = scenario.command_record(LOGS);
    let mut dispatched = Vec::new();
    for number in [20, 30] {
        let mut run = pass.clone();
        run["issue"]["number"] = number.into();
        run["issue"]["url"] = scenario.issue_url(number).into();
        run["outcome"] = if number == 20 {
            "ready for review".into()
        } else {
            Value::Null
        };
        let mut child = pass.clone();
        child["kind"] = "Base fix".into();
        child["issue"]["number"] = 42.into();
        child["issue"]["url"] = scenario.issue_url(42).into();
        child["parent_record"] = scenario
            .path(LOGS)
            .join("pass.command.json")
            .to_str()
            .unwrap()
            .into();
        child["parent_dispatch"] = scenario.issue_url(number).into();
        child["outcome"] = if number == 20 {
            "merged".into()
        } else {
            Value::Null
        };
        fs::write(
            scenario
                .path(LOGS)
                .join(format!("dispatch-{number}.child.json")),
            serde_json::to_vec(&child).unwrap(),
        )
        .unwrap();
        dispatched.push(run);
    }
    pass["kind"] = "Security run".into();
    pass["issue"] = Value::Null;
    pass["outcome"] = Value::Null;
    pass["dispatched"] = dispatched.into();
    fs::write(
        scenario.path(LOGS).join("pass.command.json"),
        serde_json::to_vec(&pass).unwrap(),
    )
    .unwrap();
    let result = scenario.run(&["status", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 3);
    let ready = commands
        .iter()
        .find(|command| command["kind"] == "Run" && command["issue"]["number"] == 20)
        .unwrap();
    assert_eq!(ready["group"], "ready for review");
    assert_eq!(ready["children"].as_array().unwrap().len(), 1);
    assert_eq!(ready["children"][0]["issue"]["number"], 42);
    assert_eq!(ready["children"][0]["state"], "ended");
    let pass = commands
        .iter()
        .find(|command| command["kind"] == "Security run")
        .unwrap();
    assert_eq!(pass["children"], serde_json::json!([]));
    for (index, state) in ["ended", "died"].into_iter().enumerate() {
        let run = &pass["dispatched"][index];
        assert_eq!(run["children"].as_array().unwrap().len(), 1, "{status}");
        assert_eq!(run["children"][0]["issue"]["number"], 42);
        assert_eq!(run["children"][0]["state"], state);
    }
}

// Issue #1194: Ready for review learns from GitHub. These tests run the
// `thirdshift` binary against the fake `gh`, with ages set by fixture
// timestamps and by the kept answer's file time.

const GITHUB_CACHE: &str = "home/.thirdshift/logs/github-inbox.json";

#[test]
fn review_open_pr_row_exposes_when_github_observed_it() {
    fn has_time(value: &Value) -> bool {
        match value {
            Value::String(text) => chrono::DateTime::parse_from_rfc3339(text).is_ok(),
            Value::Array(values) => values.iter().any(has_time),
            Value::Object(values) => values.values().any(has_time),
            _ => false,
        }
    }
    let scenario = Scenario::new();
    ready_record_with_pr(&scenario, 7, &hours_ago(240), Some(7));
    github_pr_is(&scenario, 7, "OPEN", None, None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "ready for review", "{status}");
    assert_eq!(row["github_state"], "open", "{row}");
    assert!(
        row.as_object()
            .unwrap()
            .iter()
            .any(|(key, value)| { key.starts_with("github") && has_time(value) }),
        "each row must expose when GitHub observed its facts: {row}"
    );
}

#[test]
fn review_github_settlement_does_not_restart_dismissal_retention() {
    let scenario = Scenario::new();
    failed_record(&scenario, 7, &hours_ago(240));
    github_issue_is(&scenario, 7, "OPEN", None);
    let first = scenario.run(&["status", "--json"]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let dismissed = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    let sidecar = scenario.path(LOGS).join("7-fixture.command.dismiss.json");
    let set_dismissal_age = |hours| {
        let mut file: Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        file["rows"][0]["at"] = hours_ago(hours).into();
        fs::write(&sidecar, serde_json::to_vec(&file).unwrap()).unwrap();
    };
    set_dismissal_age(144);
    github_issue_is(&scenario, 7, "CLOSED", Some(&hours_ago(1)));
    age_cache_file(&scenario, 360);
    let settled = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(settled.code, Some(0), "{}", settled.stderr);
    let settled: Value = serde_json::from_str(&settled.stdout).unwrap();
    assert_eq!(settled["commands"][0]["group"], "resolved", "{settled}");
    // Advance the dismissal fixture past seven days; the GitHub closure
    // was learned while the dismissal was still retained.
    set_dismissal_age(192);
    let expired = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(expired.code, Some(0), "{}", expired.stderr);
    let expired: Value = serde_json::from_str(&expired.stdout).unwrap();
    assert_eq!(expired["commands"][0]["group"], "history", "{expired}");
    assert!(expired["commands"][0]["dismissal"].is_object(), "{expired}");
}

#[test]
fn review_later_start_after_end_is_found_despite_an_earlier_overlap() {
    let scenario = Scenario::new();
    let old_start = hours_ago(288);
    let old_end = hours_ago(240);
    let overlap = hours_ago(264);
    let taken = hours_ago(12);
    for (number, start, end, outcome) in [
        (7, old_start.as_str(), old_end.as_str(), "failed"),
        (8, overlap.as_str(), overlap.as_str(), "merged"),
        (9, taken.as_str(), taken.as_str(), "merged"),
    ] {
        write_record(&scenario, number, start, Some(end));
        change_record(&scenario, number, |record| {
            record["issue"] = serde_json::json!({
                "number": 7, "url": scenario.issue_url(7), "title": "Fixture"
            });
            record["outcome"] = outcome.into();
        });
    }
    github_issue_is(&scenario, 7, "OPEN", None);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["started_at"] == old_start)
        .unwrap();
    assert_eq!(row["group"], "resolved", "{status}");
    assert_eq!(row["left_reason"], "taken up by a later Command", "{row}");
    assert_eq!(row["left_at"], taken, "{row}");
}

fn github_pr_url(number: u64) -> String {
    format!("https://github.com/acme/widgets/pull/{number}")
}

fn ready_record_with_pr(scenario: &Scenario, number: u32, ended: &str, pr: Option<u64>) {
    write_record(scenario, number, ended, Some(ended));
    change_record(scenario, number, |record| {
        record["issue"] = serde_json::json!({
            "number": number,
            "url": scenario.issue_url(number),
            "title": "Fixture",
        });
        if let Some(pr) = pr {
            record["pull_request"] = github_pr_url(pr).into();
        }
    });
}

fn github_pr_is(
    scenario: &Scenario,
    number: u64,
    state: &str,
    merged_at: Option<&str>,
    closed_at: Option<&str>,
) {
    let mut gh = scenario.gh_state();
    let prs = gh["prs"].as_array_mut().unwrap();
    let mut pr = serde_json::json!({
        "number": number,
        "url": github_pr_url(number),
        "head": format!("issue-{number}"),
        "base": "main",
        "state": state,
        "isDraft": false,
        "title": "Fixture",
        "body": "",
    });
    if let Some(at) = merged_at {
        pr["mergedAt"] = at.into();
    }
    if let Some(at) = closed_at {
        pr["closedAt"] = at.into();
    }
    if let Some(existing) = prs.iter_mut().find(|pr| pr["url"] == github_pr_url(number)) {
        *existing = pr;
    } else {
        prs.push(pr);
    }
    scenario.write_gh_state(&gh);
}

fn graphql_calls(scenario: &Scenario) -> Vec<Vec<String>> {
    scenario.gh_calls_of("api", "graphql")
}

fn age_cache_file(scenario: &Scenario, seconds: u64) {
    let file = fs::File::options()
        .write(true)
        .open(scenario.path(GITHUB_CACHE))
        .unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(seconds))
        .unwrap();
    if let Ok(freshness) = fs::File::options()
        .write(true)
        .open(scenario.path(GITHUB_CACHE).with_extension("refreshed"))
    {
        freshness
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(seconds))
            .unwrap();
    }
}

fn hours_ago(hours: i64) -> String {
    (chrono::Local::now() - chrono::Duration::hours(hours)).to_rfc3339()
}

fn failed_record(scenario: &Scenario, number: u32, ended: &str) {
    write_record(scenario, number, ended, Some(ended));
    change_record(scenario, number, |record| {
        record["outcome"] = "failed".into()
    });
}

fn github_issue_is(scenario: &Scenario, number: u32, state: &str, closed_at: Option<&str>) {
    scenario.issue_is(number, state);
    let mut gh = scenario.gh_state();
    gh["issue_closed_at"][number.to_string()] = closed_at.into();
    scenario.write_gh_state(&gh);
}

fn security_record(scenario: &Scenario, number: u32, urls: &[&str], failed: bool) {
    failed_record(scenario, number, &hours_ago(if failed { 48 } else { 240 }));
    change_record(scenario, number, |record| {
        record["kind"] = "Security run".into();
        record["issue"] = Value::Null;
        record["security_findings_waiting"] = true.into();
        record["outcome"] = if failed {
            "failed"
        } else {
            "findings recorded"
        }
        .into();
        record["security_findings"] = urls
            .iter()
            .map(|url| {
                serde_json::json!({
                    "url": url, "title": "Private title", "severity": null
                })
            })
            .collect();
    });
}

#[test]
fn finding_leaves_only_after_its_last_private_record_is_triaged() {
    let scenario = Scenario::new();
    let first = "https://github.com/acme/widgets/security/advisories/GHSA-first";
    let second = "https://github.com/acme/widgets/security/advisories/GHSA-second";
    security_record(&scenario, 7, &[first, second], false);
    let mut gh = scenario.gh_state();
    gh["advisories"] = serde_json::json!([
        {"ghsa_id":"GHSA-first", "html_url":first, "state":"draft", "severity":null,
         "summary":"Secret title", "description":"Secret write-up"},
        {"ghsa_id":"GHSA-second", "html_url":second, "state":"draft", "severity":"high"},
        {"ghsa_id":"GHSA-unrelated", "html_url":"https://github.com/acme/widgets/security/advisories/GHSA-unrelated",
         "state":"draft", "severity":null}
    ]);
    scenario.write_gh_state(&gh);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "needs you", "{status}");
    let finding_answer = &status["commands"][0]["github"]["findings"];
    assert_eq!(finding_answer["waits"][first], true);
    assert_eq!(finding_answer["waits"][second], false);
    assert!(
        chrono::DateTime::parse_from_rfc3339(finding_answer["answered_at"].as_str().unwrap())
            .is_ok()
    );
    gh["advisories"][0]["state"] = "closed".into();
    scenario.write_gh_state(&gh);
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "resolved", "{status}");
    assert_eq!(
        status["commands"][0]["left_reason"], "findings triaged",
        "{status}"
    );
    let cache = fs::read_to_string(scenario.path(GITHUB_CACHE)).unwrap();
    for private in ["Secret title", "Secret write-up", "high"] {
        assert!(!cache.contains(private), "{cache}");
    }
    let text = scenario.run(&["status"]);
    assert!(text.stdout.contains("findings triaged"), "{}", text.stdout);
}

#[test]
fn private_findings_and_dismissed_failures_follow_github_triage() {
    let scenario = Scenario::new();
    let url = "https://github.com/acme/widgets/issues/21";
    security_record(&scenario, 7, &[url], true);
    let mut gh = scenario.gh_state();
    gh["private"] = true.into();
    scenario.write_gh_state(&gh);
    scenario.issue_is(21, "OPEN");
    scenario.issue_labelled(21, &["security-finding", "needs-triage"]);
    let path = scenario.path(LOGS).join("7-fixture.command.json");
    let selector = path.to_str().unwrap();
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        status["commands"][0]["needs_you_kind"], "failed",
        "{status}"
    );
    assert_eq!(scenario.run(&["dismiss", selector]).code, Some(0));
    let output = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        status["commands"][0]["needs_you_kind"], "finding",
        "{status}"
    );

    for (state, labels, waits) in [
        ("CLOSED", vec!["security-finding", "needs-triage"], false),
        ("OPEN", vec!["security-finding", "needs-triage"], true),
        ("OPEN", vec!["security-finding"], false),
        ("OPEN", vec!["needs-triage"], false),
    ] {
        scenario.issue_is(21, state);
        scenario.issue_labelled(21, &labels);
        age_cache_file(&scenario, 360);
        let output = scenario.run(&["status", "--all", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(
            status["commands"][0]["group"] == "needs you",
            waits,
            "{status}"
        );
        if !waits {
            assert_eq!(
                status["commands"][0]["left_reason"], "findings triaged",
                "{status}"
            );
            assert!(
                status["commands"][0]["left_at"].as_str().is_some(),
                "{status}"
            );
        }
    }
}

#[test]
fn findings_share_one_paginated_repository_list_and_respect_the_rest_reserve() {
    for remaining in [499, 500] {
        let scenario = Scenario::new();
        let url = "https://github.com/acme/widgets/security/advisories/GHSA-last";
        security_record(&scenario, 7, &[url], false);
        security_record(&scenario, 8, &[url], true);
        failed_record(&scenario, 9, &hours_ago(1));
        change_record(&scenario, 9, |record| {
            record["repository"] = "other/repo".into()
        });
        let mut gh = scenario.gh_state();
        let mut records: Vec<_> = (0..100).map(|number| serde_json::json!({
            "html_url":format!("https://github.com/acme/widgets/security/advisories/GHSA-{number}"),
            "state":"published", "severity":null
        })).collect();
        records.push(serde_json::json!({"html_url":url, "state":"draft", "severity":null}));
        gh["advisories"] = records.into();
        let reset = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
        gh["rate_limits"] = serde_json::json!({
            "graphql":{"remaining":4999,"reset":reset},
            "core":{"remaining":remaining,"reset":reset}
        });
        scenario.write_gh_state(&gh);
        let output = scenario.run(&["status", "--all", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let lists: Vec<_> = scenario
            .gh_calls()
            .into_iter()
            .filter(|call| call.iter().any(|arg| arg.contains("security-advisories?")))
            .collect();
        if remaining == 499 {
            assert!(lists.is_empty(), "{lists:?}");
        } else {
            assert_eq!(lists.len(), 1, "{lists:?}");
            assert!(lists[0].iter().any(|arg| arg == "--paginate"));
            let status: Value = serde_json::from_str(&output.stdout).unwrap();
            assert!(
                status["commands"].as_array().unwrap().iter().any(|row| {
                    row["kind"] == "Security run" && row["needs_you_kind"] == "finding"
                }),
                "{status}"
            );
        }
    }
}

#[test]
fn finding_answers_survive_failure_reopen_and_freeze_in_history() {
    let scenario = Scenario::new();
    let url = "https://github.com/acme/widgets/security/advisories/GHSA-waiting";
    security_record(&scenario, 7, &[url], false);
    scenario.waiting_security_finding();
    let read = || {
        let output = scenario.run(&["status", "--all", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        serde_json::from_str::<Value>(&output.stdout).unwrap()
    };
    assert_eq!(read()["commands"][0]["group"], "needs you");
    let mut gh = scenario.gh_state();
    gh["advisories"][0]["state"] = "published".into();
    gh["advisory_error"] = "gh: Forbidden (HTTP 403)".into();
    scenario.write_gh_state(&gh);
    age_cache_file(&scenario, 360);
    let failed = read();
    assert_eq!(failed["commands"][0]["group"], "needs you", "{failed}");
    assert!(
        failed["github_failure"]
            .as_str()
            .unwrap()
            .contains("Forbidden")
    );
    gh.as_object_mut().unwrap().remove("advisory_error");
    scenario.write_gh_state(&gh);
    let triaged = read();
    assert_eq!(
        triaged["commands"][0]["left_reason"], "findings triaged",
        "{triaged}"
    );
    age_cache_file(&scenario, 360);
    let unchanged = read();
    assert_eq!(
        unchanged["commands"][0]["left_at"],
        triaged["commands"][0]["left_at"]
    );
    gh["advisories"][0]["state"] = "draft".into();
    scenario.write_gh_state(&gh);
    age_cache_file(&scenario, 360);
    assert_eq!(read()["commands"][0]["group"], "needs you");

    gh["advisories"][0]["state"] = "closed".into();
    scenario.write_gh_state(&gh);
    age_cache_file(&scenario, 360);
    read();
    let path = scenario.path(GITHUB_CACHE);
    let mut cache: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for answer in cache["findings"].as_object_mut().unwrap().values_mut() {
        answer["triaged_at"] = hours_ago(240).into();
    }
    fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
    age_cache_file(&scenario, 360);
    let calls = scenario.gh_calls().len();
    assert_eq!(read()["commands"][0]["group"], "history");
    assert_eq!(
        scenario.gh_calls().len(),
        calls,
        "History is not read again"
    );
    security_record(&scenario, 8, &[url], false);
    gh["advisories"][0]["state"] = "draft".into();
    scenario.write_gh_state(&gh);
    let status = read();
    let rows = status["commands"].as_array().unwrap();
    assert_eq!(
        rows.iter().filter(|row| row["group"] == "history").count(),
        1,
        "{status}"
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row["needs_you_kind"] == "finding")
            .count(),
        1,
        "{status}"
    );
}

#[test]
fn private_finding_discovery_rechecks_the_reserve_before_its_fallback_list() {
    let scenario = Scenario::new();
    security_record(
        &scenario,
        7,
        &["https://github.com/acme/widgets/issues/21"],
        false,
    );
    let mut gh = scenario.gh_state();
    gh["private"] = true.into();
    let reset = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp();
    gh["rate_limits"] = serde_json::json!({
        "graphql":{"remaining":4999,"reset":reset},
        "core":{"remaining":500,"reset":reset}
    });
    gh["next_rate_limits"] = serde_json::json!({
        "graphql":{"remaining":4999,"reset":reset},
        "core":{"remaining":498,"reset":reset}
    });
    scenario.write_gh_state(&gh);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    assert!(
        !scenario.gh_calls().iter().any(|call| call
            .iter()
            .any(|arg| { arg.contains("/issues?state=all&labels=security-finding") })),
        "a fallback list read below the reserve: {:?}",
        scenario.gh_calls()
    );
}

#[test]
fn review_graphql_failure_survives_successful_finding_list() {
    let scenario = Scenario::new();
    ready_record_with_pr(&scenario, 9, &hours_ago(240), Some(9));
    github_pr_is(&scenario, 9, "OPEN", None, None);
    let url = "https://github.com/acme/widgets/security/advisories/GHSA-waiting";
    security_record(&scenario, 7, &[url], false);
    scenario.waiting_security_finding();
    let good = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(good.code, Some(0), "{}", good.stderr);
    age_cache_file(&scenario, 360);
    scenario.gh_fails("api graphql");
    let failed = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(failed.code, Some(0), "{}", failed.stderr);
    let status: Value = serde_json::from_str(&failed.stdout).unwrap();
    assert!(
        status["github_failure"].as_str().is_some(),
        "a successful finding list erased the GraphQL failure: {status}"
    );
    assert!(
        status["github_answer_age_seconds"].as_u64().unwrap() >= 300,
        "{status}"
    );
}

#[test]
fn finding_exposed_by_a_settled_failed_fix_leaves_from_its_own_triage_time() {
    let scenario = Scenario::new();
    failed_record(&scenario, 8, &hours_ago(240));
    scenario.waiting_security_finding();
    let dispatched: Value = serde_json::from_slice(
        &fs::read(scenario.path(LOGS).join("8-fixture.command.json")).unwrap(),
    )
    .unwrap();
    change_record(&scenario, 8, |record| {
        record["kind"] = "Security run".into();
        record["issue"] = Value::Null;
        record["security_findings_waiting"] = true.into();
        record["dispatched"] = vec![dispatched].into();
    });
    github_issue_is(&scenario, 8, "CLOSED", Some(&hours_ago(216)));
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "Security run")
        .unwrap();
    assert_eq!(row["needs_you_kind"], "finding", "{status}");
    let mut gh = scenario.gh_state();
    gh["advisories"][0]["state"] = "closed".into();
    scenario.write_gh_state(&gh);
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "Security run")
        .unwrap();
    assert_eq!(row["left_reason"], "findings triaged", "{status}");
    assert_eq!(row["group"], "resolved", "{status}");
}

#[test]
fn closed_issue_takes_a_failed_row_into_resolved_from_its_close_time() {
    let scenario = Scenario::new();
    let closed = hours_ago(1);
    failed_record(&scenario, 7, &hours_ago(240));
    github_issue_is(&scenario, 7, "CLOSED", Some(&closed));
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "resolved", "{status}");
    assert_eq!(row["left_from"], "needs you", "{row}");
    assert_eq!(row["left_reason"], "issue closed", "{row}");
    assert_eq!(row["left_at"], closed, "{row}");
    let issue_answer = &row["github"]["subjects"][scenario.issue_url(7)];
    assert_eq!(issue_answer["facts"]["closed"], true);
    assert_eq!(issue_answer["facts"]["closed_at"], closed);
    assert!(
        chrono::DateTime::parse_from_rfc3339(issue_answer["answered_at"].as_str().unwrap()).is_ok()
    );
    let text = scenario.run(&["status"]);
    assert!(
        text.stdout
            .contains(&format!("left needs you: issue closed at {closed}")),
        "{}",
        text.stdout
    );
    assert_eq!(graphql_calls(&scenario).len(), 1);
}

#[test]
fn failed_rows_leave_only_for_merged_pull_requests_while_their_issues_are_open() {
    for (state, group, reason) in [
        ("MERGED", "resolved", Some("pull request merged")),
        ("CLOSED", "needs you", None),
        ("OPEN", "needs you", None),
    ] {
        let scenario = Scenario::new();
        let at = hours_ago(1);
        failed_record(&scenario, 7, &hours_ago(240));
        change_record(&scenario, 7, |record| {
            record["pull_request"] = github_pr_url(7).into()
        });
        github_pr_is(
            &scenario,
            7,
            state,
            (state == "MERGED").then_some(at.as_str()),
            (state == "CLOSED").then_some(at.as_str()),
        );
        let output = scenario.run(&["status", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        let row = &status["commands"][0];
        assert_eq!(row["group"], group, "{state}: {status}");
        assert_eq!(row["left_reason"].as_str(), reason, "{row}");
        if reason.is_some() {
            assert_eq!(row["left_at"], at, "{row}");
        }
        let calls = graphql_calls(&scenario);
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0]
                .iter()
                .any(|arg| arg.contains("issue(number:") && arg.contains("pullRequest(number:")),
            "{calls:?}"
        );
    }
}

#[test]
fn triage_rows_leave_when_the_label_is_removed_and_keep_the_first_observed_time() {
    let scenario = Scenario::new();
    failed_record(&scenario, 7, &hours_ago(240));
    fs::write(
        scenario.path(LOGS).join("7-fixture.command.stop.json"),
        r#"{"who":"jacob","requeue":false}"#,
    )
    .unwrap();
    scenario.issue_labelled(7, &["NEEDS-TRIAGE"]);
    let output = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        status["commands"][0]["needs_you_kind"], "triage",
        "{status}"
    );
    scenario.issue_labelled(7, &[]);
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "resolved", "{status}");
    assert_eq!(row["left_reason"], "needs-triage removed", "{row}");
    let first = row["left_at"].as_str().unwrap();
    assert!(
        chrono::DateTime::parse_from_rfc3339(first).unwrap()
            > chrono::Local::now() - chrono::Duration::minutes(1)
    );
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--json"]);
    let refreshed: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(refreshed["commands"][0]["left_at"], first, "{refreshed}");
    scenario.issue_labelled(7, &["needs-triage"]);
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--json"]);
    let reopened: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(reopened["commands"][0]["group"], "needs you", "{reopened}");
}

#[test]
fn architect_and_security_failures_use_their_dispatched_plans_and_fixes() {
    for (kind, run_kind, settlement) in [
        ("Architect run", "Spec run", "issue closed"),
        ("Architect run", "Run", "pull request merged"),
        ("Security run", "Run", "issue closed"),
        ("Security run", "Spec run", "pull request merged"),
    ] {
        let scenario = Scenario::new();
        let at = hours_ago(1);
        failed_record(&scenario, 8, &hours_ago(240));
        let dispatched: Value = serde_json::from_slice(
            &fs::read(scenario.path(LOGS).join("8-fixture.command.json")).unwrap(),
        )
        .unwrap();
        change_record(&scenario, 8, |record| {
            let mut run = dispatched.clone();
            run["kind"] = run_kind.into();
            run["pull_request"] = github_pr_url(8).into();
            record["kind"] = kind.into();
            record["name"] = "pass-fixture".into();
            // A pass has no issue or pull request of its own.
            record["issue"] = Value::Null;
            record["dispatched"] = vec![run].into();
        });
        if settlement == "issue closed" {
            github_issue_is(&scenario, 8, "CLOSED", Some(&at));
            github_pr_is(&scenario, 8, "CLOSED", None, Some(&at));
        } else {
            github_issue_is(&scenario, 8, "OPEN", None);
            github_pr_is(&scenario, 8, "MERGED", Some(&at), None);
        }
        let output = scenario.run(&["status", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        let row = &status["commands"][0];
        assert_eq!(row["kind"], kind, "{status}");
        assert_eq!(row["group"], "resolved", "{status}");
        assert_eq!(row["left_reason"], settlement, "{row}");
        assert_eq!(row["left_at"], at, "{row}");
    }
}

#[test]
fn reopening_and_undo_put_failed_rows_where_github_says_without_overriding_dismissals() {
    for hours in [12, 240] {
        let scenario = Scenario::new();
        let closed = hours_ago(1);
        failed_record(&scenario, 7, &hours_ago(hours));
        github_issue_is(&scenario, 7, "CLOSED", Some(&closed));
        let output = scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        let settled_group = if hours == 12 { "done" } else { "resolved" };
        assert_eq!(status["commands"][0]["group"], settled_group, "{status}");
        github_issue_is(&scenario, 7, "OPEN", None);
        age_cache_file(&scenario, 360);
        let output = scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], "needs you", "{status}");
        let dismissed = scenario.run(&["dismiss", "widgets#7"]);
        assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
        github_issue_is(&scenario, 7, "CLOSED", Some(&closed));
        age_cache_file(&scenario, 360);
        let output = scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], settled_group, "{status}");
        assert_eq!(
            status["commands"][0]["dismissal"]["kind"], "Failed",
            "{status}"
        );
        github_issue_is(&scenario, 7, "OPEN", None);
        age_cache_file(&scenario, 360);
        let output = scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], settled_group, "{status}");
        github_issue_is(&scenario, 7, "CLOSED", Some(&closed));
        age_cache_file(&scenario, 360);
        scenario.run(&["status", "--json"]);
        let undone = scenario.run(&["dismiss", "widgets#7", "--undo"]);
        assert_eq!(undone.code, Some(0), "{}", undone.stderr);
        let output = scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], settled_group, "{status}");
        assert_eq!(
            status["commands"][0]["left_reason"], "issue closed",
            "{status}"
        );
        assert!(status["commands"][0].get("dismissal").is_none(), "{status}");
    }
}

#[test]
fn needs_you_ages_only_rows_github_cannot_speak_for() {
    let scenario = Scenario::new();
    let ended = hours_ago(240);
    for number in [7, 8, 99] {
        failed_record(&scenario, number, &ended);
    }
    change_record(&scenario, 8, |record| {
        record["kind"] = "Architect run".into();
        record["name"] = "architect-fixture".into();
        record["issue"] = Value::Null;
        record["cause"] = "billing failure".into();
    });
    scenario.gh_fails("api graphql");
    let output = scenario.run(&["status", "--all", "--json"]);
    let unread: Value = serde_json::from_str(&output.stdout).unwrap();
    assert!(
        unread["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["group"] == "history"),
        "{unread}"
    );
    let mut gh = scenario.gh_state();
    gh["failing"] = serde_json::json!([]);
    scenario.write_gh_state(&gh);
    let output = scenario.run(&["status", "--all", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let rows = status["commands"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{status}");
    for row in rows {
        assert_eq!(
            row["group"],
            if row["issue"]["number"] == 7 {
                "needs you"
            } else {
                "history"
            },
            "{row}"
        );
    }
    // An issue closed outside the retention period goes straight to History
    // and a later reopen cannot bring its frozen row back.
    github_issue_is(&scenario, 7, "CLOSED", Some(&hours_ago(200)));
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--all", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert!(
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["group"] == "history"),
        "{status}"
    );
    let calls = graphql_calls(&scenario).len();
    github_issue_is(&scenario, 7, "OPEN", None);
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--all", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert!(
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["group"] == "history"),
        "{status}"
    );
    assert_eq!(graphql_calls(&scenario).len(), calls);
}

#[test]
fn github_settlement_bounds_resolved_even_when_the_command_ended_more_recently() {
    let scenario = Scenario::new();
    failed_record(&scenario, 7, &hours_ago(48));
    let closed = hours_ago(200);
    github_issue_is(&scenario, 7, "CLOSED", Some(&closed));
    let output = scenario.run(&["status", "--all", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "history", "{status}");
    assert_eq!(status["commands"][0]["left_at"], closed, "{status}");
}

#[test]
fn review_security_fix_settlement_preserves_other_waiting_findings() {
    let scenario = Scenario::new();
    scenario.waiting_security_finding();
    let ended = hours_ago(48);
    let settled = hours_ago(1);
    failed_record(&scenario, 8, &ended);
    let dispatched: Value = serde_json::from_slice(
        &fs::read(scenario.path(LOGS).join("8-fixture.command.json")).unwrap(),
    )
    .unwrap();
    change_record(&scenario, 8, |record| {
        let mut run = dispatched.clone();
        run["outcome"] = "merged".into();
        run["pull_request"] = github_pr_url(8).into();
        record["kind"] = "Security run".into();
        record["name"] = "secure-fixture".into();
        record["issue"] = Value::Null;
        record["outcome"] = "findings recorded".into();
        // Another finding from this audit still awaits the Day shift.
        record["security_findings_waiting"] = true.into();
        record["dispatched"] = vec![run].into();
    });
    github_issue_is(&scenario, 8, "CLOSED", Some(&settled));
    github_pr_is(&scenario, 8, "MERGED", Some(&settled), None);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "Security run")
        .unwrap();
    assert_eq!(row["group"], "needs you", "{status}");
    assert_eq!(row["needs_you_kind"], "finding", "{status}");
}

#[test]
fn review_later_triage_stop_does_not_inherit_an_older_absence_time() {
    let scenario = Scenario::new();
    let old_absence = hours_ago(200);
    let ended = hours_ago(48);
    failed_record(&scenario, 7, &ended);
    fs::write(
        scenario.path(LOGS).join("7-fixture.command.stop.json"),
        r#"{"who":"jacob","requeue":false}"#,
    )
    .unwrap();
    github_issue_is(&scenario, 7, "OPEN", None);
    scenario.issue_labelled(7, &[]);
    let issue = scenario.issue_url(7);
    let mut subjects = serde_json::Map::new();
    subjects.insert(
        issue,
        serde_json::json!({
            "closed": false,
            "closed_at": null,
            "needs_triage": false,
            "triaged_at": old_absence
        }),
    );
    fs::write(
        scenario.path(GITHUB_CACHE),
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "subjects": subjects,
            "history": {}
        }))
        .unwrap(),
    )
    .unwrap();
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "resolved", "{status}");
    assert!(
        chrono::DateTime::parse_from_rfc3339(row["left_at"].as_str().unwrap()).unwrap()
            >= chrono::DateTime::parse_from_rfc3339(&ended).unwrap(),
        "{status}"
    );
}

#[test]
fn review_failed_security_fix_settlement_preserves_other_waiting_findings() {
    let scenario = Scenario::new();
    scenario.waiting_security_finding();
    let ended = hours_ago(48);
    let settled = hours_ago(1);
    failed_record(&scenario, 8, &ended);
    let dispatched: Value = serde_json::from_slice(
        &fs::read(scenario.path(LOGS).join("8-fixture.command.json")).unwrap(),
    )
    .unwrap();
    change_record(&scenario, 8, |record| {
        let mut run = dispatched.clone();
        run["pull_request"] = github_pr_url(8).into();
        record["kind"] = "Security run".into();
        record["name"] = "secure-fixture".into();
        record["issue"] = Value::Null;
        record["outcome"] = "failed".into();
        // Another finding from this audit still awaits the Day shift.
        record["security_findings_waiting"] = true.into();
        record["dispatched"] = vec![run].into();
    });
    let record_path = scenario.path(LOGS).join("8-fixture.command.json");
    let selector = record_path.to_str().unwrap();
    // Dismissing the failed fix exposes the independent findings, whose
    // dismissal then covers the same row even after GitHub settles the fix.
    for kind in ["failed", "finding"] {
        let output = scenario.run(&["status", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(status["commands"][0]["needs_you_kind"], kind, "{status}");
        let dismissed = scenario.run(&["dismiss", selector]);
        assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    }
    github_issue_is(&scenario, 8, "CLOSED", Some(&settled));
    github_pr_is(&scenario, 8, "MERGED", Some(&settled), None);
    age_cache_file(&scenario, 360);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "resolved", "{status}");
    assert_eq!(
        status["commands"][0]["dismissal"]["kinds"],
        serde_json::json!(["Failed", "Finding"]),
        "{status}"
    );
    let undone = scenario.run(&["dismiss", selector, "--undo"]);
    assert_eq!(undone.code, Some(0), "{}", undone.stderr);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "Security run")
        .unwrap();
    assert_eq!(row["group"], "needs you", "{status}");
    assert_eq!(row["needs_you_kind"], "finding", "{status}");
}

#[test]
fn merged_pull_request_takes_its_ready_row_into_resolved_saying_so() {
    let scenario = Scenario::new();
    let ended = hours_ago(48);
    let merged = hours_ago(1);
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"].as_array().unwrap().len(), 1, "{status}");
    let row = &status["commands"][0];
    assert_eq!(row["group"], "resolved", "{row}");
    assert_eq!(row["left_from"], "ready for review", "{row}");
    assert_eq!(row["left_reason"], "pull request merged", "{row}");
    assert_eq!(row["left_at"], merged.as_str(), "{row}");
    assert_eq!(row["github_state"], "merged", "{row}");
    assert_eq!(row["github_at"], merged.as_str(), "{row}");
    let age = status["github_answer_age_seconds"]
        .as_u64()
        .expect("{status}");
    assert!(age <= 60, "{status}");
    let text = scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout.contains(&format!(
            "left ready for review: pull request merged at {merged}"
        )),
        "{}",
        text.stdout
    );
    // One batched read plus the quota preflight, and no call that writes anything.
    let calls = graphql_calls(&scenario);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0]
            .iter()
            .any(|arg| arg.contains("pullRequest(number:") && arg.contains("rateLimit")),
        "{calls:?}"
    );
    assert!(
        scenario
            .gh_calls()
            .iter()
            .all(|call| call[0] == "api" && matches!(call[1].as_str(), "graphql" | "rate_limit")),
        "{:?}",
        scenario.gh_calls()
    );
    // A fresher answer is reused: the second status reads no GitHub.
    scenario.run(&["status", "--json"]);
    assert_eq!(graphql_calls(&scenario).len(), 1);
}

#[test]
fn an_unresolvable_pull_request_does_not_discard_the_other_aliases() {
    let scenario = Scenario::new();
    let ended = hours_ago(48);
    let merged = hours_ago(1);
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    ready_record_with_pr(&scenario, 99, &ended, Some(99));
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let rows = status["commands"].as_array().unwrap();
    let merged = rows.iter().find(|row| row["issue"]["number"] == 7).unwrap();
    assert_eq!(merged["group"], "resolved", "{status}");
    assert_eq!(merged["github_state"], "merged", "{merged}");
    let unresolved = rows
        .iter()
        .find(|row| row["issue"]["number"] == 99)
        .unwrap();
    assert_eq!(unresolved["group"], "ready for review", "{status}");
    assert!(unresolved.get("github_state").is_none(), "{unresolved}");
    assert_eq!(graphql_calls(&scenario).len(), 1);
}

#[test]
fn open_pull_request_keeps_its_ready_row_however_old() {
    let scenario = Scenario::new();
    let ended = (chrono::Local::now() - chrono::Duration::days(10)).to_rfc3339();
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "OPEN", None, None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "ready for review", "{status}");
    assert_eq!(row["github_state"], "open", "{row}");
    assert!(row.get("github_at").is_none_or(Value::is_null), "{row}");
    assert!(row.get("left_reason").is_none_or(Value::is_null), "{row}");
}

#[test]
fn closed_pull_request_moves_its_row_saying_closed() {
    let scenario = Scenario::new();
    let ended = hours_ago(48);
    let closed = hours_ago(1);
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "CLOSED", None, Some(&closed));
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "resolved", "{row}");
    assert_eq!(row["left_reason"], "pull request closed", "{row}");
    assert_eq!(row["left_at"], closed.as_str(), "{row}");
    assert_eq!(row["github_state"], "closed", "{row}");
    assert_eq!(row["github_at"], closed.as_str(), "{row}");
}

#[test]
fn rows_github_cannot_speak_for_still_age_out() {
    let scenario = Scenario::new();
    let ended = (chrono::Local::now() - chrono::Duration::days(10)).to_rfc3339();
    // Pull request 99 is unknown to GitHub; record 8 has no pull request.
    ready_record_with_pr(&scenario, 7, &ended, Some(99));
    ready_record_with_pr(&scenario, 8, &ended, None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"], serde_json::json!([]), "{status}");
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let groups: Vec<_> = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["issue"]["number"].as_u64().unwrap(),
                row["group"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        groups,
        [(7, "history".to_string()), (8, "history".to_string())]
    );
}

#[test]
fn dismissal_of_an_old_open_pull_request_survives_github_refreshes_until_undo() {
    let scenario = Scenario::new();
    let ended = hours_ago(30 * 24);
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "OPEN", None, None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        status["commands"][0]["group"], "ready for review",
        "{status}"
    );

    let calls = scenario.gh_calls().len();
    let output = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    assert_eq!(scenario.gh_calls().len(), calls);

    let closed = hours_ago(1);
    for (state, at) in [
        ("OPEN", None),
        ("CLOSED", Some(closed.as_str())),
        ("OPEN", None),
    ] {
        github_pr_is(&scenario, 7, state, None, at);
        age_cache_file(&scenario, 360);
        let output = scenario.run(&["status", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        let row = &status["commands"][0];
        assert_eq!(row["group"], "resolved", "{status}");
        assert_eq!(row["dismissal"]["kind"], "Ready", "{row}");
        assert_eq!(row["github_state"], state.to_ascii_lowercase(), "{row}");
        assert!(!row["dismissal"]["who"].as_str().unwrap().is_empty());
    }

    let calls = scenario.gh_calls().len();
    let output = scenario.run(&["dismiss", "widgets#7", "--undo"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    assert_eq!(scenario.gh_calls().len(), calls);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "ready for review", "{status}");
    assert_eq!(row["github_state"], "open", "{row}");
    assert!(row.get("dismissal").is_none(), "{row}");
}

#[test]
fn dismiss_refuses_a_ready_row_already_moved_by_the_kept_github_answer() {
    let scenario = Scenario::new();
    let ended = hours_ago(48);
    let merged = hours_ago(1);
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "resolved", "{status}");

    let calls = scenario.gh_calls().len();
    let output = scenario.run(&["dismiss", "widgets#7"]);
    assert_ne!(output.code, Some(0));
    assert!(
        output.stderr.contains("not Needs you or Ready for review"),
        "{}",
        output.stderr
    );
    assert_eq!(scenario.gh_calls().len(), calls);
}

#[test]
fn reopened_pull_request_brings_back_a_resolved_row() {
    let scenario = Scenario::new();
    let ended = hours_ago(48);
    let merged = hours_ago(1);
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    let output = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "resolved", "{status}");
    github_pr_is(&scenario, 7, "OPEN", None, None);
    fs::remove_file(scenario.path(GITHUB_CACHE)).unwrap();
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "ready for review", "{status}");
    assert_eq!(row["github_state"], "open", "{row}");
}

#[test]
fn merged_weeks_ago_goes_straight_to_history_and_is_not_reread() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    let ended = (now - chrono::Duration::days(30)).to_rfc3339();
    let merged = (now - chrono::Duration::days(20)).to_rfc3339();
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "history", "{status}");
    assert_eq!(row["left_reason"], "pull request merged", "{row}");
    assert_eq!(graphql_calls(&scenario).len(), 1);
    // From History on the row is not read again.
    scenario.run(&["status", "--all", "--json"]);
    assert_eq!(graphql_calls(&scenario).len(), 1);
}

#[test]
fn a_history_row_does_not_block_a_current_row_sharing_its_pull_request() {
    let scenario = Scenario::new();
    let old_end = (chrono::Local::now() - chrono::Duration::days(10)).to_rfc3339();
    ready_record_with_pr(&scenario, 7, &old_end, Some(7));
    github_pr_is(&scenario, 7, "OPEN", None, None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let first: Value = serde_json::from_str(&output.stdout).unwrap();
    let url = github_pr_url(7);
    let observed = first["commands"][0]["github"]["subjects"][&url]["answered_at"].clone();
    assert!(chrono::DateTime::parse_from_rfc3339(observed.as_str().unwrap()).is_ok());
    fs::rename(
        scenario.path(LOGS).join("7-fixture.command.json"),
        scenario.path(LOGS).join("7-old.command.json"),
    )
    .unwrap();
    let current_start = (chrono::Local::now() - chrono::Duration::days(8)).to_rfc3339();
    let current_end = (chrono::Local::now() - chrono::Duration::days(6)).to_rfc3339();
    ready_record_with_pr(&scenario, 7, &current_end, Some(7));
    change_record(&scenario, 7, |record| {
        record["started_at"] = current_start.into()
    });
    let merged = hours_ago(1);
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    age_cache_file(&scenario, 360);
    // The second invocation proves that History's own facts stay frozen
    // after the current row's refreshed answer has been saved.
    for _ in 0..2 {
        let output = scenario.run(&["status", "--all", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        let rows = status["commands"].as_array().unwrap();
        let current = rows
            .iter()
            .find(|row| row["ended_at"] == current_end)
            .unwrap();
        assert_eq!(current["group"], "resolved", "{status}");
        assert_eq!(current["github_state"], "merged", "{current}");
        let history = rows.iter().find(|row| row["ended_at"] == old_end).unwrap();
        assert_eq!(history["group"], "history", "{status}");
        assert_eq!(history["github_state"], "open", "{history}");
        assert_eq!(history["github"]["subjects"][&url]["answered_at"], observed);
        assert_ne!(current["github"]["subjects"][&url]["answered_at"], observed);
    }
    assert_eq!(graphql_calls(&scenario).len(), 2);
}

#[test]
fn unusable_kept_answers_are_read_again_even_when_the_file_is_fresh() {
    for contents in ["{", r#"{"version":2,"pull_requests":{}}"#] {
        let scenario = Scenario::new();
        let ended = hours_ago(48);
        let merged = hours_ago(1);
        ready_record_with_pr(&scenario, 7, &ended, Some(7));
        github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
        fs::write(scenario.path(GITHUB_CACHE), contents).unwrap();
        let output = scenario.run(&["status", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], "resolved", "{status}");
        assert_eq!(status["commands"][0]["github_state"], "merged", "{status}");
        assert_eq!(graphql_calls(&scenario).len(), 1);
    }
}

#[test]
fn stale_kept_answer_is_read_again() {
    let scenario = Scenario::new();
    let ended = hours_ago(48);
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "OPEN", None, None);
    scenario.run(&["status", "--json"]);
    assert_eq!(graphql_calls(&scenario).len(), 1);
    age_cache_file(&scenario, 360);
    scenario.run(&["status", "--json"]);
    assert_eq!(graphql_calls(&scenario).len(), 2);
}

#[test]
fn dispatched_ticket_run_is_judged_by_its_own_pull_request() {
    let scenario = Scenario::new();
    let outer_end = (chrono::Local::now() - chrono::Duration::hours(70)).to_rfc3339();
    let outer_start = (chrono::Local::now() - chrono::Duration::hours(72)).to_rfc3339();
    let ended = hours_ago(48);
    let merged = hours_ago(1);
    // A Pickup run whose dispatched Ticket run ended ready with its own
    // pull request; the outer run names none.
    write_record(&scenario, 7, &outer_start, Some(&outer_end));
    let path: PathBuf = scenario.path(LOGS).join("7-fixture.command.json");
    let mut outer: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut dispatched = outer.clone();
    dispatched["kind"] = "Run".into();
    dispatched["issue"] = serde_json::json!({
        "number": 7,
        "url": scenario.issue_url(7),
        "title": "Fixture",
    });
    dispatched["started_at"] = ended.clone().into();
    dispatched["ended_at"] = ended.clone().into();
    dispatched["outcome"] = "ready for review".into();
    dispatched["pull_request"] = github_pr_url(7).into();
    outer["kind"] = "Pickup run".into();
    outer["outcome"] = "ready for review".into();
    outer["dispatched"] = vec![dispatched].into();
    fs::write(&path, serde_json::to_vec(&outer).unwrap()).unwrap();
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let rows: Vec<_> = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["kind"].as_str().unwrap().to_owned(),
                row["group"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(
        rows.contains(&("Pickup run".to_string(), "resolved".to_string())),
        "{status}"
    );
    let dispatched = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "Run")
        .expect("{status}");
    assert_eq!(dispatched["group"], "resolved", "{dispatched}");
    assert_eq!(
        dispatched["left_reason"], "pull request merged",
        "{dispatched}"
    );
    assert_eq!(dispatched["github_state"], "merged", "{dispatched}");
}

#[test]
fn status_json_gives_null_answer_age_with_nothing_to_read() {
    let scenario = Scenario::new();
    let ended = hours_ago(2);
    write_record(&scenario, 7, &ended, Some(&ended));
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["github_answer_age_seconds"], Value::Null, "{status}");
    assert!(graphql_calls(&scenario).is_empty());
    assert!(!scenario.path(GITHUB_CACHE).exists());
}

#[test]
fn reopened_pull_request_brings_back_a_done_row() {
    let scenario = Scenario::new();
    let ended = hours_ago(12);
    let merged = hours_ago(6);
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    let output = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "done", "{status}");
    github_pr_is(&scenario, 7, "OPEN", None, None);
    fs::remove_file(scenario.path(GITHUB_CACHE)).unwrap();
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        status["commands"][0]["group"], "ready for review",
        "{status}"
    );
}

#[test]
fn failed_github_read_keeps_the_last_answer() {
    let scenario = Scenario::new();
    let ended = (chrono::Local::now() - chrono::Duration::days(10)).to_rfc3339();
    ready_record_with_pr(&scenario, 7, &ended, Some(7));
    github_pr_is(&scenario, 7, "OPEN", None, None);
    // GitHub unreachable and nothing kept yet: the row not read yet goes
    // by age.
    scenario.gh_fails("api graphql");
    let output = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "history", "{status}");
    assert_eq!(status["github_last_answered_at"], Value::Null);
    assert!(status["github_failure"].as_str().is_some());
    // A good read keeps its answer; a later failed read keeps the last one,
    // even for a pull request GitHub has since merged.
    let mut gh = scenario.gh_state();
    gh["failing"] = serde_json::json!([]);
    scenario.write_gh_state(&gh);
    let output = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        status["commands"][0]["group"], "ready for review",
        "{status}"
    );
    let merged = hours_ago(1);
    github_pr_is(&scenario, 7, "MERGED", Some(&merged), None);
    let reads_before_failure = graphql_calls(&scenario).len();
    age_cache_file(&scenario, 360);
    scenario.gh_fails("api graphql");
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "ready for review", "{status}");
    assert_eq!(row["github_state"], "open", "{row}");
    assert!(graphql_calls(&scenario).len() > reads_before_failure);
    assert!(status["github_answer_age_seconds"].as_u64().unwrap() >= 360);
}

#[test]
fn github_failures_warn_with_last_answer_and_clear_after_recovery() {
    for diagnostic in [
        "error connecting to api.github.com: network is unreachable",
        "HTTP 401: Bad credentials (https://api.github.com/graphql)",
        "HTTP 403: API rate limit exceeded (https://api.github.com/graphql)",
    ] {
        let scenario = Scenario::new();
        ready_record_with_pr(&scenario, 7, &hours_ago(240), Some(7));
        github_pr_is(&scenario, 7, "OPEN", None, None);
        failed_record(&scenario, 9, &hours_ago(240));
        github_issue_is(&scenario, 9, "OPEN", None);
        let good = scenario.run(&["status", "--json"]);
        assert_eq!(good.code, Some(0), "{}", good.stderr);
        age_cache_file(&scenario, 360);
        // A never-read row still goes by age, while the old open row keeps
        // the facts from its last answer even after GitHub merges it.
        ready_record_with_pr(&scenario, 8, &hours_ago(240), Some(8));
        github_pr_is(&scenario, 7, "MERGED", Some(&hours_ago(1)), None);
        github_issue_is(&scenario, 9, "CLOSED", Some(&hours_ago(1)));
        let mut gh = scenario.gh_state();
        gh["failing"] = serde_json::json!(["api graphql"]);
        gh["failing_stderr"] = diagnostic.into();
        scenario.write_gh_state(&gh);
        let output = scenario.run(&["status", "--all", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        let rows = status["commands"].as_array().unwrap();
        assert!(
            rows.iter()
                .any(|row| row["issue"]["number"] == 7 && row["group"] == "ready for review"),
            "{status}"
        );
        assert!(
            rows.iter()
                .any(|row| row["issue"]["number"] == 8 && row["group"] == "history"),
            "{status}"
        );
        assert!(
            rows.iter()
                .any(|row| row["issue"]["number"] == 9 && row["group"] == "needs you"),
            "{status}"
        );
        let answered = status["github_last_answered_at"]
            .as_str()
            .expect(&output.stdout);
        assert!(chrono::DateTime::parse_from_rfc3339(answered).is_ok());
        assert!(
            status["github_failure"]
                .as_str()
                .unwrap()
                .contains(diagnostic),
            "{status}"
        );
        let text = scenario.run(&["status"]);
        assert!(
            text.stdout.contains("GitHub last answered"),
            "{}",
            text.stdout
        );
        assert!(text.stdout.contains(answered), "{}", text.stdout);
        assert!(text.stdout.contains(diagnostic), "{}", text.stdout);
        gh["failing"] = serde_json::json!([]);
        scenario.write_gh_state(&gh);
        let output = scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(status["github_failure"], Value::Null, "{status}");
        assert!(
            status["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["issue"]["number"] == 9 && row["left_reason"] == "issue closed"),
            "{status}"
        );
        assert!(status["github_answer_age_seconds"].as_u64().unwrap() < 5);
        assert!(
            !scenario
                .run(&["status"])
                .stdout
                .contains("GitHub last answered")
        );
    }
}

#[test]
fn status_reserves_both_quotas_until_their_resets() {
    for (resource, kept_name, description) in
        [("graphql", "graphql", "GraphQL"), ("core", "rest", "REST")]
    {
        let scenario = Scenario::new();
        ready_record_with_pr(&scenario, 7, &hours_ago(48), Some(7));
        github_pr_is(&scenario, 7, "OPEN", None, None);
        failed_record(&scenario, 8, &hours_ago(240));
        github_issue_is(&scenario, 8, "OPEN", None);
        let reset = chrono::Utc::now() + chrono::Duration::hours(1);
        let mut gh = scenario.gh_state();
        gh["rate_limits"][resource] = serde_json::json!({
            "remaining": 500, "reset": reset.timestamp(),
        });
        scenario.write_gh_state(&gh);
        // Exactly 500 is allowed; fewer than 500 protects the reserve.
        assert_eq!(scenario.run(&["status", "--json"]).code, Some(0));
        assert_eq!(graphql_calls(&scenario).len(), 1);
        age_cache_file(&scenario, 360);
        github_issue_is(&scenario, 8, "CLOSED", Some(&hours_ago(1)));
        gh = scenario.gh_state();
        gh["rate_limits"][resource]["remaining"] = 499.into();
        scenario.write_gh_state(&gh);
        let output = scenario.run(&["status", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        assert_eq!(
            graphql_calls(&scenario).len(),
            1,
            "spent the factory's {description} reserve"
        );
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        let rows = status["commands"].as_array().unwrap();
        assert!(
            rows.iter()
                .any(|row| row["issue"]["number"] == 7 && row["group"] == "ready for review"),
            "{status}"
        );
        assert!(
            rows.iter()
                .any(|row| row["issue"]["number"] == 8 && row["group"] == "needs you"),
            "{status}"
        );
        assert!(status["github_answer_age_seconds"].as_u64().unwrap() >= 360);
        let before = scenario.gh_calls();
        let text = scenario.run(&["status"]);
        assert!(text.stdout.contains(description), "{}", text.stdout);
        assert!(text.stdout.contains("499"), "{}", text.stdout);
        assert!(text.stdout.contains("until"), "{}", text.stdout);
        assert_eq!(
            scenario.gh_calls(),
            before,
            "read while {description} quota was low"
        );
        // Move the recorded reset into the past at the fixture's time seam.
        let path = scenario.path(GITHUB_CACHE);
        let mut kept: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        kept["quotas"][kept_name]["reset_at"] = hours_ago(1).into();
        fs::write(path, serde_json::to_vec(&kept).unwrap()).unwrap();
        gh["rate_limits"][resource]["remaining"] = 5000.into();
        scenario.write_gh_state(&gh);
        let output = scenario.run(&["status", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        assert_eq!(graphql_calls(&scenario).len(), 2);
        let status: Value = serde_json::from_str(&output.stdout).unwrap();
        assert!(
            status["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["issue"]["number"] == 8 && row["left_reason"] == "issue closed"),
            "{status}"
        );
        assert!(!scenario.run(&["status"]).stdout.contains("Warning:"));
    }
}

#[test]
fn graphql_response_stops_following_batches_and_warns_while_fresh() {
    let scenario = Scenario::new();
    let ended = hours_ago(48);
    for number in 1..=101 {
        ready_record_with_pr(&scenario, number, &ended, Some(number.into()));
        github_pr_is(&scenario, number.into(), "OPEN", None, None);
    }
    let mut gh = scenario.gh_state();
    gh["inbox_rate_limit"] = serde_json::json!({
        "remaining": 499,
        "resetAt": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
    });
    scenario.write_gh_state(&gh);
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        graphql_calls(&scenario).len(),
        1,
        "read another batch after reaching the reserve"
    );
    assert_eq!(
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["github_state"] == "open")
            .count(),
        100
    );
    assert!(status["github_answer_age_seconds"].as_u64().unwrap() < 5);
    let before = scenario.gh_calls();
    let text = scenario.run(&["status"]);
    assert!(
        text.stdout.contains("499 GraphQL points remaining"),
        "{}",
        text.stdout
    );
    assert_eq!(scenario.gh_calls(), before);
}

#[test]
fn legacy_github_answer_time_survives_a_failed_refresh() {
    let scenario = Scenario::new();
    ready_record_with_pr(&scenario, 7, &hours_ago(48), Some(7));
    github_pr_is(&scenario, 7, "OPEN", None, None);
    assert_eq!(scenario.run(&["status", "--json"]).code, Some(0));

    // A supported version-1 answer predating the separate freshness marker.
    let path = scenario.path(GITHUB_CACHE);
    let mut legacy: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy["version"] = 1.into();
    let subjects = legacy.as_object_mut().unwrap().remove("subjects").unwrap();
    legacy["pull_requests"] = subjects;
    for field in ["failure", "quotas", "warning"] {
        legacy.as_object_mut().unwrap().remove(field);
    }
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    fs::remove_file(path.with_extension("refreshed")).unwrap();
    age_cache_file(&scenario, 360);
    scenario.gh_fails("api graphql");

    let first = scenario.run(&["status", "--json"]);
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    let first: Value = serde_json::from_str(&first.stdout).unwrap();
    let answered = first["github_last_answered_at"]
        .as_str()
        .expect("the version-1 file records a last answer")
        .to_owned();

    let second = scenario.run(&["status", "--json"]);
    assert_eq!(second.code, Some(0), "{}", second.stderr);
    let second: Value = serde_json::from_str(&second.stdout).unwrap();
    assert_eq!(second["commands"][0]["group"], "ready for review");
    assert_eq!(
        second["github_last_answered_at"], answered,
        "a failed refresh must preserve the last successful read time: {second}"
    );
    let text = scenario.run(&["status"]);
    assert!(text.stdout.contains(&answered), "{}", text.stdout);
    assert!(!text.stdout.contains("GitHub has not answered yet"));
}

#[test]
fn repair_resume_and_security_review_keep_their_own_session_logs() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, &support::leaves_running("cargo test"));
    scenario.agent_does_in_session(2, &format!("{OPENS_PR}gh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\":\"test\",\"conclusion\":\"failure\"}}]'\n"));
    scenario.agent_does_in_session(3, "printf '%s\\n' 'Security review: {\"unaddressed_count\":0,\"findings\":[],\"pre_existing_count\":0}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n");
    scenario.agent_does_in_session(4, "echo fix > fix.txt\ngit add fix.txt\ngit commit -qm 'Fix CI'\ngh fake checks \"$(git rev-parse HEAD)\" '[{\"name\":\"test\",\"conclusion\":\"success\"}]'\n");
    let ended = scenario.run(&[&scenario.issue_url(7), "security-review"]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let record = scenario.command_record(LOGS);
    let stages = record["stages"].as_array().unwrap();
    for (name, log) in [
        ("Resume", "-implement-resume.logs.jsonl"),
        ("Security review", "-security-review.logs.jsonl"),
        ("Repair", "-repair-1.logs.jsonl"),
    ] {
        assert!(
            stages.iter().any(|stage| stage["name"] == name
                && stage["session_log"]
                    .as_str()
                    .is_some_and(|path| path.ends_with(log))),
            "{record}"
        );
    }
}
