use super::*;
use crate::support::resend::ResendStandIn;

fn start(home: &Path, resend: &ResendStandIn, environment_key: bool) -> Console {
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command
        .args(["console"])
        .env("HOME", home)
        .env("THIRDSHIFT_RESEND_URL", resend.url())
        .env_remove("RESEND_API_KEY");
    if environment_key {
        command.env("RESEND_API_KEY", "re_environment");
    }
    Console::start_command(command)
}

fn form(console: &Console) -> (String, String) {
    let response = get(console, "/set-password", "");
    assert_eq!(response.status(), 200);
    let cookie = cookie(&response);
    (cookie, csrf(&response.text().unwrap()))
}

fn post(console: &Console, path: &str, fields: &[(&str, &str)]) -> reqwest::blocking::Response {
    let (cookie, csrf) = form(console);
    let mut fields = fields.to_vec();
    fields.push(("csrf", &csrf));
    client()
        .post(format!("{}{path}", console.address))
        .header("cookie", cookie)
        .form(&fields)
        .send()
        .unwrap()
}

fn code(resend: &ResendStandIn) -> String {
    wait_for_email(resend);
    let requests = resend.requests();
    let text = requests.last().unwrap().body["text"].as_str().unwrap();
    text.split_whitespace()
        .find(|word| word.len() == 6 && word.bytes().all(|b| b.is_ascii_digit()))
        .expect("six-digit code in email")
        .into()
}

fn wait_for_email(resend: &ResendStandIn) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while resend.requests().is_empty() {
        assert!(Instant::now() < deadline, "no code reached Resend");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn email_code_creates_and_resets_an_account_without_revealing_the_allowlist() {
    for environment_key in [false, true] {
        let home = configured();
        fs::write(
            home.path().join(".thirdshift/credentials.toml"),
            "[resend]\nkey = 're_credentials'\n",
        )
        .unwrap();
        let mut config = fs::read_to_string(home.path().join(".thirdshift/config.toml")).unwrap();
        config.push_str("[email]\nfrom = 'console@example.test'\n");
        fs::write(home.path().join(".thirdshift/config.toml"), config).unwrap();
        let resend = ResendStandIn::replying(200, "{}");
        if environment_key {
            assert!(
                password(home.path(), None, "old password", "old password")
                    .0
                    .success()
            );
        }
        let console = start(home.path(), &resend, environment_key);
        let old_session =
            environment_key.then(|| cookie(&sign_in(&console, EMAIL, "old password")));
        assert!(
            get(&console, "/sign-in", "")
                .text()
                .unwrap()
                .contains("/set-password")
        );
        let unknown = post(
            &console,
            "/set-password/code",
            &[("email", "unknown@example.test")],
        );
        let allowed = post(&console, "/set-password/code", &[("email", EMAIL)]);
        assert_eq!(unknown.status(), allowed.status());
        assert_eq!(unknown.text().unwrap(), allowed.text().unwrap());
        wait_for_email(&resend);
        let requests = resend.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/emails");
        assert_eq!(requests[0].body["to"], EMAIL);
        assert_eq!(requests[0].body["from"], "console@example.test");
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some(if environment_key {
                "Bearer re_environment"
            } else {
                "Bearer re_credentials"
            })
        );
        let code = code(&resend);
        let stored: Value = serde_json::from_slice(
            &fs::read(home.path().join(".thirdshift/console-accounts.json")).unwrap(),
        )
        .unwrap();
        assert!(
            stored[EMAIL]["codes"]["pending"]["hash"]
                .as_str()
                .unwrap()
                .starts_with("$argon2id$")
        );
        assert!(
            !serde_json::to_string(&stored)
                .unwrap()
                .contains(&format!("\"{code}\""))
        );
        let response = post(
            &console,
            "/set-password",
            &[("email", EMAIL), ("code", &code), ("password", PASSWORD)],
        );
        assert_eq!(response.status(), 303);
        let session = cookie(&response);
        assert_eq!(get(&console, "/", &session).status(), 200);
        if let Some(old_session) = old_session {
            assert_eq!(get(&console, "/", &old_session).status(), 303);
            assert_eq!(sign_in(&console, EMAIL, "old password").status(), 401);
        }
        assert_eq!(sign_in(&console, EMAIL, PASSWORD).status(), 303);
        let stored: Value = serde_json::from_slice(
            &fs::read(home.path().join(".thirdshift/console-accounts.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(stored[EMAIL]["verified"], true);
        assert_eq!(
            post(
                &console,
                "/set-password",
                &[("email", EMAIL), ("code", &code), ("password", "replay")]
            )
            .status(),
            401
        );
        assert!(
            password(home.path(), None, "new password", "new password")
                .0
                .success()
        );
        assert_eq!(get(&console, "/", &session).status(), 303);
        let events: Vec<Value> =
            serde_json::from_slice(&cli(home.path(), &["console", "log", "--json"]).stdout)
                .unwrap();
        assert!(
            events
                .iter()
                .any(|event| event["event"] == "code-sent" && event["email"] == EMAIL)
        );
        assert!(
            events
                .iter()
                .any(|event| event["event"] == "password-set" && event["email"] == EMAIL)
        );
    }
}

#[test]
fn the_code_page_uses_the_console_theme_and_stylesheet() {
    let home = configured();
    let resend = ResendStandIn::replying(200, "{}");
    let console = start(home.path(), &resend, true);
    let response = post(&console, "/set-password/code", &[("email", EMAIL)]);
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains("data-theme=\"focus-auto\""), "{html}");
    assert!(html.contains("href=\"/assets/console.css\""), "{html}");
    assert!(html.contains("a code has been sent"), "{html}");
    assert!(html.contains("href=\"/set-password\""), "{html}");
    wait_for_email(&resend);
}

#[test]
fn code_flow_requires_csrf_and_enforces_five_tries_and_send_throttling_over_http() {
    let home = configured();
    let resend = ResendStandIn::replying(200, "{}");
    let console = start(home.path(), &resend, true);
    for path in ["/set-password/code", "/set-password"] {
        assert_eq!(
            client()
                .post(format!("{}{path}", console.address))
                .form(&[("email", EMAIL), ("code", "123456"), ("password", PASSWORD)])
                .send()
                .unwrap()
                .status(),
            403
        );
        let (cookie, _) = form(&console);
        assert_eq!(
            client()
                .post(format!("{}{path}", console.address))
                .header("cookie", cookie)
                .form(&[
                    ("email", EMAIL),
                    ("code", "123456"),
                    ("password", PASSWORD),
                    ("csrf", "wrong")
                ])
                .send()
                .unwrap()
                .status(),
            403
        );
    }
    assert!(resend.requests().is_empty());
    let first = post(&console, "/set-password/code", &[("email", EMAIL)])
        .text()
        .unwrap();
    let code = code(&resend);
    assert_eq!(
        post(&console, "/set-password/code", &[("email", EMAIL)])
            .text()
            .unwrap(),
        first
    );
    assert_eq!(resend.requests().len(), 1);
    let wrong = if code == "000000" { "111111" } else { "000000" };
    for _ in 0..5 {
        assert_eq!(
            post(
                &console,
                "/set-password",
                &[("email", EMAIL), ("code", wrong), ("password", PASSWORD)]
            )
            .status(),
            401
        );
    }
    // The attempt budget is durable across server restarts.
    drop(console);
    let console = start(home.path(), &resend, true);
    assert_eq!(
        post(
            &console,
            "/set-password",
            &[("email", EMAIL), ("code", &code), ("password", PASSWORD)]
        )
        .status(),
        401
    );
    assert_eq!(
        post(&console, "/set-password/code", &[("email", EMAIL)])
            .text()
            .unwrap(),
        first
    );
    assert_eq!(resend.requests().len(), 1);
}

#[test]
fn email_flow_is_off_without_a_key_and_in_no_sign_in_mode() {
    let home = configured();
    let resend = ResendStandIn::replying(200, "{}");
    let console = start(home.path(), &resend, false);
    for path in ["/sign-in", "/set-password"] {
        let html = get(&console, path, "").text().unwrap();
        assert!(html.contains("thirdshift console password"));
        assert!(!html.contains("Send code"));
    }
    for path in ["/set-password/code", "/set-password"] {
        let page = get(&console, "/sign-in", "");
        let cookie = cookie(&page);
        let csrf = csrf(&page.text().unwrap());
        let response = client()
            .post(format!("{}{path}", console.address))
            .header("cookie", cookie)
            .form(&[
                ("email", EMAIL),
                ("code", "123456"),
                ("password", PASSWORD),
                ("csrf", &csrf),
            ])
            .send()
            .unwrap();
        assert_eq!(response.status(), 503);
        assert!(
            response
                .text()
                .unwrap()
                .contains("thirdshift console password")
        );
    }
    assert!(resend.requests().is_empty());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    assert_eq!(get(&console, "/set-password", "").status(), 404);
    for path in ["/set-password/code", "/set-password"] {
        assert_eq!(
            client()
                .post(format!("{}{path}", console.address))
                .form(&[("email", EMAIL)])
                .send()
                .unwrap()
                .status(),
            404
        );
    }
}

#[test]
fn resend_refusals_do_not_reveal_the_allowlist_or_record_a_code_sent() {
    let home = configured();
    let resend = ResendStandIn::replying(503, "unavailable");
    let console = start(home.path(), &resend, true);
    let unknown = post(
        &console,
        "/set-password/code",
        &[("email", "unknown@example.test")],
    );
    let allowed = post(&console, "/set-password/code", &[("email", EMAIL)]);
    assert_eq!(unknown.status(), allowed.status());
    assert_eq!(unknown.text().unwrap(), allowed.text().unwrap());
    wait_for_email(&resend);
    assert_eq!(resend.requests().len(), 1);
    let events: Vec<Value> =
        serde_json::from_slice(&cli(home.path(), &["console", "log", "--json"]).stdout).unwrap();
    assert!(!events.iter().any(|event| event["event"] == "code-sent"));
}

#[test]
fn code_response_timing_does_not_reveal_the_allowlist() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let resend_url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut length = 0;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        reader.read_exact(&mut vec![0; length]).unwrap();
        std::thread::sleep(Duration::from_secs(2));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\
                  Connection: close\r\n\r\n{}",
            )
            .unwrap();
    });

    let home = configured();
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command
        .args(["console"])
        .env("HOME", home.path())
        .env("RESEND_API_KEY", "re_test")
        .env("THIRDSHIFT_RESEND_URL", resend_url);
    let console = Console::start_command(command);

    let started = Instant::now();
    let unknown = post(
        &console,
        "/set-password/code",
        &[("email", "unknown@example.test")],
    );
    let unknown_elapsed = started.elapsed();
    let unknown_status = unknown.status();
    let unknown_body = unknown.text().unwrap();

    let started = Instant::now();
    let allowed = post(&console, "/set-password/code", &[("email", EMAIL)]);
    let allowed_elapsed = started.elapsed();
    let allowed_status = allowed.status();
    let allowed_body = allowed.text().unwrap();
    server.join().unwrap();

    assert_eq!(unknown_status, allowed_status);
    assert_eq!(unknown_body, allowed_body);
    assert!(
        allowed_elapsed.abs_diff(unknown_elapsed) < Duration::from_secs(1),
        "allowlist exposed: unknown={unknown_elapsed:?}, allowed={allowed_elapsed:?}",
    );
}

// Prepare old durable input while the server is stopped, rather than adding
// a test-only clock to the production HTTP interface.
fn stored_codes(home: &Path, edit: impl FnOnce(&mut Value)) {
    let path = home.join(".thirdshift/console-accounts.json");
    let mut accounts: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    edit(&mut accounts[EMAIL]["codes"]);
    fs::write(path, serde_json::to_vec(&accounts).unwrap()).unwrap();
}

#[test]
fn code_expiry_is_enforced_over_http_with_a_delivered_code() {
    for (age, expected_status) in [(14 * 60, 303), (15 * 60, 401)] {
        let home = configured();
        let resend = ResendStandIn::replying(200, "{}");
        let console = start(home.path(), &resend, true);
        assert_eq!(
            post(&console, "/set-password/code", &[("email", EMAIL)]).status(),
            200
        );
        let code = code(&resend);
        // The log command takes the shared lock, so the delivered send is
        // finished before the fixture is closed and its timestamps aged.
        assert!(cli(home.path(), &["console", "log"]).status.success());
        drop(console);
        stored_codes(home.path(), |codes| {
            codes["pending"]["sent_at"] = json!(chrono::Utc::now().timestamp() - age);
        });
        let console = start(home.path(), &resend, true);
        assert_eq!(
            post(
                &console,
                "/set-password",
                &[("email", EMAIL), ("code", &code), ("password", PASSWORD)]
            )
            .status(),
            expected_status
        );
    }
}

#[test]
fn daily_code_limit_is_enforced_over_http_and_ages_out() {
    let home = configured();
    let resend = ResendStandIn::replying(200, "{}");
    let console = start(home.path(), &resend, true);
    assert_eq!(
        post(&console, "/set-password/code", &[("email", EMAIL)]).status(),
        200
    );
    let _ = code(&resend);
    assert!(cli(home.path(), &["console", "log"]).status.success());
    drop(console);
    stored_codes(home.path(), |codes| {
        let now = chrono::Utc::now().timestamp();
        codes["sent"] = json!(
            (1..=10)
                .rev()
                .map(|minute| now - minute * 60)
                .collect::<Vec<_>>()
        );
    });
    let console = start(home.path(), &resend, true);
    let refused = post(&console, "/set-password/code", &[("email", EMAIL)])
        .text()
        .unwrap();
    // Delivery is asynchronous to prevent allowlist disclosure. Observe the
    // absence of a second email across a bounded delivery window.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        assert_eq!(resend.requests().len(), 1, "eleventh code was sent");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(cli(home.path(), &["console", "log"]).status.success());
    drop(console);
    stored_codes(home.path(), |codes| {
        let now = chrono::Utc::now().timestamp();
        codes["sent"] = json!([now - 24 * 60 * 60]);
    });
    let console = start(home.path(), &resend, true);
    assert_eq!(
        post(&console, "/set-password/code", &[("email", EMAIL)])
            .text()
            .unwrap(),
        refused
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while resend.requests().len() < 2 {
        assert!(Instant::now() < deadline, "code stayed blocked after a day");
        std::thread::sleep(Duration::from_millis(10));
    }
    let fresh = code(&resend);
    assert_eq!(
        post(
            &console,
            "/set-password",
            &[("email", EMAIL), ("code", &fresh), ("password", PASSWORD)]
        )
        .status(),
        303
    );
}
