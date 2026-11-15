//! Literal Cron job interpretation, without filesystem reads or shell execution.
use std::path::{Path, PathBuf};

use crate::{args, config};

use super::CommandKind;

/// Independent inventory and Launch projections of one command, tokenized once.
pub(crate) struct Interpretation {
    recognized: bool,
    kind: Option<CommandKind>,
    launch: Option<LiteralLaunch>,
}

struct LiteralLaunch {
    directory: String,
    output_log: Option<String>,
    command: Option<args::Command>,
}

pub(crate) struct LaunchFacts {
    pub directory: PathBuf,
    pub output_log: Option<PathBuf>,
    /// A parsed candidate; Console still decides which Commands it can start.
    pub command: Option<args::Command>,
}

impl Interpretation {
    pub(crate) fn new(command: &str) -> Self {
        let command = shell_command(command);
        // Inclusion is lexical, so failed shell/CLI parsing cannot hide an entry.
        let recognized = command
            .split(|c: char| c.is_whitespace() || "'\";&|()<>".contains(c))
            .any(|word| word.rsplit('/').next() == Some("thirdshift"));
        let Some(words) = shell_words(command) else {
            return Self {
                recognized,
                kind: None,
                launch: None,
            };
        };
        let initial = initial_directory(&words);
        let invocation = initial.map_or(words.as_slice(), |(_, rest)| rest);
        let launch = initial
            .and_then(|(path, rest)| launch_invocation(rest).map(|selected| (path, selected)));
        let selected = launch.map_or(invocation, |(_, selected)| selected);
        let parsed = selected
            .split_first()
            .and_then(|(_, arguments)| literal_cli_arguments(arguments))
            .and_then(|arguments| args::parse(&arguments).ok());
        let kind = if selected == invocation && is_plain_invocation(invocation) {
            parsed.as_ref().and_then(command_kind)
        } else {
            None
        };
        let launch = launch.map(|(directory, selected)| LiteralLaunch {
            directory: directory.into(),
            output_log: selected
                .windows(2)
                .find(|pair| matches!(pair[0].as_str(), ">>" | ">" | "1>" | "1>>" | "&>" | "&>>"))
                .map(|pair| pair[1].clone())
                .filter(|log| !log.contains(['$', '`'])),
            command: parsed,
        });
        Self {
            recognized,
            kind,
            launch,
        }
    }

    pub(crate) fn recognized(&self) -> bool {
        self.recognized
    }

    pub(crate) fn command_kind(&self) -> Option<CommandKind> {
        self.kind
    }

    /// Expand only literal paths, using the requesting machine's supplied home.
    pub(crate) fn into_launch(self, home: &Path) -> Option<LaunchFacts> {
        let launch = self.launch?;
        let directory = config::expand_home(&launch.directory, home)?;
        let output_log = launch
            .output_log
            .map(|log| config::expand_home(&log, home).unwrap_or_else(|| directory.join(log)));
        Some(LaunchFacts {
            directory,
            output_log,
            command: launch.command,
        })
    }
}

fn initial_directory(words: &[String]) -> Option<(&str, &[String])> {
    let [cd, path, and, rest @ ..] = words else {
        return None;
    };
    (cd == "cd" && and == "&&" && !path.contains(['$', '`'])).then_some((path.as_str(), rest))
}

fn is_plain_invocation(words: &[String]) -> bool {
    let Some((executable, arguments)) = words.split_first() else {
        return false;
    };
    !executable.contains(['$', '`'])
        && Path::new(executable)
            .file_name()
            .is_some_and(|name| name == "thirdshift")
        && !arguments
            .iter()
            .any(|word| matches!(word.as_str(), "&&" | "||" | ";" | "|" | "&"))
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "inventory recognizes work Commands only; inspection and administration commands have unknown kind"
)]
fn command_kind(command: &args::Command) -> Option<CommandKind> {
    use args::Command as Parsed;
    Some(match command {
        Parsed::Pickup(_) => CommandKind::Pickup,
        Parsed::Secure(_) => CommandKind::Security,
        Parsed::Architect(_) => CommandKind::Architect,
        Parsed::Triage(_) => CommandKind::Triage,
        Parsed::Fix(_) => CommandKind::Fix,
        Parsed::Review(_) => CommandKind::Review,
        Parsed::Release(_) => CommandKind::Release,
        Parsed::Process(args::ProcessAction::Run(_)) => CommandKind::Process,
        // An Issue URL can select a Spec run or Ticket run; labels are not read here.
        _ => return None,
    })
}

fn launch_invocation(rest: &[String]) -> Option<&[String]> {
    rest.split(|word| matches!(word.as_str(), "&&" | ";" | "||"))
        // A later directory change makes the initial cd unreliable. Leave
        // these compounds display-only rather than infer the wrong checkout.
        .take_while(|part| {
            let executable = part.iter().map(String::as_str).find(|word| {
                !word.contains('=') && !matches!(*word, "command" | "builtin" | "--" | "-p" | "!")
            });
            !matches!(
                executable,
                Some("cd" | "pushd" | "popd" | "eval" | "source" | ".")
            )
        })
        .find(|part| {
            let executable = if part.first().is_some_and(|word| {
                Path::new(word)
                    .file_name()
                    .is_some_and(|name| name == "env")
            }) {
                // Retain launcher words in the candidate: starting it must not
                // silently discard the environment used by the Cron job.
                part.iter().skip(1).find(|word| !word.contains('='))
            } else {
                part.first()
            };
            executable.is_some_and(|word| {
                Path::new(word)
                    .file_name()
                    .is_some_and(|name| name == "thirdshift")
            })
        })
}

/// Cron sends everything after its first unescaped % to stdin, even in quotes.
fn shell_command(command: &str) -> &str {
    let mut escaped = false;
    let command_end = command
        .char_indices()
        .find_map(|(index, character)| {
            let separator = character == '%' && !escaped;
            escaped = character == '\\' && !escaped;
            separator.then_some(index)
        })
        .unwrap_or(command.len());
    &command[..command_end]
}

// Keep literal CLI words before shell redirects; expansions remain unknown.
fn literal_cli_arguments(words: &[String]) -> Option<Vec<String>> {
    let arguments: Vec<_> = words
        .iter()
        .take_while(|word| {
            let operator = word.trim_start_matches(|c: char| c.is_ascii_digit());
            !matches!(
                operator,
                ">" | ">>" | "&>" | "&>>" | "<" | ">&" | "<&" | "#"
            )
        })
        .cloned()
        .collect();
    if arguments.iter().any(|word| word.contains(['$', '`'])) {
        return None;
    }
    Some(arguments)
}

// shlex splits shell words, but leaves operators attached to those words.
// Separate unquoted operators first, preserving quotes and escaped characters.
fn shell_words(command: &str) -> Option<Vec<String>> {
    let mut spaced = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut chars = command.chars().peekable();
    while let Some(character) = chars.next() {
        if escaped {
            spaced.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            spaced.push(character);
            escaped = true;
            continue;
        }
        if quote == Some(character) {
            quote = None;
        } else if quote.is_none() && matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if quote.is_none() && "&|;<>".contains(character) {
            // An attached IO number belongs to its redirect; standalone
            // numbers remain CLI arguments (parallel 2 >> log).
            let word_start = spaced.rfind(char::is_whitespace).map_or(0, |at| at + 1);
            let word = &spaced[word_start..];
            let io_number = matches!(character, '<' | '>')
                && !word.is_empty()
                && word.bytes().all(|byte| byte.is_ascii_digit());
            if !io_number {
                spaced.push(' ');
            }
            spaced.push(character);
            if chars.peek() == Some(&character)
                || matches!(character, '>' | '<') && chars.peek() == Some(&'&')
            {
                spaced.push(chars.next()?);
            } else if character == '&' && chars.peek() == Some(&'>') {
                spaced.push(chars.next()?);
                if chars.peek() == Some(&'>') {
                    spaced.push(chars.next()?);
                }
            }
            spaced.push(' ');
            continue;
        }
        spaced.push(character);
    }
    shlex::split(&spaced)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_changing_launch_forms_leave_later_commands_unresolved() {
        for change in [
            "cd /other",
            "pushd /other",
            "popd",
            "command cd /other",
            "command -p -- cd /other",
            "builtin cd /other",
            "CDPATH=.. cd other",
            "! cd /other",
            "eval 'cd /other'",
            "source /change-directory.sh",
            ". /change-directory.sh",
        ] {
            let text = format!("cd /repo && {change} && thirdshift pickup");
            let interpreted = Interpretation::new(&text);
            assert!(interpreted.recognized(), "{text}");
            assert!(interpreted.command_kind().is_none(), "{text}");
            assert!(
                interpreted.into_launch(Path::new("/home/test")).is_none(),
                "{text}"
            );
        }
    }

    #[test]
    fn literal_launch_facts_preserve_cron_separators_paths_and_stdout_logs() {
        let home = Path::new("/home/test");
        for (text, directory, log, kind, candidate) in [
            (
                "cd /repo&&thirdshift pickup parallel 2>>cron.log 2>&1",
                Some("/repo"),
                None,
                None,
                false,
            ),
            (
                "cd /repo&&thirdshift pickup parallel 2 >>cron.log 2>&1",
                Some("/repo"),
                Some("/repo/cron.log"),
                Some("Pickup run"),
                true,
            ),
            (
                "cd /repo && thirdshift pickup parallel 2 2>>errors.log",
                Some("/repo"),
                None,
                Some("Pickup run"),
                true,
            ),
            (
                "cd /repo && thirdshift pickup parallel 2 2>&1",
                Some("/repo"),
                None,
                Some("Pickup run"),
                true,
            ),
            (
                "cd /repo && thirdshift pickup parallel 2 &>>~/cron.log",
                Some("/repo"),
                Some("/home/test/cron.log"),
                Some("Pickup run"),
                true,
            ),
            (
                "cd /repo && thirdshift pickup parallel 2 1>/tmp/cron.log",
                Some("/repo"),
                Some("/tmp/cron.log"),
                Some("Pickup run"),
                true,
            ),
            (
                "cd /repo && thirdshift pickup parallel 2 >'$LOG'",
                Some("/repo"),
                None,
                Some("Pickup run"),
                true,
            ),
            (
                "cd /repo && thirdshift pickup parallel 2 >'`log`'",
                Some("/repo"),
                None,
                Some("Pickup run"),
                true,
            ),
            (
                "cd relative && thirdshift pickup",
                None,
                None,
                Some("Pickup run"),
                false,
            ),
            (
                "cd ~someone/repo && thirdshift pickup",
                None,
                None,
                Some("Pickup run"),
                false,
            ),
            (
                "cd '$HOME/repo' && thirdshift pickup",
                None,
                None,
                None,
                false,
            ),
            (
                "cd /repo && thirdshift pickup%stdin && cd elsewhere",
                Some("/repo"),
                None,
                Some("Pickup run"),
                true,
            ),
            (
                "cd /repo && thirdshift pickup >'quoted%log'",
                None,
                None,
                None,
                false,
            ),
            (
                r"cd /repo && thirdshift pickup >escaped\%log",
                Some("/repo"),
                Some("/repo/escaped%log"),
                Some("Pickup run"),
                true,
            ),
            (
                r"cd /repo && thirdshift pickup >escaped\\%stdin",
                Some("/repo"),
                Some("/repo/escaped\\"),
                Some("Pickup run"),
                true,
            ),
        ] {
            let interpreted = Interpretation::new(text);
            assert!(interpreted.recognized(), "{text}");
            assert_eq!(
                interpreted
                    .command_kind()
                    .map(|kind| kind.to_string())
                    .as_deref(),
                kind,
                "{text}"
            );
            let facts = interpreted.into_launch(home);
            assert_eq!(
                facts.as_ref().map(|facts| facts.directory.as_path()),
                directory.map(Path::new),
                "{text}"
            );
            assert_eq!(
                facts.as_ref().and_then(|facts| facts.output_log.as_deref()),
                log.map(Path::new),
                "{text}"
            );
            let parsed = facts.and_then(|facts| facts.command);
            if candidate {
                let Some(args::Command::Pickup(pass)) = parsed else {
                    panic!("expected Pickup: {text}");
                };
                if text.contains("parallel 2") {
                    assert_eq!(pass.flags.parallel.map(|n| n.get()), Some(2), "{text}");
                }
            } else {
                assert!(parsed.is_none(), "{text}");
            }
        }
    }

    #[test]
    fn interpretation_preserves_independent_cron_job_facts() {
        let home = std::path::Path::new("/home/test");
        // Command kind and a parsed start candidate are independent projections.
        for (text, shown, kind, directory, candidate) in [
            ("thirdshift pickup", true, Some("Pickup run"), None, false),
            (
                "cd ~/repo && thirdshift pickup",
                true,
                Some("Pickup run"),
                Some("/home/test/repo"),
                true,
            ),
            (
                "cd ~/repo && env X=1 thirdshift pickup",
                true,
                None,
                Some("/home/test/repo"),
                false,
            ),
            (
                "cd ~/repo && thirdshift status --json",
                true,
                None,
                Some("/home/test/repo"),
                true,
            ),
            (
                "cd ~/repo && thirdshift pickup --unknown",
                true,
                None,
                Some("/home/test/repo"),
                false,
            ),
            (
                "cd ~/repo && thirdshift pickup base '$BASE'",
                true,
                None,
                Some("/home/test/repo"),
                false,
            ),
            (
                "cd ~/repo && cd /elsewhere && thirdshift pickup",
                true,
                None,
                None,
                false,
            ),
            (
                "cd ~/repo && thirdshift pickup && echo done",
                true,
                None,
                Some("/home/test/repo"),
                true,
            ),
            (
                "cd ~/repo && thirdshift fix https://github.com/acme/widgets/pull/7",
                true,
                Some("Fix run"),
                Some("/home/test/repo"),
                true,
            ),
            (
                "cd ~/repo && thirdshift review https://github.com/acme/widgets/pull/7",
                true,
                Some("Review run"),
                Some("/home/test/repo"),
                true,
            ),
            (
                "cd ~/repo && thirdshift triage",
                true,
                Some("Triage run"),
                Some("/home/test/repo"),
                true,
            ),
            ("cd '~/repo && thirdshift pickup", true, None, None, false),
            ("backup%thirdshift pickup", false, None, None, false),
        ] {
            let interpreted = Interpretation::new(text);
            assert_eq!(interpreted.recognized(), shown, "{text}");
            assert_eq!(
                interpreted
                    .command_kind()
                    .map(|kind| kind.to_string())
                    .as_deref(),
                kind,
                "{text}"
            );
            let facts = interpreted.into_launch(home);
            assert_eq!(
                facts.as_ref().map(|facts| facts.directory.as_path()),
                directory.map(std::path::Path::new),
                "{text}"
            );
            assert_eq!(
                facts.as_ref().is_some_and(|facts| facts.command.is_some()),
                candidate,
                "{text}"
            );
            assert!(
                facts.is_none_or(|facts| facts.output_log.is_none()),
                "{text}"
            );
        }
    }
}
