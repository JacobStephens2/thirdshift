//! OpenCode's JSONL stream and standalone session export, with stdin prompts.
use crate::claude::{beside_skills_snapshot, exit_code, git_here, script_for, skills_snapshot};
use crate::json::{Array, Json, Null, number, object, string};
use crate::muse::emit;
use crate::outlast_interrupts;
use std::fs;
use std::io::Read;
use std::process::Command;

pub fn main(argv: Vec<String>) {
    let record_path = crate::env_path("FAKE_OPENCODE_RECORD");
    let export_path = |id: &str| record_path.with_extension(format!("{id}.export.json"));
    if argv.starts_with(&["session".into(), "export".into()]) {
        crate::append_record(
            &record_path.with_extension("exports.json"),
            object([
                ("argv", Array(argv.iter().map(string).collect())),
                (
                    "no_auto_update",
                    string(std::env::var("OPENCODE_DISABLE_AUTOUPDATE").unwrap_or_default()),
                ),
            ]),
        );
        if std::env::var("FAKE_OPENCODE_NO_EXPORT").is_ok() {
            crate::exit(1);
        }
        if std::env::var("FAKE_OPENCODE_CORRUPT_EXPORT").is_ok() {
            println!("truncated {{");
            crate::exit(0);
        }
        let export_script = if argv.last().is_some_and(|id| id == "check-session") {
            "FAKE_OPENCODE_CHECK_EXPORT_SCRIPT"
        } else {
            "FAKE_OPENCODE_EXPORT_SCRIPT"
        };
        if let Ok(script) = std::env::var(export_script) {
            outlast_interrupts();
            let status = Command::new("bash").arg("-e").arg(script).status().unwrap();
            if !status.success() {
                crate::exit(exit_code(status));
            }
        }
        println!(
            "{}",
            fs::read_to_string(export_path(argv.last().unwrap())).unwrap()
        );
        crate::exit(0);
    }
    assert_eq!(argv[0], "run");
    assert!(argv.iter().any(|arg| arg == "--standalone"));
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).unwrap();
    let record = object([
        ("argv", Array(argv.iter().map(string).collect())),
        ("prompt", string(&prompt)),
        (
            "no_auto_update",
            string(std::env::var("OPENCODE_DISABLE_AUTOUPDATE").unwrap_or_default()),
        ),
        (
            "config_content",
            string(std::env::var("OPENCODE_CONFIG_CONTENT").unwrap_or_default()),
        ),
    ]);
    if prompt == "Reply with OK." {
        crate::append_record(&record_path.with_extension("checks.json"), record);
        if argv
            .iter()
            .any(|arg| arg.contains("bad-model") || arg.contains("#bogus"))
        {
            emit(object([
                ("type", string("error")),
                (
                    "error",
                    object([
                        ("type", string("provider.no-route")),
                        ("message", string("Model or variant unavailable")),
                    ]),
                ),
            ]));
            crate::exit(1);
        }
        crate::check_script();
        if let Ok(script) = std::env::var("FAKE_OPENCODE_CHECK_SCRIPT") {
            let status = Command::new("bash").arg("-e").arg(script).status().unwrap();
            if !status.success() {
                crate::exit(exit_code(status));
            }
        }
        event("check-session", "text", object([("text", string("OK"))]));
        fs::write(
            export_path("check-session"),
            object([
                (
                    "info",
                    object([(
                        "outcome",
                        string(
                            if std::env::var("FAKE_OPENCODE_CHECK_EXPORT_FAILED").is_ok() {
                                "failed"
                            } else {
                                "succeeded"
                            },
                        ),
                    )]),
                ),
                ("messages", Array(vec![])),
            ])
            .dump(),
        )
        .unwrap();
        crate::exit(0);
    }
    let (status, _) = git_here(&["status", "--porcelain"]);
    let cwd = std::env::current_dir().unwrap();
    let mut record = record;
    record.set("cwd", string(cwd.to_str().unwrap()));
    record.set("git_status", string(status));
    record.set("skill_files", skills_snapshot(".agents/skills"));
    record.set("beside_skills", beside_skills_snapshot(".agents/skills"));
    record.set(
        "agents_link",
        fs::read_link("AGENTS.md")
            .ok()
            .map(|path| string(path.to_str().unwrap()))
            .unwrap_or(Null),
    );
    record.set(
        "agents_text",
        fs::read_to_string("AGENTS.md")
            .ok()
            .map(string)
            .unwrap_or(Null),
    );
    let lock = crate::lock_beside(&record_path);
    let records = crate::append_record(&record_path, record);
    let records = records.items();
    let script = script_for(records);
    drop(lock);
    let session = records.len();
    let id = argv
        .windows(2)
        .find(|pair| pair[0] == "-s")
        .map(|pair| pair[1].clone())
        .unwrap_or_else(|| format!("fake-opencode-{session}"));
    outlast_interrupts();
    event(&id, "step_start", object([]));
    if let Some(rest) = prompt.strip_prefix("Load ") {
        let skill = rest.split_whitespace().next().unwrap();
        if std::env::var("FAKE_OPENCODE_SKIP_SKILL").is_err() {
            event(
                &id,
                "tool_use",
                object([
                    ("tool", string("skill")),
                    (
                        "state",
                        object([
                            ("status", string("completed")),
                            ("input", object([("id", string(skill))])),
                        ]),
                    ),
                ]),
            );
        }
    }
    let final_message = format!("{}.final-message.{session}", record_path.display());
    let error = format!("{}.error.{session}", record_path.display());
    let status = Command::new("bash")
        .arg("-e")
        .arg(script)
        .env("FAKE_CLAUDE_FINAL_MESSAGE", &final_message)
        .env("FAKE_OPENCODE_ERROR", &error)
        .status()
        .unwrap();
    let code = exit_code(status);
    let message = fs::read_to_string(&final_message).unwrap_or_else(|_| "done".into());
    event(&id, "text", object([("text", string("earlier reply"))]));
    event(&id, "text", object([("text", string(&message))]));
    let failed = code != 0 || std::env::var("FAKE_OPENCODE_EXPORT_FAILED").is_ok();
    let error = fs::read_to_string(error).unwrap_or_else(|_| "OpenCode's turn failed".into());
    let assistant = |text: &str| {
        object([
            ("type", string("assistant")),
            (
                "tokens",
                object([
                    ("input", number(1200)),
                    ("output", number(300)),
                    ("reasoning", number(100)),
                    (
                        "cache",
                        object([("read", number(200)), ("write", number(0))]),
                    ),
                ]),
            ),
            ("cost", number(0)),
            (
                "content",
                Array(vec![object([
                    ("type", string("text")),
                    ("text", string(text)),
                ])]),
            ),
        ])
    };
    fs::write(
        export_path(&id),
        object([
            (
                "info",
                object([
                    (
                        "outcome",
                        string(if failed { "failed" } else { "succeeded" }),
                    ),
                    ("tokens", object([("input", number(999999))])),
                    ("error", object([("message", string(&error))])),
                ]),
            ),
            (
                "messages",
                Array(vec![
                    object([("type", string("user")), ("text", string("prompt"))]),
                    assistant("earlier reply"),
                    assistant(&message),
                ]),
            ),
        ])
        .dump(),
    )
    .unwrap();
    // Alternate complete and dropped closing events, as real OpenCode does.
    if session.is_multiple_of(2) {
        event(
            &id,
            "step_finish",
            object([
                ("reason", string("stop")),
                (
                    "tokens",
                    object([("input", number(1200)), ("output", number(300))]),
                ),
            ]),
        );
    }
    if code != 0 {
        emit(object([
            ("type", string("error")),
            ("sessionID", string(&id)),
            ("error", object([("message", string(error))])),
        ]));
    }
    crate::exit(code);
}

fn event(id: &str, kind: &str, part: Json) {
    emit(object([
        ("type", string(kind)),
        (
            "sessionID",
            string(if std::env::var("FAKE_OPENCODE_NO_SESSION_ID").is_ok() {
                ""
            } else {
                id
            }),
        ),
        ("part", part),
    ]));
}
