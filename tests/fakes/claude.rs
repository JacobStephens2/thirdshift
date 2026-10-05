//! Fake `claude` for the thirdshift test harness.
//!
//! Each call appends a record to the JSON list in $FAKE_CLAUDE_RECORD: its
//! argv, prompt, working directory, the branch checked out there, whether a
//! merge is in progress there, a snapshot of the `thirdshift-*` skills in its
//! `.claude/skills/`, through their links, and of the files written out
//! beside the skills those links point to (the links' targets are gone by
//! the time a test looks).
//!
//! It then runs a per-test bash script in the working directory, emits a
//! couple of stream-json lines, and exits with the script's exit code. A
//! session's issue is the first Issue URL in its prompt, and its count the
//! number of sessions so far whose prompt names that issue, this one
//! included. The script is the first of these files that exists:
//!
//! ```text
//! $FAKE_CLAUDE_SCRIPT.issue-<issue>.<count>
//! $FAKE_CLAUDE_SCRIPT.issue-<issue>
//! $FAKE_CLAUDE_SCRIPT.<n>     for the n-th call overall (1-based)
//! $FAKE_CLAUDE_SCRIPT
//! ```
//!
//! Parallel child Runs start sessions at once, so recording a call holds an
//! exclusive lock on $FAKE_CLAUDE_RECORD.lock, and the script is chosen under
//! it.
//!
//! The `init` line carries session id `fake-session-<n>` for the n-th call.
//! Lines the script appends to the file named by $FAKE_CLAUDE_AFTER_RESULT
//! are emitted after the closing `result` line, e.g. to kill a background
//! task as the session ends. What the script writes to the file named by
//! $FAKE_CLAUDE_FINAL_MESSAGE is the closing `result` line's `result`: the
//! session's final message.
//!
//! Called without --output-format, as `claude -p` with its prompt on stdin,
//! it records stdin as `stdin` and prints only what the script prints, as
//! print mode's plain text output does.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use crate::json::{Array, Bool, Json, Null, object, string};

/// Where Claude Code finds a worktree's project skills.
const SKILLS: &str = ".claude/skills";

/// Every file under `dir`, by its path relative to `root`, with its
/// contents, into `files`. Like Python's os.walk, which the fake was first
/// written with, it doesn't follow a symlink to a directory below `dir`.
fn snapshot_into(files: &mut Json, root: &Path, dir: &Path) {
    let mut dirs = vec![dir.to_owned()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if fs::symlink_metadata(&path).unwrap().is_dir() {
                dirs.push(path);
            } else if !path.is_dir() {
                let relative = path.strip_prefix(root).unwrap();
                let contents = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
                files.set(relative.to_str().unwrap(), string(contents));
            }
        }
    }
}

/// The `thirdshift-*` entries of the working directory's `skills`, its
/// project skills.
fn linked_skills(skills: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(skills) else {
        return Vec::new();
    };
    entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("thirdshift-"))
        })
        .collect()
}

/// Every file of the `thirdshift-*` skills in the working directory's
/// `skills`, through their links, by its path relative to `skills`, with its
/// contents.
pub fn skills_snapshot(skills: &str) -> Json {
    let mut files = object([]);
    for skill in linked_skills(skills) {
        snapshot_into(&mut files, Path::new(skills), &skill);
    }
    files
}

/// The files written out beside the skills the `thirdshift-*` links in the
/// working directory's `skills` point to, such as their licence, by name,
/// with their contents.
pub fn beside_skills_snapshot(skills: &str) -> Json {
    let mut files = object([]);
    let written = linked_skills(skills)
        .first()
        .and_then(|link| fs::read_link(link).ok())
        .and_then(|target| target.parent().map(Path::to_owned));
    if let Some(written) = written {
        for entry in fs::read_dir(&written).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let name = path.file_name().unwrap().to_str().unwrap();
                files.set(name, string(fs::read_to_string(&path).unwrap()));
            }
        }
    }
    files
}

/// The number of the first Issue URL in `prompt`, if any.
fn issue_of(prompt: &str) -> Option<&str> {
    const START: &str = "https://github.com/";
    let is_name = |c: char| c != '/' && !c.is_whitespace();
    prompt.match_indices(START).find_map(|(at, _)| {
        let rest = &prompt[at + START.len()..];
        let owner = rest.find(|c| !is_name(c)).filter(|&end| end > 0)?;
        let rest = rest[owner..].strip_prefix('/')?;
        let repo = rest.find(|c| !is_name(c)).filter(|&end| end > 0)?;
        let rest = rest[repo..].strip_prefix("/issues/")?;
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        (digits > 0).then(|| &rest[..digits])
    })
}

fn prompt_of(record: &Json) -> &str {
    record.at("prompt").as_str().unwrap_or("")
}

/// The script for the session just recorded last in `records`.
pub fn script_for(records: &[Json]) -> PathBuf {
    let script = crate::env_path("FAKE_CLAUDE_SCRIPT").display().to_string();
    let mut candidates = Vec::new();
    if let Some(issue) = issue_of(prompt_of(records.last().unwrap())) {
        let count = records
            .iter()
            .filter(|record| issue_of(prompt_of(record)) == Some(issue))
            .count();
        candidates.push(format!("{script}.issue-{issue}.{count}"));
        candidates.push(format!("{script}.issue-{issue}"));
    }
    candidates.push(format!("{script}.{}", records.len()));
    candidates
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
        .unwrap_or(script.into())
}

/// `git` in the working directory, with its stdout trimmed, and whether it
/// succeeded.
pub fn git_here(args: &[&str]) -> (String, bool) {
    let output = crate::git(Path::new("."), args);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (stdout, output.status.success())
}

/// The exit code a Python fake gave for `status`, which is negative for a
/// signal and wraps as an exit code.
pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| -status.signal().unwrap() & 0xff)
}

pub fn main(argv: Vec<String>) {
    let (branch, _) = git_here(&["branch", "--show-current"]);
    let (_, merging) = git_here(&["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    let text_mode = !argv.iter().any(|arg| arg == "--output-format");
    let record_path = &crate::env_path("FAKE_CLAUDE_RECORD");

    let lock = crate::lock_beside(record_path);

    let stdin = if text_mode {
        let mut stdin = Vec::new();
        std::io::stdin().read_to_end(&mut stdin).unwrap();
        string(String::from_utf8_lossy(&stdin))
    } else {
        Null
    };
    let cwd = std::env::current_dir().unwrap();
    let cwd = cwd.to_str().unwrap();
    let records = crate::append_record(
        record_path,
        object([
            ("argv", Array(argv.iter().map(string).collect())),
            ("prompt", argv.last().map(string).unwrap_or(Null)),
            ("cwd", string(cwd)),
            ("branch", string(branch)),
            ("merging", Bool(merging)),
            ("skill_files", skills_snapshot(SKILLS)),
            ("beside_skills", beside_skills_snapshot(SKILLS)),
            ("stdin", stdin),
        ]),
    );
    let records = records.items();
    let script = script_for(records);
    drop(lock);

    let bash = || {
        let mut bash = Command::new("bash");
        bash.arg("-e").arg(&script);
        bash
    };
    if text_mode {
        crate::exit(exit_code(bash().status().unwrap()));
    }

    let session = records.len();
    let init = object([
        ("type", string("system")),
        ("subtype", string("init")),
        ("cwd", string(cwd)),
        ("session_id", string(format!("fake-session-{session}"))),
    ]);
    println!("{init}");
    // Beside the record, not in $TMPDIR, which tests expect to be left empty.
    let after_result = format!("{}.after-result.{session}", record_path.display());
    let final_message = format!("{}.final-message.{session}", record_path.display());
    let status = bash()
        .env("FAKE_CLAUDE_AFTER_RESULT", &after_result)
        .env("FAKE_CLAUDE_FINAL_MESSAGE", &final_message)
        .status()
        .unwrap();
    let code = exit_code(status);
    let mut result = object([
        ("type", string("result")),
        ("subtype", string("success")),
        ("is_error", Bool(code != 0)),
    ]);
    if let Ok(message) = fs::read_to_string(&final_message) {
        result.set("result", string(message));
    }
    println!("{result}");
    if let Ok(lines) = fs::read_to_string(&after_result) {
        print!("{lines}");
        std::io::stdout().flush().unwrap();
    }
    crate::exit(code);
}
