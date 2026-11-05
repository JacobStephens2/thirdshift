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

#[test]
fn read_api_equals_cli_for_command_trees_stages_waterfall_and_logs() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    // Both processes observe the same wall clock, including the Waterfall's
    // `now`. Deadlines and waits still use the real monotonic clock.
    let shim = home.path().join("api-clock.c");
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
            .arg(home.path().join("api-clock.so"))
            .status()
            .unwrap()
            .success()
    );
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
    for path in [
        "/api/v1/stop?command=widgets%237",
        "/api/v1/dismiss?command=widgets%237",
        "/api/v1/sign-in",
        "/api/v1/absent",
        "/api/v1/events",
    ] {
        let response = console.get(path);
        assert_eq!(response.status(), 404, "{path}");
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
