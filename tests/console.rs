//! The Console's public seam: the binary, fixture records and HTTP.
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

struct Console {
    child: Child,
    address: String,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

impl Console {
    fn start(binary: &Path, home: &Path) -> Self {
        let mut child = Command::new(binary)
            .args(["console", "--no-sign-in"])
            .env("HOME", home)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut console = Self {
            child,
            address: String::new(),
            lines,
            reader: Some(reader),
        };
        console.address = console
            .lines
            .recv_timeout(Duration::from_secs(10))
            .expect("Console startup");
        assert!(
            console.address.starts_with("http://127.0.0.1:"),
            "{}",
            console.address
        );
        console
    }

    fn get(&self, path: &str) -> reqwest::blocking::Response {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(format!("{}{path}", self.address))
            .send()
            .unwrap()
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reader.take().unwrap().join().unwrap();
    }
}

fn configure(home: &Path) {
    fs::create_dir_all(home.join(".thirdshift")).unwrap();
    fs::write(home.join(".thirdshift/config.toml"),
        "[console]\nport = 0\nother_consoles = [\"https://other.example.test\"]\ntheme = \"focus-auto\"\n").unwrap();
}

fn record(home: &Path, number: u64, outcome: Option<&str>, days: i64) -> Value {
    let time = (chrono::Local::now() - chrono::Duration::days(days)).to_rfc3339();
    let record = json!({
        "version": 1, "kind": "Spec run", "repository": "acme/widgets",
        "issue": {"number": number, "url": format!("https://github.com/acme/widgets/issues/{number}"), "title": "Widgets <script>alert(1)</script>"},
        "hostname": "fixture", "process_id": 0, "process_start": "gone",
        "harness": "codex", "model": "gpt-6", "effort": "high",
        "started_at": time, "ended_at": outcome.map(|_| time.clone()),
        "stages": [{"name": "implement", "began_at": time, "session_log": null}], "session_log": null,
        "outcome": outcome, "cause": null, "issue_branch": null,
        "pull_request": if outcome == Some("ready for review") { Some("https://github.com/acme/widgets/pull/10") } else { None },
        "tickets": [{"issue": {"number": 99, "url": "https://github.com/acme/widgets/issues/99", "title": "Next"}, "state": "waiting", "blockers": [{"number": 98, "url": "https://github.com/acme/widgets/issues/98", "title": "First"}]}]
    });
    write_record(home, number, &record);
    record
}

fn write_record(home: &Path, number: u64, record: &Value) {
    let dir = home.join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{number}-fixture.command.json")),
        serde_json::to_vec(record).unwrap(),
    )
    .unwrap();
}

#[test]
fn inbox_over_http_matches_status_and_embeds_only_local_resources() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 1, None, 0);
    record(home.path(), 2, Some("ready for review"), 0);
    record(home.path(), 3, Some("merged"), 0);
    record(home.path(), 4, Some("merged"), 2);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let response = console.get("/");
    assert_eq!(response.status(), 200);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("text/html")
    );
    let html = response.text().unwrap();
    let status = Command::new(binary)
        .args(["status", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    let groups: Vec<_> = html
        .split("data-group=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap())
        .collect();
    assert_eq!(
        groups,
        [
            "needs you",
            "ready for review",
            "landing",
            "working",
            "done",
            "resolved"
        ]
    );
    let subjects: Vec<_> = html
        .split("<span class=\"subject\">")
        .skip(1)
        .map(|rest| rest.split("</span>").next().unwrap())
        .collect();
    let expected: Vec<_> = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            format!(
                "{}#{}",
                item["repository"].as_str().unwrap(),
                item["issue"]["number"]
            )
        })
        .collect();
    assert_eq!(subjects, expected);
    for item in status["commands"].as_array().unwrap() {
        let number = item["issue"]["number"].as_u64().unwrap();
        let group = item["group"].as_str().unwrap();
        let section = html
            .split(&format!("data-group=\"{group}\""))
            .nth(1)
            .unwrap()
            .split("</details>")
            .next()
            .unwrap();
        assert!(
            section.contains(&format!("acme/widgets#{number}")),
            "{section}"
        );
    }
    assert!(html.contains("thirdshift status"));
    assert!(html.contains(status["hostname"].as_str().unwrap()));
    assert!(html.contains("https://other.example.test"));
    assert!(html.contains("Ticket #99 · waiting on #98"));
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("Died"));
    assert!(html.contains("died at implement"));
    for path in [
        "/assets/console.css",
        "/assets/console.js",
        "/assets/vendor/htmx-2.0.7.min.js",
        "/assets/vendor/htmx-ext-sse-2.2.3.js",
        "/assets/vendor/idiomorph-0.7.4.min.js",
        "/assets/fonts/ibm-plex-sans-latin.woff2",
        "/assets/fonts/ibm-plex-mono-latin.woff2",
    ] {
        let response = console.get(path);
        assert_eq!(response.status(), 200, "{path}");
        assert!(!response.bytes().unwrap().is_empty());
    }
    assert_eq!(console.get("/assets/../../Cargo.toml").status(), 404);
    assert_eq!(console.get("/missing").status(), 404);
}

#[test]
fn child_runs_nest_and_each_request_reads_current_records() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 7, Some("failed"), 0);
    let mut child = record(home.path(), 8, Some("merged"), 0);
    child["parent_record"] = json!(
        home.path()
            .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.command.json")
    );
    write_record(home.path(), 8, &child);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let nested = html.split("class=\"children\"").nth(1).expect(&html);
    assert!(
        nested
            .split("</ul>")
            .next()
            .unwrap()
            .contains("acme/widgets#8")
    );
    let mut updated = record(home.path(), 7, Some("merged"), 0);
    updated["issue"]["title"] = json!("Changed after startup");
    write_record(home.path(), 7, &updated);
    assert!(
        console
            .get("/")
            .text()
            .unwrap()
            .contains("Changed after startup")
    );
}

#[test]
fn console_requires_explicit_no_sign_in_and_rejects_invalid_settings() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let refused = Command::new(binary)
        .arg("console")
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--no-sign-in"));
    for settings in [
        "port = -1",
        "port = 65536",
        "other_consoles = ['javascript:alert(1)']",
        "theme = 'typo'",
        "bind = '0.0.0.0'",
    ] {
        fs::write(
            home.path().join(".thirdshift/config.toml"),
            format!("[console]\n{settings}\n"),
        )
        .unwrap();
        let refused = Command::new(binary)
            .args(["console", "--no-sign-in"])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(!refused.status.success(), "{settings}");
        assert!(String::from_utf8_lossy(&refused.stderr).contains("[console]"));
    }
}

#[cfg(unix)]
#[test]
fn replacing_the_binary_executes_the_replacement_with_the_original_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let binary = home.path().join("thirdshift");
    // A child forked by another test can inherit fs::copy's writer and keep
    // the executable "Text file busy". Only the copying process opens it here.
    let copied = Command::new("cp")
        .arg(env!("CARGO_BIN_EXE_thirdshift"))
        .arg(&binary)
        .status()
        .unwrap();
    assert!(copied.success());
    let console = Console::start(&binary, home.path());
    assert_eq!(console.get("/").status(), 200);
    let replacement = home.path().join("replacement");
    fs::write(&replacement, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/reexec-args\"\nprintf 'replacement executed\\n'\n").unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&replacement, &binary).unwrap();
    assert_eq!(
        console.lines.recv_timeout(Duration::from_secs(10)).unwrap(),
        "replacement executed"
    );
    assert_eq!(
        fs::read_to_string(home.path().join("reexec-args")).unwrap(),
        "console\n--no-sign-in\n"
    );
}
