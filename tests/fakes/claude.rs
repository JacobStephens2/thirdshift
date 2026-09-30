//! Fake `claude` for the thirdshift test harness.
//!
//! Each call appends a record to the JSON list in $FAKE_CLAUDE_RECORD: its
//! argv, prompt, working directory, the branch checked out there, whether a
//! merge is in progress there, and a snapshot of the --plugin-dir contents
//! (the directory is gone by the time a test looks).
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
//! task as the session ends.
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

/// Every file under `plugin_dir`, by its path relative to it, with its
/// contents.
fn plugin_snapshot(plugin_dir: &Path) -> Json {
    let mut files = object([]);
    let mut dirs = vec![plugin_dir.to_owned()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            // Like Python's os.walk, which the fake was first written with,
            // this doesn't follow a symlink to a directory.
            if fs::symlink_metadata(&path).unwrap().is_dir() {
                dirs.push(path);
            } else if !path.is_dir() {
                let relative = path.strip_prefix(plugin_dir).unwrap();
                let contents = String::from_utf8_lossy(&fs::read(&path).unwrap()).into_owned();
                files.set(relative.to_str().unwrap(), string(contents));
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
fn script_for(records: &[Json]) -> PathBuf {
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
fn git_here(args: &[&str]) -> (String, bool) {
    let output = crate::git(Path::new("."), args);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (stdout, output.status.success())
}

/// The exit code a Python fake gave for `status`, which is negative for a
/// signal and wraps as an exit code.
fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| -status.signal().unwrap() & 0xff)
}

pub fn main(argv: Vec<String>) {
    let plugin_dir = argv
        .iter()
        .position(|arg| arg == "--plugin-dir")
        .map(|at| PathBuf::from(&argv[at + 1]));
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
            (
                "plugin_dir",
                plugin_dir
                    .as_ref()
                    .map(|dir| string(dir.to_str().unwrap()))
                    .unwrap_or(Null),
            ),
            (
                "plugin_files",
                plugin_dir
                    .as_deref()
                    .map(plugin_snapshot)
                    .unwrap_or(object([])),
            ),
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
    let status = bash()
        .env("FAKE_CLAUDE_AFTER_RESULT", &after_result)
        .status()
        .unwrap();
    let code = exit_code(status);
    let result = object([
        ("type", string("result")),
        ("subtype", string("success")),
        ("is_error", Bool(code != 0)),
    ]);
    println!("{result}");
    if let Ok(lines) = fs::read_to_string(&after_result) {
        print!("{lines}");
        std::io::stdout().flush().unwrap();
    }
    crate::exit(code);
}
