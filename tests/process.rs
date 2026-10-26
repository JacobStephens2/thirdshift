//! Custom processes through the CLI, real Git and fake Harnesses.

mod support;

use std::fs;
use std::path::Path;

use support::{RunResult, Scenario};

const REVIEW: &str = "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
skill = \"code-review\"
";

fn write_process(scenario: &Scenario, name: &str, body: &str) {
    let dir = scenario.path("home/.thirdshift/processes");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{name}.toml")), body).unwrap();
}

fn process_dir(scenario: &Scenario) -> std::path::PathBuf {
    scenario.path("home/.thirdshift/logs/acme/widgets/commands/process")
}

fn process_record(scenario: &Scenario) -> serde_json::Value {
    let dir = process_dir(scenario);
    let path = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("no process logs in {}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".command.json"))
        })
        .unwrap_or_else(|| panic!("no command record in {}", dir.display()));
    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap()
}

fn command_log(scenario: &Scenario) -> (std::path::PathBuf, String) {
    let dir = process_dir(scenario);
    let path = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "log"))
        .unwrap_or_else(|| panic!("no command log in {}", dir.display()));
    let text = fs::read_to_string(&path).unwrap();
    (path, text)
}

fn assert_no_session(scenario: &Scenario, result: &RunResult) {
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(scenario.claude_calls().is_empty(), "a session started");
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
    assert!(!process_dir(scenario).exists());
}

#[test]
fn process_lists_custom_processes_with_their_descriptions() {
    let scenario = Scenario::new();
    let empty = scenario.run(&["process"]);
    assert_eq!(empty.code, Some(0), "{}", empty.stderr);
    assert_eq!(empty.stdout, "");

    write_process(&scenario, "review", REVIEW);
    write_process(
        &scenario,
        "gather",
        "\
version = 1
description = \"Gather notes\"

[[steps]]
name = \"gather\"
skill = \"to-spec\"
",
    );
    write_process(&scenario, "broken", "description = \"not a process\"\n");
    write_process(&scenario, "check", REVIEW);

    let result = scenario.run(&["process"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        result.stdout,
        "gather: Gather notes\nreview: Review the diff\n"
    );
}

#[test]
fn check_and_run_refuse_a_broken_file_without_a_session() {
    for check in [true, false] {
        let cases = [
            (
                "review",
                Some("version = 2\ndescription = \"Review the diff\"\n"),
                ":1: version: version must be 1",
            ),
            (
                "review",
                Some("description = \"Review the diff\"\n"),
                ":1: version: version = 1 is required",
            ),
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"
extra = true
",
                ),
                ":3: unknown key: extra",
            ),
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
",
                ),
                ":5: exactly one kind:",
            ),
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
bogus = \"echo hi\"
",
                ),
                ":6: unknown key: bogus",
            ),
            (
                "ticket-run",
                None,
                "processes/ticket-run.toml:1: reserved name: ticket-run",
            ),
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
skill = \"not-a-skill\"
",
                ),
                ":6: skill not found:",
            ),
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
skill = \"code-review\"
command = \"echo hi\"
",
                ),
                ":7: exactly one kind: step review has more than one kind",
            ),
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
command = \"\"
",
                ),
                ":6: exactly one kind: command must not be empty",
            ),
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
command = 1
",
                ),
                ":6: exactly one kind: command must be a string",
            ),
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
optional = \"yes\"
skill = \"code-review\"
",
                ),
                ":6: optional: optional must be a boolean",
            ),
        ];
        for (name, body, expect) in cases {
            let scenario = Scenario::new();
            if let Some(body) = body {
                write_process(&scenario, name, body);
            }
            let result = if check {
                scenario.run(&["process", "check", name])
            } else {
                scenario.run(&["process", name])
            };
            assert_no_session(&scenario, &result);
            assert!(
                result.stderr.contains(expect),
                "check={check} {name}: expected {expect} in {}",
                result.stderr
            );
            if expect.contains("skill not found") {
                assert!(result.stderr.contains("unknown skill"), "{}", result.stderr);
                assert!(
                    result.stderr.contains("available skills:"),
                    "{}",
                    result.stderr
                );
                assert_eq!(
                    scenario
                        .launch_git(&["worktree", "list", "--porcelain"])
                        .matches("worktree ")
                        .count(),
                    1
                );
            }
        }
    }
}

#[test]
fn steps_run_detached_at_the_base_branch_tip() {
    let scenario = Scenario::new();
    scenario.origin_has_commit(
        "main",
        "base-only.txt",
        "from-base\n",
        "Move the Base branch",
    );
    let base = scenario.origin_git(&["rev-parse", "refs/heads/main"]);
    assert_ne!(
        base.trim(),
        scenario.launch_git(&["rev-parse", "HEAD"]).trim()
    );
    write_process(&scenario, "notes", REVIEW);
    scenario.agent_does(
        "test -f base-only.txt\n\
         printf '%s' \"$(git rev-parse HEAD)\" > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
    );

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.trim(), base.trim());
}

#[test]
fn a_missing_file_and_a_syntax_error_are_not_version_refusals() {
    let missing = Scenario::new();
    let absent = missing.run(&["process", "check", "missing"]);
    assert_eq!(absent.code, Some(1), "{}", absent.stderr);
    assert!(
        !absent.stderr.contains(": version:"),
        "missing file uses the version rule: {}",
        absent.stderr
    );
    assert!(missing.claude_calls().is_empty(), "{}", absent.stderr);

    let broken = Scenario::new();
    write_process(
        &broken,
        "review",
        "version = 1\ndescription = \"Review the diff\"\n[[steps]\n",
    );
    let result = broken.run(&["process", "check", "review"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        !result.stderr.contains(": version:"),
        "syntax error uses the version rule: {}",
        result.stderr
    );
}

#[test]
fn a_reserved_name_is_refused_for_every_built_in() {
    for name in [
        "check",
        "list",
        "ticket-run",
        "spec-run",
        "architect-run",
        "pickup-run",
        "security-run",
    ] {
        let scenario = Scenario::new();
        let result = if name == "check" {
            scenario.run(&["process", "check", "check"])
        } else {
            scenario.run(&["process", name])
        };
        assert_no_session(&scenario, &result);
        assert!(
            result.stderr.contains("reserved name"),
            "{name}: {}",
            result.stderr
        );
    }
}

#[test]
fn process_arguments_that_are_not_a_file_name_are_usage_errors() {
    let scenario = Scenario::new();
    for args in [
        vec!["process", "check"],
        vec!["process", "nested/name"],
        vec!["process", "check", "nested/name"],
        vec!["process", "check", "review", "extra"],
    ] {
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(2), "{args:?}: {}", result.stderr);
        assert!(
            result.stderr.contains("usage:"),
            "{args:?}: {}",
            result.stderr
        );
        assert!(scenario.claude_calls().is_empty());
    }
}

#[test]
fn check_prints_the_plan_and_starts_no_session() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "notes",
        "\
version = 1
description = \"Gather notes, then review them\"

[[steps]]
name = \"gather\"
skill = \"to-spec\"

[[steps]]
name = \"review\"
skill = \"code-review\"
",
    );

    let result = scenario.run(&["process", "check", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty(), "a session started");
    assert!(
        result
            .stdout
            .contains("notes: Gather notes, then review them")
    );
    assert!(
        result
            .stdout
            .contains("throwaway worktree at the Base branch")
    );
    assert!(result.stdout.contains("claude"));
    assert!(result.stdout.contains("gather: thirdshift-to-spec"));
    assert!(result.stdout.contains("review: thirdshift-code-review"));
    assert!(result.stdout.contains("SKILL.md"), "{}", result.stdout);
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    assert!(!process_dir(&scenario).exists());
}

#[test]
fn a_two_step_process_passes_the_first_result_to_the_second_session() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "notes",
        "\
version = 1
description = \"Gather notes, then review them\"

[[steps]]
name = \"gather\"
skill = \"to-spec\"

[[steps]]
name = \"judge\"
skill = \"code-review\"
",
    );
    scenario.agent_does_in_session(1, "printf 'gathered' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    scenario.agent_does_in_session(
        2,
        "\
path=$(printf '%s\\n' \"$FAKE_CLAUDE_PROMPT\" | sed -n 's/^Earlier Step result gather: //p')
test -n \"$path\"
test \"$(cat \"$path\")\" = 'gathered'
printf 'judged' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
    );
    let before = scenario.launch_git(&["status", "--porcelain", "--untracked-files=all"]);

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "judged\n");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "{}", result.stderr);
    assert_eq!(calls[0]["cwd"], calls[1]["cwd"]);
    let cwd = Path::new(calls[0]["cwd"].as_str().unwrap());
    assert_ne!(cwd, scenario.launch_dir());
    assert!(!cwd.exists(), "worktree was kept: {}", result.stderr);
    let first = calls[0]["prompt"].as_str().unwrap();
    assert!(
        first.contains("This is the first Step. No earlier Step has a result."),
        "{first}"
    );
    assert!(first.contains("thirdshift-to-spec"), "{first}");
    let second = calls[1]["prompt"].as_str().unwrap();
    assert!(second.contains("Earlier Step result gather:"), "{second}");
    assert!(!second.contains("This is the first Step"), "{second}");
    let (log_path, log) = command_log(&scenario);
    let gathered = fs::read_to_string(log_path.with_extension("steps").join("gather.txt")).unwrap();
    let judged = fs::read_to_string(log_path.with_extension("steps").join("judge.txt")).unwrap();
    assert_eq!(gathered, "gathered");
    assert_eq!(judged, "judged");
    assert!(step_line(&log, "gather").contains("succeeded"), "{log}");
    assert!(step_line(&log, "judge").contains("succeeded"), "{log}");
    assert!(result.stderr.contains("step gather:"), "{}", result.stderr);
    assert!(result.stderr.contains("step judge:"), "{}", result.stderr);
    let record = process_record(&scenario);
    assert_eq!(record["kind"], "Process");
    assert!(
        record["name"]
            .as_str()
            .unwrap()
            .starts_with("process-notes-"),
        "{}",
        record["name"]
    );
    assert_eq!(record["outcome"], "succeeded");
    assert!(record["cause"].is_null(), "{}", record["cause"]);
    assert_eq!(
        scenario.launch_git(&["status", "--porcelain", "--untracked-files=all"]),
        before
    );
    assert_eq!(
        scenario
            .launch_git(&["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
}

#[test]
fn check_prints_a_command_step_and_does_not_run_it() {
    let scenario = Scenario::new();
    let marker = scenario.path("command-ran");
    write_process(
        &scenario,
        "notes",
        &format!(
            "\
version = 1
description = \"Touch a file\"

[[steps]]
name = \"gather\"
command = \"touch {}\"
optional = true
",
            marker.display()
        ),
    );

    let result = scenario.run(&["process", "check", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(!marker.exists(), "check ran the command");
    assert!(scenario.claude_calls().is_empty(), "a session started");
    assert!(
        result.stdout.contains(&format!(
            "gather: command touch {} (optional)",
            marker.display()
        )),
        "{}",
        result.stdout
    );
    assert!(
        result
            .stdout
            .contains("throwaway worktree at the Base branch")
    );
    assert!(!process_dir(&scenario).exists());
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
}

#[test]
fn a_command_step_runs_in_the_worktree_and_a_later_session_reads_its_output() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "notes",
        r#"
version = 1
description = "Gather, then read"

[[steps]]
name = "gather"
command = """
printf 'from-command\n' > marker.txt
printf 'cwd=%s\n' "$(pwd)"
printf 'results=%s\n' "$THIRDSHIFT_RESULTS_DIR"
printf 'result=%s\n' "$THIRDSHIFT_STEP_RESULT"
if [ -n "${THIRDSHIFT_ISSUE_URL+x}" ]; then printf 'issue=set\n'; else printf 'issue=unset\n'; fi
printf 'stderr-note\n' >&2
"""

[[steps]]
name = "read"
skill = "code-review"
"#,
    );
    scenario.agent_does(
        r#"
path=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Earlier Step result gather: //p')
test -n "$path"
expected=$(printf 'cwd=%s\nresults=%s\nresult=%s\nissue=unset\nstderr-note\n' "$(pwd)" "$(dirname "$path")" "$path")
test "$(cat "$path")" = "$expected"
test "$(cat marker.txt)" = "from-command"
rm marker.txt
printf 'read' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "read\n");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1, "{}", result.stderr);
    let cwd = Path::new(calls[0]["cwd"].as_str().unwrap());
    assert_ne!(cwd, scenario.launch_dir());
    assert!(!cwd.exists(), "worktree was kept: {}", result.stderr);
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(prompt.contains("Earlier Step result gather:"), "{prompt}");
    assert!(!prompt.contains("This is the first Step"), "{prompt}");
    let (log_path, log) = command_log(&scenario);
    let gathered = fs::read_to_string(log_path.with_extension("steps").join("gather.txt")).unwrap();
    assert!(gathered.contains("issue=unset\n"), "{gathered}");
    assert!(gathered.contains("stderr-note\n"), "{gathered}");
    assert!(step_line(&log, "gather").contains("succeeded"), "{log}");
    assert!(step_line(&log, "read").contains("succeeded"), "{log}");
    assert_eq!(scenario.launch_git(&["status", "--porcelain"]), "");
}

#[test]
fn an_optional_step_failure_is_recorded_and_the_process_succeeds() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "notes",
        r#"
version = 1
description = "Keep going after an optional failure"

[[steps]]
name = "flaky"
command = "printf 'partial\n' >&2; exit 9"
optional = true

[[steps]]
name = "after"
skill = "code-review"
"#,
    );
    scenario.agent_does(
        r#"
printf '%s\n' "$FAKE_CLAUDE_PROMPT" | grep -q 'Earlier Step result flaky:' && exit 1
printf '%s\n' "$FAKE_CLAUDE_PROMPT" | grep -q 'This is the first Step' && exit 1
printf '%s\n' "$FAKE_CLAUDE_PROMPT" | grep -q 'No earlier Step has a result.'
printf 'kept' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "kept\n");
    assert_eq!(scenario.claude_calls().len(), 1, "{}", result.stderr);
    let (_, log) = command_log(&scenario);
    let failed = step_line(&log, "flaky");
    assert!(failed.contains("failed"), "{log}");
    assert!(failed.contains("exit status 9"), "{failed}");
    assert!(step_line(&log, "after").contains("succeeded"), "{log}");
    let record = process_record(&scenario);
    assert_eq!(record["outcome"], "succeeded");
    assert!(record["cause"].is_null(), "{}", record["cause"]);
}

#[test]
fn a_failing_command_stops_the_process_and_the_log_names_its_exit_status() {
    let scenario = Scenario::new();
    let marker = scenario.path("later-step");
    write_process(
        &scenario,
        "notes",
        &format!(
            "\
version = 1
description = \"Fail, then continue\"

[[steps]]
name = \"fail\"
command = \"printf 'nope\\n' >&2; exit 4\"
optional = false

[[steps]]
name = \"later\"
command = \"touch {}\"

[[steps]]
name = \"read\"
skill = \"code-review\"
",
            marker.display()
        ),
    );
    scenario.agent_does("printf 'read' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(!marker.exists(), "a later command ran");
    assert!(scenario.claude_calls().is_empty(), "a later session ran");
    let (_, log) = command_log(&scenario);
    let failed = step_line(&log, "fail");
    assert!(failed.contains("failed"), "{log}");
    assert!(failed.contains("exit status 4"), "{failed}");
    assert!(!log.contains("step later:"), "{log}");
    assert!(!log.contains("step read:"), "{log}");
    let record = process_record(&scenario);
    assert_eq!(record["outcome"], "failed");
}

#[test]
fn a_failed_step_fails_the_process_and_later_steps_do_not_run() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "notes",
        "\
version = 1
description = \"Gather, judge, then publish\"

[[steps]]
name = \"gather\"
skill = \"to-spec\"

[[steps]]
name = \"judge\"
skill = \"code-review\"

[[steps]]
name = \"publish\"
skill = \"pr\"
",
    );
    scenario.agent_does_in_session(1, "printf 'gathered' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    scenario.agent_does_in_session(2, "exit 1");
    scenario.agent_does_in_session(3, "printf 'published' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(scenario.claude_calls().len(), 2, "{}", result.stderr);
    assert!(result.stderr.contains("step judge:"), "{}", result.stderr);
    assert!(result.stderr.contains("failed"), "{}", result.stderr);
    let (log_path, log) = command_log(&scenario);
    assert!(step_line(&log, "gather").contains("succeeded"), "{log}");
    assert!(step_line(&log, "judge").contains("failed"), "{log}");
    assert!(!log.contains("step publish:"), "{log}");
    assert_eq!(
        fs::read_to_string(log_path.with_extension("steps").join("gather.txt")).unwrap(),
        "gathered"
    );
    let record = process_record(&scenario);
    assert_eq!(record["kind"], "Process");
    assert_eq!(record["outcome"], "failed");
    assert!(
        record["cause"]
            .as_str()
            .is_some_and(|cause| !cause.is_empty()),
        "{record}"
    );
}

#[test]
fn a_worktree_with_uncommitted_changes_is_kept_and_the_launch_checkout_is_not() {
    let scenario = Scenario::new();
    write_process(&scenario, "notes", REVIEW);
    scenario.agent_does(
        "echo 'skill work' > result.txt\nprintf 'Done.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );
    let before = scenario.launch_git(&["status", "--porcelain", "--untracked-files=all"]);

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "Done.\n");
    let calls = scenario.claude_calls();
    let cwd = Path::new(calls[0]["cwd"].as_str().unwrap());
    assert!(cwd.exists(), "worktree was deleted");
    assert_eq!(
        fs::read_to_string(cwd.join("result.txt")).unwrap(),
        "skill work\n"
    );
    assert_eq!(
        scenario.launch_git(&["status", "--porcelain", "--untracked-files=all"]),
        before
    );
}

#[test]
fn a_process_waits_for_resources_before_its_first_session() {
    let scenario = Scenario::new();
    scenario.user_config_is("[resources]\nmin_available_memory_mb = 9007199254740991\n");
    write_process(&scenario, "notes", REVIEW);
    scenario.agent_does("printf 'Done.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    let release = |_: u32| {
        assert!(
            scenario.claude_calls().is_empty(),
            "session started while resources were blocked"
        );
        scenario.user_config_is("");
    };

    let result = scenario.run_terminal(
        &["process", "notes"],
        &[],
        &[support::TerminalStep::inspect(
            "waiting for host resources:",
            &release,
        )],
    );

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(result.stdout, "Done.\n");
    assert!(result.stderr.contains("host resources available"));
}

#[test]
fn interrupting_a_resource_wait_starts_no_session() {
    let scenario = Scenario::new();
    scenario.user_config_is("[resources]\nmin_available_memory_mb = 9007199254740991\n");
    write_process(&scenario, "notes", REVIEW);
    scenario.agent_does("printf 'Done.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run_on_terminal(
        &["process", "notes"],
        &[],
        &[("waiting for host resources:", support::CTRL_C)],
    );

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"), "{}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(scenario.claude_calls().is_empty());
    let record = process_record(&scenario);
    assert_eq!(record["outcome"], "interrupted");
}

#[test]
fn stop_interrupts_a_running_command_step() {
    let scenario = Scenario::new();
    let started = scenario.path("started");
    let later = scenario.path("later-step");
    write_process(
        &scenario,
        "notes",
        &format!(
            "\
version = 1
description = \"Loop until stopped\"

[[steps]]
name = \"wait\"
command = \"touch '{}'; while true; do sleep 0.05; done\"
optional = true

[[steps]]
name = \"later\"
command = \"touch '{}'\"
",
            started.display(),
            later.display()
        ),
    );
    let mut held = scenario.run_until(&["process", "notes"], &[], "started");
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
    let command = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["kind"] == "Process")
        .unwrap_or_else(|| panic!("no Process in {status}"));
    let name = command["name"].as_str().unwrap();
    let stopped = scenario.run(&["stop", name]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();

    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    assert!(scenario.claude_calls().is_empty(), "a session started");
    assert!(!later.exists(), "a later step ran");
    let record = process_record(&scenario);
    assert_eq!(record["outcome"], "interrupted");
    let (_, log) = command_log(&scenario);
    assert!(step_line(&log, "wait").contains("interrupted"), "{log}");
    assert!(!log.contains("step later:"), "{log}");
}

#[test]
fn stop_interrupts_a_running_process() {
    let scenario = Scenario::new();
    write_process(&scenario, "notes", REVIEW);
    scenario.agent_does(&format!(
        "touch '{}'\nwhile true; do sleep 0.05; done\n",
        scenario.path("started").display()
    ));
    let mut held = scenario.run_until(&["process", "notes"], &[], "started");
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
    let command = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["kind"] == "Process")
        .unwrap_or_else(|| panic!("no Process in {status}"));
    let name = command["name"].as_str().unwrap();
    let stopped = scenario.run(&["stop", name]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();

    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    assert!(name.starts_with("process-notes-"), "{name}");
    let record = process_record(&scenario);
    assert_eq!(record["outcome"], "interrupted");
}

fn step_line<'a>(log: &'a str, name: &str) -> &'a str {
    log.lines()
        .find(|line| line.contains(&format!("step {name}:")))
        .unwrap_or_else(|| panic!("no step {name} in {log}"))
}
