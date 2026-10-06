//! Fake Muse: runs the scenario script, emits envelopes, and writes its session log.

use std::fs;
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::process::Command;

use crate::claude::{beside_skills_snapshot, exit_code, git_here, script_for, skills_snapshot};
use crate::json::{Array, Bool, Json, Null, object, string};

/// Where Muse finds a worktree's project skills.
const SKILLS: &str = ".agents/skills";

/// Whether stdin is `/dev/null`, by `fstat` on it, as macOS has no `/proc`.
pub(crate) fn stdin_is_null() -> bool {
    let stdin = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map(fs::File::from)
        .and_then(|file| file.metadata());
    let (Ok(stdin), Ok(null)) = (stdin, fs::metadata("/dev/null")) else {
        return false;
    };
    stdin.rdev() == null.rdev() && stdin.ino() == null.ino()
}

/// Catch SIGINT and SIGTERM, doing nothing on either, so the fake outlasts
/// them until its script ends. A caught signal, unlike an ignored one, is
/// back to its default in the script, which can trap it.
fn outlast_interrupts() {
    // From the C library, which the standard library links: SIGINT and
    // SIGTERM are 2 and 15 on Linux and macOS alike.
    unsafe extern "C" {
        fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    }
    extern "C" fn caught(_: i32) {}
    for signum in [2, 15] {
        // SAFETY: the handler does nothing, so it is async-signal-safe.
        unsafe { signal(signum, caught) };
    }
}

/// Print one line of the stream.
fn emit(event: Json) {
    println!("{event}");
    std::io::stdout().flush().unwrap();
}

pub fn main(argv: Vec<String>) {
    match argv.first().map(String::as_str) {
        Some("exec") if argv.iter().any(|arg| arg == "--json") => {}
        _ => crate::die(
            &format!(
                "fake muse: unexpected arguments {}",
                crate::python_list(&argv)
            ),
            2,
        ),
    }
    if argv.last().is_some_and(|prompt| prompt == "Reply with OK.") {
        let path = crate::env_path("FAKE_MUSE_RECORD").with_extension("checks.json");
        crate::append_record(
            &path,
            object([
                ("argv", Array(argv.iter().map(string).collect())),
                ("stdin_null", Bool(stdin_is_null())),
                (
                    "no_auto_update",
                    string(std::env::var("MUSE_NO_AUTO_UPDATE").unwrap_or_default()),
                ),
            ]),
        );
        if argv
            .windows(2)
            .any(|pair| pair[0] == "--model" && pair[1] == "bad-model")
        {
            emit_event(
                "fake-check",
                "run.terminal.failed",
                object([("reason", string("model does not exist or you lack access"))]),
            );
            crate::exit(1);
        }
        emit_event(
            "fake-check",
            "run.terminal.completed",
            object([("text", string("OK"))]),
        );
        crate::exit(0);
    }
    let (branch, _) = git_here(&["branch", "--show-current"]);
    let (status, _) = git_here(&["status", "--porcelain"]);
    let record_path = &crate::env_path("FAKE_MUSE_RECORD");

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
            (
                "no_auto_update",
                string(std::env::var("MUSE_NO_AUTO_UPDATE").unwrap_or_default()),
            ),
            ("skill_files", skills_snapshot(SKILLS)),
            ("beside_skills", beside_skills_snapshot(SKILLS)),
        ]),
    );
    let records = records.items();
    let script = script_for(records);
    drop(lock);

    let session = records.len();
    outlast_interrupts();
    let id = argv
        .windows(2)
        .find(|pair| pair[0] == "--session-id")
        .map(|pair| pair[1].clone())
        .unwrap_or_else(|| format!("fake-muse-{session}"));
    emit_event(&id, "run.lifecycle.started", object([]));
    let prompt = argv.last().unwrap();
    if let Some(rest) = prompt.strip_prefix("Load ") {
        let skill = rest.split_whitespace().next().unwrap();
        if std::env::var("FAKE_MUSE_SKIP_SKILL").is_err() {
            emit_event(
                &id,
                "tool.result",
                object([
                    (
                        "correlation_facts",
                        object([
                            ("tool_name", string("read_skill")),
                            ("outcome", string("success")),
                        ]),
                    ),
                    (
                        "text",
                        string(format!(
                            "<read-skill-result name=\"{skill}\" status=\"ok\">\n</read-skill-result>"
                        )),
                    ),
                ]),
            );
        }
    }
    // Beside the record, not in $TMPDIR, which tests expect to be left empty.
    let final_message = format!("{}.final-message.{session}", record_path.display());
    let error = format!("{}.error.{session}", record_path.display());
    let status = Command::new("bash")
        .arg("-e")
        .arg(&script)
        .env("FAKE_CLAUDE_FINAL_MESSAGE", &final_message)
        .env("FAKE_MUSE_ERROR", &error)
        .status()
        .unwrap();
    let code = exit_code(status);
    let message = fs::read_to_string(&final_message).unwrap_or_else(|_| "done".to_string());
    emit_event(
        &id,
        "run.output.delta",
        object([("text", string(&message))]),
    );
    if std::env::var("FAKE_MUSE_NO_LOG").is_err() {
        let dir = crate::env_path("HOME")
            .join(".local/share/muse/sessions/2026/10/06")
            .join(&id);
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join("session.jsonl");
        let previous = object([
            ("kind", string("assistant_message_committed")),
            ("text", string("earlier reply")),
        ]);
        let last = object([
            ("kind", string("assistant_message_committed")),
            ("text", string(&message)),
        ]);
        let usage = object([
            ("kind", string("model_completed")),
            (
                "usage",
                object([
                    ("input_tokens", crate::json::number(1200)),
                    ("cached_tokens", crate::json::number(200)),
                    ("output_tokens", crate::json::number(300)),
                    ("reasoning_tokens", crate::json::number(100)),
                ]),
            ),
        ]);
        let record = |event| {
            object([
                (
                    "stream",
                    object([("kind", string("session")), ("id", string(&id))]),
                ),
                ("payload_type", string("runtime.session")),
                (
                    "payload",
                    object([("kind", string("run")), ("event", event)]),
                ),
            ])
        };
        fs::write(
            &log,
            format!(
                "{}\n{}\n{}\n",
                record(previous),
                record(usage),
                record(last)
            ),
        )
        .unwrap();
    }
    if std::env::var("FAKE_MUSE_CORRUPT_LOG").is_ok() {
        let log = crate::env_path("HOME")
            .join(".local/share/muse/sessions/2026/10/06")
            .join(&id)
            .join("session.jsonl");
        fs::write(log, "truncated {").unwrap();
    }
    if code == 0 {
        // Muse's terminal text joins all replies; the session log doesn't.
        emit_event(
            &id,
            "run.terminal.completed",
            object([("text", string(format!("earlier reply{message}")))]),
        );
        crate::exit(0);
    }
    let message = fs::read_to_string(&error)
        .map(|message| message.trim_end().to_string())
        .unwrap_or_else(|_| format!("the agent's script exited {code}"));
    emit_event(
        &id,
        "run.terminal.failed",
        object([("reason", string(message))]),
    );
    crate::exit(1);
}

fn emit_event(id: &str, kind: &str, payload: Json) {
    emit(object([
        (
            "stream",
            object([("kind", string("session")), ("id", string(id))]),
        ),
        ("payload_type", string(kind)),
        ("payload", payload),
    ]));
}
