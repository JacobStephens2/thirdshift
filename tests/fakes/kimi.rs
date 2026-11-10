//! Fake Kimi Code: `provider list --json`, and `-p` sessions printing Kimi's
//! `stream-json` lines while recording usage only in the session's
//! `wire.jsonl` under `KIMI_CODE_HOME`, as Kimi Code 2.5.2 does.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use crate::claude::{beside_skills_snapshot, exit_code, git_here, script_for, skills_snapshot};
use crate::json::{Array, Bool, Null, number, object, string};
use crate::{outlast_interrupts, stdin_is_null};

pub const PROVIDERS: &str = include_str!("../fixtures/kimi-providers.json");

/// Each step's uncached input, output, cache reads and cache writes, as the
/// session's `step.end` events record them.
const STEPS: [[u64; 4]; 3] = [
    [1200, 150, 4096, 512],
    [300, 80, 5120, 0],
    [45, 20, 5376, 64],
];

fn after<'a>(argv: &'a [String], flag: &str) -> Option<&'a String> {
    argv.iter()
        .position(|arg| arg == flag)
        .and_then(|at| argv.get(at + 1))
}

fn env(name: &str) -> crate::json::Json {
    std::env::var(name).map(string).unwrap_or(Null)
}

fn line(json: crate::json::Json) {
    println!("{json}");
    std::io::stdout().flush().unwrap();
}

pub fn main(argv: Vec<String>) {
    let catalog = argv.starts_with(&["provider".into(), "list".into(), "--json".into()]);
    let prompt = after(&argv, "-p").cloned();
    if !catalog && prompt.is_none() {
        crate::die("fake kimi: expected provider list --json or -p", 2);
    }
    // A release summary's input is its agent file's instructions.
    let agent = after(&argv, "--agent-file").map(|path| fs::read_to_string(path).unwrap());
    let (agent_tools, instructions) = match agent.as_deref().map(|text| {
        let (front, body) = text
            .strip_prefix("---\n")
            .and_then(|text| text.split_once("\n---\n"))
            .expect("fake kimi: an agent file starts with frontmatter");
        let tools = front.lines().find_map(|line| line.strip_prefix("tools: "));
        (
            tools.map(string),
            body.trim_start_matches('\n').strip_suffix('\n'),
        )
    }) {
        Some((tools, body)) => (tools.unwrap_or(Null), body.map(string)),
        None => (Null, None),
    };
    let record_path = crate::env_path("FAKE_KIMI_RECORD");
    let lock = crate::lock_beside(&record_path);
    let (status, _) = git_here(&["status", "--porcelain"]);
    let cwd = std::env::current_dir().unwrap();
    let records = crate::append_record(
        &record_path,
        object([
            ("argv", Array(argv.iter().map(string).collect())),
            (
                "prompt",
                instructions
                    .or_else(|| prompt.as_ref().map(string))
                    .unwrap_or(Null),
            ),
            ("agent_tools", agent_tools),
            ("cwd", string(cwd.to_str().unwrap())),
            ("stdin_null", Bool(stdin_is_null())),
            ("git_status", string(status)),
            ("skill_files", skills_snapshot(".agents/skills")),
            ("beside_skills", beside_skills_snapshot(".agents/skills")),
            ("KIMI_CODE_HOME", env("KIMI_CODE_HOME")),
            (
                "KIMI_CODE_TRUST_WORKSPACE",
                env("KIMI_CODE_TRUST_WORKSPACE"),
            ),
            ("KIMI_CODE_NO_AUTO_UPDATE", env("KIMI_CODE_NO_AUTO_UPDATE")),
            ("KIMI_DISABLE_TELEMETRY", env("KIMI_DISABLE_TELEMETRY")),
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
        crate::check_script();
        let providers = std::env::var_os("FAKE_KIMI_PROVIDERS")
            .map(|path| fs::read_to_string(path).unwrap())
            .unwrap_or_else(|| PROVIDERS.to_string());
        println!("{providers}");
        crate::exit(0);
    }
    let prompt = prompt.unwrap();
    outlast_interrupts();
    let session = sessions.len();
    let script = script_for(&sessions);
    let id = after(&argv, "--session")
        .cloned()
        .unwrap_or_else(|| format!("fake-kimi-{session}"));
    line(object([
        ("role", string("meta")),
        ("type", string("system.version")),
        (
            "version",
            string(std::env::var("FAKE_HARNESS_VERSION").unwrap_or("9.8.7".into())),
        ),
    ]));
    if let Some(rest) = prompt.strip_prefix("Load ")
        && std::env::var_os("FAKE_KIMI_SKIP_SKILL").is_none()
    {
        let skill = rest.split_whitespace().next().unwrap();
        line(object([
            ("role", string("assistant")),
            (
                "tool_calls",
                Array(vec![object([
                    ("type", string("function")),
                    ("id", string("call_skill")),
                    (
                        "function",
                        object([
                            ("name", string("Skill")),
                            (
                                "arguments",
                                string(object([("skill", string(skill))]).to_string()),
                            ),
                        ]),
                    ),
                ])]),
            ),
        ]));
        line(object([
            ("role", string("tool")),
            ("tool_call_id", string("call_skill")),
            (
                "content",
                string(format!(
                    "Skill \"{skill}\" loaded inline. Follow its instructions."
                )),
            ),
        ]));
    }
    let final_message = format!("{}.final-message.{session}", record_path.display());
    let status = Command::new("bash")
        .arg("-e")
        .arg(script)
        .env("FAKE_CLAUDE_FINAL_MESSAGE", &final_message)
        .status()
        .unwrap();
    let code = exit_code(status);
    write_wire(&id, after(&argv, "-m").map(String::as_str));
    if let Ok(message) = fs::read_to_string(&final_message) {
        line(object([
            ("role", string("assistant")),
            ("content", string(message)),
        ]));
    }
    if code == 0 {
        line(object([
            ("role", string("meta")),
            ("type", string("session.resume_hint")),
            ("session_id", string(&id)),
            ("command", string(format!("kimi -r {id}"))),
        ]));
    }
    crate::exit(code);
}

/// Append the session's main-agent record: the Model alias it is bound to,
/// then a `step.end` with usage for each step, or without usage under
/// `FAKE_KIMI_NO_USAGE`. A Resume appends to the same record, as Kimi does.
fn write_wire(id: &str, model: Option<&str>) {
    let home =
        PathBuf::from(std::env::var_os("KIMI_CODE_HOME").expect("KIMI_CODE_HOME is not set"));
    let dir = home.join("sessions/wd-fake").join(id).join("agents/main");
    fs::create_dir_all(&dir).unwrap();
    let mut wire = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("wire.jsonl"))
        .unwrap();
    let alias = model.unwrap_or("kimi-code/kimi-for-coding");
    let mut lines = vec![object([
        ("type", string("profile.bind")),
        ("agentId", string("main")),
        ("modelAlias", string(alias)),
    ])];
    let without_usage = std::env::var_os("FAKE_KIMI_NO_USAGE").is_some();
    for (step, [other, output, read, write]) in STEPS.into_iter().enumerate() {
        let mut event = object([
            ("type", string("step.end")),
            ("uuid", string(format!("step-{}", step + 1))),
            ("turnId", string("1")),
            ("step", number(step + 1)),
            ("finishReason", string("completed")),
        ]);
        if !without_usage {
            event.set(
                "usage",
                object([
                    ("inputOther", number(other)),
                    ("output", number(output)),
                    ("inputCacheRead", number(read)),
                    ("inputCacheCreation", number(write)),
                ]),
            );
        }
        lines.push(object([
            ("type", string("context.append_loop_event")),
            ("agentId", string("main")),
            ("event", event),
        ]));
    }
    for record in lines {
        writeln!(wire, "{record}").unwrap();
    }
}
