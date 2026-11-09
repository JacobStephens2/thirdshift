//! Shared handshakes for overlapping Commands against the CLI.

use std::{fmt::Display, fs};

use super::Scenario;

/// A script that announces `started-<name>` in the scenario root and waits
/// for `release-<name>`, or until the scenario is gone. Append the agent's
/// work at the call site.
pub fn waits_on(name: impl Display) -> String {
    format!(
        r#"
root=$(dirname "$FAKE_CLAUDE_RECORD")
touch "$root/started-{name}"
while [ -d "$root" ] && [ ! -e "$root/release-{name}" ]; do sleep 0.05; done
"#
    )
}

/// Let an agent held by [`waits_on`] continue.
pub fn release(scenario: &Scenario, name: impl Display) {
    fs::write(scenario.path(&format!("release-{name}")), "").unwrap();
}

/// The Pickup reservation refs on origin, one line each.
pub fn reservations(scenario: &Scenario) -> String {
    scenario
        .origin_git(&[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads/thirdshift/pickup",
        ])
        .trim_end()
        .to_string()
}
