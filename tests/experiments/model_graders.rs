use super::*;

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
    let packet: Value = serde_json::from_slice(
        &fs::read(PathBuf::from(calls[0]["cwd"].as_str().unwrap()).join("packet.json")).unwrap(),
    )
    .unwrap();
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
