//! Bug Ticket runs diagnose before implementation, and may end needing more information.

mod support;

use support::Scenario;
use support::resend::saved_notification;

const IMPLEMENT: &str = r#"
test -f .claude/skills/thirdshift-diagnosing-bugs/SKILL.md || test -f .agents/skills/thirdshift-diagnosing-bugs/SKILL.md || exit 1
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Fix bug'
gh pr create --base main --head issue-7 --title 'Fix bug' --body 'Closes #7'
echo 'Bug reproduced: fixed' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#;

fn late_background_reply(earlier: &str) -> String {
    let earlier = serde_json::json!({
        "type": "assistant", "message": {
            "id": "earlier", "role": "assistant",
            "content": [{"type": "text", "text": earlier}]
        }
    });
    format!(
        r#"
cat <<'JSON'
{earlier}
{{"type":"system","subtype":"task_notification","task_id":"test","status":"failed","summary":"Earlier test failed"}}
{{"type":"assistant","message":{{"id":"later","role":"assistant","content":[{{"type":"text","text":"The earlier background failure was handled. The PR is unchanged."}}]}}}}
JSON
echo 'The earlier background failure was handled. The PR is unchanged.' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
    )
}

#[test]
fn bug_diagnosis_survives_a_late_background_reply() {
    for harness in ["claude", "grok"] {
        for reproduced in [true, false] {
            let scenario = Scenario::new();
            scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
            let script = if reproduced {
                IMPLEMENT
            } else {
                r#"
gh fake labels 7 '["bug", "needs-info", "in-progress"]'
echo 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
            };
            let earlier = if reproduced {
                "Bug reproduced: fixed"
            } else {
                "Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42"
            };
            scenario.agent_does(&format!("{script}\n{}", late_background_reply(earlier)));

            let result = scenario.run(&["harness", harness, &scenario.issue_url(7)]);

            assert_eq!(result.code, Some(0), "{}", result.stderr);
            assert_eq!(
                result.stdout,
                if reproduced {
                    "https://github.com/acme/widgets/pull/1\n"
                } else {
                    "https://github.com/acme/widgets/issues/7#issuecomment-42\n"
                }
            );
        }
    }
}

#[test]
fn bug_diagnosis_without_any_completion_line_keeps_its_error_after_a_late_reply() {
    for harness in ["claude", "grok"] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["bug"]);
        scenario.agent_does(&format!(
            "echo 'Done.' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n{}",
            late_background_reply("Done.")
        ));

        let result = scenario.run(&["harness", harness, &scenario.issue_url(7)]);

        assert_eq!(result.code, Some(1), "{}", result.stderr);
        assert!(
            result.stderr.contains(
                "bug diagnosis ended without a reproduced or not reproduced completion line"
            ),
            "{}",
            result.stderr
        );
    }
}

/// Probes the checkout the way #1362 did, unless the prompt names a merge base.
/// An already-fixed branch then goes on to review and the pull request.
const DIAGNOSE_AT_NAMED_MERGE_BASE: &str = r#"
hand_back() {
  gh fake labels 7 '["bug", "needs-info", "in-progress"]'
  gh issue comment 7 --body '## Triage Notes'
  printf '%s\n' 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
}
deliver_fix() {
  reports=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*`\([^`]*\)\/standards\.md`.*/\1/p')
  test -n "$reports"
  printf 'Standards\n\nFiles read:\n- behavior.txt\n' > "$reports/standards.md"
  printf 'Spec\n\nFiles read:\n- behavior.txt\n' > "$reports/spec.md"
  printf '%s\n' '{"standards":0,"spec":0}' > "$reports/summary.json"
  gh pr create --base main --head issue-7 --title 'Fix bug' --body 'Closes #7'
  printf '%s\n' 'Bug reproduced: fixed' > "$FAKE_CLAUDE_FINAL_MESSAGE"
}
merge_base=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Merge base commit: `\(.*\)`\.$/\1/p')
if [ -n "$merge_base" ]; then
  baseline=$(git show "$merge_base:behavior.txt")
  if [ "$baseline" != broken ]; then
    hand_back
    exit 0
  fi
  if [ "$(cat behavior.txt)" != fixed ]; then
    printf '%s\n' fixed > behavior.txt
    git add behavior.txt
    git commit -q -m 'Fix bug'
  fi
  deliver_fix
  exit 0
fi
if [ "$(cat behavior.txt 2>/dev/null || true)" = broken ]; then
  printf '%s\n' fixed > behavior.txt
  git add behavior.txt
  git commit -q -m 'Fix bug'
  deliver_fix
  exit 0
fi
hand_back
"#;

fn origin_merge_base(scenario: &Scenario, base: &str, branch: &str) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &scenario.origin_dir().to_string_lossy(),
            "merge-base",
            &format!("refs/heads/{base}"),
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

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
fn not_reproduced_accepts_canonical_comment_for_leading_zero_issue_url() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.agent_does(r#"
gh fake labels 7 '["bug", "needs-info", "in-progress"]'
echo 'Bug not reproduced: https://github.com/acme/widgets/issues/7#issuecomment-42' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);
    let input = "https://github.com/acme/widgets/issues/007";
    let result = scenario.run(&[input, "--merge"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        result.stdout,
        "https://github.com/acme/widgets/issues/7#issuecomment-42\n"
    );
    assert_eq!(scenario.gh_state()["prs"], serde_json::json!([]));
}

#[test]
fn other_issue_labels_keep_the_implementation_prompt_and_ignore_a_diagnosis_marker() {
    for labels in [
        vec![],
        vec!["enhancement"],
        vec!["bug", "security-fix"],
        vec!["bug", "FlAkY-TeSt"],
    ] {
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
        if labels.contains(&"FlAkY-TeSt") {
            assert!(
                prompt.contains("recorded CI failure is the evidence"),
                "{prompt}"
            );
        }
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

#[test]
fn a_continuation_reproduces_at_the_merge_base_and_delivers_the_existing_fix() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.origin_has_commit("main", "behavior.txt", "broken\n", "Bug is present");
    scenario.origin_has_branch("issue-7", "main", &[]);
    scenario.origin_has_commit("issue-7", "behavior.txt", "fixed\n", "Fix the bug");
    let merge_base = origin_merge_base(&scenario, "main", "issue-7");
    let history = scenario.origin_log("issue-7");
    scenario.agent_does(DIAGNOSE_AT_NAMED_MERGE_BASE);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(
        result.stdout, "https://github.com/acme/widgets/pull/1\n",
        "{}",
        result.stderr
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let prompt = scenario.first_prompt();
    assert!(
        prompt.contains(&format!("Merge base commit: `{merge_base}`.")),
        "{prompt}"
    );
    let calls = scenario.claude_calls();
    let skill = calls[0]["skill_files"]["thirdshift-diagnosing-bugs/SKILL.md"]
        .as_str()
        .unwrap();
    assert!(skill.contains("Reproduce at that merge base"), "{skill}");
    assert!(!scenario.issue_labels(7).contains(&"needs-info".to_string()));
    assert!(scenario.gh_calls_of("issue", "comment").is_empty());
    assert!(
        !result.stderr.contains("no review reports were left"),
        "{}",
        result.stderr
    );
    assert_eq!(scenario.origin_log("issue-7"), history);
}

#[test]
fn a_continuation_whose_merge_base_lacks_the_bug_still_needs_info() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.origin_has_commit("main", "behavior.txt", "fine\n", "No bug");
    scenario.origin_has_branch("issue-7", "main", &[]);
    scenario.origin_has_commit("issue-7", "notes.txt", "still fine\n", "Unrelated");
    let history = scenario.origin_log("issue-7");
    scenario.agent_does(DIAGNOSE_AT_NAMED_MERGE_BASE);

    let result = scenario.run(&[&scenario.issue_url(7)]);

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
    let prompt = scenario.first_prompt();
    let merge_base = origin_merge_base(&scenario, "main", "issue-7");
    assert!(
        prompt.contains(&format!("Merge base commit: `{merge_base}`.")),
        "{prompt}"
    );
    assert_eq!(scenario.gh_state()["prs"], serde_json::json!([]));
    assert_eq!(scenario.origin_log("issue-7"), history);
    assert_eq!(scenario.issue_labels(7), ["bug", "needs-info"]);
    assert!(
        scenario
            .gh_calls_of("issue", "comment")
            .iter()
            .any(|call| call.iter().any(|arg| arg.contains("Triage Notes")))
    );
}

#[test]
fn a_fresh_bug_run_reproduces_on_its_checkout_without_a_merge_base() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.origin_has_commit("main", "behavior.txt", "broken\n", "Bug is present");
    scenario.agent_does(DIAGNOSE_AT_NAMED_MERGE_BASE);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        result.stdout, "https://github.com/acme/widgets/pull/1\n",
        "{}",
        result.stderr
    );
    let prompt = scenario.first_prompt();
    assert!(!prompt.contains("Merge base commit:"), "{prompt}");
    assert_eq!(
        scenario.origin_file("issue-7", "behavior.txt").as_deref(),
        Some("fixed\n")
    );
}

#[test]
fn a_continuation_with_nothing_beyond_the_base_reproduces_on_its_checkout() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.origin_has_commit("main", "behavior.txt", "broken\n", "Bug is present");
    scenario.origin_has_branch("issue-7", "main", &[]);
    scenario.agent_does(DIAGNOSE_AT_NAMED_MERGE_BASE);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        result.stdout, "https://github.com/acme/widgets/pull/1\n",
        "{}",
        result.stderr
    );
    let prompt = scenario.first_prompt();
    assert!(
        prompt.contains("You are continuing work on branch issue-7"),
        "{prompt}"
    );
    assert!(!prompt.contains("Merge base commit:"), "{prompt}");
    assert_eq!(
        scenario.origin_file("issue-7", "behavior.txt").as_deref(),
        Some("fixed\n")
    );
}

#[test]
fn a_continuation_finishes_a_fix_the_branch_does_not_have_yet() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.origin_has_commit("main", "behavior.txt", "broken\n", "Bug is present");
    scenario.origin_has_branch("issue-7", "main", &[]);
    scenario.origin_has_commit("issue-7", "notes.txt", "not a fix\n", "Earlier work");
    scenario.agent_does(DIAGNOSE_AT_NAMED_MERGE_BASE);

    let result = scenario.run(&[&scenario.issue_url(7)]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        result.stdout, "https://github.com/acme/widgets/pull/1\n",
        "{}",
        result.stderr
    );
    let prompt = scenario.first_prompt();
    let merge_base = origin_merge_base(&scenario, "main", "issue-7");
    assert!(
        prompt.contains(&format!("Merge base commit: `{merge_base}`.")),
        "{prompt}"
    );
    assert_eq!(
        scenario.origin_file("issue-7", "behavior.txt").as_deref(),
        Some("fixed\n")
    );
    assert_eq!(scenario.origin_log("issue-7").unwrap()[0], "Fix bug");
}
