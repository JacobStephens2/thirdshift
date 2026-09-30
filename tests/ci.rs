//! Watching CI: after each push thirdshift waits for checks on the head
//! commit, hands red CI to a CI-fix Repair, and gives up after 5 Repairs, or
//! as soon as a CI-fix Repair makes no commit.

mod support;

use support::Scenario;

/// The agent adds feature.txt and opens a PR.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn no_checks_within_the_grace_period_means_no_ci_and_success() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert!(
        result.stderr.contains("no CI checks appeared"),
        "stderr: {}",
        result.stderr
    );
    scenario.assert_cleaned_up("issue-7");
}

/// Bash that sets the check runs on the worktree's HEAD to `checks`, a JSON
/// list, on the fake GitHub.
fn checks_on_head(checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse HEAD)\" '{checks}'\n")
}

/// Bash that commits a fix, `fix-<n>.txt`, as the CI-fix Repair would.
fn commits_fix(n: usize) -> String {
    format!("echo fix > fix-{n}.txt\ngit add fix-{n}.txt\ngit commit -q -m \"Fix CI {n}\"\n")
}

const GREEN: &str =
    r#"[{"name": "test", "conclusion": "success", "url": "https://ci.example/test"}]"#;
const RED: &str =
    r#"[{"name": "test", "conclusion": "failure", "url": "https://ci.example/test"}]"#;

#[test]
fn checks_pending_then_green_is_success() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        checks_on_head(
            r#"[{"name": "test", "conclusion": "success", "pending_polls": 3},
                {"name": "lint", "conclusion": "skipped"}]"#
        )
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert!(
        result.stderr.contains("1 of 2 checks still running"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("CI passed"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn red_ci_is_handed_to_a_ci_fix_repair_whose_fix_turns_it_green() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}gh fake statuses \"$(git rev-parse HEAD)\" '{}'\n",
        checks_on_head(
            r#"[{"name": "test", "conclusion": "failure", "url": "https://ci.example/test"},
                {"name": "build", "conclusion": "success", "url": "https://ci.example/build"}]"#
        ),
        r#"[{"context": "deploy/preview", "state": "error", "url": "https://deploy.example/7"}]"#,
    ));
    scenario.agent_does_in_session(2, &format!("{}{}", commits_fix(1), checks_on_head(GREEN)));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1]["prompt"],
        "CI failed on pull request https://github.com/acme/widgets/pull/1 (branch issue-7, implementing https://github.com/acme/widgets/issues/7).\n\
         \n\
         Failed checks:\n\
         - test: https://ci.example/test\n\
         - deploy/preview: https://deploy.example/7\n\
         \n\
         Read the failure logs (e.g. `gh run view <run-id> --log-failed`), find the root cause, and fix it. Do not skip, disable, or weaken tests or checks to make them pass.\n\
         Run the affected checks locally, commit, and push issue-7.\n\
         \n\
         If a failure is not caused by this branch (it is flaky, or also fails on main), do not change code for it. Instead, add it to a \"CI notes\" section of the pull request body with a one-line explanation.\n\
         \n\
         You run headless: nobody is watching, and ending your turn ends the session. Run tests and other long commands in the foreground, raising the Bash timeout if needed. If a command is moved to the background, wait for that task by its own task id or output file, never by process names or patterns (`pgrep`, `ps | grep`, and the like): other sessions on this machine run the same commands. Never end your turn while a background task you depend on is still running: ending the turn kills it.\n"
    );
    assert_eq!(calls[1]["cwd"], calls[0]["cwd"]);
    // thirdshift pushes the fix even though the Repair didn't.
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Fix CI 1".to_string(),
            "Add feature".to_string(),
            "Initial commit".to_string(),
        ])
    );
    let logs = scenario.entries("home/.thirdshift/logs");
    assert!(
        logs.iter().any(|name| name.ends_with("-repair-1.jsonl")),
        "logs: {logs:?}"
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn red_ci_after_the_last_repair_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    for session in 2..=6 {
        scenario.agent_does_in_session(
            session,
            &format!("{}{}", commits_fix(session - 1), checks_on_head(RED)),
        );
    }

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert!(
        result.stderr.contains("repairs exhausted: CI red"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        scenario.claude_calls().len(),
        6,
        "the implement session and 5 Repairs"
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[..2],
        [
            "thirdshift: failed run (repairs exhausted: CI red)".to_string(),
            "Fix CI 5".to_string(),
        ]
    );
    scenario.assert_cleaned_up("issue-7");
}

/// The cause of a Failed run whose CI-fix Repair left `head`, CI red, as it
/// was.
fn declined_ci_fix(head: &str) -> String {
    format!(
        "CI red on {} and the Repair found nothing to fix on the branch",
        &head[..7]
    )
}

/// Assert that a Failed run's stderr ends by naming the `kind` session's log.
fn assert_session_log_is(result: &support::RunResult, kind: &str) {
    let last = result.stderr.lines().last().unwrap_or_default();
    assert!(
        last.starts_with("thirdshift: session log: ") && last.ends_with(&format!("-{kind}.jsonl")),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_ci_fix_repair_that_makes_no_commit_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    // The Repair finds the failure isn't the branch's and commits nothing.
    scenario.agent_does_in_session(2, "true\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let watched = scenario.origin_git(&["rev-parse", "issue-7~1"]);
    let cause = declined_ci_fix(watched.trim());
    assert!(result.stderr.contains(&cause), "stderr: {}", result.stderr);
    assert!(
        !result.stderr.contains("repairs exhausted"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        result.stderr.matches("starting Repair").count(),
        1,
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        scenario.claude_calls().len(),
        2,
        "the implement session and 1 Repair"
    );
    assert_session_log_is(&result, "repair-1");
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            format!("thirdshift: failed run ({cause})"),
            "Add feature".to_string(),
            "Initial commit".to_string(),
        ])
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_second_ci_fix_repair_that_makes_no_commit_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    scenario.agent_does_in_session(2, &format!("{}{}", commits_fix(1), checks_on_head(RED)));
    // The second Repair commits nothing.
    scenario.agent_does_in_session(3, "true\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    let watched = scenario.origin_git(&["rev-parse", "issue-7~1"]);
    assert!(
        result.stderr.contains(&declined_ci_fix(watched.trim())),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        scenario.claude_calls().len(),
        3,
        "the implement session and 2 Repairs"
    );
    assert_session_log_is(&result, "repair-2");
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[1],
        "Fix CI 1".to_string()
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
}

/// Bash that pushes `file` with `content` to main from another clone, as if
/// someone else's work landed while the agent was busy.
fn base_moves_on(file: &str, content: &str) -> String {
    format!(
        r#"
other="$(mktemp -d)"
git clone -q https://github.com/acme/widgets.git "$other"
echo "{content}" > "$other/{file}"
git -C "$other" add {file}
git -C "$other" commit -q -m "Base moves on: {file}"
git -C "$other" push -q origin main
rm -rf "$other"
"#
    )
}

/// The conflict Repair keeps both sides of `file` and finishes the merge.
fn resolves_conflict(file: &str) -> String {
    format!("echo resolved > {file}\ngit add {file}\ngit commit -q --no-edit\n")
}

#[test]
fn a_ci_fix_repair_that_makes_no_commit_goes_round_if_the_base_branch_moved() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    // The Repair commits nothing, but main moves on meanwhile, so CI is
    // watched on the merge commit instead, which has no checks.
    scenario.agent_does_in_session(2, &base_moves_on("other.txt", "other"));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    assert!(
        result.stderr.contains("no CI checks appeared"),
        "stderr: {}",
        result.stderr
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn conflict_and_ci_fix_repairs_share_one_cap() {
    let scenario = Scenario::new();
    // Repair 1 resolves a conflict and leaves CI red; Repairs 2 to 4 fix CI
    // but it stays red; Repair 5 fixes CI again, but main moves on to a
    // conflict, which would need a 6th Repair.
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        base_moves_on("feature.txt", "base feature")
    ));
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}{}",
            resolves_conflict("feature.txt"),
            checks_on_head(RED)
        ),
    );
    for session in 3..=5 {
        scenario.agent_does_in_session(
            session,
            &format!("{}{}", commits_fix(session - 2), checks_on_head(RED)),
        );
    }
    scenario.agent_does_in_session(
        6,
        &format!(
            "{}{}{}",
            commits_fix(4),
            checks_on_head(GREEN),
            base_moves_on("fix-4.txt", "base fix")
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert!(
        result.stderr.contains("repairs exhausted: conflict"),
        "stderr: {}",
        result.stderr
    );
    let prompts: Vec<String> = scenario
        .claude_calls()
        .iter()
        .map(|call| {
            call["prompt"]
                .as_str()
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(prompts.len(), 6, "prompts: {prompts:?}");
    assert_eq!(prompts[1], "/thirdshift:resolving-merge-conflicts");
    for prompt in &prompts[2..] {
        assert!(
            prompt.starts_with("CI failed on pull request"),
            "prompts: {prompts:?}"
        );
    }
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn after_a_repair_the_base_branch_is_merged_again_before_ci_is_watched() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    // The fix is green on its own, but main moves on meanwhile, so CI is
    // watched on the merge commit instead, which has no checks.
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}{}{}",
            commits_fix(1),
            checks_on_head(GREEN),
            base_moves_on("other.txt", "other")
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    assert_eq!(
        scenario.origin_git(&["log", "--first-parent", "--format=%s", "issue-7"]),
        "Merge remote-tracking branch 'origin/main' into issue-7\n\
         Fix CI 1\n\
         Add feature\n\
         Initial commit\n"
    );
    let merge = scenario.origin_git(&["rev-parse", "issue-7"]);
    let last_checks_path = scenario
        .gh_calls()
        .into_iter()
        .filter(|call| call[0] == "api")
        .filter_map(|call| call.last().cloned())
        .rfind(|path| path.contains("/check-runs"))
        .unwrap();
    assert!(
        last_checks_path.contains(merge.trim()),
        "last watched: {last_checks_path}, merge: {merge}"
    );
    assert!(
        result.stderr.contains("no CI checks appeared"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_pr_closed_by_the_end_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}{}gh fake pr issue-7 state '\"CLOSED\"'\n",
            commits_fix(1),
            checks_on_head(GREEN)
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert!(
        result
            .stderr
            .contains("PR https://github.com/acme/widgets/pull/1 is closed, not open"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout, "", "a closed PR is not printed");
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn an_unmergeable_pr_at_the_end_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake pr issue-7 mergeable '\"CONFLICTING\"'\n"
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert!(
        result
            .stderr
            .contains("PR https://github.com/acme/widgets/pull/1 is not mergeable"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
}

#[test]
fn a_pr_sent_back_to_draft_by_the_end_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}{}gh pr ready issue-7 --undo\n",
            commits_fix(1),
            checks_on_head(GREEN)
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert!(
        result.stderr.contains("is a draft"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn waits_for_github_to_work_out_whether_the_pr_is_mergeable() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake pr issue-7 unknown_polls 3\n{}",
        checks_on_head(GREEN)
    ));

    // At the harness's 100ms poll interval, the default 300ms grace period
    // holds only about three reads, and this needs a fourth. Green checks keep
    // the longer grace period from slowing the CI wait.
    let result = scenario.run_with_env(
        &[&scenario.issue_url(7)],
        &[("THIRDSHIFT_CI_GRACE_MS", "5000")],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
}

#[test]
fn a_pr_whose_mergeability_stays_unknown_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake pr issue-7 mergeable '\"UNKNOWN\"'\n"
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert!(
        result.stderr.contains(
            "GitHub has not worked out whether PR https://github.com/acme/widgets/pull/1 is mergeable"
        ),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
}
