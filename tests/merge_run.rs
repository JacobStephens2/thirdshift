//! The Merge run: `thirdshift merge <Issue URL>` does everything a Run does,
//! then the Self-merge: it merges the ready, mergeable, green PR with a merge
//! commit on exactly the head commit whose CI it watched, then deletes the
//! Issue branch on origin and closes the issue unless it is already closed.

mod support;

use support::resend::ResendStandIn;
use support::{Scenario, leaves_running};

/// The agent commits its work and opens a PR that closes issue #7, into
/// `main`.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

#[test]
fn a_pr_merged_while_the_opening_session_runs_is_reported_as_merged() {
    for merge in [false, true] {
        let scenario = Scenario::new();
        // Simulate a human merging the published work while the session is
        // still running, then GitHub deleting the Issue branch.
        scenario.agent_does(&format!(
            "{AGENT_OPENS_PR}\
             git push -q origin HEAD\n\
             gh pr merge 1 --merge --match-head-commit \"$(git rev-parse HEAD)\"\n\
             git push -q origin --delete issue-7\n"
        ));
        let issue = scenario.issue_url(7);
        let args = if merge {
            vec!["merge", &issue]
        } else {
            vec![&*issue]
        };

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(result.stdout, format!("{PR_URL}\n"));
        assert_eq!(
            result.stderr.lines().last(),
            Some(format!("thirdshift: PR {PR_URL} is merged").as_str())
        );
        assert_eq!(scenario.gh_calls_of("pr", "merge").len(), 1);
        assert!(scenario.gh_calls_of("pr", "edit").is_empty());
        assert!(scenario.gh_calls_of("pr", "ready").is_empty());
        assert_eq!(scenario.origin_log("issue-7"), None);
        scenario.assert_cleaned_up("issue-7");
    }
}

#[test]
fn a_pr_merged_during_ci_is_reported_as_merged_without_recreating_its_branch() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        on_ci_read(
            1,
            r#"gh pr merge 1 --merge --match-head-commit "$(git ls-remote origin refs/heads/issue-7 | cut -f1)"
git push -q origin --delete issue-7"#
        )
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is merged").as_str())
    );
    assert_eq!(scenario.gh_calls_of("pr", "merge").len(), 1);
    assert_eq!(scenario.origin_log("issue-7"), None);
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_merged_pr_with_local_work_left_out_still_fails_and_retains_that_work() {
    for leftover in [
        "echo leftover >> feature.txt",
        "echo leftover > untracked.txt",
        "echo leftover >> feature.txt; git add feature.txt",
        "echo leftover >> feature.txt; git add feature.txt; git commit -qm Leftover",
    ] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "{AGENT_OPENS_PR}\
             git push -q origin HEAD\n\
             gh pr merge 1 --merge --match-head-commit \"$(git rev-parse HEAD)\"\n\
             git push -q origin --delete issue-7\n\
             {leftover}\n"
        ));

        let result = scenario.run(&["merge", &scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("is merged, not open"),
            "{}",
            result.stderr
        );
        assert!(
            result.stderr.contains("keeping the worktree"),
            "{}",
            result.stderr
        );
        let worktree = scenario.launch_dir().with_file_name("widgets-issue-7");
        let file = if leftover.contains("untracked.txt") {
            "untracked.txt"
        } else {
            "feature.txt"
        };
        assert!(
            std::fs::read_to_string(worktree.join(file))
                .unwrap()
                .contains("leftover")
        );
        assert_eq!(scenario.origin_log("issue-7"), None);
    }
}

#[test]
fn externally_merged_pr_retains_tracked_changes_hidden_by_index_flags() {
    for flag in ["assume-unchanged", "skip-worktree"] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "{AGENT_OPENS_PR}\
             git push -q origin HEAD\n\
             gh pr merge 1 --merge --match-head-commit \"$(git rev-parse HEAD)\"\n\
             git push -q origin --delete issue-7\n\
             git update-index --{flag} feature.txt\n\
             echo leftover >> feature.txt\n"
        ));

        let result = scenario.run(&["merge", &scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{flag}: {}", result.stderr);
        let worktree = scenario.launch_dir().with_file_name("widgets-issue-7");
        assert!(
            std::fs::read_to_string(worktree.join("feature.txt"))
                .unwrap()
                .contains("leftover")
        );
        assert_eq!(scenario.origin_log("issue-7"), None);
    }
}

/// Launch from `develop`, a branch other than the default, and have the agent
/// open its PR into it, then run `then`.
fn merge_into_develop(scenario: &Scenario, then: &str) {
    scenario.origin_has_branch("develop", "main", &[]);
    scenario.launch_checks_out("develop");
    scenario.agent_does(&format!(
        "{}{then}",
        AGENT_OPENS_PR.replace("--base main", "--base develop")
    ));
}

const CLOSING_COMMENT: &str = "Closed by #1, merged into develop by a thirdshift Merge run.";

/// Assert that `base`'s tip on origin is a merge commit of the head commit
/// thirdshift asked `gh pr merge` to match, and return that head.
fn assert_issue_7_merged_into(scenario: &Scenario, base: &str) -> String {
    let calls = scenario.gh_calls_of("pr", "merge");
    let head = calls
        .last()
        .and_then(|call| call.last())
        .expect("thirdshift never ran gh pr merge")
        .clone();
    let parents = scenario.origin_git(&["log", "-1", "--format=%P", &format!("refs/heads/{base}")]);
    let parents: Vec<&str> = parents.split_whitespace().collect();
    assert_eq!(parents.len(), 2, "{base}'s tip is not a merge commit");
    assert_eq!(
        parents[1], head,
        "{base}'s tip does not merge issue-7's head"
    );
    head
}

#[test]
fn a_clean_merge_run_merges_the_pr_and_says_so() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is merged").as_str()),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
    assert_eq!(scenario.origin_log("issue-7"), None);
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn no_gh_call_asks_for_the_prs_closing_issue_references() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    scenario.run(&["merge", &scenario.issue_url(7)]);

    for call in scenario.gh_calls() {
        assert!(
            !call
                .iter()
                .any(|arg| arg.contains("closingIssuesReferences")),
            "gh {call:?} asks for a field gh 2.45 does not have"
        );
    }
}

#[test]
fn the_merge_is_a_merge_commit_of_exactly_the_watched_head() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let head = assert_issue_7_merged_into(&scenario, "main");
    let short = &head[..7];
    assert!(
        result
            .stderr
            .contains(&format!("waiting up to 0s for CI on {short}")),
        "CI was not watched on {short}: {}",
        result.stderr
    );
    assert_eq!(
        scenario.gh_calls_of("pr", "merge"),
        vec![vec![
            "pr",
            "merge",
            "1",
            "--repo",
            "acme/widgets",
            "--merge",
            "--match-head-commit",
            head.as_str(),
        ]]
    );
}

#[test]
fn the_merge_flag_also_starts_a_merge_run() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&["--merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
}

#[test]
fn a_merge_run_gives_the_agent_the_same_implement_prompt_as_a_run() {
    // Two Runs, one of them a Merge run: the behavior is that they agree.
    let run = Scenario::new();
    run.agent_does(AGENT_OPENS_PR);
    let merge_run = Scenario::new();
    merge_run.agent_does(AGENT_OPENS_PR);

    run.run(&[&run.issue_url(7)]);
    merge_run.run(&["merge", &merge_run.issue_url(7)]);

    assert_eq!(
        merge_run.first_prompt_with_report_placeholder(),
        run.first_prompt_with_report_placeholder()
    );
}

#[test]
fn a_run_without_merge_leaves_the_pr_open_and_ready() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is ready for review").as_str())
    );
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
    let pr = &scenario.gh_state()["prs"][0];
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], false);
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
}

#[test]
fn retargeting_during_ci_fails_delivery_and_drafts_the_captured_pr() {
    for merge in [false, true] {
        let scenario = Scenario::new();
        scenario.origin_has_branch("develop", "main", &[]);
        scenario.agent_does(&format!(
            "{AGENT_OPENS_PR}gh fake on-ci-read 1 'gh fake pr issue-7 base \"\\\"develop\\\"\"'\n"
        ));
        let issue = scenario.issue_url(7);
        let args = if merge {
            vec!["merge", &issue]
        } else {
            vec![&*issue]
        };

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
        assert!(
            result.stderr.contains("PR targets develop, not main"),
            "{}",
            result.stderr
        );
        assert!(scenario.gh_calls_of("pr", "merge").is_empty());
        assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
        assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
        assert!(scenario.origin_log("issue-7").is_some());
        assert_eq!(scenario.origin_log("main").unwrap(), vec!["Initial commit"]);
        assert_eq!(
            scenario.origin_log("develop").unwrap(),
            vec!["Initial commit"]
        );
    }
}

#[test]
fn a_replacement_from_the_same_branch_cannot_rescue_delivery_or_be_modified() {
    for merge in [false, true] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "{AGENT_OPENS_PR}{}",
            on_ci_read(
                1,
                r#"gh fake pr issue-7 state '"CLOSED"'
gh pr create --base main --head issue-7 --title Replacement --body untouched"#
            )
        ));
        let issue = scenario.issue_url(7);
        let args = if merge {
            vec!["merge", &issue]
        } else {
            vec![&*issue]
        };

        let result = scenario.run(&args);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("pull/1 is closed, not open"),
            "{}",
            result.stderr
        );
        assert_eq!(result.stdout, "");
        let state = scenario.gh_state();
        assert_eq!(state["prs"][1]["state"], "OPEN");
        assert_eq!(state["prs"][1]["isDraft"], false);
        assert_eq!(state["prs"][1]["body"], "untouched");
        assert!(scenario.gh_calls_of("pr", "merge").is_empty());
        assert_eq!(state["issues"]["7"], "OPEN");
        assert!(scenario.origin_log("issue-7").is_some());
    }
}

#[test]
fn a_merge_into_a_branch_other_than_the_default_closes_the_issue_with_a_comment() {
    let scenario = Scenario::new();
    merge_into_develop(&scenario, "");

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_issue_7_merged_into(&scenario, "develop");
    assert_eq!(scenario.origin_log("main").unwrap(), vec!["Initial commit"]);
    assert_eq!(scenario.origin_log("issue-7"), None);
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
    assert_eq!(
        scenario.gh_state()["comments"]["7"],
        serde_json::json!([CLOSING_COMMENT])
    );
}

#[test]
fn an_issue_close_failure_links_the_issue_and_keeps_the_manual_close_command() {
    let scenario = Scenario::new();
    merge_into_develop(&scenario, "");
    scenario.gh_fails("issue close");

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(
            "could not close issue #7 (https://github.com/acme/widgets/issues/7), \
             so if it is still open, close it by hand: \
             gh issue close 7 --repo acme/widgets --comment \
             'Closed by #1, merged into develop by a thirdshift Merge run.'"
        ),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is merged").as_str())
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn an_already_closed_issue_has_no_close_failure_warning() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake after-merge 'gh fake issue 7 CLOSED'\n"
    ));
    scenario.gh_fails("issue close");

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(scenario.gh_calls_of("issue", "close").is_empty());
    assert!(
        !result.stderr.contains("could not close issue"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
}

#[test]
fn an_issue_branch_already_deleted_by_github_is_no_warning() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake after-merge 'git push -q origin --delete issue-7'\n"
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.origin_log("issue-7"), None);
    assert!(
        !result.stderr.contains("warning"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn confirmed_merge_finishing_preserves_a_replacement_checkout_and_its_origin() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    let script = scenario.path("replace-after-merge.sh");
    let worktree = scenario.path("work/widgets-issue-7");
    let other_origin = scenario.path("replacement-origin.git");
    std::fs::write(
        &script,
        format!(
            r#"set -e
git clone -q --bare "$(git remote get-url origin)" '{other_origin}'
mv '{worktree}' '{worktree}.saved'
git clone -q --branch issue-7 '{other_origin}' '{worktree}'
printf 'replacement work\n' > '{worktree}/manual.txt'
"#,
            other_origin = other_origin.display(),
            worktree = worktree.display()
        ),
    )
    .unwrap();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake after-merge \"bash '{}'\"\n",
        script.display()
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.origin_log("issue-7"), None);
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
    assert!(scenario.issue_labels(7).is_empty());
    assert_eq!(
        std::fs::read_to_string(worktree.join("manual.txt")).unwrap(),
        "replacement work\n"
    );
    let replacement = std::process::Command::new("git")
        .args([
            "--git-dir",
            other_origin.to_str().unwrap(),
            "show-ref",
            "--verify",
            "refs/heads/issue-7",
        ])
        .output()
        .unwrap();
    assert!(
        replacement.status.success(),
        "replacement origin branch was deleted"
    );
    assert!(
        result.stderr.contains("retaining checkout"),
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("could not delete issue-7 on origin"),
        "{}",
        result.stderr
    );
}

#[test]
fn repository_identity_failure_during_confirmed_merge_finishing_warns_and_still_ends_the_claim() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    let script = scenario.path("replace-repository-after-merge.sh");
    let common = scenario.launch_dir().join(".git");
    std::fs::write(
        &script,
        format!(
            r#"set -e
mv '{common}' '{common}.saved'
mkdir '{common}'
"#,
            common = common.display()
        ),
    )
    .unwrap();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake after-merge \"bash '{}'\"\n",
        script.display()
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("could not delete issue-7 on origin"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("common repository identity"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
    assert!(scenario.issue_labels(7).is_empty());
    assert!(scenario.origin_log("issue-7").is_some());
}

#[test]
fn ctrl_c_after_the_merge_finishes_the_post_merge_steps_and_exits_as_merged() {
    let scenario = Scenario::new();
    merge_into_develop(
        &scenario,
        "gh fake after-merge 'touch \"$(dirname \"$FAKE_GH_STATE\")/merged\"; sleep 1'\n",
    );

    let result = scenario.run_and_signal(&["merge", &scenario.issue_url(7)], "merged", "INT");

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is merged").as_str())
    );
    assert_eq!(scenario.origin_log("issue-7"), None);
    assert_eq!(
        scenario.gh_state()["comments"]["7"],
        serde_json::json!([CLOSING_COMMENT])
    );
    assert_eq!(scenario.issue_labels(7), Vec::<String>::new());
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_merge_that_reports_failure_but_went_through_is_a_merge() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!("{AGENT_OPENS_PR}gh fake after-merge 'exit 1'\n"));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(scenario.origin_log("issue-7"), None);
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
}

#[test]
fn a_failed_merge_of_a_pr_someone_else_merged_at_another_head_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake after-merge 'gh fake pr issue-7 headRefOid \\\"0000000\\\"; exit 1'\n"
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
}

#[test]
fn ctrl_c_before_the_merge_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake on-ci-read 1 'touch \"$(dirname \"$FAKE_GH_STATE\")/watching\"; sleep 1'\n"
    ));

    let result = scenario.run_and_signal(&["merge", &scenario.issue_url(7)], "watching", "INT");

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("interrupted"),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "OPEN");
    assert_eq!(scenario.origin_log("main").unwrap(), vec!["Initial commit"]);
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
}

/// Bash that has someone else push `file` with `content` to issue-7 from
/// another clone, as a commit with `subject`: a Foreign commit.
fn someone_pushes_to_issue_7(file: &str, content: &str, subject: &str) -> String {
    format!(
        r#"
other="$(mktemp -d)"
git clone -q -b issue-7 https://github.com/acme/widgets.git "$other"
echo "{content}" > "$other/{file}"
git -C "$other" add -A
git -C "$other" commit -q -m "{subject}"
git -C "$other" push -q origin issue-7
rm -rf "$other"
"#
    )
}

/// Bash that runs `script` the first time thirdshift reads CI on each of the
/// next `times` head commits.
fn on_ci_read(times: usize, script: &str) -> String {
    format!(
        "gh fake on-ci-read {times} '{}'\n",
        script.replace('\'', r"'\''")
    )
}

/// The full sha of the commit with `subject` on `branch` in origin.
fn sha_of(scenario: &Scenario, branch: &str, subject: &str) -> String {
    scenario
        .origin_git(&[
            "log",
            "--format=%H",
            "--fixed-strings",
            &format!("--grep={subject}"),
            &format!("refs/heads/{branch}"),
        ])
        .trim()
        .to_string()
}

/// The full sha of `commit`'s first parent in origin.
fn parent_of(scenario: &Scenario, commit: &str) -> String {
    scenario
        .origin_git(&["rev-parse", &format!("{commit}^")])
        .trim()
        .to_string()
}

/// Assert that `prompt` is a review Repair's, reviewing from `fixed_point`.
fn assert_review_repair_from(prompt: &serde_json::Value, fixed_point: &str) {
    let prompt = prompt.as_str().unwrap();
    let fixed_point = fixed_point.trim();
    assert!(
        prompt.starts_with("/thirdshift-code-review"),
        "prompt: {prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "Review with the `thirdshift-code-review` skill using {fixed_point} as the fixed point"
        )),
        "prompt: {prompt}"
    );
    assert!(prompt.contains("Unaddressed findings"), "prompt: {prompt}");
}

#[test]
fn a_tree_identical_foreign_commit_is_merged_without_a_review_repair() {
    let scenario = Scenario::new();
    let empty_commit = someone_pushes_to_issue_7("feature.txt", "feature", "Empty review")
        .replace("commit -q -m", "commit -q --allow-empty -m");
    scenario.agent_does_in_session(
        1,
        &format!("{AGENT_OPENS_PR}{}", on_ci_read(1, &empty_commit)),
    );

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "only the implement session"
    );
    let foreign = sha_of(&scenario, "main", "Empty review");
    assert_eq!(assert_issue_7_merged_into(&scenario, "main"), foreign);
    assert_eq!(
        scenario.origin_file("main", "feature.txt").unwrap(),
        "feature\n"
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn cancelled_ci_after_a_foreign_push_is_superseded_without_a_ci_fix_or_check_rerun() {
    let scenario = Scenario::new();
    let cancelled = r#"[{"name":"test","conclusion":"cancelled","url":"https://github.com/acme/widgets/actions/runs/900/job/1","rerun":{"conclusion":"success"}}]"#;
    let green = r#"[{"name":"test","conclusion":"success"}]"#;
    scenario.agent_does_in_session(1, &format!(
        "{AGENT_OPENS_PR}gh fake checks \"$(git rev-parse HEAD)\" '{cancelled}'\n{}",
        on_ci_read(1, &format!(
            "{}\ngh fake checks \"$(git ls-remote origin refs/heads/issue-7 | cut -f1)\" '{green}'\n",
            someone_pushes_to_issue_7("late.txt", "late", "Late commit")
        ))
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("was superseded by a newer push"),
        "{}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("CI red; starting Repair"),
        "{}",
        result.stderr
    );
    assert!(scenario.gh_calls_of("run", "rerun").is_empty());
    let calls = scenario.claude_calls();
    assert_eq!(
        calls.len(),
        2,
        "only implementation and Foreign commit review: {}",
        result.stderr
    );
    let late = sha_of(&scenario, "main", "Late commit");
    assert_review_repair_from(&calls[1]["prompt"], &parent_of(&scenario, &late));
    assert!(
        result
            .stderr
            .contains(&format!("CI passed on {}", &late[..7])),
        "{}",
        result.stderr
    );
    assert_eq!(assert_issue_7_merged_into(&scenario, "main"), late);
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_foreign_commit_pushed_during_ci_is_merged_in_reviewed_then_merged() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}{}",
            on_ci_read(
                1,
                &someone_pushes_to_issue_7("late.txt", "late", "Late commit")
            )
        ),
    );

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    let late = sha_of(&scenario, "main", "Late commit");
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: merging new commit {late} from origin/issue-7\n"
        )),
        "stderr: {}",
        result.stderr
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2, "the implement session and a review Repair");
    assert_review_repair_from(&calls[1]["prompt"], &parent_of(&scenario, &late));
    assert!(
        result
            .stderr
            .contains("thirdshift: repair-1: session started"),
        "stderr: {}",
        result.stderr
    );
    // Found before the merge was tried, so the one merge is of the reviewed head.
    assert_eq!(scenario.gh_calls_of("pr", "merge").len(), 1);
    let head = assert_issue_7_merged_into(&scenario, "main");
    assert_eq!(head, late);
    assert_eq!(scenario.origin_file("main", "late.txt").unwrap(), "late\n");
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn an_empty_foreign_review_does_not_bounce_but_a_later_changed_batch_is_reviewed() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}{}",
            on_ci_read(
                1,
                &someone_pushes_to_issue_7("late.txt", "late", "First changed tree")
            )
        ),
    );
    // The other writer records an empty review while this Run's review makes
    // no changes. Later CI brings a genuinely changed tree from that writer.
    let empty_review =
        someone_pushes_to_issue_7("feature.txt", "feature", "Other writer empty review")
            .replace("commit -q -m", "commit -q --allow-empty -m");
    scenario.agent_does_in_session(
        2,
        &format!(
            "{empty_review}{}",
            on_ci_read(
                1,
                &someone_pushes_to_issue_7("later.txt", "later", "Second changed tree")
            )
        ),
    );

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 3, "implement and two reviews of changed trees");
    let first = sha_of(&scenario, "main", "First changed tree");
    let empty = sha_of(&scenario, "main", "Other writer empty review");
    let second = sha_of(&scenario, "main", "Second changed tree");
    assert_review_repair_from(&calls[1]["prompt"], &parent_of(&scenario, &first));
    assert_review_repair_from(&calls[2]["prompt"], &empty);
    assert_eq!(parent_of(&scenario, &empty), first);
    assert_eq!(parent_of(&scenario, &second), empty);
    assert_eq!(assert_issue_7_merged_into(&scenario, "main"), second);
    assert_eq!(
        scenario.origin_file("main", "later.txt").unwrap(),
        "later\n"
    );
    assert_eq!(
        scenario.gh_state()["prs"][0]["body"],
        "Closes #7\n\nBuilt with claude · default model · default effort <!-- thirdshift:built-with -->\n"
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn tree_identical_commits_from_another_writer_terminate_without_review_repairs() {
    let scenario = Scenario::new();
    let empty_commits = r#"
other="$(mktemp -d)"
git clone -q -b issue-7 https://github.com/acme/widgets.git "$other"
git -C "$other" commit -q --allow-empty -m "Other writer empty review"
git -C "$other" push -q origin issue-7
rm -rf "$other"
"#;
    scenario.agent_does_in_session(
        1,
        &format!("{AGENT_OPENS_PR}{}", on_ci_read(2, empty_commits)),
    );

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "no Review Repairs for either empty commit"
    );
    assert_eq!(
        scenario
            .origin_log("main")
            .unwrap()
            .iter()
            .filter(|subject| *subject == "Other writer empty review")
            .count(),
        2
    );
    assert_issue_7_merged_into(&scenario, "main");
    assert_eq!(
        scenario.gh_state()["prs"][0]["body"],
        "Closes #7\n\nBuilt with claude · default model · default effort <!-- thirdshift:built-with -->\n"
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_no_change_review_reconciles_an_empty_batch_and_still_reviews_later_changes() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}{}",
            on_ci_read(
                1,
                &someone_pushes_to_issue_7("first.txt", "first", "First changed tree")
            )
        ),
    );
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}{}",
            // An unrelated writer has independently reviewed the same tree.
            someone_pushes_to_issue_7("temporary.txt", "temporary", "Temporary tree")
                + r#"
other="$(mktemp -d)"
git clone -q -b issue-7 https://github.com/acme/widgets.git "$other"
git -C "$other" rm -q temporary.txt
git -C "$other" commit -q -m "Restore reviewed tree"
git -C "$other" push -q origin issue-7
rm -rf "$other"
"#,
            on_ci_read(
                1,
                &someone_pushes_to_issue_7("later.txt", "later", "Later changed tree")
            )
        ),
    );
    // Review sessions deliberately make no file change and no commit.
    scenario.agent_does_in_session(3, "true\n");

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(
        calls.len(),
        3,
        "implement and two changed-tree reviews only"
    );
    let first = sha_of(&scenario, "main", "First changed tree");
    assert_review_repair_from(&calls[1]["prompt"], &parent_of(&scenario, &first));
    let restored = sha_of(&scenario, "main", "Restore reviewed tree");
    assert_review_repair_from(&calls[2]["prompt"], &restored);
    assert_eq!(
        scenario.gh_state()["prs"][0]["body"],
        "Closes #7\n\nBuilt with claude · default model · default effort <!-- thirdshift:built-with -->\n"
    );
    assert_eq!(
        scenario.origin_file("main", "first.txt").unwrap(),
        "first\n"
    );
    assert_eq!(
        scenario.origin_file("main", "later.txt").unwrap(),
        "later\n"
    );
    assert_eq!(
        assert_issue_7_merged_into(&scenario, "main"),
        sha_of(&scenario, "main", "Later changed tree")
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_foreign_commit_that_conflicts_with_local_work_gets_a_conflict_repair_then_a_review() {
    let scenario = Scenario::new();
    // CI goes red; while the CI-fix Repair rewrites feature.txt, someone else
    // pushes their own change to it, so the fix can't be pushed as it is.
    scenario.agent_does_in_session(1, &format!("{AGENT_OPENS_PR}{}", checks_on_head(RED)));
    scenario.agent_does_in_session(
        2,
        &format!(
            "{}echo fixed > feature.txt\ngit commit -q -am \"Fix CI 1\"\n",
            someone_pushes_to_issue_7("feature.txt", "theirs", "Their feature")
        ),
    );
    scenario.agent_does_in_session(
        3,
        "echo both > feature.txt\ngit add feature.txt\ngit commit -q --no-edit\n",
    );

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 4, "implement, CI fix, conflict and review");
    assert_eq!(calls[2]["merging"], true);
    let conflict = calls[2]["prompt"].as_str().unwrap();
    assert!(
        conflict.starts_with("/thirdshift-resolving-merge-conflicts")
            && conflict.contains("A merge of origin/issue-7 into issue-7 is in progress"),
        "prompt: {conflict}"
    );
    assert_review_repair_from(&calls[3]["prompt"], &sha_of(&scenario, "main", "Fix CI 1"));
    let theirs = sha_of(&scenario, "main", "Their feature");
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: merging new commit {theirs} from origin/issue-7"
        )),
        "stderr: {}",
        result.stderr
    );
    for repair in ["repair-1", "repair-2", "repair-3"] {
        assert!(
            result
                .stderr
                .contains(&format!("thirdshift: {repair}: session started")),
            "no {repair} in stderr: {}",
            result.stderr
        );
    }
    assert_issue_7_merged_into(&scenario, "main");
    assert_eq!(
        scenario.origin_file("main", "feature.txt").unwrap(),
        "both\n"
    );
}

#[test]
fn a_review_repair_whose_background_work_was_killed_gets_a_resume() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}{}",
            on_ci_read(
                1,
                &someone_pushes_to_issue_7("late.txt", "late", "Late commit")
            )
        ),
    );
    scenario.agent_does_in_session(2, &leaves_running("cargo test"));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 3);
    assert!(
        calls[2]["prompt"]
            .as_str()
            .unwrap()
            .contains("was killed when your turn ended"),
        "prompt: {}",
        calls[2]["prompt"]
    );
    let logs = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/issue", "jsonl");
    assert!(
        logs.iter()
            .any(|log| log.ends_with("-repair-1-resume.jsonl")),
        "logs: {logs:?}"
    );
}

/// Bash that records `by`'s edit to issue 7's `kind`, its body or title, dated
/// after any Run's start.
fn edits_issue_7(kind: &str, by: &str) -> String {
    format!(
        "gh fake edits 7 '[{{\"kind\": \"{kind}\", \"by\": \"{by}\", \"at\": \"2999-01-01T00:00:00Z\"}}]'\n"
    )
}

#[test]
fn an_untrusted_edit_while_the_run_is_under_way_starts_no_repair() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}{}{}",
            checks_on_head(RED),
            edits_issue_7("body", "outsider")
        ),
    );

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(
            "#7 body edited since the Run started by an account that is not a Trusted account; starting no further session"
        ),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1, "no Repair session");
}

#[test]
fn an_untrusted_edit_while_the_run_is_under_way_starts_no_resume() {
    let scenario = Scenario::new();
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}{}{}",
            edits_issue_7("body", "outsider"),
            leaves_running("cargo test")
        ),
    );

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("starting no further session"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1, "no Resume session");
}

#[test]
fn an_untrusted_edit_dated_just_before_the_runs_local_start_still_stops_repairs() {
    let scenario = Scenario::new();
    // GitHub dates the edit before this machine's clock says the Run
    // started: the gap since the trust check, or clock lag.
    let at = (chrono::Utc::now() - chrono::TimeDelta::seconds(30)).format("%Y-%m-%dT%H:%M:%SZ");
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}{}gh fake edits 7 '[{{\"kind\": \"body\", \"by\": \"outsider\", \"at\": \"{at}\"}}]'\n",
            checks_on_head(RED),
        ),
    );
    let result = scenario.run(&["merge", &scenario.issue_url(7)]);
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("starting no further session"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1, "no Repair session");
}

/// Bash that has the next `times` merge attempts fail with `error`.
fn refuse_merges(times: usize, error: &str) -> String {
    format!("gh fake refuse-merges {times} '{error}'\n")
}

/// Bash that sets the check runs on the worktree's HEAD to `checks`.
fn checks_on_head(checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse HEAD)\" '{checks}'\n")
}
const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;

#[test]
fn a_merge_refused_with_nothing_left_to_fix_leaves_the_pr_ready_for_review() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        refuse_merges(1, "Merge commits are not allowed on this repository."),
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert!(
        result
            .stderr
            .contains("Merge commits are not allowed on this repository."),
        "stderr: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("is merged"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    let pr = &scenario.gh_state()["prs"][0];
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], false);
    // No failure commit: the PR stays on the head whose CI was watched.
    let head = scenario
        .origin_git(&["rev-parse", "refs/heads/issue-7"])
        .trim()
        .to_string();
    assert_eq!(scenario.gh_calls_of("pr", "merge")[0].last(), Some(&head));
    assert_eq!(scenario.origin_log("main").unwrap(), vec!["Initial commit"]);
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_merge_run_ends_with_the_issue_closed_and_its_claim_removed() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}echo 'Bug reproduced: fixed' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n"
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
    assert_eq!(scenario.issue_labels(7), ["bug"]);
    assert!(
        result
            .stderr
            .contains("thirdshift: removing in-progress from issue #7\n"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("warning"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn an_issue_github_closed_after_the_merge_has_its_claim_removed_too() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake after-merge 'gh fake issue 7 CLOSED'\n"
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(scenario.gh_calls_of("issue", "close").is_empty());
    assert_eq!(scenario.issue_labels(7), Vec::<String>::new());
}

#[test]
fn a_required_review_refuses_the_merge_without_another_round_and_names_the_outcome() {
    for decision in ["REVIEW_REQUIRED", "CHANGES_REQUESTED"] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["ready-for-agent"]);
        scenario.agent_does(&format!(
            "{AGENT_OPENS_PR}gh fake pr issue-7 reviewDecision '\"{decision}\"'\n{}",
            checks_on_head(r#"[{"name":"test","conclusion":"success"}]"#),
        ));
        let resend = ResendStandIn::replying(200, r#"{"id":"1"}"#);

        let result = scenario.run_with_env(
            &[
                "merge",
                &scenario.issue_url(7),
                "email",
                "owner@example.com",
            ],
            &[
                ("THIRDSHIFT_RESEND_URL", resend.url()),
                ("RESEND_API_KEY", "test-key"),
            ],
        );

        assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
        assert_eq!(result.stdout, format!("{PR_URL}\n"));
        let reason = format!(
            "PR {PR_URL} needs an approving review before it can merge; it is ready for review"
        );
        assert!(result.stderr.contains(&reason), "{}", result.stderr);
        assert!(scenario.gh_calls_of("pr", "merge").is_empty());
        assert_eq!(
            result.stderr.matches("waiting up to 0s for CI on").count(),
            1,
            "{}",
            result.stderr
        );
        assert_eq!(scenario.claude_calls().len(), 1);
        let gh = scenario.gh_state();
        assert_eq!(gh["prs"][0]["state"], "OPEN");
        assert_eq!(gh["prs"][0]["isDraft"], false);
        assert_eq!(gh["issues"]["7"], "OPEN");
        assert_eq!(scenario.issue_labels(7), ["in-progress"]);
        assert_eq!(
            scenario.origin_log("issue-7").unwrap(),
            ["Add feature", "Initial commit"]
        );
        scenario.assert_cleaned_up("issue-7");
        let requests = resend.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].body["subject"]
                .as_str()
                .unwrap()
                .contains("review required"),
            "{:?}",
            requests[0].body
        );
        assert!(
            requests[0].body["text"].as_str().unwrap().contains(&reason),
            "{:?}",
            requests[0].body
        );
        assert!(
            requests[0].body["text"]
                .as_str()
                .unwrap()
                .contains("Result:       review required")
        );
    }
}

#[test]
fn unblocked_or_unknown_review_decisions_allow_the_guarded_merge() {
    for decision in [r#""""#, r#""APPROVED""#, r#""FUTURE_VALUE""#, "null"] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "{AGENT_OPENS_PR}gh fake pr issue-7 reviewDecision '{decision}'\n"
        ));

        let result = scenario.run(&["merge", &scenario.issue_url(7)]);

        assert_eq!(result.code, Some(0), "{decision}: {}", result.stderr);
        assert_issue_7_merged_into(&scenario, "main");
        assert!(scenario.gh_calls().iter().any(|call| call
            == &[
                "pr",
                "view",
                "1",
                "--repo",
                "acme/widgets",
                "--json",
                "reviewDecision"
            ]));
    }
}

#[test]
fn an_unreadable_review_decision_keeps_existing_merge_and_policy_refusal_behavior() {
    for refused in [false, true] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "{AGENT_OPENS_PR}gh fake pr issue-7 reviewDecision '\"REVIEW_REQUIRED\"'\n\
             gh fake fails 'pr view 1 --repo acme/widgets --json reviewDecision'\n{}",
            if refused {
                refuse_merges(1, "Merge commits are not allowed.")
            } else {
                String::new()
            },
        ));

        let result = scenario.run(&["merge", &scenario.issue_url(7)]);

        assert_eq!(
            result.code,
            Some(if refused { 1 } else { 0 }),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.gh_calls_of("pr", "merge").len(), 1);
        assert!(!result.stderr.contains("needs an approving review"));
        if refused {
            assert!(
                result.stderr.contains("Merge commits are not allowed."),
                "{}",
                result.stderr
            );
            assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
        } else {
            assert_issue_7_merged_into(&scenario, "main");
        }
    }
}

#[test]
fn the_review_decision_is_read_again_after_a_repair_before_retrying_the_merge() {
    let scenario = Scenario::new();
    let hook = format!(
        "{}\ngh fake pr issue-7 reviewDecision '\"REVIEW_REQUIRED\"'\n",
        someone_pushes_to_issue_7("late.txt", "late", "Late commit"),
    );
    scenario.agent_does_in_session(
        1,
        &format!(
            "{AGENT_OPENS_PR}gh fake on-merge 1 '{}'\n{}",
            hook.replace('\'', r"'\''"),
            refuse_merges(1, "Head branch was modified."),
        ),
    );
    scenario.agent_does_in_session(2, "");

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert!(
        result.stderr.contains("needs an approving review"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.gh_calls_of("pr", "merge").len(), 1);
    assert_eq!(scenario.claude_calls().len(), 2);
    assert_eq!(
        result.stderr.matches("waiting up to 0s for CI on").count(),
        2,
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario
            .gh_calls()
            .iter()
            .filter(|call| call.last().is_some_and(|arg| arg == "reviewDecision"))
            .count(),
        2
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    assert_eq!(scenario.origin_log("main").unwrap(), ["Initial commit"]);
    assert_eq!(
        scenario.origin_file("issue-7", "late.txt").as_deref(),
        Some("late\n")
    );
}
