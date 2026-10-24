//! Background GitHub reads through the Console's HTTP and status seams.
use super::*;
use support::Scenario;

fn start(scenario: &Scenario) -> Console {
    start_binary(scenario, Path::new(env!("CARGO_BIN_EXE_thirdshift")))
}

fn start_binary(scenario: &Scenario, binary: &Path) -> Console {
    let mut command = Command::new(binary);
    command
        .args(["console", "--no-sign-in"])
        .env("HOME", scenario.path("home"))
        .env(
            "PATH",
            format!(
                "{}:{}",
                scenario.path("bin").display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("FAKE_GH_STATE", scenario.path("gh-state.json"))
        .env("FAKE_GH_RECORD", scenario.path("gh-calls.json"))
        .env("THIRDSHIFT_GITHUB_INTERVAL_MS", "250");
    Console::start_command(command)
}

fn pr_is(scenario: &Scenario, state: &str) {
    let mut gh = scenario.gh_state();
    gh["prs"] = json!([{
        "number": 10, "url": "https://github.com/acme/widgets/pull/10",
        "head": "issue-81", "base": "main", "state": state, "isDraft": false,
        "mergedAt": if state == "MERGED" { Some(chrono::Local::now().to_rfc3339()) } else { None },
    }]);
    scenario.write_gh_state(&gh);
}

fn group(page: &str, name: &str) -> String {
    page.split(&format!("data-group=\"{name}\""))
        .nth(1)
        .unwrap()
        .split("</details>")
        .next()
        .unwrap()
        .to_owned()
}

fn events(console: &Console) -> BufReader<reqwest::blocking::Response> {
    BufReader::new(console.get("/events"))
}

fn wait_for(console: &Console, stream: &mut impl BufRead, name: &str) {
    loop {
        assert!(sse_event(stream).contains("event: commands"));
        if group(&console.get("/").text().unwrap(), name).contains("acme/widgets#81") {
            return;
        }
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let began = std::time::Instant::now();
    while !condition() {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "background read did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn hold_first_read(scenario: &Scenario) {
    fs::create_dir(scenario.path("bin/held")).unwrap();
    fs::rename(scenario.path("bin/gh"), scenario.path("bin/held/gh")).unwrap();
    support::write_executable(
        &scenario.path("bin/gh"),
        "#!/bin/sh\nif ! test -f \"$FAKE_GH_STATE.started\"; then\n  touch \"$FAKE_GH_STATE.started\"\n  sleep 2\nfi\nexec \"$(dirname \"$0\")/held/gh\" \"$@\"\n",
    );
}

fn assert_idle_events(console: &Console, duration: Duration) {
    let response = reqwest::blocking::Client::builder()
        .timeout(duration)
        .build()
        .unwrap()
        .get(format!("{}/events", console.address))
        .send()
        .unwrap();
    let mut stream = BufReader::new(response);
    assert!(sse_event(&mut stream).contains("event: commands"));
    let began = std::time::Instant::now();
    loop {
        let mut line = String::new();
        match stream.read_line(&mut line) {
            Ok(0) => panic!("Inbox stream closed"),
            Ok(_) => assert!(
                !line.starts_with("event: commands")
                    || began.elapsed() < Duration::from_millis(500),
                "unchanged read woke the Inbox: {line}"
            ),
            Err(_) => {
                assert!(
                    began.elapsed() >= duration - Duration::from_millis(500),
                    "stream failed before idle observation"
                );
                break;
            }
        }
    }
}

#[test]
fn github_merge_wakes_open_inbox_and_status_reuses_the_answer() {
    let scenario = Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    record(&home, 81, Some("ready for review"), 0);
    pr_is(&scenario, "OPEN");
    let console = start(&scenario);
    let mut stream = events(&console);
    wait_for(&console, &mut stream, "ready for review");
    pr_is(&scenario, "MERGED");
    wait_for(&console, &mut stream, "done");
    assert!(
        !group(&console.get("/").text().unwrap(), "ready for review").contains("acme/widgets#81")
    );
    // Stop the Console to distinguish status's reads from the background reads.
    drop(stream);
    drop(console);
    let before = scenario.gh_calls();
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "done", "{status}");
    assert_eq!(
        status["commands"][0]["left_reason"], "pull request merged",
        "{status}"
    );
    assert_eq!(
        scenario.gh_calls(),
        before,
        "status read GitHub instead of the Console's answer"
    );
}

#[test]
fn github_issue_closure_wakes_needs_you_and_status_reuses_the_answer() {
    let scenario = Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    record(&home, 81, Some("failed"), 10);
    scenario.issue_is(81, "OPEN");
    let console = start(&scenario);
    let mut stream = events(&console);
    // Local age would put this row in History; its first GitHub read must
    // still bring it into Needs you while the issue is open.
    wait_for(&console, &mut stream, "needs you");
    let closed = (chrono::Local::now() - chrono::Duration::hours(1)).to_rfc3339();
    let mut gh = scenario.gh_state();
    gh["issues"]["81"] = json!("CLOSED");
    gh["issue_closed_at"]["81"] = json!(closed);
    scenario.write_gh_state(&gh);
    wait_for(&console, &mut stream, "resolved");
    assert!(!group(&console.get("/").text().unwrap(), "needs you").contains("acme/widgets#81"));
    // Repeated issue reads keep their first label-absence timestamp, so
    // unchanged facts must leave open pages idle too.
    assert_idle_events(&console, Duration::from_secs(2));
    drop(stream);
    drop(console);
    let before = scenario.gh_calls();
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    let row = &status["commands"][0];
    assert_eq!(row["group"], "resolved", "{status}");
    assert_eq!(row["left_reason"], "issue closed", "{status}");
    assert_eq!(row["left_at"], closed, "{status}");
    assert_eq!(
        scenario.gh_calls(),
        before,
        "status read GitHub instead of the Console's answer"
    );
}

#[test]
fn unchanged_github_reads_leave_open_inbox_idle_and_keep_status_fresh() {
    let scenario = Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    record(&home, 81, Some("ready for review"), 0);
    pr_is(&scenario, "OPEN");
    assert_eq!(scenario.run(&["status", "--json"]).code, Some(0));
    age_answer(&home, 600);
    let config = fs::read(home.join(".thirdshift/config.toml")).unwrap();
    let console = start(&scenario);
    // No HTTP request has been made: the Console still reads for this machine.
    wait_until(|| scenario.gh_calls_of("api", "graphql").len() >= 3);
    assert_idle_events(&console, Duration::from_secs(2));
    assert!(scenario.gh_calls_of("api", "graphql").len() >= 6);
    drop(console);
    let before = scenario.gh_calls();
    let output = scenario.run(&["status", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert!(
        status["github_answer_age_seconds"].as_u64().unwrap() < 5,
        "{status}"
    );
    assert_eq!(scenario.gh_calls(), before);
    assert_eq!(
        fs::read(home.join(".thirdshift/config.toml")).unwrap(),
        config
    );
}

#[test]
fn inbox_remains_responsive_during_a_blocked_github_read() {
    let scenario = Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    record(&home, 81, Some("ready for review"), 0);
    pr_is(&scenario, "OPEN");
    // Hold gh at its external seam. A finite pause also bounds cleanup if an
    // assertion fails while the Console is waiting for this child.
    hold_first_read(&scenario);
    let console = start(&scenario);
    wait_until(|| scenario.path("gh-state.json.started").exists());
    let began = std::time::Instant::now();
    assert!(
        group(&console.get("/").text().unwrap(), "ready for review").contains("acme/widgets#81")
    );
    let mut stream = events(&console);
    assert!(sse_event(&mut stream).contains("event: commands"));
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "GitHub blocked HTTP"
    );
    // Wait for the owned gh call before ending the fixture.
    wait_until(|| !scenario.gh_calls_of("api", "graphql").is_empty());
}

#[test]
fn history_freezes_do_not_wake_an_unchanged_inbox() {
    let scenario = Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    let mut old = record(&home, 80, Some("ready for review"), 30);
    old["pull_request"] = json!("https://github.com/acme/widgets/pull/11");
    write_record(&home, 80, &old);
    record(&home, 81, Some("ready for review"), 2);
    pr_is(&scenario, "OPEN");
    let mut gh = scenario.gh_state();
    let merged = (chrono::Local::now() - chrono::Duration::days(20)).to_rfc3339();
    gh["prs"].as_array_mut().unwrap().push(json!({
        "number": 11, "url": "https://github.com/acme/widgets/pull/11",
        "head": "issue-80", "base": "main", "state": "MERGED", "isDraft": false,
        "mergedAt": merged,
    }));
    scenario.write_gh_state(&gh);
    // The first answer moves the old row to History. The Console's next
    // read must keep that row's facts without disturbing the current row.
    assert_eq!(scenario.run(&["status", "--json"]).code, Some(0));
    hold_first_read(&scenario);
    let console = start(&scenario);
    wait_until(|| scenario.path("gh-state.json.started").exists());
    let before = console.get("/").text().unwrap();
    assert!(!before.contains("acme/widgets#80"));
    assert_idle_events(&console, Duration::from_secs(3));
    assert_eq!(console.get("/").text().unwrap(), before);
    assert!(scenario.gh_calls_of("api", "graphql").len() >= 2);
}

#[test]
fn a_binary_update_interrupts_an_inflight_github_read() {
    let scenario = Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    record(&home, 81, Some("ready for review"), 0);
    fs::remove_file(scenario.path("bin/gh")).unwrap();
    support::write_executable(
        &scenario.path("bin/gh"),
        "#!/bin/sh\ntouch \"$HOME/gh-started\"\nsleep 4\nexit 1\n",
    );
    let binary = home.join("thirdshift");
    support::copy_executable(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &binary);
    let console = start_binary(&scenario, &binary);
    wait_until(|| home.join("gh-started").exists());
    let replacement = home.join("replacement");
    support::write_executable(
        &replacement,
        "#!/bin/sh\nprintf 'replacement executed\\n'\n",
    );
    fs::rename(replacement, binary).unwrap();
    let restarted = console.lines.recv_timeout(Duration::from_secs(2));
    if restarted.is_err() {
        // Let the finite held child finish and be reaped even on a regression.
        let _ = console.lines.recv_timeout(Duration::from_secs(6));
    }
    assert_eq!(restarted.unwrap(), "replacement executed");
}

fn age_answer(home: &Path, seconds: u64) {
    for name in ["github-inbox.json", "github-inbox.refreshed"] {
        fs::File::options()
            .write(true)
            .open(home.join(".thirdshift/logs").join(name))
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(seconds))
            .unwrap();
    }
}

#[test]
fn failure_warning_wakes_open_inbox_and_recovers() {
    let scenario = Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    record(&home, 81, Some("ready for review"), 10);
    pr_is(&scenario, "OPEN");
    assert_eq!(scenario.run(&["status", "--json"]).code, Some(0));
    age_answer(&home, 600);
    hold_first_read(&scenario);
    let console = start(&scenario);
    wait_until(|| scenario.path("gh-state.json.started").exists());
    // Start observing while the good read is blocked. Its unchanged answer
    // must stay quiet, but a later failure becoming stale must wake this page.
    let mut stream = events(&console);
    assert!(sse_event(&mut stream).contains("event: commands"));
    wait_until(|| scenario.gh_calls_of("api", "graphql").len() >= 2);
    wait_until(|| {
        fs::metadata(home.join(".thirdshift/logs/github-inbox.refreshed"))
            .unwrap()
            .modified()
            .unwrap()
            .elapsed()
            .unwrap()
            < Duration::from_secs(5)
    });
    let mut gh = scenario.gh_state();
    gh["failing"] = json!(["api graphql"]);
    gh["failing_stderr"] = json!("HTTP 401: Bad credentials <signed out>");
    scenario.write_gh_state(&gh);
    age_answer(&home, 299);
    loop {
        assert!(sse_event(&mut stream).contains("event: commands"));
        let page = console.get("/").text().unwrap();
        if page.contains("GitHub last answered at") {
            assert!(
                page.contains("HTTP 401: Bad credentials &#60;signed out&#62;"),
                "{page}"
            );
            assert!(group(&page, "ready for review").contains("acme/widgets#81"));
            break;
        }
    }
    let output = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&output.stdout).unwrap();
    assert!(
        console
            .get("/")
            .text()
            .unwrap()
            .contains(status["github_last_answered_at"].as_str().unwrap())
    );
    gh["failing"] = json!([]);
    scenario.write_gh_state(&gh);
    pr_is(&scenario, "MERGED");
    drop(stream);
    let mut stream = events(&console);
    wait_for(&console, &mut stream, "resolved");
    assert!(
        !console
            .get("/")
            .text()
            .unwrap()
            .contains("latest read failed")
    );
}

#[test]
fn console_and_status_share_both_quota_reserves_and_resume_at_reset() {
    for (resource, description) in [("graphql", "GraphQL points"), ("core", "REST requests")] {
        let scenario = Scenario::new();
        let home = scenario.path("home");
        configure(&home);
        record(&home, 81, Some("ready for review"), 10);
        pr_is(&scenario, "OPEN");
        assert_eq!(scenario.run(&["status", "--json"]).code, Some(0));
        age_answer(&home, 600);
        let mut gh = scenario.gh_state();
        gh["rate_limits"][resource] = json!({
            "remaining": 499,
            "reset": (chrono::Utc::now() + chrono::Duration::seconds(4)).timestamp(),
        });
        scenario.write_gh_state(&gh);
        let console = start(&scenario);
        wait_until(|| console.get("/").text().unwrap().contains(description));
        let page = console.get("/").text().unwrap();
        assert!(page.contains("499"));
        assert!(page.contains("until"));
        assert!(group(&page, "ready for review").contains("acme/widgets#81"));
        let before = scenario.gh_calls();
        let text = scenario.run(&["status"]);
        assert!(text.stdout.contains(description), "{}", text.stdout);
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(
            scenario.gh_calls(),
            before,
            "Console or status read before reset"
        );
        gh["rate_limits"][resource]["remaining"] = json!(5000);
        scenario.write_gh_state(&gh);
        let mut stream = events(&console);
        loop {
            assert!(sse_event(&mut stream).contains("event: commands"));
            if !console.get("/").text().unwrap().contains(description) {
                break;
            }
        }
        assert!(scenario.gh_calls_of("api", "graphql").len() >= 2);
        drop(stream);
        drop(console);
        assert!(!scenario.run(&["status"]).stdout.contains("Warning:"));
    }
}
