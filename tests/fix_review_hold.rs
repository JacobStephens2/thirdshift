//! Fix runs repair unreviewed heads without failing unattended merge requests.

mod support;

use serde_json::json;
use support::Scenario;

fn broken_unreviewed_pr(scenario: &Scenario) -> String {
    scenario.origin_has_branch("unreviewed", "main", &["Manual feature"]);
    let url = scenario.github_has_pr("unreviewed", "main", "OPEN");
    let head = scenario.origin_git(&["rev-parse", "unreviewed"]);
    let mut state = scenario.gh_state();
    state["checks"][head.trim()] = json!([{"name":"test","conclusion":"failure"}]);
    scenario.write_gh_state(&state);
    scenario.agent_does(
        r#"
echo repaired > ci-fix.txt
git add ci-fix.txt
git commit -qm 'Fix branch CI'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );
    url
}

fn assert_repaired(scenario: &Scenario, expected_state: &str) {
    let delivered_branch = if expected_state == "MERGED" {
        "main"
    } else {
        "unreviewed"
    };
    assert_eq!(
        scenario
            .origin_file(delivered_branch, "ci-fix.txt")
            .as_deref(),
        Some("repaired\n")
    );
    let state = scenario.gh_state();
    assert_eq!(state["prs"][0]["state"], expected_state);
    assert_eq!(state["prs"][0]["isDraft"], false);
    assert!(state["prs"][0]["labels"].as_array().unwrap().is_empty());
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(
        scenario
            .launch_git(&["ls-remote", "origin", "refs/thirdshift/reservations/*"])
            .is_empty()
    );
}

#[test]
fn merge_pass_repairs_an_unreviewed_head_and_finishes_ready_for_review() {
    let scenario = Scenario::new();
    let url = broken_unreviewed_pr(&scenario);

    let result = scenario.run(&["fix", "base", "main", "merge", "no-base-fix", "no-email"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("has no recorded agent review"),
        "{}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains(&format!("PR {url} is ready for review")),
        "{}",
        result.stderr
    );
    assert_repaired(&scenario, "OPEN");
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/fix");
    assert_eq!(record["outcome"], "ready for review", "{record}");
    let (command_log, notification) = support::resend::saved_notification(&scenario, "fix");
    let command_log = std::fs::read_to_string(command_log).unwrap();
    assert!(command_log.contains("Self-merge held:"), "{command_log}");
    assert!(notification.contains("ready for review"), "{notification}");
    assert!(!notification.contains("Failed"), "{notification}");
    let activity =
        std::fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(activity.contains("ready for review"), "{activity}");
    assert!(!activity.contains("failed"), "{activity}");

    let next = scenario.run(&["fix", "base", "main", "merge", "no-email"]);
    assert_eq!(next.code, Some(0), "{}", next.stderr);
    assert!(
        next.stderr.contains("no eligible pull request"),
        "{}",
        next.stderr
    );
    assert_eq!(scenario.claude_calls().len(), 1);
}

#[test]
fn merge_defaults_and_explicit_no_merge_leave_unreviewed_repairs_ready() {
    for targeted in [false, true] {
        for no_merge in [false, true] {
            let scenario = Scenario::new();
            scenario.user_config_is("[merge]\nalways = true\n");
            let url = broken_unreviewed_pr(&scenario);
            let mut args = if targeted {
                vec!["fix", &url]
            } else {
                vec!["fix", "base", "main"]
            };
            args.extend(["no-base-fix", "no-email"]);
            if no_merge {
                args.push("no-merge");
            }

            let result = scenario.run(&args);

            assert_eq!(result.code, Some(0), "{}", result.stderr);
            assert_eq!(
                result.stderr.contains("Self-merge held:"),
                !no_merge,
                "{}",
                result.stderr
            );
            assert_repaired(&scenario, "OPEN");
            assert!(scenario.gh_calls_of("pr", "merge").is_empty());
            let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/fix");
            assert_eq!(record["outcome"], "ready for review", "{record}");
        }
    }
}

#[test]
fn explicit_targeted_merge_repairs_but_still_fails_with_the_review_hold() {
    let scenario = Scenario::new();
    let url = broken_unreviewed_pr(&scenario);

    let result = scenario.run(&["fix", &url, "merge", "no-base-fix", "no-email"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("has no recorded agent review"),
        "{}",
        result.stderr
    );
    assert_repaired(&scenario, "OPEN");
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/fix");
    assert_eq!(record["outcome"], "failed", "{record}");
}

#[test]
fn merge_pass_accepts_exact_authored_reviewed_and_pushed_head_evidence() {
    for evidence in ["authored_head", "reviewed_head", "pushed_head"] {
        let scenario = Scenario::new();
        broken_unreviewed_pr(&scenario);
        let head = scenario.origin_git(&["rev-parse", "unreviewed"]);
        let mut marker = json!({});
        marker[evidence] = head.trim().into();
        let mut state = scenario.gh_state();
        state["prs"][0]["comments"] = json!([{
            "body": format!("<!-- thirdshift-review {marker} -->"),
            "author_association": "COLLABORATOR", "user": {"login": "runner"}
        }]);
        scenario.write_gh_state(&state);

        let result = scenario.run(&["fix", "base", "main", "merge", "no-base-fix", "no-email"]);

        assert_eq!(result.code, Some(0), "{evidence}: {}", result.stderr);
        assert_repaired(&scenario, "MERGED");
        assert_eq!(scenario.gh_calls_of("pr", "merge").len(), 1);
        let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/fix");
        assert_eq!(record["outcome"], "merged", "{record}");
    }
}

#[test]
fn merge_pass_holds_review_for_the_head_acquired_after_selection() {
    let scenario = Scenario::new();
    broken_unreviewed_pr(&scenario);
    let original = scenario.origin_git(&["rev-parse", "unreviewed"]);
    scenario.origin_has_branch("concurrent", "unreviewed", &["Unreviewed follow-up"]);
    let foreign = scenario.origin_git(&["rev-parse", "concurrent"]);
    let mut state = scenario.gh_state();
    state["checks"][foreign.trim()] = json!([{"name":"test","conclusion":"failure"}]);
    state["prs"][0]["comments"] = json!([{
        "body": format!("<!-- thirdshift-review {} -->", json!({"reviewed_head": original.trim()})),
        "author_association": "COLLABORATOR", "user": {"login": "runner"}
    }]);
    scenario.write_gh_state(&state);
    let marker = scenario.path("advanced-at-fetch");
    support::write_executable(
        &scenario.path("bin/git"),
        &format!(
            r#"#!/bin/sh
if [ "$1" = fetch ] && [ ! -e '{marker}' ]; then
  /usr/bin/git --git-dir='{origin}' update-ref refs/heads/unreviewed '{foreign}' || exit 1
  touch '{marker}'
fi
exec /usr/bin/git "$@"
"#,
            marker = marker.display(),
            origin = scenario.origin_dir().display(),
            foreign = foreign.trim()
        ),
    );

    let result = scenario.run(&["fix", "base", "main", "merge", "no-base-fix", "no-email"]);

    assert!(
        marker.is_file(),
        "acquisition race did not run: {}",
        result.stderr
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains(&format!(
            "adopted head {} has no recorded agent review",
            foreign.trim()
        )),
        "{}",
        result.stderr
    );
    assert_repaired(&scenario, "OPEN");
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
}

#[test]
fn merge_default_pass_repairs_an_unreviewed_conflict_and_finishes_ready() {
    let scenario = Scenario::new();
    scenario.user_config_is("[merge]\nalways = true\n");
    scenario.origin_has_branch("unreviewed", "main", &["Manual feature"]);
    scenario.origin_has_commit("unreviewed", "shared.txt", "feature", "Feature content");
    scenario.origin_has_commit("main", "shared.txt", "base", "Base content");
    scenario.github_has_pr("unreviewed", "main", "OPEN");
    let mut state = scenario.gh_state();
    state["prs"][0]["mergeable"] = "CONFLICTING".into();
    scenario.write_gh_state(&state);
    scenario.agent_does(
        r#"
echo resolved > shared.txt
echo repaired > ci-fix.txt
git add shared.txt ci-fix.txt
git commit -qm 'Resolve conflict'
gh fake pr unreviewed mergeable '"MERGEABLE"'
gh fake checks "$(git rev-parse HEAD)" '[{"name":"test","conclusion":"success"}]'
"#,
    );

    let result = scenario.run(&["fix", "base", "main", "no-base-fix", "no-email"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_repaired(&scenario, "OPEN");
    assert_eq!(
        scenario.origin_file("unreviewed", "shared.txt").as_deref(),
        Some("resolved\n")
    );
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
}
