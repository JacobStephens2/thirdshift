use super::*;

#[test]
fn a_sandboxed_model_grader_sees_only_its_packet() {
    if !std::process::Command::new("bwrap")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        println!("SKIPPED (sandboxed Grader): bwrap is not on PATH");
        return;
    }
    let fixture = Fixture::new();
    // Finish Trials through the pass-through launcher, leaving grading pending.
    fs::write(fixture.scenario.path("claude-script.sh.grader"), "exit 1\n").unwrap();
    assert_eq!(fixture.run(&[]).code, Some(1));
    let experiment = fixture.experiment();
    let directory = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        experiment["id"].as_str().unwrap()
    ));
    let grader: Value =
        serde_json::from_slice(&fs::read(directory.join("grader.json")).unwrap()).unwrap();
    let scope_marker = fixture.scenario.path("model-grader-scope-started");
    fs::write(
        fixture.scenario.path("bin/systemd-run"),
        format!(
            "#!/bin/sh\ntouch {:?}\nwhile [ \"$#\" -gt 0 ] && [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"\n",
            scope_marker
        ),
    )
    .unwrap();
    let binary = PathBuf::from(grader["harness"]["path"].as_str().unwrap());
    fs::write(&binary, format!(r##"#!/bin/sh
set -eu
test ! -r '{directory}/experiment.json'
test ! -e '{directory}/trials'
test ! -e '{directory}/results'
test ! -e '{host}'
python3 - <<'PY'
import json, os
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
assert len(packet['trials']) == 2
assert open('/proc/self/oom_score_adj').read().strip() == '1000'
json.dump({{'scores': [dict(label=t['label'], passed=True, reason='Isolated packet.', wrong_finding=False, borderline=False) for t in packet['trials']]}}, open(os.environ['THIRDSHIFT_GRADER_SCORES'], 'w'))
PY
echo '{{"type":"result","subtype":"success","result":"graded"}}'
"##, directory = directory.display(), host = fixture.scenario.launch_dir().display())).unwrap();
    let resumed = fixture.scenario.run(&["experiment", "candidate", "--json"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let completed = fixture.experiment();
    assert_eq!(completed["state"], "completed");
    assert!(
        scope_marker.is_file(),
        "Model Grader bypassed its selected scope launcher"
    );
    for result in completed["results"].as_array().unwrap() {
        assert_eq!(result["score"]["model"]["reason"], "Isolated packet.");
    }
}

#[test]
fn model_grader_output_stays_in_private_artifacts() {
    let fixture = Fixture::new();
    let script = fixture.scenario.path("claude-script.sh.grader");
    let valid = fs::read_to_string(&script).unwrap();
    fs::write(
        &script,
        format!(
            "{valid}\nprintf 'private-model-grader-evidence' >&2\nprintf 'private-model-grader-evidence' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n"
        ),
    )
    .unwrap();
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(!run.stdout.contains("private-model-grader-evidence"));
    assert!(!run.stderr.contains("private-model-grader-evidence"));
    let graders = fixture.grader_calls();
    assert_eq!(graders.len(), 1);
    let directory = PathBuf::from(graders[0]["cwd"].as_str().unwrap());
    for file in ["stderr", "stdout.logs.jsonl"] {
        assert!(
            fs::read_to_string(directory.join(file))
                .unwrap()
                .contains("private-model-grader-evidence")
        );
    }
    let folder = "home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment";
    for log in fixture.scenario.log_files(folder, "log") {
        let log = fs::read_to_string(fixture.scenario.path(folder).join(log)).unwrap();
        assert!(!log.contains("private-model-grader-evidence"), "{log}");
    }
}

#[test]
fn model_grader_timeout_keeps_trace_and_resume_reuses_trials() {
    let fixture = Fixture::new();
    let script = fixture.scenario.path("claude-script.sh.grader");
    let valid = fs::read_to_string(&script).unwrap();
    fs::write(&script, format!("sleep 3\n{valid}")).unwrap();
    let paused = fixture.run(&["--timeout", "1"]);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    assert_eq!(fixture.experiment()["state"], "paused");
    assert_eq!(fixture.trial_calls().len(), 2);
    let graders = fixture.grader_calls();
    assert_eq!(graders.len(), 1);
    let directory = PathBuf::from(graders[0]["cwd"].as_str().unwrap());
    for file in ["stdout.logs.jsonl", "stdout.spans.jsonl"] {
        assert!(
            !fs::read_to_string(directory.join(file)).unwrap().is_empty(),
            "timeout lost Grader evidence in {file}"
        );
    }
    fs::write(&script, valid).unwrap();
    let resumed = fixture.run(&["--timeout", "1"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    assert_eq!(fixture.experiment()["state"], "completed");
    assert_eq!(fixture.trial_calls().len(), 2, "resume reran Trials");
    assert_eq!(fixture.grader_calls().len(), 2);
    let folder = "home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment";
    let records = fixture.scenario.log_files(folder, "json");
    assert_eq!(records.len(), 2);
    let mut grader_ids = Vec::new();
    for file in records {
        let record: Value =
            serde_json::from_slice(&fs::read(fixture.scenario.path(folder).join(file)).unwrap())
                .unwrap();
        let spans = export_spans(&fixture, &record);
        let graders: Vec<_> = spans
            .iter()
            .filter(|span| span["name"] == "invoke_agent model Grader")
            .collect();
        assert_eq!(graders.len(), 1, "each Command must export its own Grader");
        grader_ids.push(graders[0]["spanId"].clone());
    }
    assert_ne!(grader_ids[0], grader_ids[1]);
}

#[test]
fn model_grader_sessions_keep_otlp_traces() {
    let fixture = Fixture::new();
    let run = fixture.run(&["--prior", "0.01"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let graders = fixture.grader_calls();
    assert_eq!(graders.len(), 1);
    let directory = PathBuf::from(graders[0]["cwd"].as_str().unwrap());
    let files: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    for (suffix, resource_key) in [
        (".logs.jsonl", "resourceLogs"),
        (".spans.jsonl", "resourceSpans"),
    ] {
        let matching: Vec<_> = files
            .iter()
            .filter(|path| path.to_string_lossy().ends_with(suffix))
            .collect();
        assert!(
            !matching.is_empty(),
            "Grader session has no {suffix} Session trace in {}",
            directory.display()
        );
        let mut requests = 0;
        for path in matching {
            for line in fs::read_to_string(path).unwrap().lines() {
                let request: Value = serde_json::from_str(line).unwrap();
                assert!(request[resource_key].is_array(), "{request}");
                requests += 1;
            }
        }
        assert!(requests > 0, "Grader {suffix} trace is empty");
    }
    let record = fixture
        .scenario
        .command_record("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment");
    let spans = export_spans(&fixture, &record);
    let graders: Vec<_> = spans
        .iter()
        .filter(|span| span["name"] == "invoke_agent model Grader")
        .collect();
    assert_eq!(graders.len(), 1, "model Grader missing from Command Trace");
    assert!(
        spans.iter().any(|span| {
            span["name"] == "Trials" && span["spanId"] == graders[0]["parentSpanId"]
        })
    );
}

fn export_spans(fixture: &Fixture, record: &Value) -> Vec<Value> {
    let export = fixture
        .scenario
        .path(&format!("exports/{}", record["name"].as_str().unwrap()));
    let exported = fixture.scenario.run(&[
        "logs",
        record["name"].as_str().unwrap(),
        "--export",
        export.to_str().unwrap(),
    ]);
    assert_eq!(exported.code, Some(0), "{}", exported.stderr);
    trace_spans(&export.join("spans.jsonl"))
}

fn assert_metadata_table_is_blinded(table: &str) {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&format!(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
case "$PWD" in
  *--candidate--*) identity='candidate'; model='other-private-model' ;;
  *) identity='baseline'; model='private-model' ;;
esac
printf '{table}' "$model" "$identity" > "$reports/standards.md"
printf 'Reported the invented finding.\n' > "$reports/spec.md"
"#));
    let run = fixture.run(&[
        "--baseline-model",
        "private-model",
        "--candidate-model",
        "other-private-model",
        "--prior",
        "0.01",
    ]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let packet = fixture.grader_packet();
    assert_eq!(packet["trials"].as_array().unwrap().len(), 2);
    let text = packet.to_string().to_ascii_lowercase();
    for identity in ["baseline", "candidate", "claude", "private-model", "max"] {
        assert!(
            !text.contains(identity),
            "Grader packet leaked identifying metadata {identity}: {packet}"
        );
    }
}

#[test]
fn metadata_rows_beneath_generic_table_headers_are_blinded() {
    assert_metadata_table_is_blinded(
        r"| Field | Value |\n| --- | --- |\n| Harness | claude |\n| Model | %s |\n| Effort | max |\n| Arm | %s |\n",
    );
}

#[test]
fn metadata_rows_beneath_setting_value_headers_are_blinded() {
    assert_metadata_table_is_blinded(
        r"| Field | Setting |\n| --- | --- |\n| Harness | claude |\n| Model | %s |\n| Effort | max |\n| Arm | %s |\n",
    );
}

#[test]
fn finding_tables_preserve_harness_model_effort_and_arm_evidence() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
cat > "$reports/spec.md" <<'EOF'
| Area | Finding |
| --- | --- |
| Harness | lookup accepts a directory as an executable. |
| Model | selection accepts invalid input. |
| Effort | validation is missing. |
| Arm | setup can accidentally reuse credentials. |
EOF
"#);
    let run = fixture.run(&["--prior", "0.01"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let packet = fixture.grader_packet();
    for trial in packet["trials"].as_array().unwrap() {
        let report = trial["spec"].as_str().unwrap();
        for finding in [
            "lookup accepts a directory as an executable.",
            "selection accepts invalid input.",
            "validation is missing.",
            "setup can accidentally reuse credentials.",
        ] {
            assert!(
                report.contains(finding),
                "Blinding erased genuine finding {finding}: {report}"
            );
        }
    }
}

#[test]
fn horizontal_metadata_tables_are_blinded() {
    assert_metadata_table_is_blinded(
        r"| Harness | Model | Effort | Arm |\n| --- | --- | --- | --- |\n| claude | %s | max | %s |\n",
    );
}

#[test]
fn blind_packets_preserve_findings_about_harness_model_and_effort_behavior() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
cat > "$reports/spec.md" <<'EOF'
Harness lookup accepts a directory as an executable.
Model selection accepts invalid input.
Effort validation is missing.
Arm setup can accidentally reuse credentials.
EOF
"#);
    fs::write(fixture.scenario.path("claude-script.sh.grader"), r#"
python3 - <<'PY'
import json, os
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
for trial in packet['trials']:
    for finding in ['Harness lookup accepts a directory', 'Model selection accepts invalid input', 'Effort validation is missing', 'Arm setup can accidentally reuse credentials']:
        assert finding in trial['spec'], (finding, trial['spec'])
json.dump({'scores': [dict(label=trial['label'], passed=True, reason='Finding text preserved.', wrong_finding=False, borderline=False) for trial in packet['trials']]}, open(os.environ['THIRDSHIFT_GRADER_SCORES'], 'w'))
PY
"#).unwrap();
    let run = fixture.run(&["--prior", "0.01"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
}

#[test]
fn invalid_model_scores_pause_without_partial_scores_and_resume_without_trials() {
    for mutation in [
        "scores.pop()",
        "scores[1] = scores[0]",
        "scores[0]['label'] = 'unknown'",
        "scores[0]['reason'] = '   '",
        "scores[0]['passed'] = 'yes'",
        "scores[0]['extra'] = True",
        "del scores[0]['borderline']",
    ] {
        let fixture = Fixture::new();
        let script_path = fixture.scenario.path("claude-script.sh.grader");
        let valid = fs::read_to_string(&script_path).unwrap();
        let script = format!(
            r#"
python3 - <<'PY'
import json, os
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
scores = [dict(label=trial['label'], passed=True, reason='Written criteria met.', wrong_finding=False, borderline=False) for trial in packet['trials']]
{mutation}
json.dump({{'scores': scores}}, open(os.environ['THIRDSHIFT_GRADER_SCORES'], 'w'))
PY
"#
        );
        fs::write(&script_path, script).unwrap();
        let paused = fixture.run(&["--prior", "0.01"]);
        assert_eq!(paused.code, Some(1), "{mutation}: {}", paused.stderr);
        let experiment = fixture.experiment();
        assert_eq!(experiment["state"], "paused");
        assert!(experiment["verdict"].is_null());
        for result in experiment["results"].as_array().unwrap() {
            assert!(
                result["score"]["model"].is_null(),
                "partial model Score: {mutation}"
            );
        }
        assert_eq!(fixture.trial_calls().len(), 2);
        fs::write(&script_path, valid).unwrap();
        fixture
            .scenario
            .user_config_is("[harness.codex]\nmodel = \"gpt-6.1-sol\"\n");
        let resumed = fixture.run(&["--prior", "0.01"]);
        assert_eq!(resumed.code, Some(0), "{mutation}: {}", resumed.stderr);
        let completed = fixture.experiment();
        assert_eq!(completed["state"], "completed");
        assert_eq!(fixture.trial_calls().len(), 2, "finished Trials reran");
        assert!(
            fixture.scenario.codex_calls().is_empty(),
            "resume changed the fixed Grader"
        );
        for result in completed["results"].as_array().unwrap() {
            assert_eq!(result["score"]["model"]["reason"], "Written criteria met.");
            assert_eq!(
                result["score"]["passed"], false,
                "model approval overrode missing reports"
            );
        }
    }
}

#[test]
fn blind_model_scores_reach_each_trial_and_the_verdict() {
    let fixture = Fixture::new();
    fixture
        .scenario
        .user_config_is("[harness.codex]\nmodel = \"gpt-6.1-sol\"\neffort = \"high\"\n");
    fixture.scenario.agent_does(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
cat > "$reports/standards.md" <<'EOF'
Harness: claude
**Model:** private-model
| Effort | max |
Arm: baseline
Built with claude · private-model · max
Generated with Claude Code
Co-Authored-By: Claude <noreply@example.invalid>
EOF
case "$PWD" in
  *--candidate--*) printf 'Caught expected defect.\n' > "$reports/spec.md" ;;
  *) printf 'Missed required defect.\n' > "$reports/spec.md" ;;
esac
"#);
    fs::write(fixture.scenario.path("claude-script.sh.grader"), r#"
python3 - <<'PY'
import json, os
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
assert packet['criteria'] == 'Report the invented finding.'
assert len(packet['trials']) == 6
text = json.dumps(packet).lower()
for identity in ['baseline', 'candidate', 'private-model', 'claude', 'effort', 'co-authored']:
    assert identity not in text, (identity, text)
scores = []
for trial in packet['trials']:
    caught = 'Caught expected defect.' in trial['spec']
    scores.append(dict(label=trial['label'], passed=caught, reason='Caught it.' if caught else 'Missed it.', wrong_finding=not caught, borderline=caught))
json.dump({'scores': scores}, open(os.environ['THIRDSHIFT_GRADER_SCORES'], 'w'))
PY
"#).unwrap();

    let run = fixture.run(&[
        "--baseline-model",
        "private-model",
        "--candidate-model",
        "other-private-model",
        "--trials-per-task",
        "3",
        "--prior",
        "0.01",
    ]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for result in experiment["results"].as_array().unwrap() {
        let caught = result["arm"] == "candidate";
        assert_eq!(result["score"]["passed"], caught);
        assert_eq!(result["score"]["wrong_finding"], !caught);
        assert_eq!(result["score"]["borderline"], caught);
        assert_eq!(
            result["score"]["model"]["reason"],
            if caught { "Caught it." } else { "Missed it." }
        );
    }
    assert_eq!(experiment["verdict"]["provisional_verdict"], "pass");
    assert_eq!(experiment["verdict"]["verdict"], "waiting_on_rechecks");
    assert_eq!(experiment["verdict"]["borderline_trials"], 3);
    assert_eq!(
        experiment["verdict"]["suites"]["spec-review"]["wrong_findings"]["verdict"],
        "pass"
    );
    let calls = fixture.scenario.codex_calls();
    assert_eq!(calls.len(), 1, "one fresh Grader for this Eval task");
    assert!(
        calls[0]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "gpt-6.1-sol")
    );
    assert!(
        calls[0]["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "--dangerously-bypass-approvals-and-sandbox")
    );
    let packet = fixture.grader_packet();
    let order: Vec<_> = packet["trials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|trial| trial["spec"].as_str().unwrap())
        .collect();
    assert_ne!(
        order,
        ["Missed required defect.\n"; 3]
            .into_iter()
            .chain(["Caught expected defect.\n"; 3])
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fixture
            .run(&[
                "--baseline-model",
                "private-model",
                "--candidate-model",
                "other-private-model",
                "--trials-per-task",
                "3",
                "--prior",
                "0.01",
            ])
            .code,
        Some(0)
    );
    assert_eq!(
        fixture.scenario.codex_calls().len(),
        1,
        "resume reuses validated Scores"
    );
}

#[test]
fn metadata_rows_contiguous_with_a_findings_table_are_blinded() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
cat > "$reports/spec.md" <<'EOF'
| Area | Finding |
| --- | --- |
| Harness | lookup accepts a directory as an executable. |
| Harness | claude |
| Model | private-model |
EOF
"#);
    let run = fixture.run(&["--prior", "0.01"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let packet = fixture.grader_packet();
    assert_eq!(packet["trials"].as_array().unwrap().len(), 2);
    for trial in packet["trials"].as_array().unwrap() {
        let report = trial["spec"].as_str().unwrap();
        assert!(
            report.contains("lookup accepts a directory as an executable."),
            "Blinding erased genuine finding: {report}"
        );
    }
    let text = packet.to_string().to_ascii_lowercase();
    for identity in ["claude", "private-model"] {
        assert!(
            !text.contains(identity),
            "Grader packet leaked identifying metadata {identity}: {packet}"
        );
    }
}
