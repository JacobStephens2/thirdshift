//! Git behavior in isolated Commands, so recorded interruption cannot leak
//! into another test running in the same process.

use super::*;
use crate::interrupt;
use crate::test_support::with_recorded_signal;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::mpsc;

/// Observe whether acquisition finishes while an independent open still
/// holds the lock. Release and join before the caller can assert, even when
/// acquisition ignores interruption or returns unexpectedly early.
fn contended_repository_lock(git: &Git, signal: Option<libc::c_int>) -> (bool, Result<File>) {
    let path = git.common_dir().unwrap().join("thirdshift-test.lock");
    let held = File::create(path).unwrap();
    held.lock().unwrap();
    std::thread::scope(|scope| {
        let (finished, received) = mpsc::channel();
        let contender = scope.spawn(move || {
            let result = git.lock("thirdshift-test.lock");
            let _ = finished.send(());
            result
        });
        std::thread::sleep(Duration::from_millis(200));
        let delivered = signal.map(signal_hook::low_level::raise);
        let finished_while_held = received.recv_timeout(Duration::from_secs(1)).is_ok();
        drop(held);
        let result = contender.join().unwrap();
        if let Some(delivered) = delivered {
            delivered.unwrap();
        }
        (finished_while_held, result)
    })
}

#[test]
fn ordinary_repository_lock_contention_stops_before_the_holder_releases() {
    with_recorded_signal(
        "git::execution_tests::ordinary_repository_lock_contention_stops_before_the_holder_releases",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            let (finished_while_held, result) = contended_repository_lock(&git, Some(signal));

            assert!(finished_while_held, "acquisition waited for holder release");
            assert_eq!(result.unwrap_err().to_string(), "interrupted");
            assert!(interrupt::requested());
            let independent = File::open(git.dir().join(".git/thirdshift-test.lock")).unwrap();
            independent.try_lock().unwrap();
        },
    );
}

#[test]
fn completion_repository_lock_waits_through_arriving_and_recorded_interruption() {
    with_recorded_signal(
        "git::execution_tests::completion_repository_lock_waits_through_arriving_and_recorded_interruption",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            let completion = git.completion();
            let path = git.common_dir().unwrap().join("thirdshift-test.lock");
            // First deliver a new request during contention, then acquire
            // again with that request already recorded.
            for signal in [Some(signal), None] {
                let (finished_while_held, result) = contended_repository_lock(&completion, signal);

                assert!(!finished_while_held, "completion did not wait for release");
                let acquired = result.unwrap();
                let independent = File::open(&path).unwrap();
                assert!(matches!(
                    independent.try_lock(),
                    Err(TryLockError::WouldBlock)
                ));
                assert!(interrupt::requested());
                assert_eq!(
                    git.lock("ordinary.lock").unwrap_err().to_string(),
                    "interrupted"
                );
                assert_eq!(
                    Git::new(git.dir())
                        .lock("ordinary.lock")
                        .unwrap_err()
                        .to_string(),
                    "interrupted"
                );
                assert_eq!(
                    git.run(&["rev-parse", "HEAD"]).unwrap_err().to_string(),
                    "interrupted"
                );
                assert!(!path.with_file_name("ordinary.lock").exists());
                drop(acquired);
                independent.try_lock().unwrap();
                assert!(path.exists(), "release removed the shared locking identity");
            }
        },
    );
}

#[test]
fn repository_lock_is_shared_by_worktrees_and_owned_until_drop() {
    let (temp, git) = super::tests::repo_with_origin();
    let worktree_path = temp.path().join("worktree");
    git.run(&[
        "worktree",
        "add",
        "--detach",
        worktree_path.to_str().unwrap(),
        "HEAD",
    ])
    .unwrap();
    let worktree = Git::new(worktree_path);
    let path = git.common_dir().unwrap().join("thirdshift-test.lock");
    let acquired = git.lock("thirdshift-test.lock").unwrap();
    let independent =
        File::open(worktree.common_dir().unwrap().join("thirdshift-test.lock")).unwrap();

    assert!(matches!(
        independent.try_lock(),
        Err(TryLockError::WouldBlock)
    ));
    drop(acquired);
    let acquired = worktree.lock("thirdshift-test.lock").unwrap();
    assert!(matches!(
        independent.try_lock(),
        Err(TryLockError::WouldBlock)
    ));
    drop(acquired);
    independent.try_lock().unwrap();
    assert!(path.exists(), "release removed the shared locking identity");
}

#[test]
fn ordinary_git_refuses_to_spawn_after_interruption() {
    with_recorded_signal(
        "git::execution_tests::ordinary_git_refuses_to_spawn_after_interruption",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            signal_hook::low_level::raise(signal).unwrap();
            let error = git
                .run(&["-c", "alias.start=!touch started", "start"])
                .unwrap_err();
            assert_eq!(error.to_string(), "interrupted");
            assert!(!git.dir().join("started").exists());
            assert_eq!(
                git.lock("ordinary.lock").unwrap_err().to_string(),
                "interrupted"
            );
            assert!(!git.dir().join(".git/ordinary.lock").exists());
        },
    );
}

#[test]
fn completion_git_finishes_without_permitting_the_original_view() {
    with_recorded_signal(
        "git::execution_tests::completion_git_finishes_without_permitting_the_original_view",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            let completion = git.completion();
            signal_hook::low_level::raise(signal).unwrap();
            completion
                .run(&["config", "thirdshift.finished", "yes"])
                .unwrap();
            assert_eq!(
                completion.run(&["config", "thirdshift.finished"]).unwrap(),
                "yes"
            );
            assert_eq!(completion.dir(), git.dir());
            assert!(interrupt::requested());
            assert_eq!(
                git.run(&["config", "thirdshift.finished"])
                    .unwrap_err()
                    .to_string(),
                "interrupted"
            );
            assert_eq!(
                Git::new(git.dir())
                    .succeeds(&["rev-parse", "HEAD"])
                    .unwrap_err()
                    .to_string(),
                "interrupted"
            );
        },
    );
}

#[test]
fn optional_answers_distinguish_absence_from_execution_failure() {
    with_recorded_signal(
        "git::execution_tests::optional_answers_distinguish_absence_from_execution_failure",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            assert_eq!(
                git.run_optional(&["symbolic-ref", "--short", "HEAD"])
                    .unwrap()
                    .as_deref(),
                Some("main")
            );
            assert_eq!(
                git.run_optional(&["rev-parse", "--verify", "--quiet", "refs/heads/absent"])
                    .unwrap(),
                None
            );
            assert!(
                !git.succeeds(&["rev-parse", "--verify", "--quiet", "refs/heads/absent"])
                    .unwrap()
            );
            assert!(git.succeeds(&["rev-parse", "HEAD"]).unwrap());
            let missing = Git::new(git.dir().join("absent-directory"));
            assert!(missing.run_optional(&["rev-parse", "HEAD"]).is_err());
            assert!(missing.succeeds(&["rev-parse", "HEAD"]).is_err());
            signal_hook::low_level::raise(signal).unwrap();
            assert_eq!(
                git.run_optional(&["rev-parse", "--verify", "--quiet", "refs/heads/absent"])
                    .unwrap_err()
                    .to_string(),
                "interrupted"
            );
            assert_eq!(
                git.succeeds(&["rev-parse", "--verify", "--quiet", "refs/heads/absent"])
                    .unwrap_err()
                    .to_string(),
                "interrupted"
            );
        },
    );
}

/// The executable is a real Git alias. Only its recorded PID is used for
/// fallback cleanup, including when an assertion fails.
struct Fixture {
    pid: PathBuf,
}

impl Fixture {
    fn new(git: &Git, script: &str) -> Self {
        let pid = git.dir().join("fixture.pid");
        let path = git.dir().join("fixture.sh");
        fs::write(
            &path,
            format!("#!/bin/sh\necho $$ > fixture.pid\n{script}\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        Self { pid }
    }

    fn pid(&self) -> Option<libc::pid_t> {
        fs::read_to_string(&self.pid).ok()?.trim().parse().ok()
    }
}

fn exists(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only probes the PID recorded by this fixture.
    unsafe { libc::kill(pid, 0) == 0 }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(pid) = self.pid()
            && exists(pid)
        {
            // SAFETY: this PID belongs to the test's finite executable.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

const FIXTURE: &[&str] = &["-c", "alias.fixture=!./fixture.sh", "fixture"];

#[test]
fn a_parent_only_signal_stops_active_git_and_propagates_through_probes() {
    with_recorded_signal(
        "git::execution_tests::a_parent_only_signal_stops_active_git_and_propagates_through_probes",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            let fixture = Fixture::new(
                &git,
                &format!("kill -{signal} {}\nexec sleep 3", std::process::id()),
            );
            let started = Instant::now();
            assert_eq!(
                git.succeeds(FIXTURE).unwrap_err().to_string(),
                "interrupted"
            );
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "Git waited for natural expiry"
            );
            let pid = fixture.pid().expect("the executable never started");
            let deadline = Instant::now() + Duration::from_secs(2);
            while exists(pid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!exists(pid), "Git left its owned executable alive");
            assert_eq!(
                git.run_optional(FIXTURE).unwrap_err().to_string(),
                "interrupted"
            );
            assert_eq!(git.run(FIXTURE).unwrap_err().to_string(), "interrupted");
        },
    );
}

#[test]
fn completion_git_finishes_with_a_newly_arriving_signal() {
    with_recorded_signal(
        "git::execution_tests::completion_git_finishes_with_a_newly_arriving_signal",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            let _fixture = Fixture::new(
                &git,
                &format!(
                    "kill -{signal} {}\nsleep 0.1\nprintf 'completed\\n'",
                    std::process::id()
                ),
            );
            assert_eq!(git.completion().run(FIXTURE).unwrap(), "completed");
            assert!(interrupt::requested());
            assert_eq!(git.run(FIXTURE).unwrap_err().to_string(), "interrupted");
        },
    );
}

#[test]
fn an_optional_probe_propagates_active_cancellation() {
    with_recorded_signal(
        "git::execution_tests::an_optional_probe_propagates_active_cancellation",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            let _fixture = Fixture::new(
                &git,
                &format!("kill -{signal} {}\nexec sleep 3", std::process::id()),
            );
            assert_eq!(
                git.run_optional(FIXTURE).unwrap_err().to_string(),
                "interrupted"
            );
        },
    );
}

fn interrupt_lock_retry(git: &Git, signal: libc::c_int) -> Result<String> {
    let held = git.dir().join("held");
    let tried = git.dir().join("tried");
    fs::write(&held, "").unwrap();
    let _fixture = Fixture::new(
        git,
        "if test -f held; then\n touch tried\n echo \"fatal: Unable to create 'config.lock': File exists\" >&2\n exit 1\nfi\nprintf 'finished\\n'",
    );
    let release = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !tried.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let attempted = tried.exists();
        std::thread::sleep(Duration::from_millis(50));
        signal_hook::low_level::raise(signal).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        fs::remove_file(held).unwrap();
        assert!(attempted, "Git never reached the held lock");
    });
    let result = git.run(FIXTURE);
    release.join().unwrap();
    result
}

#[test]
fn ordinary_lock_retries_observe_interruption() {
    with_recorded_signal(
        "git::execution_tests::ordinary_lock_retries_observe_interruption",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            assert_eq!(
                interrupt_lock_retry(&git, signal).unwrap_err().to_string(),
                "interrupted"
            );
        },
    );
}

#[test]
fn completion_lock_retries_ignore_existing_and_arriving_interruption() {
    with_recorded_signal(
        "git::execution_tests::completion_lock_retries_ignore_existing_and_arriving_interruption",
        |signal| {
            let (_temp, git) = super::tests::repo_with_origin();
            let completion = git.completion();
            assert_eq!(
                interrupt_lock_retry(&completion, signal).unwrap(),
                "finished"
            );
            // The same held-lock path also works with the flag already recorded.
            fs::remove_file(git.dir().join("tried")).unwrap();
            assert_eq!(
                interrupt_lock_retry(&completion, signal).unwrap(),
                "finished"
            );
            assert!(interrupt::requested());
            assert_eq!(git.run(FIXTURE).unwrap_err().to_string(), "interrupted");
        },
    );
}
