//! Command-owned temporary files, observed through real CLI sessions.
mod support;

use std::path::PathBuf;
use support::Scenario;

const OPENS_PR: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'
"#;

fn saved_tmp(scenario: &Scenario) -> PathBuf {
    std::fs::read_to_string(scenario.path("session-tmp"))
        .unwrap()
        .trim()
        .into()
}

fn capture(scenario: &Scenario) -> String {
    format!(
        "printf '%s' \"${{TMPDIR:-/tmp}}\" > '{}'\nmktemp > '{}'\n(readlink .claude/skills/thirdshift-implement || readlink .agents/skills/thirdshift-implement) > '{}'\n",
        scenario.path("session-tmp").display(),
        scenario.path("temporary-file").display(),
        scenario.path("factory-skill").display(),
    )
}

fn write_process(scenario: &Scenario) {
    let processes = scenario.path("home/.thirdshift/processes");
    std::fs::create_dir_all(&processes).unwrap();
    std::fs::write(processes.join("scratch-test.toml"), "version = 1\ndescription = 'Scratch lifecycle'\n[[steps]]\nname = 'inspect'\nskill = 'code-review'\n").unwrap();
}

#[test]
fn sessions_and_factory_skills_use_command_scratch_and_success_removes_it() {
    for harness in ["claude", "codex"] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!("{}{OPENS_PR}", capture(&scenario)));
        let result = scenario.run(&[&scenario.issue_url(7), "--harness", harness]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let tmp = saved_tmp(&scenario);
        assert!(
            tmp.starts_with(scenario.path("home/.thirdshift/scratch")),
            "{tmp:?}"
        );
        let file = std::fs::read_to_string(scenario.path("temporary-file")).unwrap();
        assert!(PathBuf::from(file.trim()).starts_with(&tmp), "{file}");
        let skill = std::fs::read_to_string(scenario.path("factory-skill")).unwrap();
        assert!(
            PathBuf::from(skill.trim()).starts_with(tmp.parent().unwrap()),
            "{skill}"
        );
        assert!(!tmp.parent().unwrap().exists(), "{tmp:?}");
    }
}

#[test]
fn startup_reaps_a_killed_commands_scratch_but_never_a_live_commands() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{}touch '{}'\nwhile :; do sleep 1; done\n",
        capture(&scenario),
        scenario.path("started").display(),
    ));
    let mut held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
    let tmp = saved_tmp(&scenario);
    assert!(tmp.is_dir(), "{tmp:?}");
    // A different issue starts a Command on the same host while the first lives.
    scenario.agent_does("exit 3\n");
    let live = scenario.run(&[&scenario.issue_url(8)]);
    assert_eq!(live.code, Some(1), "{}", live.stderr);
    assert!(tmp.is_dir(), "a live Command lost {tmp:?}");
    held.kill();
    assert!(tmp.is_dir(), "SIGKILL unexpectedly ran cleanup");
    let next = scenario.run(&[&scenario.issue_url(9)]);
    assert_eq!(next.code, Some(1), "{}", next.stderr);
    assert!(!tmp.parent().unwrap().exists(), "orphan survived: {tmp:?}");
    assert!(
        next.stderr.contains("removed orphaned Command scratch"),
        "{}",
        next.stderr
    );
}

#[test]
fn recovery_keeps_scratch_until_a_successful_continuation_or_dismissal() {
    for dismiss in [false, true] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!("{}exit 3\n", capture(&scenario)));
        let failed = scenario.run(&[&scenario.issue_url(7)]);
        assert_eq!(failed.code, Some(1), "{}", failed.stderr);
        assert!(failed.stderr.contains("Recovery:"), "{}", failed.stderr);
        let tmp = saved_tmp(&scenario);
        assert!(tmp.is_dir(), "Recovery lost its scratch: {tmp:?}");
        scenario.agent_does("exit 3\n");
        scenario.run(&[&scenario.issue_url(8)]);
        assert!(tmp.is_dir(), "unrelated startup removed retained scratch");
        let completed = if dismiss {
            scenario.run(&["dismiss", "widgets#7"])
        } else {
            scenario.agent_does(OPENS_PR);
            scenario.run(&[&scenario.issue_url(7)])
        };
        assert_eq!(completed.code, Some(0), "{}", completed.stderr);
        assert!(
            !tmp.parent().unwrap().exists(),
            "retained scratch survived: {tmp:?}"
        );
    }
}

#[test]
fn legacy_cleanup_requires_age_and_the_exact_factory_layout() {
    fn copy(source: &std::path::Path, dest: &std::path::Path) {
        std::fs::create_dir_all(dest).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &dest.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), dest.join(entry.file_name())).unwrap();
            }
        }
    }
    let scenario = Scenario::new();
    let root = scenario.path("tmp");
    let age = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 86400);
    for name in ["old", "fresh", "user", "symlink"] {
        let path = root.join(format!("thirdshift-skills-{name}"));
        copy(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/skills")),
            &path,
        );
        if name == "user" {
            std::fs::write(path.join("thirdshift-tdd/my-notes"), "keep me").unwrap();
        }
        if name == "symlink" {
            std::fs::remove_file(path.join("LICENSE")).unwrap();
            std::os::unix::fs::symlink(
                root.join("thirdshift-skills-fresh/LICENSE"),
                path.join("LICENSE"),
            )
            .unwrap();
        }
        if name != "fresh" {
            std::fs::File::open(&path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(age))
                .unwrap();
        }
    }
    scenario.agent_does(OPENS_PR);
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        !root.join("thirdshift-skills-old").exists(),
        "legacy Factory skills survived"
    );
    for name in ["fresh", "user", "symlink"] {
        assert!(
            root.join(format!("thirdshift-skills-{name}")).is_dir(),
            "removed {name}"
        );
    }
    assert!(
        result.stderr.contains("removed legacy Factory skills"),
        "{}",
        result.stderr
    );
}

#[test]
fn sandboxed_triage_uses_writable_command_scratch() {
    let probe = std::process::Command::new("bwrap")
        .args(["--ro-bind", "/", "/", "--", "/bin/true"])
        .status();
    assert!(
        probe.is_ok_and(|status| status.success()),
        "bwrap is required for this regression"
    );
    let scenario = Scenario::new();
    let expected = scenario.path("home/.thirdshift/scratch");
    let recommendation = r#"{"category":"bug","state":"needs-info","issues":[],"investigation":"Scratch was writable."}"#;
    let event = serde_json::json!({"type":"result", "subtype":"success", "is_error":false,
        "result":format!("Triage recommendation: {recommendation}")});
    std::fs::remove_file(scenario.path("bin/claude")).unwrap();
    support::write_executable(
        &scenario.path("bin/claude"),
        &format!(
            r#"#!/bin/bash
set -eu
if [ "$1" = auth ]; then echo '{{"loggedIn":true}}'; exit; fi
case "$TMPDIR" in '{expected}'/*) ;; *) echo "wrong TMPDIR: $TMPDIR" >&2; exit 42 ;; esac
mktemp
printf '%s\n' '{event}'
"#,
            expected = expected.display()
        ),
    );
    std::fs::write(scenario.path("report.txt"), "Investigate a cache miss.").unwrap();
    let result = scenario.run(&[
        "triage",
        "--recommend-only",
        "--read-machine",
        "--file",
        scenario.path("report.txt").to_str().unwrap(),
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(std::fs::read_dir(&expected).unwrap().count(), 0);
}

#[test]
fn sigterm_finishes_a_process_without_recovery_and_removes_scratch() {
    let scenario = Scenario::new();
    write_process(&scenario);
    scenario.agent_does(&format!(
        "{}touch '{}'\nwhile :; do sleep 1; done\n",
        capture(&scenario),
        scenario.path("started").display()
    ));
    let result = scenario.run_and_signal(&["process", "scratch-test"], "started", "TERM");
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let tmp = saved_tmp(&scenario);
    assert!(
        tmp.starts_with(scenario.path("home/.thirdshift/scratch")),
        "{tmp:?}"
    );
    assert!(!tmp.parent().unwrap().exists(), "SIGTERM left {tmp:?}");
}

#[test]
fn a_paused_command_keeps_scratch_until_it_is_continued() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{}echo '{{\"type\":\"rate_limit_event\",\"rate_limit_info\":{{\"status\":\"rejected\",\"rateLimitType\":\"seven_day\",\"resetsAt\":4000000000,\"overageStatus\":\"rejected\"}}}}'\nexit 1\n", capture(&scenario)));
    let paused = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(paused.code, Some(75), "{}", paused.stderr);
    let tmp = saved_tmp(&scenario);
    assert!(tmp.is_dir(), "pause lost scratch: {tmp:?}");
    scenario.plan_usage_is(serde_json::json!({}));
    scenario.agent_does(OPENS_PR);
    let continued = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(continued.code, Some(0), "{}", continued.stderr);
    assert!(!tmp.parent().unwrap().exists(), "continuation left {tmp:?}");
}

#[test]
fn a_spec_and_its_child_ticket_share_command_scratch() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[])]);
    scenario.agent_does_for(21, &format!("{}echo feature > feature.txt\ngit add feature.txt\ngit commit -qm 'Ticket'\ngh pr create --base issue-20 --head issue-21 --title Ticket --body 'Closes #21'\n", capture(&scenario)));
    scenario.agent_does_for(20, &format!("test \"$TMPDIR\" = \"$(cat '{}')\"\nprintf reviewed > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n", scenario.path("session-tmp").display()));
    let result = scenario.run(&[&scenario.issue_url(20)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let tmp = saved_tmp(&scenario);
    assert!(
        tmp.starts_with(scenario.path("home/.thirdshift/scratch")),
        "{tmp:?}"
    );
    assert!(!tmp.parent().unwrap().exists());
}

#[test]
fn continuing_an_unscoped_paused_process_removes_its_retained_scratch() {
    let scenario = Scenario::new();
    write_process(&scenario);
    scenario.agent_does(&format!("{}echo '{{\"type\":\"rate_limit_event\",\"rate_limit_info\":{{\"status\":\"rejected\",\"rateLimitType\":\"seven_day\",\"resetsAt\":4000000000,\"overageStatus\":\"rejected\"}}}}'\nexit 1\n", capture(&scenario)));
    let paused = scenario.run(&["process", "scratch-test"]);
    assert_eq!(paused.code, Some(75), "{}", paused.stderr);
    let tmp = saved_tmp(&scenario);
    assert!(tmp.is_dir());
    scenario.plan_usage_is(serde_json::json!({}));
    scenario.agent_does("printf reviewed > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n");
    let continued = scenario.run(&["process", "scratch-test"]);
    assert_eq!(continued.code, Some(0), "{}", continued.stderr);
    assert!(
        !tmp.parent().unwrap().exists(),
        "unscoped continuation left {tmp:?}"
    );
}
