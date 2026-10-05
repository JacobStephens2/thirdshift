//! The Harness a Command's agent sessions run on, with its Model and Effort:
//! one [`Choice`] per Command, made by the command, else the User config,
//! else the default, checked before any work, passed on to every session and
//! child Run, and recorded with what the Command did.

use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

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
    fn of(&self, harness: Harness) -> &ModelAndEffort {
        match harness {
            Harness::Claude => &self.claude,
            Harness::Codex => &self.codex,
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

    /// Check, before any work, that sessions can run on this: Codex is not
    /// supported yet; the Harness's CLI must be on `PATH`; and a Model named
    /// for Claude must take a minimal test call, with the Effort, if any.
    /// Each failure says what to change.
    pub fn check(&self) -> Result<()> {
        let cli = self.harness.name();
        if self.harness == Harness::Codex {
            bail!(
                "harness codex, chosen by {}, is not supported yet: Codex sessions come in a \
                 later thirdshift; choose harness claude",
                self.chosen_by
            );
        }
        if !on_path(cli) {
            bail!(
                "{cli} is not on PATH, and the Harness {cli} is chosen by {}: install it, \
                 or choose another Harness",
                self.chosen_by
            );
        }
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
        let said = [&output.stderr, &output.stdout]
            .map(|said| String::from_utf8_lossy(said).trim().to_string())
            .into_iter()
            .filter(|said| !said.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let effort = match &self.effort {
            Some(effort) => format!(" with the Effort {effort}"),
            None => String::new(),
        };
        bail!("{cli} refused a test call on the Model {model}{effort}: {said}")
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

/// The marker that ends the line thirdshift writes in a pull request's body,
/// so it can be found and replaced.
const BUILT_WITH_MARKER: &str = "<!-- thirdshift:built-with -->";

/// `body` with `choice`'s line in it: in place of the one thirdshift wrote
/// before, or else added at the end.
pub fn with_built_with(body: &str, choice: &Choice) -> String {
    let line = format!("{} {BUILT_WITH_MARKER}", choice.built_with());
    if body.contains(BUILT_WITH_MARKER) {
        return body
            .split_inclusive('\n')
            .map(|old| {
                if old.trim_end().ends_with(BUILT_WITH_MARKER) {
                    let end = &old[old.trim_end_matches(['\r', '\n']).len()..];
                    format!("{line}{end}")
                } else {
                    old.to_string()
                }
            })
            .collect();
    }
    if body.trim().is_empty() {
        return format!("{line}\n");
    }
    format!("{}\n\n{line}\n", body.trim_end())
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
    fn the_built_with_line_is_added_to_a_body_and_replaces_the_one_written_before() {
        let opus = choice(Harness::Claude, "opus", "high", ChosenBy::Command);
        let line = "Built with claude · opus · high <!-- thirdshift:built-with -->";

        let added = with_built_with("Adds a button.\n\nCloses #7\n", &opus);
        assert_eq!(added, format!("Adds a button.\n\nCloses #7\n\n{line}\n"));
        assert_eq!(with_built_with("", &opus), format!("{line}\n"));

        let old = "Adds a button.\n\nBuilt with claude · default model · default effort \
                   <!-- thirdshift:built-with -->\n\nCloses #7\n";
        assert_eq!(
            with_built_with(old, &opus),
            format!("Adds a button.\n\n{line}\n\nCloses #7\n")
        );
        assert_eq!(with_built_with(&added, &opus), added);
    }
}
