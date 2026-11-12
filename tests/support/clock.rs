//! Child-only wall-clock changes, leaving host and monotonic clocks untouched.

use std::path::Path;
use std::process::Command;

pub fn compile(library: &Path) {
    let compiled = Command::new("cc")
        .args(["-shared", "-fPIC"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/console/session-clock.c"
        ))
        .arg("-o")
        .arg(library)
        .arg("-ldl")
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
}
