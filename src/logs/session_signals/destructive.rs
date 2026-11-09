//! Advisory, literal shell-command patterns. Never execute inspected commands.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Flag {
    pattern: String,
    command: String,
}

struct Pattern {
    name: &'static str,
    matches: fn(&Command) -> bool,
}

struct Command<'a> {
    words: &'a [String],
    directory: Option<&'a Path>,
    worktree: Option<&'a Path>,
}

// Keep each pattern and its name together: extending the check is one entry
// here plus fixture cases, without changing metrics or Run control flow.
const PATTERNS: &[Pattern] = &[
    Pattern {
        name: "force-push",
        matches: |command| {
            let words: Vec<_> = command.words.iter().map(String::as_str).collect();
            words.starts_with(&["git", "push"])
                && (words[2..]
                    .iter()
                    .take_while(|word| **word != "--")
                    .any(|word| {
                        matches!(*word, "--force" | "-f" | "--force-with-lease")
                            || word.starts_with("--force-with-lease=")
                            || (word.starts_with('-')
                                && !word.starts_with("--")
                                && word.contains('f'))
                    })
                    || words[2..].iter().any(|word| word.starts_with('+')))
        },
    },
    Pattern {
        name: "shared-history-rewrite",
        matches: |command| {
            let words: Vec<_> = command.words.iter().map(String::as_str).collect();
            // Explicit branch/ref operands name refs shared by the repository's
            // worktrees, unlike a normal rebase onto an upstream in this Worktree.
            (words.starts_with(&["git", "branch"])
                && words.len() >= 4
                && !words.contains(&"--list")
                && words[2..]
                    .iter()
                    .any(|word| matches!(*word, "-f" | "--force")))
                || (words.starts_with(&["git", "update-ref"])
                    && operands(&words[2..], &["-m"])
                        .first()
                        .is_some_and(|reference| reference.starts_with("refs/heads/")))
                || (words.starts_with(&["git", "rebase"])
                    && !words[2..].iter().any(|word| {
                        matches!(
                            *word,
                            "--abort"
                                | "--continue"
                                | "--quit"
                                | "--skip"
                                | "--show-current-patch"
                                | "--edit-todo"
                        )
                    })
                    && operands(
                        &words[2..],
                        &[
                            "--onto",
                            "-s",
                            "--strategy",
                            "-X",
                            "--strategy-option",
                            "-x",
                            "--exec",
                            "--empty",
                            "-C",
                            "--whitespace",
                        ],
                    )
                    .len()
                        >= if words.contains(&"--root") { 1 } else { 2 })
        },
    },
    Pattern {
        name: "recursive-delete-outside-worktree",
        matches: |command| {
            if command.words.first().is_none_or(|word| word != "rm") {
                return false;
            }
            let args = &command.words[1..];
            let recursive = args.iter().take_while(|arg| *arg != "--").any(|arg| {
                arg == "--recursive"
                    || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains(['r', 'R']))
            });
            let Some(worktree) = command.worktree else {
                return false;
            };
            let mut options = true;
            recursive
                && args.iter().any(|arg| {
                    if options && arg == "--" {
                        options = false;
                        return false;
                    }
                    if options && arg.starts_with('-') {
                        return false;
                    }
                    resolve(arg, command.directory).is_some_and(|path| !path.starts_with(worktree))
                })
        },
    },
];

pub(super) fn check(tool: &str, input: &Value, worktree: Option<&Path>) -> Vec<Flag> {
    if !matches!(
        tool,
        "Bash"
            | "bash"
            | "command_execution"
            | "run_command"
            | "run_terminal_command"
            | "run_shell_command"
            | "exec_command"
            | "shell"
            | "shell_command"
    ) {
        return Vec::new();
    }
    let Some(command) = input["command"]
        .as_str()
        .or_else(|| input["CommandLine"].as_str())
        .or_else(|| input["cmd"].as_str())
    else {
        return Vec::new();
    };
    let worktree = worktree.and_then(|path| resolve(path.to_str()?, None));
    let directory = ["cwd", "workdir", "Cwd"]
        .iter()
        .find_map(|key| input[key].as_str())
        .map(|path| resolve(path, worktree.as_deref()))
        .unwrap_or_else(|| worktree.clone());
    let mut matched = HashSet::new();
    inspect(command, directory, worktree.as_deref(), 0, &mut matched);
    PATTERNS
        .iter()
        .filter(|pattern| matched.contains(pattern.name))
        .map(|pattern| Flag {
            pattern: pattern.name.into(),
            command: command.into(),
        })
        .collect()
}

fn inspect(
    script: &str,
    mut directory: Option<PathBuf>,
    worktree: Option<&Path>,
    depth: usize,
    matched: &mut HashSet<&'static str>,
) {
    if depth > 16 {
        return;
    }
    let mut scopes = Vec::new();
    for part in commands(script) {
        let (words, isolated) = match part {
            Part::Words { words, isolated } => (words, isolated),
            Part::Open => {
                scopes.push(directory.clone());
                continue;
            }
            Part::Close { restore } => {
                if let Some(parent) = scopes.pop()
                    && restore
                {
                    directory = parent;
                }
                continue;
            }
        };
        let words = unwrapped(words);
        if words.first().is_some_and(|word| word == "cd") {
            // Pipeline and background commands cannot change the parent shell.
            if !isolated {
                directory = words
                    .iter()
                    .skip(1)
                    .find(|word| *word != "--")
                    .and_then(|path| resolve(path, directory.as_deref()));
            }
            continue;
        }
        if words
            .first()
            .is_some_and(|word| matches!(word.as_str(), "bash" | "sh" | "zsh"))
        {
            if let Some(flag) = words
                .iter()
                .position(|word| matches!(word.as_str(), "-c" | "-lc"))
                && let Some(script) = words.get(flag + 1)
            {
                inspect(script, directory.clone(), worktree, depth + 1, matched);
            }
            continue;
        }
        let invocation = Command {
            words: &words,
            directory: directory.as_deref(),
            worktree,
        };
        for pattern in PATTERNS {
            if (pattern.matches)(&invocation) {
                matched.insert(pattern.name);
            }
        }
    }
}

fn unwrapped(mut words: Vec<String>) -> Vec<String> {
    while let Some(first) = words.first() {
        let program = first.rsplit('/').next().unwrap_or(first);
        if matches!(
            program,
            "if" | "then" | "elif" | "else" | "while" | "until" | "do" | "!"
        ) || first.split_once('=').is_some_and(|(key, _)| {
            !key.is_empty()
                && key
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        }) {
            words.remove(0);
        } else if matches!(program, "env" | "sudo" | "command" | "exec") {
            words.remove(0);
            while words.first().is_some_and(|word| word.starts_with('-')) {
                let option = words.remove(0);
                if matches!(option.as_str(), "-u" | "-g" | "--user" | "--group")
                    && !words.is_empty()
                {
                    words.remove(0);
                }
            }
        } else {
            words[0] = program.to_string();
            break;
        }
    }
    if words.first().is_some_and(|word| word == "git") {
        while words.get(1).is_some_and(|word| word.starts_with('-')) {
            let option = words.remove(1);
            if matches!(option.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree")
                && words.len() > 1
            {
                words.remove(1);
            }
        }
    }
    words
}

fn operands<'a>(words: &[&'a str], value_options: &[&str]) -> Vec<&'a str> {
    let mut operands = Vec::new();
    let mut words = words.iter().copied();
    while let Some(word) = words.next() {
        if word == "--" {
            operands.extend(words);
            break;
        }
        if value_options.contains(&word) {
            words.next();
        } else if !word.starts_with('-') {
            operands.push(word);
        }
    }
    operands
}

// Lexical resolution keeps past traces readable after Worktree removal. Shell
// expansions and symlinks are deliberately not evaluated against today's host.
fn resolve(path: &str, directory: Option<&Path>) -> Option<PathBuf> {
    if path.contains(['$', '`']) || path.starts_with('~') {
        return None;
    }
    let path = Path::new(path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        directory?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            component @ (Component::Prefix(_) | Component::RootDir | Component::Normal(_)) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Some(normalized)
}

/// Split literal shell words and command separators while preserving quoted
/// arguments. A quoted example in echo/printf remains an argument, not a call.
enum Part {
    Words { words: Vec<String>, isolated: bool },
    Open,
    Close { restore: bool },
}

struct Heredoc {
    delimiter: String,
    strip_tabs: bool,
}

#[derive(Default)]
struct Words {
    current: Option<String>,
    complete: Vec<String>,
    delimiter: Option<bool>,
    heredocs: Vec<Heredoc>,
}

impl Words {
    fn finish(&mut self) {
        if let Some(word) = self.current.take() {
            if let Some(strip_tabs) = self.delimiter.take() {
                self.heredocs.push(Heredoc {
                    delimiter: word,
                    strip_tabs,
                });
            } else {
                self.complete.push(word);
            }
        }
    }

    fn take(&mut self) -> Vec<String> {
        self.finish();
        std::mem::take(&mut self.complete)
    }
}

fn commands(script: &str) -> Vec<Part> {
    let mut commands = Vec::new();
    let mut words = Words::default();
    let mut quote = None;
    let mut pipeline = false;
    let mut groups = Vec::new();
    let mut chars = script.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && quote != Some('\'') {
            if let Some(escaped) = chars.next()
                && escaped != '\n'
            {
                words.current.get_or_insert_default().push(escaped);
            }
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
            words.current.get_or_insert_default();
        } else if quote.is_none() && ch == '#' && words.current.is_none() {
            while chars.peek().is_some_and(|ch| *ch != '\n') {
                chars.next();
            }
        } else if quote.is_none() && ch == '<' && chars.peek() == Some(&'<') {
            words.finish();
            chars.next();
            if chars.peek() == Some(&'<') {
                chars.next(); // A here-string has no following body to skip.
            } else {
                let strip_tabs = chars.peek() == Some(&'-');
                if strip_tabs {
                    chars.next();
                }
                words.delimiter = Some(strip_tabs);
            }
        } else if quote.is_none()
            && (ch.is_whitespace()
                || matches!(ch, ';' | '|' | '&' | '(' | ')')
                || (matches!(ch, '{' | '}')
                    && words.current.is_none()
                    && chars.peek().is_none_or(|next| {
                        next.is_whitespace() || matches!(next, ';' | '|' | '&' | '(' | ')')
                    })))
        {
            words.finish();
            let inherited_pipeline = pipeline;
            if matches!(ch, '\n' | ';' | '|' | '&' | '(' | ')' | '{' | '}') {
                let paired = matches!(ch, '|' | '&') && chars.peek() == Some(&ch);
                if paired {
                    chars.next();
                }
                let single_operator = matches!(ch, '|' | '&') && !paired;
                let complete = words.take();
                if !complete.is_empty() {
                    commands.push(Part::Words {
                        words: complete,
                        isolated: pipeline || single_operator,
                    });
                    pipeline = false;
                } else if single_operator && let Some(Part::Close { restore }) = commands.last_mut()
                {
                    // The entire group runs separately when piped/backgrounded,
                    // including directory changes before its closing brace.
                    *restore = true;
                }
                if matches!(ch, '|' | '&') {
                    pipeline = ch == '|' && !paired;
                }
            }
            match ch {
                '(' | '{' => {
                    commands.push(Part::Open);
                    groups.push(ch == '(' || inherited_pipeline);
                    pipeline = false;
                }
                ')' | '}' => {
                    commands.push(Part::Close {
                        restore: groups.pop().unwrap_or(true),
                    });
                }
                '\n' => {
                    for heredoc in std::mem::take(&mut words.heredocs) {
                        loop {
                            let line: String =
                                chars.by_ref().take_while(|ch| *ch != '\n').collect();
                            let line = if heredoc.strip_tabs {
                                line.trim_start_matches('\t')
                            } else {
                                &line
                            };
                            if line == heredoc.delimiter || chars.peek().is_none() {
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        } else {
            words.current.get_or_insert_default().push(ch);
        }
    }
    let complete = words.take();
    if !complete.is_empty() {
        commands.push(Part::Words {
            words: complete,
            isolated: pipeline,
        });
    }
    commands
}
