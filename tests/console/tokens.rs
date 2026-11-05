use super::*;
use std::process::Output;

fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}

pub(super) fn create(home: &Path, holder: &str, options: &[&str]) -> String {
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

pub(super) fn request(
    console: &Console,
    token: &str,
    path: &str,
) -> reqwest::blocking::RequestBuilder {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
        .get(format!("{}{path}", console.address))
        .bearer_auth(token)
}

#[test]
fn failed_tokens_are_slowed_then_limited_per_forwarded_address() {
    let home = tempfile::tempdir().unwrap();
    let token = create(home.path(), "reader", &[]);
    let console = server(home.path());
    for attempt in 1..=8 {
        let started = std::time::Instant::now();
        let response = request(&console, "bad-token", "/api/v1/whoami")
            .header("x-forwarded-proto", "https")
            .header("x-forwarded-for", "192.0.2.1")
            .send()
            .unwrap();
        if attempt == 3 {
            assert!(
                started.elapsed() >= Duration::from_millis(200),
                "failed tokens must slow down"
            );
        }
        if attempt < 8 {
            assert_eq!(response.status(), 401, "attempt {attempt}");
        } else {
            assert_eq!(response.status(), 429);
            let retry: u64 = response.headers()["retry-after"]
                .to_str()
                .unwrap()
                .parse()
                .unwrap();
            assert!((1..=60).contains(&retry));
            let body = response.json::<Value>().unwrap();
            assert!(body["error"].is_string());
            assert_eq!(body["retry_after"], retry);
        }
        // A successful request elsewhere must not erase this address's failures.
        assert_eq!(
            request(&console, &token, "/api/v1/whoami")
                .header("x-forwarded-proto", "https")
                .header("x-forwarded-for", "192.0.2.2")
                .send()
                .unwrap()
                .status(),
            200
        );
    }
    assert_eq!(
        request(&console, &token, "/api/v1/whoami")
            .header("x-forwarded-proto", "https")
            .header("x-forwarded-for", "192.0.2.1")
            .send()
            .unwrap()
            .status(),
        429
    );
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
    let description = api::json(
        request(&console, &token, "/api/v1/openapi.json")
            .send()
            .unwrap(),
    );
    assert_eq!(description, api::cli(home.path(), &["console", "openapi"]));
    assert!(!description.to_string().contains(&token));
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
    for path in ["/api/v1/status", "/api/v1/whoami", "/api/v1/openapi.json"] {
        let response = request(&console, &token, path).send().unwrap();
        assert_eq!(response.status(), 403, "{path}");
        assert!(
            response.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("read")
        );
    }
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
fn security_scope_covers_dispatched_fix_details_and_sessions() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut parent = record(home.path(), 2, Some("failed"), 0);
    let directory = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = directory.join("7-20261105T120000-0500-implement.logs.jsonl");
    super::support::session_trace::write(&session, "private-security-fix-session\n");
    let mut fix = parent.clone();
    fix["kind"] = json!("Run");
    fix["name"] = json!("acme/widgets#7");
    fix["issue"]["number"] = json!(7);
    fix["issue"]["url"] = json!("https://github.com/acme/widgets/issues/7");
    fix["cause"] = json!("private-security-fix-details");
    fix["session_log"] = json!(session);
    fix["stages"][0]["session_log"] = json!(session);
    parent["kind"] = json!("Security run");
    parent["name"] = json!("secure-fixture");
    parent["issue"] = Value::Null;
    parent["dispatched"] = json!([fix]);
    write_record(home.path(), 2, &parent);
    let reader = create(home.path(), "reader", &[]);
    let reviewer = create(home.path(), "reviewer", &["--scopes", "read,security"]);
    let console = server(home.path());
    let logs = api::json(request(&console, &reader, "/api/v1/logs").send().unwrap());
    let id = logs["logs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|log| {
            log["name"]
                .as_str()
                .unwrap()
                .ends_with("implement.logs.jsonl")
        })
        .unwrap()["id"]
        .as_str()
        .unwrap();
    for path in [
        "/api/v1/peek?command=widgets%237".to_owned(),
        "/api/v1/spans?command=widgets%237".to_owned(),
        "/api/v1/waterfall?command=widgets%237".to_owned(),
        "/api/v1/log?command=widgets%237&session=implement".to_owned(),
        format!("/api/v1/logs/{id}"),
    ] {
        let response = request(&console, &reader, &path).send().unwrap();
        assert_eq!(response.status(), 403, "{path}");
        let error = response.json::<Value>().unwrap();
        assert!(error["error"].as_str().unwrap().contains("security"));
        assert!(!error.to_string().contains("private-security-fix"));
        assert_eq!(
            request(&console, &reviewer, &path).send().unwrap().status(),
            200,
            "{path}"
        );
    }
    let scoring = home.path().join(".thirdshift/evals/scoring");
    fs::create_dir_all(&scoring).unwrap();
    fs::write(scoring.join("broken.json"), "{").unwrap();
    for path in [
        "/api/v1/log?command=widgets%237&session=implement".to_owned(),
        format!("/api/v1/logs/{id}"),
    ] {
        let denied = request(&console, &reader, &path).send().unwrap();
        assert_eq!(denied.status(), 403, "{path}");
        assert!(
            denied.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("security")
        );
        let allowed = api::json(request(&console, &reviewer, &path).send().unwrap());
        assert!(
            allowed["contents"]
                .as_str()
                .unwrap()
                .contains("private-security-fix-session")
        );
    }
}

#[test]
fn read_scope_cannot_read_security_findings() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut security = record(home.path(), 2, Some("findings recorded"), 1);
    let directory = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let session = directory.join("2-20261105T120000-0500-security-audit.logs.jsonl");
    let ordinary = directory.join("2-20261105T120000-0500-implement.logs.jsonl");
    super::support::session_trace::write(&session, "PoC finding title 1755\n");
    super::support::session_trace::write(&ordinary, "ordinary session marker\n");
    security["kind"] = json!("Security run");
    security["name"] = json!("secure-fixture");
    security["issue"] = Value::Null;
    security["security_findings_waiting"] = json!(true);
    security["security_findings"] = json!([{
        "title": "PoC finding title 1755",
        "severity": "high",
        "url": "https://github.com/acme/widgets/security/advisories/GHSA-test"
    }]);
    security["session_log"] = json!(session);
    security["stages"][0]["session_log"] = json!(ordinary);
    write_record(home.path(), 2, &security);
    let token = create(home.path(), "reader", &[]);
    let console = server(home.path());
    for path in ["/api/v1/history", "/api/v1/status?all=true"] {
        let response = request(&console, &token, path).send().unwrap();
        let status = response.status();
        let body = response.text().unwrap();
        assert_eq!(status, 200, "{path}: {body}");
        assert!(
            !body.contains("PoC finding title 1755") && !body.contains("GHSA-test"),
            "{path} disclosed a security finding to a read token: {body}"
        );
        assert!(
            !body.contains("\"security_findings_waiting\":true"),
            "{path} told a read token that findings are waiting: {body}"
        );
    }
    for path in [
        "/api/v1/peek?command=secure-fixture",
        "/api/v1/waterfall?command=secure-fixture",
        "/api/v1/spans?command=secure-fixture",
    ] {
        let details = request(&console, &token, path).send().unwrap();
        assert_eq!(details.status(), 403, "{path}");
        assert!(
            details.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("security")
        );
    }
    assert_eq!(
        request(&console, &token, "/api/v1/security")
            .send()
            .unwrap()
            .status(),
        403
    );
    let session_log = request(
        &console,
        &token,
        "/api/v1/log?command=secure-fixture&session=security-audit",
    )
    .send()
    .unwrap();
    assert_eq!(session_log.status(), 403);
    let session_body = session_log.text().unwrap();
    assert!(
        !session_body.contains("PoC finding title 1755"),
        "{session_body}"
    );
    let ordinary_log = request(
        &console,
        &token,
        "/api/v1/log?command=secure-fixture&session=implement",
    )
    .send()
    .unwrap();
    assert_eq!(ordinary_log.status(), 403);
    let ordinary_body = ordinary_log.text().unwrap();
    assert!(
        !ordinary_body.contains("ordinary session marker"),
        "{ordinary_body}"
    );
    assert!(!ordinary_body.contains("PoC finding title 1755"));
    let listed = request(&console, &token, "/api/v1/logs")
        .send()
        .unwrap()
        .json::<Value>()
        .unwrap();
    let logs = listed["logs"].as_array().unwrap();
    let security_id = logs
        .iter()
        .find(|log| {
            log["name"]
                .as_str()
                .is_some_and(|name| name.contains("security-audit"))
        })
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let by_id = request(&console, &token, &format!("/api/v1/logs/{security_id}"))
        .send()
        .unwrap();
    assert_eq!(by_id.status(), 403);
    let by_id_body = by_id.text().unwrap();
    assert!(
        !by_id_body.contains("PoC finding title 1755"),
        "{by_id_body}"
    );
    let implement_id = logs
        .iter()
        .find(|log| {
            log["name"]
                .as_str()
                .is_some_and(|name| name.contains("implement"))
        })
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let implement = request(&console, &token, &format!("/api/v1/logs/{implement_id}"))
        .send()
        .unwrap();
    assert_eq!(implement.status(), 403);
    let implement_body = implement.text().unwrap();
    assert!(
        !implement_body.contains("ordinary session marker"),
        "{implement_body}"
    );

    let reviewer = create(home.path(), "reviewer", &["--scopes", "read,security"]);
    assert_eq!(
        api::json(
            request(&console, &reviewer, "/api/v1/peek?command=secure-fixture")
                .send()
                .unwrap()
        ),
        api::cli(home.path(), &["show", "secure-fixture", "--json"])
    );
    let visible = request(&console, &reviewer, "/api/v1/history")
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert!(visible.contains("PoC finding title 1755"));
    assert!(visible.contains("GHSA-test"));
    assert!(visible.contains("\"security_findings_waiting\":true"));
    let trace = request(
        &console,
        &reviewer,
        "/api/v1/log?command=secure-fixture&session=security-audit",
    )
    .send()
    .unwrap();
    assert_eq!(trace.status(), 200);
    let trace_body = trace.text().unwrap();
    assert!(
        trace_body.contains("PoC finding title 1755"),
        "{trace_body}"
    );
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
