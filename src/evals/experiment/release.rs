//! Release selection can skip work without creating an Experiment.

use anyhow::Result;
use std::path::Path;
use std::process::Command;

use crate::process::{self, Control, Interruption};

pub(super) fn supports_trials(repository: &Path, root: &Path, commit: &str) -> Result<bool> {
    let builds = root.join("builds");
    std::fs::create_dir_all(&builds)?;
    let binary = super::arms::build(repository, &builds.canonicalize()?, commit)?;
    // No request: a supporting build prints usage without starting a Trial.
    let output = process::output(
        Command::new(binary).arg("--trial"),
        None,
        Control {
            name: "release Baseline Trial flag",
            interruption: Interruption::Ordinary,
            stop: &|child| process::stop(child, &[libc::SIGTERM, libc::SIGKILL]),
        },
    )?;
    anyhow::ensure!(
        output.status.code().is_some(),
        "release Baseline probe was killed: {}",
        output.status
    );
    Ok(
        String::from_utf8_lossy(&output.stderr)
            .contains("usage: thirdshift --trial <request.json>"),
    )
}

pub(super) fn skip(code: &str, reason: &str, json: bool) -> Result<()> {
    crate::progress::step(reason);
    if json {
        println!(
            "{}",
            serde_json::json!({"release_skip": code, "reason": reason})
        );
    } else {
        println!("{reason}");
    }
    Ok(())
}
