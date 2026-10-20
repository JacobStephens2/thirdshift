use super::*;

#[test]
fn console_stop_confirms_authenticates_and_triages_or_requeues_a_live_command() {
    for requeue in [false, true] {
        let scenario = support::Scenario::new();
        let home = scenario.path("home");
        scenario.user_config_is(&format!(
            "[console]\nport = 0\nallowed_emails = [\"{EMAIL}\"]\n"
        ));
        assert!(password(&home, None, PASSWORD, PASSWORD).0.success());
        scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
        scenario.agent_does(&format!(
            "trap 'while ! test -f {root}/release; do sleep 0.02; done; exit 143' TERM\n\
             echo unfinished > work.txt\ntouch {root}/started\n\
             while ! test -f {root}/release; do sleep 0.05; done\nexit 1\n",
            root = scenario.path("").display()
        ));
        let _held = HeldFixture {
            run: Some(scenario.run_until(&[&scenario.issue_url(7)], &[], "started")),
            release: scenario.path("release"),
        };
        let console =
            Console::start_mode(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &home, false);
        let route = "/stop?command=widgets%237";
        assert_eq!(get(&console, route, "").status(), 303);
        assert_eq!(
            client()
                .post(format!("{}/stop", console.address))
                .send()
                .unwrap()
                .status(),
            303
        );
        let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
        let peek = get(&console, "/peek?command=widgets%237", &session)
            .text()
            .unwrap();
        assert!(peek.contains(">Stop</button>"), "{peek}");
        assert!(peek.contains("thirdshift stop"), "{peek}");
        let confirm = get(&console, route, &session).text().unwrap();
        assert!(confirm.contains("needs-triage"), "{confirm}");
        assert!(confirm.contains("Requeue instead"), "{confirm}");
        let token = csrf(&confirm);
        let post = |token: &str, confirmed: &str| {
            client()
                .post(format!("{}/stop", console.address))
                .header("cookie", &session)
                .form(&[
                    ("command", "widgets#7"),
                    ("csrf", token),
                    ("confirmed", confirmed),
                    ("ending", if requeue { "requeue" } else { "triage" }),
                ])
                .send()
                .unwrap()
        };
        assert_eq!(post("", "yes").status(), 403);
        assert_eq!(post(&token, "").status(), 400);
        assert!(
            !fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
                .unwrap()
                .contains("stopped from")
        );
        let response = post(&token, "yes");
        assert_eq!(response.status(), 303, "{}", response.text().unwrap());
        let ending = get(&console, route, &session).text().unwrap();
        assert!(ending.contains("still ending"), "{ending}");
        assert!(ending.contains("stopping-bar"), "{ending}");
        assert!(!ending.contains("name=\"ending\""), "{ending}");
        let stream = get(&console, "/events", &session);
        let mut stream = BufReader::new(stream);
        fs::write(scenario.path("release"), "").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            assert!(Instant::now() < deadline, "Stop outcome never arrived");
            let mut line = String::new();
            assert!(stream.read_line(&mut line).unwrap() > 0);
            if !line.contains("event: commands") {
                continue;
            }
            let page = get(&console, route, &session).text().unwrap();
            if page.contains("still ending") {
                continue;
            }
            assert!(page.contains("interrupted"), "{page}");
            assert!(page.contains("Issue branch left: issue-7"), "{page}");
            assert!(!page.contains("name=\"ending\""), "{page}");
            break;
        }
        assert_eq!(
            scenario.issue_labels(7),
            if requeue {
                vec!["bug", "ready-for-agent"]
            } else {
                vec!["bug", "in-progress", "needs-triage"]
            }
        );
        assert_eq!(
            scenario.origin_file("issue-7", "work.txt"),
            Some("unfinished\n".into())
        );
        let activity =
            fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
                .unwrap();
        assert!(
            activity.contains(&format!("Run #7 stopped from the Console by {EMAIL}")),
            "{activity}"
        );
        assert_eq!(post(&token, "yes").status(), 409);
        let peek = get(&console, "/peek?command=widgets%237", &session)
            .text()
            .unwrap();
        assert!(!peek.contains(">Stop</button>"), "{peek}");
    }
}
