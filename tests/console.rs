//! The Console's public seam: the binary, fixture records and HTTP.
mod support;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{Value, json};

#[cfg(unix)]
#[path = "console/accounts.rs"]
mod accounts;

struct Console {
    child: Child,
    address: String,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
}

struct HeldFixture {
    run: Option<support::HeldRun>,
    release: std::path::PathBuf,
}

impl Drop for HeldFixture {
    fn drop(&mut self) {
        // Release and reap the fixture even when a regression assertion fails.
        let _ = fs::write(&self.release, "");
        if let Some(run) = self.run.take() {
            run.finish();
        }
    }
}

impl Console {
    fn start(binary: &Path, home: &Path) -> Self {
        Self::start_mode(binary, home, true)
    }

    fn start_mode(binary: &Path, home: &Path, no_sign_in: bool) -> Self {
        let args: &[&str] = if no_sign_in {
            &["console", "--no-sign-in"]
        } else {
            &["console"]
        };
        let mut child = Command::new(binary)
            .args(args)
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
fn command_changes_push_events_over_http() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = console.get("/events");
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    let mut stream = BufReader::new(response);
    fn changed(stream: &mut impl BufRead) {
        let mut event = String::new();
        loop {
            let mut line = String::new();
            assert!(stream.read_line(&mut line).unwrap() > 0, "stream ended");
            if line.trim().is_empty() && event.contains("event: commands") {
                assert!(event.contains("data: changed"), "{event}");
                return;
            }
            if line.trim().is_empty() {
                event.clear();
            } else {
                event.push_str(&line);
            }
        }
    }
    changed(&mut stream); // Reconnects always catch up, even if an event was missed.
    let mut fixture = record(home.path(), 81, None, 0);
    changed(&mut stream);
    assert!(console.get("/").text().unwrap().contains("acme/widgets#81"));
    fixture["stages"][0]["name"] = json!("CI watch");
    write_record(home.path(), 81, &fixture);
    changed(&mut stream);
    assert!(
        console
            .get("/")
            .text()
            .unwrap()
            .contains("died at CI watch")
    );
    fixture["outcome"] = json!("merged");
    fixture["ended_at"] = json!(chrono::Local::now().to_rfc3339());
    write_record(home.path(), 81, &fixture);
    changed(&mut stream);
    assert!(console.get("/").text().unwrap().contains(">merged</p>"));
}

#[test]
fn heartbeat_observes_log_lengths_and_is_flat_for_silent_runs() {
    let scenario = support::Scenario::new();
    scenario.user_config_is("[console]\nport = 0\n");
    scenario.agent_does(&format!(
        "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\nexit 1\n",
        scenario.path("started").display(),
        scenario.path("release").display()
    ));
    let _held = HeldFixture {
        run: Some(scenario.run_until(&[&scenario.issue_url(7)], &[], "started")),
        release: scenario.path("release"),
    };
    let dir = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    let path = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".command.json"))
        .unwrap();
    let mut fixture: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let session = std::path::PathBuf::from(fixture["session_log"].as_str().unwrap());
    let log = path.with_file_name(
        path.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .replace(".command.json", ".log"),
    );
    let began = (chrono::Local::now() - chrono::Duration::minutes(20)).to_rfc3339();
    fixture["stages"] = json!([{"name": "implement", "began_at": began, "session_log": session}]);
    fs::write(&path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    fs::write(&log, "a").unwrap();
    fs::write(&session, "ab").unwrap();
    let console = Console::start(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        &scenario.path("home"),
    );
    let html = console.get("/").text().unwrap();
    assert!(html.contains("data-log-growth=\"1:2\""), "{html}");
    assert!(html.contains("data-logging=\"true\""));
    fs::write(&session, "abcde").unwrap();
    assert!(
        console
            .get("/")
            .text()
            .unwrap()
            .contains("data-log-growth=\"1:5\"")
    );
    for file in [&log, &session] {
        fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(std::time::SystemTime::now() - Duration::from_secs(20 * 60)),
            )
            .unwrap();
    }
    let html = console.get("/").text().unwrap();
    assert!(html.contains("data-state=\"Silent\""));
    assert!(html.contains("data-logging=\"false\""));
    assert!(html.contains("data-log-growth=\"1:5\""));
    drop(console);
}

#[test]
fn history_keeps_every_ended_command_in_time_order_and_filters() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 1, Some("merged"), 80);
    let mut failed = record(home.path(), 2, Some("failed"), 2);
    failed["cause"] = json!("same failure");
    write_record(home.path(), 2, &failed);
    failed["issue"]["number"] = json!(3);
    failed["started_at"] = json!(chrono::Local::now().to_rfc3339());
    failed["ended_at"] = failed["started_at"].clone();
    write_record(home.path(), 3, &failed);
    record(home.path(), 4, None, 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/?page=history").text().unwrap();
    assert!(html.contains("<h2>History</h2>"));
    assert!(html.find("acme/widgets#3").unwrap() < html.find("acme/widgets#2").unwrap());
    assert!(html.find("acme/widgets#2").unwrap() < html.find("acme/widgets#1").unwrap());
    // A missing process with no outcome is a Command that died, retained too.
    assert!(html.contains("acme/widgets#4"));
    let filtered = console
        .get("/?page=history&outcome=merged&repository=acme%2Fwidgets")
        .text()
        .unwrap();
    assert!(filtered.contains("acme/widgets#1"));
    assert!(!filtered.contains("acme/widgets#2"));
    assert!(
        !console
            .get("/?page=history&repository=elsewhere")
            .text()
            .unwrap()
            .contains("acme/widgets#1")
    );
    assert_eq!(console.get("/?page=waterfall").status(), 200);
}

#[test]
fn waterfall_matches_claude_calls_to_command_times_and_keeps_unmatched_calls() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut value = record(home.path(), 7, Some("merged"), 0);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("7-20261001T235950+0000-implement.jsonl");
    fs::write(&session, concat!(
        "{\"type\":\"system\",\"subtype\":\"init\",\"cwd\":\"/work\"}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"id\":\"a\",\"name\":\"Bash\",\"input\":{\"command\":\"rg widgets\"}},{\"type\":\"tool_use\",\"id\":\"b\",\"name\":\"Read\",\"input\":{\"file_path\":\"/work/missing.rs\"}},{\"type\":\"tool_use\",\"id\":\"c\",\"name\":\"Bash\",\"input\":{\"command\":\"rg widgets\"}}]}}\n"
    )).unwrap();
    fs::write(dir.join("7-fixture.log"), "thirdshift: 23:59:50 starting\nthirdshift: 23:59:55 implement: $ rg widgets\nthirdshift: 00:00:05 implement: $ rg widgets\nthirdshift: 00:00:10 finished\n").unwrap();
    value["harness"] = json!("claude");
    // Records retain fractions; Command logs quantize calls to whole seconds.
    value["started_at"] = json!("2026-10-01T23:59:55.100+00:00");
    value["ended_at"] = json!("2026-10-02T00:00:10+00:00");
    value["stages"] =
        json!([{ "name": "implement", "began_at": value["started_at"], "session_log": session }]);
    value["session_log"] = json!(session);
    write_record(home.path(), 7, &value);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console
        .get("/waterfall?command=widgets%237")
        .text()
        .unwrap();
    assert!(html.contains("<h2>Waterfall</h2>"), "{html}");
    assert_eq!(
        html.matches("class=\"waterfall-tool\"").count(),
        3,
        "{html}"
    );
    assert!(
        html.contains("data-start=\"2026-10-01T23:59:55+00:00\" data-seconds=\"10\""),
        "{html}"
    );
    assert!(html.contains("data-start=\"2026-10-02T00:00:05+00:00\" data-seconds=\"5\""));
    assert!(html.contains("Read missing.rs"));
    assert!(html.contains("Time unknown"));
    let show = Command::new(binary)
        .args(["show", "widgets#7", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(show.status.success());
    let detail: Value = serde_json::from_slice(&show.stdout).unwrap();
    assert!(html.contains(detail["command"]["stages"][0]["began_at"].as_str().unwrap()));
    let logs = Command::new(binary)
        .args(["logs", "widgets#7"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(logs.status.success());
    assert!(
        String::from_utf8(logs.stdout)
            .unwrap()
            .contains("23:59:55 implement: $ rg widgets")
    );
}

#[test]
fn waterfall_nests_ticket_runs_and_matches_codex_items_once_in_their_scope() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut parent = record(home.path(), 7, Some("merged"), 0);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = dir.join("99-20261001T120000+0000-implement.jsonl");
    fs::write(&session, concat!(
        "{\"type\":\"thread.started\",\"thread_id\":\"fixture\"}\n",
        "{\"type\":\"item.started\",\"item\":{\"id\":\"a\",\"type\":\"command_execution\",\"command\":\"/bin/bash -lc 'rg widgets'\",\"status\":\"in_progress\"}}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"b\",\"type\":\"file_change\",\"changes\":[{\"path\":\"/work/src/widgets.rs\",\"kind\":\"update\"}]}}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"a\",\"type\":\"command_execution\",\"command\":\"/bin/bash -lc 'rg widgets'\",\"status\":\"completed\",\"exit_code\":0}}\n",
        "{\"type\":\"item.started\",\"item\":{\"id\":\"c\",\"type\":\"mcp_tool_call\",\"server\":\"github\",\"tool\":\"get_issue\",\"status\":\"in_progress\"}}\n",
        "{\"type\":\"item.completed\",\"item\":{\"id\":\"c\",\"type\":\"mcp_tool_call\",\"server\":\"github\",\"tool\":\"get_issue\",\"status\":\"completed\"}}\n"
    )).unwrap();
    parent["started_at"] = json!("2026-10-01T12:00:00+00:00");
    parent["ended_at"] = json!("2026-10-01T12:01:00+00:00");
    parent["stages"] =
        json!([{ "name": "Tickets", "began_at": parent["started_at"], "session_log": null }]);
    write_record(home.path(), 7, &parent);
    let mut child = parent.clone();
    child["kind"] = json!("Run");
    child["name"] = json!("acme/widgets#99");
    child["issue"]["number"] = json!(99);
    child["parent_record"] = json!(dir.join("7-fixture.command.json"));
    child["started_at"] = json!("2026-10-01T12:00:10+00:00");
    child["ended_at"] = json!("2026-10-01T12:00:40+00:00");
    child["stages"] = json!([
        { "name": "Setup", "began_at": child["started_at"], "session_log": null },
        { "name": "implement", "began_at": "2026-10-01T12:00:15.100+00:00", "session_log": session }
    ]);
    child["session_log"] = json!(session);
    fs::write(
        dir.join("7-fixture.issue-99.child.json"),
        serde_json::to_vec(&child).unwrap(),
    )
    .unwrap();
    fs::write(dir.join("7-fixture.log"), "thirdshift: 12:00:00 starting\nthirdshift: 12:00:11 #98: implement: $ rg widgets\nthirdshift: 12:00:15 #99: implement: $ rg widgets\nthirdshift: 12:00:20 #99: implement: Edit src/widgets.rs\nthirdshift: 12:00:30 #99: implement: github get_issue\nthirdshift: 12:01:00 finished\n").unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console
        .get("/waterfall?command=widgets%237")
        .text()
        .unwrap();
    assert!(html.contains("data-depth=\"1\""), "{html}");
    assert!(html.contains("acme/widgets#99"));
    assert_eq!(
        html.matches("class=\"waterfall-tool\"").count(),
        3,
        "{html}"
    );
    let tool_stage = html
        .split("<li class=\"waterfall-stage\">")
        .find(|stage| stage.contains("data-start=\"2026-10-01T12:00:15+00:00\""))
        .unwrap();
    assert!(tool_stage.contains("<h4>implement</h4>"), "{tool_stage}");
    for (start, seconds) in [("15", 5), ("20", 10), ("30", 10)] {
        assert!(
            html.contains(&format!(
                "data-start=\"2026-10-01T12:00:{start}+00:00\" data-seconds=\"{seconds}\""
            )),
            "{html}"
        );
    }
    let output = Command::new(binary)
        .args(["show", "widgets#7", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let show: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(show["command"]["children"][0]["issue"]["number"], 99);
    assert_eq!(console.get("/?page=4").status(), 200);
    let single = console
        .get("/waterfall?command=widgets%2399")
        .text()
        .unwrap();
    assert_eq!(single.matches("class=\"waterfall-tool\"").count(), 3);
    assert!(
        single.contains("data-start=\"2026-10-01T12:00:15+00:00\" data-seconds=\"5\""),
        "{single}"
    );
}

#[test]
fn time_pages_pair_with_all_status_stages_and_repository_usual_ranges() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let now = chrono::Local::now();
    for number in 1..=4 {
        let mut value = record(home.path(), number, Some("merged"), 0);
        value["started_at"] = json!((now - chrono::Duration::hours(number as i64)).to_rfc3339());
        let began = value["started_at"].as_str().unwrap().to_owned();
        let middle = (chrono::DateTime::parse_from_rfc3339(&began).unwrap()
            + chrono::Duration::minutes(10))
        .to_rfc3339();
        let end = (chrono::DateTime::parse_from_rfc3339(&middle).unwrap()
            + chrono::Duration::minutes(20))
        .to_rfc3339();
        value["stages"] = json!([
            {"name": "implement", "began_at": began, "session_log": null},
            {"name": "CI watch", "began_at": middle, "session_log": null}
        ]);
        value["ended_at"] = json!(end);
        write_record(home.path(), number, &value);
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let output = Command::new(binary)
        .args(["status", "--all", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    let stages = console.get("/stages").text().unwrap();
    assert!(stages.contains("<h2>Stages</h2>"), "{stages}");
    assert!(stages.contains("thirdshift status --all --json"));
    for command in status["commands"].as_array().unwrap() {
        let number = command["issue"]["number"].as_u64().unwrap();
        assert!(stages.contains(&format!("acme/widgets#{number}")));
        assert_eq!(command["usual_ranges"]["implement"]["lower_seconds"], 600.0);
        assert_eq!(command["usual_ranges"]["CI watch"]["upper_seconds"], 1200.0);
    }
    assert!(stages.contains("Usual range"));
    assert!(stages.contains("10–10 min"));
    assert!(stages.contains("20–20 min"));
    assert_eq!(stages.matches("Complete · 10 min").count(), 4);
    assert_eq!(stages.matches("Complete · 20 min").count(), 4);
    for (window, count) in [("3.5", 3), ("12", 4), ("24", 4)] {
        let html = console
            .get(&format!("/lanes?hours={window}"))
            .text()
            .unwrap();
        assert!(html.contains("<h2>Lanes</h2>"), "{html}");
        assert_eq!(html.matches("class=\"lane\"").count(), count);
        assert_eq!(html.matches("data-seconds=\"600\"").count(), count);
        assert_eq!(html.matches("data-seconds=\"1200\"").count(), count);
        assert!(html.contains("History"));
    }
    assert_eq!(console.get("/?page=2").status(), 200);
    assert_eq!(console.get("/?page=3").status(), 200);
}

#[test]
fn security_lists_findings_and_sessions_from_the_cli_inventory() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 1, Some("merged"), 0);
    let mut security = record(home.path(), 2, Some("findings recorded"), 50);
    let session = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/2-fixture-security-audit.jsonl");
    fs::write(&session, "Security session output").unwrap();
    security["kind"] = json!("Security run");
    security["name"] = json!("secure-fixture");
    security["issue"] = Value::Null;
    security["session_log"] = json!(session);
    security["security_findings"] = json!([{"title": "Bound <input>", "severity": "high", "url": "https://github.com/acme/widgets/security/advisories/GHSA-test"}]);
    write_record(home.path(), 2, &security);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/?page=security").text().unwrap();
    assert!(html.contains("<h2>Security</h2>"));
    assert!(html.contains("Bound &#60;input&#62;"));
    assert!(html.contains("high"));
    assert!(html.contains("GHSA-test"));
    assert!(html.contains("2-fixture-security-audit.jsonl"));
    assert!(!html.contains("acme/widgets#1"));
    assert!(html.contains("thirdshift status --all --json"));
    assert!(html.contains("show"));
    let shown = Command::new(binary)
        .args(["show", "secure-fixture", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(shown.status.success());
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown["command"]["session_logs"], json!([session]));
    assert_eq!(
        shown["command"]["security_findings"],
        security["security_findings"]
    );
}

#[test]
fn peek_and_full_logs_use_the_exact_selected_command_and_share_show_details() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut old = record(home.path(), 7, Some("failed"), 90);
    old["issue"]["title"] = json!("Older Command <unsafe>");
    old["cause"] = json!("Old failure");
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let selector = folder.join("7-old.command.json");
    let session = folder.join("7-old-implement.jsonl");
    fs::write(&session, "Session <output>\n").unwrap();
    old["session_log"] = json!(session);
    fs::write(&selector, serde_json::to_vec(&old).unwrap()).unwrap();
    fs::write(folder.join("7-old.log"), "Old full Command <output>\n").unwrap();
    record(home.path(), 7, Some("merged"), 0);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let shown = Command::new(binary)
        .args(["show", selector.to_str().unwrap(), "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown["command"]["cause"], "Old failure");
    let console = Console::start(binary, home.path());
    let mut url = reqwest::Url::parse("http://localhost/peek").unwrap();
    url.query_pairs_mut()
        .append_pair("command", selector.to_str().unwrap());
    let peek = console
        .get(&format!("/peek?{}", url.query().unwrap()))
        .text()
        .unwrap();
    for expected in [
        "Older Command &#60;unsafe&#62;",
        "Old failure",
        "implement",
        "gpt-6",
        "7-old-implement.jsonl",
        "tickets",
        "usual ranges",
        "thirdshift show",
    ] {
        assert!(peek.contains(expected), "missing {expected}: {peek}");
    }
    let log = console
        .get(&format!("/log?{}", url.query().unwrap()))
        .text()
        .unwrap();
    assert!(
        log.contains("Old full Command &lt;output&gt;")
            || log.contains("Old full Command &#60;output&#62;")
    );
    url.query_pairs_mut()
        .append_pair("session", session.to_str().unwrap());
    let session_html = console
        .get(&format!("/log?{}", url.query().unwrap()))
        .text()
        .unwrap();
    assert!(
        session_html.contains("Session &lt;output&gt;")
            || session_html.contains("Session &#60;output&#62;")
    );
    assert_eq!(console.get("/peek?command=/etc/passwd").status(), 404);
    url.query_pairs_mut()
        .clear()
        .append_pair("command", selector.to_str().unwrap())
        .append_pair("session", "/etc/passwd");
    assert_eq!(
        console
            .get(&format!("/log?{}", url.query().unwrap()))
            .status(),
        404
    );
    let html = console.get("/?page=history").text().unwrap();
    assert!(html.contains("id=\"peek\" hidden"));
    assert!(html.contains("7-old.command.json"));
}

#[test]
fn dispatched_run_and_its_security_command_have_distinct_peeks() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut outer = record(home.path(), 1, Some("findings recorded"), 50);
    outer["kind"] = json!("Security run");
    outer["name"] = json!("secure-fixture");
    outer["issue"] = Value::Null;
    let mut dispatched = outer.clone();
    dispatched["kind"] = json!("Run");
    dispatched["name"] = json!("acme/widgets#42");
    dispatched["issue"] =
        json!({"number":42,"url":"https://github.com/acme/widgets/issues/42","title":"Fix it"});
    dispatched["started_at"] = json!(chrono::Local::now().to_rfc3339());
    outer["dispatched"] = json!([dispatched]);
    write_record(home.path(), 1, &outer);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/1-fixture.command.json");
    for (selector, expected) in [
        (path.display().to_string(), "Security run"),
        (format!("{}::acme/widgets#42", path.display()), "Run"),
    ] {
        let shown = Command::new(binary)
            .args(["show", &selector, "--json"])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(
            shown.status.success(),
            "{}",
            String::from_utf8_lossy(&shown.stderr)
        );
        let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
        assert_eq!(shown["command"]["kind"], expected);
    }
    let console = Console::start(binary, home.path());
    let html = console.get("/?page=security").text().unwrap();
    assert!(html.contains(&format!(
        "data-command=\"{}::acme/widgets#42\"",
        path.display()
    )));
    assert!(html.contains(&format!(
        "data-command=\"{}::secure-fixture\"",
        path.display()
    )));
}

#[test]
fn history_reaches_activity_and_session_logs_without_records() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let root = home.path().join(".thirdshift/logs/acme/widgets");
    let folder = root.join("commands/issue");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("8-20260101T120000+0000.log"),
        "Old Command output",
    )
    .unwrap();
    fs::write(root.join("activity.log"), "2026-01-01 12:00:00 Run #8 started: commands/issue/8-20260101T120000+0000.log\n2026-01-01 12:01:00 Run #8 ended: merged\n").unwrap();
    let session = folder.join("9-20260102T120000+0000-implement.jsonl");
    fs::write(&session, "Session without a Command record").unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/?page=history").text().unwrap();
    assert!(html.contains("acme/widgets#8"));
    assert!(html.contains("acme/widgets#9"));
    assert!(html.find("acme/widgets#9").unwrap() < html.find("acme/widgets#8").unwrap());
    assert!(html.contains(&format!("data-command=\"{}\"", session.display())));
    let shown = Command::new(binary)
        .args(["show", session.to_str().unwrap(), "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown["command"]["session_logs"], json!([session]));
}

#[test]
fn logs_inventory_matches_cli_and_reads_current_layout_only() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("7-20261003T120000-0400.log"),
        "Command <output>\n",
    )
    .unwrap();
    fs::write(folder.join("7-20261003T120000-0400-implement.jsonl"),
        "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Hello <reader>\"}}\n").unwrap();
    let old = home.path().join(".thirdshift/logs/acme/widgets/sessions");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("7-20261003T120000-0400-implement.jsonl"), "old\n").unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let output = Command::new(binary)
        .args(["logs", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inventory: Value = serde_json::from_slice(&output.stdout).unwrap();
    let logs = inventory["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 2);
    let page = console.get("/logs").text().unwrap();
    assert!(page.contains("thirdshift logs"));
    for log in logs {
        let url = format!("/logs/{}", log["id"].as_str().unwrap());
        assert!(page.contains(&url));
        let response = console.get(&url);
        assert_eq!(response.status(), 200);
        let html = response.text().unwrap();
        let raw = Command::new(binary)
            .args(["logs", "--file", log["id"].as_str().unwrap()])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(raw.status.success());
        if log["kind"] == "Command log" {
            assert_eq!(raw.stdout, b"Command <output>\n");
            assert!(
                html.contains("Command &#60;output&#62;")
                    || html.contains("Command &lt;output&gt;"),
                "{html}"
            );
        } else {
            assert!(
                html.contains("Hello &#60;reader&#62;") || html.contains("Hello &lt;reader&gt;"),
                "{html}"
            );
        }
        assert!(!html.contains("<output>"));
    }
    assert_eq!(console.get("/logs/not-an-inventory-id").status(), 404);
}

#[test]
fn command_logs_have_the_same_exact_owner_filter_in_cli_and_http() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    let names = [
        "7-20261003T120000-0400~one.log",
        "8-20261003T120000-0400~two.log",
    ];
    for name in names {
        fs::write(folder.join(name), "output\n").unwrap();
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let inventory = Command::new(binary)
        .args(["logs", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    let inventory: Value = serde_json::from_slice(&inventory.stdout).unwrap();
    let owner = inventory["logs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|log| log["name"].as_str().unwrap().ends_with(names[0]))
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let selected = Command::new(binary)
        .args(["logs", "--command", owner, "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let selected: Value = serde_json::from_slice(&selected.stdout).unwrap();
    let logs = selected["logs"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["id"], owner);
    let console = Console::start(binary, home.path());
    let html = console
        .get(&format!("/logs/command/{owner}"))
        .text()
        .unwrap();
    assert!(html.contains(&format!("href=\"/logs/{owner}\"")));
    assert!(!html.contains(names[1]));
    assert!(html.contains(&format!("thirdshift logs --command {owner} --json")));
}

#[test]
fn log_stream_starts_at_the_byte_offset_and_sends_only_appended_html() {
    use std::io::Write;
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join("7-20261003T120000-0400-implement.jsonl");
    let first = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"original message\"}}\n";
    fs::write(&path, first).unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let inventory = console.get("/logs").text().unwrap();
    let link = inventory
        .split("href=\"")
        .skip(1)
        .filter_map(|part| part.split_once('"'))
        .find(|(url, _)| url.starts_with("/logs/"))
        .unwrap()
        .0;
    let initial = console.get(link).text().unwrap();
    assert!(initial.contains(&format!("data-offset=\"{}\"", first.len())));
    let second = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"new café message\"}}\n";
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(second.as_bytes())
        .unwrap();
    let response = console.get(&format!("{link}/stream/{}", first.len()));
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let events = response.text().unwrap();
    assert!(!events.contains("original message"), "{events}");
    assert!(events.contains("new café message"), "{events}");
    assert!(
        events.contains(&format!("id: {}", first.len() + second.len())),
        "{events}"
    );
    assert!(events.contains("\"done\":true"));
}

#[test]
fn claude_and_codex_conversations_keep_tools_thinking_and_unknown_events() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    for (kind, fixture) in [
        ("claude", include_str!("fixtures/console-claude.jsonl")),
        ("codex", include_str!("fixtures/console-codex.jsonl")),
    ] {
        fs::write(
            folder.join(format!("7-20261003T120000-0400-{kind}.jsonl")),
            fixture,
        )
        .unwrap();
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let page = console.get("/logs").text().unwrap();
    for (kind, thinking) in [
        ("claude", "Consider the widget shape."),
        ("codex", "Consider a safe edit."),
    ] {
        let link = page
            .split("href=\"")
            .skip(1)
            .filter_map(|part| part.split_once('"'))
            .find(|(_, label)| {
                label
                    .split("</a>")
                    .next()
                    .unwrap()
                    .ends_with(&format!("-{kind}.jsonl"))
            })
            .unwrap()
            .0;
        let html = console.get(link).text().unwrap();
        assert!(html.contains(thinking));
        assert!(html.contains("<details class=\"thinking\"><summary>Thinking</summary>"));
        assert!(html.contains("<details class=\"tool-group\"><summary>Tools</summary>"));
        assert!(html.contains("data-tool=\"Bash\""));
        assert!(html.contains("data-tool=\"Read\""));
        assert!(html.contains("Widgets are ready."));
        if kind == "claude" {
            assert!(html.contains("class=\"diff\""));
            assert!(html.contains("-old widget\n+new widget"), "{html}");
            assert_eq!(html.matches("class=\"tool-group\"").count(), 1);
            assert!(html.contains("data-tool=\"Grep\""));
            assert!(html.contains("data-tool=\"NewTool\""));
            assert!(html.contains("unknown block stays visible"));
            assert!(html.contains("unknown event stays visible"));
            assert!(html.contains("not JSON: retained diagnostic"));
        } else {
            assert!(html.contains("unknown Codex item visible"));
            assert!(html.contains("Patch unavailable in this log."));
            assert!(html.contains("class=\"fallback\""));
            assert!(!html.contains("class=\"diff\""));
        }
        assert!(!html.contains("<script>bad()</script>"));
    }
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
    let child_log = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/8-fixture-implement.jsonl");
    fs::write(&child_log, "Ticket Session output\n").unwrap();
    child["session_log"] = json!(child_log);
    write_record(home.path(), 8, &child);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let parent_link = html
        .split("href=\"")
        .skip(1)
        .filter_map(|part| part.split_once('"'))
        .find(|(_, label)| label.starts_with(">Run and Session logs</a>"))
        .unwrap()
        .0;
    let parent_logs = console.get(parent_link).text().unwrap();
    assert!(
        parent_logs.contains("8-fixture-implement.jsonl"),
        "{parent_logs}"
    );
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
    let _open_stream = console.get("/events");
    // A reader must not keep graceful shutdown waiting on an infinite stream.
    #[cfg(target_os = "linux")]
    let _stream = {
        let mut live = record(home.path(), 7, None, 0);
        let stat = fs::read_to_string(format!("/proc/{}/stat", std::process::id())).unwrap();
        let ticks = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap();
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        live["process_id"] = json!(std::process::id());
        live["process_start"] = json!(format!("{}:{ticks}", boot.trim()));
        let log = home
            .path()
            .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture-implement.jsonl");
        fs::write(&log, "initial line\n").unwrap();
        live["session_log"] = json!(log);
        write_record(home.path(), 7, &live);
        let inventory = console.get("/logs").text().unwrap();
        let link = inventory
            .split("href=\"")
            .skip(1)
            .filter_map(|part| part.split_once('"'))
            .find(|(url, _)| url.starts_with("/logs/"))
            .unwrap()
            .0;
        console.get(&format!("{link}/stream/0"))
    };
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
