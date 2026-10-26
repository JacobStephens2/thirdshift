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
command = \"echo hi\"
",
                ),
                ":6: unknown key: command",
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

const FAMILIES: &str = "\
version = 1
description = \"Three families, then a judge\"

[worktrees.claude]
kind = \"throwaway\"

[worktrees.codex]
kind = \"throwaway\"

[worktrees.grok]
kind = \"throwaway\"

[worktrees.judge]
kind = \"throwaway\"

[[steps]]
name = \"claude\"
skill = \"code-review\"
worktree = \"claude\"
parallel = \"families\"

[[steps]]
name = \"codex\"
skill = \"code-review\"
worktree = \"codex\"
parallel = \"families\"

[[steps]]
name = \"grok\"
skill = \"code-review\"
worktree = \"grok\"
parallel = \"families\"

[[steps]]
name = \"judge\"
skill = \"to-spec\"
worktree = \"judge\"
";

#[test]
fn check_prints_each_steps_worktree_and_which_steps_run_together() {
    let scenario = Scenario::new();
    write_process(&scenario, "families", FAMILIES);

    let result = scenario.run(&["process", "check", "families"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty(), "a session started");
    assert!(
        result
            .stdout
            .contains("families: Three families, then a judge")
    );
    for name in ["claude", "codex", "grok", "judge"] {
        assert!(
            result
                .stdout
                .contains(&format!("worktree {name}: throwaway at the Base branch")),
            "{}",
            result.stdout
        );
        assert!(
            result.stdout.contains(&format!("(worktree {name})")),
            "{}",
            result.stdout
        );
    }
    assert!(result.stdout.contains("together: claude, codex, grok\n"));
    assert_eq!(
        result.stdout.matches("together:").count(),
        1,
        "{}",
        result.stdout
    );
    assert!(result.stdout.contains("SKILL.md"), "{}", result.stdout);
    assert!(
        !result
            .stdout
            .contains("together: claude, codex, grok, judge")
    );
    assert_eq!(worktree_count(&scenario), 1);
    assert!(!process_dir(&scenario).exists());
}

#[test]
fn parallel_steps_that_share_a_worktree_or_name_an_unknown_one_are_refused() {
    let cases = [
        (
            "\
version = 1
description = \"Shared\"

[worktrees.one]
kind = \"throwaway\"

[[steps]]
name = \"a\"
skill = \"code-review\"
worktree = \"one\"
parallel = \"g\"

[[steps]]
name = \"b\"
skill = \"code-review\"
worktree = \"one\"
parallel = \"g\"
",
            ":16: shared worktree:",
        ),
        (
            "\
version = 1
description = \"Implicit\"

[[steps]]
name = \"a\"
skill = \"code-review\"
parallel = \"g\"

[[steps]]
name = \"b\"
skill = \"code-review\"
parallel = \"g\"
",
            ":12: shared worktree:",
        ),
        (
            "\
version = 1
description = \"Missing\"

[[steps]]
name = \"a\"
skill = \"code-review\"
worktree = \"missing\"
",
            ":7: unknown worktree:",
        ),
        (
            "\
version = 1
description = \"Nameless\"

[worktrees.one]
kind = \"throwaway\"

[[steps]]
name = \"a\"
skill = \"code-review\"
",
            ":8: worktree:",
        ),
        (
            "\
version = 1
description = \"Branch\"

[worktrees.fix]
kind = \"issue-branch\"
",
            ":5: worktree kind:",
        ),
    ];
    for check in [true, false] {
        for (body, expect) in cases {
            let scenario = Scenario::new();
            write_process(&scenario, "families", body);
            let result = if check {
                scenario.run(&["process", "check", "families"])
            } else {
                scenario.run(&["process", "families"])
            };
            assert_no_session(&scenario, &result);
            assert!(
                result.stderr.contains(expect),
                "check={check} expected {expect} in {}",
                result.stderr
            );
            assert_eq!(worktree_count(&scenario), 1, "{}", result.stderr);
        }
    }
}

#[test]
fn three_parallel_steps_overlap_and_the_next_step_sees_every_result() {
    let scenario = Scenario::new();
    write_process(&scenario, "families", FAMILIES);
    scenario.agent_does(&family_sessions(
        "printf '%s\\n' \"$name\" > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
        "\
for step in claude codex grok; do
  path=$(printf '%s\\n' \"$FAKE_CLAUDE_PROMPT\" | sed -n \"s/^Earlier Step result ${step}: //p\")
  test -n \"$path\"
  test \"$(cat \"$path\")\" = \"$step\"
done
printf 'judged' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
    ));

    let result = scenario.run(&["process", "families"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "judged\n");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 4, "{}", result.stderr);
    let mut directories = Vec::new();
    for step in ["claude", "codex", "grok", "judge"] {
        let prompt = prompt_of(&calls, step);
        let cwd = cwd_of(&calls, step);
        assert_ne!(cwd, scenario.launch_dir());
        assert!(!cwd.exists(), "{step} worktree kept: {}", cwd.display());
        directories.push(cwd.to_path_buf());
        if step == "judge" {
            assert!(!prompt.contains("This is the first Step"), "{prompt}");
        } else {
            assert!(
                prompt.contains("This is the first Step. No earlier Step has a result."),
                "{step}: {prompt}"
            );
        }
    }
    directories.sort();
    directories.dedup();
    assert_eq!(directories.len(), 4, "parallel steps shared a worktree");
    let (log_path, log) = command_log(&scenario);
    for step in ["claude", "codex", "grok", "judge"] {
        assert!(step_line(&log, step).contains("succeeded"), "{log}");
    }
    let steps = log_path.with_extension("steps");
    for step in ["claude", "codex", "grok"] {
        assert_eq!(
            fs::read_to_string(steps.join(format!("{step}.txt"))).unwrap(),
            format!("{step}\n")
        );
    }
    assert_eq!(worktree_count(&scenario), 1);
    assert_eq!(process_record(&scenario)["outcome"], "succeeded");
}

#[test]
fn a_failed_parallel_step_lets_the_others_finish_before_the_process_fails() {
    let scenario = Scenario::new();
    write_process(&scenario, "families", FAMILIES);
    scenario.agent_does(&family_sessions(
        "\
if [ \"$name\" = \"codex\" ]; then
  exit 1
fi
sleep 0.3
printf '%s\\n' \"$name\" > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
        "printf 'judge ran\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    ));

    let result = scenario.run(&["process", "families"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(result.stdout, "");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 3, "{}", result.stderr);
    assert!(prompt_of_optional(&calls, "judge").is_none(), "{calls:?}");
    for step in ["claude", "grok"] {
        assert!(!cwd_of(&calls, step).exists(), "{step} kept");
    }
    assert!(!cwd_of(&calls, "codex").exists(), "failed worktree kept");
    let (log_path, log) = command_log(&scenario);
    assert!(step_line(&log, "claude").contains("succeeded"), "{log}");
    assert!(step_line(&log, "grok").contains("succeeded"), "{log}");
    assert!(step_line(&log, "codex").contains("failed"), "{log}");
    assert!(!log.contains("step judge:"), "{log}");
    let steps = log_path.with_extension("steps");
    assert_eq!(
        fs::read_to_string(steps.join("claude.txt")).unwrap(),
        "claude\n"
    );
    assert_eq!(
        fs::read_to_string(steps.join("grok.txt")).unwrap(),
        "grok\n"
    );
    assert!(!steps.join("codex.txt").exists());
    assert!(!steps.join("judge.txt").exists());
    assert_eq!(worktree_count(&scenario), 1, "{}", result.stderr);
    assert_eq!(process_record(&scenario)["outcome"], "failed");
}

#[test]
fn a_failed_parallel_step_keeps_only_a_worktree_retention_would_keep() {
    let scenario = Scenario::new();
    write_process(&scenario, "families", FAMILIES);
    scenario.agent_does(&family_sessions(
        "\
if [ \"$name\" = \"codex\" ]; then
  printf 'kept\\n' > leftover.txt
  exit 1
fi
printf '%s\\n' \"$name\" > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
        "printf 'judge ran\\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    ));

    let result = scenario.run(&["process", "families"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 3, "{}", result.stderr);
    let kept = cwd_of(&calls, "codex");
    assert!(
        kept.exists(),
        "dirty worktree was removed: {}",
        result.stderr
    );
    assert_eq!(
        fs::read_to_string(kept.join("leftover.txt")).unwrap(),
        "kept\n"
    );
    assert!(
        result.stderr.contains("retaining checkout"),
        "{}",
        result.stderr
    );
    for step in ["claude", "grok"] {
        assert!(!cwd_of(&calls, step).exists(), "{step} kept");
    }
    assert_eq!(worktree_count(&scenario), 2, "{}", result.stderr);
    assert_eq!(
        scenario.launch_git(&["status", "--porcelain", "--untracked-files=all"]),
        ""
    );
}

#[test]
fn an_optional_parallel_failure_does_not_fail_the_group() {
    let scenario = Scenario::new();
    let mut body = FAMILIES.to_string();
    body = body.replacen(
        "\
name = \"codex\"
skill = \"code-review\"
worktree = \"codex\"
parallel = \"families\"
",
        "\
name = \"codex\"
skill = \"code-review\"
worktree = \"codex\"
parallel = \"families\"
optional = true
",
        1,
    );
    write_process(&scenario, "families", &body);
    let plan = scenario.run(&["process", "check", "families"]);
    assert_eq!(plan.code, Some(0), "{}", plan.stderr);
    assert!(
        plan.stdout.contains("(worktree codex) (optional)"),
        "{}",
        plan.stdout
    );
    scenario.agent_does(&family_sessions(
        "\
if [ \"$name\" = \"codex\" ]; then
  exit 1
fi
printf '%s\\n' \"$name\" > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
        "\
if printf '%s\\n' \"$FAKE_CLAUDE_PROMPT\" | grep -q 'Earlier Step result codex:'; then
  exit 1
fi
for step in claude grok; do
  path=$(printf '%s\\n' \"$FAKE_CLAUDE_PROMPT\" | sed -n \"s/^Earlier Step result ${step}: //p\")
  test \"$(cat \"$path\")\" = \"$step\"
done
printf 'judged' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
    ));

    let result = scenario.run(&["process", "families"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "judged\n");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 4, "{}", result.stderr);
    let (_, log) = command_log(&scenario);
    assert!(step_line(&log, "codex").contains("failed"), "{log}");
    for step in ["claude", "grok", "judge"] {
        assert!(step_line(&log, step).contains("succeeded"), "{log}");
        assert!(!cwd_of(&calls, step).exists(), "{step} kept");
    }
    assert!(!cwd_of(&calls, "codex").exists());
    assert_eq!(worktree_count(&scenario), 1);
    assert_eq!(process_record(&scenario)["outcome"], "succeeded");
}

#[test]
fn the_same_parallel_name_after_another_step_is_a_new_group() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "groups",
        "\
version = 1
description = \"Two groups of the same name\"

[worktrees.wa]
kind = \"throwaway\"

[worktrees.wb]
kind = \"throwaway\"

[worktrees.wm]
kind = \"throwaway\"

[worktrees.wc]
kind = \"throwaway\"

[worktrees.wd]
kind = \"throwaway\"

[worktrees.wj]
kind = \"throwaway\"

[[steps]]
name = \"a\"
skill = \"code-review\"
worktree = \"wa\"
parallel = \"g\"

[[steps]]
name = \"b\"
skill = \"code-review\"
worktree = \"wb\"
parallel = \"g\"

[[steps]]
name = \"mid\"
skill = \"to-spec\"
worktree = \"wm\"

[[steps]]
name = \"c\"
skill = \"code-review\"
worktree = \"wc\"
parallel = \"g\"

[[steps]]
name = \"d\"
skill = \"code-review\"
worktree = \"wd\"
parallel = \"g\"

[[steps]]
name = \"judge\"
skill = \"pr\"
worktree = \"wj\"
",
    );
    scenario.agent_does(
        r#"name=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^This is Step \([^ ]*\) of the Custom process .*/\1/p')
root=$(dirname "$FAKE_CLAUDE_RECORD")
overlap() {
  dir=$1
  shift
  mkdir -p "$dir"
  touch "$dir/$name"
  ready=0
  for _ in $(seq 1 100); do
    ready=1
    for member in "$@"; do
      if [ ! -f "$dir/$member" ]; then
        ready=0
      fi
    done
    if [ "$ready" = 1 ]; then
      break
    fi
    sleep 0.05
  done
  test "$ready" = 1
}
has() {
  printf '%s\n' "$FAKE_CLAUDE_PROMPT" | grep -q "Earlier Step result $1:"
}
case "$name" in
  a|b)
    if has mid || has c || has d; then exit 1; fi
    overlap "$root/first" a b
    printf '%s\n' "$name" > "$FAKE_CLAUDE_FINAL_MESSAGE"
    ;;
  mid)
    for step in a b; do
      path=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n "s/^Earlier Step result ${step}: //p")
      test "$(cat "$path")" = "$step"
    done
    if has c || has d; then exit 1; fi
    printf 'mid\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
    ;;
  c|d)
    for step in a b mid; do
      path=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n "s/^Earlier Step result ${step}: //p")
      test -n "$path"
    done
    other=d
    if [ "$name" = "d" ]; then other=c; fi
    if has "$other"; then exit 1; fi
    overlap "$root/second" c d
    printf '%s\n' "$name" > "$FAKE_CLAUDE_FINAL_MESSAGE"
    ;;
  judge)
    for step in a b mid c d; do
      path=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n "s/^Earlier Step result ${step}: //p")
      test "$(cat "$path")" = "$step"
    done
    printf 'judged' > "$FAKE_CLAUDE_FINAL_MESSAGE"
    ;;
esac
"#,
    );

    let result = scenario.run(&["process", "groups"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "judged\n");
    assert_eq!(scenario.claude_calls().len(), 6, "{}", result.stderr);
    let plan = scenario.run(&["process", "check", "groups"]);
    assert!(plan.stdout.contains("together: a, b\n"), "{}", plan.stdout);
    assert!(plan.stdout.contains("together: c, d\n"), "{}", plan.stdout);
    assert_eq!(
        plan.stdout.matches("together:").count(),
        2,
        "{}",
        plan.stdout
    );
    assert_eq!(worktree_count(&scenario), 1);
}

#[test]
fn an_optional_step_failure_is_recorded_and_later_steps_run() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "notes",
        "\
version = 1
description = \"Optional gather\"

[[steps]]
name = \"gather\"
skill = \"to-spec\"
optional = true

[[steps]]
name = \"judge\"
skill = \"code-review\"
",
    );
    scenario.agent_does_in_session(1, "exit 1");
    scenario.agent_does_in_session(
        2,
        "\
if printf '%s\\n' \"$FAKE_CLAUDE_PROMPT\" | grep -q 'Earlier Step result gather:'; then
  exit 1
fi
printf 'judged' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"
",
    );

    let result = scenario.run(&["process", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "judged\n");
    assert_eq!(scenario.claude_calls().len(), 2, "{}", result.stderr);
    let (_, log) = command_log(&scenario);
    assert!(step_line(&log, "gather").contains("failed"), "{log}");
    assert!(step_line(&log, "judge").contains("succeeded"), "{log}");
    assert_eq!(process_record(&scenario)["outcome"], "succeeded");
    assert_eq!(worktree_count(&scenario), 1);
}

fn family_sessions(members: &str, judge: &str) -> String {
    format!(
        r#"name=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^This is Step \([^ ]*\) of the Custom process .*/\1/p')
root=$(dirname "$FAKE_CLAUDE_RECORD")
case "$name" in
  claude|codex|grok)
    mkdir -p "$root/overlap"
    touch "$root/overlap/$name"
    ready=0
    for _ in $(seq 1 100); do
      if [ -f "$root/overlap/claude" ] && [ -f "$root/overlap/codex" ] && [ -f "$root/overlap/grok" ]; then
        ready=1
        break
      fi
      sleep 0.05
    done
    test "$ready" = 1
    {members}
    ;;
  judge)
    {judge}
    ;;
esac
"#
    )
}

fn prompt_of<'a>(calls: &'a [serde_json::Value], step: &str) -> &'a str {
    prompt_of_optional(calls, step).unwrap_or_else(|| panic!("no session for {step}"))
}

fn prompt_of_optional<'a>(calls: &'a [serde_json::Value], step: &str) -> Option<&'a str> {
    let needle = format!("This is Step {step} of the Custom process");
    calls.iter().find_map(|call| {
        call["prompt"]
            .as_str()
            .filter(|prompt| prompt.contains(&needle))
    })
}

fn cwd_of<'a>(calls: &'a [serde_json::Value], step: &str) -> &'a Path {
    let needle = format!("This is Step {step} of the Custom process");
    let cwd = calls
        .iter()
        .find(|call| {
            call["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains(&needle))
        })
        .unwrap_or_else(|| panic!("no session for {step}"))["cwd"]
        .as_str()
        .unwrap();
    Path::new(cwd)
}

fn worktree_count(scenario: &Scenario) -> usize {
    scenario
        .launch_git(&["worktree", "list", "--porcelain"])
        .matches("worktree ")
        .count()
}

fn step_line<'a>(log: &'a str, name: &str) -> &'a str {
    log.lines()
        .find(|line| line.contains(&format!("step {name}:")))
        .unwrap_or_else(|| panic!("no step {name} in {log}"))
}
