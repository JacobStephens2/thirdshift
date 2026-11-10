use super::*;

#[test]
fn built_in_ticket_steps_run_locally_without_publication() {
    let scenario = Scenario::new();
    let request = prepare_local_process(&scenario);
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
        output["outcome"], "delivered",
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

fn prepare_local_process(scenario: &Scenario) -> PathBuf {
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
    let request = request(scenario);
    let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    document["session_kind"] = json!("process");
    document["inputs"]["pr_url"] = json!("");
    fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
    request
}

fn security_review_succeeds() -> &'static str {
    r#"
test "$(cat .thirdshift-issue.md)" = '# Fixture Spec

Review the fixture change.'
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
test -n "$report"
printf '[]\n' > "$report"
printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#
}

#[test]
fn custom_process_sessions_and_commands_use_the_frozen_fresh_home_in_the_sandbox() {
    if !bwrap_present() {
        println!("SKIPPED (Custom process fresh home): bwrap is not on PATH");
        return;
    }
    let scenario = Scenario::new();
    let request = prepare_local_process(&scenario);
    let home = scenario.path("frozen-home");
    let skill = home.join(".thirdshift/skills/fixture");
    fs::create_dir_all(&skill).unwrap();
    fs::write(skill.join("SKILL.md"), "Frozen instructions\n").unwrap();
    let process = home.join(".thirdshift/processes/frozen.toml");
    fs::create_dir_all(process.parent().unwrap()).unwrap();
    fs::write(&process, r#"version = 1
description = "Frozen Custom process"
[worktrees.ticket]
kind = "issue-branch"
[[steps]]
name = "custom"
skill = "fixture"
worktree = "ticket"
[[steps]]
name = "command"
command = 'test -f "$HOME/.thirdshift/processes/frozen.toml" && test "$THIRDSHIFT_ISSUE_URL" = "https://github.com/acme/widgets/issues/7" && test -n "$THIRDSHIFT_STEP_RESULT" && echo command-complete'
worktree = "ticket"
[[steps]]
name = "delivery"
catalogue = "delivery"
worktree = "ticket"
"#).unwrap();
    let mut document: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    document["process"] = json!(process);
    fs::write(&request, serde_json::to_vec(&document).unwrap()).unwrap();
    scenario.agent_does(
        r#"
test "$(cat .claude/skills/fixture/SKILL.md)" = 'Frozen instructions'
test -f "$HOME/.thirdshift/processes/frozen.toml"
echo implemented > feature.txt
git add feature.txt
git commit -q -m 'Custom implementation'
printf 'Complete.\n' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = run_trial(
        &scenario,
        &request,
        &[("THIRDSHIFT_TRIAL_HOME", home.to_str().unwrap())],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let output: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(
        output["outcome"], "delivered",
        "{output}\n{}",
        result.stderr
    );
    assert_eq!(output["steps"].as_array().unwrap().len(), 2);
    assert_eq!(
        fs::read_to_string(scenario.path("trial-output/steps/command.txt")).unwrap(),
        "command-complete\n"
    );
    assert!(scenario.gh_calls().is_empty());
}

#[test]
fn local_issue_restore_does_not_follow_a_symlink_outside_the_repository() {
    let scenario = Scenario::new();
    let request = prepare_local_process(&scenario);
    let sentinel = scenario.path("outside-sentinel");
    fs::write(&sentinel, "KEEP\n").unwrap();
    let sentinel = sentinel.display().to_string();
    scenario.agent_does_in_session(
        1,
        &format!(
            r#"
rm -f .thirdshift-issue.md
ln -s '{sentinel}' .thirdshift-issue.md
echo implemented > feature.txt
git add feature.txt
git commit -q -m 'Implement local Ticket'
"#
        ),
    );
    scenario.agent_does_in_session(2, security_review_succeeds());

    let result = run_trial(&scenario, &request, &[]);

    assert_eq!(
        fs::read_to_string(scenario.path("outside-sentinel")).unwrap(),
        "KEEP\n",
        "issue restore wrote through the session's symlink\n{}",
        result.stderr
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let issue = scenario.launch_dir().join(".thirdshift-issue.md");
    assert!(
        fs::symlink_metadata(&issue).unwrap().file_type().is_file(),
        "issue file stayed a symlink"
    );
    assert_eq!(
        fs::read_to_string(issue).unwrap(),
        "# Fixture Spec\n\nReview the fixture change.\n"
    );
}

#[test]
fn local_security_review_pins_the_base_commit_sampled_before_sessions() {
    let scenario = Scenario::new();
    let request = prepare_local_process(&scenario);
    let base = scenario
        .launch_git(&["rev-parse", "HEAD"])
        .trim()
        .to_string();
    scenario.agent_does_in_session(
        1,
        r#"
echo implemented > feature.txt
git add feature.txt
git commit -q -m 'Implement local Ticket'
git update-ref refs/heads/main HEAD
"#,
    );
    scenario.agent_does_in_session(2, security_review_succeeds());

    let result = run_trial(&scenario, &request, &[]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = trial_calls(&scenario, "claude-calls.json");
    assert_eq!(calls.len(), 2, "{}", result.stderr);
    let prompt = calls[1]["prompt"].as_str().unwrap();
    let head = scenario
        .launch_git(&["rev-parse", "HEAD"])
        .trim()
        .to_string();
    assert_ne!(base, head);
    assert!(
        prompt.contains(&format!("git diff {base}...HEAD")),
        "security review did not pin the pre-session base {base}; head is {head}\n{prompt}"
    );
    assert!(
        !prompt.contains(&format!("Merge base commit: `{head}`")),
        "security review used the moved base ref {head}\n{prompt}"
    );
}
