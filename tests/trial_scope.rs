//! Real Trial -> Harness placement, with the user manager when available.

#[cfg(target_os = "linux")]
#[test]
fn trial_sessions_keep_their_cap_and_clean_detached_work() {
    let output = std::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/trial-scope.py"
        ))
        .arg(env!("CARGO_BIN_EXE_thirdshift"))
        .output()
        .unwrap();
    if output.status.code() == Some(77) {
        eprintln!("skipped: requires a systemd user manager");
        return;
    }
    assert!(output.status.success(), "{output:?}");
}
