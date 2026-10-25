use super::*;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{ExitStatus, Output};
use std::time::Instant;

const EMAIL: &str = "owner@example.test";
const PASSWORD: &str = "correct horse battery staple";

#[path = "password_codes.rs"]
mod password_codes;

#[path = "stop.rs"]
mod stop;

#[path = "dismiss.rs"]
mod dismiss;

fn configured() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    allow(home.path(), true);
    home
}

fn allow(home: &Path, allowed: bool) {
    fs::write(
        home.join(".thirdshift/config.toml"),
        format!(
            "[console]\nport = 0\nallowed_emails = [{}]\n",
            if allowed {
                "\"owner@example.test\""
            } else {
                ""
            }
        ),
    )
    .unwrap();
}

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

fn client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

fn get(console: &Console, path: &str, cookie: &str) -> reqwest::blocking::Response {
    client()
        .get(format!("{}{path}", console.address))
        .header("cookie", cookie)
        .send()
        .unwrap()
}

fn csrf(html: &str) -> String {
    html.split_once("name=\"csrf\" value=\"")
        .expect("CSRF field")
        .1
        .split('"')
        .next()
        .unwrap()
        .into()
}

fn cookie(response: &reqwest::blocking::Response) -> String {
    response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into()
}

fn sign_in(console: &Console, email: &str, password: &str) -> reqwest::blocking::Response {
    let form = get(console, "/sign-in", "");
    assert_eq!(form.status(), 200);
    let cookie = cookie(&form);
    let token = csrf(&form.text().unwrap());
    client()
        .post(format!("{}/sign-in", console.address))
        .header("cookie", cookie)
        .form(&[("email", email), ("password", password), ("csrf", &token)])
        .send()
        .unwrap()
}

#[test]
fn account_pages_and_sign_in_follow_the_configured_theme() {
    let home = configured();
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    for theme in ["focus-dark", "focus-light"] {
        fs::write(
            home.path().join(".thirdshift/config.toml"),
            format!("[console]\nport = 0\nallowed_emails = [\"{EMAIL}\"]\ntheme = \"{theme}\"\n"),
        )
        .unwrap();
        let attribute = format!("data-theme=\"{theme}\"");
        assert!(
            get(&console, "/sign-in", "")
                .text()
                .unwrap()
                .contains(&attribute)
        );
        let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
        for path in ["/", "/lanes", "/stages", "/account", "/console-log"] {
            let page = get(&console, path, &session).text().unwrap();
            assert!(page.contains(&attribute), "{path} ignores {theme}");
        }
    }
}

#[test]
fn sign_in_guards_pages_streams_and_actions_and_pairs_the_console_log_with_cli() {
    let home = configured();
    let (status, _) = password(home.path(), None, PASSWORD, PASSWORD);
    assert!(status.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    for path in [
        "/",
        "/resources",
        "/?page=lanes",
        "/?page=stages",
        "/?page=waterfall",
        "/?page=2",
        "/?page=3",
        "/?page=4",
        "/?page=history",
        "/?page=security",
        "/?page=logs",
        "/history",
        "/security",
        "/lanes",
        "/stages",
        "/waterfall",
        "/peek?command=widgets%237",
        "/log?command=widgets%237",
        "/logs",
        "/logs/command/id",
        "/logs/id",
        "/logs/id/stream/0",
        "/events",
        "/console-log",
        "/account",
        "/look",
    ] {
        assert_eq!(
            get(&console, path, "").status(),
            303,
            "{path} leaked without a session"
        );
    }
    assert_eq!(get(&console, "/assets/console.css", "").status(), 200);
    assert_eq!(
        client()
            .post(format!("{}/sign-in", console.address))
            .form(&[("email", EMAIL), ("password", PASSWORD)])
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(sign_in(&console, EMAIL, "wrong").status(), 401);
    assert_eq!(
        sign_in(&console, "outsider@example.test", PASSWORD).status(),
        401
    );
    let response = sign_in(&console, EMAIL, PASSWORD);
    assert_eq!(response.status(), 303);
    let attributes = response.headers()["set-cookie"].to_str().unwrap();
    for attribute in [
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
        "Max-Age=2592000",
        "Path=/",
    ] {
        assert!(attributes.contains(attribute), "{attributes}");
    }
    let session = cookie(&response);
    let accounts =
        fs::read_to_string(home.path().join(".thirdshift/console-accounts.json")).unwrap();
    assert!(accounts.contains("$argon2id$"));
    assert!(!accounts.contains(PASSWORD));
    assert!(!accounts.contains(session.split_once('=').unwrap().1));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(home.path().join(".thirdshift/console-accounts.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let before = cli(home.path(), &["console", "log", "--json"]).stdout;
    let page = get(&console, "/", &session);
    assert_eq!(page.status(), 200);
    for path in [
        "/?page=history",
        "/?page=security",
        "/?page=logs",
        "/history",
        "/security",
    ] {
        assert_eq!(
            get(&console, path, &session).status(),
            200,
            "signed-in access failed: {path}"
        );
    }
    assert!(
        page.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=2592000")
    );
    let account = get(&console, "/account", &session).text().unwrap();
    let token = csrf(&account);
    let log_page = get(&console, "/console-log", &session).text().unwrap();
    assert!(log_page.contains("thirdshift console log"));
    let events: Vec<Value> = serde_json::from_slice(&before).unwrap();
    for event in events {
        assert!(log_page.contains(event["event"].as_str().unwrap()));
    }
    assert_eq!(
        cli(home.path(), &["console", "log", "--json"]).stdout,
        before,
        "page views were logged"
    );
    for invalid in [None, Some("incorrect")] {
        let mut request = client()
            .post(format!("{}/sign-out", console.address))
            .header("cookie", &session);
        if let Some(token) = invalid {
            request = request.form(&[("csrf", token)]);
        }
        assert_eq!(request.send().unwrap().status(), 403);
        assert_eq!(get(&console, "/", &session).status(), 200);
    }
    let response = client()
        .post(format!("{}/sign-out", console.address))
        .header("cookie", &session)
        .form(&[("csrf", token)])
        .send()
        .unwrap();
    assert_eq!(response.status(), 303);
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(get(&console, "/", &session).status(), 303);
}

#[test]
fn signed_in_look_writes_name_the_account_and_keep_access_settings() {
    let home = configured();
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let look: Value = get(&console, "/look", &session).json().unwrap();
    for (path, fields) in [
        ("/theme", vec![("theme", "signal-light")]),
        ("/sections", vec![("hide", "done")]),
    ] {
        assert_eq!(
            client()
                .post(format!("{}{path}", console.address))
                .header("cookie", &session)
                .form(&fields)
                .send()
                .unwrap()
                .status(),
            403
        );
        assert_eq!(
            client()
                .post(format!("{}{path}", console.address))
                .header("cookie", &session)
                .header("x-csrf-token", look["csrf"].as_str().unwrap())
                .form(&fields)
                .send()
                .unwrap()
                .status(),
            200
        );
    }
    let output = cli(home.path(), &["console", "log", "--json"]);
    let events: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    for kind in ["theme-changed", "sections-changed"] {
        let changed: Vec<_> = events
            .iter()
            .filter(|event| event["event"] == kind)
            .collect();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0]["email"], EMAIL);
    }
    assert_eq!(get(&console, "/", &session).status(), 200);
    let config: Value = serde_json::to_value(
        toml::from_str::<toml::Table>(
            &fs::read_to_string(home.path().join(".thirdshift/config.toml")).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(config["console"]["allowed_emails"], json!([EMAIL]));
    assert_eq!(config["console"]["port"], 0);
}

#[test]
fn cli_and_allowlist_changes_revoke_http_sessions_without_deleting_the_account() {
    let home = configured();
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start_mode(binary, home.path(), false);
    let first = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let second = cookie(&sign_in(&console, EMAIL, PASSWORD));
    // A browser sign-out ends that browser, leaving the other one usable.
    let token = csrf(&get(&console, "/account", &first).text().unwrap());
    assert_eq!(
        client()
            .post(format!("{}/sign-out", console.address))
            .header("cookie", &first)
            .form(&[("csrf", token)])
            .send()
            .unwrap()
            .status(),
        303
    );
    assert_eq!(get(&console, "/", &first).status(), 303);
    assert_eq!(get(&console, "/", &second).status(), 200);
    let signed_out = cli(
        home.path(),
        &["console", "sign-out", "--email", EMAIL, "--json"],
    );
    assert!(signed_out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&signed_out.stdout).unwrap(),
        json!({"email": EMAIL, "signed_out": true})
    );
    assert_eq!(get(&console, "/", &second).status(), 303);
    let third = cookie(&sign_in(&console, EMAIL, PASSWORD));
    assert!(
        password(home.path(), Some(EMAIL), "new password", "new password")
            .0
            .success()
    );
    assert_eq!(get(&console, "/", &third).status(), 303);
    assert_eq!(sign_in(&console, EMAIL, PASSWORD).status(), 401);
    let fourth = cookie(&sign_in(&console, EMAIL, "new password"));
    drop(console);
    let console = Console::start_mode(binary, home.path(), false);
    assert_eq!(
        get(&console, "/", &fourth).status(),
        200,
        "sessions survive server restarts"
    );
    allow(home.path(), false);
    assert_eq!(
        get(&console, "/", &fourth).status(),
        303,
        "removal is effective on the next request"
    );
    assert_eq!(sign_in(&console, EMAIL, "new password").status(), 401);
    allow(home.path(), true);
    assert_eq!(
        get(&console, "/", &fourth).status(),
        303,
        "re-adding must not revive a session"
    );
    let fifth = cookie(&sign_in(&console, EMAIL, "new password"));
    // Removal must also be observed while no HTTP requests arrive.
    let before = fs::read_to_string(home.path().join(".thirdshift/console.log"))
        .unwrap()
        .len();
    allow(home.path(), false);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = fs::read_to_string(home.path().join(".thirdshift/console.log")).unwrap();
        if text.len() > before && text[before..].contains("blocked-address") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "idle server never revoked the removed account"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    allow(home.path(), true);
    assert_eq!(get(&console, "/", &fifth).status(), 303);
    assert_eq!(
        sign_in(&console, EMAIL, "new password").status(),
        303,
        "account password is retained"
    );
}

#[test]
fn http_slowdown_survives_restarts_and_allowed_accounts_still_need_a_password() {
    let home = configured();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start_mode(binary, home.path(), false);
    assert_eq!(
        sign_in(&console, EMAIL, PASSWORD).status(),
        401,
        "allowlisting alone grants no access"
    );
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    for _ in 0..5 {
        assert_eq!(sign_in(&console, EMAIL, "wrong").status(), 401);
    }
    let response = sign_in(&console, EMAIL, PASSWORD);
    assert_eq!(response.status(), 429);
    assert_eq!(response.headers()["retry-after"], "60");
    drop(console);
    let console = Console::start_mode(binary, home.path(), false);
    assert_eq!(sign_in(&console, EMAIL, PASSWORD).status(), 429);
}

const REVOCATIONS: [&str; 3] = ["sign-out", "password", "allowlist"];

fn revoke_session(home: &Path, how: &str) {
    match how {
        "sign-out" => assert!(cli(home, &["console", "sign-out"]).status.success()),
        "password" => assert!(password(home, None, PASSWORD, PASSWORD).0.success()),
        "allowlist" => allow(home, false),
        _ => unreachable!(),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn an_open_log_stream_closes_when_its_session_is_revoked() {
    for revoke in REVOCATIONS {
        let home = configured();
        assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
        let folder = home
            .path()
            .join(".thirdshift/logs/acme/widgets/commands/issue");
        let mut live = record(home.path(), 7, None, 0);
        let path = folder.join("7-fixture.log");
        fs::write(&path, "first line\n").unwrap();
        live["process_id"] = json!(std::process::id());
        let stat = fs::read_to_string("/proc/self/stat").unwrap();
        let ticks = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap();
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        live["process_start"] = json!(format!("{}:{ticks}", boot.trim()));
        write_record(home.path(), 7, &live);
        let console = Console::start_mode(
            Path::new(env!("CARGO_BIN_EXE_thirdshift")),
            home.path(),
            false,
        );
        let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
        let inventory = get(&console, "/logs", &session).text().unwrap();
        let link = inventory
            .split("href=\"")
            .skip(1)
            .filter_map(|part| part.split_once('"'))
            .find(|(url, _)| url.starts_with("/logs/"))
            .unwrap()
            .0;
        let mut response = get(&console, &format!("{link}/stream/0"), &session);
        assert_eq!(response.status(), 200);
        let mut buffer = [0; 4096];
        let first = response.read(&mut buffer).unwrap();
        assert!(String::from_utf8_lossy(&buffer[..first]).contains("first line"));
        assert!(
            String::from_utf8_lossy(&buffer[..first]).contains("\"done\":false"),
            "fixture must be a live stream"
        );
        revoke_session(home.path(), revoke);
        let mut rest = String::new();
        response.read_to_string(&mut rest).unwrap();
        assert!(!rest.contains("first line"), "{revoke}");
        assert_eq!(
            get(&console, &format!("{link}/stream/0"), &session).status(),
            303,
            "{revoke}"
        );
    }
}

#[test]
fn an_open_inbox_stream_closes_when_its_session_is_revoked() {
    for revoke in REVOCATIONS {
        let home = configured();
        assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
        let console = Console::start_mode(
            Path::new(env!("CARGO_BIN_EXE_thirdshift")),
            home.path(),
            false,
        );
        let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
        let response = get(&console, "/events", &session);
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        let mut stream = BufReader::new(response);
        let mut first = String::new();
        loop {
            let mut line = String::new();
            assert!(stream.read_line(&mut line).unwrap() > 0);
            if line.trim().is_empty() {
                break;
            }
            first.push_str(&line);
        }
        assert!(first.contains("event: commands"));
        revoke_session(home.path(), revoke);
        let mut rest = String::new();
        stream.read_to_string(&mut rest).unwrap();
        assert_eq!(get(&console, "/events", &session).status(), 303, "{revoke}");
    }
}

// Exercise the real terminal interface, including controlling-TTY input and echo.
fn password(home: &Path, email: Option<&str>, first: &str, second: &str) -> (ExitStatus, String) {
    let (mut master, mut slave) = (-1, -1);
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    let mut master = unsafe { fs::File::from_raw_fd(master) };
    let slave = unsafe { fs::File::from_raw_fd(slave) };
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command
        .args(["console", "password"])
        .env("HOME", home)
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave);
    if let Some(email) = email {
        command.args(["--email", email]);
    }
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut transcript = String::new();
    let mut prompts = 0;
    loop {
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("password command timed out: {transcript}");
        }
        let mut poll = libc::pollfd {
            fd: master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, 100) } > 0 {
            let mut buffer = [0; 1024];
            if let Ok(length) = master.read(&mut buffer) {
                transcript.push_str(&String::from_utf8_lossy(&buffer[..length]));
                if prompts == 0 && transcript.contains("Password: ") {
                    writeln!(master, "{first}").unwrap();
                    prompts = 1;
                }
                if prompts == 1 && transcript.contains("Confirm password: ") {
                    writeln!(master, "{second}").unwrap();
                    prompts = 2;
                }
            }
        }
        if let Some(status) = child.try_wait().unwrap() {
            return (status, transcript);
        }
    }
}

#[test]
fn console_password_prompts_privately_and_console_log_reports_account_events() {
    let home = configured();
    let (status, transcript) = password(home.path(), None, PASSWORD, PASSWORD);
    assert!(status.success(), "{transcript}");
    assert!(transcript.contains("Confirm password: "), "{transcript}");
    assert!(
        !transcript.contains(PASSWORD),
        "terminal echoed the password"
    );
    let log = cli(home.path(), &["console", "log", "--json"]);
    assert!(
        log.status.success(),
        "{}",
        String::from_utf8_lossy(&log.stderr)
    );
    let events: Vec<Value> = serde_json::from_slice(&log.stdout).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["email"], EMAIL);
    assert_eq!(events[0]["event"], "password-set");
    assert!(cli(home.path(), &["console", "sign-out"]).status.success());
    let text = cli(home.path(), &["console", "log"]);
    assert!(String::from_utf8_lossy(&text.stdout).contains("sign-out"));
    let (status, _) = password(
        home.path(),
        Some("outsider@example.test"),
        PASSWORD,
        PASSWORD,
    );
    assert!(!status.success());
    let (status, _) = password(home.path(), Some(EMAIL), "different", "mismatch");
    assert!(!status.success());
}
