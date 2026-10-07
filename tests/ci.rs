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
         You run headless: nobody is watching, and ending your turn ends the session. Run tests and other long commands in the foreground, raising the command's timeout if needed. If a command is moved to the background, wait for that task by its own task id or output file, never by process names or patterns (`pgrep`, `ps | grep`, and the like): other sessions on this machine run the same commands. Never end your turn while a background task you depend on is still running: ending the turn kills it. Before ending your turn, stop every background task you no longer need, by its task id (with the `TaskStop` tool, if you have it): a task still running when your turn ends is taken as work you were waiting on.\n"
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
    let logs = scenario.entries("home/.thirdshift/logs/acme/widgets/sessions");
    assert!(
        logs.iter().any(|name| name.ends_with("-repair-1.jsonl")),
        "logs: {logs:?}"
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

/// Assert that a Failed run's stderr ends by naming the `kind` session's log,
/// then the Command log.
fn assert_session_log_is(result: &support::RunResult, kind: &str) {
    let last = *support::before_command_log(&result.stderr)
        .last()
        .unwrap_or(&"");
    assert!(
        last.starts_with("thirdshift: session log: ") && last.ends_with(&format!("-{kind}.jsonl")),
        "stderr: {}",
        result.stderr
    );
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

/// Assert that the Run ended as a Declined CI fix with no Check re-run asked
/// for, or with only `requests`. Returns the head whose CI it watched.
fn assert_declined_ci_fix(
    scenario: &Scenario,
    result: &support::RunResult,
    requests: &[&str],
) -> String {
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
    watched.trim().to_string()
}

#[test]
fn a_repair_that_already_re_ran_ci_to_green_delivers_or_self_merges_the_unchanged_head() {
    for merge in [false, true] {
        for pending in [false, true] {
            let scenario = Scenario::new();
            let rerun = if pending {
                r#"{"conclusion": "success", "pending_polls": 3}"#
            } else {
                RERUN_PASSES
            };
            scenario.agent_does(&format!(
                "{AGENT_OPENS_PR}{}",
                checks_on_head(&format!("[{}]", actions_failure("test", 900, 1, rerun)))
            ));
            scenario.agent_does_in_session(2, "gh run rerun 900 --failed --repo acme/widgets\n");
            let issue = scenario.issue_url(7);
            let args = if merge {
                vec!["merge", &issue]
            } else {
                vec![issue.as_str()]
            };

            let result = scenario.run(&args);

            assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
            assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
            assert_one_repair(&scenario, &result);
            assert_eq!(rerun_requests(&scenario), ["900", "900"]);
            assert!(
                result.stderr.contains("were not re-run"),
                "{}",
                result.stderr
            );
            assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
            if pending {
                assert!(
                    result.stderr.contains("1 of 1 checks still running"),
                    "{}",
                    result.stderr
                );
            }
            if merge {
                assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
                let head = scenario.origin_git(&["rev-parse", "main^2"]);
                assert_eq!(
                    scenario.gh_calls_of("pr", "merge")[0],
                    [
                        "pr",
                        "merge",
                        "1",
                        "--repo",
                        "acme/widgets",
                        "--merge",
                        "--match-head-commit",
                        head.trim()
                    ]
                );
                assert!(
                    result
                        .stderr
                        .contains(&format!("CI passed on {}", &head[..7])),
                    "{}",
                    result.stderr
                );
                assert_eq!(
                    scenario
                        .origin_git(&["log", "-1", "--format=%s", head.trim()])
                        .trim(),
                    "Add feature"
                );
                assert_eq!(scenario.origin_log("issue-7"), None);
            } else {
                assert_eq!(
                    scenario.origin_log("issue-7"),
                    Some(vec![
                        "Add feature".to_string(),
                        "Initial commit".to_string()
                    ])
                );
                assert!(scenario.gh_calls_of("pr", "merge").is_empty());
            }
            scenario.assert_cleaned_up("issue-7");
        }
    }
}

#[test]
fn a_refused_request_still_waits_for_an_earlier_accepted_workflows_new_attempt() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        checks_on_head(&format!(
            "[{}, {}]",
            actions_failure(
                "test",
                900,
                1,
                r#"{"conclusion": "success", "stale_polls": 3, "pending_polls": 2}"#
            ),
            actions_failure("lint", 901, 2, RERUN_PASSES)
        ))
    ));
    scenario.agent_does_in_session(2, "gh run rerun 901 --failed --repo acme/widgets\n");

    let result = scenario.run_with_env(
        &[&scenario.issue_url(7)],
        &[("THIRDSHIFT_CI_GRACE_MS", "5000")],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_one_repair(&scenario, &result);
    assert_eq!(rerun_requests(&scenario), ["901", "900", "901"]);
    assert!(
        result.stderr.contains("were not re-run"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("1 of 2 checks still running"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
}

#[test]
fn a_refused_request_cannot_deliver_or_merge_without_readable_nonempty_checks() {
    for merge in [false, true] {
        for empty in [true, false] {
            let scenario = Scenario::new();
            scenario.agent_does(&format!(
                "{AGENT_OPENS_PR}{}gh fake fails 'run rerun'\n",
                checks_on_head(&format!(
                    "[{}]",
                    actions_failure("test", 900, 1, RERUN_PASSES)
                ))
            ));
            let repair = if empty {
                checks_on_head("[]")
            } else {
                "gh fake fails 'api'\n".to_string()
            };
            scenario.agent_does_in_session(2, &repair);
            let issue = scenario.issue_url(7);
            let args = if merge {
                vec!["merge", &issue]
            } else {
                vec![issue.as_str()]
            };

            let result = scenario.run(&args);

            assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
            assert_one_repair(&scenario, &result);
            assert_eq!(rerun_requests(&scenario), ["900"]);
            assert!(scenario.gh_calls_of("pr", "merge").is_empty());
            assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
            assert!(
                result.stderr.contains("were not re-run"),
                "{}",
                result.stderr
            );
            let cause = if empty {
                "CI checks disappeared"
            } else {
                "gh api repos/acme/widgets/commits/"
            };
            assert!(result.stderr.contains(cause), "{}", result.stderr);
        }
    }
}

#[test]
fn a_refused_request_pending_then_red_is_a_declined_ci_fix_without_another_repair() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        checks_on_head(&format!(
            "[{}]",
            actions_failure("test", 900, 1, RERUN_PASSES)
        ))
    ));
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}gh fake fails 'run rerun'\n",
            checks_on_head(
                r#"[{"name": "test", "conclusion": "failure", "pending_polls": 3,
                     "url": "https://github.com/acme/widgets/actions/runs/900/job/1"}]"#
            )
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_declined_ci_fix(&scenario, &result, &["900"]);
    assert!(
        result.stderr.contains("were not re-run"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("1 of 1 checks still running"),
        "{}",
        result.stderr
    );
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

    let watched = assert_declined_ci_fix(&scenario, &result, &[]);
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

    let watched = assert_declined_ci_fix(&scenario, &result, &["900"]);
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: the failed checks on {} were not re-run: gh run rerun 900 --failed --repo acme/widgets failed: HTTP 502: Bad Gateway",
            &watched[..7]
        )),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        result.stderr.matches("CI failed on").count(),
        2,
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
fn an_inherited_failure_re_run_along_with_the_branchs_own_is_waited_for_too() {
    let scenario = Scenario::new();
    // lint shares test's workflow run, so GitHub re-runs it too, and goes on
    // listing its failed attempt for a few reads after test's new one.
    let inherited = actions_failure(
        "lint",
        900,
        1,
        r#"{"conclusion": "success", "stale_polls": 3}"#,
    );
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}",
        checks_on_head(&format!(
            "[{}, {inherited}]",
            actions_failure("test", 900, 2, RERUN_PASSES)
        )),
        checks_on_base(r#"[{"name": "lint", "conclusion": "failure"}]"#)
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

    let watched = assert_declined_ci_fix(&scenario, &result, &["900"]);
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
         You run headless: nobody is watching, and ending your turn ends the session. Run tests and other long commands in the foreground, raising the command's timeout if needed. If a command is moved to the background, wait for that task by its own task id or output file, never by process names or patterns (`pgrep`, `ps | grep`, and the like): other sessions on this machine run the same commands. Never end your turn while a background task you depend on is still running: ending the turn kills it. Before ending your turn, stop every background task you no longer need, by its task id (with the `TaskStop` tool, if you have it): a task still running when your turn ends is taken as work you were waiting on.\n"
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
