//! The Merge run: `thirdshift merge <Issue URL>` does everything a Run does,
//! then the Self-merge: it merges the ready, mergeable, green PR with a merge
//! commit on exactly the head commit whose CI it watched, then deletes the
//! Issue branch on origin and closes the issue unless it is already closed.

mod support;

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
fn a_merge_into_the_default_branch_closes_the_issue_github_leaves_open() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
    assert_eq!(
        scenario.gh_state()["comments"]["7"],
        serde_json::json!(["Closed by #1, merged into main by a thirdshift Merge run."])
    );
    assert!(
        !result.stderr.contains("warning"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn an_issue_already_closed_after_the_merge_is_not_closed_again() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}gh fake after-merge 'gh fake issue 7 CLOSED'\n"
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["issues"]["7"], "CLOSED");
    assert!(scenario.gh_calls_of("issue", "close").is_empty());
    assert_eq!(scenario.gh_state()["comments"], serde_json::Value::Null);
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
            "issue-7",
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

    assert_eq!(merge_run.first_prompt(), run.first_prompt());
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
fn a_failed_branch_deletion_still_exits_as_merged_with_a_warning() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_OPENS_PR);
    scenario.repo_has_hook(
        &scenario.origin_dir(),
        "pre-receive",
        "#!/bin/sh\n\
         while read old new ref; do\n\
           [ \"$new\" = 0000000000000000000000000000000000000000 ] && echo 'deletion refused' && exit 1\n\
         done\n\
         exit 0\n",
    );

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert!(scenario.origin_log("issue-7").is_some());
    assert!(
        result.stderr.contains(
            "warning: could not delete issue-7 on origin, so delete it by hand: git push origin --delete issue-7"
        ),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("deletion refused"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is merged").as_str())
    );
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_failed_issue_close_still_exits_as_merged_with_a_warning() {
    let scenario = Scenario::new();
    merge_into_develop(&scenario, "gh fake fails 'issue close'\n");

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
    assert_eq!(scenario.origin_log("issue-7"), None);
    assert!(
        result.stderr.contains(&format!(
            "warning: could not close issue #7, so if it is still open, close it by hand: \
             gh issue close 7 --repo acme/widgets --comment '{CLOSING_COMMENT}'"
        )),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {PR_URL} is merged").as_str())
    );
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
    scenario.origin_git(&["rev-parse", &format!("{commit}^")])
}

/// Assert that `prompt` is a review Repair's, reviewing from `fixed_point`.
fn assert_review_repair_from(prompt: &serde_json::Value, fixed_point: &str) {
    let prompt = prompt.as_str().unwrap();
    let fixed_point = fixed_point.trim();
    assert!(
        prompt.starts_with("/thirdshift:code-review"),
        "prompt: {prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "Review with /thirdshift:code-review using {fixed_point} as the fixed point"
        )),
        "prompt: {prompt}"
    );
    assert!(prompt.contains("Unaddressed findings"), "prompt: {prompt}");
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
        conflict.starts_with("/thirdshift:resolving-merge-conflicts")
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
    let logs = scenario.entries("home/.thirdshift/logs/acme/widgets/sessions");
    assert!(
        logs.iter()
            .any(|log| log.ends_with("-repair-1-resume.jsonl")),
        "logs: {logs:?}"
    );
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
    scenario.agent_does(AGENT_OPENS_PR);

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
