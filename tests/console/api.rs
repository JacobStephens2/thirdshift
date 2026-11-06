use super::*;

pub(super) fn cli(home: &Path, args: &[&str]) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command.args(args).env("HOME", home);
    if home.join("api-clock.so").exists() {
        command.env("LD_PRELOAD", home.join("api-clock.so"));
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

pub(super) fn json(response: reqwest::blocking::Response) -> Value {
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/json");
    response.json().unwrap()
}

pub(super) fn parity(console: &Console, home: &Path, path: &str, args: &[&str]) {
    let expected = cli(home, args);
    assert_eq!(json(console.get(path)), expected, "{path}");
}

#[test]
fn theme_import_is_available_through_the_versioned_api() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let token = tokens::create(home.path(), "theme-importer", &["--scopes", "act"]);
    let reader = tokens::create(home.path(), "reader", &[]);
    let mut theme = cli(home.path(), &["console", "theme", "export", "focus-auto"]);
    theme["name"] = json!("api-custom");
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = reqwest::blocking::Client::new()
        .post(format!("{}/api/v1/themes", console.address))
        .bearer_auth(&token)
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&theme).unwrap())
        .send()
        .unwrap();
    assert_eq!(json(response), theme);
    assert_eq!(
        cli(home.path(), &["console", "theme", "export", "api-custom"]),
        theme
    );
    let client = reqwest::blocking::Client::new();
    let refused = client
        .post(format!("{}/api/v1/themes", console.address))
        .bearer_auth(reader)
        .json(&theme)
        .send()
        .unwrap();
    assert_eq!(refused.status(), 403);
    assert!(
        refused.json::<Value>().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("act")
    );
    let invalid = client
        .post(format!("{}/api/v1/themes", console.address))
        .bearer_auth(token)
        .json(&json!({"name": "../invalid"}))
        .send()
        .unwrap();
    assert_eq!(invalid.status(), 422);
    assert!(invalid.json::<Value>().unwrap()["error"].is_string());
    let events = cli(home.path(), &["console", "log", "--json"]);
    let calls: Vec<_> = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["event"] == "api-call")
        .collect();
    assert_eq!(calls.len(), 3);
    for (call, (holder, outcome)) in calls.iter().zip([
        ("theme-importer", 200),
        ("reader", 403),
        ("theme-importer", 422),
    ]) {
        assert_eq!(call["email"], holder);
        assert_eq!(call["endpoint"], "/api/v1/themes");
        assert_eq!(call["outcome"], outcome);
    }
}

#[test]
fn existing_look_read_has_its_declared_api_route() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    cli(home.path(), &["console", "theme", "press-dark", "--json"]);
    let expected = cli(
        home.path(),
        &["console", "sections", "--hide", "done", "--json"],
    );
    let token = tokens::create(home.path(), "reader", &[]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let before = cli(home.path(), &["console", "log", "--json"]);
    assert_eq!(console.get("/look").status(), 200);
    assert_eq!(console.get("/api/v1/look").status(), 200);
    assert_eq!(
        json(
            tokens::request(&console, &token, "/api/v1/look")
                .send()
                .unwrap()
        ),
        expected
    );
    assert_eq!(cli(home.path(), &["console", "log", "--json"]), before);
}

#[test]
fn read_token_can_list_available_themes_without_changing_the_console() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let token = tokens::create(home.path(), "reader", &[]);
    let expected = cli(home.path(), &["console", "theme", "list", "--json"]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let before = cli(home.path(), &["console", "log", "--json"]);
    assert_eq!(
        json(
            tokens::request(&console, &token, "/api/v1/themes")
                .send()
                .unwrap()
        ),
        expected
    );
    assert_eq!(cli(home.path(), &["console", "log", "--json"]), before);
}

#[test]
fn read_token_can_read_an_ordinary_log_without_a_command_record() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let folder = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("7-20261003T120000-0400.log"),
        "ordinary retained output\n",
    )
    .unwrap();
    let token = tokens::create(home.path(), "reader", &[]);
    let listed = cli(home.path(), &["logs", "--json"]);
    let id = listed["logs"][0]["id"].as_str().unwrap();
    let expected = cli(home.path(), &["logs", "--file", id, "--json"]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    assert_eq!(
        json(
            tokens::request(&console, &token, &format!("/api/v1/logs/{id}"))
                .send()
                .unwrap()
        ),
        expected
    );
}

#[test]
fn openapi_equals_cli_and_describes_operations_without_local_secrets() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let secret = "synthetic-credential-that-must-not-appear-in-openapi";
    fs::write(
        home.path().join(".thirdshift/credentials.toml"),
        format!("[resend]\nkey = {secret:?}\n"),
    )
    .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let document = json(console.get("/api/v1/openapi.json"));
    assert_eq!(document, cli(home.path(), &["console", "openapi"]));
    assert_eq!(document["openapi"], "3.1.1");
    let paths = document["paths"].as_object().unwrap();
    for path in [
        "/api/v1/status",
        "/api/v1/peek",
        "/api/v1/whoami",
        "/api/v1/openapi.json",
        "/api/v1/look",
        "/api/v1/themes",
    ] {
        assert!(paths.contains_key(path), "missing {path}");
    }
    for (path, methods) in paths {
        assert!(path.starts_with("/api/v1/"));
        for (_, operation) in methods.as_object().unwrap() {
            assert!(
                ["read", "security", "act", "run", "admin"]
                    .contains(&operation["x-scope"].as_str().unwrap())
            );
            for twin in operation["x-cli-twins"].as_array().unwrap() {
                assert!(twin.as_str().unwrap().starts_with("thirdshift "));
            }
            assert!(
                operation["responses"]["200"]["content"]["application/json"]["schema"].is_object()
            );
            assert!(operation["responses"]["403"].is_object());
            let limited = &operation["responses"]["429"];
            assert_eq!(
                limited["headers"]["Retry-After"]["schema"]["type"],
                "integer"
            );
            assert_eq!(
                limited["content"]["application/json"]["schema"]["properties"]["retry_after"]["type"],
                "integer"
            );
        }
    }
    let whoami = &paths["/api/v1/whoami"]["get"];
    assert_eq!(
        whoami["x-cli-twins"],
        json!(["thirdshift console whoami --json"])
    );
    let properties =
        &whoami["responses"]["200"]["content"]["application/json"]["schema"]["properties"];
    for field in [
        "hostname",
        "thirdshift_version",
        "api_version",
        "identity",
        "scopes",
    ] {
        assert!(
            properties[field].is_object(),
            "missing whoami field {field}"
        );
    }
    assert_eq!(
        document["security"],
        json!([{"ConsoleSession": []}, {"ConsoleBearer": []}])
    );
    assert_eq!(
        document["components"]["securitySchemes"]["ConsoleBearer"],
        json!({"type": "http", "scheme": "bearer"})
    );
    assert!(!paths.contains_key("/api/v1/events"));
    assert!(!paths.contains_key("/api/v1/set-password"));
    assert!(
        !serde_json::to_string(&document)
            .unwrap()
            .contains(&home.path().display().to_string())
    );
    assert!(!serde_json::to_string(&document).unwrap().contains(secret));
}

#[test]
fn inbox_api_equals_status_json_without_sign_in() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut command = record(home.path(), 7, Some("failed"), 0);
    // No GitHub subject: the test compares one local captured view, without
    // racing the Console's independent background GitHub refresh.
    command["issue"] = Value::Null;
    write_record(home.path(), 7, &command);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    parity(
        &console,
        home.path(),
        "/api/v1/status",
        &["status", "--json"],
    );
}

pub(super) fn freeze_clock(home: &Path) {
    // Both processes observe the same wall clock, including the Waterfall's
    // `now`. Deadlines and waits still use the real monotonic clock.
    let shim = home.join("api-clock.c");
    fs::write(
        &shim,
        format!(
            r#"
#define _GNU_SOURCE
#include <time.h>
#include <dlfcn.h>
int clock_gettime(clockid_t id, struct timespec *value) {{
    if (id == CLOCK_REALTIME) {{ value->tv_sec = {}; value->tv_nsec = 0; return 0; }}
    int (*original)(clockid_t, struct timespec *) = dlsym(RTLD_NEXT, "clock_gettime");
    return original(id, value);
}}
"#,
            chrono::Utc::now().timestamp() + 600
        ),
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .args(["-shared", "-fPIC"])
            .arg(shim)
            .args(["-o"])
            .arg(home.join("api-clock.so"))
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn read_api_equals_cli_for_command_trees_stages_waterfall_and_logs() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    freeze_clock(home.path());
    let mut parent = record(home.path(), 7, Some("merged"), 0);
    let mut child = record(home.path(), 8, Some("merged"), 0);
    let directory = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    child["parent_record"] = json!(directory.join("7-fixture.command.json"));
    child["kind"] = json!("Run");
    write_record(home.path(), 8, &child);
    let session = directory.join("7-20261003T120000-0400-implement.logs.jsonl");
    fs::write(&session, include_str!("../fixtures/console-codex.jsonl")).unwrap();
    fs::write(directory.join("7-fixture.log"), "Command output <unsafe>\n").unwrap();
    parent["session_log"] = json!(session);
    parent["stages"][0]["session_log"] = parent["session_log"].clone();
    write_record(home.path(), 7, &parent);
    let before = fs::read(directory.join("7-fixture.command.json")).unwrap();
    let console = Console::start_command({
        let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
        command
            .args(["console", "--no-sign-in"])
            .env("HOME", home.path())
            .env("LD_PRELOAD", home.path().join("api-clock.so"));
        command
    });
    let details = cli(home.path(), &["show", "widgets#7", "--json"]);
    assert_eq!(details["command"]["children"][0]["issue"]["number"], 8);
    assert_eq!(
        json(console.get("/api/v1/peek?command=widgets%237")),
        details
    );
    for path in [
        "/api/v1/status?all=true",
        "/api/v1/history",
        "/api/v1/lanes",
        "/api/v1/stages",
        "/api/v1/security",
    ] {
        parity(&console, home.path(), path, &["status", "--all", "--json"]);
    }
    for path in [
        "/api/v1/waterfall?command=widgets%237",
        "/api/v1/spans?command=widgets%237",
    ] {
        parity(
            &console,
            home.path(),
            path,
            &["show", "widgets#7", "--spans", "--json"],
        );
    }
    parity(
        &console,
        home.path(),
        "/api/v1/log?command=widgets%237",
        &["logs", "widgets#7", "--json"],
    );
    parity(
        &console,
        home.path(),
        "/api/v1/log?command=widgets%237&session=implement",
        &["logs", "widgets#7", "--session", "implement", "--json"],
    );
    let inventory = cli(home.path(), &["logs", "--json"]);
    assert_eq!(json(console.get("/api/v1/logs")), inventory);
    for log in inventory["logs"].as_array().unwrap() {
        let id = log["id"].as_str().unwrap();
        parity(
            &console,
            home.path(),
            &format!("/api/v1/logs/{id}"),
            &["logs", "--file", id, "--json"],
        );
        let owner = log["owners"][0].as_str().unwrap();
        parity(
            &console,
            home.path(),
            &format!("/api/v1/logs/command/{owner}"),
            &["logs", "--command", owner, "--json"],
        );
    }
    for (path, args) in [
        ("/api/v1/metrics", vec!["metrics", "--json"]),
        (
            "/api/v1/metrics?command=widgets%238",
            vec!["metrics", "widgets#8", "--json"],
        ),
        ("/api/v1/console-log", vec!["console", "log", "--json"]),
        (
            "/api/v1/themes/focus-auto",
            vec!["console", "theme", "export", "focus-auto"],
        ),
    ] {
        parity(&console, home.path(), path, &args);
    }
    assert_eq!(
        fs::read(directory.join("7-fixture.command.json")).unwrap(),
        before
    );
    assert!(!directory.join("7-fixture.command.stop.json").exists());
    assert!(!directory.join("7-fixture.command.dismiss.json").exists());
}

#[test]
fn api_path_extractor_refusals_are_json() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for path in [
        "/api/v1/logs/%FF",
        "/api/v1/logs/command/%FF",
        "/api/v1/experiments/%FF",
        "/api/v1/experiments/id/sessions/%FF",
        "/api/v1/themes/%FF",
    ] {
        let response = console.get(path);
        assert_eq!(response.status(), 400, "{path}");
        assert_eq!(
            response.headers()["content-type"],
            "application/json",
            "{path}"
        );
        assert!(
            response.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("Invalid UTF-8")
        );
    }
    let response = console.get("/logs/%FF");
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
}

#[test]
fn api_errors_are_json_and_reads_cannot_invoke_actions() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 7, Some("merged"), 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let refused = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["show", "missing#404", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(!refused.status.success());
    let response = console.get("/api/v1/peek?command=missing%23404");
    assert_eq!(response.status(), 422);
    let error: Value = response.json().unwrap();
    assert!(String::from_utf8_lossy(&refused.stderr).contains(error["error"].as_str().unwrap()));
    for path in ["/api/v1/sign-in", "/api/v1/absent", "/api/v1/events"] {
        let response = console.get(path);
        assert_eq!(response.status(), 404, "{path}");
        assert!(response.json::<Value>().unwrap()["error"].is_string());
    }
    for path in [
        "/api/v1/stop?command=widgets%237",
        "/api/v1/dismiss?command=widgets%237",
    ] {
        let response = console.get(path);
        assert_eq!(response.status(), 405);
        assert!(response.json::<Value>().unwrap()["error"].is_string());
    }
    let response = reqwest::blocking::Client::new()
        .post(format!("{}/api/v1/status", console.address))
        .send()
        .unwrap();
    assert_eq!(response.status(), 405);
    assert!(response.json::<Value>().unwrap()["error"].is_string());
    let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["logs", "widgets#7", "--follow", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--json cannot be combined with --follow")
    );
}
