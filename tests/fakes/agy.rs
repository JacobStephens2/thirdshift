//! Fake Antigravity CLI: a free tab-separated catalog and recorded agy
//! stream envelopes, running the same per-session scripts as fake Claude.
use std::fs;
use std::io::Write;
use std::process::Command;

use crate::claude::{exit_code, git_here, script_for, skills_snapshot, transported_prompt};
use crate::json::{Array, Bool, Json, Null, number, object, string};
use crate::stdin_is_null;

const CATALOG: &str = "gemini-3.8-flash-high\tGemini 3.8 Flash (High)\ngemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)\ngemini-3.8-flash-low\tGemini 3.8 Flash (Low)\ngemini-3.1-pro-high\tGemini 3.1 Pro (High)\ngemini-3.1-pro-low\tGemini 3.1 Pro (Low)\n";

fn emit(event: Json) {
    println!("{event}");
    std::io::stdout().flush().unwrap();
}

pub fn main(argv: Vec<String>) {
    let catalog = argv == ["models"];
    let record_path = crate::env_path(if catalog {
        "FAKE_AGY_CHECK_RECORD"
    } else {
        "FAKE_AGY_RECORD"
    });
    let lock = crate::lock_beside(&record_path);
    let input = transported_prompt(&argv);
    let (branch, _) = git_here(&["branch", "--show-current"]);
    let (status, _) = git_here(&["status", "--porcelain"]);
    let records = crate::append_record(
        &record_path,
        object([
            ("argv", Array(argv.iter().map(string).collect())),
            (
                "prompt",
                if catalog {
                    Null
                } else {
                    input
                        .as_ref()
                        .or_else(|| argv.last())
                        .map(string)
                        .unwrap_or(Null)
                },
            ),
            ("branch", string(branch)),
            (
                "cwd",
                string(std::env::current_dir().unwrap().display().to_string()),
            ),
            ("stdin_null", Bool(stdin_is_null())),
            ("git_status", string(status)),
            (
                "auto_update",
                string(std::env::var("AGY_CLI_DISABLE_AUTO_UPDATE").unwrap_or_default()),
            ),
            (
                "skill_files",
                if catalog {
                    object([])
                } else {
                    skills_snapshot(".agents/skills")
                },
            ),
            (
                "gemini_link",
                fs::read_link("GEMINI.md")
                    .ok()
                    .map(|p| string(p.display().to_string()))
                    .unwrap_or(Null),
            ),
            (
                "gemini_text",
                fs::read_to_string("GEMINI.md")
                    .ok()
                    .map(string)
                    .unwrap_or(Null),
            ),
        ]),
    );
    drop(lock);
    if catalog {
        if let Ok(error) = std::env::var("FAKE_AGY_CATALOG_ERROR") {
            crate::die(&error, 1);
        }
        crate::check_script();
        print!("{CATALOG}");
        crate::exit(0);
    }
    assert!(argv.iter().any(|arg| arg == "-p" || arg == "-p="));
    let records = records.items();
    let session = records.len();
    let conversation = argv
        .windows(2)
        .find(|args| args[0] == "--conversation")
        .map(|args| args[1].clone())
        .unwrap_or_else(|| format!("fake-conversation-{session}"));
    emit(object([
        ("event", string("init")),
        ("conversation_id", string(&conversation)),
        (
            "init",
            object([("permission_mode", string("always-proceed"))]),
        ),
    ]));
    let final_message = format!("{}.final-message.{session}", record_path.display());
    let error = format!("{}.error.{session}", record_path.display());
    let result = format!("{}.result.{session}", record_path.display());
    let code = exit_code(
        Command::new("bash")
            .arg("-e")
            .arg(script_for(records))
            .env("FAKE_CLAUDE_FINAL_MESSAGE", &final_message)
            .env("FAKE_AGY_ERROR", &error)
            .env("FAKE_AGY_RESULT", &result)
            .status()
            .unwrap(),
    );
    if let Ok(result) = fs::read_to_string(result) {
        emit(crate::json::parse(&result).unwrap());
        crate::exit(code);
    }
    let response = fs::read_to_string(final_message).unwrap_or_default();
    let error =
        fs::read_to_string(error).unwrap_or_else(|_| format!("the agent's script exited {code}"));
    emit(object([
        ("event", string("result")),
        (
            "result",
            object([
                ("conversation_id", string(conversation)),
                (
                    "status",
                    string(if code == 0 { "SUCCESS" } else { "ERROR" }),
                ),
                ("response", string(response)),
                (
                    "error",
                    if code == 0 {
                        Null
                    } else {
                        string(error.trim_end())
                    },
                ),
                ("num_turns", number(1)),
                (
                    "usage",
                    object([
                        ("input_tokens", number(1200)),
                        ("cache_read_tokens", number(200)),
                        ("output_tokens", number(300)),
                        ("thinking_tokens", number(150)),
                        ("total_tokens", number(1500)),
                    ]),
                ),
            ]),
        ),
    ]));
    crate::exit(code);
}
