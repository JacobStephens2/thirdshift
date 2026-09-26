//! Keeping the PR mergeable: once the PR is confirmed, thirdshift merges the
//! Base branch into the Issue branch and pushes, handing a conflicting merge to
//! a conflict Repair session first. If the Base branch moves while CI runs, it
//! merges it again, at most three times.

mod support;

use support::Scenario;

/// The agent adds feature.txt and opens a PR.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

/// While the agent works, someone else pushes `file` with `content` to main.
fn base_moves_on(file: &str, content: &str) -> String {
    format!(
        r#"
other="$(mktemp -d)"
git clone -q https://github.com/acme/widgets.git "$other"
echo "{content}" > "$other/{file}"
# -A, not {file}: a file name built by the shell may expand differently twice.
git -C "$other" add -A
git -C "$other" commit -q -m "Base moves on"
git -C "$other" push -q origin main
rm -rf "$other"
"#
    )
}

/// The conflict Repair keeps both sides of feature.txt and finishes the merge.
const REPAIR_RESOLVES_CONFLICT: &str = r#"
printf 'feature\nbase feature\n' > feature.txt
git add feature.txt
git commit -q --no-edit
"#;

/// origin's issue-7 is the agent's work with main merged in on top.
fn assert_merged_main_into_issue_7(scenario: &Scenario) {
    assert_eq!(
        scenario.origin_git(&["log", "--first-parent", "--format=%s", "issue-7"]),
        "Merge remote-tracking branch 'origin/main' into issue-7\n\
         Add feature\n\
         Initial commit\n"
    );
    assert_eq!(
        scenario.origin_git(&["rev-parse", "issue-7^2"]),
        scenario.origin_git(&["rev-parse", "main"])
    );
}

#[test]
fn merges_an_advanced_base_branch_without_a_repair() {
    let scenario = Scenario::new();
    // The agent pushes issue-7 itself, so the merge can only reach origin as a
    // fast-forward of it (origin rejects anything else).
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}git push -q origin issue-7\n{}",
        base_moves_on("other.txt", "other")
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_merged_main_into_issue_7(&scenario);
}

#[test]
fn makes_no_merge_commit_when_the_base_branch_has_not_moved() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Add feature".to_string(),
            "Initial commit".to_string()
        ])
    );
}

#[test]
fn hands_a_conflicting_merge_to_a_conflict_repair_and_pushes_its_resolution() {
    let scenario = Scenario::new();
    // The agent pushes issue-7 itself, so the resolution can only reach origin
    // as a fast-forward of it (origin rejects anything else).
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}git push -q origin issue-7\n{}",
        base_moves_on("feature.txt", "base feature")
    ));
    scenario.agent_does_in_session(2, REPAIR_RESOLVES_CONFLICT);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    let repair = &calls[1];
    assert_eq!(
        repair["prompt"],
        "/thirdshift:resolving-merge-conflicts\n\
         \n\
         A merge of origin/main into issue-7 is in progress in this worktree and has conflicts.\n\
         issue-7 implements https://github.com/acme/widgets/issues/7; its pull request is https://github.com/acme/widgets/pull/1.\n\
         \n\
         Resolve the conflicts, finish the merge, and push issue-7. Do not rebase or force-push.\n\
         \n\
         You run headless: nobody is watching, and ending your turn ends the session. Run tests and other long commands in the foreground, raising the Bash timeout if needed. Never end your turn while a background task you depend on is still running: ending the turn kills it.\n"
    );
    assert_eq!(repair["merging"], true);
    assert_eq!(repair["branch"], "issue-7");
    assert_eq!(repair["cwd"], calls[0]["cwd"]);
    let flags = |call: &serde_json::Value| {
        let argv = call["argv"].as_array().unwrap().clone();
        argv[..argv.len() - 1].to_vec()
    };
    assert_eq!(flags(repair), flags(&calls[0]), "same plugin and flags");
    assert_merged_main_into_issue_7(&scenario);
    assert_eq!(
        scenario.origin_git(&["show", "issue-7:feature.txt"]),
        "feature\nbase feature\n"
    );
}

#[test]
fn logs_the_conflict_repair_with_the_runs_shared_timestamp() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        base_moves_on("feature.txt", "base feature")
    ));
    scenario.agent_does_in_session(2, REPAIR_RESOLVES_CONFLICT);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let logs = scenario.entries("home/.thirdshift/logs");
    assert_eq!(logs.len(), 2, "logs: {logs:?}");
    let implement = logs
        .iter()
        .find(|name| name.ends_with("-implement.jsonl"))
        .expect("no implement log");
    let prefix = implement.strip_suffix("implement.jsonl").unwrap();
    assert!(
        prefix.starts_with("acme-widgets-issue-7-"),
        "log: {implement}"
    );
    assert!(
        logs.contains(&format!("{prefix}repair-1.jsonl")),
        "logs: {logs:?}"
    );
}

#[test]
fn fails_when_the_conflict_repair_leaves_the_merge_unfinished() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        base_moves_on("feature.txt", "base feature")
    ));
    scenario.agent_does_in_session(2, "true");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    // A Failed run with an open PR prints it, sent back to draft.
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    assert!(
        result
            .stderr
            .contains("merge of origin/main is still in progress"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn fails_when_the_conflict_repair_aborts_the_merge() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        base_moves_on("feature.txt", "base feature")
    ));
    scenario.agent_does_in_session(2, "git merge --abort");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    // A Failed run with an open PR prints it, sent back to draft.
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    assert!(
        result
            .stderr
            .contains("origin/main is not merged into issue-7"),
        "stderr: {}",
        result.stderr
    );
}

/// While thirdshift waits for CI on each of the next `times` new head commits,
/// someone else pushes `file` with `content` to main. The script goes inside
/// single quotes, so it must not contain one.
fn base_moves_during_ci(times: usize, file: &str, content: &str) -> String {
    format!(
        "gh fake on-ci-read {times} '{}'\n",
        base_moves_on(file, content)
    )
}

#[test]
fn merges_a_base_branch_that_moved_while_ci_passed() {
    let scenario = Scenario::new();
    // Green CI on the agent's commit; the merge commit after it has none.
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}\
         gh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\": \"test\", \"conclusion\": \"success\"}}]'\n\
         {}",
        base_moves_during_ci(1, "other.txt", "other")
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("CI passed on"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_merged_main_into_issue_7(&scenario);
}

#[test]
fn hands_a_conflict_from_a_base_branch_that_moved_during_the_ci_wait_to_a_repair() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        base_moves_during_ci(1, "feature.txt", "base feature")
    ));
    scenario.agent_does_in_session(2, REPAIR_RESOLVES_CONFLICT);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1]["merging"], true);
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    assert_merged_main_into_issue_7(&scenario);
    assert_eq!(
        scenario.origin_git(&["show", "issue-7:feature.txt"]),
        "feature\nbase feature\n"
    );
}

#[test]
fn fails_when_the_base_branch_keeps_moving_during_the_ci_wait() {
    let scenario = Scenario::new();
    // A different file each time, so every merge is clean.
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        base_moves_during_ci(100, "moved-$(date +%s%N).txt", "moved")
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert_eq!(scenario.claude_calls().len(), 1);
    // A Failed run with an open PR prints it, sent back to draft.
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    assert!(
        result
            .stderr
            .contains("origin/main kept moving while CI ran: merged it again 3 times"),
        "stderr: {}",
        result.stderr
    );
    // Three moves merged; the fourth ends the Run.
    let merges = scenario.origin_git(&["log", "--merges", "--format=%s", "issue-7"]);
    assert_eq!(merges.lines().count(), 3, "merges: {merges}");
}
