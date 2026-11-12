//! A Run that fails because the host ran out of disk space is a host failure,
//! not a failure of its issue: its Claim is released whatever is on origin,
//! with `ready-for-agent` put back and no `agent-failed`, so the Failure limit
//! does not count it, and its Command log says to free disk space.

mod support;

use support::Scenario;

/// Put a `git` in front of the real one on the Scenario's `PATH` that fails
/// each `git … <command>` whose arguments end with one of `commands`, such as
/// `"worktree add"`, with git's own out-of-disk message, as a full disk would.
fn git_out_of_disk_on(scenario: &Scenario, commands: &[&str]) {
    let real = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap();
    let cases = commands
        .iter()
        .map(|command| {
            format!(
                "  *\" {command} \"*) echo \"fatal: unable to write new index file: No space \
                 left on device\" >&2; exit 128 ;;\n"
            )
        })
        .collect::<String>();
    let script = format!(
        "#!/bin/sh\ncase \" $* \" in\n{cases}esac\nexec {} \"$@\"\n",
        real.trim()
    );
    support::write_executable(&scenario.path("bin/git"), &script);
}

/// The advice that follows an out-of-disk cause.
const OUT_OF_DISK: &str = "the host ran out of disk space; free disk space on it, then retry the \
                           Command";

#[test]
fn a_continuation_whose_worktree_cannot_be_written_for_lack_of_disk_is_handed_back_ready() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    git_out_of_disk_on(&scenario, &["worktree add"]);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.issue_labels(7),
        ["bug", "ready-for-agent"],
        "stderr: {}",
        result.stderr
    );
    // The Command's cause is followed by what to do about it.
    let lines = support::before_command_log(&result.stderr);
    assert_eq!(
        lines.last(),
        Some(&format!("thirdshift: {OUT_OF_DISK}").as_str()),
        "stderr: {}",
        result.stderr
    );
    // Work already on origin stays there.
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "Earlier work".to_string()
    );
    // Nothing waits for the Day shift, so no Recovery comment is owed.
    assert!(scenario.issue_comments(7).is_empty());
}

/// The Command log of the Scenario's one Command on issue #7.
fn command_log(scenario: &Scenario) -> String {
    let dir = "home/.thirdshift/logs/acme/widgets/commands/issue";
    let logs = scenario.log_files(dir, "log");
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    std::fs::read_to_string(scenario.path(dir).join(&logs[0])).unwrap()
}

#[test]
fn a_push_that_runs_out_of_disk_hands_the_issue_back_and_keeps_earlier_work_on_origin() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.agent_does(
        "echo done > feature.txt\ngit add feature.txt\ngit commit -q -m 'Add feature'\n",
    );
    git_out_of_disk_on(&scenario, &["push"]);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.issue_labels(7),
        ["ready-for-agent"],
        "stderr: {}",
        result.stderr
    );
    // The push failed, so the new commit is kept locally, and the earlier
    // work stays on origin.
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: Preserved: work may exist only locally on issue-7 at {}",
            scenario.path("work/widgets-issue-7").display()
        )),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "Earlier work".to_string()
    );
    let log = command_log(&scenario);
    for line in [
        format!(
            "releasing the Claim on #7: labelling it ready-for-agent, in place of in-progress, \
             without agent-failed: {OUT_OF_DISK}"
        ),
        format!("Before retrying: {OUT_OF_DISK}"),
    ] {
        assert!(log.contains(&line), "{line:?} not in log: {log}");
    }
    assert!(scenario.issue_comments(7).is_empty());
}

#[test]
fn a_failure_that_is_not_out_of_disk_still_labels_agent_failed() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.agent_does("exit 3\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.issue_labels(7), ["agent-failed"]);
    assert!(
        !result.stderr.contains("ran out of disk space"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_base_fix_agent_saying_the_disk_is_full_does_not_release_the_run_that_waited_on_it() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    // #7's agent opens its PR with `test` red on its head and on main's tip,
    // an Inherited failure, so the Run starts a Base fix on #8.
    scenario.agent_does(
        r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
gh fake checks "$(git rev-parse HEAD)" '[{"name": "test", "conclusion": "failure"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name": "test", "conclusion": "failure"}]'
"#,
    );
    // The Base fix's agent fails, its own message naming a full disk.
    scenario.agent_does_for(
        8,
        "printf 'npm test failed: ENOSPC: No space left on device' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\nexit 3\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("No space left on device"),
        "stderr: {}",
        result.stderr
    );
    // The agent's words are not the host's: the Run failed with work on
    // origin, so its issue waits for the Day shift.
    assert_eq!(
        scenario.issue_labels(7),
        ["agent-failed"],
        "stderr: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains(OUT_OF_DISK),
        "stderr: {}",
        result.stderr
    );
}
