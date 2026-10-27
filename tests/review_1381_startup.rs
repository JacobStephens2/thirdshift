mod support;
use std::fs;
use support::Scenario;
#[test]
fn later_unavailable_opencode_is_preflighted_before_steps() {
    for check in [false, true] {
        let scenario = Scenario::new();
        let dir = scenario.path("home/.thirdshift/processes");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("startup.toml"),
            r#"
version = 1
description = "Check every Harness before work"
[[steps]]
name = "first"
skill = "to-spec"
[[steps]]
name = "unavailable"
skill = "code-review"
harness = "opencode"
model = "provider/bad-model"
"#,
        )
        .unwrap();
        scenario.agent_does("printf 'done' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
        let args: &[&str] = if check {
            &["process", "check", "startup"]
        } else {
            &["process", "startup"]
        };
        let result = scenario.run(args);
        assert_eq!(
            result.code,
            Some(1),
            "check={check}: {}\n{}",
            result.stdout,
            result.stderr
        );
        assert!(
            result
                .stderr
                .contains("step unavailable's Harness opencode can't start"),
            "{}",
            result.stderr
        );
        assert!(
            scenario.claude_calls().is_empty(),
            "a Process Step session ran"
        );
        assert!(
            !scenario.path("opencode-calls.json").exists(),
            "an OpenCode Process Step session ran"
        );
    }
}
