use super::*;

const ID: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn verdict(outcome: &str) -> Value {
    json!({
        "id": ID, "state": "completed", "pending_rechecks": [],
        "verdict": {"verdict": outcome, "suites": {"code-review": {
            "accuracy": {"interval": {"lower": -0.25, "upper": 0.12}},
            "wrong_findings": {"interval": {"lower": -0.02, "upper": 0.18}}
        }}}
    })
}

fn cargo_reports(release: &Release, views: &[Value]) {
    fs::write(
        release.root().join("experiment-views.json"),
        json!(views).to_string(),
    )
    .unwrap();
    fs::write(release.bin().join("cargo"), r#"#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys
args = sys.argv[sys.argv.index('--') + 1:]
if args == ['--release-summary']:
    os.execv(os.environ['THIRDSHIFT_TEST_BINARY'], [os.environ['THIRDSHIFT_TEST_BINARY'], *args])
root = pathlib.Path(os.environ['FAKE_GH_STATE']).parent
branch = subprocess.check_output(['git', 'ls-remote', 'origin', 'refs/heads/release-0.2.0'], text=True).strip()
with (root / 'experiment-calls.jsonl').open('a') as calls:
    calls.write(json.dumps(dict(args=args, branch=branch)) + '\n')
views = root / 'experiment-views.json'
remaining = json.loads(views.read_text())
assert remaining, 'unexpected Experiment call'
view = remaining.pop(0)
views.write_text(json.dumps(remaining))
if isinstance(view, int):
    sys.exit(view)
print(json.dumps(view))
"#).unwrap();
}

fn calls(release: &Release) -> Vec<Value> {
    fs::read_to_string(release.root().join("experiment-calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn a_passing_experiment_is_checked_before_any_push() {
    let release = Release::new();
    cargo_reports(&release, &[verdict("pass")]);
    release.ci_reports(GREEN);
    let candidate = release.origin(&["rev-parse", "main"]);
    let output = release.run_script("0.2.0");
    assert!(output.status.success(), "{}", stderr(&output));
    let calls = calls(&release);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]["args"],
        json!(["experiment", "--release", candidate, "--json"])
    );
    assert_eq!(calls[0]["branch"], "");
    assert!(release.origin_tag("v0.2.0").is_some());
}

#[test]
fn failed_or_inconclusive_experiments_show_intervals_and_require_permission() {
    for outcome in ["fail", "inconclusive"] {
        let release = Release::new();
        release.ci_reports(GREEN);
        let before = release.origin_refs();
        for answer in ["", "n\n", "yes\n"] {
            cargo_reports(&release, &[verdict(outcome)]);
            let output = release.run_script_with(&["0.2.0"], answer, &[]);
            let err = stderr(&output);
            assert!(err.contains("release anyway?"), "{err}");
            assert!(err.contains("accuracy [-0.25, 0.12]"), "{err}");
            assert!(err.contains("wrong findings [-0.02, 0.18]"), "{err}");
            if answer == "yes\n" {
                assert!(output.status.success(), "{err}");
                assert!(release.origin_tag("v0.2.0").is_some());
            } else {
                assert!(!output.status.success(), "{err}");
                assert_eq!(release.origin_refs(), before);
                assert!(!release.pr_opened("release-0.2.0"));
            }
        }
        let checked = calls(&release);
        assert_eq!(
            checked[0]["args"], checked[1]["args"],
            "resume must use the same content commit"
        );
    }
}

#[test]
fn pending_rechecks_are_listed_and_waited_for_before_pushing() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let mut waiting = verdict("waiting_on_rechecks");
    waiting["pending_rechecks"] = json!([{
        "id": "trial-1", "suite": "code-review", "task": "fixture", "arm": "candidate",
        "trial": 0, "borderline": true, "decisive": false
    }]);
    cargo_reports(&release, &[waiting.clone(), waiting, verdict("pass")]);
    let output = release.run_script("0.2.0");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("trial-1"), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("waiting for your rechecks"),
        "{}",
        stderr(&output)
    );
    let checked = calls(&release);
    assert_eq!(checked.len(), 3);
    for call in &checked[1..] {
        assert_eq!(call["args"], json!(["experiments", ID, "--json"]));
    }
    assert!(checked.iter().all(|call| call["branch"] == ""));
    assert!(release.origin_tag("v0.2.0").is_some());
}

#[test]
fn resume_reuses_the_content_commit_and_checks_a_pushed_branch_before_opening_its_pr() {
    let release = Release::new();
    let candidate = release.origin(&["rev-parse", "main"]);
    let before = release.origin_refs();
    cargo_reports(&release, &[json!(1)]);
    let stopped = release.run_script("0.2.0");
    assert!(!stopped.status.success());
    assert_eq!(release.origin_refs(), before);
    release.push_bump("0.2.0");
    cargo_reports(&release, &[verdict("fail")]);
    let stopped = release.run_script("0.2.0");
    assert!(!stopped.status.success());
    assert!(!release.pr_opened("release-0.2.0"));
    cargo_reports(&release, &[verdict("pass")]);
    release.ci_reports(GREEN);
    let resumed = release.run_script("0.2.0");
    assert!(resumed.status.success(), "{}", stderr(&resumed));
    let checked = calls(&release);
    assert_eq!(checked.len(), 3);
    for call in &checked {
        assert_eq!(
            call["args"],
            json!(["experiment", "--release", candidate, "--json"])
        );
    }
    assert_ne!(checked[1]["branch"], "");
    assert!(release.origin_tag("v0.2.0").is_some());
}

#[test]
fn skipped_experiments_carry_on_with_a_note() {
    for (code, reason) in [
        ("no_suites", "no Eval suite touched"),
        (
            "baseline_without_trial",
            "last release v0.1.0 has no Trial flag",
        ),
    ] {
        let release = Release::new();
        cargo_reports(&release, &[json!({"release_skip": code, "reason": reason})]);
        release.ci_reports(GREEN);
        let output = release.run_script("0.2.0");
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(stderr(&output).contains(reason), "{}", stderr(&output));
        assert!(release.origin_tag("v0.2.0").is_some());
    }
}
