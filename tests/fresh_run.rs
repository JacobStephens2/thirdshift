//! A fresh run on the happy path: no Issue branch exists yet, the agent
//! implements the issue and opens a PR, and thirdshift pushes, checks the PR,
//! prints its URL and cleans up.

mod support;

use support::Scenario;

/// The agent commits its work and opens a PR, but leaves the pushing to
/// thirdshift.
const AGENT_COMMITS_AND_OPENS_PR: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
"#;

#[test]
fn prints_only_the_pr_url_and_exits_zero() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
}

#[test]
fn a_rejecting_pre_push_hook_in_the_target_repo_does_not_block_the_push() {
    let scenario = Scenario::new();
    scenario.launch_has_rejecting_pre_push_hook();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        "Add feature".to_string()
    );
}

#[test]
fn runs_claude_headless_in_auto_mode_in_a_sibling_worktree_on_the_issue_branch() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    let argv: Vec<&str> = call["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert!(argv.contains(&"-p"), "argv: {argv:?}");
    assert!(
        argv.windows(2).any(|w| w == ["--permission-mode", "auto"]),
        "argv: {argv:?}"
    );
    assert!(
        argv.windows(2)
            .any(|w| w == ["--output-format", "stream-json"]),
        "argv: {argv:?}"
    );
    assert!(argv.contains(&"--verbose"), "argv: {argv:?}");
    assert!(!argv.contains(&"--plugin-dir"), "argv: {argv:?}");
    assert_eq!(
        call["cwd"],
        scenario.path("work/widgets-issue-7").to_str().unwrap()
    );
    assert_eq!(call["branch"], "issue-7");
}

#[test]
fn gives_the_agent_the_fresh_prompt() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(
        scenario.claude_calls()[0]["prompt"],
        "/thirdshift-implement https://github.com/acme/widgets/issues/7\n\
         The base branch is main. Review with the `thirdshift-code-review` skill using main as the fixed point.\n\
         For each Standards or Spec finding that says behaviour is wrong, run its test or command as written before deciding the finding. \
         If it fails as the finding says, fix the code and keep the test. \
         If it passes, you may decline the finding, citing the run. \
         A passing run counts only when it exercises that finding; a green suite does not. \
         Address the Standards and Spec findings you agree with.\n\
         Push branch issue-7 and create a pull request against main using the `thirdshift-pr` skill, marked ready for review.\n\
         In the PR body, add an \"Unaddressed findings\" section listing each skipped finding under Standards or Spec, \
         with the line of code, plan decision, ADR or run that refutes each finding. \
         If you decline a finding because a Spec or the Day shift must decide, \
         file a `needs-triage` issue unless an open issue already covers it, and link the issue from the entry. \
         The issue must say what the finding is, its evidence, which pull request raised it, and why the call is not yours to make. \
         Give a new issue only the `needs-triage` label so it pauses nothing. \
         Name in the pull request body any changed file the review left unread.\n\
         Include \"Closes #7\" in the PR body.\n\
         You run headless: nobody is watching, and ending your turn ends the session. Run tests and other long commands in the foreground, raising the command's timeout if needed. If a command is moved to the background, wait for that task by its own task id or output file, never by process names or patterns (`pgrep`, `ps | grep`, and the like): other sessions on this machine run the same commands. Never end your turn while a background task you depend on is still running: ending the turn kills it. Before ending your turn, stop every background task you no longer need, by its task id (with the `TaskStop` tool, if you have it): a task still running when your turn ends is taken as work you were waiting on.\n"
    );
}

#[test]
fn finds_every_factory_skill_in_the_worktree_as_thirdshift_skill() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    scenario.assert_every_session_found_the_factory_skills();
    let files = &scenario.claude_calls()[0]["skill_files"];
    for supporting in [
        "thirdshift-tdd/tests.md",
        "thirdshift-improve-codebase-architecture/REPORT.md",
        "thirdshift-codebase-design/DEEPENING.md",
        "thirdshift-codebase-design/DESIGN-IT-TWICE.md",
    ] {
        assert!(
            files[supporting].is_string(),
            "the supporting file {supporting} is missing from the worktree's .claude/skills/"
        );
    }
}

#[test]
fn ships_the_mattpocock_skills_mit_notice_with_the_factory_skills() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    let calls = scenario.claude_calls();
    let license = calls[0]["beside_skills"]["LICENSE"].as_str();
    assert!(
        license
            .is_some_and(|text| text.contains("MIT License")
                && text.contains("Copyright (c) 2026 Matt Pocock")),
        "license: {license:?}"
    );
}

#[test]
fn the_linked_skills_are_kept_out_of_git() {
    let scenario = Scenario::new();
    let status = scenario.path("status.txt");
    scenario.agent_does(&format!(
        "git status --porcelain --untracked-files=all > '{}'\n\
         echo feature > feature.txt\n\
         git add -A\n\
         git commit -q -m 'Add everything'\n\
         gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'\n",
        status.display()
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(std::fs::read_to_string(&status).unwrap(), "");
    let committed = scenario.origin_git(&["ls-tree", "-r", "--name-only", "issue-7"]);
    assert!(committed.contains("feature.txt"), "committed: {committed}");
    assert!(!committed.contains(".claude"), "committed: {committed}");
}

#[test]
fn a_second_run_adds_no_second_exclude_entry() {
    let scenario = Scenario::new();
    scenario.issue_is(8, "OPEN");
    scenario.agent_does_for(7, AGENT_COMMITS_AND_OPENS_PR);
    scenario.agent_does_for(
        8,
        "echo other > other.txt\ngit add other.txt\ngit commit -q -m 'Add other'\n\
         gh pr create --base main --head issue-8 --title 'Add other' --body 'Closes #8'\n",
    );

    let first = scenario.run(&[&scenario.issue_url(7)]);
    let second = scenario.run(&[&scenario.issue_url(8)]);

    assert_eq!(first.code, Some(0), "stderr: {}", first.stderr);
    assert_eq!(second.code, Some(0), "stderr: {}", second.stderr);
    let exclude = std::fs::read_to_string(scenario.launch_dir().join(".git/info/exclude")).unwrap();
    let entries: Vec<_> = exclude
        .lines()
        .filter(|line| line.contains("thirdshift-"))
        .collect();
    assert_eq!(
        entries,
        ["/.claude/skills/thirdshift-*"],
        "exclude: {exclude}"
    );
}

#[test]
fn logs_the_session_stream_under_the_home_directory() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    let logs = scenario.entries("home/.thirdshift/logs/acme/widgets/sessions");
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    let name = &logs[0];
    assert!(
        name.starts_with("7-") && name.ends_with("-implement.jsonl"),
        "log: {name}"
    );
    let log = std::fs::read_to_string(
        scenario
            .path("home/.thirdshift/logs/acme/widgets/sessions")
            .join(name),
    )
    .unwrap();
    assert!(
        log.contains(r#""type": "system""#) && log.contains(r#""type": "result""#),
        "log: {log}"
    );
}

#[test]
fn pushes_the_issue_branch_when_the_agent_did_not() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Add feature".to_string(),
            "Initial commit".to_string()
        ])
    );
}

#[test]
fn succeeds_when_the_agent_pushed_the_branch_itself() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_COMMITS_AND_OPENS_PR}\ngit push -q origin issue-7\n"
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.origin_log("issue-7").map(|log| log.len()), Some(2));
}

#[test]
fn leaves_no_worktree_local_issue_branch_or_temp_directory_behind() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn marks_a_draft_pr_ready_and_succeeds() {
    let scenario = Scenario::new();
    scenario.agent_does(
        "echo feature > feature.txt\ngit add feature.txt\ngit commit -q -m 'Add feature'\n\
         gh pr create --draft --base main --head issue-7 --title 'Add feature' --body 'Closes #7'\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
}

#[test]
fn another_issues_branch_and_pr_do_not_count_as_this_issues() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("issue-70", "main", &["Other work"]);
    scenario.github_has_pr("issue-70", "main", "MERGED");
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.claude_calls()[0]["branch"], "issue-7");
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Add feature".to_string(),
            "Initial commit".to_string()
        ])
    );
}

/// Every request the Run made to change an issue's labels.
fn label_requests(scenario: &Scenario) -> Vec<Vec<String>> {
    let mut requests = scenario.gh_calls_of("api", "--method");
    requests.retain(|request| request.iter().any(|arg| arg.ends_with("/labels")));
    requests
}

#[test]
fn a_run_that_ends_with_its_pr_ready_for_review_keeps_its_claim() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result.stderr.ends_with(" is ready for review\n"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(7), ["in-progress"]);
    assert_eq!(label_requests(&scenario).len(), 1);
}

#[test]
fn the_claim_is_made_in_one_request_before_the_worktree_is_created() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let claimed = result
        .stderr
        .find("thirdshift: labelling #7 in-progress, in place of ready-for-agent\n");
    let worktree = result.stderr.find("thirdshift: creating worktree ");
    assert!(
        claimed.is_some() && claimed < worktree,
        "stderr: {}",
        result.stderr
    );
    let requests = label_requests(&scenario);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        requests[0][2..],
        [
            "PUT",
            "repos/acme/widgets/issues/7/labels",
            "--silent",
            "-f",
            "labels[]=in-progress"
        ]
    );
}

#[test]
fn the_in_progress_label_is_created_when_the_repository_lacks_it() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["bug", "ready-for-agent"]);
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(
        scenario.repo_labels(),
        ["bug", "ready-for-agent", "in-progress"]
    );
    let created = scenario.gh_calls_of("label", "create");
    assert_eq!(created.len(), 1, "{created:?}");
    assert_eq!(created[0][2], "in-progress");
}

#[test]
fn a_repository_that_has_the_in_progress_label_keeps_it_as_it_is() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["In-Progress"]);
    scenario.agent_does(AGENT_COMMITS_AND_OPENS_PR);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.repo_labels(), ["In-Progress"]);
    assert!(scenario.gh_calls_of("label", "create").is_empty());
    assert_eq!(scenario.issue_labels(7), ["in-progress"]);
}

#[test]
fn a_run_whose_claim_cannot_be_made_stops_before_any_work_naming_the_cause() {
    for (failing, cause) in [
        ("label list", "gh label list failed: HTTP 502"),
        ("label create", "gh label create in-progress "),
        (
            "api --method",
            "gh api --method PUT repos/acme/widgets/issues/7/labels --silent \
             -f labels[]=in-progress failed: HTTP 502",
        ),
    ] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["ready-for-agent"]);
        scenario.gh_fails(failing);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        scenario.assert_rejected_before_any_work(
            &result,
            &format!("thirdshift: could not make the Claim on #7: {cause}"),
        );
        assert_eq!(result.code, Some(1), "{failing}");
        assert_eq!(scenario.issue_labels(7), ["ready-for-agent"], "{failing}");
        assert!(scenario.origin_log("issue-7").is_none(), "{failing}");
    }
}
