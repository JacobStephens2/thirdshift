//! Command logs: each Run, Spec run, Architect run and Pickup run keeps a file
//! of everything it printed, stderr and stdout in order, under
//! `<logs.dir>/<owner>/<repo>/commands/<command>/`, with its Session logs
//! beside them in that folder, all stamped with the
//! command's start in local time and its UTC offset. A child Run keeps none: its lines are in the
//! Command log of what started it. A Pickup run or Architect run skipped
//! before doing any work keeps none, nor does a Run that fails before it
//! starts work.

mod support;

use std::fs;
use std::path::PathBuf;

use chrono::NaiveDateTime;
use support::resend::ResendStandIn;
use support::{RunResult, Scenario};

/// A zone four hours behind UTC, with no daylight saving time, so every
/// stamp has the offset `-0400`.
const ZONE: (&str, &str) = ("TZ", "XYZ+4");

const OFFSET: &str = "-0400";

/// Where the logs are, the default `logs.dir`, relative to the scenario.
const LOGS_DIR: &str = "home/.thirdshift/logs";

/// Where the scenario's repository's logs are, under [`LOGS_DIR`].
const LOGS: &str = "home/.thirdshift/logs/acme/widgets";

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

/// A script in which the agent for issue `issue` commits its work and opens
/// its PR into `base`, leaving the pushing to thirdshift.
fn agent_opens_pr(issue: u32, base: &str) -> String {
    format!(
        r#"
echo "{issue}" > issue-{issue}.txt
git add issue-{issue}.txt
git commit -q -m "Work on {issue}"
gh pr create --base {base} --head issue-{issue} --title "Work on {issue}" --body "Closes #{issue}"
"#
    )
}

/// Run thirdshift with `args` in [`ZONE`], with `env` too.
fn run_in_zone(scenario: &Scenario, args: &[&str], env: &[(&str, &str)]) -> RunResult {
    scenario.run_with_env(args, &[&[ZONE], env].concat())
}

/// The names of the files in `folder` under the logs, sorted, none if it is
/// missing.
fn logs_in(scenario: &Scenario, folder: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(scenario.path(LOGS).join(folder)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// The one Command log in `commands/<folder>`, which must be named
/// `<prefix><stamp>.log`, and its stamp.
fn the_one_command_log(scenario: &Scenario, folder: &str, prefix: &str) -> (PathBuf, String) {
    let logs: Vec<_> = logs_in(scenario, &format!("commands/{folder}"))
        .into_iter()
        .filter(|name| name.ends_with(".log"))
        .collect();
    assert_eq!(logs.len(), 1, "Command logs: {logs:?}");
    let stamp = logs[0]
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(".log"))
        .unwrap_or_else(|| panic!("not {prefix}<stamp>.log: {}", logs[0]));
    assert_local_stamp(stamp);
    let path = scenario
        .path(LOGS)
        .join("commands")
        .join(folder)
        .join(&logs[0]);
    (path, stamp.to_string())
}

/// Assert `stamp` is a local time with [`OFFSET`], as in
/// `20261003T120000-0400`.
fn assert_local_stamp(stamp: &str) {
    let (time, offset) = stamp.split_at(stamp.len() - OFFSET.len());
    assert_eq!(offset, OFFSET, "{stamp}");
    assert!(
        NaiveDateTime::parse_from_str(time, "%Y%m%dT%H%M%S").is_ok(),
        "{stamp}"
    );
}

/// Assert the Command log at `path` holds what the command printed: its
/// stderr, then its stdout, which only a command's last lines put there.
fn assert_holds_what_was_printed(path: &PathBuf, result: &RunResult) {
    let log = fs::read_to_string(path).unwrap();
    assert_eq!(log, format!("{}{}", result.stamped_stderr, result.stdout));
}

/// Assert the first line on stderr is `<starting>, <date> -0400`.
fn assert_dated_first_line(result: &RunResult, starting: &str) {
    let first = result.stderr.lines().next().unwrap();
    let date = first
        .strip_prefix(&format!("thirdshift: {starting}, "))
        .and_then(|rest| rest.strip_suffix(&format!(" {OFFSET}")))
        .unwrap_or_else(|| panic!("not dated: {first}"));
    assert!(
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok(),
        "{first}"
    );
}

#[test]
fn a_run_keeps_a_command_log_of_everything_it_printed_and_its_session_logs_share_its_stamp() {
    let scenario = Scenario::new();
    scenario.agent_does(&agent_opens_pr(7, "main"));

    let result = run_in_zone(&scenario, &[&scenario.issue_url(7)], &[]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (path, stamp) = the_one_command_log(&scenario, "issue", "7-");
    assert_holds_what_was_printed(&path, &result);
    assert_dated_first_line(&result, &format!("starting on {}", scenario.issue_url(7)));
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: logging this command to {}\n",
            path.display()
        )),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        logs_in(&scenario, "commands/issue"),
        [
            format!("7-{stamp}-implement.jsonl"),
            format!("7-{stamp}.log")
        ]
    );
    assert_eq!(logs_in(&scenario, ""), ["activity.log", "commands"]);
    assert_eq!(
        fs::read_dir(scenario.path(LOGS_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        ["acme"]
    );
}

#[test]
fn a_spec_runs_command_log_has_its_tickets_lines_and_its_tickets_keep_none() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[]), (22, &[21])]);
    scenario.agent_does_for(21, &agent_opens_pr(21, "issue-20"));
    scenario.agent_does_for(22, &agent_opens_pr(22, "issue-20"));

    let result = run_in_zone(&scenario, &[&scenario.issue_url(20)], &[]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (path, stamp) = the_one_command_log(&scenario, "issue", "20-");
    assert_holds_what_was_printed(&path, &result);
    let log = fs::read_to_string(&path).unwrap();
    for ticket in [21, 22] {
        assert!(
            log.contains(&format!("#{ticket}: implement: session started\n")),
            "log: {log}"
        );
    }
    assert_eq!(
        scenario.log_files(&format!("{LOGS}/commands/issue"), "jsonl"),
        [
            format!("20-{stamp}-spec-review.jsonl"),
            format!("21-{stamp}-implement.jsonl"),
            format!("22-{stamp}-implement.jsonl"),
        ]
    );
}

#[test]
fn a_pickup_run_that_takes_an_issue_keeps_a_command_log_from_its_first_line() {
    let scenario = Scenario::new();
    scenario.issue_is(6, "OPEN");
    scenario.issue_labelled(6, &["ready-for-agent", "needs-info"]);
    scenario.issue_is(7, "OPEN");
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));

    let result = run_in_zone(&scenario, &["pickup"], &[]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (path, stamp) = the_one_command_log(&scenario, "pickup", "7-");
    assert_holds_what_was_printed(&path, &result);
    assert_dated_first_line(&result, "Pickup run starting");
    let log = fs::read_to_string(&path).unwrap();
    let passed_over = log.find("#6 labelled needs-info\n").expect(&log);
    let logging = log.find("logging this command to").expect(&log);
    assert!(passed_over < logging, "log: {log}");
    assert_eq!(
        logs_in(&scenario, "commands/pickup"),
        [
            format!("7-{stamp}-implement.jsonl"),
            format!("7-{stamp}.log")
        ]
    );
}

#[test]
fn a_pickup_spec_keeps_its_tickets_session_logs_beside_its_command_log() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[]), (22, &[21])]);
    scenario.issue_labelled(20, &["ready-for-agent"]);
    scenario.agent_does_for(21, &agent_opens_pr(21, "issue-20"));
    scenario.agent_does_for(22, &agent_opens_pr(22, "issue-20"));

    let result = run_in_zone(&scenario, &["pickup"], &[]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (_, stamp) = the_one_command_log(&scenario, "pickup", "20-");
    assert_eq!(
        logs_in(&scenario, "commands/pickup"),
        [
            format!("20-{stamp}-spec-review.jsonl"),
            format!("20-{stamp}.log"),
            format!("21-{stamp}-implement.jsonl"),
            format!("22-{stamp}-implement.jsonl"),
        ]
    );
    assert!(!scenario.path(LOGS).join("sessions").exists());
    assert!(!scenario.path(LOGS).join("commands/issue").exists());
}

#[test]
fn an_architect_runs_command_log_covers_the_run_its_plan_was_dispatched_as() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    scenario.agent_does_in_session(
        1,
        r#"
url=$(gh issue create --title "Deepen the session module" --body "The plan" --label needs-triage)
printf 'Architecture review plan: %s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(8, &agent_opens_pr(8, "main"));

    let result = run_in_zone(&scenario, &["architect"], &[]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (path, stamp) = the_one_command_log(&scenario, "architect", "");
    assert_holds_what_was_printed(&path, &result);
    assert_dated_first_line(&result, "Architect run starting");
    let log = fs::read_to_string(&path).unwrap();
    assert!(
        log.ends_with(&format!("PR {PR_URL} is ready for review\n{PR_URL}\n")),
        "log: {log}"
    );
    assert_eq!(
        scenario.log_files(&format!("{LOGS}/commands/architect"), "jsonl"),
        [
            format!("8-{stamp}-implement.jsonl"),
            format!("architect-{stamp}-architecture-review.jsonl"),
        ]
    );
}

#[test]
fn a_failed_run_ends_by_naming_its_session_log_then_its_command_log() {
    let scenario = Scenario::new();
    scenario.agent_does("exit 3\n");

    let result = run_in_zone(&scenario, &[&scenario.issue_url(7)], &[]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (path, stamp) = the_one_command_log(&scenario, "issue", "7-");
    let session = scenario
        .path(LOGS)
        .join(format!("commands/issue/7-{stamp}-implement.jsonl"));
    let ending: Vec<_> = result.stderr.lines().rev().take(2).collect();
    assert_eq!(
        ending,
        [
            format!("thirdshift: command log: {}", path.display()),
            format!("thirdshift: session log: {}", session.display()),
        ]
    );
    assert_holds_what_was_printed(&path, &result);
}

#[test]
fn a_run_notification_names_the_command_log() {
    let scenario = Scenario::new();
    scenario.agent_does(&agent_opens_pr(7, "main"));
    let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

    let result = run_in_zone(
        &scenario,
        &[&scenario.issue_url(7), "--email", "me@example.com"],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_1"),
        ],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (path, _) = the_one_command_log(&scenario, "issue", "7-");
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(
        text.contains(&format!("\nCommand log:  {}\n", path.display())),
        "{text}"
    );
}

#[test]
fn an_obstructed_commands_folder_warns_and_prevents_session_logging() {
    let scenario = Scenario::new();
    scenario.agent_does(&agent_opens_pr(7, "main"));
    // Command and Session logs now share this folder.
    fs::create_dir_all(scenario.path(LOGS)).unwrap();
    fs::write(scenario.path(LOGS).join("commands"), "").unwrap();

    let result = run_in_zone(&scenario, &[&scenario.issue_url(7)], &[]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(
        result
            .stderr
            .matches("warning: could not keep the Command log:")
            .count(),
        1
    );
    assert!(
        result.stderr.contains("could not create"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert!(!scenario.path(LOGS).join("sessions").exists());
}

#[test]
fn a_run_that_fails_on_origin_match_keeps_no_command_log_and_writes_no_activity_log_line() {
    let scenario = Scenario::new();

    let result = run_in_zone(
        &scenario,
        &["https://github.com/other/widgets/issues/7"],
        &[],
    );

    scenario.assert_rejected_before_any_work(&result, "origin mismatch");
    assert_dated_first_line(
        &result,
        "starting on https://github.com/other/widgets/issues/7",
    );
    assert!(!result.stderr.contains("logging this command to"));
    assert!(!result.stderr.contains("Command log:"));
    // Neither the issue's repository's logs nor the Launch directory's.
    assert!(!scenario.path(LOGS_DIR).exists());
}

#[test]
fn a_command_that_does_no_work_keeps_no_command_log() {
    let scenario = Scenario::new();

    let result = run_in_zone(&scenario, &["version"], &[]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stderr, "");
    assert!(!scenario.path(LOGS_DIR).exists());
}

#[test]
fn a_base_fixs_lines_are_in_the_command_log_of_the_run_that_started_it() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
gh fake checks "$(git rev-parse HEAD)" '[{"name": "test", "conclusion": "failure", "url": "https://ci.example/test"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name": "test", "conclusion": "failure", "url": "https://ci.example/main/test"}]'
"#,
    );
    scenario.agent_does_for(
        8,
        &format!(
            "{}gh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\": \"test\", \"conclusion\": \"success\"}}]'\n",
            agent_opens_pr(8, "main")
        ),
    );

    let result = run_in_zone(&scenario, &[&scenario.issue_url(7), "base-fix"], &[]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let (path, stamp) = the_one_command_log(&scenario, "issue", "7-");
    assert_holds_what_was_printed(&path, &result);
    let log = fs::read_to_string(&path).unwrap();
    assert!(
        log.contains("#8: implement: session started\n"),
        "log: {log}"
    );
    assert!(
        scenario
            .log_files(&format!("{LOGS}/commands/issue"), "jsonl")
            .contains(&format!("8-{stamp}-implement.jsonl")),
        "{:?}",
        scenario.log_files(&format!("{LOGS}/commands/issue"), "jsonl")
    );
}
