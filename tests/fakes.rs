//! The fake `gh`, `claude`, `codex`, `agy`, `grok`, `muse` and `opencode` the end-to-end tests put first on
//! PATH: one executable, which acts as whichever its name says. See
//! `fakes/gh.rs`, `fakes/claude.rs` and `fakes/codex.rs` for what each does,
//! and `support/fakes.rs` for how the harness builds it.
//!
//! It uses only the standard library, so the harness can build it with plain
//! `rustc`. It started out as two Python scripts, whose startup cost was about
//! half the suite's CPU time (#207); where it mimics Python, such as in how
//! it lays out JSON, that is to keep their contract.
//!
//! Cargo also builds this file as a test target, which runs the JSON
//! module's tests and puts the fakes under clippy and rustfmt.

#![cfg_attr(test, allow(dead_code))]

#[path = "fakes/agy.rs"]
mod agy;
#[path = "fakes/claude.rs"]
mod claude;
#[path = "fakes/codex.rs"]
mod codex;
#[path = "fakes/gh.rs"]
mod gh;
#[path = "fakes/grok.rs"]
mod grok;
#[path = "fakes/json.rs"]
mod json;
#[path = "fakes/muse.rs"]
mod muse;
#[path = "fakes/opencode.rs"]
mod opencode;

use std::fs::{self, File};
use std::io::Write;
use std::os::fd::AsFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use json::{Array, Json};

/// Whether stdin is `/dev/null`, by `fstat` on it, as macOS has no `/proc`.
fn stdin_is_null() -> bool {
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

/// A finite check fixture, optionally leaving its captured pipes open after
/// the CLI exits. Tests record every owned PID rather than finding by name.
fn check_script() {
    let Ok(script) = std::env::var("FAKE_CHECK_SCRIPT") else {
        return;
    };
    writeln!(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(env_path("FAKE_CHECK_PIDS"))
            .unwrap(),
        "{}",
        std::process::id()
    )
    .unwrap();
    let mut child = Command::new("bash")
        .arg("-e")
        .arg(script)
        .env("FAKE_CHECK_CLI_PID", std::process::id().to_string())
        .spawn()
        .unwrap();
    if std::env::var_os("FAKE_CHECK_EXIT_EARLY").is_some() {
        exit(0);
    }
    let status = child.wait().unwrap();
    if !status.success() {
        exit(claude::exit_code(status));
    }
}

/// Print `message` to stderr and exit with `code`, as the fakes fail.
fn die(message: &str, code: i32) -> ! {
    eprintln!("{message}");
    exit(code)
}

/// Exit with `code`, once stdout is flushed.
fn exit(code: i32) -> ! {
    std::io::stdout().flush().unwrap();
    std::process::exit(code)
}

/// The path in the environment variable `name`, which the harness always sets.
fn env_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .unwrap_or_else(|| panic!("{name} is not set"))
        .into()
}

/// Run git in `repo`, returning the completed process.
fn git(repo: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap()
}

fn read_json(path: &Path) -> Json {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    json::parse(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Wait for an exclusive lock on `<path>.lock`, held until the file returned
/// is dropped.
fn lock_beside(path: &Path) -> File {
    let lock = File::create(format!("{}.lock", path.display())).unwrap();
    lock.lock().unwrap();
    lock
}

/// Append `entry` to the JSON list in the record file at `path`, starting
/// the list if there is none, and return the list.
fn append_record(path: &Path, entry: Json) -> Json {
    let mut records = if path.exists() {
        read_json(path)
    } else {
        Array(Vec::new())
    };
    records.items_mut().push(entry);
    fs::write(path, records.dump()).unwrap();
    records
}

/// `items` as Python shows a list of strings, for messages the Python fakes
/// wrote that way.
fn python_list(items: &[impl AsRef<str>]) -> String {
    let quoted: Vec<String> = items
        .iter()
        .map(|item| item.as_ref())
        .map(|item| format!("'{}'", item.replace('\\', "\\\\").replace('\'', "\\'")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

fn main() {
    // A fake that trips over something its contract doesn't cover fails with
    // exit code 1, as an uncaught Python exception did, not Rust's 101.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("fake: {info}");
        exit(1);
    }));
    let mut args = std::env::args();
    let name = args.next().expect("no argv[0]");
    let name = Path::new(&name).file_name().unwrap().to_str().unwrap();
    match name {
        "gh" => gh::main(args.collect()),
        "claude" => claude::main(args.collect()),
        "codex" => codex::main(args.collect()),
        "agy" => agy::main(args.collect()),
        "grok" => grok::main(args.collect()),
        "muse" => muse::main(args.collect()),
        "opencode" => opencode::main(args.collect()),
        "detached-command" => {
            let command = args.next().expect("no detached command");
            let mut command = Command::new(command);
            command.args(args);
            // SAFETY: setsid is async-signal-safe and accesses no Rust state.
            unsafe {
                command.pre_exec(|| {
                    unsafe extern "C" {
                        fn setsid() -> i32;
                    }
                    if setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let error = command.exec();
            die(&format!("could not start detached command: {error}"), 1);
        }
        _ => die(&format!("fake: no fake is called {name}"), 2),
    }
    exit(0);
}
