mod support;

use std::fs;
use support::Scenario;

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
