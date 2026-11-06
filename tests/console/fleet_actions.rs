use super::*;

fn start(home: &Path, signed_in: bool) -> Console {
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command.args(if signed_in {
        vec!["console"]
    } else {
        vec!["console", "--no-sign-in"]
    });
    command
        .env("HOME", home)
        .env("THIRDSHIFT_FLEET_INTERVAL_MS", "100")
        .env_remove("RESEND_API_KEY");
    Console::start_command(command)
}

fn pair(caller: &Path, owner: &Path, console: &Console, scopes: &str) {
    let token = tokens::create(owner, "caller", &["--scopes", scopes]);
    assert!(
        super::super::fleet::add(caller, "owner", &console.address, &token)
            .status
            .success()
    );
}

fn post(
    console: &Console,
    session: &str,
    token: &str,
    path: &str,
    command: &str,
) -> reqwest::blocking::Response {
    client()
        .post(format!("{}{path}", console.address))
        .header("cookie", session)
        .form(&[
            ("csrf", token),
            ("command", command),
            ("machine", "owner"),
            ("confirmed", "yes"),
            ("ending", "triage"),
        ])
        .send()
        .unwrap()
}

#[test]
fn signed_in_remote_dismissal_and_undo_keep_activity_on_the_owner() {
    let caller = configured();
    let owner = configured();
    assert!(
        password(caller.path(), None, PASSWORD, PASSWORD)
            .0
            .success()
    );
    let mut fixture = record(owner.path(), 7, Some("failed"), 0);
    fixture["name"] = json!("acme/widgets#7");
    write_record(owner.path(), 7, &fixture);
    let owner_console = start(owner.path(), false);
    pair(caller.path(), owner.path(), &owner_console, "read,act");
    let caller_console = start(caller.path(), true);
    let session = cookie(&sign_in(&caller_console, EMAIL, PASSWORD));
    let token = csrf(&get(&caller_console, "/account", &session).text().unwrap());
    let before = api::cli(caller.path(), &["console", "log", "--json"]);
    for (path, group, phrase) in [
        ("/dismiss", "done", "dismissed"),
        ("/dismiss/undo", "needs you", "dismissal taken back"),
    ] {
        let response = post(&caller_console, &session, &token, path, "widgets#7");
        assert_eq!(response.status(), 200, "{}", response.text().unwrap());
        let status = api::cli(owner.path(), &["status", "--local", "--json"]);
        assert_eq!(status["commands"][0]["group"], group, "{status}");
        let activity = fs::read_to_string(
            owner
                .path()
                .join(".thirdshift/logs/acme/widgets/activity.log"),
        )
        .unwrap();
        assert!(
            activity.contains(&format!("{phrase} by {EMAIL} from the Console on caller")),
            "{activity}"
        );
    }
    assert_eq!(
        api::cli(caller.path(), &["console", "log", "--json"]),
        before
    );
    assert!(
        !caller
            .path()
            .join(".thirdshift/logs/acme/widgets/activity.log")
            .exists()
    );
}

#[test]
fn remote_stop_from_console_and_cli_ends_the_owning_command() {
    for from_console in [true, false] {
        let caller = configured();
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
        let owner_console = start(&home, false);
        pair(caller.path(), &home, &owner_console, "read,act");
        if from_console {
            assert!(
                password(caller.path(), None, PASSWORD, PASSWORD)
                    .0
                    .success()
            );
            let caller_console = start(caller.path(), true);
            let session = cookie(&sign_in(&caller_console, EMAIL, PASSWORD));
            let confirm = get(
                &caller_console,
                "/stop?machine=owner&command=widgets%237",
                &session,
            )
            .text()
            .unwrap();
            assert!(
                confirm.contains("Confirm Stop") && confirm.contains("Requeue instead"),
                "{confirm}"
            );
            let response = post(
                &caller_console,
                &session,
                &csrf(&confirm),
                "/stop",
                "widgets#7",
            );
            assert_eq!(response.status(), 200);
            assert!(response.text().unwrap().contains("interrupted"));
        } else {
            let answer = api::cli(
                caller.path(),
                &["stop", "widgets#7", "--machine", "owner", "--json"],
            );
            assert_eq!(answer["outcome"], "interrupted");
        }
        assert_eq!(
            api::cli(&home, &["status", "--local", "--json"])["active_runs"],
            0
        );
        let activity =
            fs::read_to_string(home.join(".thirdshift/logs/acme/widgets/activity.log")).unwrap();
        let expected = if from_console {
            format!("stopped by {EMAIL} from the Console on caller")
        } else {
            "stopped from the Console by token caller".into()
        };
        assert!(activity.contains(&expected), "{activity}");
        assert!(
            !caller
                .path()
                .join(".thirdshift/logs/acme/widgets/activity.log")
                .exists()
        );
    }
}

#[test]
fn remote_controls_follow_each_persons_live_scopes_and_forced_posts_are_refused() {
    let caller = configured();
    let owner = configured();
    const READER: &str = "reader@example.test";
    for home in [caller.path(), owner.path()] {
        fs::write(
            home.join(".thirdshift/config.toml"),
            format!("[console]\nport = 0\nallowed_emails = [{EMAIL:?}, {READER:?}]\n"),
        )
        .unwrap();
    }
    assert!(
        password(caller.path(), Some(EMAIL), PASSWORD, PASSWORD)
            .0
            .success()
    );
    assert!(
        password(caller.path(), Some(READER), PASSWORD, PASSWORD)
            .0
            .success()
    );
    api::cli(
        owner.path(),
        &[
            "console", "scopes", "--email", READER, "--scopes", "read", "--json",
        ],
    );
    let mut fixture = record(owner.path(), 7, Some("failed"), 0);
    fixture["name"] = json!("acme/widgets#7");
    write_record(owner.path(), 7, &fixture);
    let owner_console = start(owner.path(), false);
    pair(caller.path(), owner.path(), &owner_console, "read,act");
    let caller_console = start(caller.path(), true);
    let session = cookie(&sign_in(&caller_console, EMAIL, PASSWORD));
    let reader_session = cookie(&sign_in(&caller_console, READER, PASSWORD));
    let _events = client()
        .get(format!("{}/events", caller_console.address))
        .header("cookie", &session)
        .send()
        .unwrap();
    let _reader_events = client()
        .get(format!("{}/events", caller_console.address))
        .header("cookie", &reader_session)
        .send()
        .unwrap();
    let wait_page = |session: &str, controls: bool| {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let html = get(&caller_console, "/", session).text().unwrap();
            if html.contains("data-machine=\"owner\"")
                && html.contains(">Dismiss</button>") == controls
            {
                return html;
            }
            assert!(Instant::now() < deadline, "{html}");
            std::thread::sleep(Duration::from_millis(25));
        }
    };
    let html = wait_page(&session, true);
    wait_page(&reader_session, false);
    // A second person's read-only answer must not overwrite the first person's controls.
    wait_page(&session, true);
    let token = csrf(&html);
    api::cli(
        owner.path(),
        &[
            "console", "scopes", "--email", EMAIL, "--scopes", "read", "--json",
        ],
    );
    wait_page(&session, false);
    for path in ["/stop", "/dismiss", "/dismiss/undo"] {
        let refused = post(&caller_console, &session, &token, path, "widgets#7");
        assert_eq!(refused.status(), 403);
        assert!(refused.text().unwrap().contains("Missing scope: act"));
    }
    allow(owner.path(), false);
    let refused = post(&caller_console, &session, &token, "/dismiss", "widgets#7");
    assert_eq!(refused.status(), 403);
    assert!(refused.text().unwrap().contains("not on this machine"));
    allow(owner.path(), true);
    api::cli(
        owner.path(),
        &["console", "scopes", "--scopes", "read,act", "--json"],
    );
    assert!(
        cli(owner.path(), &["console", "token", "revoke", "caller"])
            .status
            .success()
    );
    pair(caller.path(), owner.path(), &owner_console, "read");
    wait_page(&session, false);
    assert_eq!(
        post(&caller_console, &session, &token, "/dismiss", "widgets#7").status(),
        403
    );
}
