mod support;

use serde_json::Value;
use support::{Scenario, leaves_running};

#[test]
fn resuming_background_work_does_not_count_the_original_review_twice() {
    for (resume, expected) in [
        ("true\n", serde_json::json!({"standards": 2, "spec": 0})),
        (
            r#"
reports=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*`\([^`]*\)\/standards\.md`.*/\1/p')
printf '{"standards":3,"spec":1}\n' > "$reports/summary.json"
"#,
            serde_json::json!({"standards": 3, "spec": 1}),
        ),
    ] {
        let scenario = Scenario::new();
        scenario.agent_does_in_session(
            1,
            &format!(
                r#"
reports=$(printf '%s\n' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*`\([^`]*\)\/standards\.md`.*/\1/p')
printf '{{"standards":2,"spec":0}}\n' > "$reports/summary.json"
echo work > work.txt
git add work.txt
git commit -qm 'Make widgets'
gh pr create --base main --head issue-7 --title 'Make widgets' --body 'Closes #7'
{}
"#,
                leaves_running("cargo test")
            ),
        );
        // A Resume either leaves the review alone or updates its summary.
        scenario.agent_does_in_session(2, resume);
        let ended = scenario.run(&[&scenario.issue_url(7)]);
        assert_eq!(ended.code, Some(0), "{}", ended.stderr);
        assert_eq!(scenario.claude_calls().len(), 2);
        let shown = scenario.run(&["show", "widgets#7", "--json"]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
        let stages = shown["command"]["run_metrics"]["stages"]
            .as_array()
            .unwrap();
        let counts: Vec<&Value> = stages
            .iter()
            .map(|stage| &stage["review_findings"])
            .filter(|counts| !counts.is_null())
            .collect();
        assert_eq!(
            counts,
            vec![&expected],
            "one completed review must contribute its counts once: {stages:#?}"
        );
    }
}
