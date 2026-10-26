//! Shared unit-test helpers: run signal-sensitive tests in isolated
//! processes so interruption and fixture environment changes cannot leak into
//! another test, and write executables other tests can't hold open.

pub(crate) mod guidance;

mod executable;
pub(crate) use executable::write_executable;

use std::process::Command;

use crate::interrupt;

/// Git for unit-test fixtures, including commands made after repository setup.
/// Keep this on each child Command so parallel tests never change the process
/// environment and new Git views of a fixture inherit the same isolation.
pub(crate) fn git_command() -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    command
}

/// A child that announces Session s1 after writing a native Codex quota record.
/// The record may deliberately name another Session/directory or a stale time;
/// `current` writes a fresh timestamp. HOME is set only on the child Command.
pub(crate) fn native_codex_command(
    root: &std::path::Path,
    id: &str,
    directory: &std::path::Path,
    timestamp: &str,
) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .env("HOME", root.join("home"))
        .env("NATIVE_ID", id)
        .env("NATIVE_CWD", directory)
        .env("QUOTA_TIME", timestamp)
        .args(["-c", r#"
mkdir -p "$HOME/.codex/sessions/2026/10/23"
native="$HOME/.codex/sessions/2026/10/23/rollout-fixture-$NATIVE_ID.jsonl"
printf '{"type":"session_meta","payload":{"id":"%s","cwd":"%s"}}\n' "$NATIVE_ID" "$NATIVE_CWD" > "$native"
sleep 1.1
if [ "$QUOTA_TIME" = current ]; then QUOTA_TIME=$(date -u +%Y-%m-%dT%H:%M:%SZ); fi
printf '{"timestamp":"%s","type":"event_msg","payload":{"type":"token_count","rate_limits":{"secondary":{"used_percent":85}}}}\n' "$QUOTA_TIME" >> "$native"
echo '{"type":"thread.started","thread_id":"s1"}'
sleep 1
touch "$HOME/../continued"
"#]);
    command
}

/// A scratch worktree in a fresh local Git repository.
pub(crate) fn scratch_worktree() -> tempfile::TempDir {
    let worktree = tempfile::tempdir().unwrap();
    assert!(
        git_command()
            .args(["init", "-q"])
            .arg(worktree.path())
            .status()
            .unwrap()
            .success()
    );
    worktree
}

/// Exercise the fully qualified test once per recorded signal, installing
/// the handler only in the subprocess selected for that test.
pub(crate) fn with_recorded_signal(test_name: &str, exercise: impl FnOnce(libc::c_int)) {
    const TEST: &str = "THIRDSHIFT_TEST_SIGNAL_NAME";
    const SIGNAL: &str = "THIRDSHIFT_TEST_SIGNAL";
    if std::env::var(TEST).as_deref() == Ok(test_name) {
        interrupt::install().unwrap();
        exercise(std::env::var(SIGNAL).unwrap().parse().unwrap());
        return;
    }
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(TEST, test_name)
            .env(SIGNAL, signal.to_string())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{test_name}, signal {signal}: {result:?}"
        );
    }
}

/// Run a single test with a Git shim in an isolated child. The parent receives
/// its captured output; the selected child exercises the real interface.
pub(crate) fn git_fault(name: &str, script: &str) -> Option<std::process::Output> {
    if std::env::var("THIRDSHIFT_GIT_FAULT_TEST").as_deref() == Ok(name) {
        return None;
    }
    let temp = tempfile::TempDir::new().unwrap();
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(real_git.status.success());
    let shim = temp.path().join("git");
    write_executable(
        &shim,
        &format!("#!/bin/sh\nset -e\n{script}\nexec \"$THIRDSHIFT_REAL_GIT\" \"$@\"\n"),
    );
    let search = std::env::var_os("PATH").unwrap();
    let path = std::env::join_paths(
        std::iter::once(temp.path().to_path_buf()).chain(std::env::split_paths(&search)),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("PATH", path)
        .env("THIRDSHIFT_GIT_FAULT_TEST", name)
        .env(
            "THIRDSHIFT_REAL_GIT",
            String::from_utf8(real_git.stdout).unwrap().trim(),
        )
        .env("THIRDSHIFT_FAULT_MARKER", temp.path().join("fault"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    Some(output)
}
