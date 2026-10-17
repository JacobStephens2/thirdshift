//! Delivery stops when its selected PR ceases to be actionable.
mod support;

use support::Scenario;

const OPEN: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title Feature --body 'Closes #7'
"#;

#[test]
fn lifecycle_changes_during_pending_ci_stop_polling_without_false_success() {
    for state in ["MERGED", "CLOSED", "DRAFT"] {
        let scenario = Scenario::new();
        let mut transition = if state == "DRAFT" {
            "gh fake pr issue-7 isDraft true".to_string()
        } else {
            format!("gh fake pr issue-7 state '\"{state}\"'")
        };
        if state == "MERGED" {
            transition.push_str("\ngit push -q origin --delete issue-7");
        }
        scenario.agent_does(&format!(
            r#"{OPEN}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"success","pending_polls":20}}]'
gh fake on-ci-read 1 '{transition}'
"#,
            transition = transition.replace('\'', "'\\''"),
        ));

        let result = scenario.run(&[&scenario.issue_url(7), "--merge"]);

        assert_ne!(result.code, Some(0), "{state}: {}", result.stderr);
        let expected = if state == "DRAFT" {
            "is a draft".to_string()
        } else {
            format!("is {}, not open", state.to_lowercase())
        };
        assert!(
            result.stderr.contains(&expected),
            "{state}: {}",
            result.stderr
        );
        assert!(!result.stderr.contains("CI passed"), "{}", result.stderr);
        assert!(scenario.gh_calls_of("pr", "merge").is_empty());
        assert_eq!(scenario.claude_calls().len(), 1);
        if state == "MERGED" {
            assert_eq!(scenario.origin_log("issue-7"), None);
        }
        let checks = scenario.gh_state()["checks"].clone();
        let pending = checks.as_object().unwrap().values().next().unwrap()[0]["pending_polls"]
            .as_i64()
            .unwrap();
        assert!(pending >= 18, "obsolete polling continued: {checks}");
    }
}

#[test]
fn terminal_pr_after_a_session_never_recreates_a_deleted_branch_or_edits_its_body() {
    for repair in [false, true] {
        for state in ["MERGED", "CLOSED"] {
            let scenario = Scenario::new();
            let stop = format!(
                r#"
git push -q origin issue-7
gh fake pr issue-7 state '"{state}"'
git push -q origin --delete issue-7
echo unpublished > unpublished.txt
"#
            );
            if repair {
                scenario.agent_does(&format!(
                    r#"{OPEN}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure"}}]'
"#
                ));
                scenario.agent_does_in_session(2, &stop);
            } else {
                scenario.agent_does(&format!("{OPEN}{stop}"));
            }

            let result = scenario.run(&[&scenario.issue_url(7), "--merge"]);

            assert_ne!(result.code, Some(0), "{}", result.stderr);
            assert!(
                result
                    .stderr
                    .contains(&format!("is {}, not open", state.to_lowercase())),
                "{}",
                result.stderr
            );
            assert_eq!(
                scenario.origin_log("issue-7"),
                None,
                "a terminal PR must not recreate its branch: {}",
                result.stderr
            );
            let calls = scenario.claude_calls();
            assert_eq!(calls.len(), if repair { 2 } else { 1 });
            let worktree = std::path::Path::new(calls.last().unwrap()["cwd"].as_str().unwrap());
            assert_eq!(
                std::fs::read_to_string(worktree.join("unpublished.txt")).unwrap(),
                "unpublished\n"
            );
            assert!(
                result.stderr.contains("work may exist only locally"),
                "{}",
                result.stderr
            );
            if !repair {
                assert_eq!(scenario.gh_state()["prs"][0]["body"], "Closes #7");
            }
            assert!(scenario.gh_calls_of("pr", "merge").is_empty());
        }
    }
}

#[test]
fn a_drafted_repair_preserves_real_work_but_does_not_resume_delivery() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        r#"{OPEN}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure"}}]'
"#
    ));
    scenario.agent_does_in_session(
        2,
        "gh fake pr issue-7 isDraft true\necho fix > repair.txt\n",
    );

    let result = scenario.run(&[&scenario.issue_url(7), "--merge"]);

    assert_ne!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stderr.contains("is a draft"), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    assert_eq!(
        scenario.origin_file("issue-7", "repair.txt"),
        Some("fix\n".into())
    );
    assert_eq!(scenario.claude_calls().len(), 2);
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn session_failure_is_retained_when_the_pr_also_becomes_terminal() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        r#"{OPEN}
git push -q origin issue-7
gh fake pr issue-7 state '"MERGED"'
git push -q origin --delete issue-7
echo unfinished > unfinished.txt
exit 3
"#
    ));

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("claude exited 3"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("is merged, not open"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.origin_log("issue-7"), None);
    assert_eq!(scenario.gh_state()["prs"][0]["body"], "Closes #7");
    assert_eq!(
        std::fs::read_to_string(scenario.path("work/widgets-issue-7/unfinished.txt")).unwrap(),
        "unfinished\n"
    );
}

#[test]
fn external_merge_during_refused_request_is_not_claimed_as_self_merge() {
    let scenario = Scenario::new();
    let external_merge = r#"
gh fake pr issue-7 headRefOid "\"$(git rev-parse refs/remotes/origin/issue-7)\""
gh fake pr issue-7 state '"MERGED"'
git push -q origin --delete issue-7
"#;
    let quoted = external_merge.replace('\'', "'\\''");
    scenario.agent_does(&format!(
        "{OPEN}\ngh fake on-merge 1 '{quoted}'\ngh fake refuse-merges 1 'Already merged externally'\n"
    ));
    let result = scenario.run(&[&scenario.issue_url(7), "--merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("merge request failed"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("the merger is unconfirmed"),
        "{}",
        result.stderr
    );
    let comments = scenario.gh_state()["comments"].to_string();
    assert!(comments.contains("the merger is unconfirmed"), "{comments}");
    assert!(
        !comments.contains("merged into main by a thirdshift Merge run"),
        "external merge claimed as Self-merge: comments={comments}; stderr={}",
        result.stderr
    );
}

#[test]
fn unchanged_repair_does_not_make_a_pointless_delivery_push() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(r#"{OPEN}
gh fake checks "$(git rev-parse HEAD)" '[{{"name":"test","conclusion":"failure","url":"https://github.com/acme/widgets/actions/runs/900/job/1","rerun":{{"conclusion":"success"}}}}]'
"#));
    scenario.agent_does_in_session(2, "true\n");
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let (_, after_repair) = result.stderr.split_once("starting Repair 1 of 5").unwrap();
    assert!(
        !after_repair.contains("thirdshift: pushing issue-7"),
        "unchanged Repair still pushed without any upstream change: {}",
        result.stderr
    );
}

#[test]
fn externally_merged_pr_with_successful_gh_noop_is_not_attributed_to_factory() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let scenario = Scenario::new();
    fs::create_dir(scenario.path("bin/delegate")).unwrap();
    fs::rename(scenario.path("bin/gh"), scenario.path("bin/delegate/gh")).unwrap();
    fs::write(
        scenario.path("bin/gh"),
        r#"#!/bin/bash
set -e
delegate="${0%/*}/delegate/gh"
if [[ "${1-}" == pr && "${2-}" == merge ]]; then
    "$delegate" fake pr issue-7 headRefOid "\"$(git rev-parse refs/remotes/origin/issue-7)\""
    "$delegate" fake pr issue-7 state '"MERGED"'
    git push -q origin --delete issue-7
    printf '%s\n' '! Pull request #1 was already merged' >&2
    exit 0
fi
exec "$delegate" "$@"
"#,
    )
    .unwrap();
    fs::set_permissions(scenario.path("bin/gh"), fs::Permissions::from_mode(0o755)).unwrap();
    scenario.agent_does(
        r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title Feature --body 'Closes #7'
"#,
    );
    let result = scenario.run(&[&scenario.issue_url(7), "--merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0]["state"], "MERGED");
    let comments = scenario.gh_state()["comments"].to_string();
    assert!(
        !comments.contains("merged into main by a thirdshift Merge run"),
        "gh's successful no-op falsely attributed an external merge: {comments}; stderr={}",
        result.stderr
    );
}

#[test]
fn lifecycle_changes_stop_pending_ci_without_recreating_a_deleted_branch() {
    for (field, value, reason) in [
        ("state", "\"MERGED\"", "is merged, not open"),
        ("state", "\"CLOSED\"", "is closed, not open"),
        ("isDraft", "true", "is a draft"),
    ] {
        let scenario = Scenario::new();
        let delete = if field == "state" {
            "; git push -q origin --delete issue-7"
        } else {
            ""
        };
        scenario.agent_does(&format!(
            "{OPEN}\ngh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\":\"test\",\"conclusion\":\"success\",\"pending_polls\":100}}]'\ngh fake on-ci-read 1 'gh fake pr issue-7 {field} '\"'\"'{value}'\"'\"'{delete}'\n"
        ));
        let result = scenario.run(&["merge", &scenario.issue_url(7)]);
        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(result.stderr.contains(reason), "{}", result.stderr);
        assert!(!result.stderr.contains("CI passed"), "{}", result.stderr);
        if field == "state" {
            assert!(
                scenario.origin_log("issue-7").is_none(),
                "obsolete push recreated branch"
            );
        }
        assert!(scenario.gh_calls_of("pr", "merge").is_empty());
        assert_eq!(scenario.claude_calls().len(), 1);
    }
}

#[test]
fn terminal_prs_after_opening_or_repair_stop_pushes_and_body_edits() {
    for repair in [false, true] {
        for state in ["MERGED", "CLOSED"] {
            let scenario = Scenario::new();
            let terminal = format!(
                "git push -q origin issue-7\ngh fake pr issue-7 state '\"{state}\"'\ngit push -q origin --delete issue-7\necho unpublished > unpublished.txt\n"
            );
            if repair {
                scenario.agent_does(&format!(
                    "{OPEN}\ngh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\":\"test\",\"conclusion\":\"failure\"}}]'\n"
                ));
                scenario.agent_does_in_session(2, &terminal);
            } else {
                scenario.agent_does(&format!("{OPEN}{terminal}"));
            }
            let result = scenario.run(&["merge", &scenario.issue_url(7)]);
            assert_eq!(result.code, Some(1), "{}", result.stderr);
            assert!(
                result
                    .stderr
                    .contains(&format!("is {}, not open", state.to_lowercase())),
                "{}",
                result.stderr
            );
            assert!(scenario.origin_log("issue-7").is_none());
            let cwd = scenario.claude_calls()[0]["cwd"]
                .as_str()
                .unwrap()
                .to_owned();
            assert_eq!(
                std::fs::read_to_string(std::path::Path::new(&cwd).join("unpublished.txt"))
                    .unwrap(),
                "unpublished\n"
            );
            if !repair {
                assert_eq!(scenario.gh_state()["prs"][0]["body"], "Closes #7");
            }
            assert!(scenario.gh_calls_of("pr", "merge").is_empty());
            assert!(
                !result.stderr.contains("is merged\n"),
                "external merge is not Self-merge"
            );
        }
    }
}

#[test]
fn a_draft_during_repair_fails_readiness_and_preserves_real_changes() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{OPEN}\ngh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\":\"test\",\"conclusion\":\"failure\"}}]'\n"
    ));
    scenario.agent_does_in_session(
        2,
        "echo repair > repair.txt\ngh fake pr issue-7 isDraft true\n",
    );
    let result = scenario.run(&["merge", &scenario.issue_url(7)]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("is a draft"), "{}", result.stderr);
    assert_eq!(
        scenario.origin_file("issue-7", "repair.txt"),
        Some("repair\n".to_owned())
    );
    assert_eq!(scenario.gh_state()["prs"][0]["isDraft"], true);
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
    assert_eq!(scenario.claude_calls().len(), 2);
}

#[test]
fn a_failed_repair_keeps_its_failure_when_the_pr_was_merged() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{OPEN}\ngh fake checks \"$(git rev-parse HEAD)\" '[{{\"name\":\"test\",\"conclusion\":\"failure\"}}]'\n"
    ));
    scenario.agent_does_in_session(
        2,
        "gh fake pr issue-7 state '\"MERGED\"'\ngit push -q origin --delete issue-7\nexit 17\n",
    );
    let result = scenario.run(&["merge", &scenario.issue_url(7)]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result.stderr.contains("is merged, not open"),
        "{}",
        result.stderr
    );
    assert!(
        result.stderr.contains("claude exited 17"),
        "session failure lost: {}",
        result.stderr
    );
    assert!(scenario.origin_log("issue-7").is_none());
    assert!(scenario.gh_calls_of("pr", "merge").is_empty());
}
