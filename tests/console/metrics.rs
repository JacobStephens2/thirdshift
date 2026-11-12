use super::*;

#[test]
fn peek_shows_historical_run_metrics_without_rewriting_the_record() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut value = record(home.path(), 7, Some("merged"), 0);
    value["kind"] = json!("Run");
    value["version"] = json!(1);
    value["started_at"] = json!("2026-10-19T03:30:00Z");
    value["ended_at"] = json!("2026-10-19T04:00:00Z");
    value["stages"] = json!([{
        "name": "implement", "began_at": "2026-10-19T03:30:00Z"
    }]);
    write_record(home.path(), 7, &value);
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.command.json");
    let original = fs::read(&path).unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = console.get("/peek?command=widgets%237");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains("<dt>total minutes</dt><dd>30.0"), "{html}");
    assert!(
        html.contains("<dt>review findings</dt><dd>Missing"),
        "{html}"
    );
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn peek_refreshes_confirmed_bug_origins_without_rewriting_run_metrics() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut value = record(home.path(), 7, Some("merged"), 0);
    value["kind"] = json!("Run");
    value["pull_request"] = json!("https://github.com/acme/widgets/pull/23");
    value["run_metrics"] = json!({
        "outcome": "merged", "repairs": 0, "ci_rounds": 1,
        "stages": [], "total_minutes": 30.0, "session_signals": {}
    });
    write_record(home.path(), 7, &value);
    let scoring = home.path().join(".thirdshift/evals/scoring");
    fs::create_dir_all(&scoring).unwrap();
    let mut week = json!({
        "window_end": "2026-10-26T00:00:00Z",
        "bug_origins": [{
            "issue": "https://github.com/acme/widgets/issues/99", "state": "confirmed",
            "settlement": {"pull_request": {
                "url": "https://github.com/acme/widgets/pull/23", "run": null
            }}
        }]
    });
    let checkpoint = scoring.join("week.json");
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    for (state, expected) in [
        ("proposed", 0),
        ("confirmed", 1),
        ("rejected", 0),
        ("confirmed", 1),
        ("future_state", 0),
    ] {
        week["bug_origins"][0]["state"] = json!(state);
        fs::write(&checkpoint, serde_json::to_vec(&week).unwrap()).unwrap();
        let response = console.get("/peek?command=widgets%237");
        assert_eq!(response.status(), 200);
        let html = response.text().unwrap();
        assert!(
            html.contains(&format!("<dt>confirmed bug origins</dt><dd>{expected}")),
            "{state}: {html}"
        );
        let output = Command::new(binary)
            .args(["show", "widgets#7", "--json"])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let shown: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            shown["command"]["run_metrics"]["confirmed_bug_origins"], expected,
            "{state}"
        );
    }
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.command.json");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap(),
        value
    );
}

#[test]
fn peek_shows_retained_run_metrics_and_marks_missing_signals() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut value = record(home.path(), 7, Some("merged"), 0);
    value["kind"] = json!("Run");
    value["run_metrics"] = json!({
        "outcome": "merged", "repairs": 0, "ci_rounds": 2,
        "unaddressed_findings": {"standards": 1, "spec": 0, "security": null},
        "total_minutes": 12.5,
        "session_signals": null,
        "stages": [{
            "name": "implement", "seconds": 750.0, "minutes": 12.5,
            "usual_range": null, "position": "too_few_runs",
            "review_findings": {"standards": 3, "spec": null},
            "session_signals": {
                "harness":"claude", "harness_version":"1.0",
                "signals": {
                    "tokens_by_model":[], "cost_usd":null, "tool_errors":0,
                    "permission_denials":0, "killed_work":0, "repeated_tool_calls":0,
                    "destructive_commands":[{
                        "pattern":"force-push", "command":"git push --force origin main"
                    }]
                }
            }
        }]
    });
    write_record(home.path(), 7, &value);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let output = Command::new(binary)
        .args(["show", "widgets#7", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let shown: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["run_metrics"]["session_signals"] = json!({});
    value["run_metrics"]["confirmed_bug_origins"] = json!(0);
    assert_eq!(shown["command"]["run_metrics"], value["run_metrics"]);

    let response = console.get("/peek?command=widgets%237");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    let metrics = html.split_once("<dt>run metrics</dt>").unwrap().1;
    for expected in [
        "<dt>outcome</dt><dd>merged",
        "<dt>repairs</dt><dd>0",
        "<dt>ci rounds</dt><dd>2",
        "<dt>unaddressed findings</dt>",
        "<dt>standards</dt><dd>1",
        "<dt>spec</dt><dd>0",
        "<dt>security</dt><dd>Missing",
        "<dt>total minutes</dt><dd>12.5",
        "<dt>name</dt><dd>implement",
        "<dt>seconds</dt><dd>750.0",
        "<dt>minutes</dt><dd>12.5",
        "<dt>usual range</dt><dd>Missing",
        "<dt>position</dt><dd>too_few_runs",
        "<dt>review findings</dt>",
        "<dt>standards</dt><dd>3",
        "<dt>spec</dt><dd>Missing",
        "<dt>destructive commands</dt>",
        "<dt>pattern</dt><dd>force-push",
        "<dt>command</dt><dd>git push --force origin main",
    ] {
        assert!(metrics.contains(expected), "missing {expected}: {metrics}");
    }
}

#[test]
fn metrics_page_shows_outcome_stacks_and_tables_and_keeps_single_command_inspection() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 7, Some("merged"), 0);
    record(home.path(), 8, Some("failed"), 0);
    record(home.path(), 9, Some("paused"), 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = console.get("/metrics?window=7d&kind=Spec+run&repo=&harness=");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    for expected in [
        "<a href=\"/metrics\" data-jump=\"m\">Metrics</a>",
        "<h1>Metrics</h1>",
        "aria-label=\"Compared with the preceding window\"",
        "<svg class=\"metrics-bar\"",
        "<rect class=\"succeeded\"",
        "<title>Succeeded 1: merged 1</title>",
        "<title>Failed 1: failed 1</title>",
        "<title>Paused 1: paused 1</title>",
        "<td>50% (1/2)</td>",
        "<option value=\"7d\" selected>",
        "<option selected>Spec run</option>",
        "--window 7d --kind 'Spec run' --json",
    ] {
        assert!(html.contains(expected), "missing {expected}: {html}");
    }
    assert!(!html.contains("<pre>"), "{html}");
    let refused = console.get("/metrics?window=7d&grain=week");
    assert_eq!(refused.status(), 200);
    let refused = refused.text().unwrap();
    assert!(
        refused.contains("<p role=\"alert\">--window sets its own grain and dates"),
        "{refused}"
    );
    let single = console.get("/metrics?command=widgets%237").text().unwrap();
    assert!(single.starts_with("<pre>{"), "{single}");
    let css = console.get("/assets/console.css").text().unwrap();
    for line in css.lines().filter(|line| line.contains("metrics")) {
        assert!(
            !line.contains("animation") && !line.contains("transition"),
            "{line}"
        );
        assert!(!line.contains('#'), "Theme tokens only: {line}");
    }
}

#[test]
fn metrics_form_keeps_every_repeated_filter_value() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 7, Some("merged"), 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console
        .get("/metrics?repo=acme/widgets&repo=acme/gadgets")
        .text()
        .unwrap();
    assert!(html.contains("--repo acme/gadgets"), "{html}");
    assert!(
        html.contains("value=\"acme/gadgets\""),
        "form lost the second repo: {html}"
    );
}

#[test]
fn metrics_page_shows_the_four_signal_groups_as_the_api_reports_them() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    for (number, outcome, minutes, repairs) in [
        (7, "merged", 30.0, 0),
        (8, "merged", 50.0, 2),
        (9, "failed", 0.5, 0),
    ] {
        let mut value = record(home.path(), number, Some(outcome), 1);
        value["run_metrics"] = json!({
            "outcome": outcome, "repairs": repairs, "ci_rounds": 1,
            "unaddressed_findings": {"standards": 4, "spec": 2, "security": 0},
            "total_minutes": minutes, "session_signals": {},
            "stages": [{
                "name": "implement", "seconds": minutes * 60.0, "minutes": minutes,
                "usual_range": {"lower_seconds": 600.0, "upper_seconds": 1200.0},
                "position": if minutes > 20.0 { "above" } else { "within" },
                "review_findings": {"standards": 3, "spec": 1},
                "session_signals": {"harness": "codex", "harness_version": "1.0", "signals": {
                    "tokens_by_model": [], "cost_usd": minutes / 10.0, "tool_errors": 1,
                    "permission_denials": 0, "killed_work": 0, "repeated_tool_calls": 2,
                    "destructive_commands": []
                }}
            }]
        });
        write_record(home.path(), number, &value);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let api: Value = console.get("/api/v1/metrics?window=7d").json().unwrap();
    let efficiency = &api["groups"]["efficiency"]["lines"][0];
    let window = &efficiency["window"];
    assert_eq!(window["first_pass_clean_rate"], 0.5, "{api:#}");
    assert_eq!(window["minutes_per_success"], 40.25);
    assert_eq!(window["cost_usd_per_success"], 4.025);
    assert_eq!(efficiency["model"], "gpt-6");
    let html = console.get("/metrics?window=7d").text().unwrap();
    for expected in [
        "<h2 id=\"metrics-yield\">Yield and pull-request survival</h2>",
        "<h2 id=\"metrics-quality\">Post-merge quality and interventions</h2>",
        "<h2 id=\"metrics-efficiency\">Efficiency, wasted effort and Repair churn</h2>",
        "<h2 id=\"metrics-friction\">Stage bottlenecks and session friction</h2>",
        "<svg class=\"metrics-spark\"",
        "<td>gpt-6</td>",
        "<td>1/2 (50%)</td>",
        "<td>40.2 (3 observed, 0 missing)</td>",
        "<td>$4.03 (3 observed, 0 missing)</td>",
        "<td>2/3 (67%)</td>",
        "<th scope=\"col\">Change</th>",
    ] {
        assert!(html.contains(expected), "missing {expected}: {html}");
    }
    // Findings show only as Security holds, not as a quality score.
    let groups = html.split_once("metrics-efficiency").unwrap().1;
    assert!(!groups.contains("Standards findings") && !groups.contains("Spec findings"));
}

#[test]
fn metrics_page_heads_the_yield_group_without_a_bounded_window() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/metrics").text().unwrap();
    assert!(html.contains("<h2 id=\"metrics-yield\">"), "{html}");
    assert_eq!(
        html.matches("<section").count(),
        html.matches("</section>").count(),
        "{html}"
    );
}

#[test]
fn metrics_page_heads_the_yield_group_when_unbounded() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for query in ["/metrics", "/metrics?window=all"] {
        let html = console.get(query).text().unwrap();
        assert!(
            html.contains("<h2 id=\"metrics-yield\">Yield and pull-request survival</h2>"),
            "{query}: {html}"
        );
        let main = html
            .split_once("<main")
            .unwrap()
            .1
            .split_once("</main>")
            .unwrap()
            .0;
        assert_eq!(
            main.matches("<section").count(),
            main.matches("</section>").count(),
            "{query}"
        );
    }
}

#[test]
fn where_runs_failed_and_failed_buckets_open_the_matching_commands_in_history() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut held = record(home.path(), 7, Some("failed"), 0);
    held["thirdshift_version"] = json!("0.30.0");
    held["stopped_at"] = json!("Self-merge");
    held["failure_category"] = json!("held");
    write_record(home.path(), 7, &held);
    let mut older = record(home.path(), 8, Some("failed"), 1);
    older["thirdshift_version"] = json!("0.29.0");
    write_record(home.path(), 8, &older);
    record(home.path(), 9, Some("merged"), 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let api: Value = console
        .get("/api/v1/metrics?window=7d&grain=")
        .json()
        .unwrap();
    let failures = &api["where_runs_failed"];
    assert_eq!(
        (
            &failures["failed"],
            &failures["observed"],
            &failures["missing"]
        ),
        (&json!(2), &json!(1), &json!(1)),
        "{api:#}"
    );
    let html = console.get("/metrics?window=7d").text().unwrap();
    assert!(
        html.contains("<h2 id=\"metrics-failures\">Where runs failed</h2>"),
        "{html}"
    );
    assert!(
        html.contains("◆ </span>thirdshift 0.29.0 → 0.30.0"),
        "{html}"
    );
    let hrefs: Vec<String> = html
        .split("<a href=\"")
        .skip(1)
        .filter_map(|tail| {
            tail.split_once('"')
                .map(|(href, _)| href.replace("&#38;", "&"))
        })
        .collect();
    // Which fixture Commands a History page shows.
    let shown = |link: &str| {
        let history = console.get(link).text().unwrap();
        assert!(
            history.contains("Failed Commands a Metrics link chose"),
            "{link}: {history}"
        );
        [7, 8, 9]
            .into_iter()
            .filter(|number| history.contains(&format!("issues/{number}\"")))
            .collect::<Vec<_>>()
    };
    let number = |selector: &Value| {
        if selector.as_str().unwrap().contains("/7-fixture.") {
            7
        } else {
            8
        }
    };
    // The page shows each row the API reports, linked to History by the row
    // and the page's own window, which shows just that row's Commands.
    for row in failures["rows"].as_array().unwrap() {
        let mut url = reqwest::Url::parse("http://localhost/history").unwrap();
        url.query_pairs_mut().append_pair("window", "7d");
        if let Some(stage) = row["stopped_at"].as_str() {
            url.query_pairs_mut().append_pair("stopped_at", stage);
        }
        url.query_pairs_mut()
            .append_pair("category", row["category"].as_str().unwrap());
        let link = format!("{}?{}", url.path(), url.query().unwrap());
        assert!(hrefs.contains(&link), "missing {link}: {hrefs:#?}");
        assert_eq!(
            shown(&link),
            [number(&row["commands"][0]["selector"])],
            "{link}"
        );
        let stage = row["stopped_at"].as_str().unwrap_or("no stage");
        let category = row["category"].as_str().unwrap();
        assert!(
            html.contains(&format!("{stage}</a></th><td>{category}</td>")),
            "missing {stage} {category}: {html}"
        );
    }
    // Each day's Failed count opens that day's Failed Commands, as the API lists them.
    let buckets = api["outcomes"].as_array().unwrap();
    assert_eq!(buckets.len(), 2, "{api:#}");
    for bucket in buckets {
        let failed = bucket["failed_commands"].as_array().unwrap();
        assert_eq!(failed.len(), 1, "{bucket:#}");
        let day = bucket["bucket"].as_str().unwrap();
        let link = hrefs
            .iter()
            .find(|href| href.contains(&format!("failed_on={day}")))
            .unwrap_or_else(|| panic!("no Failed link for {day}: {hrefs:#?}"));
        assert_eq!(shown(link), [number(&failed[0])], "{link}");
    }
    // Each Command details link opens.
    let details: Vec<_> = hrefs
        .iter()
        .filter(|href| href.starts_with("/peek?command="))
        .collect();
    assert_eq!(details.len(), 2, "{hrefs:#?}");
    for href in details {
        assert_eq!(console.get(href).status(), 200, "{href}");
    }
    // A version slices the view as it stands.
    let html = console
        .get("/metrics?window=7d&repo=acme/widgets")
        .text()
        .unwrap();
    assert!(
        html.contains("href=\"/metrics?window=7d&#38;repo=acme%2Fwidgets&#38;version=0.30.0\""),
        "{html}"
    );
}

#[test]
fn failure_drilldown_links_stay_within_a_proxy_request_line() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    for number in 1..=120 {
        let mut failed = record(home.path(), number, Some("failed"), 0);
        failed["stopped_at"] = json!("CI watch");
        failed["failure_category"] = json!("stage_failed");
        write_record(home.path(), number, &failed);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/metrics?window=7d").text().unwrap();
    for href in html
        .split("<a href=\"")
        .skip(1)
        .filter_map(|t| t.split_once('"').map(|(h, _)| h.to_owned()))
    {
        if href.starts_with("/history?") {
            // Apache's default LimitRequestLine, less "GET " and " HTTP/1.1".
            assert!(href.len() <= 8190 - 13, "{} bytes", href.len());
        }
    }
}

/// `metrics` on `home`, or its failure.
fn metrics(home: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .arg("metrics")
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

/// A native saved-slot fixture, including the digest requirement for fallback.
fn saved_slot(
    home: &Path,
    name: &str,
    reported: bool,
    at: &str,
    query: &str,
) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(name.as_bytes());
    if reported {
        digest.update([1]);
    }
    let hex: String = digest.finalize()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let directory = home.join(".thirdshift/logs/fleet-metrics");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{hex}.json"));
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "version": 1, "machine": name, "person": null, "last_seen": at,
            "pushed_at": reported.then_some(at), "views": {query: {
                "fetched_at": at, "filters": if query.contains("kind=") { "kind=Run" } else { "" },
                "weekly": false, "summary": {"outcomes": [], "comparison": null}
            }}
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn metrics_peer(home: &Path, address: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(
        home.join(".thirdshift/config.toml"),
        format!(
            "[console]\nport = 0\nfleet = [{{ name = 'capture-peer', address = '{address}' }}]\n"
        ),
    )
    .unwrap();
    let credentials = home.join(".thirdshift/credentials.toml");
    fs::write(&credentials, "[fleet]\ncapture-peer = 'fixture-token'\n").unwrap();
    fs::set_permissions(credentials, fs::Permissions::from_mode(0o600)).unwrap();
}

fn metrics_listener() -> std::net::TcpListener {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn metrics_request(listener: &std::net::TcpListener) -> std::net::TcpStream {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line, "GET /api/v1/metrics?window=7d HTTP/1.1\r\n");
                loop {
                    line.clear();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "metrics peer request never arrived"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

fn metrics_reply(mut stream: std::net::TcpStream, code: u16, body: &str) {
    use std::io::Write;
    write!(
        stream,
        "HTTP/1.1 {code} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}

#[test]
fn console_metrics_reuses_captured_choices_and_results_during_a_live_read() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let peer = metrics_listener();
    metrics_peer(
        home.path(),
        &format!("http://{}", peer.local_addr().unwrap()),
    );
    let at = chrono::Utc::now().to_rfc3339();
    let retired = saved_slot(home.path(), "capture-retired", false, &at, "window=7d");
    // Saving the live result fails, but must not cost the result or captured sources.
    let peer_slot = saved_slot(home.path(), "capture-peer", false, &at, "window=7d");
    fs::remove_file(&peer_slot).unwrap();
    fs::create_dir(peer_slot.with_extension("lock")).unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let address = &console.address;
    let html = std::thread::scope(|threads| {
        let response = threads.spawn(|| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap()
                .get(format!("{address}/metrics?window=7d"))
                .send()
                .unwrap()
                .text()
                .unwrap()
        });
        let request = metrics_request(&peer);
        fs::remove_file(retired).unwrap();
        saved_slot(home.path(), "capture-late", true, &at, "window=7d");
        metrics_reply(request, 200, r#"{"outcomes":[],"comparison":null}"#);
        response.join().unwrap()
    });
    assert!(html.contains("value=\"capture-retired\""), "{html}");
    assert!(
        html.contains("<strong>capture-retired</strong>"),
        "choices and results must use the same capture: {html}"
    );
    assert!(!html.contains("capture-late"), "{html}");
    assert!(html.contains("<strong>capture-peer</strong>"), "{html}");
    assert!(
        html.contains("snapshot not saved:"),
        "live data survives save failure: {html}"
    );
    let next = console
        .get("/metrics?machine=capture-late&window=7d")
        .text()
        .unwrap();
    assert!(next.contains("<strong>capture-late</strong>"), "{next}");
    assert!(!next.contains("capture-retired"), "{next}");
}

#[test]
fn console_metrics_observes_a_saved_path_once_and_the_local_api_never_observes_it() {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let at = chrono::Utc::now().to_rfc3339();
    let path = saved_slot(home.path(), "capture-once", true, &at, "window=7d");
    let contents = fs::read(&path).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    // No writer is attached yet: reading saved Fleet sources would block this
    // local API request until the HTTP client's deadline, failing the test.
    let local = api::json(console.get("/api/v1/metrics?window=7d"));
    assert!(local.get("machines").is_none(), "{local}");
    let address = &console.address;
    let html = std::thread::scope(|threads| {
        let response = threads.spawn(|| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap()
                .get(format!("{address}/metrics?machine=capture-once&window=7d"))
                .send()
                .unwrap()
                .text()
                .unwrap()
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let mut writer = loop {
            match fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
            {
                Ok(writer) => break writer,
                Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "saved source was never observed"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        writer.write_all(&contents).unwrap();
        // The reader gets EOF only after removal. A second capture would see
        // an unknown machine; the request must reuse its first observation.
        fs::remove_file(&path).unwrap();
        drop(writer);
        response.join().unwrap()
    });
    assert!(html.contains("value=\"capture-once\""), "{html}");
    assert!(html.contains("<strong>capture-once</strong>"), "{html}");
    let next = console.get("/metrics?machine=capture-once&window=7d");
    assert_ne!(next.status(), 200, "a new request sees the removed source");
}

#[test]
fn unavailable_metrics_choose_exact_slots_before_views_and_never_override_refusals() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let peer = metrics_listener();
    metrics_peer(
        home.path(),
        &format!("http://{}", peer.local_addr().unwrap()),
    );
    let at = "2026-11-12T00:00:00Z";
    saved_slot(home.path(), "capture-peer", false, at, "window=7d");
    saved_slot(home.path(), "capture-peer", true, at, "window=7d");
    let load = |code: u16| {
        std::thread::scope(|threads| {
            let result = threads.spawn(|| {
                metrics(
                    home.path(),
                    &["--machine", "capture-peer", "--window", "7d", "--json"],
                )
            });
            let request = metrics_request(&peer);
            metrics_reply(request, code, "{}");
            let result = result.join().unwrap();
            assert!(result.status.success(), "{result:?}");
            serde_json::from_slice::<Value>(&result.stdout).unwrap()
        })
    };
    let tied = load(503);
    assert_eq!(tied["machines"][0]["status"], "reported", "{tied}");
    saved_slot(
        home.path(),
        "capture-peer",
        false,
        "2026-11-12T00:00:01Z",
        "window=7d",
    );
    let newer_read = load(503);
    assert_eq!(
        newer_read["machines"][0]["status"], "cached",
        "{newer_read}"
    );
    saved_slot(
        home.path(),
        "capture-peer",
        true,
        "2026-11-12T00:00:02Z",
        "window=30d&kind=Run",
    );
    let newer_report = load(503);
    assert_eq!(
        newer_report["machines"][0]["status"], "reported",
        "{newer_report}"
    );
    assert!(
        newer_report["machines"][0]["error"]
            .as_str()
            .unwrap()
            .contains("no saved snapshot for this view")
    );
    for (code, status) in [
        (403, "refused"),
        (404, "incompatible"),
        (200, "incompatible"),
    ] {
        let refused = load(code);
        assert_eq!(refused["machines"][0]["status"], status, "{refused}");
        assert!(refused["machines"][0]["last_seen"].is_null(), "{refused}");
        assert!(
            refused["outcomes"].as_array().unwrap().is_empty(),
            "{refused}"
        );
    }
}

fn spec_runs(fleet: &Value) -> &Value {
    fleet["comparison"]["fleet"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "Spec run")
        .unwrap()
}

#[test]
fn fleet_metrics_accept_older_peer_and_saved_summaries_without_unreadable_accounts() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let listener = metrics_listener();
    metrics_peer(
        home.path(),
        &format!("http://{}", listener.local_addr().unwrap()),
    );
    let live = std::thread::scope(|threads| {
        let result = threads.spawn(|| {
            api::cli(
                home.path(),
                &[
                    "metrics",
                    "--machine",
                    "capture-peer",
                    "--window",
                    "7d",
                    "--json",
                ],
            )
        });
        metrics_reply(metrics_request(&listener), 200, r#"{"outcomes":[]}"#);
        result.join().unwrap()
    });
    assert_eq!(live["machines"][0]["status"], "ok");
    assert_eq!(live["machines"][0]["unavailable_accounts"], json!([]));
    drop(listener);
    // Replace the newly saved summary with an older snapshot missing the field.
    saved_slot(
        home.path(),
        "capture-peer",
        false,
        &chrono::Utc::now().to_rfc3339(),
        "window=7d",
    );
    let cached = api::cli(
        home.path(),
        &[
            "metrics",
            "--machine",
            "capture-peer",
            "--window",
            "7d",
            "--json",
        ],
    );
    assert_eq!(cached["machines"][0]["status"], "cached");
    assert_eq!(cached["machines"][0]["unavailable_accounts"], json!([]));
}

#[test]
fn fleet_metrics_keep_unreadable_accounts_per_machine() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    for (home, accounts) in [
        (local_home.path(), "['thirdshift-missing-local']"),
        (
            remote_home.path(),
            "['thirdshift-missing-b', 'thirdshift-missing-a']",
        ),
    ] {
        configure(home);
        let path = home.join(".thirdshift/config.toml");
        let config = fs::read_to_string(&path).unwrap();
        fs::write(path, format!("{config}other_accounts = {accounts}\n")).unwrap();
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let mut remote = Some(Console::start(binary, remote_home.path()));
    let local = Console::start(binary, local_home.path());
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    let paired = super::fleet::add(
        local_home.path(),
        "remote",
        &remote.as_ref().unwrap().address,
        &token,
    );
    assert!(paired.status.success(), "{paired:?}");
    let expected = json!(["thirdshift-missing-a", "thirdshift-missing-b"]);
    let own = api::cli(remote_home.path(), &["metrics", "--json"]);
    assert_eq!(own["unavailable_accounts"], expected);

    for status in ["ok", "cached"] {
        if status == "cached" {
            drop(remote.take());
        }
        let fleet = api::cli(local_home.path(), &["metrics", "--fleet", "--json"]);
        let machines = fleet["machines"].as_array().unwrap();
        assert_eq!(
            machines[0]["unavailable_accounts"],
            json!(["thirdshift-missing-local"])
        );
        let peer = machines.iter().find(|m| m["name"] == "remote").unwrap();
        assert_eq!(peer["status"], status);
        assert_eq!(peer["unavailable_accounts"], expected);
        let one = api::cli(
            local_home.path(),
            &["metrics", "--machine", "remote", "--json"],
        );
        assert_eq!(one["machines"][0]["unavailable_accounts"], expected);
        let text = metrics(local_home.path(), &["--fleet"]);
        assert!(text.status.success(), "{text:?}");
        let text = String::from_utf8_lossy(&text.stdout);
        let line = text.lines().find(|line| line.contains("remote")).unwrap();
        assert!(line.contains(&format!("status: {status}")), "{text}");
        assert!(
            line.contains("unreadable accounts: thirdshift-missing-a, thirdshift-missing-b"),
            "{text}"
        );
        let html = local.get("/metrics").text().unwrap();
        let machines = html
            .split("class=\"metrics-machines\"")
            .nth(1)
            .unwrap()
            .split("</ul>")
            .next()
            .unwrap();
        assert!(
            machines.contains("unreadable accounts: thirdshift-missing-local"),
            "{machines}"
        );
        assert!(
            machines.contains("unreadable accounts: thirdshift-missing-a, thirdshift-missing-b"),
            "{machines}"
        );
    }
}

#[test]
fn fleet_metrics_merge_paired_consoles_and_keep_an_offline_peer_cached() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    configure(local_home.path());
    configure(remote_home.path());
    for (home, number, outcome) in [
        (local_home.path(), 71, "failed"),
        (remote_home.path(), 72, "merged"),
        (remote_home.path(), 73, "failed"),
    ] {
        let mut value = record(home, number, Some(outcome), 0);
        value["name"] = json!(format!("widgets#{number}"));
        write_record(home, number, &value);
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start(binary, remote_home.path());
    let local = Console::start(binary, local_home.path());
    // Before pairing, only a link-only entry is configured: the page stays local.
    assert!(
        local
            .get("/metrics")
            .text()
            .unwrap()
            .contains("Yield and pull-request survival")
    );
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    let paired = super::fleet::add(local_home.path(), "remote", &remote.address, &token);
    assert!(paired.status.success(), "{paired:?}");
    let hostname = api::json(local.get("/api/v1/whoami"))["hostname"]
        .as_str()
        .unwrap()
        .to_owned();

    let fleet = api::cli(
        local_home.path(),
        &["metrics", "--fleet", "--window", "7d", "--json"],
    );
    assert!(fleet["machine"].is_null(), "{fleet}");
    let states: Vec<_> = fleet["machines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|machine| {
            (
                machine["name"].as_str().unwrap(),
                machine["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        states,
        [
            (hostname.as_str(), "ok"),
            ("other.example.test", "link-only"),
            ("remote", "ok")
        ]
    );
    // Each machine's own counts, added once: the Fleet is their sum.
    let alone = |home: &Path| {
        api::cli(home, &["metrics", "--window", "7d", "--json"])["comparison"]["rows"][0]["current"]
            .clone()
    };
    let (mine, theirs) = (alone(local_home.path()), alone(remote_home.path()));
    let total = spec_runs(&fleet);
    for count in ["runs", "succeeded", "failed"] {
        assert_eq!(
            total["current"][count].as_u64(),
            Some(mine[count].as_u64().unwrap() + theirs[count].as_u64().unwrap()),
            "{count}: {fleet}"
        );
    }
    assert_eq!(
        (&total["current"]["succeeded"], &total["current"]["failed"]),
        (&json!(1), &json!(2))
    );
    assert_eq!(total["delta"]["runs"], 3);
    let machines: Vec<_> = fleet["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| bucket["machine"].as_str().unwrap())
        .collect();
    assert_eq!(machines, [hostname.as_str(), "remote"]);
    let failed = &fleet["where_runs_failed"];
    assert_eq!(failed["failed"], 2);
    assert_eq!(failed["rows"][0]["failed"], 2);
    assert_eq!(
        failed["rows"][0]["machines"],
        json!({hostname.as_str(): 1, "remote": 1})
    );

    // The page defaults to the Fleet and opens a peer's Failed Command there.
    let html = local.get("/metrics?window=7d").text().unwrap();
    assert!(html.contains("<h1>Metrics · Fleet</h1>"), "{html}");
    assert!(
        html.contains("thirdshift metrics --fleet --window 7d --json"),
        "{html}"
    );
    let peer_details = html
        .split("<a href=\"")
        .skip(1)
        .filter_map(|tail| {
            tail.split_once('"')
                .map(|(href, _)| href.replace("&#38;", "&"))
        })
        .find(|href| href.starts_with("/peek?") && href.ends_with("&machine=remote"))
        .unwrap_or_else(|| panic!("{html}"));
    assert!(
        peer_details.contains("73-fixture.command.json"),
        "{peer_details}"
    );
    let details = local.get(&peer_details);
    assert_eq!(details.status(), 200);
    assert!(details.text().unwrap().contains("73"));

    // The API stays local: it refuses to read the Fleet for a caller.
    for query in [
        "?fleet=true",
        "?machine=remote",
        &format!("?machine={hostname}"),
    ] {
        let refused = local.get(&format!("/api/v1/metrics{query}"));
        assert_eq!(refused.status(), 400, "{query}");
        assert!(refused.text().unwrap().contains("explicit client view"));
    }

    // One machine at a time, on the CLI and the page.
    let only = api::cli(
        local_home.path(),
        &["metrics", "--machine", "remote", "--window", "7d", "--json"],
    );
    assert_eq!(only["machine"], "remote");
    assert_eq!(only["machines"].as_array().unwrap().len(), 1);
    assert_eq!(spec_runs(&only)["current"], theirs);
    let html = local
        .get("/metrics?machine=remote&window=7d")
        .text()
        .unwrap();
    assert!(html.contains("<h1>Metrics · remote</h1>"), "{html}");
    assert!(
        html.contains("thirdshift metrics --machine remote --window 7d --json"),
        "{html}"
    );
    let here = api::cli(
        local_home.path(),
        &[
            "metrics",
            "--machine",
            &hostname,
            "--window",
            "7d",
            "--json",
        ],
    );
    assert_eq!(here["comparison"]["rows"][0]["current"], mine);
    let html = local
        .get(&format!("/metrics?machine={hostname}&window=7d"))
        .text()
        .unwrap();
    assert!(html.contains("Yield and pull-request survival"), "{html}");
    assert!(
        html.contains(&format!("<option value=\"{hostname}\" selected>")),
        "{html}"
    );

    // An offline peer keeps its last-read history, marked cached.
    drop(remote);
    let started = std::time::Instant::now();
    let offline = api::cli(
        local_home.path(),
        &["metrics", "--fleet", "--window", "7d", "--json"],
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    let peer = offline["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|machine| machine["name"] == "remote")
        .unwrap();
    assert_eq!(peer["status"], "cached", "{offline}");
    assert!(chrono::DateTime::parse_from_rfc3339(peer["last_seen"].as_str().unwrap()).is_ok());
    assert!(
        peer["error"]
            .as_str()
            .unwrap()
            .contains("failed or timed out"),
        "{peer}"
    );
    assert_eq!(spec_runs(&offline)["current"], spec_runs(&fleet)["current"]);
    assert_eq!(offline["where_runs_failed"]["failed"], 2);
    let html = local.get("/metrics?window=7d").text().unwrap();
    assert!(
        html.contains("cached, offline</span> · last seen <time"),
        "{html}"
    );
    assert!(
        html.contains("widgets#73 (cached, acme/widgets, <a href=\"https://github.com/acme/widgets/issues/73\">issue</a>) on remote"),
        "{html}"
    );
    let text = metrics(local_home.path(), &["--fleet", "--window", "7d"]);
    assert!(text.status.success(), "{text:?}");
    assert!(String::from_utf8_lossy(&text.stdout).contains("status: cached"));

    // A retired machine's snapshot stays too, and can still be chosen.
    let removed = Command::new(binary)
        .args(["console", "fleet", "remove", "remote"])
        .env("HOME", local_home.path())
        .output()
        .unwrap();
    assert!(removed.status.success(), "{removed:?}");
    let retired = api::cli(
        local_home.path(),
        &["metrics", "--machine", "remote", "--window", "7d", "--json"],
    );
    assert_eq!(retired["machines"][0]["status"], "cached");
    assert_eq!(
        retired["machines"][0]["error"],
        "no longer in this Console's Fleet"
    );
    assert_eq!(spec_runs(&retired)["current"], theirs);
    let html = local.get("/metrics").text().unwrap();
    assert!(
        html.contains("<h1>Metrics · Fleet</h1>"),
        "a saved snapshot keeps Fleet the default"
    );
    let unknown = metrics(local_home.path(), &["--machine", "nowhere"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown Fleet machine"));
}

#[test]
fn api_metrics_stays_local_once_a_peer_is_paired() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    configure(local_home.path());
    configure(remote_home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start(binary, remote_home.path());
    let local = Console::start(binary, local_home.path());
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    let paired = super::fleet::add(local_home.path(), "remote", &remote.address, &token);
    assert!(paired.status.success(), "{paired:?}");
    let body = api::json(local.get("/api/v1/metrics?window=7d"));
    assert!(
        body.get("machine").is_none(),
        "API answered with Fleet metrics: {body}"
    );
    assert!(body.get("machines").is_none(), "{body}");
}

#[test]
fn a_fleet_machine_named_fleet_can_be_chosen_alone() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    configure(local_home.path());
    configure(remote_home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start(binary, remote_home.path());
    let local = Console::start(binary, local_home.path());
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    let paired = super::fleet::add(local_home.path(), "fleet", &remote.address, &token);
    assert!(paired.status.success(), "{paired:?}");
    let one = api::cli(
        local_home.path(),
        &["metrics", "--machine", "fleet", "--json"],
    );
    let fleet = api::cli(local_home.path(), &["metrics", "--fleet", "--json"]);
    assert_ne!(
        one["machine"], fleet["machine"],
        "one machine reads as the whole Fleet"
    );
    let html = local.get("/metrics?machine=fleet").text().unwrap();
    assert!(!html.contains("(this machine)"), "{html}");
}

#[test]
fn fleet_metrics_keep_an_offline_peer_for_another_window() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    configure(local_home.path());
    configure(remote_home.path());
    record(remote_home.path(), 73, Some("failed"), 0);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start(binary, remote_home.path());
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    let paired = super::fleet::add(local_home.path(), "remote", &remote.address, &token);
    assert!(paired.status.success(), "{paired:?}");
    api::cli(
        local_home.path(),
        &["metrics", "--fleet", "--window", "7d", "--json"],
    );
    drop(remote);
    let other = api::cli(
        local_home.path(),
        &["metrics", "--fleet", "--window", "30d", "--json"],
    );
    assert!(
        other["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["machine"] == "remote"),
        "offline peer's last-fetched history missing: {other}"
    );
}
