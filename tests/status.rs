mod support;

use serde_json::Value;
use std::fs;
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";
const OPENS_PR: &str = "echo work > work.txt\ngit add work.txt\ngit commit -qm 'Make widgets'\ngh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'\n";

fn retained(scenario: &Scenario) -> Value {
    let files: Vec<_> = scenario
        .entries(LOGS)
        .into_iter()
        .filter(|name| name.ends_with(".command.json"))
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    serde_json::from_str(&fs::read_to_string(scenario.path(LOGS).join(&files[0])).unwrap()).unwrap()
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
            .ends_with("-implement.jsonl")
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
    let record = retained(&scenario);
    assert_eq!(record["outcome"], "ready for review");
    assert_eq!(record["cause"], Value::Null);
    assert_eq!(record["issue_branch"], "issue-7");
    assert_eq!(
        record["pull_request"],
        "https://github.com/acme/widgets/pull/1"
    );
}

#[test]
fn a_merge_run_retains_delivery_stages_and_their_start_times() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&["merge", &scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let record = retained(&scenario);
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
                .ends_with("-implement.jsonl")
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
    let record = retained(&scenario);
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

#[test]
fn status_shows_recent_outcomes_and_all_history_newest_first() {
    let scenario = Scenario::new();
    let now = chrono::Local::now();
    let recent = (now - chrono::Duration::hours(1)).to_rfc3339();
    let old = (now - chrono::Duration::days(2)).to_rfc3339();
    // The recent completion began before the old completion: filter by end,
    // and order history by start, rather than by directory or end time.
    let earliest = (now - chrono::Duration::days(3)).to_rfc3339();
    write_record(&scenario, 7, &earliest, Some(&recent));
    write_record(&scenario, 8, &old, Some(&old));
    let result = scenario.run(&["status", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(status["commands"].as_array().unwrap().len(), 1);
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
        assert_eq!(issues, [8, 7]);
    }
}

fn fixture_log(scenario: &Scenario, relative: &str, contents: &str) {
    let path = scenario
        .path("home/.thirdshift/logs/acme/widgets")
        .join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
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
    let result = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let status: Value = serde_json::from_str(&result.stdout).unwrap();
    let commands = status["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0]["kind"], "Security run");
    assert_eq!(commands[0]["issue"], Value::Null);
    assert_eq!(commands[0]["outcome"], "findings recorded");
    assert_eq!(commands[1]["started_at"], "2026-10-02T12:00:00-04:00");
    assert_eq!(commands[1]["ended_at"], "2026-10-02T13:00:00-04:00");
    assert_eq!(
        commands[1]["outcome"],
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
        "7-20261001T120000-0400-implement.jsonl",
        "8-20261002T120000-0400-spec-review.jsonl",
        "10-20261002T120000-0400-implement.jsonl", // Ticket belongs to Spec #8.
        "9-20260901T120000-0400-implement.jsonl",
        "9-20260901T120000-0400-implement-resume.jsonl",
        "9-20260901T120000-0400-repair-1.jsonl",
        "9-20260902T120000-0400-implement.jsonl",
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
        "sessions/widgets-4-20260801T120000-0400-implement.jsonl",
        "commands/issue/5-20260801T120000-0400-implement.jsonl",
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
        retained(&scenario)["issue_branch"],
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
        retained(&scenario)["issue_branch"],
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
        retained(&scenario)["issue_branch"],
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
    assert_eq!(retained(&scenario)["outcome"], Value::Null);
}

#[test]
fn status_reads_all_repositories_under_configured_logs_and_ignores_unknown_stages() {
    let scenario = Scenario::new();
    scenario.agent_does(OPENS_PR);
    let ended = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let mut record = retained(&scenario);
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
fn repair_resume_and_security_review_keep_their_own_session_logs() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(1, &support::leaves_running("cargo test"));
    scenario.agent_does_in_session(2, &format!("{OPENS_PR}gh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\":\"test\",\"conclusion\":\"failure\"}}]'\n"));
    scenario.agent_does_in_session(3, "printf '%s\\n' 'Security review: {\"unaddressed_count\":0,\"findings\":[],\"pre_existing_count\":0}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n");
    scenario.agent_does_in_session(4, "echo fix > fix.txt\ngit add fix.txt\ngit commit -qm 'Fix CI'\ngh fake checks \"$(git rev-parse HEAD)\" '[{\"name\":\"test\",\"conclusion\":\"success\"}]'\n");
    let ended = scenario.run(&[&scenario.issue_url(7), "security-review"]);
    assert_eq!(ended.code, Some(0), "{}", ended.stderr);
    let record = retained(&scenario);
    let stages = record["stages"].as_array().unwrap();
    for (name, log) in [
        ("Resume", "-implement-resume.jsonl"),
        ("Security review", "-security-review.jsonl"),
        ("Repair", "-repair-1.jsonl"),
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
