//! The Harness a Command's agent sessions run on, with its Model and Effort:
//! one [`Choice`] per Command, made by the command, else the User config,
//! else the default, checked before any work, passed on to every session and
//! child Run, and recorded with what the Command did.

use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::progress;

/// The headless agent CLI a Command's sessions run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Codex,
}

impl Harness {
    /// The Harness named `name`, as the command line and the User config
    /// write it, if it is one.
    pub fn named(name: &str) -> Option<Harness> {
        match name {
            "claude" => Some(Harness::Claude),
            "codex" => Some(Harness::Codex),
            _ => None,
        }
    }

    /// Its name, which is also its CLI's.
    pub fn name(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
        }
    }

    /// The signals that ask a session on it to stop, in the order they are
    /// sent: SIGTERM, after SIGINT for Codex, which stops cleanly only on
    /// SIGINT, interrupting its turn.
    pub fn stop_signals(self) -> &'static [libc::c_int] {
        match self {
            Harness::Claude => &[libc::SIGTERM],
            Harness::Codex => &[libc::SIGINT, libc::SIGTERM],
        }
    }

    /// Every Harness, in the order Setup lists them.
    pub const ALL: [Harness; 2] = [Harness::Claude, Harness::Codex];

    /// Whether sessions can run on it here: its CLI is on `PATH`.
    pub fn installed(self) -> bool {
        on_path(self.name())
    }
}

/// The names a Harness is chosen by, for the messages that list them.
pub const NAMES: &str = "claude or codex";

/// A Model and an Effort, each none where it is left to the Harness.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ModelAndEffort {
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// The User config's `[harness]` section: the default Harness, if it names
/// one, and a Model and Effort for each Harness.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `harness.default`.
    pub default: Option<Harness>,
    /// `[harness.claude]`.
    pub claude: ModelAndEffort,
    /// `[harness.codex]`.
    pub codex: ModelAndEffort,
}

impl Settings {
    /// The Model and Effort set for `harness`.
    pub fn of(&self, harness: Harness) -> &ModelAndEffort {
        match harness {
            Harness::Claude => &self.claude,
            Harness::Codex => &self.codex,
        }
    }

    /// The Model and Effort set for `harness`, to set them.
    pub fn of_mut(&mut self, harness: Harness) -> &mut ModelAndEffort {
        match harness {
            Harness::Claude => &mut self.claude,
            Harness::Codex => &mut self.codex,
        }
    }
}

/// What a command asked for, each none if it said nothing about it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Asked {
    /// What `harness <name>` asked for.
    pub harness: Option<Harness>,
    /// What `model <name>` and `effort <level>` asked for.
    pub model_and_effort: ModelAndEffort,
}

/// What chose the Harness, for a failure to name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChosenBy {
    /// The command's `harness` flag, or for a child Run, the command that
    /// started it.
    Command,
    /// The User config's `harness.default`.
    UserConfig,
    /// Neither: Claude Code, the default.
    Default,
}

/// The Harness, Model and Effort every session of a Command runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub harness: Harness,
    /// The Model, or none to leave it to the Harness.
    pub model: Option<String>,
    /// The Effort, or none to leave it to the Harness.
    pub effort: Option<String>,
    pub chosen_by: ChosenBy,
}

impl Default for Choice {
    /// Claude Code, with its own Model and Effort: a Command's sessions with
    /// nothing asked and nothing set.
    fn default() -> Self {
        Choice {
            harness: Harness::Claude,
            model: None,
            effort: None,
            chosen_by: ChosenBy::Default,
        }
    }
}

impl Choice {
    /// What a command that `asked` for it runs on, given the User config's
    /// `settings`: for each of the Harness, Model and Effort, what the
    /// command asked, else what the User config sets, else the default. The
    /// Model and Effort it sets are those of the Harness chosen.
    pub fn of(asked: &Asked, settings: &Settings) -> Choice {
        let (harness, chosen_by) = match (asked.harness, settings.default) {
            (Some(harness), _) => (harness, ChosenBy::Command),
            (None, Some(harness)) => (harness, ChosenBy::UserConfig),
            (None, None) => (Harness::Claude, ChosenBy::Default),
        };
        let set = settings.of(harness);
        let asked = &asked.model_and_effort;
        Choice {
            harness,
            model: asked.model.clone().or_else(|| set.model.clone()),
            effort: asked.effort.clone().or_else(|| set.effort.clone()),
            chosen_by,
        }
    }

    /// The line thirdshift writes in a pull request's body and a Run
    /// notification, as in `Built with claude · claude-opus-5-5 · high`.
    pub fn built_with(&self) -> String {
        format!("Built with {self}")
    }

    /// The arguments that run a Claude session on its Model and Effort,
    /// where they are set.
    pub fn claude_args(&self) -> Vec<&str> {
        let mut args = Vec::new();
        if let Some(model) = &self.model {
            args.extend(["--model", model]);
        }
        if let Some(effort) = &self.effort {
            args.extend(["--effort", effort]);
        }
        args
    }

    /// The arguments that run a Codex session on its Model and Effort, where
    /// they are set.
    pub fn codex_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(model) = &self.model {
            args.extend(["-m".to_string(), model.clone()]);
        }
        if let Some(effort) = &self.effort {
            args.extend([
                "-c".to_string(),
                format!("model_reasoning_effort=\"{effort}\""),
            ]);
        }
        args
    }

    /// Check, before any work, that sessions can run on this, and settle the
    /// Model and Effort on the names the Harness knows them by. The
    /// Harness's CLI must be on `PATH`. A Model named for Claude must take a
    /// minimal test call, with the Effort, if any. A Model or Effort named
    /// for Codex must be in its catalog, which costs no tokens to read: each
    /// is matched regardless of case, a Model by its slug or display name,
    /// and becomes the name Codex takes. Each failure says what to change.
    pub fn check(&mut self) -> Result<()> {
        let cli = self.harness.name();
        if !on_path(cli) {
            bail!(
                "{cli} is not on PATH, and the Harness {cli} is chosen by {}: install it, \
                 or choose another Harness",
                self.chosen_by
            );
        }
        match self.harness {
            Harness::Claude => self.test_call(),
            Harness::Codex => self.check_codex(),
        }
    }

    /// Check that Claude takes a minimal test call on the Model, if one is
    /// named, with the Effort, if any. A refusal says what Claude said.
    pub fn test_call(&self) -> Result<()> {
        let cli = self.harness.name();
        let Some(model) = &self.model else {
            return Ok(());
        };
        progress::step(format_args!(
            "checking the Model {model} with a test call to {cli}"
        ));
        let mut child = Command::new(cli)
            .arg("-p")
            .args(self.claude_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("could not run {cli} to check the Model {model}"))?;
        // Dropped once written, closing stdin, so the call can end.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(TEST_PROMPT.as_bytes());
        }
        let output = child
            .wait_with_output()
            .with_context(|| format!("could not run {cli} to check the Model {model}"))?;
        if output.status.success() {
            return Ok(());
        }
        let effort = match &self.effort {
            Some(effort) => format!(" with the Effort {effort}"),
            None => String::new(),
        };
        bail!(
            "{cli} refused a test call on the Model {model}{effort}: {}",
            said(&output)
        )
    }

    /// Settle the Model and Effort, if either is named, on Codex's names for
    /// them, from its catalog.
    fn check_codex(&mut self) -> Result<()> {
        if self.model.is_none() && self.effort.is_none() {
            return Ok(());
        }
        progress::step("checking the Model and Effort against codex debug models");
        let settled = Catalog::read()?.settle(self.model.as_deref(), self.effort.as_deref())?;
        self.model = settled.model;
        self.effort = settled.effort;
        Ok(())
    }
}

/// What a CLI said on stderr and stdout, trimmed, each on its own lines.
fn said(output: &Output) -> String {
    [&output.stderr, &output.stdout]
        .map(|said| String::from_utf8_lossy(said).trim().to_string())
        .into_iter()
        .filter(|said| !said.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// One Model in Codex's catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogModel {
    /// The name Codex takes.
    slug: String,
    display_name: Option<String>,
    /// The Efforts it supports.
    efforts: Vec<String>,
    /// Whether the catalog lists it to choose from.
    listed: bool,
}

/// The Models Codex can run, as `codex debug models` prints them.
#[derive(Debug)]
pub struct Catalog {
    models: Vec<CatalogModel>,
}

impl Catalog {
    /// The catalog `codex debug models` prints, which costs no tokens to
    /// read.
    pub fn read() -> Result<Catalog> {
        let output = Command::new("codex")
            .args(["debug", "models"])
            .stdin(Stdio::null())
            .output()
            .context("could not run codex debug models to read Codex's Models")?;
        if !output.status.success() {
            bail!(
                "codex debug models failed, so Codex's Models can't be read: {}",
                said(&output)
            );
        }
        Catalog::parse(&String::from_utf8_lossy(&output.stdout))
    }

    /// The catalog `codex debug models` printed as `json`.
    fn parse(json: &str) -> Result<Catalog> {
        let parsed: Value = serde_json::from_str(json)
            .context("codex debug models printed no JSON catalog of Models")?;
        let models = parsed["models"]
            .as_array()
            .context("codex debug models printed no list of Models")?
            .iter()
            .filter_map(|model| {
                Some(CatalogModel {
                    slug: model["slug"].as_str()?.to_string(),
                    display_name: model["display_name"].as_str().map(String::from),
                    efforts: model["supported_reasoning_levels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|level| level["effort"].as_str().map(String::from))
                        .collect(),
                    listed: model["visibility"] != "hide",
                })
            })
            .collect();
        Ok(Catalog { models })
    }

    /// The Models it lists to choose from, each as its slug and, where it
    /// has one, its display name, as in `gpt-6.1-sol (GPT-6.1-Sol)`.
    pub fn listed(&self) -> Vec<String> {
        self.models
            .iter()
            .filter(|model| model.listed)
            .map(|model| match &model.display_name {
                Some(display) => format!("{} ({display})", model.slug),
                None => model.slug.clone(),
            })
            .collect()
    }

    /// The Efforts the Model `slug` supports, or with no Model, those any
    /// Model supports, each once, in the catalog's order.
    pub fn efforts(&self, slug: Option<&str>) -> Vec<&str> {
        let mut efforts = Vec::new();
        let models = self
            .models
            .iter()
            .filter(|model| slug.is_none_or(|slug| model.slug == slug));
        for each in models.flat_map(|model| &model.efforts) {
            if !efforts.contains(&each.as_str()) {
                efforts.push(each.as_str());
            }
        }
        efforts
    }

    /// `model` and `effort` as Codex names them: the slug of the Model whose
    /// slug or display name is `model`, regardless of case, and the Effort,
    /// of those that Model supports, or with no Model of those any Model
    /// supports, that is `effort`, regardless of case. Fails naming the
    /// valid choices for a Model or Effort the catalog doesn't have.
    pub fn settle(&self, model: Option<&str>, effort: Option<&str>) -> Result<ModelAndEffort> {
        let found = match model {
            Some(model) => Some(self.model(model)?),
            None => None,
        };
        let Some(effort) = effort else {
            return Ok(ModelAndEffort {
                model: found.map(|found| found.slug.clone()),
                effort: None,
            });
        };
        let efforts = self.efforts(found.map(|found| found.slug.as_str()));
        let Some(settled) = efforts
            .iter()
            .find(|supported| supported.eq_ignore_ascii_case(effort))
        else {
            let of = match found {
                Some(found) => format!("the Codex Model {}", found.slug),
                None => "any Codex Model".to_string(),
            };
            bail!(
                "the Effort {effort} is not one {of} supports: choose one of {}",
                efforts.join(", ")
            );
        };
        Ok(ModelAndEffort {
            model: found.map(|found| found.slug.clone()),
            effort: Some(settled.to_string()),
        })
    }

    /// The Model whose slug or display name is `name`, regardless of case.
    fn model(&self, name: &str) -> Result<&CatalogModel> {
        let named = |candidate: &&CatalogModel| {
            candidate.slug.eq_ignore_ascii_case(name)
                || candidate
                    .display_name
                    .as_deref()
                    .is_some_and(|display| display.eq_ignore_ascii_case(name))
        };
        if let Some(found) = self.models.iter().find(named) {
            return Ok(found);
        }
        let listed: Vec<&str> = self
            .models
            .iter()
            .filter(|model| model.listed)
            .map(|model| model.slug.as_str())
            .collect();
        bail!(
            "the Model {name} is not in Codex's catalog: choose one of {}",
            listed.join(", ")
        )
    }
}

/// What the test call that checks a Model asks.
const TEST_PROMPT: &str = "Reply with OK.";

impl fmt::Display for Choice {
    /// `<harness> · <model> · <effort>`, as in `claude · claude-opus-5-5 ·
    /// high`, saying `default model` or `default effort` for one left to the
    /// Harness.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} · {} · {}",
            self.harness.name(),
            self.model.as_deref().unwrap_or("default model"),
            self.effort.as_deref().unwrap_or("default effort")
        )
    }
}

impl fmt::Display for ChosenBy {
    /// The setting that chose the Harness, as a failure names it.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            ChosenBy::Command => "the harness flag",
            ChosenBy::UserConfig => "harness.default in the User config",
            ChosenBy::Default => {
                "the default, as neither the harness flag nor harness.default in the User \
                 config names one"
            }
        })
    }
}

/// Whether an executable file named `cli` is in a directory on `PATH`.
fn on_path(cli: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        Path::new(&dir)
            .join(cli)
            .metadata()
            .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(model: &str, effort: &str) -> ModelAndEffort {
        let named = |value: &str| (!value.is_empty()).then(|| value.to_string());
        ModelAndEffort {
            model: named(model),
            effort: named(effort),
        }
    }

    /// A User config whose default is Codex, with a Model and Effort for
    /// each Harness.
    fn codex_default() -> Settings {
        Settings {
            default: Some(Harness::Codex),
            claude: set("opus", "high"),
            codex: set("gpt-6.1-sol", "max"),
        }
    }

    fn choice(harness: Harness, model: &str, effort: &str, chosen_by: ChosenBy) -> Choice {
        let set = set(model, effort);
        Choice {
            harness,
            model: set.model,
            effort: set.effort,
            chosen_by,
        }
    }

    #[test]
    fn with_nothing_asked_and_nothing_set_sessions_run_on_claude_with_its_own_model_and_effort() {
        let chosen = Choice::of(&Asked::default(), &Settings::default());

        assert_eq!(chosen, Choice::default());
        assert!(chosen.claude_args().is_empty());
    }

    #[test]
    fn the_user_config_chooses_the_harness_and_its_own_model_and_effort() {
        let chosen = Choice::of(&Asked::default(), &codex_default());

        assert_eq!(
            chosen,
            choice(Harness::Codex, "gpt-6.1-sol", "max", ChosenBy::UserConfig)
        );
    }

    #[test]
    fn a_harness_on_the_command_takes_that_harnesss_model_and_effort_from_the_user_config() {
        let asked = Asked {
            harness: Some(Harness::Claude),
            ..Asked::default()
        };

        let chosen = Choice::of(&asked, &codex_default());

        assert_eq!(
            chosen,
            choice(Harness::Claude, "opus", "high", ChosenBy::Command)
        );
    }

    #[test]
    fn a_model_or_effort_on_the_command_applies_to_whichever_harness_is_chosen() {
        for (asked, model, effort) in [
            (set("sonnet", ""), "sonnet", "max"),
            (set("", "low"), "gpt-6.1-sol", "low"),
            (set("sonnet", "low"), "sonnet", "low"),
        ] {
            let asked = Asked {
                harness: None,
                model_and_effort: asked,
            };

            let chosen = Choice::of(&asked, &codex_default());

            assert_eq!(
                chosen,
                choice(Harness::Codex, model, effort, ChosenBy::UserConfig),
                "{asked:?}"
            );
        }
    }

    #[test]
    fn a_harness_with_nothing_set_leaves_its_model_and_effort_to_it() {
        let settings = Settings {
            default: Some(Harness::Claude),
            ..codex_default()
        };
        let asked = Asked {
            harness: Some(Harness::Codex),
            ..Asked::default()
        };
        let chosen = Choice::of(
            &asked,
            &Settings {
                codex: ModelAndEffort::default(),
                ..settings
            },
        );

        assert_eq!(chosen, choice(Harness::Codex, "", "", ChosenBy::Command));
    }

    #[test]
    fn claude_is_given_the_model_and_effort_that_are_set() {
        for (model, effort, args) in [
            ("opus", "", vec!["--model", "opus"]),
            ("", "max", vec!["--effort", "max"]),
            (
                "claude-opus-5-5",
                "high",
                vec!["--model", "claude-opus-5-5", "--effort", "high"],
            ),
        ] {
            let chosen = choice(Harness::Claude, model, effort, ChosenBy::Command);

            assert_eq!(chosen.claude_args(), args);
        }
    }

    #[test]
    fn the_built_with_line_names_the_harness_model_and_effort_or_the_harnesss_default() {
        for (chosen, line) in [
            (
                choice(
                    Harness::Claude,
                    "claude-opus-5-5",
                    "high",
                    ChosenBy::Command,
                ),
                "Built with claude · claude-opus-5-5 · high",
            ),
            (
                Choice::default(),
                "Built with claude · default model · default effort",
            ),
            (
                choice(Harness::Codex, "", "max", ChosenBy::UserConfig),
                "Built with codex · default model · max",
            ),
        ] {
            assert_eq!(chosen.built_with(), line);
        }
    }

    #[test]
    fn codex_is_given_the_model_and_the_effort_in_its_config() {
        for (model, effort, args) in [
            ("", "", vec![]),
            ("gpt-6.1-sol", "", vec!["-m", "gpt-6.1-sol"]),
            (
                "gpt-6.1-sol",
                "max",
                vec!["-m", "gpt-6.1-sol", "-c", "model_reasoning_effort=\"max\""],
            ),
        ] {
            let chosen = choice(Harness::Codex, model, effort, ChosenBy::Command);

            assert_eq!(chosen.codex_args(), args);
        }
    }

    /// A catalog as `codex debug models` prints one, trimmed to what is read.
    fn catalog() -> Catalog {
        Catalog::parse(
            r#"{"models": [
                {"slug": "gpt-6.1-sol", "display_name": "GPT-6.1-Sol", "visibility": "list",
                 "supported_reasoning_levels": [{"effort": "low"}, {"effort": "high"}, {"effort": "max"}]},
                {"slug": "gpt-5.5", "display_name": "GPT-5.5 Classic", "visibility": "list",
                 "supported_reasoning_levels": [{"effort": "low"}, {"effort": "xhigh"}]},
                {"slug": "codex-auto-review", "display_name": "Auto review", "visibility": "hide",
                 "supported_reasoning_levels": [{"effort": "minimal"}]}
            ]}"#,
        )
        .unwrap()
    }

    fn settled(model: Option<&str>, effort: Option<&str>) -> Result<ModelAndEffort> {
        catalog().settle(model, effort)
    }

    #[test]
    fn a_codex_model_and_effort_match_regardless_of_case_and_become_codexs_names() {
        for (model, effort, settled_as) in [
            (Some("GPT-6.1-Sol"), Some("Max"), set("gpt-6.1-sol", "max")),
            (Some("gpt-6.1-sol"), None, set("gpt-6.1-sol", "")),
            (
                Some("gpt-5.5 classic"),
                Some("XHIGH"),
                set("gpt-5.5", "xhigh"),
            ),
            (
                Some("codex-auto-review"),
                None,
                set("codex-auto-review", ""),
            ),
            (None, Some("High"), set("", "high")),
            (None, None, set("", "")),
        ] {
            assert_eq!(
                settled(model, effort).unwrap(),
                settled_as,
                "{model:?} {effort:?}"
            );
        }
    }

    #[test]
    fn an_unknown_codex_model_fails_naming_the_listed_models() {
        let error = settled(Some("gpt-7"), Some("max")).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the Model gpt-7 is not in Codex's catalog: choose one of gpt-6.1-sol, gpt-5.5"
        );
    }

    #[test]
    fn an_effort_the_codex_model_does_not_support_fails_naming_those_it_does() {
        let error = settled(Some("GPT-5.5"), Some("max")).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the Effort max is not one the Codex Model gpt-5.5 supports: choose one of low, xhigh"
        );
    }

    #[test]
    fn with_no_codex_model_the_effort_must_be_one_some_model_supports() {
        let error = settled(None, Some("ultra")).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the Effort ultra is not one any Codex Model supports: choose one of low, high, max, \
             xhigh, minimal"
        );
    }

    #[test]
    fn the_catalog_lists_its_listed_models_by_slug_and_display_name() {
        assert_eq!(
            catalog().listed(),
            ["gpt-6.1-sol (GPT-6.1-Sol)", "gpt-5.5 (GPT-5.5 Classic)"]
        );
    }

    #[test]
    fn the_efforts_are_the_models_or_with_no_model_any_models_each_once() {
        let catalog = catalog();

        assert_eq!(catalog.efforts(Some("gpt-5.5")), ["low", "xhigh"]);
        assert_eq!(
            catalog.efforts(None),
            ["low", "high", "max", "xhigh", "minimal"]
        );
    }

    #[test]
    fn output_that_is_no_catalog_fails() {
        for output in ["", "not json", r#"{"models": 3}"#] {
            assert!(Catalog::parse(output).is_err(), "{output}");
        }
    }
}
