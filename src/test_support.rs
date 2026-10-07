//! Shared unit-test helpers: run signal-sensitive tests in isolated
//! processes so interruption and fixture environment changes cannot leak into
//! another test, and write executables other tests can't hold open.

use std::process::Command;

use crate::interrupt;

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

/// Write `contents` to `path` as an executable. A child process writes it,
/// not this one: a file this process held open for writing would be
/// inherited by whatever another test spawns meanwhile, and couldn't be run
/// until that let go of it ("Text file busy").
pub(crate) fn write_executable(path: &std::path::Path, contents: &str) {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(contents.as_bytes())
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "could not write {}", path.display());
}
