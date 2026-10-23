//! Codex's native session file through the binary, Session trace and Console.
use super::*;

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";
const FIRST: &str = "--- a/widgets.rs\n+++ b/widgets.rs\n@@ -1 +1 @@\n-old\n+middle\n";
const SECOND: &str =
    "--- a/widgets.rs\n+++ b/widgets.rs\n@@ -1 +1 @@\n-middle\n+<script>new()</script>\n";

fn native(patches: &[Value]) -> String {
    [
        json!({"type":"session_meta","payload":{"id":"fake-thread-1"}}),
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-native"}}),
    ]
    .into_iter()
    .chain(patches.iter().enumerate().map(|(index, changes)| {
        json!({"timestamp":"2026-10-21T12:00:00Z","type":"event_msg","payload":{
            "type":"patch_apply_end","call_id":format!("native-{index}"),
            "success":true,"status":"completed","changes":changes
        }})
    }))
    .map(|record| format!("{record}\n"))
    .collect()
}

fn native_setup(retained: &str) -> String {
    format!(
        r#"
mkdir -p "${{CODEX_HOME:-$HOME/.codex}}/sessions/2026/10/21"
cat > "${{CODEX_HOME:-$HOME/.codex}}/sessions/2026/10/21/rollout-2026-10-21T12-00-00-fake-thread-1.jsonl" <<'NATIVE'
{retained}NATIVE
"#
    )
}

fn run(scenario: &support::Scenario, retained: &str, edits: &[Value]) {
    run_with_setup(scenario, &native_setup(retained), edits, &[]);
}

fn run_with_setup(
    scenario: &support::Scenario,
    setup: &str,
    edits: &[Value],
    env: &[(&str, &str)],
) {
    let stream: String = edits
        .iter()
        .enumerate()
        .map(|(index, changes)| {
            format!("{}\n", json!({"type":"item.completed","item":{
                "id":format!("stream-{index}"),"type":"file_change","status":"completed","changes":changes
            }}))
        })
        .collect();
    scenario.agent_does(&format!(
        r#"
{setup}
cat <<'STREAM'
{stream}STREAM
echo shell > shell.txt
echo '{{"type":"item.completed","item":{{"id":"shell","type":"command_execution","command":"echo shell > shell.txt","status":"completed","exit_code":0}}}}'
echo feature > feature.txt
git add feature.txt shell.txt
git commit -qm 'Add feature'
gh pr create --base main --head issue-7 --title Feature --body 'Closes #7'
"#
    ));
    let result = scenario.run_with_env(&["harness", "codex", &scenario.issue_url(7)], env);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
}

fn amendments(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter_map(|record| {
            let event: Value =
                serde_json::from_str(record["body"]["stringValue"].as_str()?).ok()?;
            (event["type"] == "tool_call_ended"
                && event["result"]["source"] == "codex_session_file")
                .then_some(event)
        })
        .collect()
}

fn trace(scenario: &support::Scenario) -> Vec<Value> {
    let name = scenario
        .entries(LOGS)
        .into_iter()
        .find(|name| name.ends_with(".logs.jsonl"))
        .unwrap();
    fs::read_to_string(scenario.path(LOGS).join(name))
        .unwrap()
        .lines()
        .flat_map(|line| {
            let request: Value = serde_json::from_str(line).unwrap();
            request["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect()
}

fn page(scenario: &support::Scenario) -> String {
    configure(&scenario.path("home"));
    let console = Console::start(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        &scenario.path("home"),
    );
    let inventory: Value = serde_json::from_str(&scenario.run(&["logs", "--json"]).stdout).unwrap();
    let id = inventory["logs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|log| log["kind"] == "Session trace")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    console.get(&format!("/logs/{id}")).text().unwrap()
}

#[test]
fn each_codex_patch_keeps_its_own_diff_in_the_trace_and_console() {
    let scenario = support::Scenario::new();
    let retained = native(&[
        json!({"widgets.rs":{"type":"update","unified_diff":FIRST,"move_path":null}}),
        json!({"widgets.rs":{"type":"update","unified_diff":SECOND,"move_path":null}}),
    ]);
    run(
        &scenario,
        &retained,
        &[
            json!([{"path":"widgets.rs","kind":"update"}]),
            json!([{"path":"widgets.rs","kind":"update"}]),
        ],
    );
    let records = trace(&scenario);
    let amendments = amendments(&records);
    assert_eq!(amendments.len(), 2, "{records:?}");
    for (index, diff) in [FIRST, SECOND].into_iter().enumerate() {
        assert_eq!(amendments[index]["id"], format!("stream-{index}"));
        assert_eq!(amendments[index]["result"]["changes"][0]["diff"], diff);
        assert_eq!(amendments[index]["amended"], true);
    }
    let html = page(&scenario);
    assert!(html.contains("-old\n+middle"), "{html}");
    assert!(
        html.contains("-middle\n+&#60;script&#62;new()&#60;/script&#62;"),
        "{html}"
    );
    assert_eq!(html.matches("Codex’s record of this edit").count(), 2);
    assert!(!html.contains("Patch unavailable in this log."));
    assert!(!html.contains("<script>new()</script>"));
}

#[test]
fn mismatched_or_unavailable_native_files_leave_every_edit_unchanged() {
    let a = json!({"a.rs":{"type":"update","unified_diff":FIRST,"move_path":null}});
    let b = json!({"b.rs":{"type":"update","unified_diff":SECOND,"move_path":null}});
    let edits = [
        json!([{"path":"a.rs","kind":"update"}]),
        json!([{"path":"b.rs","kind":"update"}]),
    ];
    let valid = native(&[a.clone(), b.clone()]);
    let fixtures = [
        ("missing", String::new()),
        (
            "unreadable",
            format!(
                "{}\nchmod 000 \"$HOME/.codex/sessions/2026/10/21/rollout-2026-10-21T12-00-00-fake-thread-1.jsonl\"\n",
                native_setup(&valid)
            ),
        ),
        (
            "wrong order",
            native_setup(&native(&[b.clone(), a.clone()])),
        ),
        (
            "wrong path after a matching edit",
            native_setup(&native(&[
                a.clone(),
                json!({"other.rs":{"type":"update","unified_diff":SECOND}}),
            ])),
        ),
        (
            "wrong count",
            native_setup(&native(std::slice::from_ref(&a))),
        ),
        (
            "wrong thread",
            native_setup(&valid.replace("fake-thread-1", "another-thread")),
        ),
        (
            "changed format",
            native_setup(&valid.replace("\"unified_diff\"", "\"future_diff\"")),
        ),
        ("truncated", native_setup(&format!("{valid}{{not json\n"))),
    ];
    for (case, setup) in fixtures {
        let scenario = support::Scenario::new();
        run_with_setup(&scenario, &setup, &edits, &[]);
        let records = trace(&scenario);
        assert!(amendments(&records).is_empty(), "{case}");
        let html = page(&scenario);
        assert_eq!(
            html.matches("Patch unavailable in this log.").count(),
            2,
            "{case}"
        );
        assert!(!html.contains("class=\"diff\""), "{case}");
    }
}

#[test]
fn codex_adds_and_deletes_are_diffs_while_shell_edits_have_no_diff() {
    let scenario = support::Scenario::new();
    run(
        &scenario,
        &native(&[json!({
            "added.txt":{"type":"add","content":"hello\nworld"},
            "deleted.txt":{"type":"delete","content":"gone\n"}
        })]),
        &[json!([
            {"path":"deleted.txt","kind":"delete"},
            {"path":"added.txt","kind":"add"}
        ])],
    );
    let records = trace(&scenario);
    let amended = amendments(&records);
    assert_eq!(amended.len(), 1);
    assert_eq!(
        amended[0]["result"]["changes"][0]["diff"],
        "--- a/deleted.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-gone\n"
    );
    assert_eq!(
        amended[0]["result"]["changes"][1]["diff"],
        "--- /dev/null\n+++ b/added.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n\\ No newline at end of file\n"
    );
    let shell: Vec<Value> = records
        .iter()
        .flat_map(|record| record["attributes"].as_array().into_iter().flatten())
        .filter_map(|attribute| {
            if attribute["key"] != "thirdshift.event" {
                return None;
            }
            let event: Value =
                serde_json::from_str(attribute["value"]["stringValue"].as_str()?).ok()?;
            (event["type"] == "tool_call_ended" && event["id"] == "shell").then_some(event)
        })
        .collect();
    assert_eq!(shell.len(), 1);
    assert_eq!(shell[0]["tool"], "command_execution");
    assert!(shell[0]["result"].get("source").is_none());
    assert!(shell[0]["result"].get("diff").is_none());
    let html = page(&scenario);
    assert!(html.contains("-gone\n"));
    assert!(html.contains("+hello\n+world\n\\ No newline at end of file"));
    assert_eq!(html.matches("class=\"diff\"").count(), 2);
}

#[test]
fn a_custom_codex_home_supplies_only_the_latest_turn_and_matches_absolute_paths() {
    let scenario = support::Scenario::new();
    let root = scenario.path("custom-codex");
    let retained = format!(
        "{}{}\n{}\n",
        native(&[json!({"previous.rs":{"type":"update","unified_diff":FIRST}})]),
        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"resumed"}}),
        json!({"type":"event_msg","payload":{"type":"patch_apply_end","call_id":"current","success":true,"status":"completed",
            "changes":{"WORKTREE/widgets.rs":{"type":"update","unified_diff":SECOND,"move_path":null}}
        }})
    );
    let setup = format!(
        "{}\npatch_fixture=$(cat \"$CODEX_HOME/sessions/2026/10/21/rollout-2026-10-21T12-00-00-fake-thread-1.jsonl\")\nprintf '%s\\n' \"${{patch_fixture//WORKTREE/$PWD}}\" > \"$CODEX_HOME/sessions/2026/10/21/rollout-2026-10-21T12-00-00-fake-thread-1.jsonl\"\n",
        native_setup(&retained)
    );
    run_with_setup(
        &scenario,
        &setup,
        &[json!([{"path":"widgets.rs","kind":"update"}])],
        &[("CODEX_HOME", root.to_str().unwrap())],
    );
    let amended = amendments(&trace(&scenario));
    assert_eq!(amended.len(), 1);
    assert_eq!(amended[0]["result"]["changes"][0]["diff"], SECOND);
    assert!(!scenario.path("home/.codex").exists());
}

#[test]
fn ambiguous_native_files_in_different_date_folders_supply_no_diff() {
    let scenario = support::Scenario::new();
    let retained = native(&[json!({"widgets.rs":{"type":"update","unified_diff":FIRST}})]);
    let setup = format!(
        r#"{}
mkdir -p "$HOME/.codex/sessions/2026/10/22"
cp "$HOME/.codex/sessions/2026/10/21/rollout-2026-10-21T12-00-00-fake-thread-1.jsonl" "$HOME/.codex/sessions/2026/10/22/rollout-first-fake-thread-1.jsonl"
cp "$HOME/.codex/sessions/2026/10/21/rollout-2026-10-21T12-00-00-fake-thread-1.jsonl" "$HOME/.codex/sessions/2026/10/22/rollout-second-fake-thread-1.jsonl"
"#,
        native_setup(&retained)
    );
    run_with_setup(
        &scenario,
        &setup,
        &[json!([{"path":"widgets.rs","kind":"update"}])],
        &[],
    );
    assert!(amendments(&trace(&scenario)).is_empty());
    assert!(page(&scenario).contains("Patch unavailable in this log."));
}
