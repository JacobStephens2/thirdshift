use super::*;

pub(super) fn set_checks(fixture: &Fixture, commands: &[&[&str]]) {
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let manifest = fs::read_to_string(&task).unwrap();
    fs::write(
        task,
        manifest.replace(
            "[inputs]",
            &format!(
                "checks = {}\n[inputs]",
                serde_json::to_string(commands).unwrap()
            ),
        ),
    )
    .unwrap();
}

#[test]
fn ordered_checks_share_artifacts_and_continue_after_failure() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    set_checks(
        &fixture,
        &[
            &[
                "sh",
                "-c",
                "printf compiled > build-artifact; printf '%s' \"$PWD\"",
            ],
            &[
                "sh",
                "-c",
                "test \"$(cat build-artifact)\" = compiled; exit 7",
            ],
            &[
                "sh",
                "-c",
                "test \"$(cat build-artifact)\" = compiled && test -f \"$THIRDSHIFT_TRIAL_STANDARDS_REPORT\" && test -z \"$THIRDSHIFT_TRIAL_SPEC_REPORT\" && test -z \"$THIRDSHIFT_TRIAL_LOG\"",
            ],
        ],
    );
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["outcome"], "succeeded", "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
        let checks = result["score"]["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 5);
        assert_eq!(checks[2]["passed"], true, "{checks:?}");
        assert_eq!(checks[3]["passed"], false, "{checks:?}");
        assert_eq!(checks[4]["passed"], true, "{checks:?}");
        let snapshot = checks[2]["reason"]
            .as_str()
            .unwrap()
            .lines()
            .nth(1)
            .unwrap();
        assert!(
            !PathBuf::from(snapshot).exists(),
            "packet was not cleaned up"
        );
    }
    for call in fixture.trial_calls() {
        assert!(
            !PathBuf::from(call["cwd"].as_str().unwrap())
                .join("build-artifact")
                .exists()
        );
    }
}

/// The descendant records that the packet still exists while handling TERM.
/// The parent waits for it, so completion proves cleanup precedes packet removal.
fn waiting_check(fixture: &Fixture) -> String {
    let descendant = format!(
        r#"
import os, signal, sys, time
def stopped(*_):
    assert os.path.isdir(os.getcwd())
    assert os.path.isfile(os.environ['THIRDSHIFT_TRIAL_STANDARDS_REPORT'])
    with open({finished:?}, 'a') as output:
        output.write(os.getcwd() + '\n')
    sys.exit(0)
signal.signal(signal.SIGTERM, stopped)
with open({started:?}, 'w') as output:
    output.write('ready')
while True:
    time.sleep(0.05)
"#,
        finished = fixture.scenario.path("check-finished"),
        started = fixture.scenario.path("check-started"),
    );
    format!(
        r#"
import os, signal, subprocess, sys
if os.path.exists({resume:?}):
    sys.exit(0)
child = subprocess.Popen([sys.executable, '-c', {descendant:?}])
def stopped(*_):
    child.wait(timeout=5)
    sys.exit(0)
signal.signal(signal.SIGTERM, stopped)
child.wait()
"#,
        resume = fixture.scenario.path("allow-check-finish"),
    )
}

fn assert_waiting_checks_cleaned_up(fixture: &Fixture) -> usize {
    let finished = fs::read_to_string(fixture.scenario.path("check-finished")).unwrap();
    let snapshots: Vec<_> = finished.lines().collect();
    assert!(
        !snapshots.is_empty(),
        "no descendant completed its TERM handler"
    );
    for snapshot in &snapshots {
        assert!(!PathBuf::from(snapshot).exists(), "packet survived cleanup");
    }
    snapshots.len()
}

#[test]
fn check_timeout_cleans_descendants_before_packet_and_remains_failed() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let script = waiting_check(&fixture);
    set_checks(&fixture, &[&["python3", "-c", &script], &["true"]]);
    let run = fixture.run(&["--timeout", "1"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["outcome"], "succeeded", "{result}");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1, "{result}");
        assert_eq!(result["score"]["passed"], false, "{result}");
        assert_eq!(result["score"]["checks"][2]["passed"], false, "{result}");
        assert_eq!(
            result["score"]["checks"][2]["reason"], "Trial timed out",
            "{result}"
        );
        assert_eq!(result["score"]["checks"][3]["passed"], true, "{result}");
    }
    assert_eq!(assert_waiting_checks_cleaned_up(&fixture), 2);
}

#[test]
fn interrupted_check_cleans_up_without_a_score_and_experiment_resumes() {
    let fixture = Fixture::new();
    fixture.scenario.agent_does(review_reports_prelude());
    let script = waiting_check(&fixture);
    set_checks(&fixture, &[&["python3", "-c", &script]]);
    let run = fixture.scenario.run_and_signal_with_env(
        &["experiment", "candidate", "--json"],
        &NO_SANDBOX,
        "check-started",
        "TERM",
    );
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    let before = fixture.experiment();
    assert_eq!(before["state"], "interrupted", "{before}");
    assert!(before["results"].as_array().unwrap().is_empty(), "{before}");
    assert!(
        fixture.grader_calls().is_empty(),
        "grading continued after interruption"
    );
    assert_waiting_checks_cleaned_up(&fixture);

    fs::write(fixture.scenario.path("allow-check-finish"), "resume").unwrap();
    let resumed = fixture.run(&[]);
    assert_eq!(resumed.code, Some(0), "{}", resumed.stderr);
    let after = fixture.experiment();
    assert_eq!(after["id"], before["id"]);
    assert_eq!(after["state"], "completed");
    let results = after["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["outcome"], "succeeded", "{result}");
        assert_eq!(result["score"]["passed"], true, "{result}");
        assert_eq!(result["attempts"].as_array().unwrap().len(), 1, "{result}");
    }
}

#[test]
fn snapshot_preserves_files_directories_and_symlinks_but_skips_special_files() {
    let fixture = Fixture::new();
    // These files belong to the prepared repository, before the session.
    // Keep the Arm branches pinned while extending only the task's source.
    fixture
        .scenario
        .launch_git(&["checkout", "-b", "prepared-files"]);
    fs::create_dir(fixture.scenario.launch_dir().join("nested")).unwrap();
    fixture
        .scenario
        .commit_locally("nested/file", "ordinary", "Prepared file");
    std::os::unix::fs::symlink("nested/file", fixture.scenario.launch_dir().join("link")).unwrap();
    fixture.scenario.launch_git(&["add", "link"]);
    fixture
        .scenario
        .launch_git(&["commit", "-m", "Prepared symlink"]);
    let commit = fixture.scenario.launch_git(&["rev-parse", "HEAD"]);
    fixture.scenario.launch_git(&["checkout", "candidate"]);
    let task = fixture
        .scenario
        .path("home/.thirdshift/evals/suites/spec-review/tasks/fixture/task.toml");
    let mut manifest = fs::read_to_string(&task)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    manifest["base"] = toml_edit::value(commit.trim());
    manifest["change"] = toml_edit::value(commit.trim());
    fs::write(task, manifest.to_string()).unwrap();
    fixture.scenario.agent_does(&format!(
        r#"
{reports}
mkfifo pipe
python3 -c 'import socket; s=socket.socket(socket.AF_UNIX); s.bind("server.sock")'
exit 1
"#,
        reports = review_reports_prelude(),
    ));
    set_checks(
        &fixture,
        &[&[
            "sh",
            "-c",
            "test \"$(cat nested/file)\" = ordinary && test -L link && test \"$(readlink link)\" = nested/file && test \"$(cat link)\" = ordinary && test ! -e pipe && test ! -e server.sock",
        ]],
    );
    let run = fixture.run(&[]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let experiment = fixture.experiment();
    let results = experiment["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    for result in results {
        // A failed Trial still gets code checks on its prepared files;
        // session leftovers must not fail the Experiment or enter checks.
        assert_eq!(result["outcome"], "failed", "{result}");
        assert_eq!(result["score"]["checks"][2]["passed"], true, "{result}");
    }
    use std::os::unix::fs::FileTypeExt;
    for call in fixture.trial_calls() {
        let repository = PathBuf::from(call["cwd"].as_str().unwrap());
        assert!(
            fs::symlink_metadata(repository.join("pipe"))
                .unwrap()
                .file_type()
                .is_fifo()
        );
        assert!(
            fs::symlink_metadata(repository.join("server.sock"))
                .unwrap()
                .file_type()
                .is_socket()
        );
        assert_eq!(
            fs::read_link(repository.join("link")).unwrap(),
            PathBuf::from("nested/file")
        );
        assert_eq!(
            fs::read_to_string(repository.join("nested/file")).unwrap(),
            "ordinary"
        );
    }
}
