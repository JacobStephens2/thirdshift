mod support;

use serde_json::Value;
use support::Scenario;

const BLOCKED: &str = "[resources]\nmin_available_memory_mb = 9007199254740991\n";
const DELIVER: &str = r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'
"#;

#[test]
fn resources_reports_host_measurements_and_the_configured_admission_decision() {
    let scenario = Scenario::new();
    scenario.user_config_is(BLOCKED);
    let result = scenario.run(&["resources", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let snapshot: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(snapshot["ready"], false);
    assert_eq!(
        snapshot["limits"]["min_available_memory_mb"],
        9007199254740991_u64
    );
    assert!(snapshot["host"]["total_memory_mb"].as_u64().unwrap() > 0);
    assert!(snapshot["host"]["logical_cpus"].as_u64().unwrap() > 0);
    assert!(snapshot["load_per_cpu_percent"].as_f64().unwrap() >= 0.0);
    assert!(
        snapshot["reasons"][0]
            .as_str()
            .unwrap()
            .contains("available memory")
    );
    let result = scenario.run(&["resources"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("Waiting for host resources"));
    assert!(result.stdout.contains("9007199254740991 MB"));
}

#[test]
fn new_sessions_wait_for_resources_and_can_recover_after_a_config_change() {
    let scenario = Scenario::new();
    scenario.user_config_is(BLOCKED);
    scenario.agent_does(DELIVER);
    let release = |_: u32| {
        assert!(
            scenario.claude_calls().is_empty(),
            "session started while resources were blocked"
        );
        scenario.user_config_is("");
    };
    let result = scenario.run_terminal(
        &[&scenario.issue_url(7)],
        &[],
        &[support::TerminalStep::inspect(
            "waiting for host resources:",
            &release,
        )],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(result.stderr.contains("host resources available"));
}

#[test]
fn a_resume_checks_resources_again_before_starting() {
    let scenario = Scenario::new();
    scenario.user_config_is("");
    scenario.agent_does_in_session(
        1,
        &format!(
            "cat > \"$HOME/.thirdshift/config.toml\" <<'CONFIG'\n{BLOCKED}CONFIG\n{}",
            support::leaves_running("resource fixture work")
        ),
    );
    scenario.agent_does_in_session(2, DELIVER);
    let release = |_: u32| {
        assert_eq!(
            scenario.claude_calls().len(),
            1,
            "Resume started before admission"
        );
        scenario.user_config_is("");
    };
    let result = scenario.run_terminal(
        &[&scenario.issue_url(7)],
        &[],
        &[support::TerminalStep::inspect(
            "waiting for host resources:",
            &release,
        )],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(scenario.claude_calls().len(), 2);
    assert!(result.stderr.contains("resuming it once"));
}

#[test]
fn interrupting_a_resource_wait_starts_no_session() {
    let scenario = Scenario::new();
    scenario.user_config_is(BLOCKED);
    scenario.agent_does(DELIVER);
    let result = scenario.run_on_terminal(
        &[&scenario.issue_url(7)],
        &[],
        &[("waiting for host resources:", support::CTRL_C)],
    );
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(result.stderr.contains("interrupted"));
    assert!(scenario.claude_calls().is_empty());
}
