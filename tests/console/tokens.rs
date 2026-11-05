use super::*;
use std::process::Output;

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

fn create(home: &Path, holder: &str, options: &[&str]) -> String {
    let mut args = vec!["console", "token", "create", holder];
    args.extend(options);
    let output = cli(home, &args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let token = String::from_utf8(output.stdout).unwrap();
    assert!(output.stderr.is_empty());
    let token = token.trim();
    assert!(token.starts_with("ts_"), "token has a recognizable prefix");
    assert_eq!(token.len(), 67, "32 random bytes encoded as hex after ts_");
    assert!(token[3..].bytes().all(|byte| byte.is_ascii_hexdigit()));
    token.to_owned()
}

fn list(home: &Path) -> Value {
    api::cli(home, &["console", "token", "list", "--json"])
}

#[test]
fn token_cli_shows_the_secret_once_and_lists_only_metadata() {
    let home = tempfile::tempdir().unwrap();
    let token = create(home.path(), "reader", &[]);
    let tokens = list(home.path());
    assert_eq!(tokens[0]["holder"], "reader");
    assert_eq!(tokens[0]["scopes"], json!(["read"]));
    assert!(tokens[0]["issued_at"].as_str().is_some());
    assert!(tokens[0]["expires_at"].is_null());
    assert!(tokens[0]["last_use"].is_null());
    assert!(!tokens.to_string().contains(&token));
    let text = cli(home.path(), &["console", "token", "list"]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("reader") && text.contains("read"));
    assert!(!text.contains(&token));
    let duplicate = cli(home.path(), &["console", "token", "create", "reader"]);
    assert!(!duplicate.status.success());
    assert_eq!(list(home.path()), tokens);
    let revoked = cli(home.path(), &["console", "token", "revoke", "reader"]);
    assert!(revoked.status.success());
    assert_eq!(list(home.path()), json!([]));
}

fn server(home: &Path) -> Console {
    configure(home);
    fs::write(
        home.join(".thirdshift/config.toml"),
        "[console]\nport = 0\nallowed_emails = [\"owner@example.test\"]\n",
    )
    .unwrap();
    Console::start_mode(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home, false)
}

fn request(console: &Console, token: &str, path: &str) -> reqwest::blocking::RequestBuilder {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .get(format!("{}{path}", console.address))
        .bearer_auth(token)
}

#[test]
fn bearer_read_updates_last_use_and_revoke_takes_effect_on_the_next_request() {
    let home = tempfile::tempdir().unwrap();
    let token = create(home.path(), "reader", &[]);
    let console = server(home.path());
    assert_eq!(list(home.path())[0]["last_use"], Value::Null);
    let expected = api::cli(home.path(), &["status", "--json"]);
    assert_eq!(
        api::json(request(&console, &token, "/api/v1/status").send().unwrap()),
        expected
    );
    assert!(list(home.path())[0]["last_use"].as_str().is_some());
    assert!(
        cli(home.path(), &["console", "token", "revoke", "reader"])
            .status
            .success()
    );
    let response = request(&console, &token, "/api/v1/status").send().unwrap();
    assert_eq!(response.status(), 401);
    assert!(response.json::<Value>().unwrap()["error"].is_string());
}

#[test]
fn route_scopes_refuse_a_token_missing_read_and_name_the_scope() {
    let home = tempfile::tempdir().unwrap();
    let token = create(
        home.path(),
        "actor",
        &["--scopes", "security,act,run,admin"],
    );
    assert_eq!(
        list(home.path())[0]["scopes"],
        json!(["security", "act", "run", "admin"])
    );
    let console = server(home.path());
    let response = request(&console, &token, "/api/v1/status").send().unwrap();
    assert_eq!(response.status(), 403);
    assert!(
        response.json::<Value>().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("read")
    );
}

#[test]
fn whoami_reports_the_machine_caller_scopes_and_local_cli_parity() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let local = api::cli(home.path(), &["console", "whoami", "--json"]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    assert_eq!(api::json(console.get("/api/v1/whoami")), local);
    let version = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .arg("--version")
        .output()
        .unwrap();
    let version = String::from_utf8(version.stdout).unwrap();
    assert_eq!(
        local["thirdshift_version"],
        version.trim().strip_prefix("thirdshift ").unwrap()
    );
    assert_eq!(local["api_version"], "v1");
    assert!(!local["hostname"].as_str().unwrap().is_empty());
    assert_eq!(local["identity"]["kind"], "local");
    assert_eq!(
        local["scopes"],
        json!(["read", "security", "act", "run", "admin"])
    );
    let token = create(home.path(), "laptop", &["--scopes", "read,act"]);
    let identity = api::json(request(&console, &token, "/api/v1/whoami").send().unwrap());
    assert_eq!(
        identity["identity"],
        json!({"kind": "token", "holder": "laptop"})
    );
    assert_eq!(identity["scopes"], json!(["read", "act"]));
    for field in ["hostname", "thirdshift_version", "api_version"] {
        assert_eq!(identity[field], local[field]);
    }
    assert!(!identity.to_string().contains(&token));
    assert_eq!(
        request(&console, "invalid-token", "/api/v1/whoami")
            .send()
            .unwrap()
            .status(),
        401,
        "an explicit invalid bearer cannot fall back to --no-sign-in"
    );
}

#[test]
fn bearer_transport_refuses_cleartext_without_confirming_validity_or_recording_use() {
    let home = tempfile::tempdir().unwrap();
    let token = create(home.path(), "reader", &[]);
    let console = server(home.path());
    for (name, value) in [
        ("x-forwarded-proto", "http"),
        ("x-forwarded-proto", "https,http"),
        ("x-forwarded-for", "192.0.2.1"),
        ("x-forwarded-host", "console.example.test"),
        ("forwarded", "for=192.0.2.1;proto=http"),
        ("x-real-ip", "192.0.2.1"),
        ("via", "proxy"),
    ] {
        let mut refusals = Vec::new();
        for given in [&*token, "invalid-token"] {
            let response = request(&console, given, "/api/v1/status")
                .header(name, value)
                .send()
                .unwrap();
            assert_eq!(response.status(), 401, "{name}: {value}");
            assert_eq!(response.headers()["content-type"], "application/json");
            refusals.push(response.text().unwrap());
        }
        assert_eq!(
            refusals[0], refusals[1],
            "the transport refusal reveals no validity"
        );
    }
    let response = request(&console, &token, "/api/v1/status")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-proto", "https")
        .send()
        .unwrap();
    assert_eq!(response.status(), 401, "ambiguous forwarding is refused");
    assert!(list(home.path())[0]["last_use"].is_null());
    let response = request(&console, &token, "/api/v1/status")
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-for", "192.0.2.1")
        .send()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(list(home.path())[0]["last_use"].is_string());
}

#[test]
fn expired_tokens_are_refused_and_invalid_options_create_no_grant() {
    let home = tempfile::tempdir().unwrap();
    let expired = create(home.path(), "expired", &["--expires", "2000-01-01"]);
    let timestamp = create(
        home.path(),
        "timestamp",
        &["--expires", "2000-01-01T01:00:00+01:00"],
    );
    let valid = create(home.path(), "valid", &["--expires", "2100-01-01"]);
    let console = server(home.path());
    for token in [expired.as_str(), timestamp.as_str(), "invalid-token"] {
        let response = request(&console, token, "/api/v1/status").send().unwrap();
        assert_eq!(response.status(), 401);
    }
    assert_eq!(
        request(&console, &valid, "/api/v1/status")
            .send()
            .unwrap()
            .status(),
        200
    );
    let before = list(home.path());
    assert_eq!(before[0]["expires_at"], "2000-01-01T00:00:00Z");
    assert_eq!(before[1]["expires_at"], "2000-01-01T00:00:00Z");
    assert!(before[0]["last_use"].is_null() && before[1]["last_use"].is_null());
    for options in [
        vec!["--scopes", "unknown"],
        vec!["--scopes", ""],
        vec!["--expires", "bad-date"],
        vec!["--scopes", "read", "--scopes", "act"],
    ] {
        let mut args = vec!["console", "token", "create", "invalid"];
        args.extend(options);
        assert!(!cli(home.path(), &args).status.success());
    }
    assert_eq!(list(home.path()), before);
    for path in [
        "/api/v1/token/create",
        "/api/v1/token/list",
        "/api/v1/token/revoke",
        "/api/v1/tokens",
    ] {
        assert_eq!(
            request(&console, &valid, path).send().unwrap().status(),
            404
        );
    }
}
