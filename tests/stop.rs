mod support;

use std::fs;
use support::{HeldRun, RunResult, Scenario};

fn stop_and_finish(scenario: &Scenario, mut held: HeldRun, args: &[&str]) -> RunResult {
    let stopped = scenario.run(args);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    stopped
}

#[test]
fn json_stop_reports_the_outcome_and_retained_work_for_both_endings() {
    for requeue in [false, true] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["ready-for-agent"]);
        scenario.agent_does(&format!(
            "echo unfinished > work.txt\ntouch '{}'\nwhile true; do sleep 0.05; done\n",
            scenario.path("started").display()
        ));
        let held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
        let args = if requeue {
            vec!["stop", "widgets#7", "--requeue", "--json"]
        } else {
            vec!["stop", "widgets#7", "--json"]
        };
        let stopped = stop_and_finish(&scenario, held, &args);
        let output: serde_json::Value = serde_json::from_str(&stopped.stdout).unwrap();
        assert_eq!(
            output,
            serde_json::json!({
                "version": 1, "name": "acme/widgets#7", "outcome": "interrupted",
                "issue_branches": ["issue-7"], "pull_requests": []
            })
        );
        assert_eq!(
            scenario.issue_labels(7),
            if requeue {
                vec!["ready-for-agent"]
            } else {
                vec!["in-progress", "needs-triage"]
            }
        );
    }
}

#[test]
fn stopping_a_pass_with_no_issue_ends_it_without_changing_labels() {
    for kind in ["architect", "secure"] {
        for requeue in [false, true] {
            let scenario = Scenario::new();
            scenario.issue_labelled(7, &["bug"]);
            let labels = scenario.gh_state()["labels"].clone();
            scenario.agent_does(&format!(
                "touch '{}'\nwhile true; do sleep 0.05; done\n",
                scenario.path("started").display()
            ));
            let held = scenario.run_until(&[kind], &[], "started");
            let status = scenario.run(&["status", "--json"]);
            let status: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
            let command = &status["commands"][0];
            let name = command["name"].as_str().unwrap();
            let args = if requeue {
                vec!["stop", name, "--requeue"]
            } else {
                vec!["stop", name]
            };
            let stopped = stop_and_finish(&scenario, held, &args);
            assert!(command["issue"].is_null(), "{command}");
            assert_eq!(
                command["stage"]["name"],
                if kind == "architect" {
                    "Architecture review"
                } else {
                    "Security audit"
                }
            );
            assert!(stopped.stdout.contains("interrupted"), "{}", stopped.stdout);
            assert!(!stopped.stdout.contains("left:"), "{}", stopped.stdout);
            assert_eq!(scenario.gh_state()["labels"], labels);
            let activity = fs::read_to_string(
                scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"),
            )
            .unwrap();
            assert_eq!(
                activity.matches("stopped from the CLI by").count(),
                1,
                "{activity}"
            );
        }
    }
}

#[test]
fn stopping_a_pickups_run_triages_or_requeues_the_issue_it_took() {
    for requeue in [false, true] {
        let scenario = Scenario::new();
        scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
        scenario.agent_does(&format!(
            "echo work > work.txt\ngit add work.txt\ngit commit -qm Work\ngit push -q origin HEAD\ntouch '{}'\nwhile true; do sleep 0.05; done\n",
            scenario.path("started").display()
        ));
        let held = scenario.run_until(&["pickup"], &[], "started");
        let args = if requeue {
            vec!["stop", "widgets#7", "--requeue"]
        } else {
            vec!["stop", "widgets#7"]
        };
        let stopped = stop_and_finish(&scenario, held, &args);
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
            Some("work\n".into())
        );
        assert!(
            stopped.stdout.contains("Issue branch left: issue-7"),
            "{}",
            stopped.stdout
        );
    }
}

#[test]
fn stopping_a_spec_ends_both_tickets_preserves_their_work_and_only_labels_the_spec() {
    for (pickup, requeue) in [(false, false), (true, false), (true, true)] {
        let scenario = Scenario::new();
        scenario.spec_has_tickets(20, &[(21, &[]), (22, &[])]);
        scenario.issue_labelled(20, &["ready-for-agent"]);
        // Let TERM handling outlast a cleanup poll, so a fake CLI exiting
        // before its script has handled the signal reliably fails this test.
        for (ticket, sibling) in [(21, 22), (22, 21)] {
            scenario.issue_labelled(ticket, &["bug", "ready-for-agent"]);
            scenario.agent_does_for(
                ticket,
                &format!(
                    "trap 'sleep 0.25; touch {root}/interrupted-{ticket}; exit 143' TERM\n\
                 echo 'half done {ticket}' > work.txt\n\
                 touch {root}/started-{ticket}\n\
                 while ! test -f {root}/started-{sibling}; do sleep 0.02; done\n\
                 touch {root}/both-started\n\
                 while true; do sleep 0.05; done\n",
                    root = scenario.path("").display()
                ),
            );
        }
        let url = scenario.issue_url(20);
        let args = if pickup {
            vec!["pickup"]
        } else {
            vec![url.as_str()]
        };
        let held = scenario.run_until(&args, &[], "both-started");
        let args = if requeue {
            vec!["stop", "widgets#20", "--requeue"]
        } else {
            vec!["stop", "widgets#20"]
        };
        let stopped = stop_and_finish(&scenario, held, &args);
        assert!(stopped.stdout.contains("interrupted"), "{}", stopped.stdout);
        assert_eq!(
            scenario.issue_labels(20),
            if requeue {
                vec!["ready-for-agent"]
            } else {
                vec!["in-progress", "needs-triage"]
            }
        );
        for ticket in [21, 22] {
            assert!(scenario.path(&format!("interrupted-{ticket}")).exists());
            assert_eq!(scenario.issue_labels(ticket), ["bug", "ready-for-agent"]);
            assert_eq!(
                scenario.origin_file(&format!("issue-{ticket}"), "work.txt"),
                Some(format!("half done {ticket}\n"))
            );
            let shown = scenario.run(&["show", &format!("widgets#{ticket}"), "--json"]);
            assert_eq!(shown.code, Some(0), "{}", shown.stderr);
            let shown: serde_json::Value = serde_json::from_str(&shown.stdout).unwrap();
            assert_eq!(shown["command"]["state"], "ended");
            assert_eq!(shown["command"]["outcome"], "interrupted");
            scenario.assert_cleaned_up(&format!("issue-{ticket}"));
        }
        let activity =
            fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
                .unwrap();
        assert_eq!(
            activity.matches("stopped from the CLI by").count(),
            1,
            "{activity}"
        );
        assert!(
            activity.find("stopped from").unwrap() < activity.find("ended:").unwrap(),
            "{activity}"
        );
    }
}

#[test]
fn stop_reports_retained_ticket_branches() {
    let scenario = Scenario::new();
    scenario.spec_has_tickets(20, &[(21, &[])]);
    scenario.agent_does_for(
        21,
        &format!(
            "echo unfinished > work.txt\ntouch '{}'\nwhile true; do sleep 0.05; done\n",
            scenario.path("started").display()
        ),
    );
    let held = scenario.run_until(&[&scenario.issue_url(20)], &[], "started");
    let stopped = stop_and_finish(&scenario, held, &["stop", "widgets#20"]);
    assert_eq!(
        scenario.origin_file("issue-21", "work.txt"),
        Some("unfinished\n".into())
    );
    assert!(
        stopped.stdout.contains("Issue branch left: issue-21"),
        "missing retained Ticket branch: {}",
        stopped.stdout
    );
}

#[test]
fn stopping_a_tickets_issue_is_refused_naming_its_spec_without_sending_a_signal() {
    for pickup in [false, true] {
        let scenario = Scenario::new();
        scenario.spec_has_tickets(20, &[(21, &[])]);
        scenario.issue_labelled(20, &["ready-for-agent"]);
        scenario.agent_does_for(
            21,
            &format!(
                "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\ntouch '{}'\nexit 3\n",
                scenario.path("started").display(),
                scenario.path("release").display(),
                scenario.path("survived").display()
            ),
        );
        let url = scenario.issue_url(20);
        let args = if pickup {
            vec!["pickup"]
        } else {
            vec![url.as_str()]
        };
        let held = scenario.run_until(&args, &[], "started");
        let stopped = scenario.run(&["stop", "widgets#21"]);
        fs::write(scenario.path("release"), "").unwrap();
        held.finish();
        assert_ne!(stopped.code, Some(0), "{}", stopped.stdout);
        assert!(stopped.stderr.contains("Spec run"), "{}", stopped.stderr);
        assert!(
            stopped.stderr.contains("acme/widgets#20"),
            "{}",
            stopped.stderr
        );
        assert!(scenario.path("survived").exists(), "Ticket was signalled");
        let activity =
            fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
                .unwrap();
        assert!(!activity.contains("stopped from"), "{activity}");
    }
}

#[test]
fn stopping_a_run_with_nothing_on_origin_leaves_it_for_triage_and_logs_who_before_ending() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug", "ready-for-agent"]);
    scenario.agent_does(&format!(
        "touch '{}'\nwhile true; do sleep 0.05; done\n",
        scenario.path("started").display()
    ));
    let mut held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
    let stopped = scenario.run(&["stop", "widgets#7"]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    assert!(stopped.stdout.contains("interrupted"), "{}", stopped.stdout);
    assert_eq!(scenario.issue_labels(7), ["bug", "needs-triage"]);
    let activity =
        fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    let who = std::process::Command::new("id")
        .arg("-un")
        .output()
        .unwrap();
    let who = String::from_utf8(who.stdout).unwrap();
    let stopped = activity
        .find(&format!("Run #7 stopped from the CLI by {}", who.trim()))
        .unwrap();
    assert!(
        stopped < activity.find("Run #7 ended:").unwrap(),
        "{activity}"
    );
}

#[test]
fn stopping_after_a_push_keeps_the_claim_and_reports_the_branch_and_pull_request() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(&format!(
        "echo work > work.txt\ngit add work.txt\ngit commit -qm Work\ngit push -q origin HEAD\ngh pr create --base main --head issue-7 --title Work --body 'Closes #7'\ntouch '{}'\nwhile true; do sleep 0.05; done\n",
        scenario.path("started").display()
    ));
    let mut held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
    let stopped = scenario.run(&["stop", &scenario.issue_url(7)]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    assert_eq!(scenario.issue_labels(7), ["in-progress", "needs-triage"]);
    assert!(
        stopped.stdout.contains("Issue branch left: issue-7"),
        "{}",
        stopped.stdout
    );
    assert!(
        stopped
            .stdout
            .contains("https://github.com/acme/widgets/pull/1"),
        "{}",
        stopped.stdout
    );
    assert!(scenario.origin_log("issue-7").is_some());
}

#[test]
fn requeue_releases_the_claim_with_or_without_a_pushed_branch() {
    for pushed in [false, true] {
        let scenario = Scenario::new();
        // A directly invoked issue need not have been ready before its Claim.
        scenario.issue_labelled(7, &["bug"]);
        let work = if pushed {
            "echo work > work.txt\ngit add work.txt\ngit commit -qm Work\ngit push -q origin HEAD\n"
        } else {
            ""
        };
        scenario.agent_does(&format!(
            "{work}touch '{}'\nwhile true; do sleep 0.05; done\n",
            scenario.path("started").display()
        ));
        let mut held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
        let stopped = scenario.run(&["stop", "acme/widgets#7", "--requeue"]);
        if stopped.code != Some(0) {
            held.signal("TERM");
        }
        let ended = held.finish();
        assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
        assert_eq!(ended.code, Some(1), "{}", ended.stderr);
        assert_eq!(
            scenario.issue_labels(7),
            ["bug", "ready-for-agent"],
            "pushed={pushed}"
        );
        assert_eq!(scenario.origin_log("issue-7").is_some(), pushed);
        let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/issue");
        assert_eq!(record["outcome"], "interrupted");
    }
}

#[test]
fn ended_and_died_commands_are_refused_without_signalling_their_process() {
    for state in ["ended", "died"] {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\ntouch '{}'\nexit 3\n",
            scenario.path("started").display(),
            scenario.path("release").display(),
            scenario.path("survived").display()
        ));
        let held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
        let folder = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
        let path = fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().ends_with(".command.json"))
            .unwrap();
        let mut record: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if state == "ended" {
            record["outcome"] = "interrupted".into();
        } else {
            record["process_start"] = "a previous process with this PID".into();
        }
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        let stopped = scenario.run(&["stop", "widgets#7"]);
        fs::write(scenario.path("release"), "").unwrap();
        held.finish();
        assert_ne!(stopped.code, Some(0));
        assert!(stopped.stderr.contains(state), "{}", stopped.stderr);
        assert!(
            scenario.path("survived").exists(),
            "unrelated process was signalled"
        );
        assert!(!fs::read_dir(folder).unwrap().any(|entry| {
            entry
                .unwrap()
                .path()
                .to_string_lossy()
                .ends_with(".stop.json")
        }));
    }
}
