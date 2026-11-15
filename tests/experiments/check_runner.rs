use super::custom_runner::{DELIVER, declarations, harness_script, runner};
use super::*;

/// The report on stdout, whatever the exit status, with that status.
fn check(fixture: &Fixture, args: &[&str]) -> (Option<i32>, Value) {
    let mut all = vec!["evals", "check-runner"];
    all.extend_from_slice(args);
    all.push("--json");
    let result = fixture.scenario.run_with_env(&all, &NO_SANDBOX);
    let report = serde_json::from_str(&result.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: stdout {:?}, stderr {:?}",
            result.stdout, result.stderr
        )
    });
    (result.code, report)
}

fn problems(report: &Value, check: &str) -> Vec<String> {
    report["problems"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|problem| problem["check"] == check)
        .map(|problem| problem["message"].as_str().unwrap().to_owned())
        .collect()
}

fn manifest(fixture: &Fixture) -> PathBuf {
    fixture
        .scenario
        .path("home/.thirdshift/runners/fixture.toml")
}

fn append(fixture: &Fixture, text: &str) {
    let path = manifest(fixture);
    let manifest = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("{manifest}\n{text}\n")).unwrap();
}

/// A Claude stand-in reporting one priced Model turn.
fn priced_claude(fixture: &Fixture) {
    harness_script(
        fixture,
        "claude",
        r#"echo '{"type":"assistant","message":{"id":"m1","model":"claude-sonnet-4-6","usage":{"input_tokens":10,"output_tokens":2},"content":[{"type":"text","text":"done"}]}}'
echo '{"type":"result","subtype":"success","result":"done"}'"#,
    );
}

#[test]
fn a_valid_runner_passes_with_the_digest_an_experiment_records() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    let (code, report) = check(&fixture, &["fixture"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["problems"], json!([]), "{report}");
    assert!(report["trial"].is_null(), "{report}");
    assert_eq!(
        PathBuf::from(report["manifest"].as_str().unwrap()),
        manifest(&fixture).canonicalize().unwrap()
    );
    let digest = report["digest"].as_str().unwrap();
    assert_eq!(digest.len(), 64, "{report}");
    let path = manifest(&fixture);
    let (code, by_path) = check(&fixture, &[path.to_str().unwrap()]);
    assert_eq!(code, Some(0), "{by_path}");
    assert_eq!(by_path["digest"], digest);
    let text = fixture
        .scenario
        .run_with_env(&["evals", "check-runner", "fixture"], &NO_SANDBOX);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout.contains("Static checks: passed"),
        "{}",
        text.stdout
    );
    assert!(text.stdout.contains(digest), "{}", text.stdout);
    assert!(
        text.stdout.contains("Custom runner check passed."),
        "{}",
        text.stdout
    );
}

#[test]
fn malformed_manifests_are_refused_before_any_other_check() {
    for (text, evidence) in [
        ("command = []\n", "must contain a program"),
        ("command = [\"run\"]\nunknown = 1\n", "unknown field"),
        ("command = \"run\"\n", "invalid type"),
        (
            "command = [\"run\"]\ngit_exclude = [\"a\\nb\"]\n",
            "must each be one line",
        ),
    ] {
        let fixture = process::fixture();
        runner(&fixture, DELIVER);
        fs::write(manifest(&fixture), text).unwrap();
        let (code, report) = check(&fixture, &["fixture"]);
        assert_eq!(code, Some(1), "{report}");
        assert_eq!(report["ok"], false, "{report}");
        assert_eq!(report["problems"].as_array().unwrap().len(), 1, "{report}");
        let messages = problems(&report, "manifest");
        assert!(messages[0].contains(evidence), "{text}: {report}");
        assert!(report["digest"].is_null(), "{report}");
    }
    let fixture = process::fixture();
    let (code, report) = check(&fixture, &["absent"]);
    assert_eq!(code, Some(1), "{report}");
    assert!(
        problems(&report, "manifest")[0].contains("cannot resolve Custom runner absent"),
        "{report}"
    );
}

#[test]
fn missing_sources_and_escaping_home_targets_are_each_reported() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    append(
        &fixture,
        "\"missing.py\" = \"missing.py\"\n\"./runner.py\" = { home = \"../outside\" }",
    );
    let (code, report) = check(&fixture, &["fixture"]);
    assert_eq!(code, Some(1), "{report}");
    let messages = problems(&report, "files");
    assert_eq!(messages.len(), 2, "{report}");
    assert!(
        messages
            .iter()
            .any(|message| message.contains("cannot find Custom runner file missing.py")),
        "{report}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("confined relative Eval path: ../outside")),
        "{report}"
    );
    assert!(report["digest"].is_null(), "{report}");
}

#[test]
fn missing_harness_logins_and_provider_keys_are_reported_then_accepted() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    harness_script(&fixture, "claude", "exit 0");
    harness_script(&fixture, "codex", "exit 0");
    declarations(
        &fixture,
        "providers = [\"openai\"]\nharnesses = { claude = [], codex = [], unknown = [] }",
    );
    let (code, report) = check(&fixture, &["fixture"]);
    assert_eq!(code, Some(1), "{report}");
    assert!(
        problems(&report, "harnesses")[0].contains("unknown Custom runner Harness unknown"),
        "{report}"
    );
    let logins = problems(&report, "logins");
    assert_eq!(logins.len(), 2, "{report}");
    assert!(
        logins
            .iter()
            .any(|message| message.contains("declared claude has no Trials setup token")),
        "{report}"
    );
    assert!(
        logins
            .iter()
            .any(|message| message.contains("declared codex has no login")),
        "{report}"
    );
    assert_eq!(
        problems(&report, "providers"),
        ["missing [providers].openai Trial key"],
        "{report}"
    );

    let text = fs::read_to_string(manifest(&fixture))
        .unwrap()
        .replace(", unknown = []", "");
    fs::write(manifest(&fixture), text).unwrap();
    fixture.scenario.credentials_are(
        "[claude]\nsetup_token = \"trial-login\"\n[providers]\nopenai = \"trial-key\"\n",
    );
    fs::create_dir_all(fixture.scenario.path("home/.codex")).unwrap();
    fs::write(fixture.scenario.path("home/.codex/auth.json"), "{}").unwrap();
    let (code, report) = check(&fixture, &["fixture"]);
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["problems"], json!([]), "{report}");
}

#[test]
fn roles_naming_unpriced_models_are_reported() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    append(
        &fixture,
        "[roles.implement]\nmodel = \"claude-sonnet-4-6\"\n[roles.review]\nmodel = \"unpriced-model\"\neffort = \"high\"",
    );
    let (code, report) = check(&fixture, &["fixture"]);
    assert_eq!(code, Some(1), "{report}");
    let roles = problems(&report, "roles");
    assert_eq!(roles.len(), 1, "{report}");
    assert!(
        roles[0].contains("role review names Model unpriced-model"),
        "{report}"
    );
    // A pricing problem does not stop the runner from freezing.
    assert_eq!(report["digest"].as_str().unwrap().len(), 64, "{report}");
}

#[test]
fn a_dry_run_reports_outcome_checks_and_session_cost_without_an_experiment() {
    let fixture = process::fixture();
    priced_claude(&fixture);
    runner(
        &fixture,
        &(String::from(
            "subprocess.run(request['record_command'] + ['--role','implement','--','claude','-p','test'], check=True)\n",
        ) + DELIVER),
    );
    declarations(&fixture, "harnesses = { claude = [] }");
    fixture
        .scenario
        .credentials_are("[claude]\nsetup_token = \"trial-login\"\n");
    let (code, report) = check(
        &fixture,
        &["fixture", "--suite", "process", "--task", "fixture"],
    );
    assert_eq!(code, Some(0), "{report}");
    assert_eq!(report["ok"], true, "{report}");
    let trial = &report["trial"];
    assert_eq!(trial["suite"], "process", "{report}");
    assert_eq!(trial["task"], "fixture", "{report}");
    assert_eq!(trial["outcome"], "delivered", "{report}");
    assert!(trial["void_cause"].is_null(), "{report}");
    assert!(
        trial["cause"]
            .as_str()
            .unwrap()
            .contains("uncommitted working-tree files ignored by grading"),
        "{report}"
    );
    assert_eq!(trial["passed"], true, "{report}");
    let checks = trial["checks"].as_array().unwrap();
    assert!(checks.len() > 1, "{report}");
    assert!(
        checks.iter().all(|check| check["passed"] == true),
        "{report}"
    );
    assert!(
        checks.iter().any(|check| check["command"].is_array()),
        "the task's code checks ran: {report}"
    );
    let sessions = trial["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "{report}");
    assert_eq!(sessions[0]["role"], "implement", "{report}");
    assert_eq!(sessions[0]["cost_usd"], 0.000060, "{report}");
    assert_eq!(trial["cost_usd"], 0.000060, "{report}");
    assert!(
        !fixture
            .scenario
            .path("home/.thirdshift/evals/experiments")
            .exists(),
        "a dry run writes no Experiment storage"
    );
}

#[test]
fn a_dry_run_reports_its_void_cause_and_fails_the_check() {
    let fixture = process::fixture();
    harness_script(&fixture, "claude", "exit 0");
    runner(&fixture, DELIVER);
    declarations(&fixture, "harnesses = { claude = [] }");
    fixture
        .scenario
        .credentials_are("[claude]\nsetup_token = \"trial-login\"\n");
    let (code, report) = check(
        &fixture,
        &["fixture", "--suite", "process", "--task", "fixture"],
    );
    assert_eq!(code, Some(1), "{report}");
    assert_eq!(report["ok"], false, "{report}");
    assert_eq!(report["problems"], json!([]), "{report}");
    assert_eq!(report["trial"]["outcome"], "void", "{report}");
    assert_eq!(report["trial"]["void_cause"], "accounting_gap", "{report}");
    assert_eq!(report["trial"]["passed"], false, "{report}");
    let text = fixture.scenario.run_with_env(
        &[
            "evals",
            "check-runner",
            "fixture",
            "--suite",
            "process",
            "--task",
            "fixture",
        ],
        &NO_SANDBOX,
    );
    assert_eq!(text.code, Some(1), "{}", text.stderr);
    assert!(
        text.stdout
            .contains("Dry run process/fixture: void (accounting_gap); Score failed"),
        "{}",
        text.stdout
    );
    assert!(
        text.stdout.contains("Custom runner check failed."),
        "{}",
        text.stdout
    );
    assert!(
        report["trial"]["cost_usd"].is_null(),
        "an accounting gap has no cost: {report}"
    );
    assert!(text.stdout.contains("Cost: incomplete"), "{}", text.stdout);
}

#[test]
fn static_problems_skip_the_dry_run() {
    let fixture = process::fixture();
    let started = fixture.scenario.path("runner-started");
    runner(
        &fixture,
        &format!(
            "pathlib.Path({:?}).write_text('started')\n{DELIVER}",
            started
        ),
    );
    harness_script(&fixture, "claude", "exit 0");
    declarations(&fixture, "harnesses = { claude = [] }");
    let (code, report) = check(
        &fixture,
        &["fixture", "--suite", "process", "--task", "fixture"],
    );
    assert_eq!(code, Some(1), "{report}");
    assert_eq!(problems(&report, "logins").len(), 1, "{report}");
    assert!(report["trial"].is_null(), "{report}");
    assert!(!started.exists(), "the runner must not start");
}

#[test]
fn a_dry_run_refuses_unusable_tasks_before_checking_the_runner() {
    let fixture = process::fixture();
    runner(&fixture, DELIVER);
    for (args, evidence) in [
        (
            vec!["--suite", "process", "--task", "absent"],
            "cannot load Eval task process/absent",
        ),
        (
            vec!["--suite", "code-review", "--task", "fixture"],
            "Custom runners require process suites: code-review",
        ),
        (vec!["--suite", "process"], "needs both --suite and --task"),
        (
            vec!["--task", "../fixture", "--suite", "process"],
            "invalid Eval suite or task name: ../fixture",
        ),
    ] {
        let mut all = vec!["evals", "check-runner", "fixture"];
        all.extend(args);
        all.push("--json");
        let result = fixture.scenario.run_with_env(&all, &NO_SANDBOX);
        assert_ne!(result.code, Some(0), "{all:?}");
        assert!(result.stdout.is_empty(), "{all:?}: {}", result.stdout);
        assert!(
            result.stderr.contains(evidence),
            "{all:?}: {}",
            result.stderr
        );
    }
}

#[test]
fn a_dry_run_confines_the_runner_in_the_trial_sandbox() {
    let available = |program: &str, flag: &str| {
        std::process::Command::new(program)
            .arg(flag)
            .output()
            .is_ok_and(|output| output.status.success())
    };
    if !available("bwrap", "--version") || !available("socat", "-V") {
        println!("SKIPPED (dry-run sandbox): bwrap or socat is not on PATH");
        return;
    }
    let fixture = process::fixture();
    let hidden = fixture.scenario.path("operator-private");
    fs::write(&hidden, "operator secret").unwrap();
    runner(
        &fixture,
        &format!(
            "assert not pathlib.Path({hidden:?}).exists(), 'unbound host file exposed'\nassert 'THIRDSHIFT_OPERATOR_TOKEN' not in os.environ\n{DELIVER}"
        ),
    );
    let result = fixture.scenario.run_with_env(
        &[
            "evals",
            "check-runner",
            "fixture",
            "--suite",
            "process",
            "--task",
            "fixture",
            "--json",
        ],
        &[("THIRDSHIFT_OPERATOR_TOKEN", "operator-token")],
    );
    let report: Value = serde_json::from_str(&result.stdout)
        .unwrap_or_else(|error| panic!("{error}: {}", result.stderr));
    assert_eq!(report["trial"]["outcome"], "delivered", "{report}");
    assert_eq!(report["trial"]["passed"], true, "{report}");
    assert_eq!(result.code, Some(0), "{report}");
}
