//! Unattended Fix runs through the CLI, real Git and fake services.

mod support;

use serde_json::json;
use support::Scenario;

#[test]
fn no_eligible_pull_request_skips_once_without_a_notification() {
    let scenario = Scenario::new();
    for _ in 0..2 {
        let result = scenario.run(&["fix"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            result.stderr.contains("no eligible pull request"),
            "{}",
            result.stderr
        );
        assert!(result.stdout.is_empty());
    }
    assert!(scenario.claude_calls().is_empty());
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands/fix")
            .exists()
    );
    let activity =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert_eq!(activity.lines().count(), 1, "{activity}");
    assert!(
        activity.contains("Fix run skipped: no eligible pull request"),
        "{activity}"
    );
}

#[test]
fn pass_repairs_the_lowest_numbered_red_head_and_skips_its_draft_on_the_next_pass() {
    let scenario = Scenario::new();
    for branch in ["healthy", "first", "later"] {
        scenario.origin_has_branch(branch, "main", &["Feature"]);
        scenario.github_has_pr(branch, "main", "OPEN");
    }
    let mut state = scenario.gh_state();
    for branch in ["first", "later"] {
        let head = scenario.origin_git(&["rev-parse", branch]);
        state["checks"][head.trim()] =
            json!([{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]);
    }
    state["prs"].as_array_mut().unwrap().reverse();
    scenario.write_gh_state(&state);
    scenario.agent_does("exit 1");

    for number in [2, 3] {
        let result = scenario.run(&["fix", "no-base-fix", "no-email"]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        let state = scenario.gh_state();
        let pr = state["prs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|pr| pr["number"] == number)
            .unwrap();
        assert_eq!(pr["isDraft"], true, "{state}");
        assert!(pr["labels"].as_array().unwrap().is_empty());
        assert!(
            scenario
                .launch_git(&["ls-remote", "origin", "refs/thirdshift/reservations/*"])
                .is_empty()
        );
    }
    assert_eq!(scenario.claude_calls().len(), 2);
    let result = scenario.run(&["fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("no eligible pull request"),
        "{}",
        result.stderr
    );
}

#[test]
fn pass_filters_ineligible_pull_requests_and_reads_only_current_head_ci() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("other-base", "main", &["Other base"]);
    for branch in [
        "draft",
        "fork",
        "untrusted",
        "busy",
        "closed",
        "wrong-base",
        "pending",
        "mixed",
        "old-red",
        "green",
        "absent",
        "eligible",
    ] {
        scenario.origin_has_branch(branch, "main", &["Feature"]);
        scenario.github_has_pr(
            branch,
            if branch == "wrong-base" {
                "other-base"
            } else {
                "main"
            },
            "OPEN",
        );
    }
    scenario.origin_has_commit("old-red", "old.txt", "updated", "Updated head");
    let mut state = scenario.gh_state();
    for branch in [
        "draft",
        "fork",
        "untrusted",
        "busy",
        "closed",
        "wrong-base",
        "eligible",
    ] {
        let head = scenario.origin_git(&["rev-parse", branch]);
        state["checks"][head.trim()] =
            json!([{"name":"test","conclusion":"failure","url":"https://ci.example/test"}]);
    }
    state["prs"][0]["isDraft"] = true.into();
    state["prs"][1]["isCrossRepository"] = true.into();
    state["prs"][2]["authorAssociation"] = "NONE".into();
    state["prs"][3]["labels"] = json!(["In-Progress"]);
    state["prs"][4]["state"] = "CLOSED".into();
    let pending = scenario.origin_git(&["rev-parse", "pending"]);
    state["checks"][pending.trim()] =
        json!([{"name":"test","conclusion":"success","pending_polls":1000}]);
    let mixed = scenario.origin_git(&["rev-parse", "mixed"]);
    state["checks"][mixed.trim()] = json!([
        {"name":"test","conclusion":"failure"}, {"name":"lint","conclusion":"success","pending_polls":1000}
    ]);
    let old = scenario.origin_git(&["rev-parse", "old-red~1"]);
    state["checks"][old.trim()] = json!([{"name":"test","conclusion":"failure"}]);
    let green = scenario.origin_git(&["rev-parse", "green"]);
    state["checks"][green.trim()] = json!([{"name":"test","conclusion":"success"}]);
    scenario.write_gh_state(&state);
    scenario.agent_does("exit 1");

    let result = scenario.run(&["fix", "no-base-fix", "no-email"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let state = scenario.gh_state();
    assert_eq!(state["prs"][11]["isDraft"], true, "{state}");
    assert_eq!(scenario.claude_calls().len(), 1);
    for pr in state["prs"].as_array().unwrap().iter().skip(1).take(10) {
        assert_eq!(pr["isDraft"], false, "{pr}");
    }
}

#[test]
fn base_and_trusted_author_filters_accept_both_spellings() {
    for (base_flag, author_flag) in [("base", "author"), ("--base", "--author")] {
        let scenario = Scenario::new();
        scenario.origin_has_branch("release", "main", &["Release"]);
        for (branch, base) in [
            ("wrong-base", "main"),
            ("wrong-author", "release"),
            ("selected", "release"),
        ] {
            scenario.origin_has_branch(branch, base, &["Feature"]);
            scenario.github_has_pr(branch, base, "OPEN");
        }
        let mut state = scenario.gh_state();
        state["collaborators"] = json!(["colleague"]);
        state["prs"][1]["author"] = "someone-else".into();
        for branch in ["wrong-base", "wrong-author", "selected"] {
            let head = scenario.origin_git(&["rev-parse", branch]);
            state["checks"][head.trim()] = json!([{"name":"test","conclusion":"failure"}]);
        }
        scenario.write_gh_state(&state);
        scenario.agent_does("exit 1");
        let result = scenario.run(&[
            "fix",
            base_flag,
            "release",
            author_flag,
            "COLLEAGUE",
            "no-base-fix",
            "no-email",
        ]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert_eq!(scenario.gh_state()["prs"][2]["isDraft"], true);
        assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
        assert_eq!(scenario.gh_state()["prs"][1]["isDraft"], false);
    }
    let scenario = Scenario::new();
    let result = scenario.run(&["fix", "--author", "outsider"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("not a Trusted account"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn targeted_prs_reject_pass_filters_and_missing_or_repeated_values() {
    let scenario = Scenario::new();
    for args in [
        vec!["fix", "1", "base", "main"],
        vec![
            "fix",
            "--base",
            "main",
            "https://github.com/acme/widgets/pull/1",
        ],
        vec!["fix", "1", "--author", "colleague"],
        vec!["fix", "author", "colleague", "1"],
        vec!["fix", "--author"],
        vec!["fix", "author", "--merge"],
        vec!["fix", "base"],
        vec!["fix", "--base", "--merge"],
        vec!["fix", "author", "colleague", "--author", "colleague"],
        vec!["fix", "base", "main", "--base", "main"],
    ] {
        let result = scenario.run(&args);
        assert_eq!(result.code, Some(2), "{args:?}: {}", result.stderr);
    }
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn busy_origin_reservations_are_passed_over_and_all_busy_is_a_skip() {
    let scenario = Scenario::new();
    for branch in ["busy", "available"] {
        scenario.origin_has_branch(branch, "main", &["Feature"]);
        scenario.github_has_pr(branch, "main", "OPEN");
    }
    let mut state = scenario.gh_state();
    for branch in ["busy", "available"] {
        let head = scenario.origin_git(&["rev-parse", branch]);
        state["checks"][head.trim()] = json!([{"name":"test","conclusion":"failure"}]);
    }
    scenario.write_gh_state(&state);
    let owner = scenario.origin_git(&["rev-parse", "main"]);
    scenario.origin_git(&["update-ref", "refs/thirdshift/reservations/1", owner.trim()]);
    scenario.agent_does("exit 1");
    let selected = scenario.run(&["fix", "no-base-fix", "no-email"]);
    assert_eq!(selected.code, Some(1), "{}", selected.stderr);
    assert_eq!(scenario.gh_state()["prs"][1]["isDraft"], true);
    let skipped = scenario.run(&["fix"]);
    assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    assert!(
        skipped.stderr.contains("no eligible pull request"),
        "{}",
        skipped.stderr
    );
    assert_eq!(
        scenario.origin_git(&["rev-parse", "refs/thirdshift/reservations/1"]),
        owner
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn pass_skips_at_the_command_limit_without_a_session_or_notification() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("feature", "main", &["Feature"]);
    let url = scenario.github_has_pr("feature", "main", "OPEN");
    scenario.agent_does(&format!(
        "touch '{}'\nwhile test ! -f '{}'; do sleep 0.02; done\nprintf 'No findings.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
        scenario.path("started").display(), scenario.path("release").display()
    ));
    let held = scenario.run_until(&["review", &url, "--recommend-only"], &[], "started");
    let skipped = scenario.run(&["fix"]);
    std::fs::write(scenario.path("release"), "").unwrap();
    let first = held.finish();
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    assert!(
        skipped.stderr.contains("at the Command limit"),
        "{}",
        skipped.stderr
    );
    assert!(skipped.stdout.is_empty());
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands/fix")
            .exists()
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn conflict_is_selected_even_while_ci_is_pending_and_finishes_ready() {
    let scenario = Scenario::new();
    scenario.origin_has_branch("conflict", "main", &["Feature"]);
    scenario.origin_has_commit("conflict", "shared.txt", "feature", "Feature content");
    scenario.origin_has_commit("main", "shared.txt", "base", "Base content");
    let url = scenario.github_has_pr("conflict", "main", "OPEN");
    let head = scenario.origin_git(&["rev-parse", "conflict"]);
    let mut state = scenario.gh_state();
    state["prs"][0]["mergeable"] = "CONFLICTING".into();
    state["checks"][head.trim()] =
        json!([{"name":"test","conclusion":"success","pending_polls":1000}]);
    scenario.write_gh_state(&state);
    scenario.agent_does(
        r#"
test -s .thirdshift-review-input.md
test -n "$(git ls-remote origin refs/thirdshift/reservations/1)"
echo resolved > shared.txt
git add shared.txt
git commit -qm 'Resolve base conflict'
gh fake pr conflict mergeable '"MERGEABLE"'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );

    let result = scenario.run(&["fix"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains(&format!("PR {url} is ready for review")),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(
        scenario
            .first_prompt()
            .contains("/thirdshift-resolving-merge-conflicts")
    );
    assert_eq!(
        scenario.origin_file("conflict", "shared.txt").as_deref(),
        Some("resolved\n")
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], false);
    assert!(
        scenario
            .launch_git(&["ls-remote", "origin", "refs/thirdshift/reservations/*"])
            .is_empty()
    );
    let (_, notification) = support::resend::saved_notification(&scenario, "fix");
    assert!(notification.contains("Fix run"), "{notification}");
}

#[test]
fn candidate_changed_after_selection_is_released_and_the_next_pr_is_taken() {
    for action in [
        "gh fake pr stale isDraft true",
        "gh fake pr stale state '\"CLOSED\"'",
        "gh fake pr stale base '\"other-base\"'",
        "gh fake pr stale authorAssociation '\"NONE\"'",
    ] {
        let scenario = Scenario::new();
        scenario.origin_has_branch("other-base", "main", &["Base"]);
        for branch in ["stale", "waiting"] {
            scenario.origin_has_branch(branch, "main", &["Feature"]);
            scenario.github_has_pr(branch, "main", "OPEN");
        }
        let mut state = scenario.gh_state();
        for branch in ["stale", "waiting"] {
            let head = scenario.origin_git(&["rev-parse", branch]);
            state["checks"][head.trim()] = json!([{"name":"test","conclusion":"failure"}]);
        }
        // The race happens after PR selection; the unscoped Pass now observes
        // Base branch CI before reading a candidate's head.
        let base_head = scenario.origin_git(&["rev-parse", "main"]);
        state["on_ci_read"] = json!({"times":1, "seen":[base_head.trim()], "script":action});
        scenario.write_gh_state(&state);
        scenario.agent_does("exit 1");
        let result = scenario.run(&["fix", "no-base-fix", "no-email"]);
        assert_eq!(result.code, Some(1), "{action}: {}", result.stderr);
        assert_eq!(scenario.gh_state()["prs"][1]["isDraft"], true);
        assert_eq!(scenario.claude_calls().len(), 1);
        assert!(
            scenario
                .launch_git(&["ls-remote", "origin", "refs/thirdshift/reservations/*"])
                .is_empty()
        );
        assert!(
            scenario.gh_state()["prs"][0]["labels"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn a_fix_pass_can_share_the_repository_up_to_a_raised_command_limit() {
    let scenario = Scenario::new();
    scenario.user_config_is("[pass]\ncommand_limit = 2\n");
    for branch in ["reviewed", "repair"] {
        scenario.origin_has_branch(branch, "main", &["Feature"]);
        scenario.github_has_pr(branch, "main", "OPEN");
    }
    let head = scenario.origin_git(&["rev-parse", "repair"]);
    let mut state = scenario.gh_state();
    state["checks"][head.trim()] = json!([{"name":"test","conclusion":"failure"}]);
    scenario.write_gh_state(&state);
    scenario.agent_does(&format!(
        r#"
if test -e '{}'; then exit 1; fi
touch '{}'
while test ! -f '{}'; do sleep 0.02; done
printf 'No findings.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
        scenario.path("started").display(),
        scenario.path("started").display(),
        scenario.path("release").display()
    ));
    let held = scenario.run_until(
        &[
            "review",
            "https://github.com/acme/widgets/pull/1",
            "--recommend-only",
        ],
        &[],
        "started",
    );
    let repaired = scenario.run(&["fix", "no-base-fix", "no-email"]);
    std::fs::write(scenario.path("release"), "").unwrap();
    let first = held.finish();
    assert_eq!(first.code, Some(0), "{}", first.stderr);
    assert_eq!(repaired.code, Some(1), "{}", repaired.stderr);
    assert_eq!(scenario.gh_state()["prs"][1]["isDraft"], true);
    assert_eq!(scenario.claude_calls().len(), 2);
}
