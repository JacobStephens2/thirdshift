use super::*;

pub(super) fn post(
    console: &Console,
    token: &str,
    path: &str,
    form: &[(&str, &str)],
) -> reqwest::blocking::Response {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .post(format!("{}{path}", console.address))
        .bearer_auth(token)
        .form(form)
        .send()
        .unwrap()
}

#[test]
fn act_scope_dismisses_and_undoes_with_audit_and_token_activity() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 7, Some("failed"), 0);
    fixture["name"] = json!("acme/widgets#7");
    write_record(home.path(), 7, &fixture);
    let actor = tokens::create(home.path(), "operator", &["--scopes", "act"]);
    let reader = tokens::create(home.path(), "reader", &[]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let before = api::cli(home.path(), &["console", "log", "--json"]);
    api::json(
        tokens::request(&console, &reader, "/api/v1/status")
            .send()
            .unwrap(),
    );
    assert_eq!(api::cli(home.path(), &["console", "log", "--json"]), before);
    for path in [
        "/api/v1/stop",
        "/api/v1/dismiss",
        "/api/v1/dismiss/undo",
        "/api/v1/scores/mine",
        "/api/v1/bug-origins/settle",
        "/api/v1/experiments/proposals/code-review/task/approve",
        "/api/v1/theme",
        "/api/v1/sections",
    ] {
        let refused = post(&console, &reader, path, &[]);
        assert_eq!(refused.status(), 403, "{path}");
        assert!(
            refused.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("act")
        );
    }
    let dismissed = api::json(post(
        &console,
        &actor,
        "/api/v1/dismiss",
        &[("command", "widgets#7")],
    ));
    assert_eq!(dismissed["name"], "acme/widgets#7");
    assert_eq!(dismissed["dismissal"]["who"], "token operator");
    let undone = api::json(post(
        &console,
        &actor,
        "/api/v1/dismiss/undo",
        &[("command", "widgets#7")],
    ));
    assert_eq!(
        undone,
        json!({"version":1,"name":"acme/widgets#7","undone":true})
    );
    let refused = post(
        &console,
        &actor,
        "/api/v1/dismiss",
        &[("command", "missing#404")],
    );
    assert_eq!(refused.status(), 422);
    let events = api::cli(home.path(), &["console", "log", "--json"]);
    let calls: Vec<_> = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["event"] == "api-call")
        .collect();
    assert_eq!(
        calls.len(),
        11,
        "one audit event per attempted mutation, no reads: {events}"
    );
    for call in &calls[..8] {
        assert_eq!(call["email"], "reader");
        assert_eq!(call["outcome"], 403);
    }
    assert_eq!(calls[8]["email"], "operator");
    assert_eq!(calls[8]["endpoint"], "/api/v1/dismiss");
    assert_eq!(calls[8]["outcome"], 200);
    assert_eq!(calls[10]["outcome"], 422);
    let text = fs::read_to_string(
        home.path()
            .join(".thirdshift/logs/acme/widgets/activity.log"),
    )
    .unwrap();
    assert!(
        text.contains("dismissed from the Console by token operator"),
        "{text}"
    );
    assert!(
        text.contains("dismissal taken back from the Console by token operator"),
        "{text}"
    );
}

#[test]
fn act_stop_waits_for_cleanup_and_matches_cli_json_for_both_endings() {
    for requeue in [false, true] {
        let scenario = support::Scenario::new();
        let home = scenario.path("home");
        scenario.user_config_is("[console]\nport = 0\n");
        scenario.issue_labelled(7, &["ready-for-agent"]);
        scenario.agent_does(&format!(
            "echo unfinished > work.txt\ntouch '{}'\nwhile ! test -f '{}'; do sleep 0.05; done\nexit 1\n",
            scenario.path("started").display(), scenario.path("release").display()
        ));
        let _held = HeldFixture {
            run: Some(scenario.run_until(&[&scenario.issue_url(7)], &[], "started")),
            release: scenario.path("release"),
        };
        let token = tokens::create(&home, "stop-operator", &["--scopes", "act"]);
        let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &home);
        let stopped = api::json(post(
            &console,
            &token,
            "/api/v1/stop",
            &[
                ("command", "widgets#7"),
                ("requeue", if requeue { "true" } else { "false" }),
            ],
        ));
        assert_eq!(
            stopped,
            json!({"version":1,"name":"acme/widgets#7","outcome":"interrupted",
            "issue_branches":["issue-7"],"pull_requests":[]})
        );
        assert_eq!(
            scenario.issue_labels(7),
            if requeue {
                vec!["ready-for-agent"]
            } else {
                vec!["in-progress", "needs-triage"]
            }
        );
        let activity =
            fs::read_to_string(home.join(".thirdshift/logs/acme/widgets/activity.log")).unwrap();
        assert!(
            activity.contains("stopped from the Console by token stop-operator"),
            "{activity}"
        );
        let events = api::cli(&home, &["console", "log", "--json"]);
        let call = events
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["event"] == "api-call")
            .unwrap();
        assert_eq!(call["email"], "stop-operator");
        assert_eq!(call["endpoint"], "/api/v1/stop");
        assert_eq!(call["outcome"], 200);
    }
}

#[test]
fn act_look_settings_return_the_cli_json_and_refuse_invalid_values() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let token = tokens::create(home.path(), "designer", &["--scopes", "act"]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for (path, form, args) in [
        (
            "/api/v1/theme",
            vec![("theme", "press-dark")],
            vec!["console", "theme", "press-dark", "--json"],
        ),
        (
            "/api/v1/sections",
            vec![("hide", "done,resolved")],
            vec![
                "console", "sections", "--hide", "done", "--hide", "resolved", "--json",
            ],
        ),
        (
            "/api/v1/sections",
            vec![("show", "done")],
            vec!["console", "sections", "--show", "done", "--json"],
        ),
    ] {
        let actual = api::json(post(&console, &token, path, &form));
        assert_eq!(actual, api::cli(home.path(), &args));
    }
    let before = api::cli(home.path(), &["console", "sections", "--json"]);
    for (path, form) in [
        ("/api/v1/theme", vec![("theme", "unknown")]),
        ("/api/v1/sections", vec![("hide", "unknown")]),
    ] {
        let refused = post(&console, &token, path, &form);
        assert_eq!(refused.status(), 422);
        assert!(refused.json::<Value>().unwrap()["error"].is_string());
    }
    assert_eq!(
        before,
        api::cli(home.path(), &["console", "sections", "--json"])
    );
}
