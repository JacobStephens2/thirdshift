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
