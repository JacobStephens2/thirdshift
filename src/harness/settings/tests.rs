//! Behavior through the complete interaction, in an isolated environment.

use super::*;
use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[path = "../../../tests/support/fakes.rs"]
#[allow(dead_code)]
mod fakes;

const CHILD: &str = "THIRDSHIFT_SETTINGS_TEST";

#[derive(Default)]
struct Scripted {
    answers: VecDeque<(&'static str, &'static str)>,
    prompts: Vec<String>,
    said: Vec<String>,
}

impl Terminal for Scripted {
    fn read(&mut self, prompt: &str) -> Result<Option<String>> {
        self.prompts.push(prompt.to_string());
        Ok(self.answers.pop_front().map(|(expected, answer)| {
            assert!(
                prompt.contains(expected),
                "asked {prompt:?}, expected {expected:?}"
            );
            answer.trim().to_string()
        }))
    }

    fn say(&mut self, line: String) {
        self.said.push(line);
    }
}

impl Scripted {
    fn answering(answers: &[(&'static str, &'static str)]) -> Self {
        Self {
            answers: answers.iter().copied().collect(),
            ..Self::default()
        }
    }
}

/// Only the child receives a different environment. No ambient PATH fallback
/// can accidentally discover a real Harness, even for subset installation.
fn isolated(name: &str, installed: &[Harness]) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return false;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    fakes::install_named(
        &bin,
        &installed
            .iter()
            .map(|harness| harness.name())
            .collect::<Vec<_>>(),
    );
    for utility in ["git", "bash"] {
        let path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|dir| dir.join(utility))
            .find(|path| path.is_file())
            .unwrap();
        std::os::unix::fs::symlink(path, bin.join(utility)).unwrap();
    }
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    fs::write(root.join("check.sh"), "exit 0\n").unwrap();
    fs::write(root.join("claude.sh"), "exit 0\n").unwrap();
    fs::write(
        root.join("grok-models.txt"),
        include_str!("../../../tests/fixtures/grok-models.txt"),
    )
    .unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("harness::settings::tests::{name}"),
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, root)
        .env("PATH", &bin)
        .env("HOME", &home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("GROK_HOME", home.join(".grok"))
        .env("FAKE_CHECK_SCRIPT", root.join("check.sh"))
        .env("FAKE_CHECK_PIDS", root.join("check-pids"))
        .env("FAKE_CLAUDE_SCRIPT", root.join("claude.sh"))
        .env("FAKE_GROK_MODELS", root.join("grok-models.txt"))
        .current_dir(root);
    for variable in [
        "FAKE_CLAUDE_RECORD",
        "FAKE_AGY_CHECK_RECORD",
        "FAKE_GROK_RECORD",
        "FAKE_MUSE_RECORD",
        "FAKE_OPENCODE_RECORD",
    ] {
        command.env(variable, root.join(variable));
    }
    let result = command.output().unwrap();
    assert!(result.status.success(), "{result:?}");
    true
}

fn root() -> PathBuf {
    std::env::var_os(CHILD).unwrap().into()
}

#[test]
fn registered_order_and_enter_defaults_produce_one_complete_answer() {
    if isolated(
        "registered_order_and_enter_defaults_produce_one_complete_answer",
        &Harness::ALL,
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", ""), ("Effort", "")]);

    assert_eq!(
        ask(&mut terminal, &Settings::default()).unwrap(),
        Some((Harness::Claude, ModelAndEffort::default()))
    );

    assert_eq!(
        terminal.prompts,
        [
            "Harness for every Run's sessions, claude or codex or agy or grok or muse or opencode [claude]: ",
            "Model for claude [claude's own default]: ",
            "Effort for claude [claude's own default]: ",
        ]
    );
    assert!(terminal.answers.is_empty());
    assert!(!root().join("FAKE_CLAUDE_RECORD").exists());
}

#[test]
fn no_installed_harness_retains_all_settings_without_questions() {
    if isolated(
        "no_installed_harness_retains_all_settings_without_questions",
        &[],
    ) {
        return;
    }
    let current = Settings {
        default: Some(Harness::Codex),
        codex: ModelAndEffort {
            model: Some("gpt-5.5".into()),
            effort: Some("high".into()),
        },
        ..Settings::default()
    };
    let before = current.clone();
    let mut terminal = Scripted::default();

    assert_eq!(ask(&mut terminal, &current).unwrap(), None);

    assert_eq!(current, before);
    assert!(terminal.prompts.is_empty());
    assert_eq!(
        terminal.said,
        [
            "No Harness is installed here, so the harness settings stay as they are; install claude or codex or agy or grok or muse or opencode, then rerun `thirdshift setup`."
        ]
    );
}

fn model_and_effort(model: Option<&str>, effort: Option<&str>) -> ModelAndEffort {
    ModelAndEffort {
        model: model.map(String::from),
        effort: effort.map(String::from),
    }
}

fn chosen(terminal: &mut Scripted, current: &Settings) -> (Harness, ModelAndEffort) {
    let answer = ask(terminal, current).unwrap().unwrap();
    assert!(
        terminal.answers.is_empty(),
        "unasked: {:?}",
        terminal.answers
    );
    answer
}

fn records(variable: &str) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(std::env::var_os(variable).unwrap()).unwrap()).unwrap()
}

#[test]
fn configured_default_is_preferred_when_installed() {
    if isolated(
        "configured_default_is_preferred_when_installed",
        &[Harness::Claude, Harness::Codex],
    ) {
        return;
    }
    let current = Settings {
        default: Some(Harness::Codex),
        ..Settings::default()
    };
    let mut terminal =
        Scripted::answering(&[("Harness", ""), ("Model for codex", ""), ("Effort", "")]);

    assert_eq!(
        chosen(&mut terminal, &current),
        (Harness::Codex, ModelAndEffort::default())
    );

    assert_eq!(
        terminal.prompts[0],
        "Harness for every Run's sessions, claude or codex [codex]: "
    );
}

#[test]
fn an_uninstalled_default_falls_back_to_the_first_registered_installed_harness() {
    if isolated(
        "an_uninstalled_default_falls_back_to_the_first_registered_installed_harness",
        &[Harness::Agy, Harness::Codex],
    ) {
        return;
    }
    let current = Settings {
        default: Some(Harness::Muse),
        ..Settings::default()
    };
    let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", ""), ("Effort", "")]);

    assert_eq!(chosen(&mut terminal, &current).0, Harness::Codex);

    assert_eq!(
        terminal.prompts[0],
        "Harness for every Run's sessions, codex or agy [codex]: "
    );
}

#[test]
fn unknown_or_uninstalled_harnesses_are_refused_and_only_selection_is_reasked() {
    if isolated(
        "unknown_or_uninstalled_harnesses_are_refused_and_only_selection_is_reasked",
        &[Harness::Codex],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[
        ("Harness", "claude"),
        ("Harness", "gemini"),
        ("Harness", "codex"),
        ("Model", ""),
        ("Effort", ""),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).0,
        Harness::Codex
    );

    assert_eq!(
        terminal.prompts[0],
        "Harness for every Run's sessions, codex [codex]: "
    );
    assert_eq!(
        terminal.said[..2],
        [
            "claude is not installed: it isn't on PATH.",
            "Choose codex."
        ]
    );
}

#[test]
fn claude_checks_the_named_model_and_effort_with_the_existing_stdin_prompt() {
    if isolated(
        "claude_checks_the_named_model_and_effort_with_the_existing_stdin_prompt",
        &[Harness::Claude],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", "claude-opus-5-5"),
        ("Effort", "high"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()),
        (
            Harness::Claude,
            model_and_effort(Some("claude-opus-5-5"), Some("high"))
        )
    );

    let calls = records("FAKE_CLAUDE_RECORD");
    assert_eq!(calls.as_array().unwrap().len(), 1);
    assert_eq!(
        calls[0]["argv"],
        serde_json::json!(["-p", "--model", "claude-opus-5-5", "--effort", "high"])
    );
    assert_eq!(calls[0]["stdin"], "Reply with OK.");
}

#[test]
fn claude_effort_alone_makes_no_test_call() {
    if isolated("claude_effort_alone_makes_no_test_call", &[Harness::Claude]) {
        return;
    }
    let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", ""), ("Effort", "high")]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(None, Some("high"))
    );

    assert!(!root().join("FAKE_CLAUDE_RECORD").exists());
}

#[test]
fn a_claude_refusal_reports_the_error_and_reasks_the_model_and_effort_pair() {
    if isolated(
        "a_claude_refusal_reports_the_error_and_reasks_the_model_and_effort_pair",
        &[Harness::Claude],
    ) {
        return;
    }
    fs::write(root().join("claude.sh"), "if [[ ! -f refused ]]; then\n : > refused\n echo \"There's an issue with the selected model (Opus 5.5).\" >&2\n exit 1\nfi\n").unwrap();
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", "Opus 5.5"),
        ("Effort", "low"),
        ("Model", "opus"),
        ("Effort", "high"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(Some("opus"), Some("high"))
    );

    assert_eq!(
        terminal.said,
        [
            "claude refused a test call on the Model Opus 5.5 with the Effort low: There's an issue with the selected model (Opus 5.5)."
        ]
    );
    let calls = records("FAKE_CLAUDE_RECORD");
    assert_eq!(calls.as_array().unwrap().len(), 2);
    assert_eq!(
        calls[1]["argv"],
        serde_json::json!(["-p", "--model", "opus", "--effort", "high"])
    );
}

#[test]
fn enter_keeps_and_dash_clears_current_model_and_effort_for_every_harness() {
    if isolated(
        "enter_keeps_and_dash_clears_current_model_and_effort_for_every_harness",
        &Harness::ALL,
    ) {
        return;
    }
    for (harness, model, effort) in [
        (Harness::Claude, "opus", "high"),
        (Harness::Codex, "gpt-6.1-sol", "max"),
        (Harness::Agy, "gemini-3.8-flash", "medium"),
        (Harness::Grok, "grok-4.5", "high"),
        (Harness::Muse, "muse-spark-1.3", "high"),
        (Harness::OpenCode, "provider/model", "high"),
    ] {
        let mut current = Settings {
            default: Some(harness),
            ..Settings::default()
        };
        *current.of_mut(harness) = model_and_effort(Some(model), Some(effort));
        let original = current.clone();
        let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", ""), ("Effort", "")]);

        assert_eq!(
            chosen(&mut terminal, &current),
            (harness, model_and_effort(Some(model), Some(effort)))
        );

        assert_eq!(
            terminal.prompts[1],
            format!(
                "Model for {}, - for {}'s own default [{model}]: ",
                harness.name(),
                harness.name()
            )
        );
        assert_eq!(
            terminal.prompts[2],
            format!(
                "Effort for {}, - for {}'s own default [{effort}]: ",
                harness.name(),
                harness.name()
            )
        );
        let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", "-"), ("Effort", "-")]);
        assert_eq!(
            chosen(&mut terminal, &current),
            (harness, ModelAndEffort::default())
        );
        assert_eq!(
            current, original,
            "the interaction mutated current settings"
        );
    }
}

#[test]
fn codex_lists_models_and_the_selected_models_supported_efforts() {
    if isolated(
        "codex_lists_models_and_the_selected_models_supported_efforts",
        &[Harness::Codex],
    ) {
        return;
    }
    let mut terminal =
        Scripted::answering(&[("Harness", ""), ("Model", "gpt-5.5"), ("Effort", "high")]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(Some("gpt-5.5"), Some("high"))
    );

    assert_eq!(
        terminal.said,
        [
            "Codex's Models: gpt-6.1-sol (GPT-6.1-Sol), gpt-6-luna (GPT-6-Luna), gpt-5.5 (GPT-5.5)",
            "Efforts gpt-5.5 supports: low, medium, high, xhigh",
        ]
    );
    assert_eq!(
        terminal.prompts[1..],
        [
            "Model for codex [codex's own default]: ",
            "Effort for codex [codex's own default]: "
        ]
    );
}

#[test]
fn codex_without_a_model_lists_the_union_of_efforts_and_keeps_model_unset() {
    if isolated(
        "codex_without_a_model_lists_the_union_of_efforts_and_keeps_model_unset",
        &[Harness::Codex],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", ""), ("Effort", "Ultra")]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(None, Some("ultra"))
    );

    assert_eq!(
        terminal.said[1],
        "Efforts Codex's Models support: low, medium, high, xhigh, max, ultra"
    );
}

#[test]
fn codex_matches_display_names_and_efforts_without_case_sensitivity() {
    if isolated(
        "codex_matches_display_names_and_efforts_without_case_sensitivity",
        &[Harness::Codex],
    ) {
        return;
    }
    let current = Settings {
        codex: model_and_effort(Some("GPT-6-Luna"), Some("High")),
        ..Settings::default()
    };
    let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", ""), ("Effort", "")]);

    assert_eq!(
        chosen(&mut terminal, &current).1,
        model_and_effort(Some("gpt-6-luna"), Some("high"))
    );

    assert_eq!(
        terminal.prompts[1..],
        [
            "Model for codex, - for codex's own default [GPT-6-Luna]: ",
            "Effort for codex, - for codex's own default [High]: ",
        ]
    );
    assert_eq!(
        terminal.said[1],
        "Efforts gpt-6-luna supports: low, medium, high, xhigh, max"
    );
}

#[test]
fn codex_refuses_with_valid_choices_and_retries_only_the_invalid_setting() {
    if isolated(
        "codex_refuses_with_valid_choices_and_retries_only_the_invalid_setting",
        &[Harness::Codex],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", "gpt-7"),
        ("Model", "GPT-5.5"),
        ("Effort", "max"),
        ("Effort", "XHigh"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(Some("gpt-5.5"), Some("xhigh"))
    );

    assert_eq!(
        terminal.said[1],
        "the Model gpt-7 is not in Codex's catalog: choose one of gpt-6.1-sol, gpt-6-luna, gpt-5.5"
    );
    assert_eq!(
        terminal.said[3],
        "the Effort max is not one the Codex Model gpt-5.5 supports: choose one of low, medium, high, xhigh"
    );
}

#[test]
fn agy_accepts_a_bare_alias_before_its_required_effort_and_retries_only_that_effort() {
    if isolated(
        "agy_accepts_a_bare_alias_before_its_required_effort_and_retries_only_that_effort",
        &[Harness::Agy],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", "gemini-99"),
        ("Model", "Gemini-3.8-Flash"),
        ("Effort", ""),
        ("Effort", "Max"),
        ("Effort", "Medium"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(Some("gemini-3.8-flash"), Some("medium"))
    );

    assert_eq!(
        terminal.prompts[0],
        "Harness for every Run's sessions, agy [agy]: "
    );
    let said = terminal.said.join("\n");
    assert!(said.contains("the Model gemini-99 is not in"), "{said}");
    assert!(said.contains("the Effort Max is not one"), "{said}");
    assert!(said.contains("requires an Effort"), "{said}");
    let calls = records("FAKE_AGY_CHECK_RECORD");
    assert_eq!(calls.as_array().unwrap().len(), 1);
    assert_eq!(calls[0]["argv"], serde_json::json!(["models"]));
    assert_eq!(calls[0]["auto_update"], "true");
}

#[test]
fn agy_accepts_model_ids_and_labels_without_requiring_an_extra_effort() {
    if isolated(
        "agy_accepts_model_ids_and_labels_without_requiring_an_extra_effort",
        &[Harness::Agy],
    ) {
        return;
    }
    for name in ["GEMINI-3.8-FLASH-HIGH", "Gemini 3.8 Flash (High)"] {
        let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", name), ("Effort", "")]);
        assert_eq!(
            chosen(&mut terminal, &Settings::default()).1,
            model_and_effort(Some("gemini-3.8-flash-high"), None)
        );
    }
}

#[test]
fn grok_retries_only_the_invalid_setting_and_reports_its_valid_choices() {
    if isolated(
        "grok_retries_only_the_invalid_setting_and_reports_its_valid_choices",
        &[Harness::Grok],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", "bogus"),
        ("Model", "GROK-4.5"),
        ("Effort", "xhigh"),
        ("Effort", "High"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(Some("grok-4.5"), Some("high"))
    );

    let said = terminal.said.join("\n");
    assert!(
        said.contains("choose one of grok-4.7, grok-4.7-build-fast, grok-4.6, grok-4.5"),
        "{said}"
    );
    assert!(said.contains("choose one of high, medium, low"), "{said}");
    let calls = records("FAKE_GROK_RECORD");
    assert_eq!(calls.as_array().unwrap().len(), 1);
    assert_eq!(calls[0]["argv"], serde_json::json!(["models"]));
    assert_eq!(calls[0]["GROK_DISABLE_AUTOUPDATER"], "1");
    assert_eq!(calls[0]["GROK_FOLDER_TRUST"], "0");
}

#[test]
fn grok_validates_an_omitted_models_effort_against_its_known_default() {
    if isolated(
        "grok_validates_an_omitted_models_effort_against_its_known_default",
        &[Harness::Grok],
    ) {
        return;
    }
    fs::write(
        root().join("grok-models.txt"),
        include_str!("../../../tests/fixtures/grok-models.txt")
            .replace("Default model: grok-4.7", "Default model: grok-4.5"),
    )
    .unwrap();
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", ""),
        ("Effort", "xhigh"),
        ("Effort", "Medium"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(None, Some("medium"))
    );

    assert!(terminal.said[1].starts_with("Efforts Grok Build's default Model supports:"));
    assert!(
        terminal.said[2].contains("choose one of high, medium, low"),
        "{:?}",
        terminal.said
    );
}

#[test]
fn muse_suggests_its_model_canonicalizes_effort_and_reasks_a_refused_pair() {
    if isolated(
        "muse_suggests_its_model_canonicalizes_effort_and_reasks_a_refused_pair",
        &[Harness::Muse],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", "bad-model"),
        ("Effort", "Low"),
        ("Model", ""),
        ("Effort", "High"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(Some("muse-spark-1.3"), Some("high"))
    );

    assert_eq!(
        terminal.prompts[1],
        "Model for muse, - for muse's own default [muse-spark-1.3]: "
    );
    assert!(terminal.said[0].contains("model does not exist or you lack access"));
    let checks: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            root()
                .join("FAKE_MUSE_RECORD")
                .with_extension("checks.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(checks.as_array().unwrap().len(), 2);
    assert_eq!(
        checks[0]["argv"],
        serde_json::json!([
            "exec",
            "--json",
            "--yolo",
            "--model",
            "bad-model",
            "--reasoning-effort",
            "low",
            "Reply with OK."
        ])
    );
    assert_eq!(checks[0]["stdin_null"], true);
    assert_eq!(checks[0]["no_auto_update"], "1");
}

#[test]
fn muse_uses_its_cached_catalog_and_checks_effort_before_any_model_call() {
    if isolated(
        "muse_uses_its_cached_catalog_and_checks_effort_before_any_model_call",
        &[Harness::Muse],
    ) {
        return;
    }
    let cache = root().join("home/.local/share/muse/model-catalog");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("models.json"), r#"{"rows":[{"model_id":"muse-spark-1.3","display_label":"Muse Spark 1.3","visibility":"visible"}]}"#).unwrap();
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", "bad-model"),
        ("Effort", "bogus"),
        ("Model", "not-a-model"),
        ("Effort", "High"),
        ("Model", "Muse Spark 1.3"),
        ("Effort", "Max"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(Some("muse-spark-1.3"), Some("max"))
    );

    assert!(terminal.said[0].contains("Effort"));
    assert!(terminal.said[1].contains("not in Muse's cached catalog"));
    assert!(
        !root()
            .join("FAKE_MUSE_RECORD")
            .with_extension("checks.json")
            .exists()
    );
}

#[test]
fn muse_with_model_cleared_does_not_make_a_model_call() {
    if isolated(
        "muse_with_model_cleared_does_not_make_a_model_call",
        &[Harness::Muse],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[("Harness", ""), ("Model", "-"), ("Effort", "High")]);
    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(None, Some("high"))
    );
    assert!(
        !root()
            .join("FAKE_MUSE_RECORD")
            .with_extension("checks.json")
            .exists()
    );
}

#[test]
fn opencode_requires_a_model_for_effort_and_reasks_refused_pairs_using_standalone_checks() {
    if isolated(
        "opencode_requires_a_model_for_effort_and_reasks_refused_pairs_using_standalone_checks",
        &[Harness::OpenCode],
    ) {
        return;
    }
    let mut terminal = Scripted::answering(&[
        ("Harness", ""),
        ("Model", ""),
        ("Effort", "high"),
        ("Model", "bad-model"),
        ("Effort", "bogus"),
        ("Model", "provider/model"),
        ("Effort", "high"),
    ]);

    assert_eq!(
        chosen(&mut terminal, &Settings::default()).1,
        model_and_effort(Some("provider/model"), Some("high"))
    );

    assert_eq!(
        terminal.said[0],
        "the OpenCode Effort needs a Model: set model <provider>/<model> too"
    );
    assert!(terminal.said[1].contains("Model or variant unavailable"));
    let checks: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            root()
                .join("FAKE_OPENCODE_RECORD")
                .with_extension("checks.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(checks.as_array().unwrap().len(), 2);
    assert_eq!(
        checks[1]["argv"],
        serde_json::json!([
            "run",
            "--standalone",
            "--format",
            "json",
            "--auto",
            "-m",
            "provider/model#high"
        ])
    );
    assert_eq!(checks[1]["prompt"], "Reply with OK.");
    assert_eq!(checks[1]["no_auto_update"], "1");
    let exports: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            root()
                .join("FAKE_OPENCODE_RECORD")
                .with_extension("exports.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(exports.as_array().unwrap().len(), 1);
    assert_eq!(
        exports[0]["argv"],
        serde_json::json!(["session", "export", "--standalone", "check-session"])
    );
}

#[test]
fn recoverable_catalog_failures_retain_every_setting_after_selecting_another_harness() {
    if isolated(
        "recoverable_catalog_failures_retain_every_setting_after_selecting_another_harness",
        &Harness::ALL,
    ) {
        return;
    }
    fs::write(
        root().join("check.sh"),
        "echo 'not signed in' >&2\nexit 1\n",
    )
    .unwrap();
    let mut current = Settings {
        default: Some(Harness::Claude),
        ..Settings::default()
    };
    for harness in Harness::ALL {
        *current.of_mut(harness) =
            model_and_effort(Some("previous-model"), Some("previous-effort"));
    }
    let before = current.clone();
    for (harness, recovery) in [
        ("codex", "once codex debug models works"),
        ("agy", "once agy models works"),
        ("grok", "once grok models works"),
    ] {
        let mut terminal = Scripted::answering(&[("Harness", harness)]);

        assert_eq!(ask(&mut terminal, &current).unwrap(), None);

        assert!(terminal.answers.is_empty());
        assert_eq!(terminal.prompts.len(), 1);
        assert!(terminal.said[0].contains("not signed in"));
        assert!(terminal.said[0].contains(recovery));
        assert_eq!(current, before);
    }
}

#[test]
fn eof_at_selection_model_or_effort_propagates_the_existing_error() {
    if isolated(
        "eof_at_selection_model_or_effort_propagates_the_existing_error",
        &Harness::ALL,
    ) {
        return;
    }
    for harness in Harness::ALL {
        for answers in [
            vec![],
            vec![("Harness", harness.name())],
            vec![("Harness", harness.name()), ("Model", "")],
        ] {
            let mut terminal = Scripted::answering(&answers);
            assert_eq!(
                ask(&mut terminal, &Settings::default())
                    .unwrap_err()
                    .to_string(),
                "Setup ended before its last answer; nothing written"
            );
        }
    }
}

#[test]
fn terminal_read_failure_propagates_and_cannot_become_retention() {
    if isolated(
        "terminal_read_failure_propagates_and_cannot_become_retention",
        &[Harness::Claude],
    ) {
        return;
    }
    struct Broken;
    impl Terminal for Broken {
        fn read(&mut self, _prompt: &str) -> Result<Option<String>> {
            anyhow::bail!("terminal read failed")
        }
        fn say(&mut self, _line: String) {
            panic!("read failure became retention or retry")
        }
    }
    assert_eq!(
        ask(&mut Broken, &Settings::default())
            .unwrap_err()
            .to_string(),
        "terminal read failed"
    );
}

#[test]
fn recorded_interruption_precedes_selection_and_retention() {
    if isolated(
        "recorded_interruption_precedes_selection_and_retention",
        &[],
    ) {
        return;
    }
    crate::interrupt::install().unwrap();
    signal_hook::low_level::raise(libc::SIGINT).unwrap();
    let mut terminal = Scripted::default();
    assert_eq!(
        ask(&mut terminal, &Settings::default())
            .unwrap_err()
            .to_string(),
        "interrupted"
    );
    assert!(terminal.prompts.is_empty());
    assert!(terminal.said.is_empty());
}

#[test]
fn an_interrupted_catalog_read_is_an_error_instead_of_retention() {
    if isolated(
        "an_interrupted_catalog_read_is_an_error_instead_of_retention",
        &[Harness::Codex],
    ) {
        return;
    }
    interrupt_the_check();
    let mut terminal = Scripted::answering(&[("Harness", "")]);
    assert_eq!(
        ask(&mut terminal, &Settings::default())
            .unwrap_err()
            .to_string(),
        "interrupted"
    );
    assert_eq!(terminal.prompts.len(), 1);
    assert!(terminal.said.is_empty());
}

#[test]
fn an_interrupted_model_check_is_an_error_instead_of_retrying_the_pair() {
    if isolated(
        "an_interrupted_model_check_is_an_error_instead_of_retrying_the_pair",
        &[Harness::Claude],
    ) {
        return;
    }
    interrupt_the_check();
    let mut terminal =
        Scripted::answering(&[("Harness", ""), ("Model", "opus"), ("Effort", "high")]);
    assert_eq!(
        ask(&mut terminal, &Settings::default())
            .unwrap_err()
            .to_string(),
        "interrupted"
    );
    assert_eq!(terminal.prompts.len(), 3);
    assert!(terminal.said.is_empty());
    assert_eq!(records("FAKE_CLAUDE_RECORD").as_array().unwrap().len(), 1);
}

fn interrupt_the_check() {
    crate::interrupt::install().unwrap();
    fs::write(
        root().join("check.sh"),
        format!("kill -INT {}\nexit 1\n", std::process::id()),
    )
    .unwrap();
}
