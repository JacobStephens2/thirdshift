//! The Activity log: `<logs.dir>/<owner>/<repo>/activity.log`, one running
//! record per repository of what the factory did there. A Run, a Spec run,
//! an Architect run or a Pickup run writes a line when it starts work,
//! naming its Command log, and one when it ends, with its outcome; the run a
//! command dispatched or started writes none of its own. A skipped Pickup
//! run or Architect run writes a line only when its reason differs from the
//! last line of its own kind. With `activity.quiet_skips`, a skipped pass
//! prints nothing at all, its Activity log line its only trace.

mod support;

use std::fs;

use chrono::{Duration, NaiveDateTime, Utc};
use support::{RunResult, Scenario};

/// A zone four hours behind UTC, with no daylight saving time, so every
/// line's local time is UTC less four hours.
const ZONE: (&str, &str) = ("TZ", "XYZ+4");

/// Where the logs are, the default `logs.dir`, relative to the scenario.
const LOGS_DIR: &str = "home/.thirdshift/logs";

/// The scenario's repository's Activity log, relative to the scenario.
const ACTIVITY_LOG: &str = "home/.thirdshift/logs/acme/widgets/activity.log";

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

const QUIET_SKIPS: &str = "[activity]\nquiet_skips = true\n";

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

/// Run thirdshift with `args` in [`ZONE`].
fn run_in_zone(scenario: &Scenario, args: &[&str]) -> RunResult {
    scenario.run_with_env(args, &[ZONE])
}

/// The Activity log's lines, each without the local date and time it starts
/// with, once that is checked to be a moment ago in [`ZONE`].
fn activity(scenario: &Scenario) -> Vec<String> {
    let log = fs::read_to_string(scenario.path(ACTIVITY_LOG)).unwrap_or_default();
    let local_now = Utc::now().naive_utc() - Duration::hours(4);
    log.lines()
        .map(|line| {
            let (time, rest) = line.split_at_checked(19).expect(line);
            let time = NaiveDateTime::parse_from_str(time, "%Y-%m-%d %H:%M:%S")
                .unwrap_or_else(|_| panic!("no local date and time: {line}"));
            let ago = local_now - time;
            assert!(
                ago >= Duration::zero() && ago < Duration::minutes(5),
                "not local time a moment ago: {line}"
            );
            rest.strip_prefix(' ').expect(line).to_string()
        })
        .collect()
}

/// The name of the one Command log in `commands/<folder>`.
fn the_one_command_log(scenario: &Scenario, folder: &str) -> String {
    let names = scenario.entries(&format!("{LOGS_DIR}/acme/widgets/commands/{folder}"));
    assert_eq!(names.len(), 1, "{names:?}");
    format!("commands/{folder}/{}", names[0])
}

/// Make issue `number` open and labelled `ready-for-agent`.
fn ready_issue(scenario: &Scenario, number: u32) {
    scenario.issue_is(number, "OPEN");
    scenario.issue_labelled(number, &["ready-for-agent"]);
}

/// Make issue `number` an open Architect plan, which skips an Architect run.
fn open_plan(scenario: &Scenario, number: u32) {
    scenario.issue_is(number, "OPEN");
    scenario.issue_labelled(number, &["architect-plan"]);
}

/// How a start line names the Harness, Model and Effort when nothing chose
/// them.
const ON_CLAUDE: &str = "on claude · default model · default effort";

#[test]
fn a_run_typed_by_hand_writes_its_start_naming_its_command_log_and_its_end_with_its_outcome() {
    let scenario = Scenario::new();
    scenario.agent_does(&agent_opens_pr(7, "main"));

    let result = run_in_zone(&scenario, &[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let command_log = the_one_command_log(&scenario, "issue");
    assert_eq!(
        activity(&scenario),
        [
            format!("Run #7 started: {command_log}, {ON_CLAUDE}"),
            format!("Run #7 ended: PR {PR_URL} is ready for review"),
        ]
    );
}

#[test]
fn a_failed_run_ends_its_line_with_its_cause() {
    let scenario = Scenario::new();
    scenario.agent_does("exit 3\n");

    let result = run_in_zone(&scenario, &[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let lines = activity(&scenario);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[1].starts_with("Run #7 ended: failed: ") && lines[1].contains("claude exited 3"),
        "{lines:?}"
    );
}

#[test]
fn a_spec_run_writes_its_lines_and_its_tickets_write_none() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[])]);
    scenario.agent_does_for(21, &agent_opens_pr(21, "issue-20"));

    let result = run_in_zone(&scenario, &[&scenario.issue_url(20)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let command_log = the_one_command_log(&scenario, "issue");
    let lines = activity(&scenario);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        lines[0],
        format!("Spec run #20 started: {command_log}, {ON_CLAUDE}")
    );
    assert!(lines[1].starts_with("Spec run #20 ended: PR "), "{lines:?}");
}

#[test]
fn a_pickup_run_that_takes_an_issue_names_it_and_the_run_it_dispatched_writes_nothing() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));

    let result = run_in_zone(&scenario, &["pickup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let command_log = the_one_command_log(&scenario, "pickup");
    assert_eq!(
        activity(&scenario),
        [
            format!("Pickup run #7 started: {command_log}, {ON_CLAUDE}"),
            format!("Pickup run #7 ended: PR {PR_URL} is ready for review"),
        ]
    );
}

#[test]
fn an_architect_run_that_dispatches_its_plan_ends_with_the_plan_and_that_runs_outcome() {
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

    let result = run_in_zone(&scenario, &["architect"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let command_log = the_one_command_log(&scenario, "architect");
    assert_eq!(
        activity(&scenario),
        [
            format!("Architect run started: {command_log}, {ON_CLAUDE}"),
            format!(
                "Architect run ended: plan {} dispatched: PR {PR_URL} is ready for review",
                scenario.issue_url(8)
            ),
        ]
    );
}

#[test]
fn the_first_skip_on_a_repository_creates_its_folder_and_repeated_skips_write_one_line() {
    let scenario = Scenario::new();
    assert!(!scenario.path(LOGS_DIR).exists());

    for _ in 0..3 {
        let result = run_in_zone(&scenario, &["pickup"]);

        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
        // Without activity.quiet_skips, a skip prints as ever.
        assert!(
            result
                .stderr
                .ends_with("thirdshift: no Ready issue on acme/widgets\n"),
            "stderr: {}",
            result.stderr
        );
    }

    assert_eq!(
        activity(&scenario),
        ["Pickup run skipped: no Ready issue on acme/widgets"]
    );
}

#[test]
fn a_skip_because_another_pass_is_running_is_recorded_like_any_other() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7);
    scenario.agent_does_for(
        7,
        &format!(
            r#"
root=$(dirname "$FAKE_CLAUDE_RECORD")
touch "$root/started"
while [ -d "$root" ] && [ ! -e "$root/release" ]; do sleep 0.05; done
{}"#,
            agent_opens_pr(7, "main")
        ),
    );
    let first = scenario.run_until(&["pickup"], &[ZONE], "started");

    let second = run_in_zone(&scenario, &["pickup"]);
    let third = run_in_zone(&scenario, &["architect"]);
    fs::write(scenario.path("release"), "").unwrap();
    let first = first.finish();

    for result in [&first, &second, &third] {
        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    }
    let lines = activity(&scenario);
    let running = "skipped: an Architect run or a Pickup run is already running on acme/widgets";
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[0].starts_with("Pickup run #7 started: "), "{lines:?}");
    assert_eq!(lines[1], format!("Pickup run {running}"));
    assert_eq!(lines[2], format!("Architect run {running}"));
    assert!(lines[3].starts_with("Pickup run #7 ended: "), "{lines:?}");
}

#[test]
fn an_activity_log_that_cannot_be_written_is_one_warning_and_changes_nothing_else() {
    let scenario = Scenario::new();
    scenario.agent_does(&agent_opens_pr(7, "main"));
    // A folder where the Activity log would go.
    fs::create_dir_all(scenario.path(ACTIVITY_LOG)).unwrap();

    let result = run_in_zone(&scenario, &[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    let warnings: Vec<_> = result
        .stderr
        .lines()
        .filter(|line| line.contains("warning:"))
        .collect();
    assert_eq!(warnings.len(), 1, "stderr: {}", result.stderr);
    assert!(
        warnings[0].starts_with("thirdshift: warning: could not keep the Activity log: "),
        "{}",
        warnings[0]
    );
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is ready for review").as_str())
    );
    the_one_command_log(&scenario, "issue");
}

#[test]
fn with_quiet_skips_a_skipped_pass_prints_nothing_and_still_writes_its_line() {
    let scenario = Scenario::new();
    scenario.user_config_is(QUIET_SKIPS);
    open_plan(&scenario, 5);

    let pickup = run_in_zone(&scenario, &["pickup"]);
    let architect = run_in_zone(&scenario, &["architect"]);

    for result in [&pickup, &architect] {
        assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "");
    }
    let lines = activity(&scenario);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        lines[0],
        "Pickup run skipped: no Ready issue on acme/widgets"
    );
    assert!(
        lines[1].starts_with("Architect run skipped: Architect plan #5 "),
        "{lines:?}"
    );
}

#[test]
fn with_quiet_skips_a_pass_that_works_prints_as_ever() {
    let scenario = Scenario::new();
    scenario.user_config_is(QUIET_SKIPS);
    scenario.issue_is(6, "OPEN");
    scenario.issue_labelled(6, &["ready-for-agent", "needs-info"]);
    ready_issue(&scenario, 7);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));

    let result = run_in_zone(&scenario, &["pickup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    let mut lines = result.stderr.lines();
    assert!(
        lines
            .next()
            .is_some_and(|line| line.starts_with("thirdshift: Pickup run starting, ")),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(lines.next(), Some("thirdshift: #6 labelled needs-info"));
    let command_log = scenario
        .path(LOGS_DIR)
        .join("acme/widgets")
        .join(the_one_command_log(&scenario, "pickup"));
    assert_eq!(
        fs::read_to_string(command_log).unwrap(),
        format!("{}{}", result.stamped_stderr, result.stdout)
    );
}

#[test]
fn with_quiet_skips_a_pass_that_fails_prints_its_lines_and_its_error() {
    let scenario = Scenario::new();
    scenario.user_config_is(QUIET_SKIPS);
    scenario.gh_fails("issue list");

    let result = run_in_zone(&scenario, &["pickup"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result
            .stderr
            .starts_with("thirdshift: Pickup run starting, "),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.lines().count() > 1,
        "stderr: {}",
        result.stderr
    );
}
