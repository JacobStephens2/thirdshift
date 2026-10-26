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
            (
                "review",
                Some(
                    "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"review\"
skill = \"code-review\"
harness = \"nope\"
",
                ),
                ":7: unknown harness: nope",
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
    assert!(result.stdout.contains("harness: claude (default)"));
    assert!(result.stdout.contains("model: default (default)"));
    assert!(result.stdout.contains("effort: default (default)"));
    assert_eq!(result.stdout.matches("starts: yes").count(), 2);
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

#[test]
fn steps_on_claude_codex_and_agy_use_the_model_and_effort_each_step_asked_for() {
    let scenario = Scenario::new();
    write_process(
        &scenario,
        "families",
        "\
version = 1
description = \"One skill on three Harnesses\"

[[steps]]
name = \"claude\"
skill = \"to-spec\"
harness = \"claude\"
model = \"opus\"
effort = \"high\"

[[steps]]
name = \"codex\"
skill = \"code-review\"
harness = \"codex\"
model = \"gpt-6.1-sol\"
effort = \"max\"

[[steps]]
name = \"agy\"
skill = \"pr\"
harness = \"agy\"
model = \"gemini-3.8-flash\"
effort = \"high\"
",
    );
    scenario.agent_does("printf 'done' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run(&["process", "families"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "done\n");
    let claude = scenario.claude_calls();
    let codex = scenario.codex_calls();
    let agy = scenario.agy_calls();
    assert_eq!(claude.len(), 1, "{}", result.stderr);
    assert_eq!(codex.len(), 1, "{}", result.stderr);
    assert_eq!(agy.len(), 1, "{}", result.stderr);
    assert_flag(&claude[0], "--model", "opus");
    assert_flag(&claude[0], "--effort", "high");
    assert_flag(&codex[0], "-m", "gpt-6.1-sol");
    assert!(
        argv_of(&codex[0]).contains(&"model_reasoning_effort=\"max\"".to_string()),
        "{:?}",
        argv_of(&codex[0])
    );
    assert_flag(&agy[0], "--model", "gemini-3.8-flash");
    assert_flag(&agy[0], "--effort", "high");
    let claude_prompt = claude[0]["prompt"].as_str().unwrap();
    assert!(
        claude_prompt.contains("This is the first Step. No earlier Step has a result."),
        "{claude_prompt}"
    );
    let codex_prompt = codex[0]["prompt"].as_str().unwrap();
    assert!(
        codex_prompt.contains("Earlier Step result claude:"),
        "{codex_prompt}"
    );
    let agy_prompt = agy[0]["prompt"].as_str().unwrap();
    assert!(
        agy_prompt.contains("Earlier Step result claude:"),
        "{agy_prompt}"
    );
    assert!(
        agy_prompt.contains("Earlier Step result codex:"),
        "{agy_prompt}"
    );
}

#[test]
fn a_step_that_names_nothing_uses_the_commands_harness_model_and_effort() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "\
[harness]
default = \"agy\"

[harness.agy]
model = \"gemini-3.8-flash\"
effort = \"low\"

[harness.codex]
model = \"gpt-6-luna\"
effort = \"low\"
",
    );
    write_process(&scenario, "notes", REVIEW);
    scenario.agent_does("printf 'done' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let flagged = scenario.run(&[
        "process",
        "notes",
        "--harness",
        "codex",
        "--model",
        "gpt-6.1-sol",
        "--effort",
        "max",
    ]);

    assert_eq!(flagged.code, Some(0), "{}", flagged.stderr);
    assert!(scenario.claude_calls().is_empty(), "{}", flagged.stderr);
    assert!(scenario.agy_calls().is_empty(), "{}", flagged.stderr);
    let codex = scenario.codex_calls();
    assert_eq!(codex.len(), 1, "{}", flagged.stderr);
    assert_flag(&codex[0], "-m", "gpt-6.1-sol");
    assert!(
        argv_of(&codex[0]).contains(&"model_reasoning_effort=\"max\"".to_string()),
        "{:?}",
        argv_of(&codex[0])
    );

    let configured = Scenario::new();
    configured.user_config_is(
        "\
[harness]
default = \"agy\"

[harness.agy]
model = \"gemini-3.8-flash\"
effort = \"low\"
",
    );
    write_process(&configured, "notes", REVIEW);
    configured.agent_does("printf 'done' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = configured.run(&["process", "notes"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(configured.claude_calls().is_empty(), "{}", result.stderr);
    assert!(configured.codex_calls().is_empty(), "{}", result.stderr);
    let agy = configured.agy_calls();
    assert_eq!(agy.len(), 1, "{}", result.stderr);
    assert_flag(&agy[0], "--model", "gemini-3.8-flash");
    assert_flag(&agy[0], "--effort", "low");
}

#[test]
fn a_step_on_another_harness_keeps_that_harnesss_model_and_effort() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "\
[harness.agy]
model = \"gemini-3.8-flash\"
effort = \"low\"
",
    );
    write_process(
        &scenario,
        "notes",
        "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"command\"
skill = \"to-spec\"

[[steps]]
name = \"other\"
skill = \"code-review\"
harness = \"agy\"
",
    );
    scenario.agent_does("printf 'done' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");

    let result = scenario.run(&[
        "process",
        "--harness",
        "codex",
        "--model",
        "gpt-6.1-sol",
        "--effort",
        "max",
        "notes",
    ]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(scenario.claude_calls().is_empty(), "{}", result.stderr);
    let codex = scenario.codex_calls();
    let agy = scenario.agy_calls();
    assert_eq!(codex.len(), 1, "{}", result.stderr);
    assert_eq!(agy.len(), 1, "{}", result.stderr);
    assert_flag(&codex[0], "-m", "gpt-6.1-sol");
    assert!(
        argv_of(&codex[0]).contains(&"model_reasoning_effort=\"max\"".to_string()),
        "{:?}",
        argv_of(&codex[0])
    );
    assert_flag(&agy[0], "--model", "gemini-3.8-flash");
    assert_flag(&agy[0], "--effort", "low");
    let agy_argv = argv_of(&agy[0]);
    assert!(
        !agy_argv.iter().any(|arg| arg.contains("gpt-6.1-sol")),
        "{agy_argv:?}"
    );
}

#[test]
fn check_names_where_each_step_got_its_harness_model_and_effort() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "\
[harness]
default = \"codex\"

[harness.codex]
model = \"gpt-6-luna\"
effort = \"high\"

[harness.claude]
model = \"opus\"
effort = \"high\"
",
    );
    write_process(
        &scenario,
        "sources",
        "\
version = 1
description = \"Where each setting came from\"

[[steps]]
name = \"configured\"
skill = \"to-spec\"

[[steps]]
name = \"named\"
skill = \"code-review\"
harness = \"agy\"
model = \"gemini-3.8-flash\"
effort = \"low\"

[[steps]]
name = \"partial\"
skill = \"pr\"
harness = \"claude\"
",
    );

    let configured = scenario.run(&["process", "check", "sources"]);

    assert_eq!(configured.code, Some(0), "{}", configured.stderr);
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
    assert!(scenario.agy_calls().is_empty());
    assert!(
        configured
            .stdout
            .contains("harness: codex (User config)\nmodel: gpt-6-luna (User config)\neffort: high (User config)\nstarts: yes"),
        "{}",
        configured.stdout
    );
    assert!(
        configured.stdout.contains(
            "harness: agy (Step)\nmodel: gemini-3.8-flash (Step)\neffort: low (Step)\nstarts: yes"
        ),
        "{}",
        configured.stdout
    );
    assert!(
        configured.stdout.contains(
            "harness: claude (Step)\nmodel: opus (User config)\neffort: high (User config)\nstarts: yes"
        ),
        "{}",
        configured.stdout
    );

    let flagged = scenario.run(&[
        "process",
        "check",
        "sources",
        "harness",
        "codex",
        "model",
        "gpt-6.1-sol",
    ]);

    assert_eq!(flagged.code, Some(0), "{}", flagged.stderr);
    assert!(
        flagged.stdout.contains(
            "harness: codex (command)\nmodel: gpt-6.1-sol (command)\neffort: high (User config)\nstarts: yes"
        ),
        "{}",
        flagged.stdout
    );
    assert!(scenario.claude_calls().is_empty());
    assert!(scenario.codex_calls().is_empty());
    assert!(scenario.agy_calls().is_empty());
}

#[test]
fn a_harness_that_cannot_start_fails_before_any_session() {
    for check in [false, true] {
        let scenario = Scenario::new();
        write_process(
            &scenario,
            "notes",
            "\
version = 1
description = \"Review the diff\"

[[steps]]
name = \"first\"
skill = \"to-spec\"

[[steps]]
name = \"second\"
skill = \"code-review\"
harness = \"codex\"
model = \"gpt-6.1-sol\"
effort = \"high\"
",
        );
        scenario.agent_does("printf 'done' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
        let args: &[&str] = if check {
            &["process", "check", "notes"]
        } else {
            &["process", "notes"]
        };

        let result = scenario.run_with_env(args, &[("FAKE_CODEX_SIGNED_OUT", "1")]);

        assert_eq!(result.code, Some(1), "check={check}: {}", result.stderr);
        assert!(
            result
                .stderr
                .contains("step second's Harness codex can't start"),
            "check={check}: {}",
            result.stderr
        );
        assert!(
            result.stderr.contains("codex is not signed in"),
            "check={check}: {}",
            result.stderr
        );
        assert!(scenario.claude_calls().is_empty(), "check={check}");
        assert!(scenario.codex_calls().is_empty(), "check={check}");
        assert!(scenario.agy_calls().is_empty(), "check={check}");
        assert!(!process_dir(&scenario).exists());
        if check {
            assert!(result.stdout.contains("starts: yes"), "{}", result.stdout);
            assert!(result.stdout.contains("starts: no:"), "{}", result.stdout);
        } else {
            assert_eq!(result.stdout, "");
        }

        let inherited = Scenario::new();
        write_process(&inherited, "notes", REVIEW);
        inherited.agent_does("printf 'done' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
        let inherited_args: &[&str] = if check {
            &["process", "check", "notes", "--harness", "codex"]
        } else {
            &["process", "notes", "--harness", "codex"]
        };
        let inherited_result =
            inherited.run_with_env(inherited_args, &[("FAKE_CODEX_SIGNED_OUT", "1")]);
        assert_eq!(
            inherited_result.code,
            Some(1),
            "check={check}: {}",
            inherited_result.stderr
        );
        assert!(
            inherited_result
                .stderr
                .contains("step review's Harness codex can't start"),
            "check={check}: {}",
            inherited_result.stderr
        );
        assert!(inherited.claude_calls().is_empty(), "check={check}");
        assert!(inherited.codex_calls().is_empty(), "check={check}");
    }
}

fn argv_of(call: &serde_json::Value) -> Vec<String> {
    call["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_string())
        .collect()
}

fn assert_flag(call: &serde_json::Value, flag: &str, value: &str) {
    let argv = argv_of(call);
    let at = argv
        .iter()
        .position(|arg| arg == flag)
        .unwrap_or_else(|| panic!("{flag} missing in {argv:?}"));
    assert_eq!(
        argv.get(at + 1).map(String::as_str),
        Some(value),
        "{argv:?}"
    );
}

fn step_line<'a>(log: &'a str, name: &str) -> &'a str {
    log.lines()
        .find(|line| line.contains(&format!("step {name}:")))
        .unwrap_or_else(|| panic!("no step {name} in {log}"))
}
