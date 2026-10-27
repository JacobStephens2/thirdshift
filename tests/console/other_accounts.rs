use super::*;

#[cfg(target_os = "linux")]
struct Machine {
    root: tempfile::TempDir,
}

#[cfg(target_os = "linux")]
impl Machine {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        for name in ["serving", "conductor", "unreadable"] {
            configure(&root.path().join(name));
        }
        let source = root.path().join("passwd.c");
        fs::write(&source, r#"
#include <pwd.h>
#include <stdio.h>
#include <stdlib.h>
static FILE *accounts;
void setpwent(void) { if (accounts) fclose(accounts); accounts = fopen(getenv("FIXTURE_PASSWD"), "r"); }
struct passwd *getpwent(void) { return accounts ? fgetpwent(accounts) : NULL; }
void endpwent(void) { if (accounts) fclose(accounts); accounts = NULL; }
"#).unwrap();
        assert!(
            Command::new("cc")
                .args(["-shared", "-fPIC"])
                .arg(&source)
                .arg("-o")
                .arg(root.path().join("passwd.so"))
                .status()
                .unwrap()
                .success()
        );
        let uid = unsafe { libc::geteuid() };
        fs::write(root.path().join("passwd"), format!("serving:x:{uid}:1::{}:/bin/sh\nconductor:x:{}:1::{}:/bin/sh\nunreadable:x:{}:1::{}:/bin/sh\n", root.path().join("serving").display(), uid+1, root.path().join("conductor").display(), uid+2, root.path().join("unreadable").display())).unwrap();
        record(&root.path().join("unreadable"), 99, Some("failed"), 0);
        fs::set_permissions(
            root.path().join("unreadable"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(bin.join("gh"), "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755)).unwrap();
        Self { root }
    }

    fn home(&self, name: &str) -> std::path::PathBuf {
        self.root.path().join(name)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
        command
            .env("HOME", self.home("serving"))
            .env("LD_PRELOAD", self.root.path().join("passwd.so"))
            .env("FIXTURE_PASSWD", self.root.path().join("passwd"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            );
        command
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        self.command().args(args).output().unwrap()
    }
    fn setting(&self, value: &str) {
        assert!(
            self.run(&["console", "other-accounts", value])
                .status
                .success()
        );
    }
}

#[cfg(target_os = "linux")]
impl Drop for Machine {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(self.home("unreadable"), fs::Permissions::from_mode(0o700)).unwrap();
    }
}

#[cfg(target_os = "linux")]
fn retained_files(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(retained_files(&path));
        } else {
            files.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    files
}

#[cfg(target_os = "linux")]
#[test]
fn foreign_process_names_and_accounts_survive_across_console_views() {
    let machine = Machine::new();
    let home = machine.home("conductor");
    let mut process = record(&home, 7, Some("failed"), 0);
    process["kind"] = json!("Process");
    process["process_name"] = json!("notes");
    process["name"] = json!("process-fixture");
    process["issue"] = Value::Null;
    write_record(&home, 7, &process);
    machine.setting("readable");
    let console = Console::start_command({
        let mut command = machine.command();
        command.args(["console", "--no-sign-in"]);
        command
    });
    for page in ["inbox", "lanes", "stages", "waterfall"] {
        let html = console.get(&format!("/?page={page}")).text().unwrap();
        assert!(
            html.contains("Process notes on acme/widgets") && html.contains("conductor"),
            "{page}: {html}"
        );
    }
    let spans: Value = console.get("/spans").json().unwrap();
    assert!(
        spans["spans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|span| span["name"]
                .as_str()
                .is_some_and(|name| name.contains("Process notes on acme/widgets")
                    && name.contains("conductor"))),
        "{spans}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn unreadable_foreign_session_traces_leave_waterfall_available() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::new();
    let home = machine.home("conductor");
    let mut command = record(&home, 7, Some("failed"), 0);
    let dir = home.join(".thirdshift/logs/acme/widgets/commands/issue");
    let trace = dir.join("7-fixture-implement.logs.jsonl");
    fs::write(&trace, "").unwrap();
    fs::set_permissions(&trace, fs::Permissions::from_mode(0o000)).unwrap();
    command["session_log"] = json!(trace);
    command["stages"][0]["session_log"] = command["session_log"].clone();
    write_record(&home, 7, &command);
    fs::write(dir.join("7-fixture.log"), "readable output").unwrap();
    machine.setting("readable");
    let console = Console::start_command({
        let mut command = machine.command();
        command.args(["console", "--no-sign-in"]);
        command
    });
    for route in ["/", "/?page=waterfall", "/spans", "/logs"] {
        let response = console.get(route);
        assert_eq!(
            response.status(),
            200,
            "{route}: {}",
            response.text().unwrap()
        );
    }
    let spans: Value = console.get("/spans").json().unwrap();
    assert!(
        spans["spans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|span| span["kind"] == "cmd")
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 3);
    assert_eq!(
        fs::metadata(&trace).unwrap().permissions().mode() & 0o777,
        0o000
    );
}

#[cfg(target_os = "linux")]
#[test]
fn foreign_only_status_keeps_github_answers_under_the_serving_account() {
    let machine = Machine::new();
    record(&machine.home("conductor"), 7, Some("failed"), 0);
    machine.setting("readable");
    let before = retained_files(&machine.home("conductor"));
    assert!(!machine.home("serving").join(".thirdshift/logs").exists());
    let output = machine.run(&["status", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["commands"][0]["account"], "conductor");
    assert!(
        machine
            .home("serving")
            .join(".thirdshift/logs/github-inbox.json")
            .exists()
    );
    assert_eq!(retained_files(&machine.home("conductor")), before);
}

#[cfg(target_os = "linux")]
#[test]
fn unreadable_foreign_logs_are_omitted_without_a_page_error() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::new();
    let home = machine.home("conductor");
    record(&home, 7, Some("failed"), 0);
    let dir = home.join(".thirdshift/logs/acme/widgets/commands/issue");
    let log = dir.join("7-fixture.log");
    fs::write(&log, "private output").unwrap();
    machine.setting("conductor");
    let output = machine.run(&["logs", "--json"]);
    let logs: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = logs["logs"][0]["id"].as_str().unwrap();
    fs::set_permissions(&log, fs::Permissions::from_mode(0o000)).unwrap();
    let output = machine.run(&["logs", "--json"]);
    let logs: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(logs["logs"].as_array().unwrap().is_empty());
    let console = Console::start_command({
        let mut command = machine.command();
        command.args(["console", "--no-sign-in"]);
        command
    });
    assert_eq!(console.get(&format!("/logs/{id}")).status(), 404);
    let selector = dir.join("7-fixture.command.json");
    let query = serde_urlencoded::to_string([("command", selector.to_str().unwrap())]).unwrap();
    assert_eq!(console.get(&format!("/log?{query}")).status(), 404);
    assert!(console.get("/logs").status().is_success());
    assert!(console.get("/").status().is_success());
}

#[cfg(target_os = "linux")]
#[test]
fn other_account_live_actions_never_signal_or_write_and_details_never_cache_there() {
    use super::watching::{FixtureProcess, live_record};
    let machine = Machine::new();
    let home = machine.home("conductor");
    let mut work = FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
    let fixture = live_record(&home, 8, work.0.id());
    write_record(&home, 8, &fixture);
    let mut ended = record(&home, 7, Some("failed"), 0);
    let dir = home.join(".thirdshift/logs/acme/widgets/commands/issue");
    let trace = dir.join("7-fixture-implement.logs.jsonl");
    fs::write(&trace, "").unwrap();
    ended["session_log"] = json!(trace);
    ended["stages"][0]["session_log"] = ended["session_log"].clone();
    write_record(&home, 7, &ended);
    machine.setting("conductor");
    let before = retained_files(&home);
    let path = dir.join("8-fixture.command.json");
    let selector = path.to_str().unwrap();
    for args in [
        vec!["stop", selector],
        vec!["stop", selector, "--requeue"],
        vec!["dismiss", selector],
        vec!["dismiss", selector, "--undo"],
    ] {
        let output = machine.run(&args);
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("account conductor"),
            "{output:?}"
        );
        assert!(work.0.try_wait().unwrap().is_none());
    }
    let console = Console::start_command({
        let mut command = machine.command();
        command.args(["console", "--no-sign-in"]);
        command
    });
    let token: Value = console.get("/look").json().unwrap();
    for (action, ending) in [
        ("stop", "triage"),
        ("stop", "requeue"),
        ("dismiss", ""),
        ("dismiss/undo", ""),
    ] {
        let response = reqwest::blocking::Client::new()
            .post(format!("{}/{action}", console.address))
            .form(&[
                ("command", selector),
                ("csrf", token["csrf"].as_str().unwrap()),
                ("ending", ending),
                ("confirmed", "yes"),
            ])
            .send()
            .unwrap();
        assert_eq!(response.status(), 409);
        assert!(response.text().unwrap().contains("account conductor"));
        assert!(work.0.try_wait().unwrap().is_none());
    }
    let path = dir.join("7-fixture.command.json");
    let output = machine.run(&["show", path.to_str().unwrap(), "--json"]);
    assert!(output.status.success(), "{output:?}");
    let details: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(details["command"]["account"], "conductor");
    assert!(details["command"]["run_metrics"].is_object());
    let query = serde_urlencoded::to_string([("command", path.to_str().unwrap())]).unwrap();
    assert!(console.get(&format!("/peek?{query}")).status().is_success());
    assert_eq!(retained_files(&home), before);
}

#[cfg(target_os = "linux")]
#[test]
fn readable_accounts_merge_needs_you_and_all_readers_without_foreign_writes() {
    let machine = Machine::new();
    let foreign = machine.home("conductor");
    let serving = machine.home("serving");
    let mut own_failure = record(&serving, 1, Some("failed"), 0);
    own_failure["cause"] = json!("same cause across accounts");
    write_record(&serving, 1, &own_failure);
    let mut foreign_failure = record(&foreign, 7, Some("failed"), 0);
    foreign_failure["cause"] = own_failure["cause"].clone();
    write_record(&foreign, 7, &foreign_failure);
    let dir = foreign.join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::write(dir.join("7-fixture.log"), "foreign output\n").unwrap();
    let override_dir = foreign.join("retained");
    fs::rename(foreign.join(".thirdshift/logs"), &override_dir).unwrap();
    fs::write(
        foreign.join(".thirdshift/config.toml"),
        "[logs]\ndir = '~/retained/'\n",
    )
    .unwrap();
    let output = machine.run(&["status", "--json"]);
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["commands"].as_array().unwrap().len(), 1);
    machine.setting("readable");
    let output = machine.run(&["status", "--json"]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("unreadable"),
        "{output:?}"
    );
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = status["commands"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{status}");
    let other = rows
        .iter()
        .find(|row| row["account"] == "conductor")
        .expect("account tag");
    assert_eq!(other["group"], "needs you");
    let path = override_dir.join("acme/widgets/commands/issue/7-fixture.command.json");
    let before = fs::read(&path).unwrap();
    let command = path.to_str().unwrap();
    let console = Console::start_command({
        let mut cmd = machine.command();
        cmd.args(["console", "--no-sign-in"]);
        cmd
    });
    let html = console.get("/").text().unwrap();
    assert!(section(&html, "needs you").contains("conductor"), "{html}");
    assert!(html.contains("2 need you"), "{html}");
    let row = html
        .split("<li class=\"command\"")
        .find(|row| row.contains("account-tag") && row.contains("conductor"))
        .unwrap();
    let inventory_url = row
        .split("href=\"")
        .skip(1)
        .filter_map(|part| part.split_once('"'))
        .map(|(url, _)| url)
        .find(|url| url.starts_with("/logs/command/"))
        .unwrap();
    assert!(
        console
            .get(inventory_url)
            .text()
            .unwrap()
            .contains("7-fixture.log")
    );
    for page in ["lanes", "stages"] {
        let html = console.get(&format!("/?page={page}")).text().unwrap();
        assert!(
            html.contains("[conductor]") && html.contains("acme/widgets#7"),
            "{html}"
        );
    }
    let query = serde_urlencoded::to_string([("command", command)]).unwrap();
    let peek = console.get(&format!("/peek?{query}")).text().unwrap();
    assert!(peek.contains("conductor"), "{peek}");
    assert!(
        !peek.contains(">Stop</button>") && !peek.contains(">Dismiss</button>"),
        "{peek}"
    );
    assert!(
        console
            .get("/?page=history")
            .text()
            .unwrap()
            .contains("conductor")
    );
    assert!(
        console
            .get(&format!("/spans?{query}"))
            .status()
            .is_success()
    );
    assert!(
        console
            .get(&format!("/log?{query}"))
            .text()
            .unwrap()
            .contains("foreign output")
    );
    let logs = machine.run(&["logs", "--json"]);
    let logs: Value = serde_json::from_slice(&logs.stdout).unwrap();
    let log = logs["logs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|log| log["account"] == "conductor")
        .unwrap();
    let id = log["id"].as_str().unwrap();
    assert!(
        console
            .get(&format!("/logs/{id}"))
            .text()
            .unwrap()
            .contains("foreign output")
    );
    let token: Value = console.get("/look").json().unwrap();
    for action in ["stop", "dismiss"] {
        let output = machine.run(&[action, command, "--json"]);
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("conductor"),
            "{output:?}"
        );
        let response = reqwest::blocking::Client::new()
            .post(format!("{}/{action}", console.address))
            .form(&[
                ("command", command),
                ("csrf", token["csrf"].as_str().unwrap()),
                ("ending", "triage"),
                ("confirmed", "yes"),
            ])
            .send()
            .unwrap();
        let body = response.text().unwrap();
        assert!(body.contains("conductor"), "{body}");
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 2);
    assert!(!override_dir.join("acme/widgets/activity.log").exists());
    machine.setting("conductor,unreadable,missing-account");
    let output = machine.run(&["status", "--json"]);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert_eq!(errors.matches("account unreadable").count(), 1, "{errors}");
    assert_eq!(
        errors.matches("account missing-account").count(),
        1,
        "{errors}"
    );
}

/// Another account chooses the Session path in its Command record. The serving
/// process must not open a trace outside that account's log directory.
#[cfg(target_os = "linux")]
#[test]
fn other_account_session_references_stay_inside_their_log_directory() {
    let machine = Machine::new();
    let home = machine.home("conductor");
    let outside = machine
        .home("serving")
        .parent()
        .unwrap()
        .join("outside-secret.logs.jsonl");
    let secret = "CANARY-OUTSIDE-SESSION";
    let spans = super::support::session_trace::artifact(&outside, "spans.jsonl");
    let attr = |key: &str, value: &str| json!({"key": key, "value": {"stringValue": value}});
    fs::write(
        &spans,
        serde_json::to_vec(&json!({
            "resourceSpans": [{"scopeSpans": [{"spans": [{
                "spanId": "1111111111111111",
                "name": "invoke_agent implement",
                "startTimeUnixNano": "1790856010000000000",
                "endTimeUnixNano": "1790856050000000000",
                "attributes": [
                    attr("gen_ai.operation.name", "invoke_agent"),
                    attr("thirdshift.session.prompt", secret)
                ]
            }]}]}]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut command = record(&home, 7, Some("failed"), 0);
    command["session_log"] = json!(outside);
    command["stages"][0]["session_log"] = command["session_log"].clone();
    write_record(&home, 7, &command);
    machine.setting("readable");
    let output = machine.run(&["show", "widgets#7", "--spans", "--json"]);
    let shown = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{output:?}");
    assert!(
        shown.contains("conductor"),
        "the other account's Command was not loaded: {shown}"
    );
    assert!(
        !shown.contains(secret),
        "session trace outside the other account's logs was read:\n{shown}"
    );
}

/// Another account's Command record is untrusted. Its issue URL and repository
/// must not be interpolated into the serving account's `gh` requests.
#[cfg(target_os = "linux")]
#[test]
fn other_account_records_cannot_shape_the_serving_accounts_github_requests() {
    let machine = Machine::new();
    let home = machine.home("conductor");
    let mut spec = record(&home, 7, Some("failed"), 0);
    spec["issue"]["url"] = json!("https://github.com/acme\"/widgets/issues/7");
    write_record(&home, 7, &spec);
    let mut security = record(&home, 8, Some("failed"), 0);
    security["kind"] = json!("Security run");
    security["security_findings_waiting"] = json!(true);
    security["repository"] = json!("acme/../../sensitive");
    security["issue"] = Value::Null;
    write_record(&home, 8, &security);
    let log = machine.home("serving").parent().unwrap().join("gh.log");
    fs::write(
        machine.home("serving").parent().unwrap().join("bin/gh"),
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\ncase \"$1 $2\" in\n  'api rate_limit')\n    printf '%s\\n' '{{\"resources\":{{\"graphql\":{{\"remaining\":5000,\"reset\":1893456000}},\"core\":{{\"remaining\":5000,\"reset\":1893456000}}}}}}'\n    ;;\n  'api graphql')\n    printf '%s\\n' '{{\"data\":{{\"rateLimit\":{{\"remaining\":5000,\"resetAt\":\"2030-01-01T00:00:00Z\"}}}}}}'\n    ;;\n  *)\n    printf '%s\\n' '[]'\n    ;;\nesac\nexit 0\n",
            shell_quote(&log)
        ),
    )
    .unwrap();
    machine.setting("readable");
    let output = machine.run(&["status", "--json"]);
    assert!(output.status.success(), "{output:?}");
    let seen = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !seen.contains("acme\"") && !seen.contains("sensitive"),
        "other-account record reached gh:\n{seen}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

#[test]
fn other_accounts_cli_sets_shows_and_validates_without_changing_other_settings() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let config = home.path().join(".thirdshift/config.toml");
    let before = fs::read_to_string(&config).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .args(args)
            .env("HOME", home.path())
            .output()
            .unwrap()
    };
    let initial = run(&["console", "other-accounts", "--json"]);
    assert!(initial.status.success(), "{initial:?}");
    assert_eq!(
        serde_json::from_slice::<Value>(&initial.stdout).unwrap(),
        json!("none")
    );
    for (input, expected) in [
        ("readable", json!("readable")),
        ("conductor,operator", json!(["conductor", "operator"])),
        ("none", json!("none")),
    ] {
        let result = run(&["console", "other-accounts", input, "--json"]);
        assert!(result.status.success(), "{result:?}");
        assert_eq!(
            serde_json::from_slice::<Value>(&result.stdout).unwrap(),
            expected
        );
        let result = run(&["console", "other-accounts", "--json"]);
        assert_eq!(
            serde_json::from_slice::<Value>(&result.stdout).unwrap(),
            expected
        );
    }
    assert!(fs::read_to_string(&config).unwrap().starts_with(&before));
    let saved = fs::read_to_string(&config).unwrap();
    for invalid in ["../operator", "", "a,,b", "a/b"] {
        let result = run(&["console", "other-accounts", invalid]);
        assert!(!result.status.success(), "{result:?}");
        assert_eq!(fs::read_to_string(&config).unwrap(), saved);
    }
}
