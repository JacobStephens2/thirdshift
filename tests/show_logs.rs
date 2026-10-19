mod support;

use serde_json::Value;
use std::fs;
use std::time::{Duration, Instant};
use support::Scenario;

const OPENS_PR: &str = "echo work > work.txt\ngit add work.txt\ngit commit -qm 'Make widgets'\ngh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'\n";

#[test]
fn owned_and_legacy_commands_keep_exact_paths_and_record_precedence() {
    let scenario = Scenario::new();
    let folder = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    let mut activity = String::new();
    for (stem, suffix, started, marker) in [
        (
            "7-20261003T120000-0400",
            ".123-start-1-0",
            "2026-10-03T12:00:00-04:00",
            "legacy",
        ),
        (
            "7-20261003T120000-0400~ownerA",
            "",
            "2026-10-03T12:00:01-04:00",
            "first",
        ),
        (
            "7-20261003T120000-0400~ownerB",
            "",
            "2026-10-03T12:00:02-04:00",
            "second",
        ),
    ] {
        let log = folder.join(format!("{stem}.log"));
        let session = folder.join(format!("{stem}-implement.jsonl"));
        fs::write(&log, format!("{marker} Command output\n")).unwrap();
        fs::write(&session, format!("{marker} Session output\n")).unwrap();
        let record = serde_json::json!({
            "version": 1, "kind": "Run", "repository": "acme/widgets", "name": "acme/widgets#7",
            "issue": {"number": 7, "url": scenario.issue_url(7), "title": marker},
            "hostname": "factory", "process_id": 0, "process_start": "gone", "harness": "claude",
            "model": null, "effort": null, "started_at": started, "ended_at": started,
            "stages": [], "session_log": null, "outcome": "ready for review", "cause": null,
            "issue_branch": null, "pull_request": null
        });
        fs::write(
            folder.join(format!("{stem}{suffix}.command.json")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        activity.push_str(&format!("2026-10-03 12:00:00 Run #7 started: commands/issue/{stem}.log, on claude\n2026-10-03 12:00:01 Run #7 ended: ready for review\n"));
    }
    fs::write(
        folder
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("activity.log"),
        activity,
    )
    .unwrap();
    let status = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["version"], 1);
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 3, "{status}");
    for command in commands {
        assert_eq!(command["source"], "command_record");
        let output = fs::read_to_string(command["command_log"].as_str().unwrap()).unwrap();
        assert!(output.starts_with(command["issue"]["title"].as_str().unwrap()));
    }
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(
        shown["command"]["command_log"],
        folder
            .join("7-20261003T120000-0400~ownerB.log")
            .to_str()
            .unwrap()
    );
    assert_eq!(
        shown["command"]["session_logs"],
        serde_json::json!([folder.join("7-20261003T120000-0400~ownerB-implement.jsonl")])
    );
    for args in [
        vec!["logs", "widgets#7"],
        vec!["logs", "widgets#7", "--follow"],
    ] {
        let output = scenario.run(&args);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        assert_eq!(output.stdout, "second Command output\n");
    }
    let session = scenario.run(&["logs", "widgets#7", "--session", "implement"]);
    assert_eq!(session.code, Some(0), "{}", session.stderr);
    assert_eq!(session.stdout, "second Session output\n");
}

#[test]
fn show_activity_history_lists_its_session_logs() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let session = shown["command"]["session_log"].as_str().unwrap().to_owned();
    let log = std::path::Path::new(shown["command"]["command_log"].as_str().unwrap());
    for entry in fs::read_dir(log.parent().unwrap()).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().ends_with(".command.json") {
            fs::remove_file(path).unwrap();
        }
    }
    let result = scenario.run(&["show", "widgets#7", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let result: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(result["command"]["source"], "activity_log");
    assert!(
        result["command"]["session_logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path.as_str() == Some(session.as_str())),
        "missing Session log: {result}"
    );
}

#[test]
fn show_lists_the_original_architect_session_and_its_resume() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent", "architecture"]);
    scenario.agent_does(&support::leaves_running("cargo test"));
    scenario.agent_does_in_session(
        2,
        r#"url=$(gh issue create --title 'Plan' --body 'The plan' --label needs-triage)
printf 'Architecture review plan: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let ended = scenario.run(&["architect", "--plan-only"]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let status = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let name = status["commands"][0]["name"].as_str().unwrap();
    let current = std::path::Path::new(status["commands"][0]["session_log"].as_str().unwrap());
    let original = std::path::PathBuf::from(current.to_string_lossy().replace(
        "-architecture-review-resume.jsonl",
        "-architecture-review.jsonl",
    ));
    assert_ne!(current, original.as_path());
    assert!(original.is_file(), "{}", original.display());
    let text = scenario.run(&["show", name]);
    let json = scenario.run(&["show", name, "--json"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert_eq!(json.code, Some(0), "{}", json.stderr);
    assert!(
        text.stdout.contains(original.to_str().unwrap()),
        "original Session log missing: {}",
        text.stdout
    );
    assert!(
        json.stdout.contains(original.to_str().unwrap()),
        "original Session log missing: {}",
        json.stdout
    );
}

#[test]
fn naming_prefers_a_live_command_then_the_latest_and_refuses_ambiguous_repositories() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let mut future = shown["command"].clone();
    future["started_at"] = "2099-01-01T00:00:00Z".into();
    future["ended_at"] = "2099-01-01T01:00:00Z".into();
    let log = std::path::Path::new(future["command_log"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("7-future.command.json");
    fs::write(log, serde_json::to_vec(&future).unwrap()).unwrap();
    scenario.agent_does(&format!(
        "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\necho continued >> work.txt\ngit add work.txt\ngit commit -qm Continue\n",
        scenario.path("started").display(), scenario.path("release").display()
    ));
    let held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
    let live: Vec<_> = ["widgets#7", "acme/widgets#7", &scenario.issue_url(7)]
        .into_iter()
        .map(|name| scenario.run(&["show", name, "--json"]))
        .collect();
    fs::write(scenario.path("release"), "").unwrap();
    let ended = held.finish();
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    for result in live {
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let shown: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(shown["command"]["state"], "live");
        assert_ne!(shown["command"]["started_at"], "2099-01-01T00:00:00Z");
    }
    let latest = scenario.run(&["show", "widgets#7", "--json"]);
    let latest: Value = serde_json::from_str(&latest.stdout).unwrap();
    assert_eq!(latest["command"]["started_at"], "2099-01-01T00:00:00Z");
    future["repository"] = "other/widgets".into();
    future["issue"]["url"] = "https://github.com/other/widgets/issues/7".into();
    future["name"] = "other/widgets#7".into();
    let folder = scenario.path("home/.thirdshift/logs/other/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("7-future.command.json"),
        serde_json::to_vec(&future).unwrap(),
    )
    .unwrap();
    for command in ["show", "logs"] {
        let ambiguous = scenario.run(&[command, "widgets#7"]);
        assert_ne!(ambiguous.code, Some(0));
        assert!(
            ambiguous.stderr.contains("ambiguous Command name"),
            "{}",
            ambiguous.stderr
        );
        for repo in ["acme/widgets", "other/widgets"] {
            assert!(ambiguous.stderr.contains(repo));
        }
    }
    let qualified = scenario.run_from(
        &scenario.path("home"),
        &["show", "acme/widgets#7", "--json"],
    );
    assert_eq!(qualified.code, Some(0), "{}", qualified.stderr);
}

#[test]
fn an_architect_command_is_named_by_its_kind_and_stamp() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent", "architecture"]);
    scenario.agent_does(
        r#"url=$(gh issue create --title 'Plan' --body 'The plan' --label needs-triage)
printf 'Architecture review plan: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let ended = scenario.run(&["architect", "--plan-only"]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let status = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let name = status["commands"][0]["name"].as_str().unwrap();
    assert!(name.starts_with("architect-"));
    let shown = scenario.run(&["show", name, "--json"]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert_eq!(shown["command"]["kind"], "Architect run");
    assert_eq!(shown["command"]["issue"], Value::Null);
    let logs = scenario.run(&["logs", name, "--session", "architecture-review"]);
    assert_eq!(logs.code, Some(0), "{}", logs.stderr);
    assert_eq!(
        logs.stdout,
        fs::read_to_string(shown["command"]["session_log"].as_str().unwrap()).unwrap()
    );
}

#[test]
fn a_tickets_issue_names_its_run_inside_the_spec_command() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[])]);
    scenario.agent_does_for(21, &format!(
        "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\necho ticket > ticket.txt\ngit add ticket.txt\ngit commit -qm Ticket\ngh pr create --base issue-20 --head issue-21 --title Ticket --body 'Closes #21'\n",
        scenario.path("started").display(), scenario.path("release").display()
    ));
    let held = scenario.run_until(&[&scenario.issue_url(20)], &[], "started");
    let ticket = scenario.run(&["show", "widgets#21", "--json"]);
    let parent = scenario.run(&["show", "widgets#20", "--json"]);
    let logs = scenario.run(&["logs", "widgets#21", "--session", "implement"]);
    fs::write(scenario.path("release"), "").unwrap();
    let ended = held.finish();
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    assert_eq!(ticket.code, Some(0), "{}", ticket.stderr);
    assert_eq!(logs.code, Some(0), "{}", logs.stderr);
    let ticket: Value = serde_json::from_str(&ticket.stdout).unwrap();
    let parent: Value = serde_json::from_str(&parent.stdout).unwrap();
    assert_eq!(ticket["command"]["issue"]["number"], 21);
    assert_eq!(ticket["command"]["state"], "live");
    assert_eq!(
        ticket["command"]["command_log"],
        parent["command"]["command_log"]
    );
    assert_eq!(parent["command"]["children"][0]["issue"]["number"], 21);
    assert!(
        logs.stdout
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok())
    );
    assert!(logs.stdout.contains("session_id"));
}

#[test]
fn logs_follow_streams_appended_bytes_and_exits_when_the_command_ends() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\necho '{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{{\"command\":\"cargo test --appended\"}}}}]}}}}'\nwhile ! test -f '{}'; do sleep 0.02; done\n{OPENS_PR}",
        scenario.path("started").display(), scenario.path("append").display(), scenario.path("release").display()
    ));
    let held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let path = shown["command"]["command_log"].as_str().unwrap();
    let output = scenario.path("followed.txt");
    let follower = scenario.read_to_file(&["logs", "widgets#7", "--follow"], &output);
    let session_output = scenario.path("session-followed.txt");
    let session_follower = scenario.read_to_file(
        &["logs", "widgets#7", "--follow", "--session", "implement"],
        &session_output,
    );
    let initial_deadline = Instant::now() + Duration::from_secs(5);
    while (fs::read_to_string(&output).unwrap().is_empty()
        || fs::read_to_string(&session_output).unwrap().is_empty())
        && Instant::now() < initial_deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::write(scenario.path("append"), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!fs::read_to_string(&output)
        .unwrap()
        .contains("cargo test --appended")
        || !fs::read_to_string(&session_output)
            .unwrap()
            .contains("cargo test --appended"))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let live_output = fs::read_to_string(&output).unwrap();
    let live_session = fs::read_to_string(&session_output).unwrap();
    fs::write(scenario.path("release"), "").unwrap();
    let ended = held.finish();
    let followed = follower.finish();
    let session_followed = session_follower.finish();
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    assert_eq!(followed.code, Some(0), "{}", followed.stderr);
    assert_eq!(
        session_followed.code,
        Some(0),
        "{}",
        session_followed.stderr
    );
    assert!(
        live_session.contains("cargo test --appended"),
        "{live_session}"
    );
    assert_eq!(
        fs::read_to_string(session_output).unwrap(),
        fs::read_to_string(shown["command"]["session_log"].as_str().unwrap()).unwrap()
    );
    assert!(
        live_output.contains("cargo test --appended"),
        "{live_output}"
    );
    assert_eq!(
        fs::read_to_string(output).unwrap(),
        fs::read_to_string(path).unwrap()
    );
}

#[test]
fn logs_session_repair_selects_the_latest_numbered_repair() {
    let scenario = Scenario::new();
    let red = r#"gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]'
"#;
    let green = r#"gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success","url":"https://ci.example/test"}]'
"#;
    scenario.agent_does(&format!("{OPENS_PR}{red}"));
    scenario.agent_does_in_session(
        2,
        &format!("echo fix > fix.txt\ngit add fix.txt\ngit commit -qm Fix\n{red}"),
    );
    scenario.agent_does_in_session(
        3,
        &format!("echo fixed >> fix.txt\ngit add fix.txt\ngit commit -qm Fixed\n{green}"),
    );
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let latest = shown["command"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|stage| stage["session_log"].as_str())
        .find(|path| path.ends_with("-repair-2.jsonl"))
        .unwrap();
    let logs = scenario.run(&["logs", "widgets#7", "--session", "repair"]);
    assert_eq!(logs.code, Some(0), "{}", logs.stderr);
    assert_eq!(logs.stdout, fs::read_to_string(latest).unwrap());
}

#[test]
fn logs_session_selects_the_latest_implement_including_its_resume() {
    let scenario = Scenario::new();
    scenario.agent_does(&support::leaves_running("cargo test"));
    scenario.agent_does_in_session(2, OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let session = shown["command"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|stage| stage["session_log"].as_str())
        .find(|path| path.ends_with("-implement-resume.jsonl"))
        .unwrap();
    let expected = fs::read_to_string(session).unwrap();
    let result = scenario.run(&["logs", "widgets#7", "--session", "implement"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, expected);
    let missing = scenario.run(&["logs", "widgets#7", "--session", "spec-review"]);
    assert_ne!(missing.code, Some(0));
    assert!(
        missing.stderr.contains("no spec-review Session log"),
        "{}",
        missing.stderr
    );
}

#[test]
fn logs_can_read_sessions_from_activity_and_session_only_history() {
    let scenario = Scenario::new();
    scenario.agent_does(&support::leaves_running("cargo test"));
    scenario.agent_does_in_session(2, OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let shown = scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    let command_log = std::path::Path::new(shown["command"]["command_log"].as_str().unwrap());
    let latest_session = shown["command"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|stage| stage["session_log"].as_str())
        .next_back()
        .unwrap();
    let expected = fs::read_to_string(latest_session).unwrap();
    for entry in fs::read_dir(command_log.parent().unwrap()).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().ends_with(".command.json") {
            fs::remove_file(path).unwrap();
        }
    }
    let activity = scenario.run(&["logs", "widgets#7", "--session", "implement"]);
    fs::remove_file(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log")).unwrap();
    let session_only = scenario.run(&["logs", "widgets#7", "--session", "implement"]);
    for result in [activity, session_only] {
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(result.stdout, expected);
    }
}

#[test]
fn show_reads_the_latest_live_event_from_claude_and_codex_session_tails() {
    for (harness, event) in [
        (
            "claude",
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}]}}"#,
        ),
        (
            "codex",
            r#"{"type":"item.started","item":{"id":"cmd","type":"command_execution","command":"/bin/bash -lc 'cargo test'","status":"in_progress"}}"#,
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\n{OPENS_PR}",
            scenario.path("started").display(),
            scenario.path("release").display()
        ));
        let held = scenario.run_until(
            &[&scenario.issue_url(7), "harness", harness],
            &[],
            "started",
        );
        let status = scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&status.stdout).unwrap();
        let session = status["commands"][0]["session_log"].as_str().unwrap();
        // Only the tail is relevant; unknown, malformed and incomplete lines
        // must not hide the last useful event.
        let contents = format!(
            "{}\n{event}\n{{\"type\":\"future-event\"}}\nnot json\n{{\"partial\":",
            "x".repeat(100_000)
        );
        fs::write(session, contents).unwrap();
        let result = scenario.run(&["show", "widgets#7", "--json"]);
        let text = scenario.run(&["show", "widgets#7"]);
        fs::write(scenario.path("release"), "").unwrap();
        let ended = held.finish();
        assert_eq!(ended.code, Some(0), "{}", ended.stderr);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let shown: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(
            shown["command"]["latest_event"], "Bash: cargo test",
            "{harness}: {shown}"
        );
        assert!(text.stdout.contains("Bash: cargo test"), "{}", text.stdout);
        let ended = scenario.run(&["show", "widgets#7", "--json"]);
        let ended: Value = serde_json::from_str(&ended.stdout).unwrap();
        assert!(ended["command"]["latest_event"].is_null());
    }
}

#[test]
fn show_names_one_command_and_logs_prints_its_exact_output() {
    let scenario = Scenario::new();
    scenario.issue_titled(7, "Make widgets");
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let mut expected = None;
    for name in ["widgets#7", "acme/widgets#7", &scenario.issue_url(7)] {
        let result = scenario.run(&["show", name, "--json"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let shown: Value = serde_json::from_str(&result.stdout).unwrap();
        assert_eq!(shown["version"], 1);
        let command = &shown["command"];
        assert_eq!(command["issue"]["number"], 7);
        assert_eq!(command["outcome"], "ready for review");
        assert!(command["stages"].as_array().unwrap().iter().all(|stage| {
            chrono::DateTime::parse_from_rfc3339(stage["began_at"].as_str().unwrap()).is_ok()
        }));
        let identity = (command["process_id"].clone(), command["started_at"].clone());
        if let Some(expected) = &expected {
            assert_eq!(&identity, expected);
        }
        let log = fs::read_to_string(command["command_log"].as_str().unwrap()).unwrap();
        let result = scenario.run(&["logs", name]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(result.stdout, log);
        let text = scenario.run(&["show", name]);
        assert_eq!(text.code, Some(0), "{}", text.stderr);
        assert!(text.stdout.contains("Make widgets"), "{}", text.stdout);
        assert!(
            text.stdout
                .contains(command["command_log"].as_str().unwrap())
        );
        expected = Some(identity);
    }
}
