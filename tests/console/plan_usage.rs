//! Plan usage in the Console: `/plan-usage` and `/api/v1/plan-usage` answer
//! as `thirdshift plan-usage` does, across the Fleet once it has one,
//! grouping shared logins; a reachable peer's view is saved for when it goes
//! offline; a machine that cannot be reached inbound pushes its own view;
//! the header warns of a used-up route; and clearing cooldowns is an `act`
//! action (#2182).
use super::*;
use std::io::Write;

/// Not this host's name, which the receiving Console keeps for itself.
const HOLDER: &str = "laptop-vm-fixture";

/// Where the Console fixture puts its fake executables ahead of PATH.
fn fixture_bin(home: &Path) -> std::path::PathBuf {
    home.join("console-fixture-bin")
}

/// Fake passive-only Harness CLIs, so routes to them count as installed on
/// this machine's Console and CLI alike.
fn installed(home: &Path) {
    let bin = fixture_bin(home);
    fs::create_dir_all(&bin).unwrap();
    for name in ["grok", "agy"] {
        support::write_executable(&bin.join(name), "#!/bin/sh\nexit 0\n");
    }
}

/// The CLI with the Console fixture's PATH.
fn thirdshift(home: &Path, args: &[&str]) -> std::process::Output {
    let mut paths = vec![fixture_bin(home)];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(args)
        .env("HOME", home)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .output()
        .unwrap()
}

fn cli(home: &Path, args: &[&str]) -> Value {
    let output = thirdshift(home, args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn parity(console: &Console, home: &Path, path: &str, args: &[&str]) {
    assert_eq!(api::json(console.get(path)), cli(home, args), "{path}");
}

/// `configure`'s User config with `extra` appended.
fn configure_with(home: &Path, extra: &str) {
    configure(home);
    let path = home.join(".thirdshift/config.toml");
    let config = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("{config}{extra}")).unwrap();
}

/// The machine's saved Plan usage, one state per Harness.
fn saved(home: &Path, harnesses: Value) {
    fs::write(
        home.join(".thirdshift/plan-usage.json"),
        json!({"harnesses": harnesses}).to_string(),
    )
    .unwrap();
}

fn rfc3339(hours: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::hours(hours)).to_rfc3339()
}

/// Grok in cooldown on both its routes, agy above the 85% ceiling.
const ROUTES: &str =
    "[harness]\ndefault = \"grok\"\nroutes = [\"grok\", \"agy\"]\nmax_utilization = 85\n";

fn limited(home: &Path) {
    saved(
        home,
        json!({
            "grok": {"observed_at": rfc3339(0), "source": "session",
                "cooldown": {"since": rfc3339(0), "until": rfc3339(1), "backoff_minutes": 60}},
            "agy": {"observed_at": rfc3339(0), "source": "session",
                "windows": [{"name": "weekly", "used_percent": 90.0, "resets_at": rfc3339(24)}]}
        }),
    );
}

fn post_form(
    console: &Console,
    token: &str,
    path: &str,
    form: &[(&str, &str)],
) -> reqwest::blocking::Response {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .post(format!("{}{path}", console.address))
        .bearer_auth(token)
        .form(form)
        .send()
        .unwrap()
}

fn post_json(
    console: &Console,
    token: &str,
    path: &str,
    body: &Value,
) -> reqwest::blocking::Response {
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

fn availability<'a>(view: &'a Value, harness: &str) -> Vec<&'a str> {
    view["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["harness"] == harness)
        .flat_map(|row| row["routes"].as_array().unwrap())
        .map(|route| route["availability"].as_str().unwrap())
        .collect()
}

fn machine<'a>(fleet: &'a Value, name: &str) -> &'a Value {
    fleet["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|machine| machine["name"] == name)
        .unwrap_or_else(|| panic!("{name} missing: {fleet:#}"))
}

/// Each login row's Harness and machines, in order.
fn logins(fleet: &Value) -> Vec<(String, Vec<String>)> {
    fleet["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|login| {
            (
                login["harness"].as_str().unwrap().to_owned(),
                login["machines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|name| name.as_str().unwrap().to_owned())
                    .collect(),
            )
        })
        .collect()
}

fn hostname(console: &Console) -> String {
    api::json(console.get("/api/v1/whoami"))["hostname"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn plan_usage_page_and_api_answer_as_the_cli_and_the_header_warns_of_limited_routes() {
    let home = tempfile::tempdir().unwrap();
    configure_with(home.path(), ROUTES);
    limited(home.path());
    installed(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let host = hostname(&console);

    // Without a paired or saved Fleet machine, the view is this machine's.
    parity(
        &console,
        home.path(),
        "/api/v1/plan-usage",
        &["plan-usage", "--json"],
    );
    let local = cli(home.path(), &["plan-usage", "--json"]);
    assert!(local.get("machines").is_none(), "{local:#}");
    assert_eq!(availability(&local, "grok"), ["cooldown", "cooldown"]);
    assert_eq!(availability(&local, "agy"), ["at_ceiling"]);
    for (path, args) in [
        (
            "/api/v1/plan-usage?local=true",
            vec!["plan-usage", "--local", "--json"],
        ),
        (
            &format!("/api/v1/plan-usage?machine={host}"),
            vec!["plan-usage", "--machine", &host, "--json"],
        ),
    ] {
        parity(&console, home.path(), path, &args);
        assert_eq!(
            api::json(console.get(path))["harnesses"],
            local["harnesses"]
        );
    }
    // The Fleet can be asked for: this machine alone, and the link-only
    // entry that answers nothing.
    let fleet = api::json(console.get("/api/v1/plan-usage?fleet=true"));
    assert_eq!(machine(&fleet, &host)["status"], "ok", "{fleet:#}");
    assert_eq!(machine(&fleet, "other.example.test")["status"], "link-only");
    let refused = console.get("/api/v1/plan-usage?machine=nowhere");
    assert_eq!(refused.status(), 422);
    assert!(refused.text().unwrap().contains("unknown Fleet machine"));
    assert_eq!(console.get("/api/v1/plan-usage?clear=agy").status(), 422);

    for path in ["/", "/plan-usage", "/resources", "/metrics", "/account"] {
        let html = console.get(path).text().unwrap();
        let header = html.split("</header>").next().unwrap();
        assert!(
            header.contains("<p id=\"plan-usage-warning\" role=\"status\"><a href=\"/plan-usage\""),
            "{path}: {header}"
        );
        assert!(
            header.contains("Plan usage · 3 routes limited"),
            "{path}: {header}"
        );
        assert!(
            header.contains("grok harness.default, ceiling 85%: in cooldown"),
            "{path}: {header}"
        );
        assert!(header.contains("agy harness.routes 2, ceiling 85%: at its ceiling"));
        assert!(
            html.contains("<a href=\"/plan-usage\" data-jump=\"p\">Plan usage</a>"),
            "{path}"
        );
    }
    let page = console.get("/plan-usage").text().unwrap();
    assert!(page.contains("thirdshift plan-usage --machine"), "{page}");
    assert!(
        page.contains("harness.routes 1, ceiling 85%: in cooldown"),
        "{page}"
    );
    assert!(page.contains("weekly</dt>"), "{page}");
    assert!(page.contains("90% used"), "{page}");
    assert!(page.contains("action=\"/plan-usage/clear\""), "{page}");
    // `9 Resources` stays the host's own RAM, CPU and disk.
    let resources = console.get("/resources").text().unwrap();
    let main = resources.split("<main>").nth(1).unwrap();
    assert!(!main.contains("Plan usage"), "{main}");

    // Clearing is an act action: read alone is refused.
    let reader = tokens::create(home.path(), "reader", &[]);
    let refused = post_form(
        &console,
        &reader,
        "/api/v1/plan-usage/clear",
        &[("harness", "grok")],
    );
    assert_eq!(refused.status(), 403);
    let actor = tokens::create(home.path(), "actor", &["--scopes", "read,act"]);
    let refused = post_form(
        &console,
        &actor,
        "/api/v1/plan-usage/clear",
        &[("harness", "opencode")],
    );
    assert_eq!(refused.status(), 422);
    let cleared = post_form(
        &console,
        &actor,
        "/api/v1/plan-usage/clear",
        &[("harness", "grok")],
    );
    assert_eq!(
        api::json(cleared),
        json!({"version": 1, "cleared": ["grok"]})
    );
    let after = api::json(console.get("/api/v1/plan-usage"));
    assert_eq!(availability(&after, "grok"), ["available", "available"]);
    let header = console.get("/").text().unwrap();
    assert!(header.contains("Plan usage · 1 route limited"), "{header}");
    let cleared = post_form(&console, &actor, "/api/v1/plan-usage/clear", &[]);
    let everything = api::json(cleared);
    assert_eq!(
        everything["cleared"].as_array().unwrap().len(),
        5,
        "{everything}"
    );
    // The CLI twin answers the same way.
    assert_eq!(
        cli(home.path(), &["plan-usage", "--clear", "agy", "--json"]),
        json!({"version": 1, "cleared": ["agy"]})
    );
    let refused = thirdshift(
        home.path(),
        &["plan-usage", "--clear", "--machine", "remote"],
    );
    assert!(!refused.status.success());
}

#[test]
fn no_header_warning_while_every_configured_route_has_room() {
    let home = tempfile::tempdir().unwrap();
    configure_with(home.path(), ROUTES);
    installed(home.path());
    saved(
        home.path(),
        json!({"agy": {"observed_at": rfc3339(0), "source": "session",
            "windows": [{"name": "weekly", "used_percent": 40.0, "resets_at": rfc3339(24)}]}}),
    );
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    assert!(!html.contains("plan-usage-warning"), "{html}");
}

/// A Claude state signed in to the shared login, observed `hours` ago.
fn claude(hours: i64, used: f64) -> Value {
    json!({"observed_at": rfc3339(-hours), "source": "session",
        "account": {"signed_in": true, "label": "me@example.test", "organization": "org-1"},
        "windows": [{"name": "five_hour", "used_percent": used, "resets_at": rfc3339(3)}]})
}

#[test]
fn fleet_plan_usage_groups_shared_logins_and_keeps_an_offline_peer_cached() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    configure(local_home.path());
    configure(remote_home.path());
    let agy = json!({"observed_at": rfc3339(0), "source": "session",
        "windows": [{"name": "weekly", "used_percent": 12.0}]});
    saved(
        local_home.path(),
        json!({"claude": claude(2, 10.0), "agy": agy}),
    );
    saved(
        remote_home.path(),
        json!({"claude": claude(0, 70.0), "agy": agy}),
    );
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start(binary, remote_home.path());
    let local = Console::start(binary, local_home.path());
    let host = hostname(&local);
    // Only a link-only entry: this machine's view.
    assert!(
        api::json(local.get("/api/v1/plan-usage"))
            .get("machines")
            .is_none()
    );
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    let paired = super::fleet::add(local_home.path(), "remote", &remote.address, &token);
    assert!(paired.status.success(), "{paired:?}");

    // A paired machine makes the Fleet the default, CLI and API alike.
    let fleet = cli(local_home.path(), &["plan-usage", "--json"]);
    assert!(fleet["machine"].is_null(), "{fleet:#}");
    let states: Vec<_> = fleet["machines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["name"].as_str().unwrap(), m["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        states,
        [
            (host.as_str(), "ok"),
            ("other.example.test", "link-only"),
            ("remote", "ok")
        ]
    );
    let expected = [
        ("claude".to_owned(), vec![host.clone(), "remote".to_owned()]),
        ("agy".to_owned(), vec![host.clone()]),
        ("agy".to_owned(), vec!["remote".to_owned()]),
    ];
    assert_eq!(logins(&fleet), expected, "{fleet:#}");
    let shared = &fleet["harnesses"][0];
    assert_eq!(shared["login"], "organization org-1 · me@example.test");
    assert_eq!(shared["observed_on"], "remote");
    assert_eq!(shared["windows"][0]["used_percent"], 70.0);
    let routed: Vec<_> = shared["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|route| route["machine"].as_str().unwrap())
        .collect();
    assert_eq!(routed, [host.as_str(), "remote"], "{shared:#}");
    let api = api::json(local.get("/api/v1/plan-usage"));
    assert_eq!(logins(&api), expected, "{api:#}");
    assert_eq!(
        api["harnesses"],
        api::json(local.get("/api/v1/plan-usage?fleet=true"))["harnesses"]
    );

    let html = local.get("/plan-usage").text().unwrap();
    assert!(
        html.contains("thirdshift plan-usage --fleet --json"),
        "{html}"
    );
    assert!(
        html.contains(&format!("claude · shared by {host}, remote")),
        "{html}"
    );
    assert!(
        html.contains("organization org-1 · me@example.test"),
        "{html}"
    );
    assert!(html.contains("<option value=\"remote\""), "{html}");
    let text = thirdshift(local_home.path(), &["plan-usage"]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains(&format!("claude, shared by {host}, remote")),
        "{text}"
    );
    assert!(text.contains("freshest on remote"), "{text}");

    let one = cli(
        local_home.path(),
        &["plan-usage", "--machine", "remote", "--json"],
    );
    assert_eq!(one["machine"], "remote");
    assert_eq!(
        logins(&one),
        [
            ("claude".to_owned(), vec!["remote".to_owned()]),
            ("agy".to_owned(), vec!["remote".to_owned()])
        ]
    );
    assert_eq!(
        api::json(local.get("/api/v1/plan-usage?machine=remote"))["harnesses"],
        one["harnesses"]
    );

    // Offline, the peer's last view stands in, marked with when it was seen.
    drop(remote);
    let fleet = cli(local_home.path(), &["plan-usage", "--json"]);
    let offline = machine(&fleet, "remote");
    assert_eq!(offline["status"], "cached", "{fleet:#}");
    assert!(chrono::DateTime::parse_from_rfc3339(offline["last_seen"].as_str().unwrap()).is_ok());
    assert!(offline["error"].as_str().is_some(), "{fleet:#}");
    assert_eq!(logins(&fleet), expected, "{fleet:#}");
    assert_eq!(fleet["harnesses"][0]["observed_on"], "remote");
    let html = local.get("/plan-usage").text().unwrap();
    assert!(
        html.contains("cached, offline</span> · last seen <time"),
        "{html}"
    );
    let unknown = thirdshift(local_home.path(), &["plan-usage", "--machine", "nowhere"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown Fleet machine"));
}

/// `console report add` on `home`, the token on stdin.
fn pair_report(home: &Path, address: &str, token: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["console", "report", "add", "hub", address])
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = writeln!(child.stdin.take().unwrap(), "{token}");
    child.wait_with_output().unwrap()
}

#[test]
fn a_pushed_plan_usage_view_shows_the_worker_read_only_on_the_hub() {
    let hub_home = tempfile::tempdir().unwrap();
    let worker_home = tempfile::tempdir().unwrap();
    configure(hub_home.path());
    configure_with(worker_home.path(), ROUTES);
    limited(worker_home.path());
    installed(worker_home.path());
    let hub = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), hub_home.path());
    let token = tokens::create(hub_home.path(), HOLDER, &["--scopes", "report"]);

    // The token pushes only its holder's own view, and a readable one.
    let view = json!({"version": 1, "harnesses": []});
    let other = post_json(
        &hub,
        &token,
        "/api/v1/plan-usage/report",
        &json!({"version": 1, "machine": "elsewhere", "view": view}),
    );
    assert_eq!(other.status(), 403);
    assert!(
        other
            .text()
            .unwrap()
            .contains("only its holder's own Plan usage")
    );
    let forged = post_json(
        &hub,
        &token,
        "/api/v1/plan-usage/report",
        &json!({"version": 1, "machine": HOLDER, "view": {"version": 1, "harnesses": [
            {"harness": "grok", "installed": true, "active": false, "routes": [],
             "source": "\u{1b}[2Jforged"}]}}),
    );
    assert_eq!(forged.status(), 422);
    let reader = tokens::create(hub_home.path(), "reader", &[]);
    let refused = post_json(
        &hub,
        &reader,
        "/api/v1/plan-usage/report",
        &json!({"version": 1, "machine": "reader", "view": view}),
    );
    assert_eq!(refused.status(), 403);
    assert!(
        api::json(hub.get("/api/v1/plan-usage"))
            .get("machines")
            .is_none()
    );

    let paired = pair_report(worker_home.path(), &hub.address, &token);
    assert!(paired.status.success(), "{paired:?}");
    let pushed = thirdshift(worker_home.path(), &["plan-usage", "--push"]);
    assert!(pushed.status.success(), "{pushed:?}");
    assert_eq!(
        String::from_utf8_lossy(&pushed.stdout).trim(),
        "Reported to hub"
    );
    let unknown = thirdshift(worker_home.path(), &["plan-usage", "--push", "elsewhere"]);
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown report destination"));

    // A reported machine makes the Fleet the hub's default view.
    let fleet = cli(hub_home.path(), &["plan-usage", "--json"]);
    let reported = machine(&fleet, HOLDER);
    assert_eq!(reported["status"], "reported", "{fleet:#}");
    assert!(reported["address"].is_null());
    assert!(chrono::DateTime::parse_from_rfc3339(reported["pushed_at"].as_str().unwrap()).is_ok());
    let grok = fleet["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|login| login["harness"] == "grok")
        .unwrap_or_else(|| panic!("{fleet:#}"));
    assert_eq!(grok["machines"], json!([HOLDER]));
    assert_eq!(grok["routes"][0]["availability"], "cooldown");
    let html = hub.get("/plan-usage").text().unwrap();
    assert!(
        html.contains("reported, read-only</span> · pushed <time"),
        "{html}"
    );
    assert!(html.contains("(just now)"), "{html}");
    // The hub's header warns only of its own routes.
    let header = html.split("</header>").next().unwrap();
    assert!(!header.contains("plan-usage-warning"), "{header}");
}

#[test]
fn a_finished_command_pushes_its_machines_plan_usage() {
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
    scenario.agent_does_for(
        7,
        "echo '{\"type\": \"rate_limit_event\", \"rate_limit_info\": {\"status\": \"allowed\", \"rateLimitType\": \"five_hour\", \"utilization\": 0.42, \"resetsAt\": 4000000000}}'\n\
         echo 7 > issue-7.txt\ngit add issue-7.txt\ngit commit -q -m 'Work on 7'\n\
         gh pr create --base main --head issue-7 --title 'Work on 7' --body 'Closes #7'\n",
    );
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(!result.stderr.contains("not reported"), "{}", result.stderr);
    let fleet = cli(hub_home.path(), &["plan-usage", "--json"]);
    assert_eq!(machine(&fleet, HOLDER)["status"], "reported", "{fleet:#}");
    let claude = fleet["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|login| login["harness"] == "claude" && login["machines"] == json!([HOLDER]))
        .unwrap_or_else(|| panic!("{fleet:#}"));
    assert_eq!(claude["windows"][0]["used_percent"], 42.0, "{claude:#}");
}
