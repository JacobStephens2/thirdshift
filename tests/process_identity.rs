//! Process provenance through the CLI's Command records and Session traces.
mod support;

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use support::Scenario;

const ISSUE_LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";
const PROCESS_LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/process";
const REVIEW: &str = "\
version = 1
description = \"Review\"
[[steps]]
name = \"review\"
skill = \"code-review\"
[[steps]]
name = \"confirm\"
skill = \"code-review\"
";

fn write_process(scenario: &Scenario, source: &str) {
    let directory = scenario.path("home/.thirdshift/processes");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("notes.toml"), source).unwrap();
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn ticket_hash() -> String {
    hash(include_bytes!("../processes/ticket-run.toml"))
}

fn opens_pr(issue: u32, base: &str) -> String {
    format!(
        "echo work > work-{issue}.txt\ngit add .\ngit commit -qm 'Do work'\ngh pr create --base {base} --head issue-{issue} --title Work --body 'Closes #{issue}'\n"
    )
}

fn assert_record(record: &Value, name: &str, kind: &str, hash: Option<&str>) {
    assert_eq!(record["process_name"], name, "{record}");
    assert_eq!(record["process_kind"], kind, "{record}");
    assert_eq!(record["process_hash"].as_str(), hash, "{record}");
}

fn assert_trace(path: &std::path::Path, name: &str, kind: &str, hash: Option<&str>) {
    let mut resources = 0;
    for line in fs::read_to_string(path).unwrap().lines() {
        let request: Value = serde_json::from_str(line).unwrap();
        let batches = request["resourceLogs"]
            .as_array()
            .or_else(|| request["resourceSpans"].as_array())
            .unwrap();
        for batch in batches {
            let attributes = batch["resource"]["attributes"].as_array().unwrap();
            let get = |key| {
                attributes
                    .iter()
                    .find(|attr| attr["key"] == key)
                    .and_then(|attr| attr["value"]["stringValue"].as_str())
            };
            assert_eq!(get("thirdshift.process.name"), Some(name), "{batch}");
            assert_eq!(get("thirdshift.process.kind"), Some(kind), "{batch}");
            assert_eq!(get("thirdshift.process.hash"), hash, "{batch}");
            resources += 1;
        }
    }
    assert!(resources > 0, "no Session trace resources in {path:?}");
}

fn traces(scenario: &Scenario, folder: &str) -> Vec<std::path::PathBuf> {
    let paths: Vec<_> = scenario
        .entries(folder)
        .into_iter()
        .filter(|name| name.ends_with(".logs.jsonl") || name.ends_with(".spans.jsonl"))
        .map(|name| scenario.path(folder).join(name))
        .collect();
    assert!(!paths.is_empty(), "no Session traces in {folder}");
    paths
}

#[test]
fn architect_run_records_its_compiled_process_in_records_and_traces() {
    let scenario = Scenario::new();
    scenario.repo_has_labels(&["needs-triage", "ready-for-agent"]);
    scenario.agent_does(
        "url=$(gh issue create --title 'Deepen sessions' --body Plan --label needs-triage)\nprintf 'Architecture review plan: %s\\n' \"$url\" > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
    );
    let result = scenario.run(&["architect", "--plan-only"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let folder = "home/.thirdshift/logs/acme/widgets/commands/architect";
    let expected_hash = hash(include_bytes!("../processes/architect-run.toml"));
    assert_record(
        &scenario.command_record(folder),
        "architect-run",
        "built-in",
        Some(&expected_hash),
    );
    for path in traces(&scenario, folder) {
        assert_trace(&path, "architect-run", "built-in", Some(&expected_hash));
    }
}

#[test]
fn security_run_records_its_compiled_process_in_records_and_traces() {
    let scenario = Scenario::new();
    scenario.agent_does(
        r#"
printf '%s' "$FAKE_CLAUDE_PROMPT" > prompt.txt
output=$(sed -n 's/^Output directory: `\(.*\)`\.$/\1/p' prompt.txt)
test -n "$output"
printf '%s\n' '[]' > "$output/findings.json"
printf '%s\n' '[]' > "$output/coverage-ledger.json"
printf '%s\n' '{"run_status":"complete"}' > "$output/run-metadata.json"
printf '%s\n' 'Security audit: complete' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    let result = scenario.run(&["secure"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let folder = "home/.thirdshift/logs/acme/widgets/commands/secure";
    let expected_hash = hash(include_bytes!("../processes/security-run.toml"));
    assert_record(
        &scenario.command_record(folder),
        "security-run",
        "built-in",
        Some(&expected_hash),
    );
    for path in traces(&scenario, folder) {
        assert_trace(&path, "security-run", "built-in", Some(&expected_hash));
    }
}

#[test]
fn ticket_run_records_its_compiled_process_in_records_and_traces() {
    let scenario = Scenario::new();
    scenario.agent_does(&opens_pr(7, "main"));
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_record(
        &scenario.command_record(ISSUE_LOGS),
        "ticket-run",
        "built-in",
        Some(&ticket_hash()),
    );
    for path in traces(&scenario, ISSUE_LOGS) {
        assert_trace(&path, "ticket-run", "built-in", Some(&ticket_hash()));
    }
}

#[test]
fn custom_process_hashes_the_loaded_bytes_and_changes_on_the_next_run() {
    let scenario = Scenario::new();
    scenario.agent_does(
        "printf '\\n# edited while running\\n' >> \"$HOME/.thirdshift/processes/notes.toml\"\n\
         printf Done > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    );
    let mut seen = std::collections::HashSet::new();
    for source in [REVIEW.to_string(), REVIEW.replace('\n', "\r\n")] {
        write_process(&scenario, &source);
        let result = scenario.run(&["process", "notes"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let new_files: Vec<_> = scenario
            .entries(PROCESS_LOGS)
            .into_iter()
            .filter(|name| seen.insert(name.clone()))
            .map(|name| scenario.path(PROCESS_LOGS).join(name))
            .collect();
        let record_path = new_files
            .iter()
            .find(|path| path.to_string_lossy().ends_with(".command.json"))
            .unwrap();
        let record: Value = serde_json::from_slice(&fs::read(record_path).unwrap()).unwrap();
        let expected_hash = hash(source.as_bytes());
        assert_record(&record, "notes", "custom", Some(&expected_hash));
        let mut count = 0;
        for path in new_files.iter().filter(|path| {
            let name = path.to_string_lossy();
            name.ends_with(".logs.jsonl") || name.ends_with(".spans.jsonl")
        }) {
            assert_trace(path, "notes", "custom", Some(&expected_hash));
            count += 1;
        }
        assert_eq!(count, 4, "two Steps retain logs and spans");
    }
}

#[test]
fn a_custom_process_and_its_spec_and_ticket_keep_separate_process_identities() {
    let scenario = Scenario::new();
    let source = "\
version = 1
description = \"Publish a Spec, run its Ticket, then review\"
[[steps]]
name = \"publish\"
skill = \"to-spec\"
[[steps]]
name = \"run\"
catalogue = \"run-published-issue\"
[[steps]]
name = \"review\"
skill = \"code-review\"
";
    write_process(&scenario, source);
    scenario.agent_does("printf Done > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
    scenario.agent_does_in_session(
        1,
        r#"
spec=$(gh issue create --title Spec --body 'Build widgets')
ticket=$(gh issue create --title Ticket --body 'Build one widget')
gh api --method POST repos/acme/widgets/issues/${spec##*/}/sub_issues -F sub_issue_id=${ticket##*/}
printf '%s\n' "$spec" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(9, &opens_pr(9, "issue-8"));
    let result = scenario.run(&["process", "notes"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let record = scenario.command_record(PROCESS_LOGS);
    assert_record(&record, "notes", "custom", Some(&hash(source.as_bytes())));
    let spec = &record["dispatched"][0];
    assert_eq!(spec["kind"], "Spec run");
    let spec_hash = hash(include_bytes!("../processes/spec-run.toml"));
    assert_record(spec, "spec-run", "built-in", Some(&spec_hash));
    let children: Vec<_> = scenario
        .entries(PROCESS_LOGS)
        .into_iter()
        .filter(|name| name.ends_with(".child.json"))
        .collect();
    assert_eq!(children.len(), 1, "{children:?}");
    let ticket: Value =
        serde_json::from_slice(&fs::read(scenario.path(PROCESS_LOGS).join(&children[0])).unwrap())
            .unwrap();
    assert_record(&ticket, "ticket-run", "built-in", Some(&ticket_hash()));
    let mut counts = [0; 3];
    for path in traces(&scenario, PROCESS_LOGS) {
        let name = path.file_name().unwrap().to_str().unwrap();
        if name.contains("-spec-review.") {
            assert_trace(&path, "spec-run", "built-in", Some(&spec_hash));
            counts[0] += 1;
        } else if name.starts_with("9-") {
            assert_trace(&path, "ticket-run", "built-in", Some(&ticket_hash()));
            counts[1] += 1;
        } else {
            assert_trace(&path, "notes", "custom", Some(&hash(source.as_bytes())));
            counts[2] += 1;
        }
    }
    assert_eq!(counts, [2, 2, 4], "Spec, Ticket and Custom Session traces");
}

#[test]
fn records_without_process_metadata_or_with_only_a_custom_name_still_read() {
    for custom in [false, true] {
        let scenario = Scenario::new();
        let (folder, kind, label) = if custom {
            write_process(&scenario, REVIEW);
            scenario.agent_does("printf Done > \"$FAKE_CLAUDE_FINAL_MESSAGE\"");
            let result = scenario.run(&["process", "notes"]);
            assert_eq!(result.code, Some(0), "{}", result.stderr);
            (PROCESS_LOGS, "Process", "Process notes on acme/widgets")
        } else {
            scenario.agent_does(&opens_pr(7, "main"));
            let result = scenario.run(&[&scenario.issue_url(7)]);
            assert_eq!(result.code, Some(0), "{}", result.stderr);
            (ISSUE_LOGS, "Run", "acme/widgets#7")
        };
        let name = scenario
            .entries(folder)
            .into_iter()
            .find(|name| name.ends_with(".command.json"))
            .unwrap();
        let path = scenario.path(folder).join(name);
        let mut legacy = scenario.command_record(folder);
        let fields = legacy.as_object_mut().unwrap();
        fields.remove("process_kind");
        fields.remove("process_hash");
        if !custom {
            fields.remove("process_name");
        }
        fs::write(path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let shown = scenario.run(&["show", legacy["name"].as_str().unwrap(), "--json"]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
        assert_eq!(shown["command"]["kind"], kind);
        assert_eq!(shown["command"]["process_name"], legacy["process_name"]);
        assert!(shown["command"]["process_kind"].is_null());
        assert!(shown["command"]["process_hash"].is_null());
        let status = scenario.run(&["status"]);
        assert_eq!(status.code, Some(0), "{}", status.stderr);
        assert!(status.stdout.contains(label), "{}", status.stdout);
    }
}

#[test]
fn a_parallel_custom_process_resume_keeps_its_own_process_identity() {
    let scenario = Scenario::new();
    let source = r#"
version = 1
description = "Dispatch a Ticket alongside a sibling session"

[worktrees.publish]
kind = "throwaway"
[worktrees.side]
kind = "throwaway"

[[steps]]
name = "publish"
skill = "to-spec"
worktree = "publish"

[[steps]]
name = "run"
catalogue = "run-published-issue"
worktree = "publish"
parallel = "after"

[[steps]]
name = "side"
skill = "code-review"
worktree = "side"
parallel = "after"
"#;
    write_process(&scenario, source);
    scenario.agent_does_in_session(
        1,
        r#"
url=$(gh issue create --title Ticket --body 'Build one widget')
printf '%s\n' "$url" > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#,
    );
    scenario.agent_does_for(
        8,
        &format!(
            r#"
touch "$HOME/ticket-active"
for attempt in $(seq 1 1000); do
  test -f "$HOME/side-resumed" && break
  sleep 0.01
done
test -f "$HOME/side-resumed"
{}
"#,
            opens_pr(8, "main")
        ),
    );
    scenario.agent_does(&format!(
        r#"
if test -f "$HOME/side-first"; then
  touch "$HOME/side-resumed"
  printf Done > "$FAKE_CLAUDE_FINAL_MESSAGE"
else
  touch "$HOME/side-first"
  for attempt in $(seq 1 1000); do
    test -f "$HOME/ticket-active" && break
    sleep 0.01
  done
  test -f "$HOME/ticket-active"
  printf Waiting > "$FAKE_CLAUDE_FINAL_MESSAGE"
  {}
fi
"#,
        support::leaves_running("sibling work")
    ));

    let result = scenario.run(&["process", "notes"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let expected_hash = hash(source.as_bytes());
    assert_record(
        &scenario.command_record(PROCESS_LOGS),
        "notes",
        "custom",
        Some(&expected_hash),
    );
    let resumed: Vec<_> = traces(&scenario, PROCESS_LOGS)
        .into_iter()
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("-process-step-3-resume.")
        })
        .collect();
    assert_eq!(resumed.len(), 2, "logs and spans for the sibling Resume");
    for path in resumed {
        assert_trace(&path, "notes", "custom", Some(&expected_hash));
    }
}
