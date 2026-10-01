//! Watching CI: after each push thirdshift waits for checks on the head
//! commit, hands red CI to a CI-fix Repair, and gives up after 5 Repairs, or
//! once a CI-fix Repair makes no commit and a Check re-run, if the failed
//! checks can have one, leaves CI red. A red check that also fails on the
//! Base branch commit the head merged in is an Inherited failure, which gets
//! no Repair and no Check re-run.

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

/// A check run named `name`, job `job` of the GitHub Actions workflow run
/// `run`, that failed. With a `rerun`, as `{"conclusion": "success"}`, that is
/// what its Check re-run comes to; with `null`, it fails again.
fn actions_failure(name: &str, run: u32, job: u32, rerun: &str) -> String {
    format!(
        r#"{{"name": "{name}", "conclusion": "failure",
             "url": "https://github.com/acme/widgets/actions/runs/{run}/job/{job}",
             "rerun": {rerun}}}"#
    )
}

const RERUN_PASSES: &str = r#"{"conclusion": "success"}"#;

/// The workflow runs thirdshift asked GitHub to re-run the failed jobs of, in
/// order.
fn rerun_requests(scenario: &Scenario) -> Vec<String> {
    scenario
        .gh_calls_of("run", "rerun")
        .into_iter()
        .map(|call| {
            assert_eq!(call[3..], ["--failed", "--repo", "acme/widgets"]);
            call[2].clone()
        })
        .collect()
}

/// Assert that the Run started exactly one Repair.
fn assert_one_repair(scenario: &Scenario, result: &support::RunResult) {
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
}

#[test]
fn a_ci_fix_repair_that_makes_no_commit_gets_one_check_re_run_and_green_is_success() {
    let scenario = Scenario::new();
    // Two failed checks of one workflow run, and a third of another.
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        checks_on_head(&format!(
            "[{}, {}, {}]",
            actions_failure("test", 900, 1, RERUN_PASSES),
            actions_failure("lint", 900, 2, RERUN_PASSES),
            actions_failure("docs", 901, 3, RERUN_PASSES)
        ))
    ));
    // The Repair finds the failure flaky and commits nothing.
    scenario.agent_does_in_session(2, "true\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_one_repair(&scenario, &result);
    assert_eq!(rerun_requests(&scenario), ["900", "901"]);
    let head = scenario.origin_git(&["rev-parse", "issue-7"]);
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: re-running the failed checks on {}: test, lint, docs\n",
            &head[..7]
        )),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains(&format!("CI passed on {}", &head[..7])),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Add feature".to_string(),
            "Initial commit".to_string()
        ])
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_check_re_run_that_fails_again_is_a_failed_run_with_no_second_repair_or_re_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        checks_on_head(&format!("[{}]", actions_failure("test", 900, 1, "null")))
    ));
    scenario.agent_does_in_session(2, "true\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert_one_repair(&scenario, &result);
    assert_eq!(rerun_requests(&scenario), ["900"]);
    let watched = scenario.origin_git(&["rev-parse", "issue-7~1"]);
    let cause = declined_ci_fix(watched.trim());
    assert!(result.stderr.contains(&cause), "stderr: {}", result.stderr);
    assert_eq!(
        result.stderr.matches("CI failed on").count(),
        2,
        "before the Repair and after the Check re-run; stderr: {}",
        result.stderr
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
}

/// Assert that the Run ended as a Declined CI fix with no Check re-run asked
/// for, or with only `requests`.
fn assert_declined_ci_fix(scenario: &Scenario, result: &support::RunResult, requests: &[&str]) {
    assert_ne!(result.code, Some(0));
    assert_one_repair(scenario, result);
    assert_eq!(rerun_requests(scenario), requests);
    let watched = scenario.origin_git(&["rev-parse", "issue-7~1"]);
    let cause = declined_ci_fix(watched.trim());
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        format!("thirdshift: failed run ({cause})"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
}

#[test]
fn a_failed_commit_status_cant_be_re_run_so_nothing_is() {
    let scenario = Scenario::new();
    // The check run could be re-run, but the commit status could not, so the
    // head could not go green.
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}gh fake statuses \"$(git rev-parse HEAD)\" '{}'\n",
        checks_on_head(&format!(
            "[{}]",
            actions_failure("test", 900, 1, RERUN_PASSES)
        )),
        r#"[{"context": "deploy/preview", "state": "error", "url": "https://deploy.example/7"}]"#,
    ));
    scenario.agent_does_in_session(2, "true\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_declined_ci_fix(&scenario, &result, &[]);
    let watched = scenario.origin_git(&["rev-parse", "issue-7~1"]);
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: the failed checks on {} can't be re-run: not GitHub Actions jobs: deploy/preview\n",
            &watched[..7]
        )),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_check_re_run_github_refuses_is_a_declined_ci_fix_that_shows_the_refusal() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}gh fake fails 'run rerun'\n",
        checks_on_head(&format!(
            "[{}]",
            actions_failure("test", 900, 1, RERUN_PASSES)
        ))
    ));
    scenario.agent_does_in_session(2, "true\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_declined_ci_fix(&scenario, &result, &["900"]);
    assert!(
        result.stderr.contains(
            "thirdshift: GitHub refused the re-run: gh run rerun 900 --failed --repo acme/widgets failed: HTTP 502: Bad Gateway"
        ),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        result.stderr.matches("CI failed on").count(),
        1,
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_check_re_run_leaves_inherited_failures_alone() {
    let scenario = Scenario::new();
    let inherited = actions_failure("lint", 800, 1, RERUN_PASSES);
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}",
        checks_on_head(&format!(
            "[{}, {inherited}]",
            actions_failure("test", 900, 2, RERUN_PASSES)
        )),
        checks_on_base(&format!("[{inherited}]"))
    ));
    scenario.agent_does_in_session(2, "true\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    // test is green after its Check re-run; lint is as red as on main.
    assert_ne!(result.code, Some(0));
    assert_one_repair(&scenario, &result);
    assert_eq!(rerun_requests(&scenario), ["900"]);
    let watched = scenario.origin_git(&["rev-parse", "issue-7~1"]);
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: re-running the failed checks on {}: test\n",
            &watched[..7]
        )),
        "stderr: {}",
        result.stderr
    );
    let base_commit = scenario.origin_git(&["rev-parse", "main"]);
    let cause = inherited_failure("lint", &base_commit);
    assert!(result.stderr.contains(&cause), "stderr: {}", result.stderr);
    assert!(
        !result.stderr.contains("nothing to fix on the branch"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_ci_fix_repair_that_commits_gets_no_check_re_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        checks_on_head(&format!(
            "[{}]",
            actions_failure("test", 900, 1, RERUN_PASSES)
        ))
    ));
    scenario.agent_does_in_session(2, &format!("{}{}", commits_fix(1), checks_on_head(GREEN)));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_one_repair(&scenario, &result);
    assert_eq!(rerun_requests(&scenario), Vec::<String>::new());
    assert!(
        !result.stderr.contains("re-running"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn the_attempt_before_a_check_re_run_is_never_taken_for_its_result() {
    let scenario = Scenario::new();
    // GitHub goes on listing the failed attempt for a few reads after the
    // re-run is asked for, then the new attempt, running, then passed.
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        checks_on_head(&format!(
            "[{}]",
            actions_failure(
                "test",
                900,
                1,
                r#"{"conclusion": "success", "stale_polls": 3, "pending_polls": 2}"#
            )
        ))
    ));
    scenario.agent_does_in_session(2, "true\n");

    // The default 300ms grace period holds only about three reads.
    let result = scenario.run_with_env(
        &[&scenario.issue_url(7)],
        &[("THIRDSHIFT_CI_GRACE_MS", "5000")],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_one_repair(&scenario, &result);
    assert_eq!(rerun_requests(&scenario), ["900"]);
    assert_eq!(
        result.stderr.matches("CI failed on").count(),
        1,
        "only before the Repair; stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("1 of 1 checks still running"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
}

#[test]
fn a_check_re_run_that_never_appears_is_a_declined_ci_fix() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        checks_on_head(&format!(
            "[{}]",
            actions_failure(
                "test",
                900,
                1,
                r#"{"conclusion": "success", "stale_polls": 1000}"#
            )
        ))
    ));
    scenario.agent_does_in_session(2, "true\n");

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_declined_ci_fix(&scenario, &result, &["900"]);
    let watched = scenario.origin_git(&["rev-parse", "issue-7~1"]);
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: no re-run appeared on {} within 0s\n",
            &watched[..7]
        )),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        result.stderr.matches("CI failed on").count(),
        1,
        "only before the Repair; stderr: {}",
        result.stderr
    );
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

/// Bash that sets the check runs on `origin/main`'s tip, as the worktree last
/// fetched it, to `checks`.
fn checks_on_base(checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse origin/main)\" '{checks}'\n")
}

/// The cause of a Failed run whose red `checks` are all Inherited failures
/// from `main` at `base_commit`.
fn inherited_failure(checks: &str, base_commit: &str) -> String {
    format!(
        "CI red on {checks}, which also fails on main at {}; fix main first",
        &base_commit.trim()[..7]
    )
}

#[test]
fn checks_that_also_fail_on_the_base_branch_commit_fail_the_run_without_a_repair() {
    const BOTH_RED: &str = r#"[{"name": "test", "conclusion": "failure"},
                               {"name": "lint", "conclusion": "failure"},
                               {"name": "build", "conclusion": "success"}]"#;
    let scenario = Scenario::new();
    // A commit status is compared like a check run.
    const STATUS_RED: &str = r#"[{"context": "deploy/preview", "state": "error"}]"#;
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}\
         gh fake statuses \"$(git rev-parse HEAD)\" '{STATUS_RED}'\n\
         gh fake statuses \"$(git rev-parse origin/main)\" '{STATUS_RED}'\n",
        checks_on_head(BOTH_RED),
        checks_on_base(BOTH_RED)
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let base_commit = scenario.origin_git(&["rev-parse", "main"]);
    let cause = inherited_failure("test, lint, deploy/preview", &base_commit);
    assert!(result.stderr.contains(&cause), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: Inherited failures (also failing on main at {}): test, lint, deploy/preview\n",
            &base_commit[..7]
        )),
        "stderr: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("starting Repair"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1, "the implement session");
    assert_session_log_is(&result, "implement");
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
fn with_mixed_failures_the_repair_fixes_only_the_branchs_own_and_the_run_then_fails_on_the_inherited()
 {
    const TEST_RED: &str =
        r#"{"name": "test", "conclusion": "failure", "url": "https://ci.example/test"}"#;
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}",
        checks_on_head(&format!(
            r#"[{TEST_RED},
                {{"name": "lint", "conclusion": "failure", "url": "https://ci.example/lint"}}]"#
        )),
        checks_on_base(&format!("[{TEST_RED}]"))
    ));
    // The Repair turns lint green; test stays red, as on main.
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}{}",
            commits_fix(1),
            checks_on_head(&format!(
                r#"[{TEST_RED}, {{"name": "lint", "conclusion": "success"}}]"#
            ))
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "the implement session and 1 Repair");
    assert_eq!(
        calls[1]["prompt"],
        "CI failed on pull request https://github.com/acme/widgets/pull/1 (branch issue-7, implementing https://github.com/acme/widgets/issues/7).\n\
         \n\
         Failed checks:\n\
         - lint: https://ci.example/lint\n\
         \n\
         Also failing on `main`; don't fix:\n\
         - test: https://ci.example/test\n\
         \n\
         Read the failure logs (e.g. `gh run view <run-id> --log-failed`), find the root cause, and fix it. Do not skip, disable, or weaken tests or checks to make them pass.\n\
         Run the affected checks locally, commit, and push issue-7.\n\
         \n\
         If a failure is not caused by this branch (it is flaky, or also fails on main), do not change code for it. Instead, add it to a \"CI notes\" section of the pull request body with a one-line explanation.\n\
         \n\
         You run headless: nobody is watching, and ending your turn ends the session. Run tests and other long commands in the foreground, raising the Bash timeout if needed. If a command is moved to the background, wait for that task by its own task id or output file, never by process names or patterns (`pgrep`, `ps | grep`, and the like): other sessions on this machine run the same commands. Never end your turn while a background task you depend on is still running: ending the turn kills it.\n"
    );
    let base_commit = scenario.origin_git(&["rev-parse", "main"]);
    let cause = inherited_failure("test", &base_commit);
    assert!(result.stderr.contains(&cause), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[..2],
        [
            format!("thirdshift: failed run ({cause})"),
            "Fix CI 1".to_string()
        ]
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
}

/// Assert that `prompt` is a CI-fix Repair's that names no Inherited failure.
fn assert_ci_fix_repair_with_no_inherited_failures(prompt: &serde_json::Value) {
    let prompt = prompt.as_str().unwrap();
    assert!(
        prompt.starts_with("CI failed on pull request") && prompt.contains("- test"),
        "prompt: {prompt}"
    );
    assert!(!prompt.contains("Also failing on"), "prompt: {prompt}");
}

#[test]
fn a_check_still_pending_on_the_base_branch_commit_is_handed_to_a_ci_fix_repair() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}",
        checks_on_head(RED),
        checks_on_base(r#"[{"name": "test", "conclusion": "failure", "pending_polls": 1000}]"#)
    ));
    scenario.agent_does_in_session(2, &format!("{}{}", commits_fix(1), checks_on_head(GREEN)));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "the implement session and 1 Repair");
    assert_ci_fix_repair_with_no_inherited_failures(&calls[1]["prompt"]);
}

#[test]
fn a_check_passed_or_missing_on_the_base_branch_commit_is_handed_to_a_ci_fix_repair() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}",
        checks_on_head(
            r#"[{"name": "test", "conclusion": "failure"},
                {"name": "lint", "conclusion": "failure"},
                {"name": "build", "conclusion": "failure"}]"#
        ),
        // test passed on main, lint never ran there, only one of the two
        // checks named build failed there, and what else failed there is
        // another check.
        checks_on_base(
            r#"[{"name": "test", "conclusion": "success"},
                {"name": "build", "conclusion": "failure"},
                {"name": "build", "conclusion": "success"},
                {"name": "deploy", "conclusion": "failure"}]"#
        )
    ));
    scenario.agent_does_in_session(2, &format!("{}{}", commits_fix(1), checks_on_head(GREEN)));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "the implement session and 1 Repair");
    assert_ci_fix_repair_with_no_inherited_failures(&calls[1]["prompt"]);
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("- lint") && prompt.contains("- build"),
        "prompt: {prompt}"
    );
}

/// Bash that runs `script` the first time thirdshift reads the checks on each
/// of the next `times` commits, with the commit in `$FAKE_CI_SHA`.
fn on_ci_read(times: usize, script: &str) -> String {
    format!(
        "gh fake on-ci-read {times} '{}'\n",
        script.replace('\'', r"'\''")
    )
}

#[test]
fn when_the_base_branch_moved_since_inherited_failures_it_is_merged_again_and_ci_watched_again() {
    let scenario = Scenario::new();
    // main moves on while CI runs on the head, as if someone fixed it, so CI
    // is watched on the merge commit instead, which has no checks.
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}{}",
        checks_on_head(RED),
        checks_on_base(RED),
        on_ci_read(1, &base_moves_on("other.txt", "other"))
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1, "the implement session");
    assert!(
        result
            .stderr
            .contains("origin/main moved while CI ran; merging it again"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("no CI checks appeared"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_base_branch_that_moved_and_is_still_red_fails_the_run_naming_its_new_commit() {
    let scenario = Scenario::new();
    // main moves on, once, while CI runs on the head, and the check is as red
    // on main's new commit, and on the head that merges it, as it was before.
    let stays_red = format!(
        r#"
gh fake checks "$FAKE_CI_SHA" '{RED}'
moved="$(dirname "$FAKE_GH_STATE")/base-moved"
if [ ! -e "$moved" ]; then
touch "$moved"
{}
gh fake checks "$(git ls-remote https://github.com/acme/widgets.git refs/heads/main | cut -f1)" '{RED}'
fi
"#,
        base_moves_on("other.txt", "other")
    );
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}{}",
        checks_on_head(RED),
        checks_on_base(RED),
        on_ci_read(4, &stays_red)
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_ne!(result.code, Some(0));
    assert_eq!(scenario.claude_calls().len(), 1, "the implement session");
    assert!(
        result
            .stderr
            .contains("origin/main moved while CI ran; merging it again"),
        "stderr: {}",
        result.stderr
    );
    let moved_base = scenario.origin_git(&["rev-parse", "main"]);
    assert_ne!(moved_base, scenario.origin_git(&["rev-parse", "main~1"]));
    let cause = inherited_failure("test", &moved_base);
    assert!(result.stderr.contains(&cause), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
}
