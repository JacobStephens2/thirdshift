//! The Console's public seam: the binary, fixture records and HTTP.
mod support;

#[path = "console/resources.rs"]
mod resources;

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

#[path = "console/look.rs"]
mod look;

#[path = "console/assets.rs"]
mod assets;

#[path = "console/inbox.rs"]
mod inbox;

#[path = "console/github.rs"]
mod github;

#[path = "console/codex_patches.rs"]
mod codex_patches;

#[path = "console/waterfall.rs"]
mod waterfall;

#[path = "console/spans.rs"]
mod spans;

#[path = "console/metrics.rs"]
mod metrics;

#[cfg(target_os = "linux")]
#[path = "console/idle.rs"]
mod idle;

#[cfg(unix)]
#[path = "console/watching.rs"]
mod watching;

#[cfg(unix)]
#[path = "console/cronjobs.rs"]
mod cronjobs;

#[path = "console/experiments.rs"]
mod experiments;

#[path = "console/scoring.rs"]
mod scoring;

#[path = "console/security_fixes.rs"]
mod security_fixes;

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
        let mut command = Command::new(binary);
        command
            .args(args)
            .env("HOME", home)
            .env_remove("RESEND_API_KEY");
        Self::start_command(command)
    }

    fn start_command(mut command: Command) -> Self {
        let mut child = command
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

    fn log_link(&self, suffix: &str) -> String {
        let page = self.get("/logs").text().unwrap();
        page.split("href=\"")
            .skip(1)
            .filter_map(|part| part.split_once('"'))
            .find(|(_, label)| label.split("</a>").next().unwrap().ends_with(suffix))
            .unwrap()
            .0
            .to_owned()
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reader.take().unwrap().join().unwrap();
    }
}

fn sse_event(stream: &mut impl BufRead) -> String {
    let mut message = String::new();
    loop {
        let mut line = String::new();
        assert!(stream.read_line(&mut line).unwrap() > 0, "stream closed");
        if line.trim().is_empty() && !message.is_empty() {
            return message;
        }
        message.push_str(&line);
    }
}

fn configure(home: &Path) {
    fs::create_dir_all(home.join(".thirdshift")).unwrap();
    fs::write(home.join(".thirdshift/config.toml"),
        "[console]\nport = 0\nother_consoles = [\"https://other.example.test\"]\ntheme = \"focus-auto\"\n").unwrap();
}

/// How the Console shows `rfc3339` up to its zone name: this machine's date
/// and time to the minute, as in `Ended 2026-10-23 19:03 EDT`.
/// One Inbox section's markup, found by its group name.
fn section<'a>(page: &'a str, group: &str) -> &'a str {
    page.split(&format!("class=\"group\" data-group=\"{group}\""))
        .nth(1)
        .expect(page)
        .split("</section>")
        .next()
        .unwrap()
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

fn span_tree(console: &Console, command: &str) -> Value {
    console
        .get(&format!("/spans?command={command}"))
        .json()
        .unwrap()
}

fn tool_spans(tree: &Value) -> Vec<&Value> {
    tree["spans"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|span| span["kind"] == "tool")
        .collect()
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
    {
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
        let old_log = std::path::PathBuf::from(fixture["session_log"].as_str().unwrap());
        let session = old_log.clone();
        let log = path.with_file_name(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .replace(".command.json", ".log"),
        );
        let began = (chrono::Local::now() - chrono::Duration::minutes(20)).to_rfc3339();
        fixture["stages"] =
            json!([{"name": "implement", "began_at": began, "session_log": old_log}]);
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
    record(
        home.path(),
        5,
        Some("PR https://github.com/acme/widgets/pull/5 is merged"),
        90,
    );
    record(home.path(), 6, Some("failed: claude exited 1"), 95);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/?page=history").text().unwrap();
    // The Outcome filter offers kinds of outcome, not each PR or error.
    let select = &html[html.find("<select name=\"outcome\"").unwrap()..];
    let select = &select[..select.find("</select>").unwrap()];
    assert!(select.contains(">merged</option>"), "{select}");
    assert!(select.contains(">failed</option>"), "{select}");
    assert!(
        !select.contains("pull/5") && !select.contains("claude exited"),
        "{select}"
    );
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
    assert!(filtered.contains("acme/widgets#5"));
    let failures = console.get("/?page=history&outcome=failed").text().unwrap();
    assert!(failures.contains("acme/widgets#2") && failures.contains("acme/widgets#6"));
    assert!(!failures.contains("acme/widgets#5"));
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
fn time_pages_name_the_command_kind_without_repeating_it_for_commands_without_issues() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut pickup = record(home.path(), 7, Some("merged"), 0);
    pickup["kind"] = json!("Pickup run");
    pickup["name"] = json!("pickup-fixture");
    pickup["stages"][0]["name"] = json!("dispatched Run");
    let mut dispatched = pickup.clone();
    dispatched["kind"] = json!("Run");
    dispatched["name"] = json!("acme/widgets#7");
    dispatched["stages"][0]["name"] = json!("implement");
    pickup["dispatched"] = json!([dispatched]);
    write_record(home.path(), 7, &pickup);
    let mut security = record(home.path(), 8, Some("findings recorded"), 0);
    security["kind"] = json!("Security run");
    security["issue"] = Value::Null;
    write_record(home.path(), 8, &security);
    let now = chrono::Local::now();
    let time = now.format("%Y-%m-%d %H:%M:%S");
    let stamp = now.format("%Y%m%dT%H%M%S%z");
    fs::write(
        home.path().join(".thirdshift/logs/acme/widgets/activity.log"),
        format!(
            "{time} Spec run #9 started: commands/issue/9-{stamp}.log\n{time} Spec run #9 ended: merged\n"
        ),
    )
    .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for page in ["/lanes", "/stages"] {
        let html = console.get(page).text().unwrap();
        for heading in [
            "Pickup run · acme/widgets#7",
            "Ticket run · acme/widgets#7",
            "Security run on acme/widgets",
            "Spec run · acme/widgets#9",
        ] {
            assert!(html.contains(&format!(">{heading}</a>")), "{page}: {html}");
            if page == "/stages" {
                assert!(html.contains(&format!("class=\"cell-run\">{heading}: </span>")));
            }
        }
        assert!(!html.contains("Security run · Security run"));
    }
}

#[test]
fn stages_groups_exact_spec_attempts_by_owner_start_and_keeps_dispatches() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    for (number, kind, started, parent) in [
        (1, "Spec run", "2026-01-01T01:00:00Z", None),
        (2, "Run", "2026-01-01T05:00:00Z", Some(1)),
        (3, "Run", "2026-01-01T03:00:00Z", Some(1)),
        (4, "Spec run", "2026-01-01T02:00:00Z", None),
        (5, "Run", "2026-01-01T04:00:00Z", Some(4)),
        (6, "Run", "unknown", None),
        (8, "Run", "2026-01-01T03:30:00Z", Some(4)),
        (9, "Run", "unknown", Some(4)),
        (10, "Run", "2026-01-01T04:00:00Z", Some(4)),
        (12, "Run", "2026-01-01T05:30:00Z", Some(11)),
        (13, "Run", "2026-01-01T06:00:00Z", Some(11)),
    ] {
        let mut value = record(home.path(), number, Some("merged"), 0);
        value["kind"] = json!(kind);
        value["started_at"] = json!(started);
        if kind == "Spec run" {
            value["issue"]["number"] = json!(99);
        }
        if let Some(parent) = parent {
            value["parent_record"] = json!(home.path().join(format!(
                ".thirdshift/logs/acme/widgets/commands/issue/{parent}-fixture.command.json"
            )));
            if parent == 11 {
                value["parent_dispatch"] = json!("https://github.com/acme/widgets/issues/99");
            }
        }
        write_record(home.path(), number, &value);
    }
    let mut pickup = record(home.path(), 11, Some("merged"), 0);
    let mut dispatched = pickup.clone();
    dispatched["name"] = json!("dispatched-spec");
    dispatched["issue"]["number"] = json!(99);
    dispatched["issue"]["url"] = json!("https://github.com/acme/widgets/issues/99");
    dispatched["started_at"] = json!("2026-01-01T00:10:00Z");
    pickup["kind"] = json!("Pickup run");
    pickup["issue"] = Value::Null;
    pickup["started_at"] = json!("2026-01-01T00:00:00Z");
    pickup["dispatched"] = json!([dispatched]);
    write_record(home.path(), 11, &pickup);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/stages").text().unwrap();
    assert_eq!(html.matches("class=\"stage-grid\"").count(), 1, "{html}");
    assert_eq!(html.matches("scope=\"colgroup\"").count(), 5, "{html}");
    let columns: Vec<_> = html
        .match_indices("data-command=\"")
        .map(|(index, _)| html[index..].split('"').nth(1).unwrap().to_owned())
        .collect();
    let numbers: Vec<_> = columns
        .iter()
        .map(|column| {
            column
                .rsplit('/')
                .next()
                .unwrap()
                .split('-')
                .next()
                .unwrap()
        })
        .collect();
    assert_eq!(
        numbers,
        [
            "4", "10", "5", "8", "9", "1", "2", "3", "11", "13", "12", "11", "6"
        ]
    );
    assert_eq!(
        columns
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        13
    );
    assert!(html.contains("Dispatched by Pickup run on acme/widgets"));
    assert!(
        html.contains("Ticket of Spec run · acme/widgets#99"),
        "{html}"
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
fn stages_orders_known_rows_by_run_progression_then_unknown_names() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    for (index, names) in [
        vec!["z legacy stage", "A future stage"],
        vec![
            "Security audit",
            "Security reproduction",
            "Security fix publishing",
            "Security close check",
            "dispatched Run",
            "dispatched Spec run",
        ],
        vec!["Architecture review", "Tickets", "Spec review"],
        vec![
            "implement",
            "Resume",
            "Security review",
            "pull request",
            "CI watch",
            "Repair",
            "CI watch",
            "Self-merge",
            "cleanup",
        ],
    ]
    .into_iter()
    .enumerate()
    {
        let number = index as u64 + 1;
        let mut value = record(home.path(), number, Some("merged"), 0);
        value["stages"] = names
            .into_iter()
            .map(|name| json!({"name": name, "began_at": value["started_at"], "session_log": null}))
            .collect();
        write_record(home.path(), number, &value);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for page in ["/stages", "/?page=stages"] {
        let html = console.get(page).text().unwrap();
        assert_eq!(html.matches("<tbody>").count(), 1, "{html}");
        for table in html.split("<tbody>").skip(1) {
            let names: Vec<_> = table
                .split("</tbody>")
                .next()
                .unwrap()
                .split("<th scope=\"row\">")
                .skip(1)
                .map(|row| row.split("<br>").next().unwrap())
                .collect();
            assert_eq!(
                names,
                [
                    "implement",
                    "Resume",
                    "Security review",
                    "pull request",
                    "CI watch",
                    "Repair",
                    "Self-merge",
                    "cleanup",
                    "Architecture review",
                    "Tickets",
                    "Spec review",
                    "Security audit",
                    "Security reproduction",
                    "Security fix publishing",
                    "Security close check",
                    "dispatched Ticket run",
                    "dispatched Spec run",
                    "A future stage",
                    "z legacy stage",
                ],
                "{page}"
            );
        }
    }
}

#[test]
#[cfg(target_os = "linux")]
fn stages_distinguishes_dispatch_from_insufficient_usual_range_samples() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let now = chrono::Local::now();
    let stat = fs::read_to_string(format!("/proc/{}/stat", std::process::id())).unwrap();
    let ticks = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    for (number, kind, stage) in [
        (1, "Pickup run", "dispatched Run"),
        (2, "Security run", "dispatched Spec run"),
    ] {
        let mut value = record(home.path(), number, None, 0);
        value["kind"] = json!(kind);
        value["name"] = json!(format!("pass-{number}"));
        value["issue"] = Value::Null;
        value["process_id"] = json!(std::process::id());
        value["process_start"] = json!(format!("{}:{ticks}", boot.trim()));
        value["started_at"] = json!((now - chrono::Duration::minutes(2)).to_rfc3339());
        value["stages"] = json!([
            {"name": "implement", "began_at": value["started_at"], "session_log": null},
            {"name": stage, "began_at": (now - chrono::Duration::minutes(1)).to_rfc3339(), "session_log": null}
        ]);
        write_record(home.path(), number, &value);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/?page=stages").text().unwrap();
    for name in ["dispatched Ticket run", "dispatched Spec run"] {
        let row = html
            .split("<tr>")
            .find(|row| row.starts_with(&format!("<th scope=\"row\">{name}<br>")))
            .unwrap();
        assert!(row.contains("Usual range: —</span>"), "{row}");
        assert!(row.contains("class=\"running-stage "), "{row}");
        assert!(row.contains("Running · 1 min"), "{row}");
        assert!(row.contains("Not reached · —"), "{row}");
    }
    assert!(html.contains("Usual range: Not enough samples</span>"));
}

#[test]
fn security_lists_findings_and_sessions_from_the_cli_inventory() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 1, Some("merged"), 0);
    let mut security = record(home.path(), 2, Some("findings recorded"), 50);
    let session = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/2-fixture-security-audit.logs.jsonl");
    support::session_trace::write(&session, "Security session output");
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
    assert!(html.contains("2-fixture-security-audit.logs.jsonl"));
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
    let session = folder.join("7-old-implement.logs.jsonl");
    support::session_trace::write(&session, "Session <output>\n");
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
    assert_eq!(shown["command"]["issue"]["title"], "Older Command <unsafe>");
    let console = Console::start(binary, home.path());
    let mut url = reqwest::Url::parse("http://localhost/peek").unwrap();
    url.query_pairs_mut()
        .append_pair("command", selector.to_str().unwrap());
    let peek = console
        .get(&format!("/peek?{}", url.query().unwrap()))
        .text()
        .unwrap();
    let heading = peek.split("</h2>").next().unwrap();
    assert!(heading.contains("acme/widgets#7</a>"), "{heading}");
    assert!(
        heading.contains("Older Command &#60;unsafe&#62;"),
        "{heading}"
    );
    for expected in [
        "Older Command &#60;unsafe&#62;",
        "Old failure",
        "implement",
        "gpt-6",
        "7-old-implement.logs.jsonl",
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
fn ready_dispatches_leave_ready_for_review_and_are_shown_at_most_once() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    // Pickup run 1 dispatched a ready Run on issue 7, replaced by record 2;
    // Pickup run 3 dispatched a ready Run on issue 9, older than inbox.days.
    for (file, issue, hours) in [(1, 7, 2), (3, 9, 8 * 24 + 1)] {
        let ended = (chrono::Local::now() - chrono::Duration::hours(hours)).to_rfc3339();
        let mut outer = record(home.path(), file, Some("ready for review"), 0);
        outer["kind"] = json!("Pickup run");
        outer["name"] = json!("pickup-fixture");
        outer["issue"] = Value::Null;
        outer["pull_request"] = Value::Null;
        outer["started_at"] = json!(ended);
        outer["ended_at"] = json!(ended);
        let mut dispatched = record(home.path(), file, Some("ready for review"), 0);
        dispatched["kind"] = json!("Run");
        dispatched["name"] = json!(format!("acme/widgets#{issue}"));
        dispatched["issue"]["number"] = json!(issue);
        dispatched["started_at"] = json!(ended);
        dispatched["ended_at"] = json!(ended);
        outer["dispatched"] = json!([dispatched]);
        write_record(home.path(), file, &outer);
    }
    let mut later = record(home.path(), 2, Some("merged"), 0);
    later["issue"]["number"] = json!(7);
    write_record(home.path(), 2, &later);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let groups = |flags: &[&str]| {
        let status = Command::new(binary)
            .args(flags)
            .env("HOME", home.path())
            .output()
            .unwrap();
        let status: Value = serde_json::from_slice(&status.stdout).unwrap();
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["kind"].as_str().unwrap().to_owned(),
                    row["issue"]["number"].as_u64(),
                    row["group"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let row = |kind: &str, issue: Option<u64>, group: &str| (kind.into(), issue, group.into());
    let current = [
        row("Spec run", Some(7), "done"),
        row("Pickup run", None, "done"),
        row("Run", Some(7), "done"),
    ];
    assert_eq!(groups(&["status", "--json"]), current);
    let mut all = current.to_vec();
    all.extend([
        row("Pickup run", None, "history"),
        row("Run", Some(9), "history"),
    ]);
    assert_eq!(groups(&["status", "--all", "--json"]), all);
    let console = Console::start(binary, home.path());
    let html = console.get("/").text().unwrap();
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    for (file, issue, shown) in [(1, 7, 1), (3, 9, 0)] {
        let path = folder.join(format!("{file}-fixture.command.json"));
        let row = format!("data-command=\"{}::acme/widgets#{issue}\"", path.display());
        assert_eq!(html.matches(&row).count(), shown, "{html}");
    }
}

/// The visible text of `html`, tags and attribute values left out, and any
/// RFC 3339 instant in it, such as `2026-10-25T17:43:26-04:00`.
fn raw_instants(html: &str) -> Vec<String> {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(19))
        .filter(|&i| {
            b"dddd-dd-ddTdd:dd:dd"
                .iter()
                .zip(&bytes[i..])
                .all(|(want, got)| {
                    if *want == b'd' {
                        got.is_ascii_digit()
                    } else {
                        want == got
                    }
                })
        })
        .map(|i| text[i..i + 19].to_owned())
        .collect()
}

#[test]
fn inbox_and_command_details_show_instants_in_local_minutes() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 7, Some("failed"), 0);
    fixture["cause"] = json!("quota resets at 2026-10-24T03:28:16Z");
    write_record(home.path(), 7, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    assert_eq!(raw_instants(&html), Vec::<String>::new(), "{html}");
    assert!(
        html.contains(&format!(
            "quota resets at {}",
            support::local_minute("2026-10-24T03:28:16Z")
        )),
        "{html}"
    );
    let mut url = reqwest::Url::parse("http://localhost/peek").unwrap();
    url.query_pairs_mut().append_pair(
        "command",
        home.path()
            .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.command.json")
            .to_str()
            .unwrap(),
    );
    let peek = console
        .get(&format!("/peek?{}", url.query().unwrap()))
        .text()
        .unwrap();
    assert_eq!(raw_instants(&peek), Vec::<String>::new(), "{peek}");
    let started = fixture["started_at"].as_str().unwrap();
    assert!(
        peek.contains(&format!(
            "<dt>started at</dt><dd>{}",
            support::local_minute(started)
        )),
        "{peek}"
    );
}

#[test]
fn command_details_show_short_facts_and_fold_nested_ones_closed() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 7, Some("failed"), 0);
    fixture["cause"] = json!("Short failure");
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = folder.join("7-fixture-implement.logs.jsonl");
    support::session_trace::write(&session, "Session output\n");
    fixture["session_log"] = json!(session);
    write_record(home.path(), 7, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let mut url = reqwest::Url::parse("http://localhost/peek").unwrap();
    url.query_pairs_mut().append_pair(
        "command",
        folder.join("7-fixture.command.json").to_str().unwrap(),
    );
    let peek = console
        .get(&format!("/peek?{}", url.query().unwrap()))
        .text()
        .unwrap();
    // The high-level view: short facts in the open, nested ones behind a preview.
    assert!(peek.contains("<dt>cause</dt><dd>Short failure"), "{peek}");
    assert!(
        peek.contains("<details data-fact=\"issue\"><summary>Widgets &#60;script&#62;alert(1)&#60;/script&#62;</summary>"),
        "{peek}"
    );
    assert!(
        peek.contains("<details data-fact=\"stages\"><summary>1 item</summary>"),
        "{peek}"
    );
    assert!(
        peek.contains("<details data-fact=\"stages/1\"><summary>implement</summary>"),
        "{peek}"
    );
    assert!(
        peek.contains("<details class=\"peek-traces\" data-fact=\"session traces\"><summary>Session traces <span class=\"count\">1</span></summary>"),
        "{peek}"
    );
    assert!(!peek.contains("<details open"), "{peek}");
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
    let session = folder.join("9-20260102T120000+0000-implement.logs.jsonl");
    support::session_trace::write(&session, "Session without a Command record");
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
        "Command <output> https://github.com/acme/widgets/pull/9\n",
    )
    .unwrap();
    fs::write(
        folder.join("7-20261003T120000-0400.notification.txt"),
        "Subject: saved notification\n\nResult: ready for review\n",
    )
    .unwrap();
    fs::write(folder.join("7-20261003T120000-0400-implement.logs.jsonl"),
        "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"Hello <reader> (https://github.com/acme/widgets/issues/7)\"}}\n").unwrap();
    let session = folder.join("7-20261003T120000-0400-implement.logs.jsonl");
    fs::write(
        support::session_trace::artifact(&session, "logs.jsonl"),
        support::session_trace::records(&fs::read_to_string(&session).unwrap()),
    )
    .unwrap();
    fs::write(
        support::session_trace::artifact(&session, "spans.jsonl"),
        "{}\n",
    )
    .unwrap();
    let old = home.path().join(".thirdshift/logs/acme/widgets/sessions");
    fs::create_dir_all(&old).unwrap();
    support::session_trace::write(
        old.join("7-20261003T120000-0400-implement.logs.jsonl"),
        "old\n",
    );
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
    assert!(!page.contains("notification.txt"));
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
            assert_eq!(
                raw.stdout,
                b"Command <output> https://github.com/acme/widgets/pull/9\n"
            );
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
        let url = if log["kind"] == "Command log" {
            "https://github.com/acme/widgets/pull/9"
        } else {
            "https://github.com/acme/widgets/issues/7"
        };
        assert!(
            html.contains(&format!("<a href=\"{url}\">{url}</a>")),
            "{html}"
        );
    }
    assert_eq!(console.get("/logs/not-an-inventory-id").status(), 404);
}

#[test]
fn logs_fold_by_repository_unless_there_is_only_one() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let log = |repository: &str, number: u64| {
        let folder = home
            .path()
            .join(format!(".thirdshift/logs/{repository}/commands/issue"));
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join(format!("{number}-20261003T120000-0400.log")),
            "Command output\n",
        )
        .unwrap();
    };
    log("acme/widgets", 7);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let page = console.get("/logs").text().unwrap();
    assert!(
        page.contains("<details class=\"group log-group\" data-repository=\"acme/widgets\" open>"),
        "{page}"
    );
    log("acme/widgets", 8);
    log("aaa/first", 3);
    let page = console.get("/logs").text().unwrap();
    for (repository, count) in [("aaa/first", "1 log"), ("acme/widgets", "2 logs")] {
        let fold = format!("<details class=\"group log-group\" data-repository=\"{repository}\">");
        let start = page.find(&fold).unwrap_or_else(|| panic!("{page}"));
        let fold = &page[start..start + page[start..].find("</details>").unwrap()];
        assert!(
            fold.contains(&format!(
                "<summary>{repository} <span class=\"count\">{count}</span>"
            )),
            "{fold}"
        );
        assert!(fold.contains("class=\"log-row\""), "{fold}");
    }
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
    let path = folder.join("7-20261003T120000-0400-implement.logs.jsonl");
    let first = support::session_trace::records(
        "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"original message\"}}\n",
    );
    fs::write(&path, &first).unwrap();
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
    let second = support::session_trace::records(
        "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"new café message\"}}\n",
    );
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
fn a_log_buffered_across_several_batches_streams_every_byte_without_another_change() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 81, Some("merged"), 0);
    // More than three 256 KiB batch budgets, all written before the reader opens.
    let log: String = (0..40_000)
        .map(|line| format!("buffered line {line:06}\n"))
        .collect();
    assert!(log.len() > 3 * 256 * 1024);
    fs::write(
        home.path()
            .join(".thirdshift/logs/acme/widgets/commands/issue/81-fixture.log"),
        &log,
    )
    .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let link = console.log_link("81-fixture.log");
    let events = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .get(format!("{}{link}/stream/0", console.address))
        .send()
        .unwrap()
        .text()
        .expect("the stream ends after its final batch without another change");
    let batches: Vec<Value> = events
        .split("\n\n")
        .filter_map(|event| event.lines().find_map(|line| line.strip_prefix("data: ")))
        .map(|data| serde_json::from_str(data).unwrap())
        .collect();
    assert!(batches.len() > 3, "{} batches", batches.len());
    let offsets: Vec<u64> = batches
        .iter()
        .map(|batch| batch["offset"].as_u64().unwrap())
        .collect();
    assert!(
        offsets.windows(2).all(|pair| pair[0] < pair[1]),
        "{offsets:?}"
    );
    assert_eq!(*offsets.last().unwrap(), log.len() as u64);
    let done: Vec<bool> = batches
        .iter()
        .map(|batch| batch["done"].as_bool().unwrap())
        .collect();
    assert_eq!(done.iter().filter(|done| **done).count(), 1);
    assert!(done.last().unwrap());
    let html: String = batches
        .iter()
        .map(|batch| batch["html"].as_str().unwrap())
        .collect();
    let mut from = 0;
    for line in 0..40_000 {
        let marker = format!("buffered line {line:06}");
        from += html[from..]
            .find(&marker)
            .unwrap_or_else(|| panic!("{marker} missing or out of order"));
    }
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
            folder.join(format!("7-20261003T120000-0400-{kind}.logs.jsonl")),
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
                    .ends_with(&format!("-{kind}.logs.jsonl"))
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
fn session_trace_byte_bodies_render_as_escaped_diagnostics() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("7-20261003T120000-0400-implement.logs.jsonl"),
        "{\"resourceLogs\":[{\"scopeLogs\":[{\"logRecords\":[{\"body\":{\"bytesValue\":\"YmFkIP8gPHVuc2FmZT4K\"}}]}]}]}\n").unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let link = console.log_link("-implement.logs.jsonl");
    let html = console.get(&link).text().unwrap();
    assert!(html.contains("bad � &#60;unsafe&#62;"), "{html}");
    assert!(!html.contains("<unsafe>"));
}

#[test]
fn all_six_harness_session_traces_render_normalized_conversations() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for (harness, fixture) in [
        (
            "claude",
            include_str!("fixtures/console-claude-trace.jsonl"),
        ),
        ("codex", include_str!("fixtures/console-codex-trace.jsonl")),
        ("agy", include_str!("fixtures/console-agy-trace.jsonl")),
        ("grok", include_str!("fixtures/console-grok-trace.jsonl")),
        ("muse", include_str!("fixtures/console-muse-trace.jsonl")),
        (
            "opencode",
            include_str!("fixtures/console-opencode-trace.jsonl"),
        ),
    ] {
        let path = folder.join(format!("7-20261003T120000-0400-{harness}.logs.jsonl"));
        fs::write(&path, fixture).unwrap();
        let link = console.log_link(&format!("-{harness}.logs.jsonl"));
        let html = console.get(&link).text().unwrap();
        assert!(
            html.contains("<h3>assistant</h3><pre>Checking widgets.</pre>"),
            "{harness}: {html}"
        );
        assert!(
            html.contains("<h3>user</h3><pre>Check widgets.</pre>"),
            "{harness}: {html}"
        );
        assert!(html.contains("data-tool=\"Read\""), "{harness}: {html}");
        assert!(
            html.contains("<pre class=\"read\">widgets.txt</pre>"),
            "{harness}: {html}"
        );
        assert!(
            html.contains(
                "<pre class=\"result\">widget &#60;script&#62;bad()&#60;/script&#62;</pre>"
            ),
            "{harness}: {html}"
        );
        assert!(
            html.contains("<pre class=\"unknown-event\">future event &#60;raw&#62;</pre>"),
            "{harness}: {html}"
        );
        assert!(
            html.contains("<h3>assistant</h3><pre>Widgets are ready.</pre>"),
            "{harness}: {html}"
        );
        assert!(!html.contains("<script>bad()</script>"), "{harness}");
        assert!(
            !html.contains("raw body must not replace normalized text"),
            "{harness}: {html}"
        );
        let snapshots = matches!(harness, "muse" | "opencode");
        assert_eq!(
            html.matches("class=\"message\"").count(),
            if snapshots { 4 } else { 3 },
            "{harness}: {html}"
        );
        if snapshots {
            assert!(!html.contains("<pre>Checking </pre>"), "{harness}: {html}");
            assert!(
                html.contains("<pre>Nested widget note.</pre>"),
                "{harness}: {html}"
            );
        }
        if harness == "codex" {
            assert!(html.contains("<details class=\"thinking\"><summary>Thinking</summary><pre>Consider a safe edit.</pre></details>"), "{html}");
        }
    }
}

#[test]
fn normalized_conversation_stream_preserves_raw_only_blocks() {
    use std::io::Write;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join("7-20261003T120000-0400-claude.logs.jsonl");
    let initial = include_str!("fixtures/console-claude-trace.jsonl");
    fs::write(&path, initial).unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let link = console.log_link("-claude.logs.jsonl");
    let raw = json!({"type":"assistant","message":{"content":[
        {"type":"thinking","thinking":"Consider the widget shape."},
        {"type":"text","text":"raw body must not replace normalized text"},
        {"type":"future_block","detail":"unknown block stays visible"}
    ]}})
    .to_string();
    let appended = format!(
        "{}\n",
        json!({"resourceLogs":[{"scopeLogs":[{"logRecords":[{
            "body":{"stringValue":raw},
            "attributes":[{"key":"thirdshift.events","value":{"stringValue":json!([
                {"type":"message","role":"assistant","text":"Appended normalized reply.","parent_id":null}
            ]).to_string()}}]
        }]}]}]})
    );
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(appended.as_bytes())
        .unwrap();
    let response = console.get(&format!("{link}/stream/{}", initial.len()));
    let events = response.text().unwrap();
    assert!(events.contains("Appended normalized reply."), "{events}");
    assert!(events.contains("Consider the widget shape."), "{events}");
    assert!(events.contains("unknown block stays visible"), "{events}");
    assert!(
        !events.contains("raw body must not replace normalized text"),
        "{events}"
    );
    assert!(!events.contains("Widgets are ready."), "{events}");
    assert!(
        events.contains(&format!("id: {}", initial.len() + appended.len())),
        "{events}"
    );
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
        .split("class=\"group\" data-group=\"")
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
        .split("<span class=\"subject\" title=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap())
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
        let section = section(&html, group);
        assert!(
            section.contains(&format!("acme/widgets#{number}")),
            "{section}"
        );
    }
    assert!(html.contains("thirdshift status"));
    assert!(html.contains(status["hostname"].as_str().unwrap()));
    assert!(html.contains("https://other.example.test"));
    for page in [
        "inbox",
        "lanes",
        "stages",
        "waterfall",
        "history",
        "logs",
        "security",
    ] {
        let html = console.get(&format!("/?page={page}")).text().unwrap();
        let header = html
            .split("<header>")
            .nth(1)
            .unwrap()
            .split("</header>")
            .next()
            .unwrap();
        assert!(
            header.contains(status["hostname"].as_str().unwrap()),
            "{page}: {header}"
        );
        assert!(
            header.contains("href=\"https://other.example.test\""),
            "{page}: {header}"
        );
    }
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
        .join(".thirdshift/logs/acme/widgets/commands/issue/8-fixture-implement.logs.jsonl");
    support::session_trace::write(&child_log, "Ticket Session output\n");
    child["session_log"] = json!(child_log);
    write_record(home.path(), 8, &child);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let parent_link = html
        .split("href=\"")
        .skip(1)
        .filter_map(|part| part.split_once('"'))
        .find(|(_, label)| label.starts_with(">Command logs and Session traces</a>"))
        .unwrap()
        .0;
    let parent_logs = console.get(parent_link).text().unwrap();
    assert!(
        parent_logs.contains("8-fixture-implement.logs.jsonl"),
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
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let binary = home.path().join("thirdshift");
    support::copy_executable(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &binary);
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
            .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture-implement.logs.jsonl");
        support::session_trace::write(&log, "initial line\n");
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
    support::write_executable(
        &replacement,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/reexec-args\"\nprintf 'replacement executed\\n'\n",
    );
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

#[test]
fn urls_in_inbox_and_peek_text_are_links_and_other_text_stays_escaped() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut failed = record(home.path(), 812, Some("failed"), 0);
    failed["cause"] =
        json!("PR https://github.com/acme/widgets/pull/834 is a draft. <b>javascript:alert(1)</b>");
    failed["pull_request"] = json!("https://github.com/acme/widgets/pull/834");
    failed["tickets"][0]["blockers"][0]["url"] = json!("https://github.com/other/repo/issues/98");
    write_record(home.path(), 812, &failed);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/").text().unwrap();
    let pr = "<a href=\"https://github.com/acme/widgets/pull/834\">https://github.com/acme/widgets/pull/834</a>";
    for expected in [
        "<a href=\"https://github.com/acme/widgets/issues/812\"><span class=\"subject-repository\">acme/widgets</span><span class=\"subject-issue\">#812</span></a>",
        &format!("<p class=\"pr\">{pr}</p>"),
        &format!("<p>PR {pr} is a draft. &#60;b&#62;javascript:alert(1)&#60;/b&#62;</p>"),
        "waiting on <a href=\"https://github.com/other/repo/issues/98\">https://github.com/other/repo/issues/98</a>",
    ] {
        assert!(html.contains(expected), "missing {expected}: {html}");
    }
    let selector = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/812-fixture.command.json");
    let mut url = reqwest::Url::parse("http://localhost/peek").unwrap();
    url.query_pairs_mut()
        .append_pair("command", selector.to_str().unwrap());
    let peek = console
        .get(&format!("/peek?{}", url.query().unwrap()))
        .text()
        .unwrap();
    for expected in [
        &format!("<dd>{pr}</dd>"),
        "<dd><a href=\"https://github.com/acme/widgets/issues/812\">https://github.com/acme/widgets/issues/812</a>",
        // A long value opens from its one-line summary.
        "<summary>PR <a href=",
    ] {
        assert!(peek.contains(expected), "missing {expected}: {peek}");
    }
}

#[test]
fn stages_and_command_details_link_the_github_issue() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 7, Some("merged"), 0);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let stages = console.get("/?page=stages").text().unwrap();
    assert!(
        stages.contains("<a href=\"https://github.com/acme/widgets/issues/7\">Issue #7</a>"),
        "stages page links the Issue URL: {stages}"
    );
    let selector = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.command.json");
    let mut url = reqwest::Url::parse("http://localhost/peek").unwrap();
    url.query_pairs_mut()
        .append_pair("command", selector.to_str().unwrap());
    let peek = console
        .get(&format!("/peek?{}", url.query().unwrap()))
        .text()
        .unwrap();
    assert!(
        peek.contains("<a href=\"https://github.com/acme/widgets/issues/7\">acme/widgets#7</a>"),
        "Command details links the Issue URL: {peek}"
    );
}

#[test]
fn command_details_and_inbox_share_subject_and_title_fallbacks() {
    for (issue, name, process, subject, title) in [
        (
            Value::Null,
            "architect-fixture",
            None,
            "architect-fixture on acme/widgets",
            "",
        ),
        (
            json!({"number": 7, "url": "https://github.com/acme/widgets/issues/7"}),
            "acme/widgets#7",
            None,
            "acme/widgets#7",
            "(title unavailable)",
        ),
        (
            Value::Null,
            "process-notes-fixture",
            Some("notes"),
            "Process notes on acme/widgets",
            "",
        ),
        (
            json!({"number": 7, "url": "https://github.com/acme/widgets/issues/7"}),
            "process-notes-fixture",
            Some("notes"),
            "Process notes · acme/widgets#7",
            "(title unavailable)",
        ),
        (
            json!({"number": 7, "url": "https://github.com/acme/widgets/issues/7", "title": "Review notes"}),
            "process-notes-fixture",
            Some("notes"),
            "Process notes · acme/widgets#7",
            "Review notes",
        ),
    ] {
        let home = tempfile::tempdir().unwrap();
        configure(home.path());
        let mut facts = record(home.path(), 7, Some("failed"), 0);
        facts["issue"] = issue;
        facts["name"] = name.into();
        if let Some(process) = process {
            facts["kind"] = "Process".into();
            facts["process_name"] = process.into();
        }
        write_record(home.path(), 7, &facts);
        let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
        let inbox = console.get("/").text().unwrap();
        assert!(inbox.contains(subject), "{inbox}");
        assert!(
            inbox.contains(&format!("class=\"issue-title\">{title}</span>")),
            "{inbox}"
        );
        let selector = home
            .path()
            .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.command.json");
        let mut url = reqwest::Url::parse("http://localhost/peek").unwrap();
        url.query_pairs_mut()
            .append_pair("command", selector.to_str().unwrap());
        let peek = console
            .get(&format!("/peek?{}", url.query().unwrap()))
            .text()
            .unwrap();
        let heading = peek.split("</h2>").next().unwrap();
        assert!(heading.contains(subject), "{heading}");
        if title.is_empty() {
            assert!(!heading.contains("<span>"), "{heading}");
            assert!(!heading.contains("<a "), "{heading}");
        } else {
            assert!(heading.contains(title), "{heading}");
        }
    }
}
