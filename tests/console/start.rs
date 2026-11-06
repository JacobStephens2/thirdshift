use super::*;
use std::time::Instant;

const RECORDS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";

fn fixture() -> support::Scenario {
    let scenario = support::Scenario::new();
    scenario.user_config_is("[console]\nport = 0\n");
    support::write_executable(
        &scenario.path("bin/crontab"),
        "#!/bin/sh\ncat \"$HOME/crontab\"\n",
    );
    fs::write(
        scenario.path("home/crontab"),
        format!(
            "*/5 * * * * cd '{}' && thirdshift pickup >> ~/.thirdshift/logs/cron.log 2>&1\n",
            scenario.launch_dir().display()
        ),
    )
    .unwrap();
    scenario
}

#[test]
fn launch_directories_api_and_cli_agree_on_cron_repositories() {
    let scenario = fixture();
    let output = scenario.run(&["launch-directories", "--json"]);
    assert_eq!(output.code, Some(0), "{}", output.stderr);
    let expected = json!({"directories": [{
        "repository": "acme/widgets", "path": scenario.launch_dir()
    }]});
    assert_eq!(
        serde_json::from_str::<Value>(&output.stdout).unwrap(),
        expected
    );
    let console = Console::start_command(scenario.command(&["console", "--no-sign-in"]));
    assert_eq!(
        api::json(console.get("/api/v1/launch-directories")),
        expected
    );
}

#[test]
fn launch_directories_merge_user_config_and_never_evaluate_cron_text() {
    let scenario = fixture();
    std::os::unix::fs::symlink(scenario.launch_dir(), scenario.path("home/repo")).unwrap();
    scenario.user_config_is("launch_directories = [\"~/repo\"]\n[console]\nport = 0\n");
    fs::write(
        scenario.path("home/crontab"),
        format!(
            "*/5 * * * * cd '{}' && thirdshift pickup\n\
         */5 * * * * cd $(touch {}/executed) && thirdshift pickup\n",
            scenario.launch_dir().display(),
            scenario.path("").display()
        ),
    )
    .unwrap();
    let expected = json!({"directories": [{
        "repository": "acme/widgets", "path": scenario.launch_dir()
    }]});
    for cron in [true, false] {
        if !cron {
            fs::write(scenario.path("home/crontab"), "").unwrap();
        }
        let output = scenario.run(&["launch-directories", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        assert_eq!(
            serde_json::from_str::<Value>(&output.stdout).unwrap(),
            expected
        );
    }
    assert!(!scenario.path("executed").exists());
}

fn server(scenario: &support::Scenario) -> Console {
    let shell = scenario.path("login-shell");
    support::write_executable(
        &shell,
        "#!/bin/sh\n[ \"$1\" = -l ] || exit 91\nshift\n. \"$HOME/.profile\"\nexec /bin/sh \"$@\"\n",
    );
    fs::write(
        scenario.path("home/.profile"),
        "export LOGIN_MARKER=from-profile\n",
    )
    .unwrap();
    let mut command = scenario.command(&["console", "--no-sign-in"]);
    command.env("SHELL", shell);
    Console::start_command(command)
}

fn post(console: &Console, token: &str, body: &Value) -> reqwest::blocking::Response {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .unwrap()
        .post(format!("{}/api/v1/runs", console.address))
        .bearer_auth(token)
        .json(body)
        .send()
        .unwrap()
}

struct Release<'a>(&'a support::Scenario);
impl Drop for Release<'_> {
    fn drop(&mut self) {
        fs::write(self.0.path("release"), "").unwrap();
        if self.0.path("started").exists() {
            wait_for(&self.0.path("completed"));
        }
        if self.0.path(RECORDS).exists() {
            let _ = self.0.run(&["stop", "acme/widgets#7", "--json"]);
        }
    }
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "never appeared: {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn run_token_starts_an_attributed_ticket_that_survives_console_restart() {
    let scenario = fixture();
    // The CLI checks an explicitly named Model before it records the Command.
    scenario.agent_does_in_session(1, "exit 0\n");
    scenario.agent_waits_for_release(&format!(
        "test \"$LOGIN_MARKER\" = from-profile\necho detached-completed > '{}'\nexit 1\n",
        scenario.path("completed").display()
    ));
    let _release = Release(&scenario);
    let token = tokens::create(&scenario.path("home"), "laptop", &["--scopes", "run"]);
    let console = server(&scenario);
    let response = post(
        &console,
        &token,
        &json!({
            "repository":"acme/widgets", "issue":7, "merge":false,
            "email":{"mode":"never"}, "base_fix":false,
            "harness":"claude", "model":"opus", "effort":"high",
            "security_review":false, "security_fix":false
        }),
    );
    let response_status = response.status();
    let response_body = response.text().unwrap();
    assert_eq!(
        response_status,
        200,
        "{response_body}\ncron: {}\nrecords: {}",
        fs::read_to_string(scenario.path("home/.thirdshift/logs/cron.log")).unwrap_or_default(),
        if scenario.path(RECORDS).exists() {
            scenario.command_record(RECORDS).to_string()
        } else {
            "none".into()
        }
    );
    let response: Value = serde_json::from_str(&response_body).unwrap();
    assert_eq!(response, json!({"version":1,"name":"acme/widgets#7"}));
    let record = scenario.command_record(RECORDS);
    assert_eq!(record["started_by"]["who"], "token laptop");
    assert_eq!(record["model"], "opus");
    assert_eq!(record["effort"], "high");
    let pid = record["process_id"].as_u64().unwrap() as libc::pid_t;
    // SAFETY: getsid only reads the session ID of this fixture's process.
    assert_eq!(unsafe { libc::getsid(pid) }, pid);
    wait_for(&scenario.path("started"));
    let activity =
        fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(
        activity.contains("from the Console by token laptop"),
        "{activity}"
    );
    drop(console);
    let restarted = server(&scenario);
    let status = api::json(restarted.get("/api/v1/status"));
    assert_eq!(status["active_runs"], 1, "{status}");
    fs::write(scenario.path("release"), "").unwrap();
    wait_for(&scenario.path("completed"));
    let events = api::cli(&scenario.path("home"), &["console", "log", "--json"]);
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event"] == "api-call"
                && event["email"] == "laptop"
                && event["endpoint"] == "/api/v1/runs"
                && event["outcome"] == 200)
    );
    assert!(
        fs::read_to_string(scenario.path("home/.thirdshift/logs/cron.log"))
            .unwrap()
            .contains("starting on")
    );
}

#[test]
fn only_run_scope_can_start_and_unknown_repositories_start_nothing() {
    let scenario = fixture();
    let console = server(&scenario);
    for scope in ["read", "act"] {
        let token = tokens::create(&scenario.path("home"), scope, &["--scopes", scope]);
        let response = post(
            &console,
            &token,
            &json!({"repository":"acme/widgets","issue":7}),
        );
        assert_eq!(response.status(), 403);
        assert!(
            response.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("run")
        );
    }
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    for body in [
        json!({"repository":"acme/outside","issue":7}),
        json!({"repository":"acme/widgets","issue":"https://github.com/acme/outside/issues/7"}),
        json!({"repository":"acme/widgets","issue":0}),
        json!({"repository":"acme/widgets","issue":7,"harness":"unknown"}),
        json!({"repository":"acme/widgets","issue":7,"path":"/tmp"}),
    ] {
        let response = post(&console, &token, &body);
        assert_eq!(
            response.status(),
            422,
            "{body}: {}",
            response.text().unwrap()
        );
    }
    assert!(!scenario.path(RECORDS).exists());
    assert!(!scenario.path("claude-calls.json").exists());
    let events = api::cli(&scenario.path("home"), &["console", "log", "--json"]);
    assert_eq!(
        events
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["event"] == "api-call")
            .count(),
        7
    );
}

#[test]
fn api_start_appends_to_the_repository_cron_log() {
    let scenario = fixture();
    let log = scenario.path("home/scheduled-output.log");
    fs::write(&log, "existing cron output\n").unwrap();
    fs::write(
        scenario.path("home/crontab"),
        format!(
            "*/5 * * * * cd '{}' && thirdshift pickup >> '{}' 2>&1\n",
            scenario.launch_dir().display(),
            log.display()
        ),
    )
    .unwrap();
    scenario.agent_does("exit 1\n");
    let _release = Release(&scenario);
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = server(&scenario);
    assert_eq!(
        post(
            &console,
            &token,
            &json!({"repository":"acme/widgets","issue":7})
        )
        .status(),
        200
    );
    let output = fs::read_to_string(log).unwrap();
    assert!(output.starts_with("existing cron output\n"));
    assert!(
        output.contains("starting on https://github.com/acme/widgets/issues/7"),
        "{output}"
    );
}

#[test]
fn issue_with_subissues_starts_a_spec_and_names_person_and_machine() {
    let scenario = fixture();
    scenario.user_config_is("[console]\nport = 0\nallowed_emails = [\"owner@example.test\"]\n");
    scenario.spec_has_tickets(7, &[(8, &[])]);
    scenario.agent_waits_for_release(&format!(
        "touch '{}'\nexit 1\n",
        scenario.path("completed").display()
    ));
    let _release = Release(&scenario);
    let token = tokens::create(
        &scenario.path("home"),
        "office-console",
        &["--scopes", "run"],
    );
    let console = server(&scenario);
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .unwrap()
        .post(format!("{}/api/v1/runs", console.address))
        .bearer_auth(token)
        .header("X-Thirdshift-Person", "owner@example.test")
        .json(&json!({"repository":"ACME/WIDGETS","issue":scenario.issue_url(7)}))
        .send()
        .unwrap();
    assert_eq!(api::json(response)["name"], "acme/widgets#7");
    wait_for(&scenario.path("started"));
    let record = scenario.command_record(RECORDS);
    assert_eq!(record["kind"], "Spec run");
    assert_eq!(record["started_by"]["who"], "owner@example.test");
    assert_eq!(record["started_by"]["console_holder"], "office-console");
    let activity =
        fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(
        activity.contains("by owner@example.test from the Console on office-console"),
        "{activity}"
    );
}

#[test]
fn api_ticket_still_runs_when_the_repository_is_at_its_command_limit() {
    let scenario = fixture();
    scenario.issue_is(8, "OPEN");
    scenario.agent_waits_for_release(&format!(
        "touch '{}'\nexit 1\n",
        scenario.path("completed").display()
    ));
    let held = HeldFixture {
        run: Some(scenario.run_until(&[&scenario.issue_url(8)], &[], "started")),
        release: scenario.path("release"),
    };
    let _release = Release(&scenario);
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = server(&scenario);
    assert_eq!(
        api::json(post(
            &console,
            &token,
            &json!({"repository":"acme/widgets","issue":"7"})
        ))["name"],
        "acme/widgets#7"
    );
    let status = api::json(console.get("/api/v1/status"));
    assert_eq!(status["active_runs"], 2, "{status}");
    drop(held);
}

#[test]
fn api_start_does_not_return_an_old_record_after_pid_reuse() {
    let scenario = fixture();
    scenario.agent_does("exit 1\n");
    scenario.run(&[&scenario.issue_url(7)]);
    let record = scenario.path(RECORDS).join(
        scenario
            .entries(RECORDS)
            .into_iter()
            .find(|name| name.ends_with(".command.json"))
            .unwrap(),
    );
    scenario.issue_is(8, "OPEN");
    scenario.agent_does(&format!(
        "touch '{}'\nexit 1\n",
        scenario.path("completed").display()
    ));
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = server(&scenario);
    support::write_executable(
        &scenario.path("login-shell"),
        &format!(
            "#!/bin/sh\nshift\npython3 - \"$$\" '{}' <<'PY'\n\
         import json, pathlib, sys\n\
         path = pathlib.Path(sys.argv[2])\n\
         record = json.loads(path.read_text())\n\
         record['process_id'] = int(sys.argv[1])\n\
         path.write_text(json.dumps(record))\n\
         PY\nexec /bin/sh \"$@\"\n",
            record.display()
        ),
    );
    let response = api::json(post(
        &console,
        &token,
        &json!({"repository":"acme/widgets","issue":8}),
    ));
    wait_for(&scenario.path("completed"));
    let _ = scenario.run(&["stop", "acme/widgets#8", "--json"]);
    assert_eq!(response["name"], "acme/widgets#8");
}
