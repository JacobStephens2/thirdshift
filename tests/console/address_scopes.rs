use super::*;

#[test]
fn remote_details_name_the_signed_in_person_and_honor_the_owning_allowlist_and_scopes() {
    let caller_home = configured();
    let owner_home = configured();
    let mut security = record(owner_home.path(), 7, Some("failed"), 0);
    security["kind"] = json!("Security run");
    security["security_findings"] = json!([{"title": "Private finding for this person", "url": "https://github.com/acme/widgets/security/advisories/GHSA-test", "severity": "high"}]);
    write_record(owner_home.path(), 7, &security);
    assert!(
        password(caller_home.path(), None, PASSWORD, PASSWORD)
            .0
            .success()
    );
    api::cli(
        owner_home.path(),
        &[
            "console", "scopes", "--email", EMAIL, "--scopes", "read", "--json",
        ],
    );
    let token = tokens::create(owner_home.path(), "watcher", &["--scopes", "read,security"]);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let owner = Console::start(binary, owner_home.path());
    assert!(
        crate::fleet::add(caller_home.path(), "remote", &owner.address, &token)
            .status
            .success()
    );
    let caller = Console::start_mode(binary, caller_home.path(), false);
    let session = cookie(&sign_in(&caller, EMAIL, PASSWORD));
    let path = "/peek?machine=remote&command=widgets%237";
    let denied = get(&caller, path, &session);
    assert_eq!(denied.status(), 403);
    assert!(
        denied
            .text()
            .unwrap()
            .contains("needs the `security` scope on `remote`")
    );
    api::cli(
        owner_home.path(),
        &[
            "console",
            "scopes",
            "--email",
            EMAIL,
            "--scopes",
            "read,security",
            "--json",
        ],
    );
    let allowed = get(&caller, path, &session);
    assert_eq!(allowed.status(), 200);
    assert!(
        allowed
            .text()
            .unwrap()
            .contains("Private finding for this person")
    );
    allow(owner_home.path(), false);
    let denied = get(&caller, path, &session);
    assert_eq!(denied.status(), 403);
    let message = denied.text().unwrap();
    assert!(message.contains("not on this machine"), "{message}");
    assert!(!message.contains("Private finding"));
}

#[test]
fn restricted_sessions_can_sign_out_of_their_own_browser_with_csrf() {
    for scopes in ["read", ""] {
        let home = configured();
        assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
        let console = Console::start_mode(
            Path::new(env!("CARGO_BIN_EXE_thirdshift")),
            home.path(),
            false,
        );
        let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
        api::cli(
            home.path(),
            &["console", "scopes", "--scopes", scopes, "--json"],
        );
        let account = get(&console, "/account", &session);
        assert_eq!(account.status(), 200);
        let page = account.text().unwrap();
        assert!(page.contains("Sign out this browser"));
        assert_eq!(
            page.contains("id=\"host-resources\""),
            scopes == "read",
            "{scopes}: {page}"
        );
        let post = |csrf: &str| {
            client()
                .post(format!("{}/sign-out", console.address))
                .header("cookie", &session)
                .form(&[("csrf", csrf)])
                .send()
                .unwrap()
        };
        assert_eq!(post("wrong").status(), 403);
        let signed_out = post(&csrf(&page));
        assert_eq!(signed_out.status(), 303);
        assert!(
            signed_out.headers()["set-cookie"]
                .to_str()
                .unwrap()
                .contains("Max-Age=0")
        );
        assert_eq!(get(&console, "/api/v1/whoami", &session).status(), 401);
    }
}

#[test]
fn named_person_stop_records_the_person_and_token_holder() {
    let scenario = support::Scenario::new();
    let home = scenario.path("home");
    scenario.user_config_is(&format!(
        "[console]\nport = 0\nallowed_emails = [{EMAIL:?}]\n"
    ));
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(&format!(
        "touch '{}'\nwhile ! test -f '{}'; do sleep 0.05; done\nexit 1\n",
        scenario.path("started").display(),
        scenario.path("release").display()
    ));
    let _held = HeldFixture {
        run: Some(scenario.run_until(&[&scenario.issue_url(7)], &[], "started")),
        release: scenario.path("release"),
    };
    let token = tokens::create(&home, "laptop", &["--scopes", "act"]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &home);
    let stopped = api::json(
        client()
            .post(format!("{}/api/v1/stop", console.address))
            .bearer_auth(token)
            .header("x-thirdshift-person", EMAIL)
            .form(&[("command", "widgets#7")])
            .send()
            .unwrap(),
    );
    assert_eq!(stopped["outcome"], "interrupted");
    let activity =
        fs::read_to_string(home.join(".thirdshift/logs/acme/widgets/activity.log")).unwrap();
    assert!(
        activity.contains(&format!("stopped by {EMAIL} from the Console on laptop")),
        "{activity}"
    );
}

#[test]
fn act_without_security_cannot_read_a_security_run_from_the_stop_page() {
    let home = configured();
    let mut security = record(home.path(), 2, Some("findings recorded"), 0);
    security["kind"] = json!("Security run");
    security["name"] = json!("secure-fixture");
    security["issue"] = Value::Null;
    security["security_findings"] = json!([{
        "title": "stop page private finding",
        "severity": "high",
        "url": "https://github.com/acme/widgets/security/advisories/GHSA-stop"
    }]);
    security["security_fix_findings"] = json!([{
        "title": "stop page private fix",
        "severity": "high",
        "url": "https://github.com/acme/widgets/security/advisories/GHSA-stop-fix"
    }]);
    write_record(home.path(), 2, &security);
    api::cli(
        home.path(),
        &["console", "scopes", "--scopes", "read,act", "--json"],
    );
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let identity = api::json(get(&console, "/api/v1/whoami", &session));
    assert_eq!(identity["scopes"], json!(["read", "act"]));
    record(home.path(), 7, Some("failed"), 0);
    let ordinary = get(&console, "/stop?command=widgets%237", &session);
    let ordinary_status = ordinary.status();
    let ordinary_body = ordinary.text().unwrap();
    assert_eq!(ordinary_status, 200, "{ordinary_body}");
    assert!(!ordinary_body.contains("Missing scope"), "{ordinary_body}");
    let page = get(&console, "/stop?command=secure-fixture", &session);
    let status = page.status();
    let body = page.text().unwrap();
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("security"), "{body}");
    assert!(!body.contains("stop page private finding"), "{body}");
    assert!(!body.contains("stop page private fix"), "{body}");
    api::cli(
        home.path(),
        &[
            "console",
            "scopes",
            "--scopes",
            "read,act,security",
            "--json",
        ],
    );
    let allowed = get(&console, "/stop?command=secure-fixture", &session);
    let allowed_status = allowed.status();
    let allowed_body = allowed.text().unwrap();
    assert_eq!(allowed_status, 200, "{allowed_body}");
    assert!(
        allowed_body.contains("stop page private finding"),
        "{allowed_body}"
    );
}

#[test]
fn read_only_session_refuses_security_details_and_log_streams() {
    let home = configured();
    let mut security = record(home.path(), 2, Some("findings recorded"), 0);
    let trace = home.path().join(".thirdshift/logs/acme/widgets/commands/issue/2-20261105T120000-0500-security-audit.logs.jsonl");
    support::session_trace::write(&trace, "private finding marker\n");
    security["kind"] = json!("Security run");
    security["name"] = json!("secure-fixture");
    security["issue"] = Value::Null;
    security["session_log"] = json!(trace);
    security["security_findings"] = json!([{"title": "private finding marker", "severity": "high", "url": "https://github.com/acme/widgets/security/advisories/GHSA-test"}]);
    write_record(home.path(), 2, &security);
    api::cli(
        home.path(),
        &["console", "scopes", "--scopes", "read", "--json"],
    );
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let logs = api::json(get(&console, "/api/v1/logs", &session));
    let id = logs["logs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|log| log["name"].as_str().unwrap().contains("security-audit"))
        .unwrap()["id"]
        .as_str()
        .unwrap();
    for route in [
        "/security".to_owned(),
        "/?page=security".into(),
        "/peek?command=secure-fixture".into(),
        "/waterfall?command=secure-fixture".into(),
        "/spans?command=secure-fixture".into(),
        "/log?command=secure-fixture&session=security-audit".into(),
        format!("/logs/{id}"),
        format!("/logs/{id}/stream/0"),
    ] {
        let response = get(&console, &route, &session);
        assert_eq!(response.status(), 403, "{route}");
        assert!(response.text().unwrap().contains("security"), "{route}");
    }
    for route in ["/", "/history"] {
        let page = get(&console, route, &session).text().unwrap();
        assert!(!page.contains("private finding marker"), "{route}: {page}");
    }
}

#[test]
fn http_cannot_write_or_show_address_scopes_even_with_admin() {
    let home = configured();
    api::cli(
        home.path(),
        &["console", "scopes", "--scopes", "read", "--json"],
    );
    let path = home.path().join(".thirdshift/config.toml");
    let before: toml::Table = fs::read_to_string(&path).unwrap().parse().unwrap();
    let token = tokens::create(
        home.path(),
        "admin",
        &["--scopes", "read,security,act,run,admin"],
    );
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for (route, form) in [
        ("/api/v1/theme", vec![("theme", "press-dark")]),
        ("/api/v1/sections", vec![("hide", "done")]),
    ] {
        let response = api::json(actions::post(&console, &token, route, &form));
        assert!(
            response.get("address_scopes").is_none(),
            "{route}: {response}"
        );
    }
    let document = api::json(
        tokens::request(&console, &token, "/api/v1/openapi.json")
            .send()
            .unwrap(),
    );
    for (route, methods) in document["paths"].as_object().unwrap() {
        for operation in methods.as_object().unwrap().values() {
            for twin in operation["x-cli-twins"].as_array().unwrap() {
                let twin = twin.as_str().unwrap();
                assert!(
                    !twin.starts_with("thirdshift console scopes")
                        && !twin.contains("allowed_emails"),
                    "{route}: {twin}"
                );
            }
        }
        if methods.get("post").is_none() {
            continue;
        }
        let route = route
            .replace("{suite}", "missing")
            .replace("{task}", "missing");
        let response = client()
            .post(format!("{}{route}", console.address))
            .bearer_auth(&token)
            .form(&[
                ("theme", "press-dark"),
                ("hide", "done"),
                ("command", "missing#404"),
                ("allowed_emails", "stranger@example.test"),
                ("address_scopes", "admin"),
                ("email", EMAIL),
                ("scopes", "admin"),
            ])
            .send()
            .unwrap();
        assert!(
            !response.text().unwrap().contains("address_scopes"),
            "{route}"
        );
    }
    // The config read now exists, but there is no bulk config write.
    assert_eq!(
        actions::post(
            &console,
            &token,
            "/api/v1/config",
            &[("address_scopes", "admin")]
        )
        .status(),
        405
    );
    for (key, value) in [
        ("console.allowed_emails", "['stranger@example.test']"),
        ("console.address_scopes", "{}"),
    ] {
        let response = actions::post(
            &console,
            &token,
            &format!("/api/v1/config/{key}"),
            &[("value", value)],
        );
        assert_eq!(response.status(), 422, "{key}");
        assert!(
            response.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("access")
        );
    }
    for route in [
        "/api/v1/scopes",
        "/api/v1/allowed-emails",
        "/console/scopes",
        "/scopes",
    ] {
        assert_eq!(
            actions::post(
                &console,
                &token,
                route,
                &[("email", EMAIL), ("scopes", "admin")]
            )
            .status(),
            404,
            "{route}"
        );
    }
    let after: toml::Table = fs::read_to_string(&path).unwrap().parse().unwrap();
    for key in ["allowed_emails", "address_scopes"] {
        assert_eq!(after["console"][key], before["console"][key], "{key}");
    }
}

#[test]
fn token_naming_a_person_requires_local_membership_and_both_scopes() {
    let home = configured();
    fs::write(home.path().join(".thirdshift/config.toml"), format!("[console]\nport = 0\nallowed_emails = [{EMAIL:?}, 'reader@example.test', 'actor@example.test']\naddress_scopes = {{ 'reader@example.test' = ['read'], 'actor@example.test' = ['act'] }}\n")).unwrap();
    record(home.path(), 7, Some("failed"), 0);
    let token = tokens::create(home.path(), "laptop", &["--scopes", "read,act"]);
    let reader = tokens::create(home.path(), "read-token", &[]);
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    let post = |token: &str, person: &str, path: &str| {
        client()
            .post(format!("{}{path}", console.address))
            .bearer_auth(token)
            .header("x-thirdshift-person", person)
            .form(&[("command", "widgets#7")])
            .send()
            .unwrap()
    };
    let refused = post(&token, "stranger@example.test", "/api/v1/dismiss");
    assert_eq!(refused.status(), 403);
    assert!(
        refused.json::<Value>().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("sign-in list")
    );
    for (token, person) in [
        (&*token, "reader@example.test"),
        (&*reader, "actor@example.test"),
    ] {
        let refused = post(token, person, "/api/v1/dismiss");
        assert_eq!(refused.status(), 403);
        assert!(
            refused.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("act")
        );
    }
    let identity = api::json(
        tokens::request(&console, &token, "/api/v1/whoami")
            .header("x-thirdshift-person", EMAIL)
            .send()
            .unwrap(),
    );
    assert_eq!(
        identity["identity"],
        json!({"kind": "token", "holder": "laptop", "person": EMAIL})
    );
    assert_eq!(identity["scopes"], json!(["read", "act"]));
    let dismissed = api::json(post(&token, "actor@example.test", "/api/v1/dismiss"));
    assert_eq!(dismissed["dismissal"]["who"], "actor@example.test");
    api::json(post(&token, "actor@example.test", "/api/v1/dismiss/undo"));
    let activity = fs::read_to_string(
        home.path()
            .join(".thirdshift/logs/acme/widgets/activity.log"),
    )
    .unwrap();
    assert!(
        activity.contains("dismissed by actor@example.test from the Console on laptop"),
        "{activity}"
    );
    assert!(
        activity.contains("dismissal taken back by actor@example.test from the Console on laptop"),
        "{activity}"
    );
    for person in ["", "actor@example.test,reader@example.test"] {
        assert_eq!(post(&token, person, "/api/v1/dismiss").status(), 403);
    }
    assert_eq!(
        client()
            .post(format!("{}/api/v1/dismiss", console.address))
            .bearer_auth(&token)
            .header("x-thirdshift-person", "actor@example.test")
            .header("x-thirdshift-person", EMAIL)
            .form(&[("command", "widgets#7")])
            .send()
            .unwrap()
            .status(),
        403
    );
    // Removal from the owner's sign-in list takes effect without a restart.
    allow(home.path(), true);
    assert_eq!(
        post(&token, "actor@example.test", "/api/v1/dismiss").status(),
        403
    );
}

#[test]
fn read_only_session_hides_actions_and_refuses_page_and_api_mutations() {
    let home = configured();
    let mut live = record(home.path(), 7, None, 0);
    let stat = fs::read_to_string(format!("/proc/{}/stat", std::process::id())).unwrap();
    let ticks = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    live["process_id"] = json!(std::process::id());
    live["process_start"] = json!(format!("{}:{ticks}", boot.trim()));
    write_record(home.path(), 7, &live);
    record(home.path(), 8, Some("failed"), 0);
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let look: Value = get(&console, "/look", &session).json().unwrap();
    assert!(
        get(&console, "/peek?command=widgets%237", &session)
            .text()
            .unwrap()
            .contains("action=\"/stop\"")
    );
    api::cli(
        home.path(),
        &["console", "scopes", "--scopes", "read", "--json"],
    );
    let identity = api::json(get(&console, "/api/v1/whoami", &session));
    assert_eq!(identity["scopes"], json!(["read"]));
    let page = get(&console, "/", &session).text().unwrap();
    assert!(!page.contains("action=\"/dismiss\""), "{page}");
    assert!(!page.contains("data-open-themes") && !page.contains("data-open-sections"));
    let peek = get(&console, "/peek?command=widgets%237", &session)
        .text()
        .unwrap();
    assert!(!peek.contains("action=\"/stop\""), "{peek}");
    for path in [
        "/stop",
        "/dismiss",
        "/dismiss/undo",
        "/api/v1/stop",
        "/api/v1/dismiss",
        "/api/v1/dismiss/undo",
        "/theme",
        "/sections",
    ] {
        let response = client()
            .post(format!("{}{path}", console.address))
            .header("cookie", &session)
            .header("x-csrf-token", look["csrf"].as_str().unwrap())
            .form(&[("command", "widgets#8")])
            .send()
            .unwrap();
        assert_eq!(response.status(), 403, "{path}");
        assert!(response.text().unwrap().contains("act"), "{path}");
    }
    // A session cannot substitute another address with the bearer-only header.
    assert_eq!(
        api::json(
            client()
                .get(format!("{}/api/v1/whoami", console.address))
                .header("cookie", &session)
                .header("x-thirdshift-person", "stranger@example.test")
                .send()
                .unwrap()
        )["identity"]["address"],
        EMAIL
    );
}

#[test]
fn cli_sets_and_shows_address_scopes_without_changing_other_settings() {
    let home = configured();
    let path = home.path().join(".thirdshift/config.toml");
    fs::write(&path, format!("[console] # Console settings\nport = 0 # free port\nallowed_emails = [{EMAIL:?}] # sign-in list\ntheme = 'press-dark'\n")).unwrap();
    let all = api::cli(
        home.path(),
        &["console", "scopes", "--email", EMAIL, "--json"],
    );
    assert_eq!(
        all,
        json!({"email": EMAIL, "scopes": ["read", "security", "act", "run", "admin"]})
    );
    assert_eq!(
        api::cli(
            home.path(),
            &[
                "console", "scopes", "--email", EMAIL, "--scopes", "read,act", "--json"
            ]
        ),
        json!({"email": EMAIL, "scopes": ["read", "act"]})
    );
    let shown = cli(home.path(), &["console", "scopes", "--email", EMAIL]);
    assert!(shown.status.success());
    assert_eq!(
        String::from_utf8(shown.stdout).unwrap().trim(),
        format!("{EMAIL}: read,act")
    );
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# sign-in list") && text.contains("# free port"),
        "{text}"
    );
    let config: toml::Table = text.parse().unwrap();
    assert_eq!(
        config["console"]["address_scopes"][EMAIL]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["read", "act"]
    );
    assert_eq!(config["console"]["theme"].as_str(), Some("press-dark"));
    for args in [
        vec!["console", "scopes", "--email", EMAIL, "--scopes", "unknown"],
        vec![
            "console",
            "scopes",
            "--email",
            "stranger@example.test",
            "--scopes",
            "act",
        ],
    ] {
        assert!(!cli(home.path(), &args).status.success());
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
    }
    assert_eq!(
        api::cli(
            home.path(),
            &["console", "scopes", "--scopes", "", "--json"]
        ),
        json!({"email": EMAIL, "scopes": []})
    );
}
