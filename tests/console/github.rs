//! Background GitHub reads through the Console's HTTP and status seams.
use super::*;
use support::Scenario;

fn start(scenario: &Scenario) -> Console {
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
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
fn unchanged_github_reads_leave_open_inbox_idle_and_keep_status_fresh() {
    let scenario = Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    record(&home, 81, Some("ready for review"), 0);
    pr_is(&scenario, "OPEN");
    assert_eq!(scenario.run(&["status", "--json"]).code, Some(0));
    for name in ["github-inbox.json", "github-inbox.refreshed"] {
        fs::File::options()
            .write(true)
            .open(home.join(".thirdshift/logs").join(name))
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(600))
            .unwrap();
    }
    let config = fs::read(home.join(".thirdshift/config.toml")).unwrap();
    let console = start(&scenario);
    // No HTTP request has been made: the Console still reads for this machine.
    wait_until(|| scenario.gh_calls_of("api", "graphql").len() >= 3);
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
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
                    began.elapsed() >= Duration::from_millis(1500),
                    "stream failed before idle observation"
                );
                break;
            }
        }
    }
    assert!(scenario.gh_calls_of("api", "graphql").len() >= 6);
    drop(stream);
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
    fs::create_dir(scenario.path("bin/held")).unwrap();
    fs::rename(scenario.path("bin/gh"), scenario.path("bin/held/gh")).unwrap();
    support::write_executable(
        &scenario.path("bin/gh"),
        "#!/bin/sh\nif ! test -f \"$FAKE_GH_STATE.started\"; then\n  touch \"$FAKE_GH_STATE.started\"\n  sleep 2\nfi\nexec \"$(dirname \"$0\")/held/gh\" \"$@\"\n",
    );
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
