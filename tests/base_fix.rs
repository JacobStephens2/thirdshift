//! The Base fix: with `base-fix`, a Run whose only red checks are Inherited
//! failures writes an issue for them, starts a Merge run into its Base branch
//! on that issue, waits for it, then merges the Base branch in and watches CI
//! again. One Base fix per Run; if it fails, or the checks are still
//! Inherited failures once it has merged, the Run fails naming its issue.

mod support;

use support::Scenario;
use support::resend::ResendStandIn;

/// The agent adds feature.txt and opens the PR for #7, with `test` red on its
/// head and on `main`'s tip.
const RUN_OPENS_PR_WITH_INHERITED_FAILURE: &str = r#"
echo "feature" > feature.txt
git add feature.txt
git commit -q -m "Add feature"
gh pr create --base main --head issue-7 --title "Add feature" --body "Closes #7"
gh fake checks "$(git rev-parse HEAD)" '[{"name": "test", "conclusion": "failure", "url": "https://ci.example/test"}]'
gh fake checks "$(git rev-parse origin/main)" '[{"name": "test", "conclusion": "failure", "url": "https://ci.example/main/test"}]'
"#;

/// The agent, on the Base fix issue #8, commits a fix and opens its PR into
/// `main`.
const BASE_FIX_OPENS_PR: &str = r#"
echo "fixed" > ci-fix.txt
git add ci-fix.txt
git commit -q -m "Fix CI on main"
gh pr create --base main --head issue-8 --title "Fix CI on main" --body "Closes #8"
"#;

const GREEN_ON_HEAD: &str = "gh fake checks \"$(git rev-parse HEAD)\" '[{\"name\": \"test\", \"conclusion\": \"success\"}]'\n";

const RED_ON_HEAD: &str = "gh fake checks \"$(git rev-parse HEAD)\" '[{\"name\": \"test\", \"conclusion\": \"failure\"}]'\n";

/// Resend's reply to an email it accepted.
const ACCEPTED: &str = r#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#;

const BASE_FIX_URL: &str = "https://github.com/acme/widgets/issues/8";

/// The prompt of each session, in order.
fn prompts(scenario: &Scenario) -> Vec<String> {
    scenario
        .claude_calls()
        .iter()
        .map(|call| call["prompt"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn with_base_fix_an_inherited_failure_gets_an_issue_a_merged_fix_and_the_run_finishes_green() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for(8, &format!("{BASE_FIX_OPENS_PR}{GREEN_ON_HEAD}"));
    let red_base = scenario.origin_git(&["rev-parse", "main"]);

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let gh = scenario.gh_state();
    assert_eq!(gh["titles"]["8"], "CI red on main: test");
    assert_eq!(
        gh["bodies"]["8"],
        format!(
            "CI is red on `main` at {}: these checks fail there, so every pull request into `main` inherits them.\n\
             \n\
             - test: https://ci.example/main/test\n\
             \n\
             Found by the thirdshift Run on #7, whose pull request is https://github.com/acme/widgets/pull/1.\n\
             \n\
             Fix the checks on `main`. Do not skip, disable, or weaken tests or checks to make them pass.\n",
            &red_base[..7]
        )
    );
    assert_eq!(
        gh["labels"]["8"],
        serde_json::json!(["base-fix", "ready-for-agent"])
    );
    for line in [
        format!("thirdshift: starting Base fix #8 into main: {BASE_FIX_URL}\n"),
        "thirdshift: waiting on Base fix #8\n".to_string(),
        "thirdshift: Base fix #8 merged; merging main in again\n".to_string(),
    ] {
        assert!(result.stderr.contains(&line), "stderr: {}", result.stderr);
    }
    // The Base fix was a Merge run into main, whose Self-merge closed its issue.
    assert_eq!(gh["prs"][1]["base"], "main");
    assert_eq!(gh["prs"][1]["state"], "MERGED");
    assert_eq!(gh["issues"]["8"], "CLOSED");
    // The Run merged the fixed main in, and left its PR ready for review.
    assert_eq!(
        scenario.origin_file("issue-7", "ci-fix.txt").as_deref(),
        Some("fixed\n")
    );
    assert_eq!(gh["prs"][0]["state"], "OPEN");
    assert_eq!(gh["prs"][0]["isDraft"], false);
    let prompts = prompts(&scenario);
    assert_eq!(prompts.len(), 2, "one implement session each: {prompts:?}");
    assert!(prompts[1].contains(BASE_FIX_URL), "prompt: {}", prompts[1]);
    scenario.assert_cleaned_up("issue-7");
    scenario.assert_cleaned_up("issue-8");
}

#[test]
fn the_base_fix_repairs_the_check_as_its_own_and_only_the_run_sends_a_notification() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    // On the Base fix's head, test is as red as on main, which it branched
    // off: no Inherited failure to the Base fix, whose Repair turns it green.
    scenario.agent_does_for_in_session(8, 1, &format!("{BASE_FIX_OPENS_PR}{RED_ON_HEAD}"));
    scenario.agent_does_for_in_session(
        8,
        2,
        &format!(
            "echo more > more.txt\ngit add more.txt\ngit commit -q -m 'Fix more'\n{GREEN_ON_HEAD}"
        ),
    );
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = scenario.run_with_env(
        &[
            &scenario.issue_url(7),
            "--base-fix",
            "--email",
            "me@example.com",
        ],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
    );

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let prompts = prompts(&scenario);
    assert_eq!(prompts.len(), 3, "prompts: {prompts:?}");
    assert!(
        prompts[2].starts_with("CI failed on pull request https://github.com/acme/widgets/pull/2")
            && prompts[2].contains("- test")
            && !prompts[2].contains("Also failing on"),
        "prompt: {}",
        prompts[2]
    );
    assert!(
        !result.stderr.contains("#8: Inherited failures"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.gh_state()["prs"][1]["state"], "MERGED");
    assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
    // One Run notification: the Run's, which reports the Base fix.
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(
        requests[0].body["subject"],
        "[thirdshift] acme/widgets#7 Issue 7: ready for review"
    );
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(
        text.contains(&format!("Base fix:     {BASE_FIX_URL} merged\n")),
        "text: {text}"
    );
}

#[test]
fn with_base_fix_in_the_user_config_the_base_fix_still_starts_none_of_its_own() {
    let scenario = Scenario::new();
    scenario.user_config_is("[base]\nfix = true\n");
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    // On the Base fix's head, test is as red as on main, which it branched
    // off: its own to fix, in a Repair.
    scenario.agent_does_for_in_session(8, 1, &format!("{BASE_FIX_OPENS_PR}{RED_ON_HEAD}"));
    scenario.agent_does_for_in_session(
        8,
        2,
        &format!(
            "echo more > more.txt\ngit add more.txt\ngit commit -q -m 'Fix more'\n{GREEN_ON_HEAD}"
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let prompts = prompts(&scenario);
    assert_eq!(prompts.len(), 3, "prompts: {prompts:?}");
    assert!(
        prompts[2].starts_with("CI failed on pull request https://github.com/acme/widgets/pull/2"),
        "prompt: {}",
        prompts[2]
    );
    assert_eq!(scenario.gh_state()["prs"][1]["state"], "MERGED");
    assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
}

#[test]
fn a_merge_run_self_merges_once_its_base_fix_has_merged() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for(8, &format!("{BASE_FIX_OPENS_PR}{GREEN_ON_HEAD}"));

    let result = scenario.run(&["merge", "base-fix", &scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let gh = scenario.gh_state();
    assert_eq!(gh["prs"][0]["state"], "MERGED");
    assert_eq!(gh["prs"][1]["state"], "MERGED");
    assert_eq!(gh["issues"]["7"], "CLOSED");
    for file in ["feature.txt", "ci-fix.txt"] {
        assert!(scenario.origin_file("main", file).is_some(), "{file}");
    }
}

#[test]
fn the_base_fix_merges_into_the_runs_base_branch_whatever_the_launch_directory_has_checked_out() {
    let scenario = Scenario::new();
    // A Continuation whose open PR targets develop, started from main.
    scenario.origin_has_branch("develop", "main", &[]);
    scenario.origin_has_branch("issue-7", "develop", &["Add feature"]);
    scenario.github_has_pr("issue-7", "develop", "OPEN");
    scenario.agent_does(
        r#"
gh fake checks "$(git rev-parse HEAD)" '[{"name": "test", "conclusion": "failure"}]'
gh fake checks "$(git rev-parse origin/develop)" '[{"name": "test", "conclusion": "failure"}]'
"#,
    );
    scenario.agent_does_for(
        8,
        &format!(
            "{}{GREEN_ON_HEAD}",
            BASE_FIX_OPENS_PR.replace("--base main", "--base develop")
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let gh = scenario.gh_state();
    assert_eq!(gh["titles"]["8"], "CI red on develop: test");
    assert_eq!(gh["prs"][1]["base"], "develop");
    assert_eq!(gh["prs"][1]["state"], "MERGED");
    assert!(scenario.origin_file("develop", "ci-fix.txt").is_some());
    assert!(scenario.origin_file("main", "ci-fix.txt").is_none());
    assert!(scenario.origin_file("issue-7", "ci-fix.txt").is_some());
}

#[test]
fn a_failing_base_fix_fails_the_run_naming_its_issue() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for(8, "exit 3\n");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = scenario.run_with_env(
        &[
            &scenario.issue_url(7),
            "base-fix",
            "email",
            "me@example.com",
        ],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    let cause = format!("Base fix {BASE_FIX_URL} failed: ");
    assert!(
        result
            .stderr
            .lines()
            .any(|line| line.starts_with(&format!("thirdshift: {cause}"))),
        "stderr: {}",
        result.stderr
    );
    let gh = scenario.gh_state();
    assert_eq!(gh["issues"]["8"], "OPEN");
    assert_eq!(gh["prs"][0]["isDraft"], true);
    assert!(
        scenario.origin_log("issue-7").unwrap()[0]
            .starts_with(&format!("thirdshift: failed run ({cause}")),
        "log: {:?}",
        scenario.origin_log("issue-7")
    );
    scenario.assert_cleaned_up("issue-7");
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(
        text.contains(&cause)
            && text.contains(&format!("Base fix:     {BASE_FIX_URL} not merged\n")),
        "text: {text}"
    );
}

#[test]
fn checks_still_inherited_failures_after_the_base_fix_merged_fail_the_run_with_no_second_base_fix()
{
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for(8, &format!("{BASE_FIX_OPENS_PR}{GREEN_ON_HEAD}"));
    // The Base fix merges, but test is red on main's merge commit all the
    // same, and on the head of the Run that merges it in.
    const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;
    let mut gh = scenario.gh_state();
    gh["after_merge"] = serde_json::json!(format!(
        "gh fake checks \"$FAKE_MERGE_SHA\" '{RED}'\n\
         gh fake on-ci-read 2 'gh fake checks \"$FAKE_CI_SHA\" '\\''{RED}'\\'''\n"
    ));
    scenario.write_gh_state(&gh);

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let fixed_base = scenario.origin_git(&["rev-parse", "main"]);
    let cause = format!(
        "CI red on test, which also fails on main at {}, even after Base fix {BASE_FIX_URL} merged; fix main first",
        &fixed_base[..7]
    );
    assert!(result.stderr.contains(&cause), "stderr: {}", result.stderr);
    let gh = scenario.gh_state();
    assert_eq!(gh["prs"][1]["state"], "MERGED");
    assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
    assert_eq!(gh["issues"].as_object().unwrap().len(), 2, "#7 and #8");
    assert_eq!(gh["prs"][0]["isDraft"], true);
    assert_eq!(prompts(&scenario).len(), 2, "no Repair");
}

#[test]
fn without_base_fix_an_inherited_failure_fails_the_run_and_writes_no_issue() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    let red_base = scenario.origin_git(&["rev-parse", "main"]);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(&format!(
            "CI red on test, which also fails on main at {}; fix main first",
            &red_base[..7]
        )),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.gh_calls_of("issue", "create").is_empty());
    assert!(scenario.gh_calls_of("label", "list").is_empty());
    assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
}

#[test]
fn a_label_the_repository_lacks_is_added_and_one_it_has_is_left_alone() {
    let scenario = Scenario::new();
    let mut gh = scenario.gh_state();
    gh["repo_labels"] = serde_json::json!(["bug", "Ready-For-Agent"]);
    scenario.write_gh_state(&gh);
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for(8, &format!("{BASE_FIX_OPENS_PR}{GREEN_ON_HEAD}"));

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let created = scenario.gh_calls_of("label", "create");
    assert_eq!(created.len(), 1, "{created:?}");
    assert_eq!(created[0][2], "base-fix");
    assert_eq!(
        scenario.gh_state()["repo_labels"],
        serde_json::json!(["bug", "Ready-For-Agent", "base-fix"])
    );
}

#[test]
fn help_names_base_fix_and_not_the_base_fixs_hidden_argument() {
    let scenario = Scenario::new();

    let result = scenario.run(&["help"]);

    assert_eq!(result.code, Some(0));
    assert!(result.stdout.contains("base-fix"), "{}", result.stdout);
    assert!(
        !result.stdout.contains("base-fix-into"),
        "{}",
        result.stdout
    );
}
