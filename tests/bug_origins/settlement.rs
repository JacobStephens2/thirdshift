use super::*;

#[test]
fn confirming_a_security_fix_bug_writes_no_eval_task() {
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/bug_origins/security_fix_confirm.py"
        ))
        .arg(env!("CARGO_BIN_EXE_thirdshift"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn historical_review_and_reclosed_origin_regressions() {
    for mode in [
        "merged-base",
        "no-issue",
        "reclosed",
        "dotted-repo",
        "save-failure",
    ] {
        let output = std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/bug_origins/regressions.py"
            ))
            .arg(env!("CARGO_BIN_EXE_thirdshift"))
            .arg(mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn proposed_origin() -> (Scenario, String, String) {
    let (scenario, base, head) = origin_scenario();
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    (scenario, base, head)
}

fn origin_scenario() -> (Scenario, String, String) {
    let scenario = scenario();
    let base = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_owned();
    scenario.origin_has_commit("main", "answer", "41\n", "Introduce wrong answer");
    let head = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_owned();
    scenario.origin_has_commit("main", "answer", "42\n", "Fix answer");
    let fix = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_owned();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, Some(&fix), 0)]);
    state["origin_prs"] = json!({head.clone(): [{
        "html_url": "https://github.com/acme/widgets/pull/23",
        "merged_at": (Utc::now() - TimeDelta::days(1)).to_rfc3339(),
        "body": "Built with codex 1.2.3 · model · high <!-- thirdshift:built-with -->"
    }]});
    state["origin_review_prs"] = json!({"23": {
        "body": "Closes #7\nBuilt with codex 1.2.3 · model · high <!-- thirdshift:built-with -->",
        "headRefOid": head, "baseRefName": "main", "headRefName": "issue-7",
        "mergeCommit": {"oid": head, "parents": {"nodes": [{"oid": base}]}},
        "mergedAt": (Utc::now() - TimeDelta::days(1)).to_rfc3339(),
        "commits": {"nodes": [{"commit": {"parents": {"nodes": [{"oid": base}]}}}]},
        "closingIssuesReferences": {"nodes": [{"url": scenario.issue_url(7),
            "labels": {"nodes": [], "pageInfo": {"hasNextPage": false}}}],
            "pageInfo": {"hasNextPage": false}}
    }});
    scenario.write_gh_state(&state);
    scenario.issue_is(8, "CLOSED");
    scenario.agent_does(&format!("printf '%s' '{{\"commit\":\"{head}\",\"reason\":\"The introducing change writes 41 where the answer must be 42.\"}}' > proposal.json"));
    (scenario, base, head)
}

#[test]
fn proposals_settlements_and_metrics_keep_one_exact_continuation() {
    let (scenario, _, _) = origin_scenario();
    let folder = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    std::fs::create_dir_all(&folder).unwrap();
    let mut records = Vec::new();
    for (filename, started_at) in [
        ("older.command.json", "2026-10-19T12:00:00+02:00"),
        ("newer.command.json", "2026-10-19T11:00:00Z"),
    ] {
        let record = json!({
            "version":1, "kind":"Run", "repository":"acme/widgets", "name":"acme/widgets#7",
            "issue":{"number":7,"url":scenario.issue_url(7)}, "hostname":"factory",
            "process_id":0, "process_start":"gone", "harness":"codex", "model":"model", "effort":"high",
            "started_at":started_at, "ended_at":"2026-10-19T13:00:00Z", "stages":[],
            "outcome":"ready for review", "session_log":null, "cause":null, "issue_branch":"issue-7",
            "pull_request":"https://github.com/ACME/WIDGETS/pull/23/"
        });
        let path = folder.join(filename);
        let bytes = serde_json::to_vec(&record).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        records.push((path, bytes));
    }
    let run = score(&scenario);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let proposal = weeks(&scenario).remove(0)["bug_origins"][0]["pull_requests"][0]["run"].clone();
    assert_eq!(proposal["source"], "local");
    assert_eq!(proposal["command_record"], json!(records[1].0));
    assert_eq!(proposal["name"], "acme/widgets#7");
    let run = scenario.run(&["bug-origins", "8", "--confirm", "--json"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let confirmed: Value = serde_json::from_str(&run.stdout).unwrap();
    assert_eq!(confirmed[0]["settlement"]["pull_request"]["run"], proposal);
    let calls = scenario.gh_calls();
    let counts = || {
        let run = scenario.run(&["metrics", "widgets#7", "--json"]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        serde_json::from_str::<Value>(&run.stdout).unwrap()["command"]["metrics"]["confirmed_bug_origins"].clone()
    };
    assert_eq!(counts(), 1);
    for (path, bytes) in &records {
        assert_eq!(std::fs::read(path).unwrap(), *bytes);
    }
    std::fs::remove_file(&records[1].0).unwrap();
    assert_eq!(
        counts(),
        0,
        "an unavailable saved pin must not move to the older Continuation"
    );
    assert_eq!(scenario.gh_calls(), calls);
    assert_eq!(std::fs::read(&records[0].0).unwrap(), records[0].1);
}

#[test]
fn confirming_proposes_the_reviewed_diff_and_requires_explicit_eval_approval() {
    let (scenario, base, head) = proposed_origin();
    let unsettled = scenario.run(&["evals", "list"]);
    assert_eq!(
        serde_json::from_str::<Value>(&unsettled.stdout).unwrap(),
        json!([])
    );
    let confirmed = scenario.run(&["bug-origins", "8", "--confirm", "--json"]);
    assert_eq!(confirmed.code, Some(0), "{}", confirmed.stderr);
    let confirmed: Value = serde_json::from_str(&confirmed.stdout).unwrap();
    assert_eq!(confirmed[0]["state"], "confirmed");
    assert_eq!(
        confirmed[0]["settlement"]["pull_request"]["run"]["source"],
        "built_with"
    );
    let task = confirmed[0]["settlement"]["eval_task"]["task"]
        .as_str()
        .unwrap();
    let listed = scenario.run(&["evals", "list"]);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(listed[0]["tasks"][0]["state"], "proposed");
    let destination = scenario.path("prepared-review");
    let prepare = || {
        scenario.run(&[
            "evals",
            "prepare",
            "code-review",
            task,
            destination.to_str().unwrap(),
        ])
    };
    let blocked = prepare();
    assert_eq!(blocked.code, Some(1));
    assert!(
        blocked.stderr.contains("proposed Eval tasks"),
        "{}",
        blocked.stderr
    );
    let approved = scenario.run(&["evals", "approve", "code-review", task, "--json"]);
    assert_eq!(approved.code, Some(0), "{}", approved.stderr);
    let prepared = prepare();
    assert_eq!(prepared.code, Some(0), "{}", prepared.stderr);
    assert_eq!(
        std::fs::read_to_string(destination.join("answer")).unwrap(),
        "41\n"
    );
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&destination)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    assert_eq!(git(&["rev-parse", "HEAD"]), base);
    assert_eq!(git(&["log", "--all", "--format=%H"]), base);
    assert_ne!(base, head);
    let again = scenario.run(&["bug-origins", "8", "--confirm", "--json"]);
    assert_eq!(again.code, Some(0), "{}", again.stderr);
    assert_eq!(
        scenario
            .run(&["evals", "list"])
            .stdout
            .matches("approved")
            .count(),
        1
    );
}

#[test]
fn rejecting_an_origin_is_visible_and_proposes_no_eval_task() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["scoring_bugs"] = json!([bug(8, None, 1)]);
    scenario.write_gh_state(&state);
    assert_eq!(score(&scenario).code, Some(0));
    let proposed = scenario.run(&["bug-origins", "--json"]);
    assert_eq!(proposed.code, Some(0), "{}", proposed.stderr);
    let proposed: Value = serde_json::from_str(&proposed.stdout).unwrap();
    assert_eq!(proposed[0]["state"], "proposed");
    let rejected = scenario.run(&["bug-origins", "8", "--none", "--json"]);
    assert_eq!(rejected.code, Some(0), "{}", rejected.stderr);
    let rejected: Value = serde_json::from_str(&rejected.stdout).unwrap();
    assert_eq!(rejected[0]["state"], "rejected");
    assert_eq!(weeks(&scenario)[0]["bug_origins"][0]["state"], "rejected");
    let suites = scenario.run(&["evals", "list"]);
    assert_eq!(suites.code, Some(0), "{}", suites.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&suites.stdout).unwrap(),
        json!([])
    );
}

#[test]
fn correcting_an_origin_counts_only_against_its_run_and_rejection_removes_the_count() {
    let (scenario, _, _) = proposed_origin();
    let directory = scenario.path("home/.thirdshift/evals/scoring");
    let original = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let checkpoint = directory.join("noncanonical-bug-origins.json");
    std::fs::rename(original, &checkpoint).unwrap();
    scenario.agent_does("echo 41 > answer\ngit add answer\ngit commit -q -m 'Wrong answer'\ngh pr create --base main --head issue-7 --title 'Answer' --body 'Closes #7'");
    let run = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut state = scenario.gh_state();
    state["origin_review_prs"]["1"] = state["origin_review_prs"]["23"].clone();
    scenario.write_gh_state(&state);
    let details = || {
        let run = scenario.run(&["show", "widgets#7", "--json"]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        serde_json::from_str::<Value>(&run.stdout).unwrap()
    };
    assert_eq!(
        details()["command"]["run_metrics"]["confirmed_bug_origins"],
        0
    );
    let corrected = scenario.run(&[
        "bug-origins",
        &scenario.issue_url(8),
        "--correct",
        "1",
        "--json",
    ]);
    assert_eq!(corrected.code, Some(0), "{}", corrected.stderr);
    let corrected: Value = serde_json::from_str(&corrected.stdout).unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
    assert_eq!(saved["bug_origins"][0], corrected[0]);
    assert_eq!(corrected[0]["state"], "confirmed");
    assert_eq!(corrected[0]["settlement"]["corrected"], true);
    assert_eq!(
        corrected[0]["settlement"]["pull_request"]["url"],
        "https://github.com/acme/widgets/pull/1"
    );
    assert_eq!(
        corrected[0]["settlement"]["pull_request"]["run"]["source"],
        "local"
    );
    assert_eq!(
        details()["command"]["run_metrics"]["confirmed_bug_origins"],
        1
    );
    let rejected = scenario.run(&["bug-origins", "8", "--none"]);
    assert_eq!(rejected.code, Some(0), "{}", rejected.stderr);
    assert!(rejected.stdout.contains("rejected"));
    let saved: Value = serde_json::from_slice(&std::fs::read(checkpoint).unwrap()).unwrap();
    assert_eq!(saved["bug_origins"][0]["state"], "rejected");
    assert_eq!(
        details()["command"]["run_metrics"]["confirmed_bug_origins"],
        0
    );
    let listed = scenario.run(&["evals", "list"]);
    let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
    assert_eq!(listed[0]["tasks"], json!([]));
}
