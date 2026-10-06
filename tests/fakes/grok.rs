//! Fake Grok Build: its text catalog, cache and Claude-shaped Messages stream.

use std::fs;
use std::io::Write;
use std::process::Command;

use crate::claude::{beside_skills_snapshot, exit_code, git_here, script_for, skills_snapshot};
use crate::json::{Array, Bool, Null, object, string};
use crate::{outlast_interrupts, stdin_is_null};

pub const MODELS: &str = include_str!("../fixtures/grok-models.txt");
const CACHE: &str = include_str!("../fixtures/grok-models.json");

pub fn main(argv: Vec<String>) {
    let catalog = argv.first().is_some_and(|arg| arg == "models");
    if !catalog && !argv.iter().any(|arg| arg == "-p") {
        crate::die("fake grok: expected models or -p", 2);
    }
    if !catalog {
        let prompt = argv
            .iter()
            .position(|arg| arg == "-p")
            .and_then(|at| argv.get(at + 1));
        if prompt.is_none_or(|prompt| prompt.starts_with('-')) {
            crate::die("a value is required for --single <PROMPT>", 2);
        }
    }
    let record_path = crate::env_path("FAKE_GROK_RECORD");
    let lock = crate::lock_beside(&record_path);
    let (branch, _) = git_here(&["branch", "--show-current"]);
    let (status, _) = git_here(&["status", "--porcelain"]);
    let cwd = std::env::current_dir().unwrap();
    let records = crate::append_record(
        &record_path,
        object([
            ("argv", Array(argv.iter().map(string).collect())),
            (
                "prompt",
                if catalog {
                    Null
                } else {
                    argv.last().map(string).unwrap_or(Null)
                },
            ),
            ("cwd", string(cwd.to_str().unwrap())),
            ("branch", string(branch)),
            ("stdin_null", Bool(stdin_is_null())),
            ("git_status", string(status)),
            ("skill_files", skills_snapshot(".agents/skills")),
            ("beside_skills", beside_skills_snapshot(".agents/skills")),
            (
                "GROK_DISABLE_AUTOUPDATER",
                string(std::env::var("GROK_DISABLE_AUTOUPDATER").unwrap_or_default()),
            ),
            (
                "GROK_FOLDER_TRUST",
                string(std::env::var("GROK_FOLDER_TRUST").unwrap_or_default()),
            ),
        ]),
    );
    let sessions: Vec<_> = records
        .items()
        .iter()
        .filter(|call| call.at("prompt") != &Null)
        .cloned()
        .collect();
    drop(lock);
    if catalog {
        let home = crate::env_path("HOME");
        let cache = home.join(".grok/models_cache.json");
        fs::create_dir_all(cache.parent().unwrap()).unwrap();
        fs::write(cache, CACHE).unwrap();
        println!("{MODELS}");
        crate::exit(0);
    }
    outlast_interrupts();
    let session = sessions.len();
    let script = script_for(&sessions);
    let id = argv
        .iter()
        .position(|arg| arg == "-r")
        .and_then(|at| argv.get(at + 1))
        .cloned()
        .unwrap_or_else(|| format!("fake-grok-{session}"));
    println!(
        "{}",
        object([
            ("type", string("system")),
            ("subtype", string("init")),
            ("session_id", string(&id)),
            ("cwd", string(cwd.to_str().unwrap())),
        ])
    );
    std::io::stdout().flush().unwrap();
    let final_message = format!("{}.final-message.{session}", record_path.display());
    let result = format!("{}.result.{session}", record_path.display());
    let status = Command::new("bash")
        .arg("-e")
        .arg(script)
        .env("FAKE_CLAUDE_FINAL_MESSAGE", &final_message)
        .env("FAKE_GROK_RESULT", &result)
        .status()
        .unwrap();
    let code = exit_code(status);
    if let Ok(result) = fs::read_to_string(result) {
        print!("{result}");
    } else {
        println!(
            "{}",
            object([
                ("type", string("result")),
                (
                    "subtype",
                    string(if code == 0 {
                        "success"
                    } else {
                        "error_during_execution"
                    })
                ),
                ("is_error", Bool(code != 0)),
                ("session_id", string(id)),
                (
                    "result",
                    string(fs::read_to_string(&final_message).unwrap_or_default())
                ),
                ("num_turns", crate::json::number(2)),
                ("total_cost_usd", crate::json::number(0.0127)),
                (
                    "usage",
                    object([
                        ("input_tokens", crate::json::number(812)),
                        ("output_tokens", crate::json::number(210)),
                        ("cache_read_input_tokens", crate::json::number(0)),
                    ])
                ),
            ])
        );
    }
    crate::exit(code);
}
