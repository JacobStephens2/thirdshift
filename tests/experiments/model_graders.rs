use super::*;

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
}

#[test]
fn metadata_rows_beneath_generic_table_headers_are_blinded() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
case "$PWD" in
  *--candidate--*) identity='candidate'; model='other-private-model' ;;
  *) identity='baseline'; model='private-model' ;;
esac
printf '| Field | Value |\n| --- | --- |\n| Harness | claude |\n| Model | %s |\n| Effort | max |\n| Arm | %s |\n' "$model" "$identity" > "$reports/standards.md"
printf 'Reported the invented finding.\n' > "$reports/spec.md"
"#);
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
fn metadata_rows_beneath_setting_value_headers_are_blinded() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
case "$PWD" in
  *--candidate--*) identity='candidate'; model='other-private-model' ;;
  *) identity='baseline'; model='private-model' ;;
esac
printf '| Field | Setting |\n| --- | --- |\n| Harness | claude |\n| Model | %s |\n| Effort | max |\n| Arm | %s |\n' "$model" "$identity" > "$reports/standards.md"
printf 'Reported the invented finding.\n' > "$reports/spec.md"
"#);
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
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
case "$PWD" in
  *--candidate--*) identity='candidate'; model='other-private-model' ;;
  *) identity='baseline'; model='private-model' ;;
esac
printf '| Harness | Model | Effort | Arm |\n| --- | --- | --- | --- |\n| claude | %s | max | %s |\n' "$model" "$identity" > "$reports/standards.md"
printf 'Reported the invented finding.\n' > "$reports/spec.md"
"#);
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
    assert_eq!(experiment["verdict"]["verdict"], "pass");
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
