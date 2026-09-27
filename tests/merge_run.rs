//! The Merge run: `thirdshift merge <Issue URL>` does everything a Run does,
//! then the Self-merge: it merges the ready, mergeable, green PR with a merge
//! commit on exactly the head commit whose CI it watched.

mod support;

use support::Scenario;

/// The agent commits its work and opens a PR that closes issue #7, into
/// `main`.
const AGENT_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

const PR_URL: &str = "https://github.com/acme/widgets/pull/1";

/// Every `gh pr merge` call thirdshift made.
fn merge_calls(scenario: &Scenario) -> Vec<Vec<String>> {
    scenario
        .gh_calls()
        .into_iter()
        .filter(|call| call.starts_with(&["pr".to_string(), "merge".to_string()]))
        .collect()
}

/// Assert that `base`'s tip on origin is a merge commit of issue-7's head,
/// and return that head.
fn assert_issue_7_merged_into(scenario: &Scenario, base: &str) -> String {
    let head = scenario
        .origin_git(&["rev-parse", "refs/heads/issue-7"])
        .trim()
        .to_string();
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
    scenario.assert_cleaned_up("issue-7");
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
        merge_calls(&scenario),
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
    assert!(merge_calls(&scenario).is_empty());
    let pr = &scenario.gh_state()["prs"][0];
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(pr["isDraft"], false);
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
}

#[test]
fn a_merge_into_a_branch_other_than_the_default_leaves_the_issue_open() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("develop", "main", &[]);
    scenario.launch_checks_out("develop");
    scenario.agent_does(&AGENT_OPENS_PR.replace("--base main", "--base develop"));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_issue_7_merged_into(&scenario, "develop");
    assert_eq!(scenario.origin_log("main").unwrap(), vec!["Initial commit"]);
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
}

/// While thirdshift waits for CI on the agent's head, someone else pushes a
/// commit to issue-7, so the head it watched is no longer the PR's head.
const SOMEONE_PUSHES_TO_THE_ISSUE_BRANCH_DURING_CI: &str = r#"
gh fake on-ci-read 1 '
other="$(mktemp -d)"
git clone -q -b issue-7 https://github.com/acme/widgets.git "$other"
echo late > "$other/late.txt"
git -C "$other" add -A
git -C "$other" commit -q -m "Late commit"
git -C "$other" push -q origin issue-7
rm -rf "$other"
'
"#;

#[test]
fn a_merge_refused_because_the_head_moved_is_a_failed_run() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{SOMEONE_PUSHES_TO_THE_ISSUE_BRANCH_DURING_CI}"
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("Head branch was modified"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        !result.stderr.contains("is merged"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "OPEN");
    assert_eq!(scenario.origin_log("main").unwrap(), vec!["Initial commit"]);
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
}

/// Bash that has someone else push `file` with `content` to main.
fn base_moves_on(file: &str, content: &str) -> String {
    format!(
        r#"
other="$(mktemp -d)"
git clone -q https://github.com/acme/widgets.git "$other"
echo "{content}" > "$other/{file}"
git -C "$other" add -A
git -C "$other" commit -q -m "Base moves on"
git -C "$other" push -q origin main
rm -rf "$other"
"#
    )
}

/// Bash that runs `script` before each of the next `times` merge attempts.
fn on_merge(times: usize, script: &str) -> String {
    format!(
        "gh fake on-merge {times} '{}'\n",
        script.replace('\'', r"'\''")
    )
}

/// Bash that has the next `times` merge attempts fail with `error`.
fn refuse_merges(times: usize, error: &str) -> String {
    format!("gh fake refuse-merges {times} '{error}'\n")
}

/// Bash that sets the check runs on the worktree's HEAD to `checks`.
fn checks_on_head(checks: &str) -> String {
    format!("gh fake checks \"$(git rev-parse HEAD)\" '{checks}'\n")
}

/// Bash that commits a fix, `fix-<n>.txt`, as a CI-fix Repair would.
fn commits_fix(n: usize) -> String {
    format!("echo fix > fix-{n}.txt\ngit add fix-{n}.txt\ngit commit -q -m \"Fix CI {n}\"\n")
}

const GREEN: &str = r#"[{"name": "test", "conclusion": "success"}]"#;
const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;

/// The conflict Repair keeps both sides of feature.txt and finishes the merge.
const REPAIR_RESOLVES_CONFLICT: &str = r#"
printf 'feature\nbase feature\n' > feature.txt
git add feature.txt
git commit -q --no-edit
"#;

#[test]
fn a_merge_that_fails_on_a_base_branch_conflict_gets_a_conflict_repair_then_merges() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        on_merge(1, &base_moves_on("feature.txt", "base feature"))
    ));
    scenario.agent_does_in_session(2, REPAIR_RESOLVES_CONFLICT);

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    assert!(
        result.stderr.contains("not mergeable"),
        "the merge error is not shown: {}",
        result.stderr
    );
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1]["merging"], true);
    assert_eq!(merge_calls(&scenario).len(), 2);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    assert_issue_7_merged_into(&scenario, "main");
    assert_eq!(
        scenario.origin_file("main", "feature.txt").unwrap(),
        "feature\nbase feature\n"
    );
}

#[test]
fn a_merge_that_fails_then_finds_red_ci_on_a_new_head_gets_a_ci_fix_repair_then_merges() {
    let scenario = Scenario::new();
    // The Base branch moves as the merge is tried, and CI on the head that
    // merges it in goes red.
    let at_merge = format!(
        "{}gh fake on-ci-read 1 'gh fake checks \"$FAKE_CI_SHA\" '\\''{RED}'\\'''\n",
        base_moves_on("other.txt", "other")
    );
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}{}",
        checks_on_head(GREEN),
        on_merge(1, &at_merge),
        refuse_merges(
            1,
            "Base branch was modified. Review and try the merge again."
        ),
    ));
    scenario.agent_does_in_session(2, &format!("{}{}", commits_fix(1), checks_on_head(GREEN)));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[1]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("CI failed on pull request"),
        "prompt: {}",
        calls[1]["prompt"]
    );
    let head = assert_issue_7_merged_into(&scenario, "main");
    assert_eq!(merge_calls(&scenario).len(), 2);
    assert_eq!(merge_calls(&scenario)[1].last(), Some(&head));
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "Fix CI 1",
        "the fix is not the merged head"
    );
}

#[test]
fn merge_failures_that_keep_moving_the_base_branch_spend_the_base_move_budget() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}",
        on_merge(6, &base_moves_on("other-$RANDOM$RANDOM.txt", "other")),
        refuse_merges(
            6,
            "Base branch was modified. Review and try the merge again."
        ),
    ));

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result
            .stderr
            .contains("origin/main kept moving: merged it again 5 times"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(merge_calls(&scenario).len(), 6);
    let pr = &scenario.gh_state()["prs"][0];
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(
        pr["isDraft"], true,
        "an exhausted budget is no policy refusal"
    );
    assert_eq!(result.stdout, format!("{PR_URL}\n"));
}

#[test]
fn a_repair_needed_after_a_failed_merge_counts_against_the_repair_cap() {
    let scenario = Scenario::new();
    // Five CI-fix Repairs before the first merge attempt leave none for the
    // conflict the merge then runs into.
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}{}",
        checks_on_head(RED),
        on_merge(1, &base_moves_on("feature.txt", "base feature")),
    ));
    for session in 2..=6 {
        let checks = if session == 6 { GREEN } else { RED };
        scenario.agent_does_in_session(
            session,
            &format!("{}{}", commits_fix(session - 1), checks_on_head(checks)),
        );
    }

    let result = scenario.run(&["merge", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("repairs exhausted: conflict"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 6);
    assert_eq!(merge_calls(&scenario).len(), 1);
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
}

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
    assert_eq!(scenario.origin_log("main").unwrap(), vec!["Initial commit"]);
    assert_eq!(scenario.gh_state()["issues"]["7"], "OPEN");
    scenario.assert_cleaned_up("issue-7");
}
