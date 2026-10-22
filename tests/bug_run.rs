//! Bug Runs diagnose before implementation, and may end needing more information.

mod support;

use support::Scenario;
use support::resend::saved_notification;

const IMPLEMENT: &str = r#"
test -f .claude/skills/thirdshift-diagnosing-bugs/SKILL.md || exit 1
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Fix bug'
gh pr create --base main --head issue-7 --title 'Fix bug' --body 'Closes #7'
echo 'Bug reproduced: fixed' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

#[test]
fn bug_runs_diagnose_before_implementation_in_fresh_runs_and_continuations() {
    for (continuing, label) in [(false, "bug"), (true, "BuG")] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &[label, "ready-for-agent"]);
        if continuing {
            scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
        }
        scenario.agent_does(IMPLEMENT);

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let prompt = scenario.first_prompt();
        assert!(prompt.contains("thirdshift-diagnosing-bugs"), "{prompt}");
        assert!(prompt.contains("before changing code"), "{prompt}");
        assert!(prompt.contains("Bug not reproduced: "), "{prompt}");
        assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    }
}

#[test]
fn an_unreproduced_bug_ends_successfully_with_its_comment_and_without_delivery() {
    for continuing in [false, true] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
        if continuing {
            scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
        }
        let original_branch = scenario.origin_log("issue-7");
        scenario.agent_does(r#"
gh fake labels 7 '["bug", "needs-info", "in-progress"]'
echo 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

        let result = scenario.run(&[&scenario.issue_url(7), "--merge", "--security-review"]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert_eq!(
            result.stdout,
            "https://github.com/acme/widgets/issues/7#issuecomment-42\n"
        );
        assert!(
            result.stderr.contains("bug not reproduced"),
            "{}",
            result.stderr
        );
        assert_eq!(
            scenario.claude_calls().len(),
            1,
            "no Security review or Repair"
        );
        assert_eq!(scenario.gh_state()["prs"], serde_json::json!([]));
        assert_eq!(scenario.origin_log("issue-7"), original_branch);
        assert_eq!(scenario.issue_labels(7), ["bug", "needs-info"]);
        scenario.assert_cleaned_up("issue-7");
        let (_, notification) = saved_notification(&scenario, "issue");
        assert!(
            notification.contains("bug not reproduced"),
            "{notification}"
        );
        assert!(
            notification.contains("issues/7#issuecomment-42"),
            "{notification}"
        );
        assert!(!notification.contains("failed"), "{notification}");
        let pickup = scenario.run(&["pickup"]);
        assert_eq!(pickup.code, Some(0), "{}", pickup.stderr);
        assert!(pickup.stdout.is_empty(), "{}", pickup.stdout);
        assert_eq!(
            scenario.claude_calls().len(),
            1,
            "needs-info stays out of Pickup"
        );
    }
}

#[test]
fn an_unreproduced_bug_ticket_is_not_reported_as_landed_by_its_spec_run() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(7, &[])]);
    scenario.issue_labelled(7, &["bug"]);
    scenario.agent_does_for(7, r#"
gh fake labels 7 '["bug", "needs-info"]'
echo 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&[&scenario.issue_url(20)]);

    assert_eq!(
        result.code,
        Some(1),
        "the Spec is incomplete: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("#7 bug not reproduced"),
        "{}",
        result.stderr
    );
    assert!(!result.stderr.contains("#7 landed"), "{}", result.stderr);
    assert!(!result.stderr.contains("#7 failed"), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert_eq!(scenario.gh_state()["prs"], serde_json::json!([]));
}

#[test]
fn an_unreproduced_bug_leaves_its_existing_pr_open_and_unchanged() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug"]);
    scenario.origin_has_branch("issue-7", "main", &["Earlier work"]);
    scenario.github_has_pr("issue-7", "main", "OPEN");
    let pr = scenario.gh_state()["prs"][0].clone();
    let history = scenario.origin_log("issue-7");
    scenario.agent_does(r#"
gh fake labels 7 '["bug", "needs-info", "in-progress"]'
echo 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&[&scenario.issue_url(7), "--merge"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.gh_state()["prs"][0], pr);
    assert_eq!(scenario.origin_log("issue-7"), history);
    assert!(
        scenario
            .first_prompt()
            .contains("Update PR https://github.com/acme/widgets/pull/1")
    );
}

#[test]
fn other_issue_labels_keep_the_implementation_prompt_and_ignore_a_diagnosis_marker() {
    for labels in [vec![], vec!["enhancement"], vec!["bug", "security-fix"]] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &labels);
        scenario.agent_does(&format!(r#"
{IMPLEMENT}
echo 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#));

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let prompt = scenario.first_prompt();
        assert!(prompt.starts_with("/thirdshift-implement "), "{prompt}");
        assert!(
            !prompt.contains("Use the `thirdshift-diagnosing-bugs` skill"),
            "{prompt}"
        );
        assert_eq!(result.stdout, "https://github.com/acme/widgets/pull/1\n");
    }
}

#[test]
fn an_invalid_diagnosis_handoff_is_an_operational_failure() {
    for (url, labels) in [
        (
            "https://github.com/acme/widgets/issues/8#issuecomment-42",
            r#"["bug", "needs-info"]"#,
        ),
        (
            "https://github.com/acme/widgets/issues/7#issuecomment-42",
            r#"["bug", "ready-for-agent"]"#,
        ),
    ] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["bug"]);
        scenario.agent_does(&format!(
            r#"
gh fake labels 7 '{labels}'
echo 'Bug not reproduced: {url}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
        ));

        let result = scenario.run(&[&scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains("bug not reproduced ending"),
            "{}",
            result.stderr
        );
        assert_eq!(scenario.gh_state()["prs"], serde_json::json!([]));
    }
}
