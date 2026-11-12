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
