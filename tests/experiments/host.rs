//! Host-wide Experiment isolation through the public CLI and fake Harnesses.
use super::*;

fn hold_trials(fixture: &Fixture) -> support::HeldRun {
    fixture.scenario.agent_does(&format!(
        "case \"$PWD\" in *--baseline--*) {} ;; *) {} ;; esac",
        support::concurrency::waits_on("baseline"),
        support::concurrency::waits_on("candidate")
    ));
    let experiment = fixture.scenario.run_until(
        &Fixture::experiment_args(&[]),
        &NO_SANDBOX,
        "started-baseline",
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !fixture.scenario.path("started-candidate").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "both Trials did not start"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    experiment
}

fn waiting_command(fixture: &Fixture, count: usize) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let status = fixture.scenario.run(&["status", "--json"]);
        assert_eq!(status.code, Some(0), "{}", status.stderr);
        let view: Value = serde_json::from_str(&status.stdout).unwrap();
        if let Some(command) = view["commands"].as_array().unwrap().iter().find(|command| {
            command["kind"] == "Experiment"
                && command["state"] == "live"
                && command["waiting_for"]
                    .as_array()
                    .is_some_and(|names| names.len() == count)
        }) {
            return command.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Experiment did not show {count} waiting Commands: {}",
            status.stdout
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn stop_during_the_host_wait_interrupts_cleanly_and_resume_waits_again() {
    let fixture = Fixture::new();
    fixture.scenario.launch_git(&["checkout", "main"]);
    fixture
        .scenario
        .agent_does_for(7, &support::concurrency::waits_on("ticket"));
    let mut ticket = fixture.scenario.run_until(&["7"], &[], "started-ticket");
    let args = Fixture::experiment_args(&[]);
    let experiment = fixture.scenario.start(&args, &NO_SANDBOX);
    let waiting = waiting_command(&fixture, 1);
    let stopped = fixture
        .scenario
        .run(&["stop", waiting["name"].as_str().unwrap(), "--json"]);
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    let ended = experiment.finish();
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    let view = fixture.experiment();
    assert_eq!(view["state"], "interrupted");
    assert_eq!(view["results"], json!([]));
    assert_eq!(view["waiting_for"], json!([]));
    let mut resumed = fixture.scenario.start(&args, &NO_SANDBOX);
    waiting_command(&fixture, 1);
    assert_eq!(
        fixture.scenario.claude_calls().len(),
        1,
        "a Trial started during either wait"
    );
    resumed.signal("INT");
    assert_eq!(resumed.finish().code, Some(1));
    ticket.signal("TERM");
    ticket.finish();
}

#[test]
fn no_wait_runs_trials_immediately_and_keeps_the_competing_command_count_with_the_verdict() {
    let fixture = Fixture::new();
    fixture.scenario.launch_git(&["checkout", "main"]);
    fixture.scenario.agent_does_for(
        7,
        &format!(
            "case \"$PWD\" in */trials/*) ;; *) {} ;; esac",
            support::concurrency::waits_on("ticket")
        ),
    );
    let mut ticket = fixture.scenario.run_until(&["7"], &[], "started-ticket");
    let result = fixture.run(&["--no-wait"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let view: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(view["commands_at_start"], 1);
    assert_eq!(view["no_wait"], true);
    assert_eq!(view["results"].as_array().unwrap().len(), 2);
    let text = fixture
        .scenario
        .run(&["experiments", view["id"].as_str().unwrap()]);
    assert!(
        text.stdout
            .contains("--no-wait: 1 other Command live at start"),
        "{}",
        text.stdout
    );
    ticket.signal("TERM");
    ticket.finish();
}

#[test]
fn an_experiment_waits_visibly_for_a_live_ticket_and_skips_passes_until_it_ends() {
    let fixture = Fixture::new();
    fixture.scenario.launch_git(&["checkout", "main"]);
    fixture.scenario.agent_does_for(
        7,
        &format!(
            "case \"$PWD\" in */trials/*) {} ;; *) {} ;; esac",
            support::concurrency::waits_on("trial"),
            support::concurrency::waits_on("ticket")
        ),
    );
    let mut ticket = fixture.scenario.run_until(&["7"], &[], "started-ticket");
    fixture
        .scenario
        .agent_does(&support::concurrency::waits_on("trial"));
    fixture
        .scenario
        .agent_does_for(9, &support::concurrency::waits_on("ticket9"));
    fixture.scenario.issue_is(9, "OPEN");
    let mut second = fixture.scenario.run_until(&["9"], &[], "started-ticket9");
    let mut experiment = fixture
        .scenario
        .start(&Fixture::experiment_args(&[]), &NO_SANDBOX);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let waiting = loop {
        let status = fixture.scenario.run(&["status", "--json"]);
        assert_eq!(status.code, Some(0), "{}", status.stderr);
        let status: Value = serde_json::from_str(&status.stdout).unwrap();
        if let Some(command) = status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["kind"] == "Experiment")
        {
            if command["waiting_for"]
                .as_array()
                .is_some_and(|v| v.len() == 2)
            {
                break command.clone();
            }
        }
        assert_eq!(
            fixture.scenario.claude_calls().len(),
            2,
            "a Trial started before the live Commands ended"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "Experiment never showed its wait"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert_eq!(waiting["stage"]["name"], "Waiting for Commands");
    assert!(
        waiting["waiting_for"][0]
            .as_str()
            .unwrap()
            .contains("acme/widgets#7")
    );
    let text = fixture.scenario.run(&["status"]);
    assert!(
        text.stdout.contains("Waiting for Commands") && text.stdout.contains("acme/widgets#7"),
        "{}",
        text.stdout
    );
    let log = fs::read_to_string(waiting["command_log"].as_str().unwrap()).unwrap();
    assert!(
        log.contains("Waiting for Commands") && log.contains("acme/widgets#7"),
        "{log}"
    );
    second.signal("TERM");
    second.finish();
    let refreshed = waiting_command(&fixture, 1);
    assert!(
        refreshed["waiting_for"][0]
            .as_str()
            .unwrap()
            .contains("acme/widgets#7")
    );
    assert_eq!(
        fixture.scenario.claude_calls().len(),
        2,
        "a Trial started while one Command remained"
    );
    let pass = fixture.scenario.run(&["pickup"]);
    assert_eq!(pass.code, Some(0), "{}", pass.stderr);
    assert!(pass.stderr.contains("Experiment"), "{}", pass.stderr);
    ticket.signal("TERM");
    ticket.finish();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !fixture.scenario.path("started-trial").exists() {
        if std::time::Instant::now() >= deadline {
            experiment.signal("TERM");
            let ended = experiment.finish();
            panic!(
                "Trials did not start after the other Command ended: {}",
                ended.stderr
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    experiment.signal("TERM");
    experiment.finish();
}

#[test]
fn a_ticket_started_by_hand_runs_and_warns_about_the_live_experiment() {
    let fixture = Fixture::new();
    fixture.scenario.launch_git(&["checkout", "main"]);
    let mut experiment = hold_trials(&fixture);
    fixture
        .scenario
        .agent_does_for(7, &support::concurrency::waits_on("ticket"));
    let mut ticket = fixture.scenario.run_until(&["7"], &[], "started-ticket");
    ticket.signal("TERM");
    let ended = ticket.finish();
    assert!(
        ended.stderr.contains("warning: Experiment")
            && ended.stderr.contains("will compete with its Trials"),
        "{}",
        ended.stderr
    );
    experiment.signal("TERM");
    experiment.finish();
}

#[test]
fn every_pass_skips_a_live_experiment_on_another_repository_before_github_or_sessions() {
    let fixture = Fixture::new();
    fixture.scenario.launch_git(&["checkout", "main"]);
    let mut experiment = hold_trials(&fixture);
    fs::create_dir_all(fixture.scenario.path("home/.thirdshift/processes")).unwrap();
    fs::write(fixture.scenario.path("home/.thirdshift/processes/host-check.toml"),
        "version = 1\ndescription = \"Check host\"\n[[steps]]\nname = \"check\"\ncommand = \"touch should-not-run\"\n").unwrap();
    let github = fixture.scenario.gh_state();
    let calls = fixture.scenario.gh_calls();
    let sessions = fixture.scenario.claude_calls().len();
    for args in [
        vec!["pickup"],
        vec!["architect"],
        vec!["secure"],
        vec!["review", "--author", "acme"],
        vec!["fix"],
        vec!["triage"],
        vec!["process", "host-check"],
    ] {
        for _ in 0..2 {
            let result = fixture.scenario.run(&args);
            assert_eq!(result.code, Some(0), "{args:?}: {}", result.stderr);
            assert!(
                result.stderr.contains("Experiment")
                    && result.stderr.contains("live on this machine"),
                "{args:?}: {}",
                result.stderr
            );
            assert!(result.stdout.is_empty(), "{args:?}: {}", result.stdout);
        }
    }
    assert_eq!(fixture.scenario.gh_state(), github);
    assert_eq!(fixture.scenario.gh_calls(), calls);
    assert_eq!(fixture.scenario.claude_calls().len(), sessions);
    let activity = fs::read_to_string(
        fixture
            .scenario
            .path("home/.thirdshift/logs/acme/widgets/activity.log"),
    )
    .unwrap();
    assert_eq!(
        activity
            .lines()
            .filter(|line| line.contains("skipped:") && line.contains("Experiment"))
            .count(),
        7,
        "{activity}"
    );
    experiment.signal("TERM");
    experiment.finish();
}
