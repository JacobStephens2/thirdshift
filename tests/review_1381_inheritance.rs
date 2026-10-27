mod support;

use std::fs;
use support::Scenario;

#[test]
fn omitted_step_values_inherit_the_resolved_command_config() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        r#"
[harness]
default = "codex"
[harness.codex]
model = "gpt-6.1-sol"
effort = "high"
[harness.claude]
model = "opus"
effort = "low"
"#,
    );
    let dir = scenario.path("home/.thirdshift/processes");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("inherit.toml"),
        r#"
version = 1
description = "Inherit resolved Command values"
[[steps]]
name = "command"
skill = "to-spec"
[[steps]]
name = "other"
skill = "code-review"
harness = "claude"
"#,
    )
    .unwrap();
    let result = scenario.run(&["process", "check", "inherit"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let other = result.stdout.split_once("other: ").unwrap().1;
    assert!(
        other.contains("harness: claude (Step)"),
        "{}",
        result.stdout
    );
    assert!(
        other.contains("model: gpt-6.1-sol (User config)"),
        "{}",
        result.stdout
    );
    assert!(
        other.contains("effort: high (User config)"),
        "{}",
        result.stdout
    );
    assert!(
        scenario
            .claude_calls()
            .iter()
            .all(|call| call["stdin"].is_string()),
        "a Process Step session ran"
    );
    assert!(scenario.codex_calls().is_empty());
}
