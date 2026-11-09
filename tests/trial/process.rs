use super::*;

#[test]
fn built_in_ticket_steps_run_locally_without_publication() {
    let scenario = Scenario::new();
    scenario.launch_git(&[
        "remote",
        "set-url",
        "origin",
        scenario.origin_dir().to_str().unwrap(),
    ]);
    scenario.launch_git(&["checkout", "-b", "issue-7"]);
    // The Trial's fresh home does not carry the host's Git identity.
    scenario.launch_git(&["config", "user.name", "Eval"]);
    scenario.launch_git(&["config", "user.email", "eval@example.invalid"]);
    let request = request(&scenario);
    let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    document["session_kind"] = json!("process");
    document["inputs"]["pr_url"] = json!("");
    fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
    scenario.agent_does_in_session(
        1,
        r#"
test "$(cat .thirdshift-issue.md)" = '# Fixture Spec

Review the fixture change.'
echo implemented > feature.txt
git add feature.txt
git commit -q -m 'Implement local Ticket'
echo tampered > .thirdshift-issue.md
"#,
    );
    scenario.agent_does_in_session(2, r#"
test "$(cat .thirdshift-issue.md)" = '# Fixture Spec

Review the fixture change.'
test "$(git diff main...HEAD -- feature.txt)" != ''
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
test -n "$report"
printf '[]\n' > "$report"
printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = run_trial(&scenario, &request, &[]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(
        output["outcome"], "succeeded",
        "{output}\n{}",
        result.stderr
    );
    let calls = trial_calls(&scenario, "claude-calls.json");
    assert_eq!(calls.len(), 2, "{}", result.stderr);
    for call in &calls {
        let prompt = call["prompt"].as_str().unwrap();
        assert!(prompt.contains("There is no pull request"), "{prompt}");
        assert!(!prompt.contains("Push branch"), "{prompt}");
        assert!(
            !prompt.contains("Update this branch's pull request"),
            "{prompt}"
        );
    }
    assert!(calls[1]["prompt"].as_str().unwrap().contains("git diff"));
    assert!(scenario.gh_calls().is_empty(), "{:?}", scenario.gh_calls());
    assert_eq!(scenario.gh_state()["prs"], json!([]));
    assert!(!scenario.path("home/.thirdshift/logs").exists());
    assert_eq!(
        scenario.launch_git(&["log", "-1", "--format=%s"]),
        "Implement local Ticket\n"
    );
    assert!(scenario.origin_file("issue-7", "feature.txt").is_none());
    assert!(
        result
            .stderr
            .contains("stopped before publishing Step delivery"),
        "{}",
        result.stderr
    );
}
