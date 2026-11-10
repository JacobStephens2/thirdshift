//! Known Launch directories, shared by CLI discovery and Console starts.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{config, cronjobs, git::Git, issue::Repo};

#[derive(Serialize)]
pub(crate) struct Directory {
    pub repository: String,
    pub path: PathBuf,
    #[serde(skip)]
    pub output_log: Option<PathBuf>,
}

#[derive(Serialize)]
pub(crate) struct Inventory {
    pub directories: Vec<Directory>,
}

pub(crate) fn load() -> Result<Inventory> {
    let home = config::home()?;
    let config = config::UserConfig::load()?;
    let jobs = match cronjobs::jobs() {
        Ok(jobs) => jobs,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Vec::new()
        }
        Err(error) => return Err(error),
    };
    let mut directories = BTreeMap::new();
    for job in jobs {
        if let Some(directory) = from_cron(&job.command, &home) {
            directories
                .entry(directory.path.clone())
                .or_insert(directory);
        }
    }
    for path in config.launch_directories {
        let directory = resolve(&path)
            .with_context(|| format!("invalid configured Launch directory {}", path.display()))?;
        directories
            .entry(directory.path.clone())
            .or_insert(directory);
    }
    Ok(Inventory {
        directories: directories.into_values().collect(),
    })
}

fn resolve(path: &Path) -> Result<Directory> {
    let path = path.canonicalize()?;
    let origin = Git::new(&path).run(&["config", "--get", "remote.origin.url"])?;
    let repo = Repo::of_origin(&origin).context("Launch directory origin is not on GitHub")?;
    Ok(Directory {
        repository: repo.slug(),
        path,
        output_log: None,
    })
}

// Parse literal shell words only. Never execute cron text to discover a path.
pub(crate) fn from_cron(command: &str, home: &Path) -> Option<Directory> {
    let invocation = cron_invocation(command, home)?;
    let mut directory = resolve(&invocation.directory).ok()?;
    directory.output_log = invocation.output_log;
    Some(directory)
}

struct CronInvocation {
    directory: PathBuf,
    words: Vec<String>,
    output_log: Option<PathBuf>,
}

fn cron_invocation(command: &str, home: &Path) -> Option<CronInvocation> {
    let words = shell_words(cronjobs::shell_command(command))?;
    let [cd, path, and, rest @ ..] = words.as_slice() else {
        return None;
    };
    if cd != "cd" || and != "&&" || path.contains(['$', '`']) {
        return None;
    }
    let thirdshift = rest
        .split(|word| matches!(word.as_str(), "&&" | ";" | "||"))
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
                // A literal env wrapper still uses this cd directory. Keep the
                // wrapper words below so Run now cannot discard its environment.
                part.iter().skip(1).find(|word| !word.contains('='))
            } else {
                part.first()
            };
            executable.is_some_and(|word| {
                Path::new(word)
                    .file_name()
                    .is_some_and(|name| name == "thirdshift")
            })
        })?;
    let path = config::expand_home(path, home)?;
    let output_log = thirdshift
        .windows(2)
        .find(|pair| matches!(pair[0].as_str(), ">>" | ">" | "1>" | "1>>" | "&>" | "&>>"))
        .and_then(|pair| {
            let log = &pair[1];
            if log.contains(['$', '`']) {
                return None;
            }
            config::expand_home(log, home).or_else(|| Some(path.join(log)))
        });
    Some(CronInvocation {
        directory: path,
        words: thirdshift.iter().skip(1).cloned().collect(),
        output_log,
    })
}

/// A literal cron invocation that can also be represented by the start API.
/// Shell expressions remain display-only; no cron text is ever executed here.
pub(crate) fn cron_arguments(command: &str) -> Option<(PathBuf, Vec<String>)> {
    let home = config::home().ok()?;
    let invocation = cron_invocation(command, &home)?;
    let arguments: Vec<_> = invocation
        .words
        .into_iter()
        .take_while(|word| {
            let operator = word.trim_start_matches(|c: char| c.is_ascii_digit());
            !matches!(
                operator,
                ">" | ">>" | "&>" | "&>>" | "<" | ">&" | "<&" | "#"
            )
        })
        .collect();
    if arguments.iter().any(|word| word.contains(['$', '`'])) {
        return None;
    }
    Some((invocation.directory.canonicalize().ok()?, arguments))
}

// shlex splits shell words, but leaves operators attached to those words.
// Separate unquoted operators first, preserving quotes and escaped characters.
pub(crate) fn shell_words(command: &str) -> Option<Vec<String>> {
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
            // A contiguous IO number belongs to the redirect (2>&1); a
            // standalone number remains a CLI argument (parallel 2 >> log).
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

pub fn show(json: bool) -> Result<String> {
    let inventory = load()?;
    if json {
        return Ok(serde_json::to_string_pretty(&inventory)?);
    }
    Ok(inventory
        .directories
        .iter()
        .map(|directory| format!("{}  {}", directory.repository, directory.path.display()))
        .collect::<Vec<_>>()
        .join("\n"))
}
