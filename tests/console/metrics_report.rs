//! Outbound Fleet reporting: a machine that cannot be reached inbound pushes
//! its own Run metrics summary to a Console under a holder-bound `report`
//! token, by hand with `metrics --push` and after each Command (#2069).
use super::*;
use std::io::Write;
use std::process::Output;

/// Not this host's name, which the receiving Console keeps for itself.
const HOLDER: &str = "laptop-vm-fixture";

fn thirdshift(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

/// `console report add` on `home`, the token on stdin.
fn pair(home: &Path, name: &str, address: &str, token: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["console", "report", "add", name, address])
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = writeln!(child.stdin.take().unwrap(), "{token}");
    child.wait_with_output().unwrap()
}

fn post(console: &Console, token: &str, path: &str, body: &Value) -> reqwest::blocking::Response {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .post(format!("{}{path}", console.address))
        .bearer_auth(token)
        .json(body)
        .send()
        .unwrap()
}

fn report(machine: &str) -> Value {
    json!({
        "version": 1,
        "machine": machine,
        "view": "window=7d",
        "summary": {"outcomes": [], "comparison": null, "where_runs_failed": {"rows": []}}
    })
}

fn machine<'a>(fleet: &'a Value, name: &str) -> &'a Value {
    fleet["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|machine| machine["name"] == name)
        .unwrap_or_else(|| panic!("{name} missing: {fleet}"))
}

#[test]
fn a_report_token_writes_only_its_holders_snapshot_and_reaches_nothing_else() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let refused = thirdshift(
        home.path(),
        &[
            "console",
            "token",
            "create",
            HOLDER,
            "--scopes",
            "report,read",
        ],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("cannot be combined"));
    let refused = thirdshift(
        home.path(),
        &["console", "token", "create", "-x", "--scopes", "report"],
    );
    assert!(!refused.status.success());
    let token = tokens::create(home.path(), HOLDER, &["--scopes", "report"]);
    assert_eq!(
        api::cli(home.path(), &["console", "token", "list", "--json"])[0]["scopes"],
        json!(["report"])
    );
    // A person's scopes never include report.
    let person = thirdshift(
        home.path(),
        &[
            "console",
            "scopes",
            "--email",
            "a@example.test",
            "--scopes",
            "report",
        ],
    );
    assert!(!person.status.success(), "{person:?}");

    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let saved = post(&console, &token, "/api/v1/metrics/report", &report(HOLDER));
    assert_eq!(saved.status(), 200);
    assert_eq!(saved.json::<Value>().unwrap()["machine"], HOLDER);
    let other = post(
        &console,
        &token,
        "/api/v1/metrics/report",
        &report("other-machine"),
    );
    assert_eq!(other.status(), 403);
    assert!(
        other
            .text()
            .unwrap()
            .contains("only its holder's own metrics")
    );
    let unknown = post(
        &console,
        &token,
        "/api/v1/metrics/report",
        &json!({"version": 1, "machine": HOLDER, "view": "machine=x", "summary": report(HOLDER)["summary"]}),
    );
    assert_eq!(unknown.status(), 422);
    let fleet = api::cli(home.path(), &["metrics", "--fleet", "--json"]);
    let names: Vec<_> = fleet["machines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|machine| machine["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&HOLDER), "{fleet}");
    assert!(!names.contains(&"other-machine"), "{fleet}");

    // Every other operation needs a viewer scope the token lacks.
    let description: Value =
        serde_json::from_slice(&thirdshift(home.path(), &["console", "openapi"]).stdout).unwrap();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let mut checked = 0;
    for (path, methods) in description["paths"].as_object().unwrap() {
        for (method, operation) in methods.as_object().unwrap() {
            let scope = operation["x-scope"].as_str().unwrap();
            if matches!(scope, "report" | "authenticated") {
                continue;
            }
            let uri = path
                .split('/')
                .map(|segment| {
                    if segment.starts_with('{') {
                        "x"
                    } else {
                        segment
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            let request = match method.as_str() {
                "get" => client.get(format!("{}{uri}", console.address)),
                "post" => client.post(format!("{}{uri}", console.address)),
                other => panic!("unexpected method {other}"),
            };
            let response = request.bearer_auth(&token).send().unwrap();
            assert_eq!(response.status(), 403, "{method} {path} ({scope})");
            checked += 1;
        }
    }
    assert!(checked > 40, "only {checked} operations checked");
    let whoami = api::json(
        tokens::request(&console, &token, "/api/v1/whoami")
            .send()
            .unwrap(),
    );
    assert_eq!(whoami["scopes"], json!(["report"]));
    assert_eq!(whoami["identity"]["holder"], HOLDER);
}

#[test]
fn metrics_push_shows_the_worker_read_only_on_the_receiving_fleet() {
    let hub_home = tempfile::tempdir().unwrap();
    let worker_home = tempfile::tempdir().unwrap();
    configure(hub_home.path());
    configure(worker_home.path());
    let mut failed = record(worker_home.path(), 73, Some("failed"), 0);
    failed["name"] = json!("widgets#73");
    write_record(worker_home.path(), 73, &failed);
    let none = thirdshift(worker_home.path(), &["metrics", "--push"]);
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("no report destinations"));

    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let hub = Console::start(binary, hub_home.path());
    // A viewer token cannot be paired for reporting.
    let viewer = tokens::create(hub_home.path(), "viewer", &[]);
    let refused = pair(worker_home.path(), "hub", &hub.address, &viewer);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("report scope alone"));
    let token = tokens::create(hub_home.path(), HOLDER, &["--scopes", "report"]);
    let paired = pair(worker_home.path(), "hub", &hub.address, &token);
    assert!(paired.status.success(), "{paired:?}");
    assert_eq!(
        String::from_utf8_lossy(&paired.stdout).trim(),
        format!("Reporting to hub ({}) as {HOLDER}", hub.address)
    );
    let config = fs::read_to_string(worker_home.path().join(".thirdshift/config.toml")).unwrap();
    assert!(!config.contains(&token), "{config}");
    let credentials =
        fs::read_to_string(worker_home.path().join(".thirdshift/credentials.toml")).unwrap();
    assert!(credentials.contains("[report]"), "{credentials}");
    let listed = api::cli(worker_home.path(), &["console", "report", "list", "--json"]);
    assert_eq!(listed[0]["holder"], HOLDER);
    assert_eq!(listed[0]["token"], true);

    let pushed = thirdshift(worker_home.path(), &["metrics", "--push", "--window", "7d"]);
    assert!(pushed.status.success(), "{pushed:?}");
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout).trim(),
        "Reported to hub"
    );

    let fleet = api::cli(
        hub_home.path(),
        &["metrics", "--fleet", "--window", "7d", "--json"],
    );
    let reported = machine(&fleet, HOLDER);
    assert_eq!(reported["status"], "reported", "{fleet}");
    assert!(reported["address"].is_null());
    assert!(chrono::DateTime::parse_from_rfc3339(reported["pushed_at"].as_str().unwrap()).is_ok());
    assert_eq!(
        fleet["where_runs_failed"]["rows"][0]["machines"][HOLDER], 1,
        "{fleet}"
    );
    let one = api::cli(
        hub_home.path(),
        &["metrics", "--machine", HOLDER, "--window", "7d", "--json"],
    );
    assert_eq!(one["machine"], HOLDER);
    assert_eq!(one["where_runs_failed"]["failed"], 1, "{one}");

    // The page shows it read-only with its age, its failures without links.
    for route in [
        "/metrics?window=7d",
        &format!("/metrics?machine={HOLDER}&window=7d"),
    ] {
        let html = hub.get(route).text().unwrap();
        assert!(
            html.contains("reported, read-only</span> · pushed <time"),
            "{html}"
        );
        assert!(html.contains("(just now)"), "{html}");
        assert!(
            html.contains(&format!("widgets#73 (reported) on {HOLDER}")),
            "{html}"
        );
        assert!(
            !html.contains(&format!("machine={HOLDER}\">widgets#73")),
            "{html}"
        );
        assert!(
            html.contains(&format!("<option value=\"{HOLDER}\"")),
            "{html}"
        );
    }

    // A revoked token is refused, and the push says so.
    assert!(
        thirdshift(hub_home.path(), &["console", "token", "revoke", HOLDER])
            .status
            .success()
    );
    let refused = thirdshift(worker_home.path(), &["metrics", "--push", "hub"]);
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("refused the report (HTTP 401"),
        "{refused:?}"
    );
    let unknown = thirdshift(worker_home.path(), &["metrics", "--push", "elsewhere"]);
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown report destination"));
    let removed = thirdshift(worker_home.path(), &["console", "report", "remove", "hub"]);
    assert!(removed.status.success(), "{removed:?}");
    let credentials =
        fs::read_to_string(worker_home.path().join(".thirdshift/credentials.toml")).unwrap();
    assert!(!credentials.contains(&token));
}

#[test]
fn a_finished_command_reports_its_metrics_and_an_offline_console_only_warns() {
    let hub_home = tempfile::tempdir().unwrap();
    configure(hub_home.path());
    let hub = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), hub_home.path());
    let token = tokens::create(hub_home.path(), HOLDER, &["--scopes", "report"]);
    let scenario = support::Scenario::new();
    scenario.user_config_is(&format!(
        "[console]\nreport = [{{ name = \"hub\", address = \"{}\", holder = \"{HOLDER}\" }}]\n",
        hub.address
    ));
    scenario.credentials_are(&format!("[report]\nhub = \"{token}\"\n"));
    let opens_pr = |issue: u32| {
        format!(
            "echo {issue} > issue-{issue}.txt\ngit add issue-{issue}.txt\n\
             git commit -q -m 'Work on {issue}'\n\
             gh pr create --base main --head issue-{issue} --title 'Work on {issue}' --body 'Closes #{issue}'\n"
        )
    };
    scenario.agent_does_for(7, &opens_pr(7));
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(!result.stderr.contains("not reported"), "{}", result.stderr);
    let fleet = api::cli(hub_home.path(), &["metrics", "--fleet", "--json"]);
    let reported = machine(&fleet, HOLDER);
    assert_eq!(reported["status"], "reported", "{fleet}");
    assert!(
        fleet["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|bucket| bucket["machine"] == HOLDER && bucket["runs"] == 1),
        "{fleet}"
    );

    // An offline Console costs the Command nothing but a warning.
    drop(hub);
    scenario.issue_is(8, "OPEN");
    scenario.agent_does_for(8, &opens_pr(8));
    let result = scenario.run(&[&scenario.issue_url(8)]);
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result
            .stderr
            .contains("warning: metrics not reported to hub:"),
        "{}",
        result.stderr
    );
}

#[test]
fn a_report_from_this_hosts_own_name_is_refused_or_shown() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let probe = tokens::create(home.path(), "probe", &[]);
    let host = api::json(
        tokens::request(&console, &probe, "/api/v1/whoami")
            .send()
            .unwrap(),
    )["hostname"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = tokens::create(home.path(), &host, &["--scopes", "report"]);
    let saved = post(&console, &token, "/api/v1/metrics/report", &report(&host));
    if saved.status() == 200 {
        let fleet = api::cli(home.path(), &["metrics", "--fleet", "--json"]);
        assert!(
            fleet["machines"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["name"] == host.as_str() && m["status"] == "reported"),
            "report accepted with 200 but never shown: {fleet}"
        );
    } else {
        assert_eq!(saved.status(), 422);
    }
}
