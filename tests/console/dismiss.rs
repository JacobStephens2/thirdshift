use super::*;

#[test]
fn dismissing_a_silent_command_keeps_it_running_and_offers_stop_and_undo() {
    use crate::watching::{FixtureProcess, live_record};

    let home = configured();
    let mut work = FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
    let mut fixture = live_record(home.path(), 7, work.0.id());
    let began = (chrono::Local::now() - chrono::Duration::minutes(20)).to_rfc3339();
    fixture["started_at"] = json!(began);
    fixture["stages"][0]["began_at"] = json!(began);
    write_record(home.path(), 7, &fixture);
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let peek = "/peek?command=widgets%237";
    let details = get(&console, peek, &session).text().unwrap();
    assert!(details.contains(">Stop</button>"), "{details}");
    assert!(details.contains(">Dismiss</button>"), "{details}");
    let token = csrf(&details);
    for (route, button) in [("/dismiss", "Undo"), ("/dismiss/undo", "Dismiss")] {
        assert_eq!(
            client()
                .post(format!("{}{route}", console.address))
                .header("cookie", &session)
                .form(&[("command", "widgets#7"), ("csrf", token.as_str())])
                .send()
                .unwrap()
                .status(),
            303
        );
        let details = get(&console, peek, &session).text().unwrap();
        assert!(details.contains(">Stop</button>"), "{details}");
        assert!(
            details.contains(&format!(">{button}</button>")),
            "{details}"
        );
        assert!(work.0.try_wait().unwrap().is_none());
    }
}

#[test]
fn console_dismiss_and_undo_refuse_missing_sessions_bad_csrf_and_gets() {
    let home = configured();
    record(home.path(), 7, Some("failed"), 0);
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let peek = "/peek?command=widgets%237";
    let activity_path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/activity.log");
    for (route, button) in [("/dismiss", "Dismiss"), ("/dismiss/undo", "Undo")] {
        let details = get(&console, peek, &session).text().unwrap();
        let token = csrf(&details);
        let before = fs::read_to_string(&activity_path).unwrap_or_default();
        for (cookie, csrf, expected) in [
            ("", token.as_str(), 303),
            (session.as_str(), "wrong", 403),
            (session.as_str(), "", 403),
        ] {
            let response = client()
                .post(format!("{}{route}", console.address))
                .header("cookie", cookie)
                .form(&[("command", "widgets#7"), ("csrf", csrf)])
                .send()
                .unwrap();
            assert_eq!(response.status(), expected);
            let details = get(&console, peek, &session).text().unwrap();
            assert!(
                details.contains(&format!(">{button}</button>")),
                "{details}"
            );
            assert_eq!(
                fs::read_to_string(&activity_path).unwrap_or_default(),
                before
            );
        }
        assert_eq!(
            get(&console, &format!("{route}?command=widgets%237"), &session).status(),
            405
        );
        assert_eq!(
            client()
                .post(format!("{}{route}", console.address))
                .header("cookie", &session)
                .form(&[("command", "widgets#7"), ("csrf", token.as_str())])
                .send()
                .unwrap()
                .status(),
            303
        );
    }
}

#[test]
fn console_dismiss_is_unavailable_for_commands_that_are_not_waiting() {
    let home = configured();
    record(home.path(), 7, Some("merged"), 0);
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let details = get(&console, "/peek?command=widgets%237", &session)
        .text()
        .unwrap();
    assert!(!details.contains(">Dismiss</button>"), "{details}");
    assert!(!details.contains(">Undo</button>"), "{details}");
    let token = csrf(&get(&console, "/account", &session).text().unwrap());
    for route in ["/dismiss", "/dismiss/undo"] {
        assert_eq!(
            client()
                .post(format!("{}{route}", console.address))
                .header("cookie", &session)
                .form(&[("command", "widgets#7"), ("csrf", token.as_str())])
                .send()
                .unwrap()
                .status(),
            409
        );
    }
    assert!(
        !home
            .path()
            .join(".thirdshift/logs/acme/widgets/activity.log")
            .exists()
    );
}

#[test]
fn console_dismiss_availability_follows_the_kept_github_answer() {
    let scenario = support::Scenario::new();
    let home = scenario.path("home");
    configure(&home);
    allow(&home, true);
    record(&home, 7, Some("ready for review"), 2);
    let mut open = record(&home, 8, Some("ready for review"), 30);
    open["pull_request"] = json!("https://github.com/acme/widgets/pull/11");
    write_record(&home, 8, &open);
    let merged = (chrono::Local::now() - chrono::Duration::hours(1)).to_rfc3339();
    let mut gh = scenario.gh_state();
    gh["prs"] = json!([
        {"number": 10, "url": "https://github.com/acme/widgets/pull/10", "state": "MERGED", "mergedAt": merged},
        {"number": 11, "url": "https://github.com/acme/widgets/pull/11", "state": "OPEN"}
    ]);
    scenario.write_gh_state(&gh);
    let status = scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    assert!(password(&home, None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &home, false);
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    for (number, available) in [(7, false), (8, true)] {
        let details = get(
            &console,
            &format!("/peek?command=widgets%23{number}"),
            &session,
        )
        .text()
        .unwrap();
        assert_eq!(
            details.contains(">Dismiss</button>"),
            available,
            "{details}"
        );
        let token = csrf(&get(&console, "/account", &session).text().unwrap());
        let response = client()
            .post(format!("{}/dismiss", console.address))
            .header("cookie", &session)
            .form(&[("command", format!("widgets#{number}")), ("csrf", token)])
            .send()
            .unwrap();
        assert_eq!(response.status(), if available { 303 } else { 409 });
    }
    let details = get(&console, "/peek?command=widgets%238", &session)
        .text()
        .unwrap();
    assert!(details.contains(">Undo</button>"), "{details}");
}

#[test]
fn console_dismiss_and_undo_update_open_pages_and_name_the_account() {
    for outcome in ["failed", "ready for review"] {
        let home = configured();
        record(home.path(), 7, Some(outcome), 2);
        assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
        let console = Console::start_mode(
            Path::new(env!("CARGO_BIN_EXE_thirdshift")),
            home.path(),
            false,
        );
        let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
        let peek = "/peek?command=widgets%237";
        let group = if outcome == "failed" {
            "needs you"
        } else {
            "ready for review"
        };
        for (route, button, next_button, expected_group) in [
            ("/dismiss", "Dismiss", "Undo", "resolved"),
            ("/dismiss/undo", "Undo", "Dismiss", group),
        ] {
            let details = get(&console, peek, &session).text().unwrap();
            assert!(
                details.contains(&format!(">{button}</button>")),
                "{details}"
            );
            assert!(
                details.contains(&format!("action=\"{route}\"")),
                "{details}"
            );
            assert!(details.contains("thirdshift dismiss"), "{details}");
            assert_eq!(details.contains("--undo"), button == "Undo", "{details}");
            let token = csrf(&details);
            // Two already-open pages must each receive the ordinary Command notice.
            let mut pages = (0..2)
                .map(|_| {
                    let mut stream = BufReader::new(get(&console, "/events", &session));
                    assert!(sse_event(&mut stream).contains("event: commands"));
                    stream
                })
                .collect::<Vec<_>>();
            let response = client()
                .post(format!("{}{route}", console.address))
                .header("cookie", &session)
                .form(&[("command", "widgets#7"), ("csrf", token.as_str())])
                .send()
                .unwrap();
            assert_eq!(response.status(), 303, "{}", response.text().unwrap());
            assert_eq!(response.headers()["location"], "/");
            for stream in &mut pages {
                assert!(sse_event(stream).contains("event: commands"));
                let page = get(&console, "/", &session).text().unwrap();
                let section = page
                    .split(&format!("data-group=\"{expected_group}\""))
                    .nth(1)
                    .expect(&page)
                    .split("</details>")
                    .next()
                    .unwrap();
                assert!(section.contains("widgets#7"), "{page}");
            }
            let details = get(&console, peek, &session).text().unwrap();
            assert!(
                details.contains(&format!(">{next_button}</button>")),
                "{details}"
            );
            assert!(
                !details.contains(&format!(">{button}</button>")),
                "{details}"
            );
        }
        let activity = fs::read_to_string(
            home.path()
                .join(".thirdshift/logs/acme/widgets/activity.log"),
        )
        .unwrap();
        assert!(
            activity.contains(&format!(
                "Spec run #7 dismissed from the Console by {EMAIL}"
            )),
            "{activity}"
        );
        assert!(
            activity.contains(&format!(
                "Spec run #7 dismissal taken back from the Console by {EMAIL}"
            )),
            "{activity}"
        );
    }
}
