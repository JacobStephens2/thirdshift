use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

use super::*;
use crate::evals::experiment::{Arm, trial_results};

/// Environment and interruption state belong to an isolated test process.
fn isolated(env: &[(&str, &str)]) -> bool {
    if std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").is_some() {
        crate::interrupt::install().unwrap();
        return false;
    }
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(root.path().join("home")).unwrap();
    executable(&bin.join("systemctl"), "#!/bin/sh\necho running\n");
    // The test executable has no --oom-exec entry point. This external
    // manager fixture unwraps it and launches the selected executable.
    executable(
        &bin.join("systemd-run"),
        "#!/bin/sh\nwhile [ \"$1\" != -- ]; do shift; done\nshift\nshift\n[ \"$1\" = --oom-exec ] || exit 98\nshift\nexec \"$@\"\n",
    );
    executable(&bin.join("journalctl"), "#!/bin/sh\nexit 0\n");
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            std::thread::current().name().unwrap(),
            "--nocapture",
        ])
        .env("THIRDSHIFT_EXECUTION_FIXTURE", root.path())
        .env("HOME", root.path().join("home"))
        .env("PATH", path)
        .env("THIRDSHIFT_TRIAL_NO_SANDBOX", "1")
        .envs(env.iter().copied())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

struct Fixture {
    root: PathBuf,
    task: Task,
    prepared: arms::Prepared,
    setup: Setup,
    trace: logs::trace::Context,
    signal: PathBuf,
}

impl Fixture {
    fn new(body: &str) -> Self {
        let root = tempfile::tempdir_in(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap())
            .unwrap()
            .keep();
        fs::create_dir(root.join("bin")).unwrap();
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        let git = crate::git::Git::new(&source);
        git.run(&["init", "--template="]).unwrap();
        git.run(&["config", "user.name", "Eval"]).unwrap();
        git.run(&["config", "user.email", "eval@example.invalid"])
            .unwrap();
        fs::write(source.join("CLAUDE.md"), "Review the change.\n").unwrap();
        git.run(&["add", "."]).unwrap();
        git.run(&["commit", "-m", "Base"]).unwrap();
        let base = git.run(&["rev-parse", "HEAD"]).unwrap();
        fs::write(source.join("change.txt"), "Reviewed change\n").unwrap();
        git.run(&["add", "."]).unwrap();
        git.run(&["commit", "-m", "Change"]).unwrap();
        let change = git.run(&["rev-parse", "HEAD"]).unwrap();
        let task = Task {
            suite: "code-review".into(), name: "fixture".into(), directory: root.clone(),
            manifest: serde_json::from_value(json!({
                "format": 1, "state": "approved", "session": "code-review",
                "repository": "source", "base": base, "change": change,
                "criteria": "Review the change", "checks": [["sh", "-c", "test -f change.txt"]], "inputs": {},
                "trial": {"issue_url": "https://github.com/acme/widgets/issues/7", "base": "base", "branch": "issue-7", "pr_url": "https://github.com/acme/widgets/pull/8"}
            })).unwrap(),
        };
        let binary = root.join("arm");
        executable(
            &binary,
            &format!(
                "#!/usr/bin/env python3\nimport json, os, pathlib, signal, sys, time\nrequest = json.load(open(sys.argv[2]))\nrepository = pathlib.Path(request['repository'])\noutput = pathlib.Path(request['output_dir'])\noutput.mkdir()\n{body}\n"
            ),
        );
        let harness = root.join("bin/opencode");
        executable(&harness, "#!/bin/sh\nexit 99\n");
        let arm = json!({"commit": base, "binary": binary, "harness": {"name": "opencode", "path": harness, "version": "fixture"}});
        let prepared =
            serde_json::from_value(json!({"format": 1, "baseline": arm, "candidate": arm}))
                .unwrap();
        Self {
            signal: root.join("plan-limit"),
            root,
            task,
            prepared,
            setup: Setup {
                commit: base,
                harness: "opencode".into(),
                model: None,
                effort: None,
            },
            trace: logs::trace::Context {
                trace: crate::logs::trace::id(&[b"execution"]),
                command: crate::logs::trace::id(&[b"command"]),
                run: crate::logs::trace::id(&[b"run"]),
                stage: crate::logs::trace::id(&[b"trials"]),
                identity: "execution-fixture".into(),
                repository: "acme/widgets".into(),
            },
        }
    }

    fn execution(&self, timeout: u64) -> Execution<'_> {
        Execution::prepare(
            &self.root,
            &self.prepared,
            [&self.setup, &self.setup],
            Duration::from_secs(timeout),
            &self.signal,
        )
        .unwrap()
    }

    fn request(&self) -> Request<'_> {
        Request {
            task: &self.task,
            identity: trial_results::Identity {
                suite: &self.task.suite,
                task: &self.task.name,
                arm: Arm::Candidate,
                trial: 0,
            },
            attempt: 2,
            trace: &self.trace,
        }
    }
}

const SUCCESS: &str = r#"
assert (repository / 'AGENTS.md').read_text() == 'Review the change.\n'
report = output / 'standards.md'
report.write_text('Review complete.\n')
print(json.dumps(dict(version=1, outcome='succeeded', timings=dict(total_ms=12, session_ms=10), reports=dict(standards=str(report), spec=None), void_cause=None, cause=None, session_id=None, log=None, model_evidence=None)))
"#;

#[test]
fn trial_installs_its_historical_rust_pin_before_the_session() {
    if isolated(&[]) {
        return;
    }
    let bin = PathBuf::from(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap()).join("bin");
    executable(
        &bin.join("rustup"),
        r#"#!/usr/bin/env python3
import os, pathlib, sys, tomllib
root = pathlib.Path(os.environ['THIRDSHIFT_EXECUTION_FIXTURE'])
args = sys.argv[1:]
if pathlib.Path(sys.argv[0]).name == 'cargo':
    if not (root / 'installed').exists() or not os.environ.get('RUSTUP_HOME'):
        print('pinned Rust 1.99.0 download refused by Trial proxy', file=sys.stderr)
        sys.exit(1)
    print('cargo 1.99.0 (fixture)')
elif args[:2] == ['toolchain', 'install']:
    pin = tomllib.loads(pathlib.Path('rust-toolchain.toml').read_text())['toolchain']
    assert pin['channel'] == '1.99.0', pin
    assert pin['components'] == ['rustfmt', 'clippy'], pin
    (root / 'installed').write_text('1.99.0')
    with (root / 'installs').open('a') as log: log.write('install\n')
else:
    raise AssertionError(args)
"#,
    );
    executable(
        &bin.join("cargo"),
        r#"#!/usr/bin/env python3
import os, pathlib, sys
root = pathlib.Path(os.environ['THIRDSHIFT_EXECUTION_FIXTURE'])
if not (root / 'installed').exists() or not os.environ.get('RUSTUP_HOME'):
    print('pinned Rust 1.99.0 download refused by Trial proxy', file=sys.stderr)
    sys.exit(1)
print('cargo 1.99.0 (fixture)')
"#,
    );
    let default_cargo = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cargo/bin");
    fs::create_dir_all(&default_cargo).unwrap();
    fs::copy(bin.join("cargo"), default_cargo.join("cargo")).unwrap();
    let mut fixture = Fixture::new(&format!(
        r#"
import shutil, subprocess
# Preserve historical Arms' root ordering and duplicate-skipping PATH discovery.
paths = []
for root in [pathlib.Path(os.environ['CARGO_HOME']), pathlib.Path(os.environ['HOME']) / '.cargo']:
    directory = str(root / 'bin')
    if pathlib.Path(directory).is_dir() and directory not in paths:
        paths.insert(0, directory)
directory = str(pathlib.Path(shutil.which('cargo')).resolve().parent)
if directory not in paths:
    paths.insert(0, directory)
env = dict(PATH=os.pathsep.join(paths + ['/usr/bin', '/bin']), HOME=str(output / 'home'), THIRDSHIFT_EXECUTION_FIXTURE=os.environ['THIRDSHIFT_EXECUTION_FIXTURE'])
check = subprocess.run(['cargo', '--version'], cwd=repository, env=env, capture_output=True, text=True)
assert check.returncode == 0, check.stderr
assert check.stdout == 'cargo 1.99.0 (fixture)\n'
{SUCCESS}
"#
    ));
    let source = fixture.root.join("source");
    fs::write(
        source.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.99.0\"\ncomponents = [\"rustfmt\", \"clippy\"]\n",
    )
    .unwrap();
    let git = crate::git::Git::new(&source);
    git.run(&["add", "."]).unwrap();
    git.run(&["commit", "-m", "Historical Rust pin"]).unwrap();
    fixture.task.manifest.change = git.run(&["rev-parse", "HEAD"]).unwrap();
    let execution = fixture.execution(5);
    std::thread::scope(|scope| {
        for arm in [Arm::Baseline, Arm::Candidate] {
            let fixture = &fixture;
            let execution = &execution;
            scope.spawn(move || {
                let mut request = fixture.request();
                request.identity.arm = arm;
                execution
                    .run(request, |_, output, _| {
                        assert!(output.outcome == Outcome::Succeeded, "{:?}", output.cause);
                        Ok(())
                    })
                    .unwrap();
            });
        }
    });
    let root = PathBuf::from(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap());
    assert_eq!(
        fs::read_to_string(root.join("installs")).unwrap(),
        "install\n"
    );
}

#[test]
fn rust_install_failure_voids_the_trial_without_launching_a_session() {
    if isolated(&[]) {
        return;
    }
    let root = PathBuf::from(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap());
    executable(
        &root.join("bin/rustup"),
        "#!/bin/sh\ntest \"$(cat rust-toolchain)\" = 1.99.0 || exit 99\necho 'fixture Rust download unavailable' >&2\nexit 1\n",
    );
    let mut fixture = Fixture::new("raise AssertionError('session must not start')");
    fs::write(fixture.root.join("pin"), "1.99.0\n").unwrap();
    fixture
        .task
        .manifest
        .inputs
        .insert("rust-toolchain".into(), "pin".into());
    fixture
        .execution(5)
        .run(fixture.request(), |attempt, output, score| {
            assert!(output.outcome == Outcome::Void);
            assert_eq!(output.void_cause, Some(VoidCause::SessionUnavailable));
            assert!(
                output
                    .cause
                    .unwrap()
                    .contains("fixture Rust download unavailable")
            );
            assert_eq!(output.timings.session_ms, 0);
            assert_eq!(score.checks.len(), 2, "Void Trials must omit code checks");
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            assert!(!attempt.join("session").exists());
            Ok(())
        })
        .unwrap();
}

#[test]
fn ended_muse_trial_drops_model_catalog_cache() {
    if isolated(&[]) {
        return;
    }
    let mut fixture = Fixture::new(
        r#"
catalog = output / 'home/.local/share/muse/model-catalog'
catalog.mkdir(parents=True)
(catalog / 'models.json').write_text('{"rows": []}')
native = output / 'home/.local/share/muse/sessions/retained/session.jsonl'
native.parent.mkdir(parents=True)
native.write_text('Native transcript\n')
report = output / 'standards.md'
report.write_text('Review complete.\n')
print(json.dumps(dict(version=1, outcome='succeeded', timings=dict(total_ms=12, session_ms=10), reports=dict(standards=str(report), spec=None), void_cause=None, cause=None, session_id=None, log=None, model_evidence=None)))
"#,
    );
    fixture.setup.harness = "muse".into();
    fixture
        .execution(5)
        .run(fixture.request(), |attempt, _, _| {
            assert_eq!(
                fs::read_to_string(
                    attempt.join("session/home/.local/share/muse/sessions/retained/session.jsonl")
                )
                .unwrap(),
                "Native transcript\n"
            );
            assert!(
                !attempt
                    .join("session/home/.local/share/muse/model-catalog")
                    .exists(),
                "ended Muse Trial retained its model catalog cache"
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn ended_trial_sessions_drop_caches_and_keep_native_records_for_every_outcome() {
    if isolated(&[]) {
        return;
    }
    for ending in [
        SUCCESS.to_owned(),
        SUCCESS.replace("outcome='succeeded'", "outcome='failed'"),
        SUCCESS.replace("outcome='succeeded'", "outcome='void'"),
        "sys.exit(23)".into(),
        "print('invalid protocol')".into(),
        "time.sleep(30)".into(),
        "os.kill(os.getppid(), signal.SIGTERM); time.sleep(30)".into(),
    ] {
        let fixture = Fixture::new(&format!(
            r#"
for cache in ['.codex/cache/remote_plugin_catalog', '.cache/opencode']:
    path = output / 'home' / cache
    path.mkdir(parents=True)
    (path / 'bulk').write_text('Disposable cache')
native = output / 'home/.codex/sessions/retained.jsonl'
native.parent.mkdir(parents=True)
native.write_text('Native transcript\n')
(output / 'retained.models.json').write_text('Model evidence\n')
{ending}
"#
        ));
        let result = fixture
            .execution(1)
            .run(fixture.request(), |attempt, _, _| {
                assert!(!attempt.join("session/home/.codex/cache").exists());
                assert!(!attempt.join("session/home/.cache").exists());
                Ok(())
            });
        if ending.contains("sys.exit")
            || ending.contains("invalid protocol")
            || ending.contains("os.kill")
        {
            assert!(result.is_err());
        } else {
            result.unwrap();
        }
        let trial = fixture
            .root
            .join("trials")
            .join(fixture.request().identity.key());
        let attempt = fs::read_dir(trial).unwrap().next().unwrap().unwrap().path();
        assert!(!attempt.join("session/home/.codex/cache").exists());
        assert!(!attempt.join("session/home/.cache").exists());
        assert_eq!(
            fs::read_to_string(attempt.join("session/home/.codex/sessions/retained.jsonl"))
                .unwrap(),
            "Native transcript\n"
        );
        assert_eq!(
            fs::read_to_string(attempt.join("session/retained.models.json")).unwrap(),
            "Model evidence\n"
        );
    }
}

const GIT: &str = r#"
import subprocess
def git(*args):
    return subprocess.check_output(['git', '-C', str(repository), *args], text=True).strip()
"#;

#[test]
fn review_commits_and_repository_reports_are_graded_for_both_sessions() {
    if isolated(&[]) {
        return;
    }
    for session in ["spec-review", "code-review"] {
        let mut fixture = Fixture::new(&format!(
            r#"
{GIT}
(repository / 'change.txt').write_text('Fixed by review\n')
git('add', 'change.txt')
git('-c', 'user.name=Review', '-c', 'user.email=review@example.invalid', 'commit', '-m', 'Review fix')
if request['session_kind'] == 'spec-review':
    git('push', 'origin', 'HEAD')
output = repository / '.thirdshift-review'
output.mkdir()
(repository / '.gitignore').write_text('ignored-output\n')
(repository / 'ignored-output').write_text('Session artifact')
spec = output / 'spec.md'
spec.write_text('Spec review complete.\n')
{success}
"#,
            success = SUCCESS.replace("spec=None", "spec=str(spec)")
        ));
        fixture.task.manifest.session = serde_json::from_value(json!(session)).unwrap();
        fixture.task.manifest.checks.clear();
        fixture
            .execution(5)
            .run(fixture.request(), |_, output, score| {
                assert!(
                    output.outcome == Outcome::Succeeded,
                    "{session}: {:?}",
                    output.cause
                );
                assert!(score.passed, "{session}");
                assert_eq!(
                    output.reports.read_and_blind()?.standards.as_deref(),
                    Some("Review complete.\n")
                );
                assert_eq!(
                    output.reports.read_and_blind()?.spec.as_deref(),
                    Some("Spec review complete.\n")
                );
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn session_edits_cannot_pass_checks_on_the_prepared_repository() {
    if isolated(&[]) {
        return;
    }
    let mut fixture = Fixture::new(&format!(
        r#"
(repository / 'change.txt').write_text('Fixed by review\n')
(repository / 'session-artifact').write_text('Untracked file')
{SUCCESS}
"#
    ));
    fixture.task.manifest.checks = vec![
        vec!["sh".into(), "-c".into(), "grep -q 'Fixed by review' change.txt".into()],
        vec!["sh".into(), "-c".into(), "grep -qx 'Reviewed change' change.txt && test ! -e session-artifact && grep -q 'Review complete' \"$THIRDSHIFT_TRIAL_STANDARDS_REPORT\"".into()],
    ];
    fixture
        .execution(5)
        .run(fixture.request(), |_, output, score| {
            assert!(output.outcome == Outcome::Succeeded);
            assert!(!score.passed);
            assert!(!score.checks[2].passed, "session edits passed a code check");
            assert!(
                score.checks[3].passed,
                "checks must see prepared code and the session's report"
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn session_cannot_pass_checks_by_editing_the_grader_snapshot() {
    if isolated(&[]) {
        return;
    }
    let mut fixture = Fixture::new(&format!(
        r#"
import tempfile
for path in pathlib.Path(tempfile.gettempdir()).glob('prepared-grader-*/repository/change.txt'):
    path.write_text('Fixed by review\n')
{SUCCESS}
"#
    ));
    let tmp = fixture.root.join("grader-tmp");
    fs::create_dir(&tmp).unwrap();
    // SAFETY: the test process is single-threaded until the Arm is spawned,
    // and this env is the snapshot directory both sides must share.
    unsafe { std::env::set_var("TMPDIR", &tmp) };
    fixture.task.manifest.checks = vec![vec![
        "sh".into(),
        "-c".into(),
        "grep -qx 'Fixed by review' change.txt".into(),
    ]];
    fixture
        .execution(5)
        .run(fixture.request(), |_, output, score| {
            assert!(
                !score.passed,
                "session made mechanical checks pass by editing the grader snapshot ({})",
                output.outcome
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn input_symlink_to_a_fifo_voids_without_blocking_the_supervisor() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new(&format!(
        r#"
import os
os.mkfifo(repository / 'block.fifo')
(repository / '.thirdshift-issue.md').symlink_to('block.fifo')
{SUCCESS}
"#
    ));
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = fixture
            .execution(5)
            .run(fixture.request(), |_, output, score| {
                assert_eq!(output.void_cause, Some(VoidCause::RepositoryChanged));
                assert!(!score.passed);
                Ok(())
            });
        sender.send(result).ok();
    });
    match receiver.recv_timeout(Duration::from_secs(20)) {
        Ok(result) => result.expect("Trial execution failed"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            eprintln!("integrity check blocked on an input symlink to a FIFO");
            std::process::exit(1);
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("Trial thread ended without a result");
        }
    }
}

#[test]
fn trial_preparation_ignores_caller_checkout_and_push_hooks() {
    if isolated(&[]) {
        return;
    }
    let mut fixture = Fixture::new(SUCCESS);
    fixture.task.manifest.session = crate::evals::format::Session::SpecReview;
    let hooks = fixture.root.join("hooks");
    fs::create_dir(&hooks).unwrap();
    for hook in ["post-checkout", "pre-push"] {
        executable(&hooks.join(hook), "#!/bin/sh\ntouch \"$0.called\"\n");
    }
    let config = fixture.root.join("caller.gitconfig");
    fs::write(
        &config,
        format!("[core]\n hooksPath = {}\n", hooks.display()),
    )
    .unwrap();
    // Install after source construction: isolated() disables global config.
    unsafe { std::env::set_var("GIT_CONFIG_GLOBAL", &config) };
    fixture
        .execution(5)
        .run(fixture.request(), |_, output, _| {
            assert!(output.outcome == Outcome::Succeeded);
            Ok(())
        })
        .unwrap();
    for hook in ["post-checkout", "pre-push"] {
        assert!(!hooks.join(format!("{hook}.called")).exists(), "{hook} ran");
    }
}

#[test]
fn preparation_failure_publishes_no_repository_and_launches_no_arm() {
    if isolated(&[]) {
        return;
    }
    let mut fixture = Fixture::new("(repository.parent / 'arm-started').touch()");
    fixture.task.manifest.trial.as_mut().unwrap().branch = "base/issue-7".into();
    let result: Result<()> = fixture.execution(5).run(fixture.request(), |_, _, _| {
        panic!("preparation failure reached completion")
    });
    assert!(result.is_err());
    assert!(!fixture.root.join("started").exists());
    let trial = fixture
        .root
        .join("trials")
        .join(fixture.request().identity.key());
    let attempts: Vec<_> = fs::read_dir(trial).unwrap().map(Result::unwrap).collect();
    assert_eq!(attempts.len(), 1);
    assert!(!attempts[0].path().join("repository").exists());
    assert!(!attempts[0].path().join("arm-started").exists());
}

#[test]
fn completed_score_and_evidence_live_through_publication() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new(SUCCESS);
    let execution = fixture.execution(5);
    let attempt = execution
        .run(fixture.request(), |attempt, output, score| {
            assert!(output.outcome == Outcome::Succeeded);
            assert!(score.passed);
            assert_eq!(score.checks.len(), 3);
            assert!(score.model.is_none());
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            assert!(!attempt.join("trace.spans.jsonl").exists());
            let request: Value =
                serde_json::from_slice(&fs::read(attempt.join("request.json"))?).unwrap();
            assert_eq!(request["version"], 1);
            let started: Value =
                serde_json::from_slice(&fs::read(attempt.join("started.json"))?).unwrap();
            assert_eq!(started["index"], 2);
            store::write(&attempt.join("result.json"), &output)?;
            store::write(&fixture.root.join("completed-score.json"), &score)?;
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            Ok(attempt)
        })
        .unwrap();
    assert!(!attempt.parent().unwrap().join("active.json").exists());
    assert!(fixture.root.join("started").is_file());
    for artifact in [
        "request.json",
        "stdout",
        "stderr",
        "scope",
        "started.json",
        "trace-context.json",
        "trace.logs.jsonl",
        "trace.spans.jsonl",
        "result.json",
    ] {
        assert!(attempt.join(artifact).is_file(), "missing {artifact}");
    }
    let spans = fs::read_to_string(attempt.join("trace.spans.jsonl")).unwrap();
    assert!(spans.contains("Grader"));
    assert!(spans.contains("succeeded"));
}

#[test]
fn edited_or_deleted_task_inputs_void_an_otherwise_successful_execution() {
    if isolated(&[]) {
        return;
    }
    for input in [crate::prompt::ISSUE_FILE, "context/input.md", "CLAUDE.md"] {
        for mutation in ["write_text('Edited input')", "unlink()"] {
            let mut fixture =
                Fixture::new(&format!("{SUCCESS}\n(repository / '{input}').{mutation}"));
            fs::write(fixture.root.join("input.md"), "Review the change.\n").unwrap();
            fixture
                .task
                .manifest
                .inputs
                .insert(input.into(), "input.md".into());
            fixture
                .execution(5)
                .run(fixture.request(), |attempt, output, score| {
                    assert!(output.outcome == Outcome::Void, "{input}: {mutation}");
                    assert_eq!(output.void_cause, Some(VoidCause::RepositoryChanged));
                    assert!(!score.passed);
                    assert_eq!(score.checks.len(), 2, "Void Trials skip code checks");
                    assert!(attempt.parent().unwrap().join("active.json").is_file());
                    Ok(())
                })
                .unwrap();
        }
    }
}

#[test]
fn changes_to_reviewed_commits_void_an_otherwise_successful_execution() {
    if isolated(&[]) {
        return;
    }
    for mutation in [
        "git('update-ref', 'refs/heads/base', 'HEAD')",
        "git('replace', 'HEAD', 'base')",
        "git('replace', 'base', 'HEAD')",
        "(repository / '.git/info/grafts').write_text(git('rev-parse', 'HEAD') + '\\n')",
        "(repository / '.git/shallow').write_text(git('rev-parse', 'HEAD') + '\\n')",
        "oid = git('rev-parse', 'HEAD'); (repository / '.git/objects' / oid[:2] / oid[2:]).unlink()",
    ] {
        let fixture = Fixture::new(&format!(
            r#"
{GIT}
{SUCCESS}
{mutation}
"#
        ));
        fixture
            .execution(5)
            .run(fixture.request(), |_, output, score| {
                assert!(output.outcome == Outcome::Void, "{mutation}");
                assert_eq!(output.void_cause, Some(VoidCause::RepositoryChanged));
                assert!(!score.passed);
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn issue_symlink_target_edits_are_repository_changes() {
    if isolated(&[]) {
        return;
    }
    let mut fixture = Fixture::new(&format!(
        "{SUCCESS}\n(repository / 'CLAUDE.md').write_text('Altered issue text')"
    ));
    fixture.task.manifest.checks.clear();
    let source = fixture.root.join("source");
    std::os::unix::fs::symlink("CLAUDE.md", source.join(crate::prompt::ISSUE_FILE)).unwrap();
    let git = crate::git::Git::new(&source);
    git.run(&["add", crate::prompt::ISSUE_FILE]).unwrap();
    git.run(&["commit", "-m", "Issue input symlink"]).unwrap();
    fixture.task.manifest.change = git.run(&["rev-parse", "HEAD"]).unwrap();
    fixture
        .execution(5)
        .run(fixture.request(), |_, output, _| {
            assert_eq!(
                output.void_cause,
                Some(VoidCause::RepositoryChanged),
                "Editing the text read from .thirdshift-issue.md must void the Trial"
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn timeout_preserves_failure_after_mutation_and_stops_descendants() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new(
        r#"
import subprocess
(repository / 'changed.txt').write_text('Changed')
escaped = output.parent / 'escaped'
subprocess.Popen([sys.executable, '-c', "import pathlib,sys,time; time.sleep(2); pathlib.Path(sys.argv[1]).touch()", str(escaped)])
time.sleep(20)
"#,
    );
    let attempt = fixture
        .execution(1)
        .run(fixture.request(), |attempt, output, score| {
            assert!(output.outcome == Outcome::Failed);
            assert_eq!(output.void_cause, None);
            assert_eq!(output.cause.as_deref(), Some("Trial timed out"));
            assert_eq!(output.timings.total_ms, 1000);
            assert!(!score.passed);
            Ok(attempt)
        })
        .unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        !attempt.join("escaped").exists(),
        "a timed out descendant kept working"
    );
    assert!(attempt.join("trace.spans.jsonl").is_file());
    assert!(!attempt.parent().unwrap().join("active.json").exists());
}

#[test]
fn callback_errors_propagate_after_trace_and_active_evidence_cleanup() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new(SUCCESS);
    let mut retained = None;
    let error = fixture
        .execution(5)
        .run(fixture.request(), |attempt, _, _| -> Result<()> {
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            retained = Some(attempt);
            Err(std::io::Error::from_raw_os_error(libc::ENOSPC).into())
        })
        .unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(libc::ENOSPC)
    );
    assert_eq!(
        error.to_string(),
        std::io::Error::from_raw_os_error(libc::ENOSPC).to_string()
    );
    let attempt = retained.unwrap();
    assert!(!attempt.parent().unwrap().join("active.json").exists());
    assert!(attempt.join("trace.spans.jsonl").is_file());
    assert!(!trial_results::finished(
        &fixture.root,
        &fixture.request().identity
    ));
}

#[test]
fn older_arms_stop_at_native_limits_in_fresh_homes_from_raw_and_trace_streams() {
    if isolated(&[("THIRDSHIFT_TRIAL_NO_SANDBOX", "")]) {
        return;
    }
    for session in ["code-review", "spec-review"] {
        for trace in [false, true] {
            let mut fixture = Fixture::new(&format!(
                r#"
import datetime
native = output / 'home/.codex/sessions/2026/10/24/rollout-fake-thread-1.jsonl'
native.parent.mkdir(parents=True)
event = json.dumps(dict(type='thread.started', thread_id='fake-thread-1'))
suffix = 'jsonl'
if {trace:?} == 'true':
    event = json.dumps(dict(resourceLogs=[dict(scopeLogs=[dict(logRecords=[dict(body=dict(stringValue=event))])])]))
    suffix = 'logs.jsonl'
    span = dict(traceId='11' * 16, spanId='22' * 8, name='invoke_agent codex', startTimeUnixNano='1', endTimeUnixNano='2')
    (output / 'trial-{session}.spans.jsonl').write_text(json.dumps(dict(resourceSpans=[dict(scopeSpans=[dict(spans=[span])])])) + '\n')
(output / ('trial-{session}.' + suffix)).write_text(event + '\n')
native.write_text(json.dumps(dict(type='session_meta', payload=dict(id='fake-thread-1', cwd=request['repository']))) + '\n')
time.sleep(0.2)
with native.open('a') as records:
    records.write(json.dumps(dict(timestamp=datetime.datetime.now(datetime.timezone.utc).isoformat(), type='event_msg', payload=dict(type='token_count', rate_limits=dict(secondary=dict(used_percent=85))))) + '\n')
time.sleep(2)
(output.parent / 'continued').touch()
sys.exit(99)
"#,
                trace = trace.to_string()
            ));
            fixture.setup.harness = "codex".into();
            fixture.prepared.baseline.harness.name = "codex".into();
            fixture.prepared.candidate.harness.name = "codex".into();
            if session == "spec-review" {
                fixture.task.manifest.session = crate::evals::format::Session::SpecReview;
            }
            let attempt = fixture
                .execution(5)
                .run(fixture.request(), |attempt, output, score| {
                    assert!(output.outcome == Outcome::Void);
                    assert_eq!(output.void_cause, Some(VoidCause::UsageLimit));
                    assert_eq!(output.session_id.as_deref(), Some("fake-thread-1"));
                    let suffix = if trace { "logs.jsonl" } else { "jsonl" };
                    assert!(
                        output
                            .log
                            .unwrap()
                            .ends_with(format!("trial-{session}.{suffix}"))
                    );
                    assert!(!score.passed);
                    assert!(fixture.signal.is_file());
                    Ok(attempt)
                })
                .unwrap();
            assert!(!attempt.join("continued").exists());
            assert!(attempt.join("trace.spans.jsonl").is_file());
            assert!(!attempt.parent().unwrap().join("active.json").exists());
            if trace {
                let retained: Value = serde_json::from_slice(
                    &fs::read(attempt.join(format!("session/trial-{session}.spans.jsonl")))
                        .unwrap(),
                )
                .unwrap();
                let span = &retained["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
                assert_eq!(
                    span["traceId"],
                    serde_json::to_value(fixture.trace.trace).unwrap()
                );
                let context: logs::trace::Context =
                    serde_json::from_slice(&fs::read(attempt.join("trace-context.json")).unwrap())
                        .unwrap();
                assert_eq!(
                    span["parentSpanId"],
                    serde_json::to_value(context.stage).unwrap()
                );
            }
        }
    }
}

#[test]
fn interruption_never_reaches_completion_or_creates_a_checkpoint() {
    if isolated(&[]) {
        return;
    }
    let fixture = Fixture::new("os.kill(os.getppid(), signal.SIGTERM)\ntime.sleep(20)");
    let mut called = false;
    let error = fixture
        .execution(5)
        .run(fixture.request(), |_, _, _| {
            called = true;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "interrupted");
    assert!(!called);
    let attempt =
        super::super::trial_sessions::experiment(&trial_results::files::Native, &fixture.root)
            .unwrap()
            .pop()
            .unwrap()
            .path;
    assert!(!attempt.parent().unwrap().join("active.json").exists());
    assert!(!attempt.parent().unwrap().join("pending.json").exists());
    assert!(!attempt.join("result.json").exists());
    assert!(attempt.join("trace.spans.jsonl").is_file());
}

#[test]
fn unavailable_code_grader_is_void_and_rescored_before_completion() {
    if isolated(&[("THIRDSHIFT_TRIAL_NO_SANDBOX", "")]) {
        return;
    }
    let bin = PathBuf::from(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap()).join("bin");
    executable(
        &bin.join("bwrap"),
        "#!/bin/sh\necho 'fixture sandbox unavailable' >&2\nexit 1\n",
    );
    let fixture = Fixture::new(SUCCESS);
    fixture
        .execution(5)
        .run(fixture.request(), |attempt, output, score| {
            assert!(output.outcome == Outcome::Void);
            assert_eq!(output.void_cause, Some(VoidCause::SandboxUnavailable));
            assert!(!score.passed);
            assert_eq!(
                score.checks.len(),
                2,
                "the rescored Void must omit code checks"
            );
            assert!(attempt.parent().unwrap().join("active.json").is_file());
            Ok(())
        })
        .unwrap();
}

fn protocol(outcome: &str, cause: Option<&str>) -> String {
    let output = json!({
        "version": 1, "outcome": outcome, "void_cause": cause,
        "timings": {"total_ms": 12, "session_ms": 10},
        "reports": {"standards": null, "spec": null}
    });
    format!("print({:?})", output.to_string())
}

#[test]
fn scope_cap_evidence_preserves_explicit_voids_and_unexplained_kills_fail() {
    if isolated(&[]) {
        return;
    }
    for (body, cap, outcome, cause) in [
        (
            "os.kill(os.getpid(), signal.SIGKILL)".into(),
            false,
            Outcome::Failed,
            None,
        ),
        ("sys.exit(137)".into(), false, Outcome::Failed, None),
        (
            protocol("failed", None),
            true,
            Outcome::Void,
            Some(VoidCause::MemoryCap),
        ),
        (
            "sys.exit(9)".into(),
            true,
            Outcome::Void,
            Some(VoidCause::MemoryCap),
        ),
        (
            "print('malformed')".into(),
            true,
            Outcome::Void,
            Some(VoidCause::MemoryCap),
        ),
        (
            protocol("void", Some("usage_limit")),
            true,
            Outcome::Void,
            Some(VoidCause::UsageLimit),
        ),
        (protocol("succeeded", None), true, Outcome::Succeeded, None),
    ] {
        let journal = PathBuf::from(std::env::var_os("THIRDSHIFT_EXECUTION_FIXTURE").unwrap())
            .join("bin/journalctl");
        executable(
            &journal,
            if cap {
                "#!/bin/sh\necho \"Failed with result 'oom-kill'\"\n"
            } else {
                "#!/bin/sh\nexit 0\n"
            },
        );
        let fixture = Fixture::new(&body);
        fixture
            .execution(5)
            .run(fixture.request(), |_, output, score| {
                assert!(output.outcome == outcome, "{body}: {}", output.outcome);
                assert_eq!(output.void_cause, cause, "{body}");
                assert_eq!(score.checks[0].passed, outcome == Outcome::Succeeded);
                if cause == Some(VoidCause::MemoryCap) {
                    assert_eq!(output.cause.as_deref(), Some(crate::scope::OOM_KILL_CAUSE));
                }
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn operational_failures_never_publish_completed_values() {
    if isolated(&[]) {
        return;
    }
    for (body, message) in [
        ("sys.exit(9)", "Arm Trial protocol failed"),
        ("print('malformed')", "invalid Trial result"),
        (
            "print('{\"version\":2,\"outcome\":\"failed\",\"timings\":{\"total_ms\":1,\"session_ms\":1},\"reports\":{\"standards\":null,\"spec\":null}}')",
            "unsupported Trial result",
        ),
    ] {
        let fixture = Fixture::new(body);
        let execution = fixture.execution(5);
        let mut called = false;
        let error = execution
            .run(fixture.request(), |_, _, _| {
                called = true;
                Ok(())
            })
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error:#}");
        assert!(!called);
        let trial = fixture
            .root
            .join("trials")
            .join(fixture.request().identity.key());
        assert!(!trial.join("pending.json").exists());
        assert!(!trial.join("active.json").exists());
        assert!(!trial_results::finished(
            &fixture.root,
            &fixture.request().identity
        ));
        let attempts =
            super::super::trial_sessions::experiment(&trial_results::files::Native, &fixture.root)
                .unwrap();
        assert_eq!(attempts.len(), 1);
        assert!(attempts[0].path.join("stdout").is_file());
        assert!(attempts[0].path.join("trace.spans.jsonl").is_file());
        assert!(!attempts[0].path.join("result.json").exists());
    }
}
