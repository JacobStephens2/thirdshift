//! The Base fix: with `base-fix`, a Run whose only red checks are Inherited
//! failures writes an issue for them, starts a Merge run into its Base branch
//! on that issue, waits for it, then merges the Base branch in and watches CI
//! again. One Base fix per Run; if it fails, or the checks are still
//! Inherited failures once it has merged, the Run fails naming its issue.
//! A Run that finds an open Base fix issue for the same Base branch and
//! checks waits on that one instead, as its one Base fix.

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

#[cfg(target_os = "linux")]
#[test]
fn a_run_whose_thirdshift_was_removed_after_it_started_still_starts_its_base_fix() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{RUN_OPENS_PR_WITH_INHERITED_FAILURE}{}",
        scenario.waits_to_be_replaced()
    ));
    scenario.agent_does_for(8, &format!("{BASE_FIX_OPENS_PR}{GREEN_ON_HEAD}"));

    let result = scenario
        .run_copy_replaced_midway(&[&scenario.issue_url(7), "base-fix"], |installed| {
            std::fs::remove_file(installed).unwrap()
        });

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let gh = scenario.gh_state();
    assert_eq!(gh["prs"][1]["base"], "main");
    assert_eq!(gh["prs"][1]["state"], "MERGED");
    assert_eq!(
        scenario.origin_file("issue-7", "ci-fix.txt").as_deref(),
        Some("fixed\n")
    );
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
    assert_no_advice(&result.stderr, &ADVICE);
    assert_no_advice(text, &ADVICE);
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
    // Its `Base fix:` line says what happened: nothing to link or offer.
    assert!(
        result
            .stderr
            .contains(&format!("thirdshift: Base fix: {BASE_FIX_URL} merged\n")),
        "stderr: {}",
        result.stderr
    );
    assert_no_advice(&result.stderr, &ADVICE);
    let gh = scenario.gh_state();
    assert_eq!(gh["prs"][1]["state"], "MERGED");
    assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
    assert_eq!(gh["issues"].as_object().unwrap().len(), 2, "#7 and #8");
    assert_eq!(gh["prs"][0]["isDraft"], true);
    assert_eq!(prompts(&scenario).len(), 2, "no Repair");
}

/// The Base fix issue #`number` titled `title`, open, as another Run wrote
/// it. `on_view` is bash that runs the first time thirdshift views it.
fn another_runs_base_fix_is_open(scenario: &Scenario, number: u32, title: &str, on_view: &str) {
    let number = number.to_string();
    let mut gh = scenario.gh_state();
    gh["issues"][&number] = serde_json::json!("OPEN");
    gh["titles"][&number] = serde_json::json!(title);
    gh["labels"][&number] = serde_json::json!(["base-fix", "ready-for-agent"]);
    gh["on_issue_view"][&number] = serde_json::json!(on_view);
    scenario.write_gh_state(&gh);
}

/// Bash that pushes a fix to `main` from another clone and closes the Base
/// fix issue #8, as the Self-merge of another Run's Base fix does.
const ANOTHER_RUNS_BASE_FIX_MERGES: &str = r#"
other="$(mktemp -d)"
git clone -q https://github.com/acme/widgets.git "$other"
echo "fixed" > "$other/ci-fix.txt"
git -C "$other" add ci-fix.txt
git -C "$other" commit -q -m "Fix CI on main"
git -C "$other" push -q origin main
rm -rf "$other"
gh fake issue 8 CLOSED
"#;

#[test]
fn a_run_that_finds_an_open_base_fix_issue_for_its_checks_waits_on_it_and_starts_none() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    another_runs_base_fix_is_open(
        &scenario,
        8,
        "CI red on main: test",
        ANOTHER_RUNS_BASE_FIX_MERGES,
    );

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    for line in [
        format!("thirdshift: waiting on Base fix #8, already open: {BASE_FIX_URL}\n"),
        "thirdshift: Base fix #8 closed; merging main in again\n".to_string(),
    ] {
        assert!(result.stderr.contains(&line), "stderr: {}", result.stderr);
    }
    assert!(scenario.gh_calls_of("issue", "create").is_empty());
    let gh = scenario.gh_state();
    assert_eq!(gh["issues"].as_object().unwrap().len(), 2, "#7 and #8");
    assert_eq!(prompts(&scenario).len(), 1, "no Base fix Run, no Repair");
    // The Run merged the fixed main in, and left its PR ready for review.
    assert_eq!(
        scenario.origin_file("issue-7", "ci-fix.txt").as_deref(),
        Some("fixed\n")
    );
    assert_eq!(gh["prs"][0]["state"], "OPEN");
    assert_eq!(gh["prs"][0]["isDraft"], false);
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_found_base_fix_issue_that_closes_with_the_checks_still_red_fails_the_run_naming_it() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    // It covers test among other checks, and closes with main as it was.
    another_runs_base_fix_is_open(
        &scenario,
        8,
        "CI red on main: lint, test",
        "gh fake issue 8 CLOSED\n",
    );
    let red_base = scenario.origin_git(&["rev-parse", "main"]);
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
    let cause = format!(
        "CI red on test, which also fails on main at {}, even after Base fix {BASE_FIX_URL} closed; fix main first",
        &red_base[..7]
    );
    assert!(
        result.stderr.contains(&format!("thirdshift: {cause}\n")),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.gh_calls_of("issue", "create").is_empty());
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    assert_eq!(prompts(&scenario).len(), 1, "no Base fix Run, no Repair");
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(
        text.contains(&format!(
            "Cause:        {cause}\n\
             Base fix:     {BASE_FIX_URL} closed\n"
        )),
        "text: {text}"
    );
    assert_no_advice(&result.stderr, &ADVICE);
    assert_no_advice(text, &ADVICE);
}

#[test]
fn a_check_whose_name_has_a_comma_is_found_among_the_checks_an_open_base_fix_issue_names() {
    let scenario = Scenario::new();
    scenario.agent_does(
        &RUN_OPENS_PR_WITH_INHERITED_FAILURE
            .replace(r#""name": "test""#, r#""name": "test (ubuntu, stable)""#),
    );
    another_runs_base_fix_is_open(
        &scenario,
        8,
        "CI red on main: lint, test (ubuntu, stable)",
        ANOTHER_RUNS_BASE_FIX_MERGES,
    );

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result
            .stderr
            .contains("thirdshift: waiting on Base fix #8, already open: "),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.gh_calls_of("issue", "create").is_empty());
}

#[test]
fn a_base_fix_that_failed_and_left_its_issue_open_is_started_again_by_the_next_run_to_find_it() {
    // Two Runs, as the second finding the first's Base fix is the behavior.
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for_in_session(8, 1, "exit 3\n");
    let first = scenario.run(&[&scenario.issue_url(7), "base-fix"]);
    assert_eq!(first.code, Some(1), "stderr: {}", first.stderr);
    assert_eq!(scenario.gh_state()["issues"]["8"], "OPEN");

    // The Continuation's head, the Failed run commit, is as red as main.
    scenario.agent_does_for_in_session(7, 2, RED_ON_HEAD);
    scenario.agent_does_for_in_session(8, 2, &format!("{BASE_FIX_OPENS_PR}{GREEN_ON_HEAD}"));
    let second = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(second.code, Some(0), "stderr: {}", second.stderr);
    assert!(
        second.stderr.contains(&format!(
            "thirdshift: Base fix #8 is open but no longer running; starting it again into main: {BASE_FIX_URL}\n"
        )),
        "stderr: {}",
        second.stderr
    );
    assert_eq!(scenario.gh_calls_of("issue", "create").len(), 1);
    let gh = scenario.gh_state();
    assert_eq!(gh["issues"]["8"], "CLOSED");
    assert_eq!(gh["issues"].as_object().unwrap().len(), 2, "#7 and #8");
    assert_eq!(gh["prs"][0]["isDraft"], false);
    assert!(scenario.origin_file("issue-7", "ci-fix.txt").is_some());
}

#[test]
fn an_open_base_fix_issue_for_another_base_branch_or_other_checks_is_not_waited_on() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    another_runs_base_fix_is_open(&scenario, 8, "CI red on develop: test", "true");
    another_runs_base_fix_is_open(&scenario, 9, "CI red on main: lint", "true");
    // The Run's own Base fix issue is the next after those.
    scenario.agent_does_for(
        10,
        &format!(
            "{}{GREEN_ON_HEAD}",
            BASE_FIX_OPENS_PR
                .replace("issue-8", "issue-10")
                .replace("#8", "#10")
        ),
    );

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(
            "thirdshift: starting Base fix #10 into main: https://github.com/acme/widgets/issues/10\n"
        ),
        "stderr: {}",
        result.stderr
    );
    let gh = scenario.gh_state();
    assert_eq!(gh["titles"]["10"], "CI red on main: test");
    assert_eq!(gh["issues"]["10"], "CLOSED");
    for other in ["8", "9"] {
        assert_eq!(gh["issues"][other], "OPEN", "#{other}");
    }
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
    // Only the Run's own Claim adds a label to the repository.
    assert_eq!(scenario.repo_labels(), ["in-progress"]);
    assert_eq!(scenario.gh_state()["issues"].as_object().unwrap().len(), 1);
}

#[test]
fn a_base_fix_changes_no_label_on_its_issue_while_the_run_claims_its_own() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for(8, &format!("{BASE_FIX_OPENS_PR}{GREEN_ON_HEAD}"));

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(scenario.issue_labels(8), ["base-fix", "ready-for-agent"]);
    assert_eq!(scenario.issue_labels(7), ["in-progress"]);
    assert!(!result.stderr.contains("labelling #8"), "{}", result.stderr);
}

/// The cause of a Run that failed on `test`, an Inherited failure from
/// `main` at `base_commit`, with no Base fix taken.
fn inherited_failure(base_commit: &str) -> String {
    format!(
        "CI red on test, which also fails on main at {}; fix main first",
        &base_commit[..7]
    )
}

/// Assert that `text`, a Run's stderr or its Run notification's body, has
/// none of `labels`, each as it starts a line of advice.
fn assert_no_advice(text: &str, labels: &[&str]) {
    for label in labels {
        assert!(!text.contains(label), "{label} in: {text}");
    }
}

/// Every label a line of advice starts with.
const ADVICE: [&str; 3] = ["Base check:", "Retry with:", "Or set:"];

/// What links `test` where it fails on `main`, after `Base check:`.
const BASE_CHECK: &str = "test: https://ci.example/main/test";

/// What follows `Or set:`, after the retry command.
const OR_SET: &str = "base.fix = true in ~/.thirdshift/config.toml, \
                      to allow a Base fix for every Run on this machine";

#[test]
fn a_run_not_asked_about_a_base_fix_links_the_base_branchs_failing_checks_and_offers_one() {
    let scenario = Scenario::new();
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    let cause = inherited_failure(&scenario.origin_git(&["rev-parse", "main"]));
    let resend = ResendStandIn::replying(200, ACCEPTED);
    let url = scenario.issue_url(7);

    let result = scenario.run_with_env(
        &["merge", &url, "--email", "me@example.com"],
        &[
            ("THIRDSHIFT_RESEND_URL", resend.url()),
            ("RESEND_API_KEY", "re_test_123"),
        ],
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let retry = format!("thirdshift {url} merge --email me@example.com base-fix");
    assert!(
        result.stderr.contains(&format!(
            "thirdshift: {cause}\n\
             thirdshift: Base check: {BASE_CHECK}\n\
             thirdshift: Retry with: {retry}\n\
             thirdshift: Or set: {OR_SET}\n\
             thirdshift: session log: "
        )),
        "stderr: {}",
        result.stderr
    );
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let text = requests[0].body["text"].as_str().unwrap();
    assert!(
        text.contains(&format!(
            "Cause:        {cause}\n\
             Base check:   {BASE_CHECK}\n\
             Retry with:   {retry}\n\
             Or set:       {OR_SET}\n\
             Session log:  "
        )),
        "text: {text}"
    );
    // The cause alone is the Failed-run commit's message.
    assert_eq!(
        scenario.origin_log("issue-7").unwrap()[0],
        format!("thirdshift: failed run ({cause})")
    );
    assert!(scenario.gh_calls_of("issue", "create").is_empty());
}

#[test]
fn with_no_base_fix_the_base_branchs_failing_checks_are_linked_and_no_base_fix_is_offered() {
    for flag in ["no-base-fix", "--no-base-fix"] {
        let scenario = Scenario::new();
        scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
        let cause = inherited_failure(&scenario.origin_git(&["rev-parse", "main"]));
        let resend = ResendStandIn::replying(200, ACCEPTED);

        let result = scenario.run_with_env(
            &[&scenario.issue_url(7), flag, "email", "me@example.com"],
            &[
                ("THIRDSHIFT_RESEND_URL", resend.url()),
                ("RESEND_API_KEY", "re_test_123"),
            ],
        );

        assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
        assert!(
            result.stderr.contains(&format!(
                "thirdshift: {cause}\n\
                 thirdshift: Base check: {BASE_CHECK}\n\
                 thirdshift: session log: "
            )),
            "stderr: {}",
            result.stderr
        );
        let requests = resend.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        let text = requests[0].body["text"].as_str().unwrap();
        assert!(
            text.contains(&format!(
                "Cause:        {cause}\n\
                 Base check:   {BASE_CHECK}\n\
                 Session log:  "
            )),
            "text: {text}"
        );
    }
}

#[test]
fn with_base_fix_in_the_user_config_and_no_base_fix_given_no_base_fix_is_offered() {
    let scenario = Scenario::new();
    scenario.user_config_is("[base]\nfix = true\n");
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);

    let result = scenario.run(&[&scenario.issue_url(7), "no-base-fix"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result
            .stderr
            .contains(&format!("thirdshift: Base check: {BASE_CHECK}\n")),
        "stderr: {}",
        result.stderr
    );
    assert_no_advice(&result.stderr, &["Retry with:", "Or set:"]);
}

#[test]
fn a_base_branch_check_with_no_url_is_named_alone() {
    let scenario = Scenario::new();
    scenario.agent_does(
        &RUN_OPENS_PR_WITH_INHERITED_FAILURE
            .replace(r#", "url": "https://ci.example/main/test""#, ""),
    );

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains("thirdshift: Base check: test\n"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_label_the_repository_lacks_is_added_and_one_it_has_is_left_alone() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["bug", "Ready-For-Agent", "in-progress"]);
    scenario.agent_does(RUN_OPENS_PR_WITH_INHERITED_FAILURE);
    scenario.agent_does_for(8, &format!("{BASE_FIX_OPENS_PR}{GREEN_ON_HEAD}"));

    let result = scenario.run(&[&scenario.issue_url(7), "base-fix"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let created = scenario.gh_calls_of("label", "create");
    assert_eq!(created.len(), 1, "{created:?}");
    assert_eq!(created[0][2], "base-fix");
    assert_eq!(
        scenario.repo_labels(),
        ["bug", "Ready-For-Agent", "in-progress", "base-fix"]
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
