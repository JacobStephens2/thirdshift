use super::*;

fn reviewer_regression(name: &str) {
    let output = Command::new("python3")
        .arg(format!("tests/evals/regressions/{name}.py"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("THIRDSHIFT_TEST_BINARY", env!("CARGO_BIN_EXE_thirdshift"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn preparation_preserves_executable_test_overlay_permissions() {
    reviewer_regression("executable_overlay");
}

#[test]
fn proposals_keep_the_complete_rebase_merged_ticket_range() {
    reviewer_regression("rebase_merge");
}

fn process_task(fixture: &mut Fixture, base_passes: bool) -> PathBuf {
    fs::create_dir_all(fixture.source.join("src")).unwrap();
    fs::write(
        fixture.source.join("src/counter.rs"),
        if base_passes { "pub const COUNT: u32 = 2;\n#[cfg(test)] mod tests { #[test] fn untouched() { assert!(true); } }\n" } else { "pub const COUNT: u32 = 1;\n#[cfg(test)] mod tests { #[test] fn untouched() { assert!(true); } }\n" },
    ).unwrap();
    git(&fixture.source, &["add", "."]);
    git(&fixture.source, &["commit", "-qm", "Counter base"]);
    fixture.base = git(&fixture.source, &["rev-parse", "HEAD"]);
    fs::write(fixture.source.join("src/counter.rs"), "pub const COUNT: u32 = 2;\n#[cfg(test)] mod tests { #[test] fn untouched() { assert!(true); } }\n").unwrap();
    fs::create_dir_all(fixture.source.join("tests")).unwrap();
    fs::write(fixture.source.join("tests/fake.py"), "EXPECTED = '2'\n").unwrap();
    fs::write(fixture.source.join("tests/regression.py"),
        "from pathlib import Path\nfrom fake import EXPECTED\nassert f'= {EXPECTED};' in Path('src/counter.rs').read_text(), 'counter regression'\n").unwrap();
    git(&fixture.source, &["add", "."]);
    git(
        &fixture.source,
        &["commit", "-qm", "Fix counter with separate regression"],
    );
    fixture.change = git(&fixture.source, &["rev-parse", "HEAD"]);
    let task = fixture.task("process", "approved");
    let manifest = task.join("task.toml");
    let text = fs::read_to_string(&manifest).unwrap().replace(
        "session = \"process\"",
        "session = \"process\"\nprovenance = \"real\"\nenvironment = \"host\"",
    );
    fs::write(manifest, format!("{text}\n[process_tests]\noverlay_files = [\"tests/regression.py\"]\nshared_fakes = [\"tests/fake.py\"]\nfail_to_pass = [[\"python3\", \"tests/regression.py\"]]\npass_to_pass = []\n")).unwrap();
    task
}

#[test]
fn preparation_screens_process_selectors_and_keeps_the_fix_private() {
    let mut fixture = Fixture::new();
    process_task(&mut fixture, false);
    success(fixture.prepare());
    assert!(
        fs::read_to_string(fixture.destination.join("src/counter.rs"))
            .unwrap()
            .starts_with("pub const COUNT: u32 = 1;\n")
    );
    assert!(!fixture.destination.join("tests/regression.py").exists());
    success(fixture.evals(&[
        "verify",
        fixture.destination.to_str().unwrap(),
        &fixture.base,
    ]));
}

#[test]
fn preparation_refuses_changed_inline_tests_naming_the_source_file() {
    let mut fixture = Fixture::new();
    let task = process_task(&mut fixture, false);
    let path = fixture.source.join("src/counter.rs");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, text.replace("assert!(true)", "assert_eq!(COUNT, 2)")).unwrap();
    git(&fixture.source, &["commit", "-qam", "Change inline tests"]);
    let change = git(&fixture.source, &["rev-parse", "HEAD"]);
    let manifest = task.join("task.toml");
    fs::write(
        &manifest,
        fs::read_to_string(&manifest)
            .unwrap()
            .replace(&fixture.change, &change),
    )
    .unwrap();
    let output = fixture.prepare();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("inline test") && diagnostic.contains("src/counter.rs"),
        "{diagnostic}"
    );
    assert!(!fixture.destination.exists());
}

#[test]
fn preparation_refuses_a_selector_that_passes_at_the_base() {
    let mut fixture = Fixture::new();
    process_task(&mut fixture, true);
    let output = fixture.prepare();
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("passes at base") && diagnostic.contains("tests/regression.py"),
        "{diagnostic}"
    );
    assert!(!fixture.destination.exists());
}

#[test]
fn proposals_include_only_merged_tickets_with_source_and_separate_tests() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::new();
    process_task(&mut fixture, false);
    git(
        &fixture.source,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widgets.git",
        ],
    );
    let ticket = serde_json::json!({
        "url": "https://github.com/acme/widgets/issues/7", "title": "Fix the counter", "body": "Count twice.",
        "labels": {"nodes": [{"name":"bug"}], "pageInfo":{"hasNextPage":false}},
        "subIssues": {"totalCount":0}
    });
    let candidate = |number, commit: &str, issues| {
        serde_json::json!({
            "number":number, "url":format!("https://github.com/acme/widgets/pull/{number}"),
            "baseRefName":"main", "headRefName":"issue-7", "mergeCommit":{"oid":commit},
            "closingIssuesReferences":{"nodes":issues, "pageInfo":{"hasNextPage":false}}
        })
    };
    let mut spec = ticket.clone();
    spec["subIssues"]["totalCount"] = serde_json::json!(2);
    let mut security = ticket.clone();
    security["labels"]["nodes"] = serde_json::json!([{"name":"security-fix"}]);
    let source = fixture.source.join("src/counter.rs");
    fs::write(
        &source,
        fs::read_to_string(&source)
            .unwrap()
            .replace("assert!(true)", "assert_eq!(COUNT, 2)"),
    )
    .unwrap();
    git(&fixture.source, &["commit", "-qam", "Inline test change"]);
    let inline = git(&fixture.source, &["rev-parse", "HEAD"]);
    let records = serde_json::json!({"data":{"repository":{"pullRequests":{"nodes":[
        candidate(17, &fixture.change, vec![ticket.clone()]),
        candidate(18, &fixture.change, vec![]),
        candidate(19, &fixture.change, vec![spec]),
        candidate(20, &fixture.later, vec![ticket.clone()]),
        candidate(21, &inline, vec![ticket.clone()]),
        candidate(22, &fixture.change, vec![security])
    ]}}}});
    let bin = fixture._root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let response = fixture._root.path().join("github.json");
    fs::write(&response, records.to_string()).unwrap();
    let gh = bin.join("gh");
    fs::write(
        &gh,
        "#!/usr/bin/env python3\nimport os\nprint(open(os.environ['FIXTURE_GITHUB']).read())\n",
    )
    .unwrap();
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let propose = || {
        Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .args([
                "evals",
                "propose-process",
                fixture.source.to_str().unwrap(),
                "--limit",
                "20",
            ])
            .env("HOME", &fixture.home)
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("FIXTURE_GITHUB", &response)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap()
    };
    let proposed = success(propose());
    assert_eq!(
        proposed["proposals"].as_array().unwrap().len(),
        1,
        "{proposed}"
    );
    assert_eq!(
        proposed["skipped"].as_array().unwrap().len(),
        5,
        "{proposed}"
    );
    let entry = &proposed["proposals"][0];
    assert_eq!(entry["number"], 17);
    assert_eq!(entry["kind"], "bug");
    assert_eq!(entry["non_test_lines"], 2);
    let task_name = entry["task"].as_str().unwrap();
    let task = fixture.suite("process").join("tasks").join(task_name);
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(task.join("task.toml")).unwrap()).unwrap();
    assert_eq!(manifest["state"].as_str(), Some("proposed"));
    assert_eq!(manifest["base"].as_str(), Some(fixture.base.as_str()));
    assert_eq!(
        manifest["process_tests"]["shared_fakes"][0].as_str(),
        Some("tests/fake.py")
    );
    assert!(
        fs::read_to_string(task.join("issue.md"))
            .unwrap()
            .contains("Count twice.")
    );
    assert_eq!(success(propose())["proposals"][0]["task"], task_name);
    for path in [
        "processes/ticket-run.toml",
        "skills/thirdshift-security-audit/SKILL.md",
        "skills/thirdshift-implement/SKILL.md",
        "src/prompt.rs",
        "src/harness/codex.rs",
    ] {
        assert!(
            success(fixture.evals(&["touched", path]))
                .as_array()
                .unwrap()
                .iter()
                .any(|name| name == "process"),
            "{path}"
        );
    }
    success(fixture.evals(&["approve", "process", task_name]));
    success(propose());
    let listed = success(fixture.evals(&["list"]));
    let suite = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|suite| suite["name"] == "process")
        .unwrap();
    assert_eq!(suite["trials_per_arm"], 3);
    assert_eq!(suite["tasks"][0]["state"], "approved");
    success(fixture.evals(&[
        "prepare",
        "process",
        task_name,
        fixture.destination.to_str().unwrap(),
    ]));
}
