//! The Rust toolchain pin: `rust-toolchain.toml` names one exact version, and
//! no workflow installs a floating channel beside it, so a new compiler
//! arrives as a pull request that changes that file.

use std::fs;
use std::path::Path;

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The `[toolchain]` table of `rust-toolchain.toml`.
fn toolchain() -> toml::Table {
    let text = fs::read_to_string(repository().join("rust-toolchain.toml"))
        .expect("could not read rust-toolchain.toml");
    let file: toml::Table = text.parse().expect("rust-toolchain.toml is not TOML");
    file["toolchain"].as_table().unwrap().clone()
}

#[test]
fn the_toolchain_file_names_an_exact_version() {
    let toolchain = toolchain();
    let channel = toolchain["channel"].as_str().unwrap();

    // `1.99` or `stable` would move on their own; only `1.99.0` stays put.
    let parts: Vec<&str> = channel.split('.').collect();
    assert!(
        parts.len() == 3 && parts.iter().all(|part| part.parse::<u32>().is_ok()),
        "channel {channel:?} is not an exact version"
    );
}

#[test]
fn the_toolchain_file_names_the_components_ci_runs() {
    let toolchain = toolchain();
    let components: Vec<&str> = toolchain["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| component.as_str().unwrap())
        .collect();

    for needed in ["rustfmt", "clippy"] {
        assert!(components.contains(&needed), "components: {components:?}");
    }
}

#[test]
fn no_workflow_installs_a_floating_toolchain() {
    let workflows = repository().join(".github/workflows");
    for entry in fs::read_dir(&workflows).unwrap() {
        let path = entry.unwrap().path();
        let text = fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            // dtolnay/rust-toolchain takes its version from the ref or an
            // input, never from the toolchain file. `rustup toolchain
            // install` reads the file only when it is given no toolchain.
            let names_a_toolchain = line
                .split_once("rustup toolchain install")
                .is_some_and(|(_, arguments)| !arguments.trim().is_empty());
            let floating = names_a_toolchain
                || ["dtolnay/rust-toolchain", "rustup default", "rustup update"]
                    .iter()
                    .any(|install| line.contains(install));
            assert!(!floating, "{}:{}: {line}", path.display(), number + 1);
        }
    }
}
