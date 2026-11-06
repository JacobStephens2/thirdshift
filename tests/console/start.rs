use super::*;
use std::time::Instant;

const RECORDS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";

#[test]
fn api_experiment_release_skip_and_repository_verification_use_known_launch_directories() {
    let scenario = fixture();
    let runner = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = Console::start_command(scenario.command(&["console", "--no-sign-in"]));
    let result = api::json(post(
        &console,
        &runner,
        &json!({"repository":"acme/widgets", "command":"experiment", "experiment":{"candidate":"HEAD", "baseline":"HEAD", "release":true}}),
    ));
    assert_eq!(result["outcome"], "skipped");
    assert!(
        result["reason"]
            .as_str()
            .unwrap()
            .contains("no Eval suite touched")
    );
    let invalid = post(
        &console,
        &runner,
        &json!({"repository":"acme/widgets", "command":"experiment", "experiment":{"file":"/etc/passwd"}}),
    );
    assert_eq!(invalid.status(), 422);
    assert!(
        invalid
            .text()
            .unwrap()
            .contains("unknown Experiment option")
    );
    let base = Command::new("git")
        .current_dir(scenario.launch_dir())
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(base.status.success());
    let base = String::from_utf8(base.stdout).unwrap();
    let client = reqwest::blocking::Client::new();
    let verify = |repository| {
        client
            .post(format!("{}/api/v1/evals/verify", console.address))
            .json(&json!({"repository":repository,"base":base.trim()}))
            .send()
            .unwrap()
    };
    assert_eq!(api::json(verify("acme/widgets")), json!({"safe":true}));
    assert_eq!(verify("/etc").status(), 422);
}

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
fn launch_directories_understand_adjacent_shell_operators() {
    let scenario = fixture();
    let expected = json!({"directories": [{
        "repository": "acme/widgets", "path": scenario.launch_dir()
    }]});
    for separator in ["&& thirdshift", "&&thirdshift"] {
        fs::write(
            scenario.path("home/crontab"),
            format!(
                "*/5 * * * * cd '{}'{} pickup\n",
                scenario.launch_dir().display(),
                separator
            ),
        )
        .unwrap();
        let output = scenario.run(&["launch-directories", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        assert_eq!(
            serde_json::from_str::<Value>(&output.stdout).unwrap(),
            expected
        );
    }
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
    fs::write(
        scenario.path("home/crontab"),
        format!(
            "*/5 * * * * cd '{}' && thirdshift pickup 2>&1\n",
            scenario.launch_dir().display()
        ),
    )
    .unwrap();
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
            "*/5 * * * * cd '{}' && thirdshift pickup >>'{}' 2>&1\n",
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
fn api_pickup_at_the_command_limit_returns_crons_skip_and_activity() {
    let scenario = fixture();
    scenario.agent_waits_for_release("exit 1\n");
    let held = HeldFixture {
        run: Some(scenario.run_until(&[&scenario.issue_url(7)], &[], "started")),
        release: scenario.path("release"),
    };
    let cli = scenario.run(&["pickup"]);
    assert_eq!(cli.code, Some(0), "{}", cli.stderr);
    let activity_path = scenario.path("home/.thirdshift/logs/acme/widgets/activity.log");
    let cron_activity = fs::read_to_string(&activity_path).unwrap();
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = server(&scenario);
    let response = api::json(post(
        &console,
        &token,
        &json!({"repository":"acme/widgets","command":"pickup"}),
    ));
    assert_eq!(
        response,
        json!({"version":1,"outcome":"skipped", "reason":
        "at the Command limit on acme/widgets: 1 Command is running, pass.command_limit is 1"})
    );
    // The shared skip suppression also keeps the API from adding a duplicate.
    assert_eq!(fs::read_to_string(&activity_path).unwrap(), cron_activity);
    let events = api::cli(&scenario.path("home"), &["console", "log", "--json"]);
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event"] == "api-call"
                && event["email"] == "runner"
                && event["outcome"] == 200)
    );
    drop(held);
}

#[test]
fn api_starts_other_commands_with_cli_options_and_requester() {
    for kind in [
        "architect",
        "review",
        "process",
        "skill",
        "score",
        "triage",
        "secure",
    ] {
        let scenario = fixture();
        scenario.agent_does("exit 1\n");
        let mut body = json!({"repository":"acme/widgets","command":kind});
        match kind {
            "architect" => {
                body["focus"] = json!("the session module");
                body["base"] = json!("main");
                body["plan_only"] = json!(true);
            }
            "review" => {
                scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
                body["pull_request"] = json!(scenario.github_has_pr("colleague", "main", "OPEN"));
                body["recommend_only"] = json!(true);
            }
            "process" => {
                let directory = scenario.path("home/.thirdshift/processes");
                fs::create_dir_all(&directory).unwrap();
                fs::write(
                    directory.join("inspect.toml"),
                    "version = 1\ndescription = \"Inspect the issue\"\n[[steps]]\nname = \"inspect\"\nskill = \"code-review\"\n",
                )
                .unwrap();
                body["name"] = json!("inspect");
                body["issue"] = json!(7);
            }
            "skill" => {
                body["name"] = json!("to-spec");
                body["issue"] = json!(7);
            }
            "triage" => {
                body["report"] = json!("Canary fails when the cache is empty.\nInvestigate.");
                body["read_machine"] = json!(false);
                body["recommend_only"] = json!(true);
            }
            _ => {}
        }
        let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
        let console = server(&scenario);
        let response = post(&console, &token, &body);
        let status = response.status();
        let response: Value = response.json().unwrap();
        assert_eq!(
            status,
            200,
            "{kind}: {response}\n{}",
            fs::read_to_string(scenario.path("home/.thirdshift/logs/cron.log")).unwrap_or_default()
        );
        if kind == "skill" {
            let pid = response["process_id"].as_u64().unwrap() as libc::pid_t;
            let deadline = Instant::now() + Duration::from_secs(30);
            // SAFETY: signal zero only observes this fixture's exact child.
            while unsafe { libc::kill(pid, 0) } == 0 {
                assert!(Instant::now() < deadline, "skill process {pid} did not end");
                std::thread::sleep(Duration::from_millis(25));
            }
        } else {
            let name = response["name"].as_str().unwrap();
            let shown = scenario.run(&["show", name, "--json"]);
            assert_eq!(shown.code, Some(0), "{kind}: {}", shown.stderr);
            assert!(
                shown.stdout.contains("token runner"),
                "{kind}: {}",
                shown.stdout
            );
            let _ = scenario.run(&["stop", name, "--json"]);
        }
        let repository = if kind == "score" {
            "JacobStephens2/thirdshift"
        } else {
            "acme/widgets"
        };
        let activity = fs::read_to_string(
            scenario.path(&format!("home/.thirdshift/logs/{repository}/activity.log")),
        )
        .unwrap();
        assert!(
            activity.contains("from the Console by token runner"),
            "{kind}: {activity}"
        );
    }
}

#[test]
fn api_rejects_paths_and_options_its_cli_twin_cannot_take_before_spawning() {
    let scenario = fixture();
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = server(&scenario);
    support::write_executable(
        &scenario.path("login-shell"),
        &format!(
            "#!/bin/sh\ntouch '{}'\nexit 92\n",
            scenario.path("spawned").display()
        ),
    );
    for options in [
        json!({"command":"triage","file":"/etc/passwd"}),
        json!({"command":"triage","report":"text","path":"/etc/passwd"}),
        json!({"command":"triage","report":"  "}),
        json!({"command":"triage","report":"text","merge":true}),
        json!({"command":"triage","report":"text","arguments":["--file","/etc/passwd"]}),
        json!({"command":"architect","plan_only":true,"merge":false}),
        json!({"command":"architect","focus":"merge"}),
        json!({"command":"secure","order":"highest-first"}),
        json!({"command":"pickup","focus":"area"}),
        json!({"command":"review","author":"--merge"}),
        json!({"command":"review","pull_request":"https://github.com/acme/other/pull/1"}),
        json!({"command":"review","author":"jacob","merge":true}),
        json!({"command":"process","name":"list"}),
        json!({"command":"process","name":"inspect","issue":7,"pull_request":"https://github.com/acme/widgets/pull/1"}),
        json!({"command":"skill","name":"to-spec","model":"opus"}),
        json!({"command":"score","email":{"mode":"never"}}),
        json!({"command":"score","report":"text"}),
    ] {
        let mut body = options;
        body["repository"] = json!("acme/widgets");
        let response = post(&console, &token, &body);
        assert_eq!(
            response.status(),
            422,
            "{body}: {}",
            response.text().unwrap()
        );
        assert!(
            !scenario.path("spawned").exists(),
            "{body} started a process"
        );
    }
}

#[test]
fn api_triage_delivers_report_text_verbatim_to_the_harness() {
    let scenario = fixture();
    scenario.agent_does(&format!(
        "touch '{}'\nexit 1\n",
        scenario.path("completed").display()
    ));
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = server(&scenario);
    let report = "Report with 'quotes', $(touch /tmp/must-not-run) and\nnewlines.";
    let response = api::json(post(
        &console,
        &token,
        &json!({
            "repository":"acme/widgets","command":"triage","report":report,
            "read_machine":false,"recommend_only":true,"email":{"mode":"never"}
        }),
    ));
    wait_for(&scenario.path("completed"));
    let _ = scenario.run(&["stop", response["name"].as_str().unwrap(), "--json"]);
    let calls = scenario.claude_calls();
    let encoded_report = serde_json::to_string(report).unwrap();
    assert!(
        calls.iter().any(|call| call["prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains(&encoded_report))),
        "the Harness did not receive the complete JSON-quoted Report"
    );
}

#[test]
fn api_passes_return_quiet_skips_and_review_by_author_uses_the_cli_gate() {
    let scenario = fixture();
    scenario.user_config_is("[console]\nport = 0\n[activity]\nquiet_skips = true\n");
    scenario.agent_waits_for_release("exit 1\n");
    let held = HeldFixture {
        run: Some(scenario.run_until(&[&scenario.issue_url(7)], &[], "started")),
        release: scenario.path("release"),
    };
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = server(&scenario);
    for kind in ["pickup", "architect", "secure", "review"] {
        let mut body = json!({"repository":"acme/widgets","command":kind});
        if kind == "review" {
            body["author"] = json!("colleague");
        }
        for _ in 0..2 {
            let response = api::json(post(&console, &token, &body));
            assert_eq!(response["outcome"], "skipped", "{kind}: {response}");
            assert_eq!(
                response["reason"],
                "at the Command limit on acme/widgets: 1 Command is running, pass.command_limit is 1"
            );
        }
    }
    drop(held);
}

#[test]
fn api_options_reach_each_cli_parser_as_literal_words() {
    for kind in [
        "architect",
        "pickup",
        "secure",
        "review",
        "process",
        "triage",
    ] {
        let scenario = fixture();
        scenario.agent_does("exit 1\n");
        scenario.agent_does_in_session(1, "exit 0\n");
        let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
        let console = server(&scenario);
        // Observe the login-shell seam, then execute the real CLI parser.
        support::write_executable(
            &scenario.path("login-shell"),
            &format!(
                "#!/usr/bin/python3\nimport json, os, sys\nwith open({:?}, 'w') as out: json.dump(sys.argv[7:], out)\nos.execv('/bin/sh', ['/bin/sh'] + sys.argv[2:])\n",
                scenario.path("cli-words.json").to_str().unwrap()
            ),
        );
        let mut body = json!({"repository":"acme/widgets","command":kind,
            "email":{"mode":"never"},"harness":"claude","model":"opus","effort":"high"});
        let mut expected = vec![kind.to_string()];
        match kind {
            "architect" => {
                body["base"] = json!("main");
                body["focus"] = json!("the session module");
                expected.extend(["--base", "main", "the session module"].map(String::from));
            }
            "pickup" => {
                body["base"] = json!("main");
                body["order"] = json!("highest-first");
                expected.extend(["--base", "main", "--highest-first"].map(String::from));
            }
            "secure" => {
                body["base"] = json!("main");
                expected.extend(["--base", "main"].map(String::from));
            }
            "review" => {
                body["author"] = json!("colleague");
                body["recommend_only"] = json!(true);
                expected.extend(["--author", "colleague", "--recommend-only"].map(String::from));
            }
            "process" => {
                let directory = scenario.path("home/.thirdshift/processes");
                fs::create_dir_all(&directory).unwrap();
                fs::write(directory.join("inspect.toml"),
                    "version = 1\ndescription = \"Inspect\"\n[worktrees.review]\nkind = \"pull-request\"\n[[steps]]\nname = \"inspect\"\nskill = \"code-review\"\nworktree = \"review\"\n").unwrap();
                body["name"] = json!("inspect");
                body["pull_request"] = json!("https://github.com/acme/widgets/pull/1");
                scenario.origin_has_branch("colleague", "main", &["Colleague feature"]);
                scenario.github_has_pr("colleague", "main", "OPEN");
                expected.extend(
                    [
                        "inspect",
                        "pull-request",
                        "https://github.com/acme/widgets/pull/1",
                    ]
                    .map(String::from),
                );
            }
            "triage" => {
                body["report"] = json!("Report text");
                body["read_machine"] = json!(true);
                body["recommend_only"] = json!(true);
                body["allow_ready_for_agent"] = json!(true);
                expected.extend(
                    [
                        "--recommend-only",
                        "--read-machine",
                        "--allow-ready-for-agent",
                    ]
                    .map(String::from),
                );
            }
            _ => unreachable!(),
        }
        if matches!(kind, "architect" | "pickup" | "secure") {
            body["merge"] = json!(false);
            body["parallel"] = json!(2);
            body["base_fix"] = json!(false);
            body["security_fix"] = json!(false);
            body["security_close"] = json!(false);
            body["security_review"] = json!(false);
            expected.extend(
                [
                    "--no-merge",
                    "--no-email",
                    "parallel",
                    "2",
                    "no-security-fix",
                    "no-security-close",
                    "no-security-review",
                    "harness",
                    "claude",
                    "model",
                    "opus",
                    "effort",
                    "high",
                    "no-base-fix",
                ]
                .map(String::from),
            );
        } else {
            body["security_review"] = if kind == "triage" {
                Value::Null
            } else {
                json!(false)
            };
            expected.push("--no-email".into());
            if kind != "triage" {
                expected.push("no-security-review".into());
            }
            expected
                .extend(["harness", "claude", "model", "opus", "effort", "high"].map(String::from));
        }
        let response = post(&console, &token, &body);
        let status = response.status();
        let response: Value = response.json().unwrap();
        assert_eq!(
            status,
            200,
            "{kind}: {response}\n{}",
            fs::read_to_string(scenario.path("home/.thirdshift/logs/cron.log")).unwrap_or_default()
        );
        let actual: Vec<String> =
            serde_json::from_slice(&fs::read(scenario.path("cli-words.json")).unwrap()).unwrap();
        assert_eq!(actual, expected, "{kind}");
        if let Some(name) = response["name"].as_str() {
            let _ = scenario.run(&["stop", name, "--json"]);
        }
    }
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

#[test]
fn structured_email_addresses_cannot_become_cli_flags() {
    let scenario = fixture();
    let token = tokens::create(&scenario.path("home"), "runner", &["--scopes", "run"]);
    let console = server(&scenario);
    support::write_executable(
        &scenario.path("login-shell"),
        &format!(
            "#!/bin/sh\ntouch '{}'\nexit 92\n",
            scenario.path("spawned").display()
        ),
    );
    for mode in ["always", "unless_merged"] {
        for address in ["--merge", "merge", "--security-review", "not-an-address"] {
            let response = post(
                &console,
                &token,
                &json!({
                    "repository":"acme/widgets", "issue":7, "email":{"mode":mode,"address":address}
                }),
            );
            assert_eq!(response.status(), 422);
            assert!(
                response.json::<Value>().unwrap()["error"]
                    .as_str()
                    .unwrap()
                    .contains("email address")
            );
            assert!(
                !scenario.path("spawned").exists(),
                "{address} started a Command"
            );
        }
    }
}
