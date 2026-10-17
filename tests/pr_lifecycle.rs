//! Obsolete Delivery work stops when its selected PR is no longer actionable.
mod support;

use support::Scenario;

const OPEN: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title Feature --body 'Closes #7'
"#;

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
