use super::*;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

struct Console {
    child: Child,
    lines: Receiver<String>,
    reader: Option<thread::JoinHandle<()>>,
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reader.take().unwrap().join().unwrap();
    }
}

#[test]
fn admin_update_answers_after_installation_then_console_serves_the_new_version() {
    update(false);
}

#[test]
fn fleet_cli_update_checks_and_replaces_only_the_owners_binary() {
    update(true);
}

fn update(fleet: bool) {
    let scenario = Scenario::new();
    scenario.user_config_is("[console]\nport = 0\n");
    let home = scenario.path("home");
    let install = scenario.path("installed");
    fs::create_dir(&install).unwrap();
    let binary = install.join("thirdshift");
    fs::copy(env!("CARGO_BIN_EXE_thirdshift"), &binary).unwrap();
    write_install_receipt(&scenario, &install);
    let token = Command::new(&binary)
        .env("HOME", &home)
        .args(["console", "token", "create", "updater", "--scopes", "admin"])
        .output()
        .unwrap();
    assert!(token.status.success());
    let token = String::from_utf8(token.stdout).unwrap();
    let new_binary = scenario.path("new-console");
    // The release fixture is an executable Console that identifies its version.
    // Replacing a private installed copy exercises the real installer and exec.
    fs::write(
        &new_binary,
        r#"#!/usr/bin/env python3
import http.server, json
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({'thirdshift_version': '99.0.0'}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args): pass
server = http.server.HTTPServer(('127.0.0.1', 0), Handler)
print('http://127.0.0.1:' + str(server.server_port), flush=True)
server.serve_forever()
"#,
    )
    .unwrap();
    let completed = scenario.path("installer-completed");
    let github = FakeReleases::start();
    github.publish(
        "99.0.0",
        &format!(
            "cp '{}' '{}.new'\nchmod +x '{}.new'\nmv '{}.new' '{}'\nsleep 1\ntouch '{}'",
            new_binary.display(),
            binary.display(),
            binary.display(),
            binary.display(),
            binary.display(),
            completed.display()
        ),
    );
    let mut child = Command::new(&binary)
        .args(["console", "--no-sign-in"])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("THIRDSHIFT_INSTALLER_GHE_BASE_URL", &github.url)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let console = Console {
        child,
        lines,
        reader: Some(reader),
    };
    let address = console.lines.recv_timeout(Duration::from_secs(20)).unwrap();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();
    let checked = scenario.run_with_env(
        &["update", "--check", "--json"],
        &[("THIRDSHIFT_INSTALLER_GHE_BASE_URL", &github.url)],
    );
    assert_eq!(checked.code, Some(0), "{}", checked.stderr);
    let checked: serde_json::Value = serde_json::from_str(&checked.stdout).unwrap();
    assert_eq!(checked, json!({"current":VERSION,"latest":"99.0.0"}));
    let http_check: serde_json::Value = client
        .get(format!("{address}/api/v1/update"))
        .bearer_auth(token.trim())
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(http_check, checked);
    assert!(!completed.exists(), "a version check must not install");
    let body = if fleet {
        let caller = Scenario::new();
        caller.user_config_is("");
        let mut pairing = caller
            .command(&["console", "fleet", "add", "owner", &address])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(pairing.stdin.take().unwrap(), "{}", token.trim()).unwrap();
        let paired = pairing.wait_with_output().unwrap();
        assert!(paired.status.success(), "{paired:?}");
        let remote_check = caller.run(&["update", "--check", "--json", "--machine", "owner"]);
        assert_eq!(remote_check.code, Some(0), "{}", remote_check.stderr);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&remote_check.stdout).unwrap(),
            checked
        );
        let updated = caller.run(&["update", "--json", "--machine", "owner"]);
        assert_eq!(updated.code, Some(0), "{}", updated.stderr);
        assert!(
            !caller.path("home/.thirdshift/console.log").exists(),
            "caller must not install or audit the owner's update"
        );
        updated.stdout
    } else {
        let response = client
            .post(format!("{address}/api/v1/update"))
            .bearer_auth(token.trim())
            .send()
            .unwrap();
        let status = response.status();
        let body = response.text().unwrap();
        assert_eq!(status, 200, "{body}");
        body
    };
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        json!(format!("updated thirdshift {VERSION} to 99.0.0"))
    );
    assert!(completed.exists(), "response must wait for installation");
    let new_address = console
        .lines
        .recv_timeout(Duration::from_secs(20))
        .expect("Console re-executes");
    let identity: serde_json::Value = client
        .get(format!("{new_address}/api/v1/whoami"))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(identity["thirdshift_version"], "99.0.0");
    let log = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .env("HOME", home)
        .args(["console", "log", "--json"])
        .output()
        .unwrap();
    assert!(log.status.success());
    let events: serde_json::Value = serde_json::from_slice(&log.stdout).unwrap();
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["email"] == "updater"
                && event["endpoint"] == "/api/v1/update"
                && event["outcome"] == 200)
    );
}
