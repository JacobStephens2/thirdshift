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
    assert_eq!(evidence.as_object().unwrap().len(), 3);
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
    let unavailable = |session: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .env("HOME", home.path())
            .args(["experiments", &id, "--session", session, "--json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        let response = console.get(&format!("/experiments/{id}/sessions/{session}"));
        assert_eq!(response.status(), 404);
        assert_eq!(response.text().unwrap(), "Session is unavailable.");
    };
    unavailable("not-listed");

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
    assert!(!html.contains("<script>unsafe()</script>"));
    let evidence = session_json();
    assert_eq!(
        evidence,
        json!({"id": session, "trace": true,
        "contents": fs::read_to_string(attempt.join("session/stdout.logs.jsonl")).unwrap()})
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
        let escaped_id = format!(
            "{:x}",
            Sha256::digest(
                attempt
                    .join("session/elsewhere.jsonl")
                    .strip_prefix(&directory)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
            )
        );
        unavailable(&escaped_id);
        let outside = home.path().join("outside-session");
        fs::create_dir_all(&outside).unwrap();
        fs::rename(
            attempt.join("session/stdout.logs.jsonl"),
            outside.join("stdout.logs.jsonl"),
        )
        .unwrap();
        fs::remove_dir_all(attempt.join("session")).unwrap();
        std::os::unix::fs::symlink(&outside, attempt.join("session")).unwrap();
        unavailable(session);
        fs::remove_file(attempt.join("session")).unwrap();
        fs::create_dir(attempt.join("session")).unwrap();
    }
    fs::write(attempt.join("session/stdout.logs.jsonl"), [0xff]).unwrap();
    unavailable(session);
    fs::remove_file(attempt.join("session/stdout.logs.jsonl")).unwrap();
    unavailable(session);
}

#[test]
fn selected_session_evidence_survives_unrelated_corruption_and_legacy_verdicts() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let id = "d".repeat(64);
    let directory = fixture(home.path(), &id, "2026-10-24T10:00:00Z");
    let session_folder = directory.join("trials/healthy/attempt-old/session");
    fs::create_dir_all(&session_folder).unwrap();
    let contents = "Trial evidence <script>unsafe()</script>\n";
    fs::write(session_folder.join("stdout.jsonl"), contents).unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .env("HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };
    let view: Value = serde_json::from_slice(&cli(&["experiments", &id, "--json"]).stdout).unwrap();
    let session = view["sessions"][0]["id"].as_str().unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let metadata = directory.join("experiment.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    for state in ["running", "completed"] {
        // No saved Verdict: an ordinary completed load must upgrade it.
        record["state"] = json!(state);
        fs::write(&metadata, serde_json::to_vec(&record).unwrap()).unwrap();
        for relative in [
            "trials/unrelated--baseline--0/attempt-old/session/stdout.spans.jsonl",
            "results/unrelated--baseline--0.json",
            "trials/unrelated--baseline--0/pending.json",
            "trials/unrelated--baseline--0/active.json",
            "trials/unrelated--baseline--0/attempt-old/started.json",
        ] {
            let corrupt = directory.join(relative);
            fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
            fs::write(&corrupt, "malformed").unwrap();
            let output = cli(&["experiments", &id, "--session", session, "--json"]);
            assert!(
                output.status.success(),
                "{state}: {relative}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&output.stdout).unwrap(),
                json!({"id": session, "trace": false, "contents": contents})
            );
            let response = console.get(&format!("/experiments/{id}/sessions/{session}"));
            assert_eq!(response.status(), 200, "{state}: {relative}");
            let html = response.text().unwrap();
            assert!(html.contains("Trial evidence"));
            assert!(!html.contains("<script>unsafe()</script>"));
            fs::remove_file(corrupt).unwrap();
        }
    }
}

#[test]
fn experiment_detail_says_plainly_what_the_experiment_tests() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let changed = "c".repeat(64);
    let directory = fixture(home.path(), &changed, "2026-10-24T09:00:00-04:00");
    let mut record: Value =
        serde_json::from_slice(&fs::read(directory.join("experiment.json")).unwrap()).unwrap();
    record["suites"] = json!(["code-review", "spec-review"]);
    record["bar"]["trials_per_task"]["spec-review"] = json!(1);
    record["baseline_ref"] = json!("v0.16.0");
    record["candidate_ref"] = json!("main");
    record["title"] = json!("Merge pull request #1351 from acme/issue-1223");
    fs::write(
        directory.join("experiment.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let same = "d".repeat(64);
    let directory = fixture(home.path(), &same, "2026-10-24T10:00:00-04:00");
    let mut record: Value =
        serde_json::from_slice(&fs::read(directory.join("experiment.json")).unwrap()).unwrap();
    record["candidate"] = record["baseline"].clone();
    record["baseline_ref"] = json!("71c7a497");
    record["candidate_ref"] = json!("71c7a497");
    fs::write(
        directory.join("experiment.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());

    let html = console
        .get(&format!("/experiments/{changed}"))
        .text()
        .unwrap();
    let question = html
        .split("<section class=\"experiment-question\"")
        .nth(1)
        .expect(&html)
        .split("</section>")
        .next()
        .unwrap();
    for expected in [
        "<h2>What it tests</h2>",
        "Does the Candidate do worse than the Baseline on code-review and spec-review?",
        "Build: v0.16.0 (aaaaaaaaaa) → main (bbbbbbbbbb), “Merge pull request #1351 from acme/issue-1223”",
        "Harness: claude → codex",
        "Worse means a pass share more than 10 points below the Baseline’s.",
    ] {
        assert!(
            question.contains(expected),
            "missing {expected}: {question}"
        );
    }

    let html = console.get(&format!("/experiments/{same}")).text().unwrap();
    assert!(
        html.contains("Both arms are the same build, Harness, Model and Effort, so this Experiment tests no change: it shows how much code-review results vary between runs of one setup."),
        "{html}"
    );
}

#[test]
fn experiment_pages_and_the_cli_show_the_name_or_its_default() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    // `fixture` writes a record from before names: it has no `name` key.
    let edit = |id: &str, created: &str, change: &dyn Fn(&mut Value)| {
        let directory = fixture(home.path(), id, created);
        let mut record: Value =
            serde_json::from_slice(&fs::read(directory.join("experiment.json")).unwrap()).unwrap();
        change(&mut record);
        fs::write(
            directory.join("experiment.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    };
    let legacy = "1".repeat(64);
    edit(&legacy, "2026-10-24T09:00:00-04:00", &|_| {});
    let named = "2".repeat(64);
    edit(&named, "2026-10-24T10:00:00-04:00", &|record| {
        record["name"] = json!("Prompt <tweak>");
    });
    let same = "3".repeat(64);
    edit(&same, "2026-10-24T11:00:00-04:00", &|record| {
        record["candidate"] = record["baseline"].clone();
        record["candidate_ref"] = json!("71c7a497");
    });
    let release = "4".repeat(64);
    edit(&release, "2026-10-24T12:00:00-04:00", &|record| {
        record["release"] = json!(true);
        record["baseline_ref"] = json!("v0.16.0");
        record["candidate_ref"] = json!("main");
    });
    let expected = [
        (
            &legacy,
            "aaaaaaaaaa → bbbbbbbbbb",
            false,
            "aaaaaaaaaa → bbbbbbbbbb",
        ),
        (&named, "Prompt <tweak>", true, "Prompt &#60;tweak&#62;"),
        (&same, "A/A at 71c7a497", false, "A/A at 71c7a497"),
        (&release, "Release main", false, "Release main"),
    ];

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
    let listed: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    for (id, name, set, _) in expected {
        let view = listed.iter().find(|view| view["id"] == *id).unwrap();
        assert_eq!(view["name"], name);
        assert_eq!(view["name_set"], set);
    }

    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let list = console.get("/experiments").text().unwrap();
    for (id, _, _, html) in expected {
        assert!(
            list.contains(&format!(
                "<span class=\"subject\"><a href=\"/experiments/{id}\">{html}</a></span>"
            )),
            "{list}"
        );
        let detail = console.get(&format!("/experiments/{id}")).text().unwrap();
        assert!(
            detail.contains(&format!("<title>{html} · thirdshift</title>")),
            "{detail}"
        );
        assert!(detail.contains(&format!("<h1>{html}</h1>")), "{detail}");
    }
}

#[test]
fn a_release_experiment_saved_before_names_reads_as_a_release() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let id = "5".repeat(64);
    let directory = fixture(home.path(), &id, "2026-10-24T09:00:00-04:00");
    let path = directory.join("experiment.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["baseline_ref"] = json!("v0.16.0");
    record["candidate_ref"] = json!("main");
    record["resume_command"] = json!("thirdshift 'experiment' '--release' 'main'");
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .env("HOME", home.path())
        .args(["experiments", &id, "--json"])
        .output()
        .unwrap();
    let view: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(view["name"], "Release main");
}

#[test]
fn an_experiment_whose_harness_versions_differ_is_not_named_a_slash_a() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let id = "6".repeat(64);
    let directory = fixture(home.path(), &id, "2026-10-24T13:00:00-04:00");
    let path = directory.join("experiment.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["candidate"] = record["baseline"].clone();
    record["candidate_ref"] = json!("71c7a497");
    let arm = |version: &str| {
        json!({"commit": "a".repeat(40), "binary": "/b",
        "harness": {"name": "claude", "path": "/claude", "version": version}})
    };
    record["prepared"] = json!({"format": 1, "baseline": arm("1.0.0"), "candidate": arm("2.0.0")});
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let detail = console.get(&format!("/experiments/{id}")).text().unwrap();
    assert!(
        detail.contains("Harness version: 1.0.0 → 2.0.0"),
        "{detail}"
    );
    assert!(!detail.contains("<h1>A/A at 71c7a497</h1>"), "{detail}");
}
