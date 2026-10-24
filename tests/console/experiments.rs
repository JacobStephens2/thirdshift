use super::*;

fn fixture(home: &Path, id: &str, created: &str) -> std::path::PathBuf {
    let directory = home.join(".thirdshift/evals/experiments").join(id);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("experiment.json"), serde_json::to_vec(&json!({
        "format": 1, "id": id, "state": "running", "created_at": created,
        "baseline": {"commit": "a".repeat(40), "harness": "claude", "model": null, "effort": null},
        "candidate": {"commit": "b".repeat(40), "harness": "codex", "model": null, "effort": null},
        "suites": ["code-review"], "prepared": null,
        "bar": {"margin": -0.1, "credible_level": 0.95, "prior": 1.0, "seed": 0, "draws": 100,
            "trials_per_task": {"code-review": 1}, "timeout_seconds": 1800,
            "void_rules": {"retries": 2, "causes": ["memory_cap"]}},
        "tasks": []
    })).unwrap()).unwrap();
    directory
}

#[test]
fn legacy_paused_experiment_advertises_a_supported_resume_command() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let id = "e".repeat(64);
    let directory = fixture(home.path(), &id, "2026-10-24T10:00:00Z");
    let path = directory.join("experiment.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    saved["state"] = json!("paused");
    saved["bar"]["credible_level"] = json!(0.8);
    saved["resume_command"] = json!(
        "thirdshift experiment candidate --baseline baseline --suite code-review --credible-level 0.8 --json"
    );
    fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get(&format!("/experiments/{id}")).text().unwrap();
    let resume = html
        .split("Run the same command to resume: <code>")
        .nth(1)
        .unwrap()
        .split("</code>")
        .next()
        .unwrap();
    // This deliberately quote-free fixture allows executing the displayed
    // words directly; no shell or HTML entity decoding is needed.
    let attempted = Command::new(binary)
        .env("HOME", home.path())
        .args(resume.split_whitespace().skip(1))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&attempted.stderr);
    assert!(
        !stderr.contains("unknown Experiment flag: --credible-level"),
        "displayed command is rejected by the CLI: {stderr}"
    );
    assert!(!resume.contains("--credible-level"), "{resume}");
    assert!(
        resume.contains("--pass-level") && resume.contains("--fail-level"),
        "{resume}"
    );
}

fn proposal(home: &Path, suite: &str, task: &str) {
    let directory = home.join(".thirdshift/evals/suites").join(suite);
    fs::create_dir_all(directory.join("tasks").join(task)).unwrap();
    fs::write(directory.join("suite.toml"), "format = 1\ntrials_per_arm = 1\n[dependencies]\nfactory_skills = []\nsession_prompts = []\nharness_defaults = []\nmodel_defaults = []\neffort_defaults = []\n").unwrap();
    fs::write(directory.join("tasks").join(task).join("task.toml"), format!(
        "format = 1\nstate = \"proposed\"\nsession = \"code-review\"\nrepository = \"private\"\nbase = {:?}\nchange = {:?}\ncriteria = \"Invented success criteria\"\n[inputs]\n",
        "a".repeat(40), "b".repeat(40)
    )).unwrap();
}

#[test]
fn console_approval_pairs_with_cli_and_requires_csrf() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let id = "d".repeat(64);
    fixture(home.path(), &id, "2026-10-24T10:00:00Z");
    proposal(home.path(), "code-review", "from-console");
    proposal(home.path(), "code-review", "from-cli");
    proposal(home.path(), "other-suite", "unrelated");
    let cli = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .env("HOME", home.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/experiments").text().unwrap();
    assert!(html.contains("Proposed Eval tasks"));
    assert!(html.contains("from-console"));
    assert!(html.contains("unrelated"));
    let html = console.get(&format!("/experiments/{id}")).text().unwrap();
    assert!(html.contains("from-console"));
    assert!(!html.contains("unrelated"));
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let url = format!(
        "{}/experiments/proposals/code-review/from-console/approve",
        console.address
    );
    assert_eq!(client.post(&url).send().unwrap().status(), 403);
    assert_eq!(cli(&["evals", "list"])[0]["tasks"][1]["state"], "proposed");
    let csrf = html
        .split("name=\"csrf\" value=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let response = client.post(&url).form(&[("csrf", csrf)]).send().unwrap();
    assert_eq!(response.status(), 303);
    cli(&["evals", "approve", "code-review", "from-cli", "--json"]);
    let listed = cli(&["evals", "list"]);
    for task in listed[0]["tasks"].as_array().unwrap() {
        assert_eq!(task["state"], "approved");
    }
    let html = console.get(&format!("/experiments/{id}")).text().unwrap();
    assert!(!html.contains("from-console"));
    assert!(!html.contains("from-cli"));
    assert_eq!(cli(&["experiments", &id, "--json"])["tasks"], json!([]));
}

#[test]
fn experiments_pairs_list_and_detail_with_cli_and_orders_by_start_time() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let older = "f".repeat(64);
    let newer = "a".repeat(64);
    fixture(home.path(), &older, "2026-10-24T14:00:00+02:00");
    fixture(home.path(), &newer, "2026-10-24T09:00:00-04:00");
    let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .env("HOME", home.path())
        .args(["experiments", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(listed[0]["id"], newer);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = console.get("/experiments");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains("thirdshift experiments --json"));
    assert!(html.contains("Running"));
    assert!(html.contains(&format!("/experiments/{newer}")));
    let response = console.get(&format!("/experiments/{newer}"));
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains(&format!("thirdshift experiments {newer} --json")));
    assert!(html.contains("The Bar"));
    assert!(html.contains("What was compared"));
    assert!(html.contains("Trials"));
    assert_eq!(console.get("/experiments/not-an-id").status(), 404);
}

#[test]
fn experiments_exposes_queued_running_and_retained_session_evidence() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let id = "c".repeat(64);
    let directory = fixture(home.path(), &id, "2026-10-24T10:00:00Z");
    let path = directory.join("experiment.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["baseline_ref"] = json!("v1.0");
    record["candidate_ref"] = json!("candidate");
    record["title"] = json!("Fix <script>unsafe()</script>");
    record["tasks"] = json!([{"suite": "code-review", "name": "fixture", "directory": "/private/task",
        "manifest": {"format": 1, "state": "approved", "session": "code-review", "repository": "private",
        "base": "a".repeat(40), "change": "b".repeat(40), "criteria": "private criteria", "inputs": {}, "trial": null}}]);
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    use sha2::{Digest, Sha256};
    let key = format!(
        "{:x}--baseline--0",
        Sha256::new()
            .chain_update(b"code-review\0fixture")
            .finalize()
    );
    let attempt = directory.join("trials").join(&key).join("attempt-test");
    fs::create_dir_all(attempt.join("session")).unwrap();
    fs::write(
        attempt.join("started.json"),
        r#"{"index":0,"started_at":"2026-10-24T10:01:00Z"}"#,
    )
    .unwrap();
    fs::write(attempt.join("session/stdout.jsonl"), "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Trial evidence <script>unsafe()</script>\"}]}}\n").unwrap();
    fs::write(
        directory.join("trials").join(&key).join("active.json"),
        r#"{"started_at":"2026-10-24T10:01:00Z"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .env("HOME", home.path())
        .args(["experiments", &id, "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let view: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        view["tasks"],
        json!([{"suite":"code-review", "name":"fixture"}])
    );
    assert_eq!(view["running_trials"][0]["id"], key);
    assert_eq!(
        view["running_trials"][0]["started_at"],
        "2026-10-24T10:01:00Z"
    );
    let session = view["sessions"][0]["id"].as_str().unwrap();
    let cli = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .env("HOME", home.path())
        .args(["experiments", &id, "--session", session])
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    assert!(String::from_utf8_lossy(&cli.stdout).contains("Trial evidence"));
    let session_json = || {
        let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .env("HOME", home.path())
            .args(["experiments", &id, "--session", session, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let evidence = session_json();
    assert_eq!(evidence["id"], session);
    assert_eq!(evidence["trace"], false);
    assert_eq!(
        evidence["contents"],
        fs::read_to_string(attempt.join("session/stdout.jsonl")).unwrap()
    );
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get(&format!("/experiments/{id}")).text().unwrap();
    assert!(html.contains("v1.0"));
    assert!(html.contains("candidate"));
    assert!(html.contains("Fix &#60;script&#62;"));
    assert!(html.contains("Running"));
    assert!(html.contains("Queued"));
    assert!(!html.contains("private criteria"));
    assert!(!html.contains("/private/task"));
    let response = console.get(&format!("/experiments/{id}/sessions/{session}"));
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains("Trial evidence"));
    assert!(!html.contains("<script>unsafe()</script>"));
    assert_eq!(
        console
            .get(&format!("/experiments/{id}/sessions/not-listed"))
            .status(),
        404
    );

    // A new Arm replaces its raw Session log with OTLP records. Existing links
    // still select that same session, while the page names it a Session trace.
    let raw = fs::read_to_string(attempt.join("session/stdout.jsonl")).unwrap();
    fs::write(attempt.join("session/stdout.logs.jsonl"), format!("{}\n", json!({
        "resourceLogs": [{"scopeLogs": [{"logRecords": [{"body": {"stringValue": raw.trim()}}]}]}]
    }))).unwrap();
    fs::write(attempt.join("session/stdout.spans.jsonl"), "").unwrap();
    fs::remove_file(attempt.join("session/stdout.jsonl")).unwrap();
    let response = console.get(&format!("/experiments/{id}/sessions/{session}"));
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains("Session trace"));
    assert!(html.contains("Trial evidence"));
    let evidence = session_json();
    assert_eq!(evidence["trace"], true);
    assert!(
        evidence["contents"]
            .as_str()
            .unwrap()
            .contains("Trial evidence")
    );

    #[cfg(unix)]
    {
        let secret = home.path().join("secret.jsonl");
        fs::write(&secret, "outside the Experiment").unwrap();
        std::os::unix::fs::symlink(secret, attempt.join("session/elsewhere.jsonl")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .env("HOME", home.path())
            .args(["experiments", &id, "--json"])
            .output()
            .unwrap();
        let view: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(view["sessions"].as_array().unwrap().len(), 1);
    }
}
