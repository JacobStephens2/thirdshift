mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde_json::{Value, json};
use support::{Scenario, scope_available};

/// Trials and Graders use the pass-through launcher across this suite, so fake
/// Harnesses stay observable at fixed scenario paths.
const NO_SANDBOX: [(&str, &str); 1] = [("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")];

#[path = "experiments/model_graders.rs"]
mod model_graders;

#[path = "experiments/rechecks.rs"]
mod rechecks;

#[path = "experiments/release.rs"]
mod release;

#[path = "experiments/sandbox.rs"]
mod sandbox;

#[test]
fn saved_single_level_experiments_keep_their_bar_and_verdict_when_read_and_resumed() {
    let fixture = Fixture::new();
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let id = fixture.experiment()["id"].as_str().unwrap().to_owned();
    let path = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{id}/experiment.json"
    ));
    let mut legacy: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    single_level_experiment(&mut legacy);
    // A retained decision is authoritative, even if a fresh calculation differs.
    legacy["verdict"]["verdict"] = json!("fail");
    for provisional in [None, Some(Value::Null), Some(json!("fail"))] {
        match provisional {
            Some(value) => legacy["verdict"]["provisional_verdict"] = value,
            None => {
                legacy["verdict"]
                    .as_object_mut()
                    .unwrap()
                    .remove("provisional_verdict");
            }
        }
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        let shown = fixture.experiment();
        assert_eq!(shown["bar"], legacy["bar"]);
        assert_eq!(shown["verdict"], legacy["verdict"]);
        assert_eq!(fs::read(&path).unwrap(), before);
        let trials = fixture.trial_calls().len();
        let resumed = fixture.run(&[]);
        assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
        let resumed: Value = serde_json::from_str(&resumed.stdout).unwrap();
        assert_eq!(resumed["bar"], legacy["bar"]);
        assert_eq!(resumed["verdict"], legacy["verdict"]);
        assert_eq!(fixture.trial_calls().len(), trials);
    }
}

fn single_level_experiment(legacy: &mut Value) {
    let old_bar = |bar: &mut Value| {
        let bar = bar.as_object_mut().unwrap();
        let level = bar.remove("pass_level").unwrap();
        bar.remove("fail_level");
        bar.insert("credible_level".into(), level);
    };
    old_bar(&mut legacy["bar"]);
    for suite in legacy["verdict"]["suites"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        old_bar(&mut suite["bar"]);
        suite["method"] = json!(
            "paired-dirichlet-v1; Pcg64 0.10.2; rand 0.10.3; rand_distr 0.6.0; linear quantiles"
        );
        suite["thirdshift_version"] = json!("legacy-version");
        for metric in ["accuracy", "wrong_findings"] {
            let metric = suite[metric].as_object_mut().unwrap();
            let interval = metric.remove("pass_interval").unwrap();
            metric.remove("fail_interval");
            metric.insert("interval".into(), interval);
        }
    }
}

#[test]
fn legacy_resume_commands_keep_literal_model_arguments_and_the_recorded_levels() {
    for model in ["--credible-level", "a model's name"] {
        let fixture = Fixture::new();
        let run = fixture.run(&[
            "--candidate-model",
            model,
            "--pass-level",
            "0.8",
            "--fail-level",
            "0.8",
        ]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let id = fixture.experiment()["id"].as_str().unwrap().to_owned();
        let path = fixture.scenario.path(&format!(
            "home/.thirdshift/evals/experiments/{id}/experiment.json"
        ));
        let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        saved["state"] = json!("paused");
        saved["verdict"] = Value::Null;
        saved["bar"].as_object_mut().unwrap().remove("pass_level");
        saved["bar"].as_object_mut().unwrap().remove("fail_level");
        saved["bar"]["credible_level"] = json!(0.8);
        saved["resume_command"] = json!(format!(
            "thirdshift 'experiment' 'candidate' '--candidate-model' '{}' '--credible-level' '0.8' '--json'",
            model.replace('\'', "'\\''")
        ));
        fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        let shown = fixture.experiment();
        assert_eq!(
            fs::read(&path).unwrap(),
            before,
            "reading must not rewrite stored inputs"
        );
        let command = shlex::split(shown["resume_command"].as_str().unwrap()).unwrap();
        let args: Vec<_> = command.iter().skip(1).map(String::as_str).collect();
        let trials = fixture.trial_calls().len();
        let resumed = fixture.scenario.run_with_env(&args, &NO_SANDBOX);
        assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
        let resumed: Value = serde_json::from_str(&resumed.stdout).unwrap();
        assert_eq!(resumed["id"], id);
        assert_eq!(resumed["candidate"]["model"], model);
        assert_eq!(resumed["bar"], saved["bar"]);
        assert_eq!(fixture.trial_calls().len(), trials);
        // A subsequent invocation records the translated flags itself; its
        // next saved view must still provide one supported resume command.
        let command = shlex::split(resumed["resume_command"].as_str().unwrap()).unwrap();
        let args: Vec<_> = command.iter().skip(1).map(String::as_str).collect();
        let repeated = fixture.scenario.run_with_env(&args, &NO_SANDBOX);
        assert_eq!(repeated.code, Some(0), "{}", repeated.stderr);
        assert_eq!(fixture.trial_calls().len(), trials);
    }
}

#[test]
fn experiment_fixes_both_levels_before_trials_and_refuses_changes() {
    let fixture = Fixture::new();
    let args = ["--pass-level", "0.8", "--fail-level", "0.99"];
    let run = fixture.run(&args);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let saved = fixture.experiment();
    assert_eq!(saved["bar"]["pass_level"], 0.8);
    assert_eq!(saved["bar"]["fail_level"], 0.99);
    assert!(saved["bar"].get("credible_level").is_none());
    let suite = &saved["verdict"]["suites"]["spec-review"];
    assert_eq!(suite["bar"]["pass_level"], 0.8);
    assert_eq!(suite["bar"]["fail_level"], 0.99);
    assert!(
        suite["method"]
            .as_str()
            .unwrap()
            .starts_with("paired-dirichlet-v2;")
    );
    for metric in ["accuracy", "wrong_findings"] {
        assert!(suite[metric]["pass_interval"]["lower"].is_number());
        assert!(suite[metric]["fail_interval"]["upper"].is_number());
        assert!(suite[metric].get("interval").is_none());
    }
    let trials = fixture.trial_calls().len();
    for changed in [
        ["--pass-level", "0.9", "--fail-level", "0.99"],
        ["--pass-level", "0.8", "--fail-level", "0.95"],
    ] {
        let run = fixture.run(&changed);
        assert_eq!(run.code, Some(1), "{}", run.stderr);
        assert!(
            run.stderr
                .contains("Bar cannot change after the first Trial"),
            "{}",
            run.stderr
        );
        assert_eq!(fixture.experiment()["bar"], saved["bar"]);
    }
    let resumed = fixture.run(&args);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    assert_eq!(fixture.experiment(), saved);
    assert_eq!(fixture.trial_calls().len(), trials);
}

#[test]
fn experiment_rejects_invalid_levels_and_the_removed_credible_level_flag() {
    let fixture = Fixture::new();
    for flag in ["--pass-level", "--fail-level"] {
        for invalid in ["0", "1", "-0.1", "NaN", "inf"] {
            let run = fixture.run(&[flag, invalid]);
            assert_eq!(run.code, Some(2), "{flag} {invalid}: {}", run.stderr);
            assert!(
                run.stderr
                    .contains("level must be strictly between zero and one"),
                "{}",
                run.stderr
            );
        }
    }
    let removed = fixture.run(&["--credible-level", "0.95"]);
    assert_eq!(removed.code, Some(2));
    assert!(
        removed
            .stderr
            .contains("unknown Experiment flag: --credible-level"),
        "{}",
        removed.stderr
    );
    assert!(fixture.trial_calls().is_empty());
}

fn assert_missing_trial_inputs_refused_before_builds(release: bool) {
    for session in ["code-review", "spec-review"] {
        let fixture = Fixture::with_session(session, 1);
        let tasks = fixture
            .scenario
            .path(&format!("home/.thirdshift/evals/suites/{session}/tasks"));
        let mut manifest = fs::read_to_string(tasks.join("fixture/task.toml"))
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        manifest.remove("trial");
        for name in ["fixture", "second", "proposal"] {
            let directory = tasks.join(name);
            fs::create_dir_all(&directory).unwrap();
            manifest["state"] = toml_edit::value(if name == "proposal" {
                "proposed"
            } else {
                "approved"
            });
            fs::write(directory.join("task.toml"), manifest.to_string()).unwrap();
        }
        fs::remove_dir_all(fixture.scenario.path("home/.thirdshift/evals/builds")).unwrap();
        let built = fixture.scenario.path("arm-build-started");
        let cargo = fixture.scenario.path("bin/cargo");
        fs::write(
            &cargo,
            format!("#!/bin/sh\ntouch '{}'\nexit 99\n", built.display()),
        )
        .unwrap();
        fs::set_permissions(cargo, fs::Permissions::from_mode(0o755)).unwrap();
        fixture
            .scenario
            .launch_git(&["tag", "v0.1.0", &fixture.baseline]);

        let run = fixture.run(if release { &["--release"] } else { &[] });

        assert_eq!(run.code, Some(1), "{}", run.stderr);
        assert!(
            !built.exists(),
            "Arm built before Trial inputs were checked: {}",
            run.stderr
        );
        for name in ["fixture", "second"] {
            assert!(
                run.stderr.contains(&format!("{session}/{name}")),
                "{}",
                run.stderr
            );
        }
        assert!(run.stderr.contains("[trial]"), "{}", run.stderr);
        assert!(!run.stderr.contains("proposal"), "{}", run.stderr);
        assert!(fixture.trial_calls().is_empty());
    }
}

#[test]
fn experiments_name_all_approved_tasks_missing_trial_inputs_before_building() {
    assert_missing_trial_inputs_refused_before_builds(false);
}

#[test]
fn spec_review_completed_experiment_uses_saved_inputs() {
    for release in [false, true] {
        let fixture = Fixture::new();
        fixture
            .scenario
            .launch_git(&["tag", "v0.1.0", &fixture.baseline]);
        fixture.scenario.agent_does(review_reports_prelude());
        let options: &[&str] = if release { &["--release"] } else { &[] };
        let first = fixture.run(options);
        assert_eq!(first.code, Some(0), "{}", first.stderr);
        let saved: Value = serde_json::from_str(&first.stdout).unwrap();
        assert_eq!(saved["state"], "completed");
        let trial_count = fixture.trial_calls().len();
        let task = fixture
            .scenario
            .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
        let mut manifest = fs::read_to_string(&task)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        manifest.remove("trial");
        fs::write(&task, manifest.to_string()).unwrap();

        let rerun = fixture.run(options);
        assert_eq!(
            rerun.code,
            Some(0),
            "completed Experiment must use saved inputs: {}",
            rerun.stderr
        );
        assert_eq!(serde_json::from_str::<Value>(&rerun.stdout).unwrap(), saved);
        assert_eq!(fixture.trial_calls().len(), trial_count);
    }
}

#[test]
fn unapproved_proposals_never_run_and_approval_only_changes_future_experiments() {
    let fixture = Fixture::new();
    let tasks = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks");
    let proposed = tasks.join("proposal");
    fs::create_dir(&proposed).unwrap();
    fs::copy(tasks.join("fixture/spec.md"), proposed.join("spec.md")).unwrap();
    let manifest = fs::read_to_string(tasks.join("fixture/task.toml")).unwrap();
    fs::write(
        proposed.join("task.toml"),
        manifest.replace("state = \"approved\"", "state = \"proposed\""),
    )
    .unwrap();
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    assert_eq!(
        experiment["tasks"],
        json!([{"suite":"spec-review", "name":"fixture"}])
    );
    assert_eq!(experiment["results"].as_array().unwrap().len(), 2);
    assert_eq!(fixture.trial_calls().len(), 2);
    let approved = fixture
        .scenario
        .run(&["evals", "approve", "spec-review", "proposal", "--json"]);
    assert_eq!(approved.code, Some(0), "{}", approved.stderr);
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    assert_eq!(fixture.experiment()["tasks"], experiment["tasks"]);
    assert_eq!(fixture.trial_calls().len(), 2);
    let future = fixture.run(&["--candidate-model", "future-model"]);
    assert_eq!(future.code, Some(0), "{}", future.stderr);
    let future: Value = serde_json::from_str(future.stdout.lines().last().unwrap()).unwrap();
    assert_eq!(
        future["tasks"],
        json!([
            {"suite":"spec-review", "name":"fixture"},
            {"suite":"spec-review", "name":"proposal"}
        ])
    );
    assert_eq!(future["results"].as_array().unwrap().len(), 4);
}

#[test]
fn experiment_trace_contains_trials_and_graders_and_reports_arm_usage() {
    let fixture = Fixture::new();
    // A trace-capable older Arm ignores the new inheritance environment.
    let binary = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/builds/{}/thirdshift",
        fixture.baseline,
    ));
    let original = binary.with_extension("real");
    fs::rename(&binary, &original).unwrap();
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\nunset THIRDSHIFT_TRIAL_TRACE_CONTEXT\nexec {:?} \"$@\"\n",
            original
        ),
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    fs::write(
        &task,
        manifest.replace("[inputs]", "checks = [[\"true\"]]\n[inputs]"),
    )
    .unwrap();
    fixture.scenario.agent_does(r#"
test -z "${THIRDSHIFT_TRIAL_TRACE_CONTEXT+x}"
echo '{"type":"assistant","message":{"id":"m1","model":"observed-a","usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":2,"cache_creation_input_tokens":1},"content":[]}}'
echo '{"type":"assistant","message":{"id":"m2","model":"observed-b","usage":{"input_tokens":20,"output_tokens":4},"content":[]}}'
echo '{"type":"result","subtype":"success","total_cost_usd":0.25,"modelUsage":{"observed-a":{"inputTokens":10,"outputTokens":3,"cacheReadInputTokens":2,"cacheCreationInputTokens":1,"costUSD":0.10},"observed-b":{"inputTokens":20,"outputTokens":4,"costUSD":0.15}}}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#);
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for arm in ["baseline", "candidate"] {
        let usage = &experiment["usage"][arm];
        assert_eq!(usage["models"]["observed-a"]["input_tokens"], 13);
        assert_eq!(usage["models"]["observed-a"]["output_tokens"], 3);
        assert_eq!(usage["models"]["observed-b"]["input_tokens"], 20);
        assert_eq!(usage["models"]["observed-b"]["output_tokens"], 4);
        assert_eq!(usage["cost_usd"], 0.25);
    }
    let id = experiment["id"].as_str().unwrap();
    let text = fixture.scenario.run(&["experiments", id]);
    assert!(text.stdout.contains("observed-a"), "{}", text.stdout);
    assert!(text.stdout.contains("$0.250000"), "{}", text.stdout);
    let record = fixture
        .scenario
        .command_record("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment");
    let export = fixture.scenario.path("export");
    let run = fixture.scenario.run(&[
        "logs",
        record["name"].as_str().unwrap(),
        "--export",
        export.to_str().unwrap(),
    ]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let spans = trace_spans(&export.join("spans.jsonl"));
    let root = spans.iter().find(|span| span["name"] == "Command").unwrap();
    let trials: Vec<_> = spans
        .iter()
        .filter(|span| span["name"].as_str().unwrap().starts_with("Trial "))
        .collect();
    assert_eq!(trials.len(), 2, "{spans:#?}");
    let graders: Vec<_> = spans
        .iter()
        .filter(|span| span["name"] == "Grader")
        .collect();
    assert_eq!(graders.len(), 2);
    for session in spans
        .iter()
        .filter(|span| span["name"] == "invoke_agent spec-review")
        .chain(graders)
    {
        assert!(
            trials
                .iter()
                .any(|trial| trial["spanId"] == session["parentSpanId"])
        );
    }
    assert!(spans.iter().all(|span| span["traceId"] == root["traceId"]));
    assert_eq!(
        spans
            .iter()
            .filter(|span| span["name"] == "invoke_agent spec-review")
            .count(),
        2
    );
}

fn trace_spans(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|line| {
            let request: Value = serde_json::from_str(line).unwrap();
            request["resourceSpans"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|resource| {
                    resource["scopeSpans"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .flat_map(|scope| scope["spans"].as_array().unwrap().clone())
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn timed_out_trials_keep_partial_usage_and_exportable_session_ancestry() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
echo '{"type":"assistant","message":{"id":"partial","model":"observed","usage":{"input_tokens":11,"output_tokens":2},"content":[]}}'
sleep 20
"#);
    let run = fixture.run(&["--timeout", "1"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for arm in ["baseline", "candidate"] {
        assert_eq!(
            experiment["usage"][arm]["models"]["observed"]["input_tokens"],
            11
        );
        assert!(experiment["usage"][arm]["cost_usd"].is_null());
    }
    let record = fixture
        .scenario
        .command_record("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment");
    let export = fixture.scenario.path("export");
    let exported = fixture.scenario.run(&[
        "logs",
        record["name"].as_str().unwrap(),
        "--export",
        export.to_str().unwrap(),
    ]);
    assert_eq!(exported.code, Some(0), "{}", exported.stderr);
    let spans = trace_spans(&export.join("spans.jsonl"));
    let sessions: Vec<_> = spans
        .iter()
        .filter(|span| span["name"] == "invoke_agent spec-review")
        .collect();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.iter().all(|span| span["status"]["code"] == 2));
}

#[test]
fn arm_usage_keeps_unreported_cost_and_model_unknown() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("true");
    let run = fixture.run(&[
        "--baseline-harness",
        "codex",
        "--baseline-model",
        "requested-only",
    ]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    let usage = &experiment["usage"];
    assert_eq!(usage["baseline"]["models"]["unknown"]["input_tokens"], 1200);
    assert_eq!(usage["baseline"]["models"]["unknown"]["output_tokens"], 300);
    assert_eq!(
        usage["baseline"]["models"]["unknown"]["cache_read_tokens"],
        200
    );
    assert!(usage["baseline"]["models"]["requested-only"].is_null());
    assert!(usage["baseline"]["cost_usd"].is_null());
    assert_eq!(usage["candidate"]["models"], json!({}));
    assert!(usage["candidate"]["cost_usd"].is_null());
}

#[test]
fn session_totals_keep_the_harness_reported_model() {
    let fixture = Fixture::new();
    let harness = fixture.scenario.path("bin/agy");
    let real = harness.canonicalize().unwrap();
    let fixed = fixture.scenario.path("versions/v1/agy");
    fs::copy(real, &fixed).unwrap();
    fs::remove_file(&harness).unwrap();
    std::os::unix::fs::symlink(fixed, &harness).unwrap();
    fixture.scenario.agent_does(
        r#"
echo '{"event":"init","conversation_id":"reported","init":{"model":"gemini-observed"}}'
"#,
    );
    let run = fixture.run(&["--baseline-harness", "agy", "--candidate-harness", "agy"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for arm in ["baseline", "candidate"] {
        let models = &experiment["usage"][arm]["models"];
        assert_eq!(models["gemini-observed"]["input_tokens"], 1200);
        assert_eq!(models["gemini-observed"]["output_tokens"], 300);
        assert_eq!(models["gemini-observed"]["cache_read_tokens"], 200);
        assert!(models["unknown"].is_null(), "{models:#}");
    }
}

#[test]
fn arm_usage_keeps_session_totals_when_model_calls_report_partial_usage() {
    let fixture = Fixture::new();
    let harness = fixture.scenario.path("bin/agy");
    let real = harness.canonicalize().unwrap();
    let fixed = fixture.scenario.path("versions/v1/agy");
    fs::copy(real, &fixed).unwrap();
    fs::remove_file(&harness).unwrap();
    std::os::unix::fs::symlink(fixed, &harness).unwrap();
    fixture.scenario.agent_does(
        r#"
echo '{"event":"init","conversation_id":"reported","init":{"model":"gemini-observed"}}'
echo '{"event":"step_update","step_update":{"conversation_id":"reported","step_index":1,"step_type":"agent_response","state":"DONE","usage":{"output_tokens":2}}}'
"#,
    );
    let run = fixture.run(&["--baseline-harness", "agy", "--candidate-harness", "agy"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for arm in ["baseline", "candidate"] {
        let model = &experiment["usage"][arm]["models"]["gemini-observed"];
        assert_eq!(model["input_tokens"], 1200, "{model:#}");
        assert_eq!(model["output_tokens"], 300, "{model:#}");
        assert_eq!(model["cache_read_tokens"], 200, "{model:#}");
    }
}

#[test]
fn partial_model_input_total_includes_preserved_cache_tokens() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
echo '{"type":"assistant","message":{"id":"m1","model":"observed","usage":{"input_tokens":11,"output_tokens":2,"cache_read_input_tokens":5},"content":[]}}'
echo '{"type":"result","subtype":"success","total_cost_usd":0.125,"modelUsage":{"observed":{"inputTokens":20,"outputTokens":3,"costUSD":0.125}}}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#);
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for arm in ["baseline", "candidate"] {
        let model = &experiment["usage"][arm]["models"]["observed"];
        assert_eq!(model["input_tokens"], 25);
        assert_eq!(model["output_tokens"], 3);
        assert_eq!(model["cache_read_tokens"], 5);
        assert_eq!(experiment["usage"][arm]["cost_usd"], 0.125);
    }
}

#[test]
fn partial_model_totals_preserve_other_reported_call_measurements() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
echo '{"type":"assistant","message":{"id":"m1","model":"observed","usage":{"input_tokens":11,"output_tokens":2,"cache_read_input_tokens":5},"content":[]}}'
echo '{"type":"result","subtype":"success","total_cost_usd":0.125,"modelUsage":{"observed":{"outputTokens":3,"costUSD":0.125}}}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#);
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for arm in ["baseline", "candidate"] {
        let model = &experiment["usage"][arm]["models"]["observed"];
        assert_eq!(model["input_tokens"], 16);
        assert_eq!(model["output_tokens"], 3);
        assert_eq!(model["cache_read_tokens"], 5);
        assert_eq!(experiment["usage"][arm]["cost_usd"], 0.125);
    }
}

#[test]
fn arm_usage_sums_void_retries_and_both_sessions_of_each_resume() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&format!(r#"
echo '{{"type":"assistant","message":{{"id":"m1","model":"observed","usage":{{"input_tokens":7,"output_tokens":2}},"content":[]}}}}'
echo '{{"type":"result","subtype":"success","total_cost_usd":0.05}}' >> "$FAKE_CLAUDE_AFTER_RESULT"
printf changed > changed.txt
{}
"#, support::leaves_running("cargo test")));
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for arm in ["baseline", "candidate"] {
        // Three fresh attempts, each with its opening session and Resume.
        assert_eq!(
            experiment["usage"][arm]["models"]["observed"]["input_tokens"],
            42
        );
        assert_eq!(
            experiment["usage"][arm]["models"]["observed"]["output_tokens"],
            12
        );
        assert!((experiment["usage"][arm]["cost_usd"].as_f64().unwrap() - 0.30).abs() < 1e-9);
    }
    assert_eq!(
        fixture.experiment(),
        experiment,
        "show must not count pending results twice"
    );
}

#[test]
fn spec_review_generated_instruction_fallbacks_do_not_void_trials() {
    for harness_name in ["agy", "opencode"] {
        let fixture = Fixture::new();
        let harness = fixture.scenario.path(&format!("bin/{harness_name}"));
        let real = harness.canonicalize().unwrap();
        let fixed = fixture
            .scenario
            .path(&format!("versions/v1/{harness_name}"));
        fs::copy(real, &fixed).unwrap();
        fs::remove_file(&harness).unwrap();
        std::os::unix::fs::symlink(fixed, &harness).unwrap();
        let directory = fixture
            .scenario
            .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture");
        fs::write(
            directory.join("instructions.md"),
            "Read the supplied Spec.\n",
        )
        .unwrap();
        let task = directory.join("task.toml");
        let manifest = fs::read_to_string(&task).unwrap();
        fs::write(
            &task,
            manifest.replace("[inputs]", "[inputs]\n\"CLAUDE.md\" = \"instructions.md\""),
        )
        .unwrap();
        fixture.scenario.agent_does("true");
        let run = fixture.run(&[
            "--baseline-harness",
            harness_name,
            "--candidate-harness",
            harness_name,
        ]);
        assert_eq!(run.code, Some(0), "{harness_name}: {}", run.stderr);
        let experiment = fixture.experiment();
        for result in experiment["results"].as_array().unwrap() {
            assert_eq!(
                result["outcome"], "succeeded",
                "{harness_name}: generated fallback links must not invalidate an unchanged Trial"
            );
        }
    }
}

#[test]
fn spec_review_ignored_repository_changes_are_void() {
    let fixture = Fixture::new();
    let directory = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture");
    let task = directory.join("task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    fs::write(directory.join("ignore.txt"), "ignored-by-fixture.txt\n").unwrap();
    fs::write(
        &task,
        manifest.replace("[inputs]", "[inputs]\n\".gitignore\" = \"ignore.txt\""),
    )
    .unwrap();

    fixture
        .scenario
        .agent_does("printf changed > ignored-by-fixture.txt");
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);

    let experiment = fixture.experiment();
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(
            result["outcome"], "void",
            "creating an ignored repository file must invalidate the Trial"
        );
    }
}

#[test]
fn spec_review_arm_minutes_include_limit_paused_attempts_after_resume() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(
        r#"
sleep 0.1
echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["quota exceeded"]}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#,
    );
    let paused = fixture.run(&[]);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    let paused_view = fixture.experiment();
    assert!(
        paused_view["minutes"]["baseline"].as_f64().unwrap()
            + paused_view["minutes"]["candidate"].as_f64().unwrap()
            > 0.0,
        "completed sessions must count while the Experiment is paused"
    );

    fixture.scenario.agent_does("true");
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let experiment = fixture.experiment();
    let id = experiment["id"].as_str().unwrap();
    let trials = fixture
        .scenario
        .path(&format!("home/.thirdshift/evals/experiments/{id}/trials"));
    let mut expected =
        std::collections::BTreeMap::from([("baseline", 0.0_f64), ("candidate", 0.0_f64)]);

    for trial in fs::read_dir(trials).unwrap() {
        let trial = trial.unwrap();
        let name = trial.file_name().to_string_lossy().into_owned();
        let arm = if name.contains("--baseline--") {
            "baseline"
        } else {
            "candidate"
        };
        for attempt in fs::read_dir(trial.path()).unwrap() {
            let attempt = attempt.unwrap();
            if !attempt.file_type().unwrap().is_dir() {
                continue;
            }
            let path = attempt.path().join("result.json");
            if !path.is_file() {
                continue;
            }
            let output: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            *expected.get_mut(arm).unwrap() +=
                output["timings"]["total_ms"].as_u64().unwrap() as f64 / 60_000.0;
        }
    }

    for arm in ["baseline", "candidate"] {
        let shown = experiment["minutes"][arm].as_f64().unwrap();
        assert!(
            shown + 1e-9 >= expected[arm],
            "{arm}: displayed {shown} minutes, retained attempts consumed {}",
            expected[arm]
        );
    }
}

struct Fixture {
    scenario: Scenario,
    baseline: String,
    candidate: String,
}

fn is_grader(call: &Value) -> bool {
    call["prompt"]
        .as_str()
        .unwrap()
        .starts_with("You are a Grader")
}

impl Fixture {
    fn new() -> Self {
        Self::with_session("spec-review", 1)
    }

    fn with_session(session: &str, trials: usize) -> Self {
        let scenario = Scenario::new();
        // Ordinary fixtures use a pass-through manager at the external
        // systemd seam. Live cap tests forward the real bus explicitly.
        for (name, body) in [
            (
                "systemctl",
                "case \"$*\" in '--user show -p SystemState --value') echo running ;; esac",
            ),
            (
                "systemd-run",
                "while [ \"$#\" -gt 0 ] && [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"",
            ),
            ("journalctl", "exit 0"),
        ] {
            let path = scenario.path(&format!("bin/{name}"));
            fs::write(&path, format!(
                "#!/bin/sh\nif [ -n \"${{DBUS_SESSION_BUS_ADDRESS:-}}\" ]; then exec /usr/bin/{name} \"$@\"; fi\n{body}\n"
            )).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(scenario.path("claude-script.sh.grader"), r#"
python3 - <<'PY'
import json, os
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
json.dump({'scores': [dict(label=trial['label'], passed=True, reason='Written criteria met.', wrong_finding=False, borderline=False) for trial in packet['trials']]}, open(os.environ['THIRDSHIFT_GRADER_SCORES'], 'w'))
try:
    adj = open('/proc/self/oom_score_adj').read().strip()
except OSError:
    adj = 'unknown'
open(os.environ['THIRDSHIFT_GRADER_SCORES'] + '.adj', 'w').write(adj + '\n')
PY
"#).unwrap();
        for name in ["claude", "codex"] {
            let harness = scenario.path(&format!("bin/{name}"));
            let real = harness.canonicalize().unwrap();
            let fixed = scenario.path(&format!("versions/v1/{name}"));
            fs::create_dir_all(fixed.parent().unwrap()).unwrap();
            fs::copy(real, &fixed).unwrap();
            fs::remove_file(&harness).unwrap();
            std::os::unix::fs::symlink(fixed, &harness).unwrap();
        }
        let baseline = scenario
            .launch_git(&["rev-parse", "HEAD"])
            .trim()
            .to_owned();
        scenario.launch_git(&["checkout", "-b", "candidate"]);
        fs::create_dir(scenario.launch_dir().join("src")).unwrap();
        scenario.commit_locally("src/prompt.rs", "candidate prompt", "Candidate");
        let candidate = scenario
            .launch_git(&["rev-parse", "HEAD"])
            .trim()
            .to_owned();
        for commit in [&baseline, &candidate] {
            let build = scenario.path(&format!("home/.thirdshift/evals/builds/{commit}"));
            fs::create_dir_all(&build).unwrap();
            fs::copy(env!("CARGO_BIN_EXE_thirdshift"), build.join("thirdshift")).unwrap();
        }
        let suite = scenario.path(&format!("home/.thirdshift/evals/suites/{session}"));
        let task = suite.join("tasks/fixture");
        fs::create_dir_all(&task).unwrap();
        fs::write(
            suite.join("suite.toml"),
            format!(
                r#"format = 1
trials_per_arm = {trials}
[dependencies]
factory_skills = []
session_prompts = ["src/prompt.rs"]
harness_defaults = []
model_defaults = []
effort_defaults = []
"#
            ),
        )
        .unwrap();
        fs::write(
            task.join("spec.md"),
            "# Fixture Spec\n\nReview this invented change.\n",
        )
        .unwrap();
        let change = if session == "code-review" {
            &candidate
        } else {
            &baseline
        };
        fs::write(
            task.join("task.toml"),
            format!(
                r#"format = 1
state = "approved"
session = "{session}"
repository = {:?}
base = "{baseline}"
change = "{change}"
criteria = "Report the invented finding."
[inputs]
".thirdshift-issue.md" = "spec.md"
[trial]
issue_url = "https://github.com/acme/widgets/issues/7"
base = "main"
branch = "issue-7"
pr_url = "https://github.com/acme/widgets/pull/1"
"#,
                scenario.launch_dir()
            ),
        )
        .unwrap();
        Self {
            scenario,
            baseline,
            candidate,
        }
    }

    fn run(&self, extra: &[&str]) -> support::RunResult {
        let mut args = vec!["experiment", "candidate", "--json"];
        args.extend_from_slice(extra);
        // Trials run without their sandbox here: the Experiment suite's fake
        // Harnesses stay observable at fixed scenario paths, as with
        // THIRDSHIFT_TRIAL_NO_SANDBOX. The sandbox itself is covered by the
        // sandbox module tests and the sandboxed Trial tests.
        self.scenario.run_with_env(&args, &NO_SANDBOX)
    }

    fn experiment(&self) -> Value {
        let listed = self.scenario.run(&["experiments", "--json"]);
        assert_eq!(listed.code, Some(0), "{}", listed.stderr);
        let listed: Value = serde_json::from_str(&listed.stdout).unwrap();
        let id = listed[0]["id"].as_str().unwrap();
        let shown = self.scenario.run(&["experiments", id, "--json"]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        serde_json::from_str(&shown.stdout).unwrap()
    }

    fn trial_calls(&self) -> Vec<Value> {
        self.scenario
            .claude_calls()
            .into_iter()
            .filter(|call| !is_grader(call))
            .collect()
    }

    fn grader_calls(&self) -> Vec<Value> {
        self.scenario
            .claude_calls()
            .into_iter()
            .chain(self.scenario.codex_calls())
            .filter(is_grader)
            .collect()
    }

    fn grader_packet(&self) -> Value {
        let calls = self.grader_calls();
        assert_eq!(calls.len(), 1, "expected one Eval task's Grader session");
        serde_json::from_slice(
            &fs::read(PathBuf::from(calls[0]["cwd"].as_str().unwrap()).join("packet.json"))
                .unwrap(),
        )
        .unwrap()
    }
}

fn review_reports_prelude() -> &'static str {
    r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
test -n "$reports"
printf 'Standards report\n' > "$reports/standards.md"
"#
}

#[test]
fn code_review_runs_three_trials_per_arm_on_the_reviewed_diff_and_reports_pass_cubed() {
    let fixture = Fixture::with_session("code-review", 3);
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/code-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    fs::write(&task, manifest.replace("[inputs]", "checks = [[\"sh\", \"-c\", 'grep -q caught \"$THIRDSHIFT_TRIAL_SPEC_REPORT\"']]\n[inputs]")).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"
test "$(git diff --name-only main...HEAD)" = src/prompt.rs
test "$(git show HEAD:src/prompt.rs)" = 'candidate prompt'
test "$(git rev-list --count main..HEAD)" = 1
test -z "$(git remote)"
{reports}
case "$PWD" in
  *--candidate--1/*) printf 'missed\n' > "$reports/spec.md" ;;
  *) printf 'caught\n' > "$reports/spec.md" ;;
esac
"#,
        reports = review_reports_prelude()
    ));

    let run = fixture.run(&[]);

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(run.stdout.lines().last().unwrap()).unwrap();
    assert_eq!(experiment["bar"]["trials_per_task"]["code-review"], 3);
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 6);
    for arm in ["baseline", "candidate"] {
        for trial in 0..3 {
            let result = results
                .iter()
                .find(|r| r["arm"] == arm && r["trial"] == trial)
                .unwrap();
            assert_eq!(result["outcome"], "succeeded", "{result}");
            assert_eq!(result["score"]["passed"], arm == "baseline" || trial != 1);
            assert_eq!(result["attempts"].as_array().unwrap().len(), 1);
            let attempt = &result["attempts"][0];
            assert!(
                attempt["log"]
                    .as_str()
                    .unwrap()
                    .ends_with("trial-code-review.logs.jsonl")
            );
            for axis in ["standards", "spec"] {
                assert!(std::path::Path::new(attempt["reports"][axis].as_str().unwrap()).is_file());
            }
        }
    }
    assert_eq!(
        experiment["verdict"]["suites"]["code-review"]["pass_cubed"],
        json!({"baseline": 1.0, "candidate": 0.0})
    );
    let shown = fixture.experiment();
    assert_eq!(shown["verdict"], experiment["verdict"]);
    for call in fixture.trial_calls() {
        assert!(
            call["prompt"]
                .as_str()
                .unwrap()
                .starts_with("/thirdshift-code-review main\n")
        );
    }
    assert!(fixture.scenario.gh_calls().is_empty());
}

#[test]
fn imported_code_review_suite_runs_three_trials_per_arm_and_reports_pass_cubed() {
    let fixture = Fixture::with_session("code-review", 3);
    fs::remove_dir_all(
        fixture
            .scenario
            .path("home/.thirdshift/evals/suites/code-review"),
    )
    .unwrap();
    let repository = fixture.scenario.path("home/repos/thirdshift");
    fs::create_dir_all(repository.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(fixture.scenario.launch_dir(), &repository).unwrap();
    let replays = fixture.scenario.path("home/.thirdshift/replays");
    for (name, value) in [
        (
            "inputs-2026-10-07/manifest.json",
            json!({
                "timing_bugs": [{
                    "id": 7,
                    "introducing_pr": {
                        "number": 17,
                        "merge_base": fixture.baseline,
                        "head_as_merged": fixture.candidate,
                        "implements_issue": 7
                    }
                }],
                "spec_reviews": []
            }),
        ),
        ("2026-10-09/inputs/manifest.json", json!({"codex_bugs": []})),
        (
            "2026-10-09/grading/eval-import.json",
            json!({"bugs": {"7": "Catch the invented defect."}, "specs": {}}),
        ),
        (
            "2026-10-09/context/thirdshift/7.json",
            json!({
                "title": "Invented change",
                "body": "Original issue.",
                "parent": null,
                "tickets": []
            }),
        ),
    ] {
        let path = replays.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    let imported = fixture.scenario.run(&["evals", "import-replays"]);
    assert_eq!(imported.code, Some(0), "{}", imported.stderr);
    fixture.scenario.agent_does(&format!(
        r#"
test "$(git diff --name-only main...HEAD)" = src/prompt.rs
test -z "$(git remote)"
{reports}
printf 'Spec report\n' > "$reports/spec.md"
"#,
        reports = review_reports_prelude()
    ));
    let run = fixture.run(&["--suite", "code-review"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(run.stdout.lines().last().unwrap()).unwrap();
    assert_eq!(experiment["bar"]["trials_per_task"]["code-review"], 3);
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 6);
    for result in results {
        assert_eq!(result["outcome"], "succeeded", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
        let attempt = &result["attempts"][0];
        for axis in ["standards", "spec"] {
            assert!(std::path::Path::new(attempt["reports"][axis].as_str().unwrap()).is_file());
        }
    }
    assert_eq!(
        experiment["verdict"]["suites"]["code-review"]["pass_cubed"],
        json!({"baseline": 1.0, "candidate": 1.0})
    );
}

#[test]
fn mechanical_scores_determine_the_provisional_verdict_inbox_and_arm_minutes() {
    for (passing_arm, decision, group) in [
        ("candidate", "pass", "needs you"),
        ("baseline", "fail", "needs you"),
    ] {
        let fixture = Fixture::new();
        let task = fixture
            .scenario
            .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
        let manifest = fs::read_to_string(&task).unwrap();
        fs::write(&task, manifest.replace("[inputs]",
            "checks = [[\"sh\", \"-c\", 'grep -q caught \"$THIRDSHIFT_TRIAL_SPEC_REPORT\"']]\n[inputs]"
        )).unwrap();
        fixture.scenario.agent_does(&format!(
            r#"
{reports}
case "$PWD" in
  *--{passing_arm}--*) printf 'caught\n' > "$reports/spec.md" ;;
  *) printf 'missed\n' > "$reports/spec.md" ;;
esac
"#,
            reports = review_reports_prelude()
        ));
        let run = fixture.run(&["--prior", "0.01"]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let last = run.stdout.lines().last().unwrap();
        let experiment: Value = serde_json::from_str(last).unwrap();
        assert_eq!(experiment["verdict"]["provisional_verdict"], decision);
        assert_eq!(experiment["verdict"]["verdict"], "waiting_on_rechecks");
        assert!(
            experiment["verdict"]["suites"]["spec-review"]["accuracy"]["pass_interval"]["lower"]
                .is_number()
        );
        for result in experiment["results"].as_array().unwrap() {
            assert_eq!(result["score"]["passed"], result["arm"] == passing_arm);
            assert!(
                result["score"]["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|check| check["command"].is_array())
            );
        }
        let id = experiment["id"].as_str().unwrap();
        let text = fixture.scenario.run(&["experiments", id]);
        assert!(text.stdout.contains("Baseline:"), "{}", text.stdout);
        assert!(text.stdout.contains("Candidate:"), "{}", text.stdout);
        for arm in ["baseline", "candidate"] {
            assert!(experiment["minutes"][arm].as_f64().unwrap() > 0.0);
        }
        assert_eq!(fixture.experiment(), experiment);
        let record = fixture
            .scenario
            .command_record("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment");
        assert_eq!(record["outcome"], "waiting_on_rechecks");
        let status = fixture.scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&status.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], group);
        assert_eq!(fixture.run(&["--prior", "0.01"]).stdout, run.stdout);
    }
}

#[test]
fn changed_repositories_are_void_retried_and_excluded_from_the_verdict() {
    for mutation in [
        "printf changed >> .thirdshift-issue.md",
        "printf changed > invented-untracked.txt",
        "git -c user.name=Fixture -c user.email=fixture@example.invalid commit --allow-empty -m changed",
    ] {
        let fixture = Fixture::new();
        fixture.scenario.agent_does(mutation);
        let run = fixture.run(&[]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let experiment = fixture.experiment();
        assert_eq!(experiment["verdict"]["verdict"], "inconclusive");
        assert_eq!(
            experiment["verdict"]["suites"]["spec-review"]["dropped_tasks"],
            1
        );
        for result in experiment["results"].as_array().unwrap() {
            assert_eq!(result["outcome"], "void", "mutation: {mutation}");
            assert_eq!(result["attempts"].as_array().unwrap().len(), 3);
            for attempt in result["attempts"].as_array().unwrap() {
                assert_eq!(attempt["void_cause"], "repository_changed");
            }
        }
        let status = fixture.scenario.run(&["status", "--json"]);
        let status: Value = serde_json::from_str(&status.stdout).unwrap();
        assert_eq!(status["commands"][0]["group"], "needs you");
    }
}

#[test]
fn mechanical_check_output_stays_in_private_results() {
    let fixture = Fixture::new();
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    fs::write(
        &task,
        manifest.replace(
            "[inputs]",
            "checks = [[\"printf\", \"private-grader-evidence\"]]\n[inputs]",
        ),
    )
    .unwrap();
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(run.stdout.contains("private-grader-evidence"));
    let folder = "home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment";
    for log in fixture.scenario.log_files(folder, "log") {
        let log = fs::read_to_string(fixture.scenario.path(folder).join(log)).unwrap();
        assert!(
            !log.contains("private-grader-evidence"),
            "private Score entered factory logs"
        );
    }
}

#[test]
fn experiment_records_both_arms_the_bar_logs_stages_and_each_trial() {
    let fixture = Fixture::new();
    let result = fixture.run(&[
        "--baseline-model",
        "baseline-model",
        "--candidate-model",
        "candidate-model",
    ]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout.lines().count(), 1);
    let experiment: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(experiment["state"], "completed");
    assert_eq!(experiment["baseline_ref"], "main (merge base)");
    assert_eq!(experiment["candidate_ref"], "candidate");
    assert_eq!(experiment["title"], "Candidate");
    assert!(experiment["ended_at"].as_str().is_some());
    assert!(
        experiment["resume_command"]
            .as_str()
            .unwrap()
            .contains("baseline-model")
    );
    assert!(experiment["running_trials"].as_array().unwrap().is_empty());
    assert!(!experiment["sessions"].as_array().unwrap().is_empty());
    assert_eq!(experiment["baseline"]["commit"], fixture.baseline);
    assert_eq!(experiment["candidate"]["commit"], fixture.candidate);
    assert_eq!(experiment["suites"], json!(["spec-review"]));
    assert_eq!(experiment["bar"]["margin"], -0.1);
    assert_eq!(experiment["bar"]["pass_level"], 0.95);
    assert_eq!(experiment["bar"]["fail_level"], 0.95);
    assert_eq!(experiment["bar"]["trials_per_task"]["spec-review"], 1);
    assert_eq!(experiment["results"].as_array().unwrap().len(), 2);
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "succeeded");
        assert_eq!(
            result["score"]["passed"], false,
            "a review without reports fails"
        );
        assert!(PathBuf::from(result["attempts"][0]["log"].as_str().unwrap()).is_file());
    }
    assert_eq!(fixture.experiment(), experiment);
    let folder = "home/.thirdshift/logs/JacobStephens2/thirdshift/commands/experiment";
    let record = fixture.scenario.command_record(folder);
    assert_eq!(record["kind"], "Experiment");
    assert_eq!(record["experiment"]["bar"], experiment["bar"]);
    assert_eq!(record["experiment"]["baseline"]["commit"], fixture.baseline);
    assert_eq!(
        record["experiment"]["candidate"]["commit"],
        fixture.candidate
    );
    let stages: Vec<_> = record["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(stages, ["Arm builds", "Eval tasks", "Trials"]);
    let activity = fs::read_to_string(
        fixture
            .scenario
            .path("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(
        activity.contains("Experiment started: commands/experiment/"),
        "{activity}"
    );
    assert!(
        activity.contains("Experiment ended: inconclusive"),
        "{activity}"
    );
    let logs = fixture.scenario.log_files(folder, "log");
    assert_eq!(logs.len(), 1);
    let log = fs::read_to_string(fixture.scenario.path(folder).join(&logs[0])).unwrap();
    assert!(log.contains("Trials"), "{log}");
    let calls = fixture.trial_calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().any(|c| {
        c["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "baseline-model")
    }));
    assert!(calls.iter().any(|c| {
        c["argv"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "candidate-model")
    }));
    let status = fixture.scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["commands"][0]["kind"], "Experiment");
    assert_eq!(status["commands"][0]["group"], "needs you");
    assert_eq!(
        fixture
            .run(&[
                "--baseline-model",
                "baseline-model",
                "--candidate-model",
                "candidate-model"
            ])
            .code,
        Some(0)
    );
    assert_eq!(
        fixture.trial_calls().len(),
        2,
        "completed Experiment ran again"
    );
}

#[test]
fn stop_and_resume_preserve_finished_trials_and_refuse_a_changed_bar() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&format!(
        r#"
case "$PWD" in
  *--0/*) exit 0 ;;
esac
touch '{}'
while true; do sleep 0.05; done
"#,
        fixture.scenario.path("started").display()
    ));
    let mut held = fixture.scenario.run_until(
        &[
            "experiment",
            "candidate",
            "--trials-per-task",
            "3",
            "--json",
        ],
        &NO_SANDBOX,
        "started",
    );
    let status = fixture.scenario.run(&["status", "--json"]);
    assert_eq!(status.code, Some(0), "{}", status.stderr);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let name = status["commands"][0]["name"].as_str().unwrap();
    let stopped = fixture.scenario.run(&["stop", name, "--json"]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    let stopped: Value = serde_json::from_str(&stopped.stdout).unwrap();
    assert_eq!(stopped["outcome"], "interrupted");
    let before = fixture.experiment();
    assert_eq!(before["state"], "interrupted");
    let finished = before["results"].as_array().unwrap();
    assert!(!finished.is_empty());
    assert!(finished.len() < 6);
    let calls_before = fixture.trial_calls().len();
    let changed = fixture.run(&["--trials-per-task", "3", "--seed", "1"]);
    assert_eq!(changed.code, Some(1));
    assert!(
        changed
            .stderr
            .contains("Bar cannot change after the first Trial"),
        "{}",
        changed.stderr
    );
    assert_eq!(fixture.trial_calls().len(), calls_before);
    fixture.scenario.agent_does("true");
    let resumed = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let after = fixture.experiment();
    assert_eq!(after["state"], "completed");
    assert_eq!(after["id"], before["id"]);
    assert_eq!(after["results"].as_array().unwrap().len(), 6);
    for result in finished {
        let saved = after["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|saved| saved["arm"] == result["arm"] && saved["trial"] == result["trial"])
            .unwrap();
        assert_eq!(
            saved["attempts"], result["attempts"],
            "finished Trial reran"
        );
        assert_eq!(saved["outcome"], result["outcome"]);
    }
    assert_eq!(
        fixture.trial_calls().len(),
        calls_before + 6 - finished.len()
    );
    let activity = fs::read_to_string(
        fixture
            .scenario
            .path("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(
        activity.contains("Experiment stopped from the CLI by"),
        "{activity}"
    );
}

#[test]
fn plan_limit_warnings_pause_both_workers_and_rerunning_resumes_void_trials() {
    for (harness, event) in [
        (
            "claude",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","rateLimitType":"seven_day"}}"#,
        ),
        (
            "codex",
            r#"{"type":"token_count","rate_limits":{"secondary":{"used_percent":85}}}"#,
        ),
    ] {
        let fixture = Fixture::new();
        fixture
            .scenario
            .agent_does(&format!("echo '{event}'\nsleep 0.5\n"));
        let args = [
            "--trials-per-task",
            "3",
            "--baseline-harness",
            harness,
            "--candidate-harness",
            harness,
        ];
        let paused = fixture.run(&args);
        assert_eq!(paused.code, Some(1), "{harness}: {}", paused.stderr);
        assert!(paused.stderr.contains("paused"), "{}", paused.stderr);
        let before = fixture.experiment();
        assert_eq!(before["state"], "paused");
        assert!(before["results"].as_array().unwrap().is_empty(), "{before}");
        let calls = || {
            let calls = if harness == "claude" {
                fixture.scenario.claude_calls()
            } else {
                fixture.scenario.codex_calls()
            };
            calls
                .into_iter()
                .filter(|call| !is_grader(call))
                .collect::<Vec<_>>()
        };
        let started = calls().len();
        assert!(
            (1..=2).contains(&started),
            "started {started} Trials after a warning"
        );

        fixture.scenario.agent_does("true");
        let resumed = fixture.run(&args);
        assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
        let after = fixture.experiment();
        assert_eq!(after["id"], before["id"]);
        assert_eq!(after["state"], "completed");
        assert_eq!(after["results"].as_array().unwrap().len(), 6);
        assert_eq!(calls().len(), started + 6, "void Trials must be retried");
        for result in after["results"].as_array().unwrap() {
            assert_eq!(result["outcome"], "succeeded");
        }
    }
}

#[test]
fn a_warning_blocks_the_other_worker_while_the_warned_harness_is_stopping() {
    let fixture = Fixture::new();
    let warned = fixture.scenario.path("warning-emitted");
    fixture.scenario.agent_does(&format!(
        r#"
case "$PWD" in
  *--baseline--0/*)
    trap 'sleep 1; exit 0' TERM
    while [ ! -f '{}.peer' ]; do sleep 0.01; done
    echo '{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed_warning"}}}}'
    touch '{}'
    while true; do sleep 0.05; done
    ;;
  *)
    touch '{}.peer'
    while [ ! -f '{}' ]; do sleep 0.01; done
    sleep 0.2
    ;;
esac
"#,
        warned.display(),
        warned.display(),
        warned.display(),
        warned.display()
    ));
    let paused = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    assert_eq!(fixture.experiment()["state"], "paused");
    assert_eq!(
        fixture.trial_calls().len(),
        2,
        "the other worker started work after the warning"
    );
    fixture.scenario.agent_does("true");
    let resumed = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    assert_eq!(
        fixture.trial_calls().len(),
        7,
        "resume must keep the other worker's finished Trial"
    );
}

#[test]
fn older_code_review_arms_stop_at_native_limits_in_their_fresh_homes() {
    for trace in [false, true] {
        assert_older_arm_native_limits("code-review", 3, trace);
    }
}

#[test]
fn older_arms_native_limits_are_monitored_in_their_fresh_homes() {
    for trace in [false, true] {
        assert_older_arm_native_limits("spec-review", 1, trace);
    }
}

fn assert_older_arm_native_limits(session: &str, trials: usize, trace: bool) {
    let fixture = Fixture::with_session(session, trials);
    let continued = fixture.scenario.path("continued-after-native-warning");
    for commit in [&fixture.baseline, &fixture.candidate] {
        let binary = fixture.scenario.path(&format!(
            "home/.thirdshift/evals/builds/{commit}/thirdshift"
        ));
        // A version-1 Arm with a sandbox home, before live quota detection.
        // Cover both historical storage formats. Native quota records are
        // absent from stdout, so the monitor must decode the session ID first.
        fs::write(&binary, format!(r#"#!/usr/bin/env python3
import datetime, json, pathlib, sys, time
request = json.load(open(sys.argv[2]))
output = pathlib.Path(request['output_dir'])
native = output / 'home/.codex/sessions/2026/10/24/rollout-fake-thread-1.jsonl'
native.parent.mkdir(parents=True)
event = json.dumps(dict(type='thread.started', thread_id='fake-thread-1'))
suffix = 'jsonl'
if '{trace}' == 'true':
    event = json.dumps(dict(resourceLogs=[dict(scopeLogs=[dict(logRecords=[dict(body=dict(stringValue=event))])])]))
    suffix = 'logs.jsonl'
(output / ('trial-{session}.' + suffix)).write_text(event + '\n')
native.write_text(json.dumps(dict(type='session_meta', payload=dict(id='fake-thread-1', cwd=request['repository']))) + '\n')
time.sleep(0.2)
with native.open('a') as records:
    records.write(json.dumps(dict(timestamp=datetime.datetime.now(datetime.timezone.utc).isoformat(), type='event_msg', payload=dict(type='token_count', rate_limits=dict(secondary=dict(used_percent=85))))) + '\n')
time.sleep(2)
pathlib.Path({continued:?}).touch()
sys.exit(99)
"#)).unwrap();
        fs::set_permissions(binary, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = fixture.scenario.run(&[
        "experiment",
        "candidate",
        "--json",
        "--baseline-harness",
        "codex",
        "--candidate-harness",
        "codex",
    ]);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["state"], "paused");
    assert!(
        experiment["unfinished_trials"]
            .as_array()
            .unwrap()
            .iter()
            .any(|trial| {
                trial["attempts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|attempt| attempt["void_cause"] == "usage_limit")
            }),
        "the older Arm's native quota warning was not retained: {experiment}"
    );
    assert!(
        !continued.exists(),
        "an older Arm continued after its native warning"
    );
}

#[test]
fn a_baseline_built_before_this_change_pauses_at_its_live_warning() {
    let fixture = Fixture::new();
    let older = std::env::var_os("THIRDSHIFT_REVIEW_BASELINE_BIN").map(PathBuf::from).unwrap_or_else(|| {
        // A frozen version-1 Trial adapter that records Harness stdout and
        // succeeds, as the real pre-detector baseline did. The environment
        // override also exercises the exact pinned baseline binary locally.
        let path = fixture.scenario.path("legacy-thirdshift");
        fs::write(&path, r#"#!/bin/bash
set -e
request=$2
repository=$(sed -n 's/.*"repository": *"\([^"]*\)".*/\1/p' "$request")
output=$(sed -n 's/.*"output_dir": *"\([^"]*\)".*/\1/p' "$request")
mkdir -p "$output"
cd "$repository"
claude --output-format stream-json -p 'https://github.com/acme/widgets/issues/7' > "$output/trial-spec-review.jsonl"
printf '{"version":1,"outcome":"succeeded","timings":{"total_ms":1,"session_ms":1},"void_cause":null,"cause":null,"session_id":null,"log":"%s/trial-spec-review.jsonl","model_evidence":null,"reports":{"standards":null,"spec":null}}\n' "$output"
"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    });
    fs::copy(
        older,
        fixture.scenario.path(&format!(
            "home/.thirdshift/evals/builds/{}/thirdshift",
            fixture.baseline,
        )),
    )
    .unwrap();
    let continued = fixture.scenario.path("baseline-continued-after-warning");
    fixture.scenario.agent_does(&format!(r#"
case "$PWD" in
  *--baseline--*)
    echo '{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed_warning","rateLimitType":"seven_day"}}}}'
    sleep 0.3
    touch '{}'
    ;;
  *) sleep 0.1 ;;
esac
"#, continued.display()));
    let result = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(
        result.code,
        Some(1),
        "the older Baseline's warning must pause the Experiment: stdout={}, stderr={}",
        result.stdout,
        result.stderr
    );
    assert_eq!(fixture.experiment()["state"], "paused");
    assert!(!continued.exists(), "the warned Trial kept working");
    assert!(
        fixture.scenario.claude_calls().len() <= 2,
        "another Trial started after the warning"
    );
}

#[test]
fn a_plan_limit_pause_spends_its_retry_budget_even_if_the_repository_changed() {
    let fixture = Fixture::new();
    // A worker still preparing its repository has no attempt to exhaust.
    // Both Trials must start before either warns for this zero-retry check.
    let started = fixture.scenario.path("limited-trials-started");
    fs::create_dir(&started).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"
case "$PWD" in *--baseline--*) arm=baseline ;; *) arm=candidate ;; esac
touch '{started}'/"$arm"
while [ ! -f '{started}/baseline' ] || [ ! -f '{started}/candidate' ]; do sleep 0.01; done
echo changed > changed-during-limited-trial.txt
echo '{{"type":"rate_limit_event","rate_limit_info":{{"status":"allowed_warning"}}}}'
sleep 0.2
"#,
        started = started.display()
    ));
    let paused = fixture.run(&["--void-retries", "0"]);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    assert_eq!(fixture.experiment()["state"], "paused");
    fixture.scenario.agent_does("true");
    let resumed = fixture.run(&["--void-retries", "0"]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["state"], "completed");
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(
            result["outcome"], "void",
            "a plan-limit stop must respect the Bar's zero retries: {experiment}"
        );
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(result["attempts"][0]["void_cause"], "usage_limit");
    }
    assert_eq!(fixture.trial_calls().len(), 2);
}

#[test]
fn two_workers_interleave_the_arms_without_exceeding_two_active_trials() {
    let fixture = Fixture::new();
    let directory = fixture.scenario.path("concurrency");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("active"), "0").unwrap();
    fs::write(directory.join("maximum"), "0").unwrap();
    fixture.scenario.agent_does(&format!(
        r#"
state='{}'
case "$PWD" in *--baseline--*) arm=baseline ;; *) arm=candidate ;; esac
while ! mkdir "$state/lock" 2>/dev/null; do sleep 0.01; done
active=$(cat "$state/active")
active=$((active + 1))
echo "$active" > "$state/active"
maximum=$(cat "$state/maximum")
if [ "$active" -gt "$maximum" ]; then echo "$active" > "$state/maximum"; fi
echo "$arm" >> "$state/starts"
rmdir "$state/lock"
sleep 0.3
while ! mkdir "$state/lock" 2>/dev/null; do sleep 0.01; done
active=$(cat "$state/active")
echo $((active - 1)) > "$state/active"
rmdir "$state/lock"
"#,
        directory.display()
    ));
    let result = fixture.run(&["--trials-per-task", "3"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        fs::read_to_string(directory.join("maximum"))
            .unwrap()
            .trim(),
        "2"
    );
    assert_eq!(
        fs::read_to_string(directory.join("active")).unwrap().trim(),
        "0"
    );
    let starts = fs::read_to_string(directory.join("starts")).unwrap();
    let starts: Vec<_> = starts.lines().collect();
    assert_eq!(starts.len(), 6);
    assert_ne!(starts[0], starts[1], "{starts:?}");
    assert_eq!(starts.iter().filter(|arm| **arm == "baseline").count(), 3);
    assert_eq!(starts.iter().filter(|arm| **arm == "candidate").count(), 3);
}

#[test]
fn a_timeout_is_failed_even_after_repository_changes_and_its_descendants_end() {
    let fixture = Fixture::new();
    let escaped = fixture.scenario.path("escaped-after-timeout");
    fixture.scenario.agent_does(&format!(
        r#"
printf changed > changed-by-agent.txt
(sleep 3; echo escaped > '{}') &
while true; do sleep 0.05; done
"#,
        escaped.display()
    ));
    let result = fixture.run(&["--timeout", "1"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let experiment = fixture.experiment();
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "failed");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(result["attempts"][0]["cause"], "Trial timed out");
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert!(!escaped.exists(), "timeout left a descendant running");
}

#[test]
fn login_failures_retry_twice_fresh_drop_both_arms_and_show_causes() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
case "$PWD" in *--candidate--*)
echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["login expired"]}' >> "$FAKE_CLAUDE_AFTER_RESULT"
;; esac
"#);
    let result = fixture.run(&[]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["results"].as_array().unwrap().len(), 2);
    assert_eq!(experiment["verdict"]["verdict"], "inconclusive");
    let suite = &experiment["verdict"]["suites"]["spec-review"];
    assert_eq!(suite["dropped_tasks"], 1);
    assert_eq!(suite["retained_tasks"], 0);
    for result in experiment["results"].as_array().unwrap() {
        if result["arm"] == "baseline" {
            assert_eq!(result["outcome"], "succeeded");
            assert_eq!(result["attempts"].as_array().unwrap().len(), 1);
            continue;
        }
        assert_eq!(result["outcome"], "void");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 3);
        for attempt in result["attempts"].as_array().unwrap() {
            assert_eq!(attempt["void_cause"], "session_unavailable");
        }
    }
    let calls = fixture.trial_calls();
    assert_eq!(calls.len(), 4);
    let repositories: std::collections::BTreeSet<_> = calls
        .iter()
        .map(|call| call["cwd"].as_str().unwrap())
        .collect();
    assert_eq!(repositories.len(), 4);
    assert_eq!(experiment["voids"]["baseline"]["count"], 0);
    assert_eq!(experiment["voids"]["candidate"]["count"], 3);
    assert_eq!(
        experiment["voids"]["candidate"]["causes"]["session_unavailable"],
        3
    );
    let shown = fixture
        .scenario
        .run(&["experiments", experiment["id"].as_str().unwrap()]);
    assert_eq!(shown.code, Some(0), "{}", shown.stderr);
    assert!(
        shown.stdout.contains("Baseline: 0 Void Trials"),
        "{}",
        shown.stdout
    );
    assert!(
        shown
            .stdout
            .contains("Candidate: 3 Void Trials (session_unavailable: 3)"),
        "{}",
        shown.stdout
    );
}

#[test]
fn provider_limit_pauses_consume_the_fixed_retry_budget_across_resumes() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["quota exceeded"]}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#);
    for _ in 0..3 {
        let paused = fixture.run(&[]);
        assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    }
    let finished = fixture.run(&[]);
    assert_eq!(finished.code, Some(0), "{}", finished.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["state"], "completed");
    assert_eq!(
        experiment["verdict"]["suites"]["spec-review"]["dropped_tasks"],
        1
    );
    for arm in ["baseline", "candidate"] {
        assert_eq!(experiment["voids"][arm]["count"], 3);
        assert_eq!(experiment["voids"][arm]["causes"]["usage_limit"], 3);
    }
    assert_eq!(fixture.trial_calls().len(), 6);
}

#[test]
fn an_unexplained_trial_process_kill_fails_without_a_retry() {
    let fixture = Fixture::new();
    for commit in [&fixture.baseline, &fixture.candidate] {
        fs::write(
            fixture.scenario.path(&format!(
                "home/.thirdshift/evals/builds/{commit}/thirdshift"
            )),
            "#!/bin/sh\nkill -KILL $$\n",
        )
        .unwrap();
    }
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "failed", "{result}");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(result["attempts"][0]["void_cause"], Value::Null);
    }
    assert_eq!(
        experiment["verdict"]["suites"]["spec-review"]["dropped_tasks"],
        0
    );
}

#[test]
fn an_agent_killing_its_own_harness_fails_without_a_retry() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does("kill -KILL \"$PPID\"");
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    for result in experiment["results"].as_array().unwrap() {
        assert_eq!(
            result["outcome"], "failed",
            "agent action must count against its Arm: {result}"
        );
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1);
    }
    assert_eq!(
        experiment["verdict"]["suites"]["spec-review"]["dropped_tasks"],
        0
    );
}

#[test]
fn resuming_an_experiment_preserves_its_preexisting_void_rules() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(r#"
echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["quota exceeded"]}' >> "$FAKE_CLAUDE_AFTER_RESULT"
"#);
    let paused = fixture.run(&[]);
    assert_eq!(paused.code, Some(1), "{}", paused.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["state"], "paused");
    let metadata = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}/experiment.json",
        experiment["id"].as_str().unwrap()
    ));
    // A version-1 Experiment saved before the additional external causes.
    let old_causes = json!([
        "session_unavailable",
        "usage_limit",
        "rate_limit",
        "interrupted",
        "repository_changed"
    ]);
    let mut saved: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    saved["bar"]["void_rules"]["causes"] = old_causes.clone();
    fs::write(&metadata, serde_json::to_vec(&saved).unwrap()).unwrap();
    fixture.scenario.agent_does("true");
    let resumed = fixture.run(&[]);
    assert_eq!(
        resumed.code,
        Some(0),
        "the same command must resume its saved Bar: {}",
        resumed.stderr
    );
    let after = fixture.experiment();
    assert_eq!(after["state"], "completed");
    assert_eq!(after["bar"]["void_rules"]["causes"], old_causes);
}

#[test]
fn externally_reported_host_kills_retry_fresh_and_keep_the_cause() {
    let fixture = Fixture::new();
    let output = json!({
        "version": 1, "outcome": "void", "void_cause": "host_killed",
        "cause": "Outer launcher reported a host kill",
        "timings": {"total_ms": 50, "session_ms": 40},
        "session_id": null, "log": null, "model_evidence": null,
        "reports": {"standards": null, "spec": null}
    });
    for commit in [&fixture.baseline, &fixture.candidate] {
        fs::write(
            fixture.scenario.path(&format!(
                "home/.thirdshift/evals/builds/{commit}/thirdshift"
            )),
            format!("#!/bin/sh\ncat <<'TRIAL_RESULT'\n{output}\nTRIAL_RESULT\n"),
        )
        .unwrap();
    }
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    for result in fixture.experiment()["results"].as_array().unwrap() {
        assert_eq!(result["outcome"], "void");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 3);
        for attempt in result["attempts"].as_array().unwrap() {
            assert_eq!(attempt["void_cause"], "host_killed");
        }
    }
}

#[test]
fn the_bar_never_allows_more_than_two_void_retries() {
    let fixture = Fixture::new();
    let run = fixture.run(&["--void-retries", "3"]);
    assert_eq!(run.code, Some(2), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("Void Trials allow at most two fresh retries"),
        "{}",
        run.stderr
    );
    assert!(fixture.scenario.claude_calls().is_empty());
}

#[test]
fn trial_only_memory_caps_are_counted_per_arm_and_repeated_caps_are_findings() {
    for (retries, count, repeated) in [("0", 1, false), ("2", 3, true)] {
        let fixture = Fixture::new();
        // The outer launcher reports a cap from outside the agent's sandbox.
        // An agent's own out-of-memory diagnostic is covered at the Trial seam.
        let output = json!({
            "version": 1, "outcome": "void", "void_cause": "memory_cap",
            "cause": "Trial scope memory.events reported an oom_kill",
            "timings": {"total_ms": 50, "session_ms": 40},
            "session_id": null, "log": null, "model_evidence": null,
            "reports": {"standards": null, "spec": null}
        });
        fs::write(
            fixture.scenario.path(&format!(
                "home/.thirdshift/evals/builds/{}/thirdshift",
                fixture.candidate
            )),
            format!("#!/bin/sh\ncat <<'TRIAL_RESULT'\n{output}\nTRIAL_RESULT\n"),
        )
        .unwrap();
        let run = fixture.run(&["--void-retries", retries]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let experiment = fixture.experiment();
        assert_eq!(experiment["voids"]["baseline"]["count"], 0);
        assert_eq!(experiment["voids"]["candidate"]["count"], count);
        assert_eq!(
            experiment["voids"]["candidate"]["causes"]["memory_cap"],
            count
        );
        let findings = experiment["findings"].as_array().unwrap();
        assert_eq!(!findings.is_empty(), repeated, "{experiment}");
        let shown = fixture
            .scenario
            .run(&["experiments", experiment["id"].as_str().unwrap()]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        assert_eq!(
            shown
                .stdout
                .contains("Finding: Candidate Arm repeatedly hit its memory cap (3 Void Trials)"),
            repeated,
            "{}",
            shown.stdout
        );
    }
}

#[test]
fn review_code_graders_raise_their_own_oom_score() {
    if !std::path::Path::new("/proc/self/oom_score_adj").is_file() {
        println!("SKIPPED (code Grader OOM score): no /proc/self/oom_score_adj");
        return;
    }
    let fixture = Fixture::new();
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    fs::write(&task, manifest.replace("[inputs]", "checks = [[\"sh\", \"-c\", \"printf CODE_GRADER_ADJ=; cat /proc/self/oom_score_adj\"]]\n[inputs]")).unwrap();
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert!(
        run.stdout.contains("CODE_GRADER_ADJ=1000"),
        "Code Graders must raise their own score: {}",
        run.stdout
    );
}

#[test]
fn review_an_unreachable_manager_cannot_dispatch_uncapped_trials() {
    let fixture = Fixture::new();
    let run = fixture.scenario.run_with_env(
        &["experiment", "candidate", "--json"],
        &[
            ("THIRDSHIFT_TRIAL_NO_SANDBOX", "1"),
            ("XDG_RUNTIME_DIR", "/nonexistent-thirdshift-review-runtime"),
            (
                "DBUS_SESSION_BUS_ADDRESS",
                "unix:path=/nonexistent-thirdshift-review-bus",
            ),
        ],
    );
    // Refusing dispatch is acceptable when no capped scope can be created.
    if fixture.trial_calls().is_empty() {
        assert_eq!(run.code, Some(1), "{}", run.stderr);
        assert!(
            run.stderr.contains("reachable systemd user manager"),
            "{}",
            run.stderr
        );
        return;
    }
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    let trials = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}/trials",
        experiment["id"].as_str().unwrap()
    ));
    for trial in fs::read_dir(trials).unwrap().map(Result::unwrap) {
        for attempt in fs::read_dir(trial.path()).unwrap().map(Result::unwrap) {
            if attempt.path().join("result.json").is_file() {
                assert!(
                    attempt.path().join("scope").is_file(),
                    "Trial dispatched without its required memory scope: {:?}; stderr: {}",
                    attempt.path(),
                    run.stderr
                );
            }
        }
    }
}

#[test]
fn review_a_cap_killed_tool_cannot_leave_a_successful_trial() {
    if !scope_available() {
        println!("SKIPPED (a cap-killed tool voids its Trial): no systemd user manager");
        return;
    }
    let (Ok(bus), Ok(runtime)) = (
        std::env::var("DBUS_SESSION_BUS_ADDRESS"),
        std::env::var("XDG_RUNTIME_DIR"),
    ) else {
        println!("SKIPPED (a cap-killed tool voids its Trial): no user bus in test env");
        return;
    };
    let fixture = Fixture::new();
    fs::write(
        fixture.scenario.path(&format!(
            "home/.thirdshift/evals/builds/{}/thirdshift",
            fixture.candidate
        )),
        r#"#!/usr/bin/python3
import json, signal, subprocess, sys, time
from pathlib import Path
signal.signal(signal.SIGTERM, signal.SIG_IGN)
request = json.load(open(sys.argv[2]))
output = Path(request['output_dir'])
output.mkdir(parents=True, exist_ok=True)
report = output / 'standards.md'
report.write_text('Standards report\n')
time.sleep(1)
subprocess.run([sys.executable, '-c', 'a=[bytearray(1000000) for _ in range(2000)]'])
print(json.dumps(dict(version=1, outcome='succeeded', void_cause=None, cause=None,
    timings=dict(total_ms=1, session_ms=1), session_id=None, log=None,
    model_evidence=None, reports=dict(standards=str(report), spec=None))), flush=True)
"#,
    )
    .unwrap();
    let run = fixture.scenario.run_with_env(
        &["experiment", "candidate", "--json"],
        &[
            ("THIRDSHIFT_TRIAL_NO_SANDBOX", "1"),
            ("THIRDSHIFT_TRIAL_MEMORY_HIGH", "300M"),
            ("THIRDSHIFT_TRIAL_MEMORY_MAX", "300M"),
            ("THIRDSHIFT_TRIAL_MEMORY_SWAP_MAX", "0"),
            ("DBUS_SESSION_BUS_ADDRESS", bus.as_str()),
            ("XDG_RUNTIME_DIR", runtime.as_str()),
        ],
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    let candidate = experiment["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["arm"] == "candidate")
        .unwrap();
    assert_eq!(
        candidate["outcome"], "void",
        "a capped Trial must be void: {candidate}"
    );
    assert_eq!(candidate["attempts"][0]["void_cause"], "memory_cap");
}

#[test]
fn a_trial_over_its_cap_dies_alone_and_is_void_while_others_carry_on() {
    if !scope_available() {
        println!("SKIPPED (a Trial over its cap dies alone): no systemd user manager");
        return;
    }
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("SKIPPED (a Trial over its cap dies alone): no python3");
        return;
    }
    // Forward the real user bus past the fixture's pass-through manager so
    // this acceptance test exercises kernel-enforced memory caps.
    let (Ok(bus), Ok(runtime)) = (
        std::env::var("DBUS_SESSION_BUS_ADDRESS"),
        std::env::var("XDG_RUNTIME_DIR"),
    ) else {
        println!("SKIPPED (a Trial over its cap dies alone): no user bus in test env");
        return;
    };
    let fixture = Fixture::new();
    // The Candidate Arm eats 2 GiB under a small test cap, so its Trial dies
    // alone; the Baseline Arm runs the default fake Harness and carries on.
    fs::write(
        fixture.scenario.path(&format!(
            "home/.thirdshift/evals/builds/{}/thirdshift",
            fixture.candidate
        )),
        "#!/bin/sh\nexec python3 -c 'a=[bytearray(1000000) for _ in range(2000)]'\n",
    )
    .unwrap();
    let run = fixture.scenario.run_with_env(
        &["experiment", "candidate", "--json"],
        &[
            ("THIRDSHIFT_TRIAL_NO_SANDBOX", "1"),
            ("THIRDSHIFT_TRIAL_MEMORY_HIGH", "300M"),
            ("THIRDSHIFT_TRIAL_MEMORY_MAX", "300M"),
            ("THIRDSHIFT_TRIAL_MEMORY_SWAP_MAX", "0"),
            ("DBUS_SESSION_BUS_ADDRESS", bus.as_str()),
            ("XDG_RUNTIME_DIR", runtime.as_str()),
        ],
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment: Value = serde_json::from_str(&run.stdout).unwrap();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "{experiment}");
    let baseline = results
        .iter()
        .find(|result| result["arm"] == "baseline")
        .unwrap();
    let candidate = results
        .iter()
        .find(|result| result["arm"] == "candidate")
        .unwrap();
    assert_eq!(baseline["outcome"], "succeeded", "{baseline}");
    assert_eq!(candidate["outcome"], "void", "{candidate}");
    // The default Bar retries Void Trials twice: three cap kills.
    let attempts = candidate["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 3, "{candidate}");
    for attempt in attempts {
        assert_eq!(attempt["void_cause"], "memory_cap", "{attempt}");
        assert!(
            attempt["cause"].as_str().unwrap().contains("memory.events"),
            "{attempt}"
        );
    }
    assert_eq!(experiment["voids"]["candidate"]["causes"]["memory_cap"], 3);
    let findings = experiment["findings"].as_array().unwrap();
    assert!(
        findings.iter().any(|finding| finding
            .as_str()
            .unwrap()
            .contains("Candidate Arm repeatedly hit its memory cap")),
        "{findings:?}"
    );
    // Both Trials and the model Grader ran in scopes of their own: each
    // attempt and Grader workspace kept its scope's unit name. One Baseline
    // attempt, three Candidate attempts, one Grader workspace.
    let directory = fixture.scenario.path(&format!(
        "home/.thirdshift/evals/experiments/{}",
        experiment["id"].as_str().unwrap()
    ));
    let scopes_in = |area: &str| {
        let mut scopes = Vec::new();
        for holder in fs::read_dir(directory.join(area))
            .unwrap()
            .map(Result::unwrap)
        {
            if !holder.path().is_dir() {
                continue;
            }
            for run in fs::read_dir(holder.path()).unwrap().map(Result::unwrap) {
                if !run.path().is_dir() {
                    continue;
                }
                let scope = run.path().join("scope");
                if scope.is_file() {
                    scopes.push(scope);
                }
            }
        }
        scopes
    };
    let trials = scopes_in("trials");
    let graders = scopes_in("graders");
    assert_eq!(trials.len(), 4, "{trials:?}");
    assert_eq!(graders.len(), 1, "{graders:?}");
    for scope in trials.into_iter().chain(graders) {
        assert!(
            fs::read_to_string(&scope).unwrap().ends_with(".scope"),
            "{scope:?}"
        );
    }
    // The Grader raised its own `oom_score_adj` too, inherited from its scope.
    let mut adjs = Vec::new();
    for holder in fs::read_dir(directory.join("graders"))
        .unwrap()
        .map(Result::unwrap)
    {
        for run in fs::read_dir(holder.path()).unwrap().map(Result::unwrap) {
            if !run.path().is_dir() {
                continue;
            }
            for file in fs::read_dir(run.path()).unwrap().map(Result::unwrap) {
                if file
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "adj")
                {
                    adjs.push(fs::read_to_string(file.path()).unwrap());
                }
            }
        }
    }
    assert_eq!(adjs, ["1000\n"]);
}

#[test]
fn explicit_suites_compare_one_build_on_distinct_harnesses_and_keep_fixed_paths() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(&format!(
        r#"
ln -sf /nonexistent/updated-claude '{}'
ln -sf /nonexistent/updated-codex '{}'
"#,
        fixture.scenario.path("bin/claude").display(),
        fixture.scenario.path("bin/codex").display()
    ));
    let result = fixture.scenario.run_with_env(
        &[
            "experiment",
            "main",
            "--baseline",
            "main",
            "--suite",
            "spec-review",
            "--baseline-harness",
            "claude",
            "--baseline-model",
            "opus",
            "--baseline-effort",
            "high",
            "--candidate-harness",
            "codex",
            "--candidate-model",
            "gpt-6.1-sol",
            "--candidate-effort",
            "max",
            "--trials-per-task",
            "3",
            "--json",
        ],
        &NO_SANDBOX,
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let experiment = fixture.experiment();
    assert_eq!(experiment["baseline"]["commit"], fixture.baseline);
    assert_eq!(experiment["candidate"]["commit"], fixture.baseline);
    assert_eq!(experiment["results"].as_array().unwrap().len(), 6);
    assert_eq!(fixture.trial_calls().len(), 3);
    assert_eq!(fixture.scenario.codex_calls().len(), 3);
    for (arm, harness, effort) in [
        ("baseline", "claude", "high"),
        ("candidate", "codex", "max"),
    ] {
        assert_eq!(experiment[arm]["harness"], harness);
        assert_eq!(experiment[arm]["effort"], effort);
        assert_eq!(
            experiment["prepared"][arm]["harness"]["path"],
            fixture
                .scenario
                .path(&format!("versions/v1/{harness}"))
                .to_str()
                .unwrap()
        );
    }
}

#[test]
fn distinct_suite_task_pairs_keep_distinct_trial_results() {
    let fixture = Fixture::new();
    let suites = fixture.scenario.path("home/.thirdshift/evals/suites");
    let source = suites.join("spec-review");
    for (suite, task) in [("alpha", "beta--gamma"), ("alpha--beta", "gamma")] {
        let target = suites.join(suite);
        let directory = target.join("tasks").join(task);
        fs::create_dir_all(&directory).unwrap();
        fs::copy(source.join("suite.toml"), target.join("suite.toml")).unwrap();
        fs::copy(
            source.join("tasks/fixture/task.toml"),
            directory.join("task.toml"),
        )
        .unwrap();
        fs::copy(
            source.join("tasks/fixture/spec.md"),
            directory.join("spec.md"),
        )
        .unwrap();
    }
    let result = fixture.run(&["--suite", "alpha", "--suite", "alpha--beta"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let experiment: Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(experiment["state"], "completed");
    assert_eq!(fixture.trial_calls().len(), 4);
    assert_eq!(
        experiment["results"].as_array().unwrap().len(),
        4,
        "each suite/task/Arm Trial needs its own saved result: {experiment}"
    );
}

#[test]
fn resuming_a_cut_short_retry_keeps_the_bars_void_retry_budget() {
    let fixture = Fixture::new();
    let void = r#"echo '{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["connection refused"]}' >> "$FAKE_CLAUDE_AFTER_RESULT""#;
    fixture.scenario.agent_does_in_session(1, void);
    fixture.scenario.agent_does_in_session(2, void);
    fixture.scenario.agent_does(&format!(
        r#"
case "$PWD" in *--baseline--*) arm=baseline ;; *) arm=candidate ;; esac
touch '{root}/blocked-'"$arm"
while [ ! -f '{root}/blocked-baseline' ] || [ ! -f '{root}/blocked-candidate' ]; do sleep 0.01; done
touch '{root}/started'
while true; do sleep 0.05; done
"#,
        root = fixture.scenario.path("").display()
    ));
    let mut held = fixture.scenario.run_until(
        &["experiment", "candidate", "--json"],
        &NO_SANDBOX,
        "started",
    );
    let status = fixture.scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let name = status["commands"][0]["name"].as_str().unwrap();
    let stopped = fixture.scenario.run(&["stop", name, "--json"]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    assert_eq!(fixture.trial_calls().len(), 4);
    fixture.scenario.agent_does(void);
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    assert_eq!(
        fixture.trial_calls().len(),
        8,
        "each Arm already completed one Void Trial attempt before Stop"
    );
    for result in fixture.experiment()["results"].as_array().unwrap() {
        assert_eq!(result["attempts"].as_array().unwrap().len(), 3);
    }
}
