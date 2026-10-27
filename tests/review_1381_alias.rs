mod support;
use std::fs;
use support::Scenario;

#[test]
fn a_partial_command_step_still_settles_inherited_model_aliases() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[harness]\ndefault = \"muse\"\n[harness.muse]\nmodel = \"Opus 5.5\"\neffort = \"HIGH\"\n",
    );
    let catalog = scenario.path("home/.local/share/muse/model-catalog");
    fs::create_dir_all(&catalog).unwrap();
    fs::write(
        catalog.join("models.json"),
        r#"{"rows":[{"model_id":"claude-opus-5-5","display_label":"Opus 5.5"}]}"#,
    )
    .unwrap();
    let dir = scenario.path("home/.thirdshift/processes");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("partial-alias.toml"),
        r#"
version = 1
description = "Inherit Command model while overriding effort"
[[steps]]
name = "command"
skill = "to-spec"
effort = "low"
[[steps]]
name = "other"
skill = "code-review"
harness = "claude"
"#,
    )
    .unwrap();
    let result = scenario.run(&["process", "check", "partial-alias"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result
            .stdout
            .contains("model: claude-opus-5-5 (User config)\neffort: low (Step)"),
        "{}",
        result.stdout
    );
    let other = result.stdout.split_once("other: ").unwrap().1;
    assert!(
        other.contains("model: claude-opus-5-5 (User config)"),
        "{}",
        result.stdout
    );
}
#[test]
fn omitted_values_inherit_the_command_catalog_resolution() {
    let scenario = Scenario::new();
    scenario.user_config_is(
        "[harness]\ndefault = \"muse\"\n[harness.muse]\nmodel = \"Opus 5.5\"\neffort = \"HIGH\"\n",
    );
    let catalog = scenario.path("home/.local/share/muse/model-catalog");
    fs::create_dir_all(&catalog).unwrap();
    fs::write(
        catalog.join("models.json"),
        r#"{"rows":[{"model_id":"claude-opus-5-5","display_label":"Opus 5.5"}]}"#,
    )
    .unwrap();
    let dir = scenario.path("home/.thirdshift/processes");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("alias.toml"),
        r#"
version = 1
description = "Inherit canonical Command settings"
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
    let result = scenario.run(&["process", "check", "alias"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let other = result.stdout.split_once("other: ").unwrap().1;
    assert!(
        other.contains("model: claude-opus-5-5 (User config)"),
        "{}",
        result.stdout
    );
    assert!(
        other.contains("effort: high (User config)"),
        "{}",
        result.stdout
    );
}
