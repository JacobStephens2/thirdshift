//! Fake `codex` for the thirdshift test harness.
//!
//! `codex debug models` prints [`CATALOG`], a fixed catalog of Models.
//!
//! `codex exec --json …` is an agent session, scripted as the fake `claude`'s
//! are: it runs the same per-test bash script, chosen the same way from
//! $FAKE_CLAUDE_SCRIPT by the session's issue and count, with the sessions
//! counted from its own record. Each call appends a record to the JSON list
//! in $FAKE_CODEX_RECORD: its argv, prompt, working directory, the branch
//! checked out there, whether its stdin was `/dev/null`, `git status
//! --porcelain` there, a snapshot of the `thirdshift-*` skills in its
//! `.agents/skills/`, through their links, and of the files written out
//! beside them.
//!
//! It emits `thread.started`, with thread id `fake-thread-<n>` for the n-th
//! call, then `turn.started`, then runs the script, whose stdout joins the
//! stream. What the script writes to $FAKE_CLAUDE_FINAL_MESSAGE becomes an
//! `agent_message` item. A script that succeeds ends the turn with
//! `turn.completed`, and one that fails with `turn.failed`, whose error is
//! what it wrote to
//! $FAKE_CODEX_ERROR, else a default, and exits 1, as Codex does.
//!
//! SIGINT and SIGTERM to its process group reach its script, which decides
//! how the session stops: the fake waits for its script whatever signal
//! comes, short of SIGKILL, so what the script records of the signals is
//! there once the fake has exited.

use std::fs;
use std::io::Write;
use std::process::Command;

use crate::claude::{beside_skills_snapshot, exit_code, git_here, script_for, skills_snapshot};
use crate::json::{Array, Bool, Json, Null, object, string};
use crate::{outlast_interrupts, stdin_is_null};

/// Where Codex finds a worktree's project skills.
const SKILLS: &str = ".agents/skills";

/// What `codex debug models` prints: a few Models, each with its display
/// name and the Efforts it supports.
const CATALOG: &str = r#"{"models": [
  {"slug": "gpt-6.1-sol", "display_name": "GPT-6.1-Sol", "visibility": "list", "default_reasoning_level": "low",
   "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}, {"effort": "max"}, {"effort": "ultra"}]},
  {"slug": "gpt-6-luna", "display_name": "GPT-6-Luna", "visibility": "list", "default_reasoning_level": "medium",
   "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}, {"effort": "max"}]},
  {"slug": "gpt-5.5", "display_name": "GPT-5.5", "visibility": "list", "default_reasoning_level": "medium",
   "supported_reasoning_levels": [{"effort": "low"}, {"effort": "medium"}, {"effort": "high"}, {"effort": "xhigh"}]}
]}"#;

/// Print one line of the stream.
fn emit(event: Json) {
    println!("{event}");
    std::io::stdout().flush().unwrap();
}

pub fn main(argv: Vec<String>) {
    match argv.first().map(String::as_str) {
        Some("debug") if argv.get(1).map(String::as_str) == Some("models") => {
            println!("{CATALOG}");
            crate::exit(0);
        }
        Some("exec") if argv.iter().any(|arg| arg == "--json") => {}
        _ => crate::die(
            &format!(
                "fake codex: unexpected arguments {}",
                crate::python_list(&argv)
            ),
            2,
        ),
    }
    let (branch, _) = git_here(&["branch", "--show-current"]);
    let (status, _) = git_here(&["status", "--porcelain"]);
    let record_path = &crate::env_path("FAKE_CODEX_RECORD");

    let lock = crate::lock_beside(record_path);
    let cwd = std::env::current_dir().unwrap();
    let cwd = cwd.to_str().unwrap();
    let records = crate::append_record(
        record_path,
        object([
            ("argv", Array(argv.iter().map(string).collect())),
            ("prompt", argv.last().map(string).unwrap_or(Null)),
            ("cwd", string(cwd)),
            ("branch", string(branch)),
            ("stdin_null", Bool(stdin_is_null())),
            ("git_status", string(status)),
            ("skill_files", skills_snapshot(SKILLS)),
            ("beside_skills", beside_skills_snapshot(SKILLS)),
        ]),
    );
    let records = records.items();
    let script = script_for(records);
    drop(lock);

    let session = records.len();
    outlast_interrupts();
    emit(object([
        ("type", string("thread.started")),
        ("thread_id", string(format!("fake-thread-{session}"))),
    ]));
    emit(object([("type", string("turn.started"))]));
    // Beside the record, not in $TMPDIR, which tests expect to be left empty.
    let final_message = format!("{}.final-message.{session}", record_path.display());
    let error = format!("{}.error.{session}", record_path.display());
    let status = Command::new("bash")
        .arg("-e")
        .arg(&script)
        .env("FAKE_CLAUDE_FINAL_MESSAGE", &final_message)
        .env("FAKE_CODEX_ERROR", &error)
        .status()
        .unwrap();
    let code = exit_code(status);
    if let Ok(message) = fs::read_to_string(&final_message) {
        emit(object([
            ("type", string("item.completed")),
            (
                "item",
                object([
                    ("id", string("item_final")),
                    ("type", string("agent_message")),
                    ("text", string(message)),
                ]),
            ),
        ]));
    }
    if code == 0 {
        emit(object([
            ("type", string("turn.completed")),
            (
                "usage",
                object([
                    ("input_tokens", crate::json::number(1200)),
                    ("cached_input_tokens", crate::json::number(200)),
                    ("output_tokens", crate::json::number(300)),
                ]),
            ),
        ]));
        crate::exit(0);
    }
    let message = fs::read_to_string(&error)
        .map(|message| message.trim_end().to_string())
        .unwrap_or_else(|_| format!("the agent's script exited {code}"));
    emit(object([
        ("type", string("turn.failed")),
        ("error", object([("message", string(message))])),
    ]));
    crate::exit(1);
}
