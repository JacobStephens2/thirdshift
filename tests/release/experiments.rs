//! Release gates use the real Experiment engine, with fake Harnesses at its
//! external seam. Trial Arms execute the compiled binary from its build cache.
use super::*;
use std::time::{Duration, Instant};

const ENV: &[(&str, &str)] = &[("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")];
// Setup, forty Trials, twenty Graders and public result reads share this bound.
const EXPERIMENT_BOUND: Duration = Duration::from_secs(300);

fn cache(release: &Release, commit: &str) {
    let directory = release
        .root()
        .join(format!("home/.thirdshift/evals/builds/{commit}"));
    fs::create_dir_all(&directory).unwrap();
    support::copy_executable(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        &directory.join("thirdshift"),
    );
}

fn guard_pushes(release: &Release, candidate: &str) {
    fs::write(release.root().join("gate-candidate"), candidate).unwrap();
    let git_driver = release.bin().join("git-release");
    if git_driver.exists() {
        fs::remove_file(&git_driver).unwrap();
    }
    fs::rename(release.bin().join("git"), &git_driver).unwrap();
    write_executable(
        &release.bin().join("git"),
        &format!(
            r#"#!/bin/bash
set -euo pipefail
pushing=0
for arg in "$@"; do
    case "$arg" in
    push) pushing=1 ;;
    *:refs/heads/release-*|*:refs/tags/v*)
        if [ "$pushing" = 1 ]; then
            "$THIRDSHIFT_TEST_BINARY" experiments --json > "${{FAKE_GH_STATE%/*}}/gate-view.json"
            python3 - <<'PY'
import json, os, pathlib
root = pathlib.Path(os.environ['FAKE_GH_STATE']).parent
expected = (root / 'gate-candidate').read_text()
views = json.loads((root / 'gate-view.json').read_text())
assert any(view['candidate']['commit'] == expected and view['verdict']['verdict'] == 'pass' for view in views), views
PY
        fi ;;
    esac
done
exec {driver} "$@"
"#,
            driver = json!(git_driver.display().to_string())
        ),
    );
}

fn prepare(release: &Release, tasks: usize) {
    // Keep only the external systemd manager fake; release selection, Trials,
    // grading, persistence and rechecks all run through the installed binary.
    for (name, script) in [
        ("systemctl", "echo running"),
        (
            "systemd-run",
            "while [ \"$#\" -gt 0 ] && [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"",
        ),
        ("journalctl", "exit 0"),
    ] {
        write_executable(&release.bin().join(name), &format!("#!/bin/sh\n{script}\n"));
    }
    let baseline = release.origin_tag("v0.1.0").unwrap();
    let candidate = release.origin(&["rev-parse", "main"]);
    cache(release, &baseline);
    cache(release, &candidate);
    guard_pushes(release, &candidate);
    let suite = release
        .root()
        .join("home/.thirdshift/evals/suites/spec-review");
    fs::create_dir_all(&suite).unwrap();
    fs::write(suite.join("suite.toml"), "format = 1\ntrials_per_arm = 1\n[dependencies]\nfactory_skills = []\nsession_prompts = ['README.md', 'Cargo.toml', 'after-tag.txt']\nharness_defaults = []\nmodel_defaults = []\neffort_defaults = []\n").unwrap();
    // Twenty judged tasks keep this a deciding suite under the production Bar.
    for index in 0..tasks {
        let task = suite.join(format!("tasks/fixture-{index}"));
        fs::create_dir_all(&task).unwrap();
        fs::write(
            task.join("spec.md"),
            "# Fixture Spec\n\nReview this invented change.\n",
        )
        .unwrap();
        fs::write(
            task.join("task.toml"),
            format!(
                r#"format = 1
state = "approved"
session = "spec-review"
repository = {:?}
base = "{baseline}"
change = "{baseline}"
criteria = "Leave the requested review report."
[inputs]
".thirdshift-issue.md" = "spec.md"
[trial]
issue_url = "https://github.com/JacobStephens2/thirdshift/issues/7"
base = "main"
branch = "issue-7"
pr_url = "https://github.com/JacobStephens2/thirdshift/pull/1"
[difficulty]
passes = 3
trials = 6
experiment = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
commit = "{baseline}"
harness = "claude"
"#,
                release.contributor()
            ),
        )
        .unwrap();
    }
    release.agent_runs(r#"
reports=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/.*reports, with each axis.*to `\([^`]*\)\/standards.md`.*/\1/p')
test -n "$reports"
printf 'Standards report\n' > "$reports/standards.md"
printf '%s\n' '{"type":"assistant","message":{"id":"fixture-usage","model":"claude-sonnet-5-5","usage":{"input_tokens":10,"output_tokens":2},"content":[]}}'
"#);
    fs::write(release.root().join("claude-script.sh.grader"), r#"
python3 - <<'PY'
import json, os, pathlib
packet = json.load(open(os.environ['THIRDSHIFT_GRADER_PACKET']))
root = pathlib.Path(os.environ['FAKE_GH_STATE']).parent
borderline = (root / 'borderline').exists()
json.dump({'scores': [dict(label=trial['label'], passed=True, reason='Requested report is present.', wrong_finding=False, borderline=borderline) for trial in packet['trials']]}, open(os.environ['THIRDSHIFT_GRADER_SCORES'], 'w'))
PY
printf '%s\n' '{"type":"assistant","message":{"id":"fixture-usage","model":"claude-sonnet-5-5","usage":{"input_tokens":10,"output_tokens":2},"content":[]}}'
"#).unwrap();
}

fn cli(release: &Release, args: &[&str]) -> Value {
    let output = support::child_lifetime::FixtureChild::spawn(
        release
            .command(
                Path::new(env!("CARGO_BIN_EXE_thirdshift")),
                &release.maintainer(),
            )
            .args(args)
            .envs(ENV.iter().copied()),
        "",
    )
    .wait(EXPERIMENT_BOUND);
    assert!(output.status.success(), "{}", stderr(&output));
    serde_json::from_str(&stdout(&output)).unwrap()
}

/// Runs the Release command with `args`, under the Experiment's bound: its
/// Trials and Graders may run inside the Release run.
fn release_with_experiment(release: &Release, args: &[&str]) -> Output {
    support::child_lifetime::FixtureChild::spawn(
        native::command(release, "")
            .args(args)
            .envs(ENV.iter().copied()),
        "",
    )
    .wait(EXPERIMENT_BOUND)
}

fn views(release: &Release) -> Vec<Value> {
    cli(release, &["experiments", "--json"])
        .as_array()
        .unwrap()
        .clone()
}

fn wait_for(
    release: &Release,
    child: &mut support::child_lifetime::OwnedRun,
    candidate: &str,
    verdict: &str,
) -> Value {
    let deadline = Instant::now() + EXPERIMENT_BOUND;
    loop {
        assert!(
            child.try_wait().is_none(),
            "Release ended before {verdict}: {}",
            fs::read_to_string(release.root().join("experiment-output")).unwrap()
        );
        if let Some(view) = views(release).into_iter().find(|view| {
            view["candidate"]["commit"] == candidate && view["verdict"]["verdict"] == verdict
        }) {
            return view;
        }
        assert!(
            Instant::now() < deadline,
            "Experiment at {candidate} never reached {verdict}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start(
    release: &Release,
    owned: &support::child_lifetime::OwnedRuns,
) -> support::child_lifetime::OwnedRun {
    let mut command = native::command(release, "0.2.0");
    let output = fs::File::create(release.root().join("experiment-output")).unwrap();
    command
        .arg("--with-experiment")
        .envs(ENV.iter().copied())
        .stdout(output.try_clone().unwrap())
        .stderr(output);
    owned.spawn(&mut command)
}

fn finish(release: &Release, child: &mut support::child_lifetime::OwnedRun) {
    let deadline = Instant::now() + support::WAIT_BOUND;
    loop {
        if let Some(status) = child.try_wait() {
            assert!(
                status.success(),
                "{}",
                fs::read_to_string(release.root().join("experiment-output")).unwrap()
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Release did not finish after confirmation"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn complete_experiment(release: &Release, candidate: &str) {
    let measured = cli(
        release,
        &[
            "experiment",
            "--release",
            candidate,
            "--baseline",
            "v0.1.0",
            "--json",
            "--yes",
        ],
    );
    confirm(release, &measured);
}

fn confirm(release: &Release, view: &Value) {
    let id = view["id"].as_str().unwrap();
    let pending = cli(release, &["recheck", id, "--pending", "--json"]);
    cli(
        release,
        &[
            "recheck",
            id,
            "--batch",
            pending["fingerprint"].as_str().unwrap(),
            "pass",
            "--reason",
            "Confirmed fixture reports",
            "--json",
        ],
    );
}

#[test]
fn a_passing_release_experiment_finishes_before_the_first_push() {
    let release = Release::new();
    prepare(&release, 20);
    release.ci_reports(GREEN);
    let candidate = release.origin(&["rev-parse", "main"]);
    complete_experiment(&release, &candidate);
    let calls = fs::read_to_string(release.root().join("claude-calls.json")).unwrap();
    assert!(!release.origin_branch("release-0.2.0"));
    let output = release_with_experiment(&release, &["0.2.0", "--with-experiment"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let experiments = views(&release);
    assert_eq!(experiments.len(), 1);
    assert_eq!(experiments[0]["verdict"]["verdict"], "pass");
    assert_eq!(
        fs::read_to_string(release.root().join("claude-calls.json")).unwrap(),
        calls,
        "completed Trials and Graders must be reused"
    );
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
}

#[test]
fn an_advisory_release_experiment_needs_a_terminal_override() {
    let release = Release::new();
    prepare(&release, 1);
    let before = release.origin_refs();
    let output = release_with_experiment(&release, &["0.2.0", "--with-experiment"]);
    assert!(!output.status.success());
    let experiments = views(&release);
    assert_eq!(experiments[0]["verdict"]["verdict"], "inconclusive");
    assert!(
        experiments[0]["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|result| result["outcome"] == "succeeded"),
        "{}",
        experiments[0]["results"]
    );
    assert!(stderr(&output).contains("unattended release has no terminal override"));
    assert_eq!(release.origin_refs(), before);
}

#[test]
fn a_release_waits_for_real_rechecks_before_pushing() {
    let release = Release::new();
    prepare(&release, 20);
    fs::write(release.root().join("borderline"), "").unwrap();
    release.ci_reports(GREEN);
    let candidate = release.origin(&["rev-parse", "main"]);
    let owned = support::child_lifetime::OwnedRuns::new();
    let mut child = start(&release, &owned);
    let waiting = wait_for(&release, &mut child, &candidate, "waiting_on_rechecks");
    assert!(release.origin_tag("v0.2.0").is_none());
    assert!(!release.origin_branch("release-0.2.0"));
    confirm(&release, &waiting);
    finish(&release, &mut child);
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
}

#[test]
fn repaired_content_waits_for_its_own_experiment_before_tagging() {
    let release = Release::new();
    let (old, repaired) = checks::conflicted_bump(&release);
    let initial = release.origin(&["rev-parse", &format!("{old}^")]);
    let refreshed = release.origin(&["rev-parse", "main"]);
    prepare(&release, 20);
    cache(&release, &initial);
    fs::write(release.root().join("gate-candidate"), &initial).unwrap();
    complete_experiment(&release, &initial);
    let trial_script = fs::read_to_string(release.root().join("claude-script.sh")).unwrap();
    release.agent_runs(&format!(
        r#"
if [[ "$FAKE_CLAUDE_PROMPT" = /thirdshift-resolving-merge-conflicts* ]]; then
    git fetch origin prepared-repair
    git reset --hard {repaired}
    git push origin HEAD:release-0.2.0
    gh fake pr release-0.2.0 mergeable '"MERGEABLE"'
    gh fake checks {repaired} '{GREEN}'
    printf '%s' {refreshed} > "${{FAKE_GH_STATE%/*}}/gate-candidate"
    touch "${{FAKE_GH_STATE%/*}}/borderline"
else
    {trial_script}
fi
"#
    ));
    let owned = support::child_lifetime::OwnedRuns::new();
    let mut child = start(&release, &owned);
    let waiting = wait_for(&release, &mut child, &refreshed, "waiting_on_rechecks");
    assert!(
        views(&release)
            .iter()
            .any(|view| view["candidate"]["commit"] == initial
                && view["verdict"]["verdict"] == "pass")
    );
    assert!(release.origin_tag("v0.2.0").is_none());
    confirm(&release, &waiting);
    finish(&release, &mut child);
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired));
    assert!(
        release
            .pr_body("release-0.2.0")
            .contains("-version = \"0.1.1\"")
    );
}

#[test]
fn unpublished_tag_recovery_keeps_the_prior_experiment_baseline() {
    let release = Release::new();
    release.release_workflow(failed_release());
    prepare(&release, 20);
    release.ci_reports(GREEN);
    let initial = release.origin(&["rev-parse", "main"]);
    complete_experiment(&release, &initial);
    let first = release_with_experiment(&release, &["0.2.0", "--with-experiment"]);
    assert!(!first.status.success(), "{}", stderr(&first));
    let tagged = release.origin_tag("v0.2.0").unwrap();
    release.merge_pr("newer", "Recover with newer content", "");
    release.main_ci("completed", "success");
    let candidate = release.origin(&["rev-parse", "main"]);
    cache(&release, &candidate);
    release.release_workflow(json!({}));
    guard_pushes(&release, &candidate);
    fs::write(release.root().join("borderline"), "").unwrap();
    release.ci_reports(GREEN);
    let owned = support::child_lifetime::OwnedRuns::new();
    let mut child = start(&release, &owned);
    let waiting = wait_for(&release, &mut child, &candidate, "waiting_on_rechecks");
    assert_eq!(
        waiting["baseline"]["commit"],
        release.origin_tag("v0.1.0").unwrap()
    );
    assert_eq!(release.origin_tag("v0.2.0"), Some(tagged));
    confirm(&release, &waiting);
    finish(&release, &mut child);
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    assert_eq!(
        release.origin(&[
            "rev-parse",
            &format!("{}^", release.origin_tag("v0.2.0").unwrap())
        ]),
        candidate
    );
}

#[test]
fn a_pass_without_the_experiment_never_publishes_a_version_pinned_for_one() {
    let release = Release::new();
    prepare(&release, 1);
    let output = release_with_experiment(&release, &["1.0.0", "--with-experiment"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("unattended release has no terminal override"),
        "{}",
        stderr(&output)
    );
    // Drop the push guard, so only the Release run itself can refuse 1.0.0.
    fs::rename(release.bin().join("git-release"), release.bin().join("git")).unwrap();
    let pass = release.run_with(&[], "", ENV);
    assert!(pass.status.success(), "{}", stderr(&pass));
    assert!(!release.origin_branch("release-1.0.0"), "{}", stderr(&pass));
    assert!(release.origin_tag("v1.0.0").is_none(), "{}", stderr(&pass));
    let activity = fs::read_to_string(
        release
            .root()
            .join("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(
        activity.contains("Release run skipped:")
            && activity.contains("thirdshift release 1.0.0 --with-experiment"),
        "{activity}"
    );
}

#[test]
fn a_refused_release_experiment_shows_its_suite_results_first() {
    let release = Release::new();
    prepare(&release, 1);
    let output = release_with_experiment(&release, &["0.2.0", "--with-experiment"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    let shown = err.find("spec-review: Advisory suite").expect(&err);
    assert!(
        shown
            < err
                .find("unattended release has no terminal override")
                .unwrap(),
        "{err}"
    );
}
