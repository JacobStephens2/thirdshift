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

    // A channel name or a version without its patch number would move on its
    // own; only all three numbers stay put.
    let parts: Vec<&str> = channel.split('.').collect();
    assert!(
        parts.len() == 3 && parts.iter().all(|part| part.parse::<u32>().is_ok()),
        "channel {channel:?} is not an exact version"
    );
}

#[test]
fn the_toolchain_file_names_rustfmt_and_clippy() {
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
            // The toolchain actions (dtolnay/rust-toolchain,
            // actions-rust-lang/setup-rust-toolchain) can take their version
            // from a ref or an input. `rustup toolchain install` reads the
            // toolchain file only when it is given no toolchain.
            let toolchain_action = line.contains("uses:") && line.contains("rust-toolchain");
            let names_a_toolchain = line
                .split_once("rustup toolchain install")
                .is_some_and(|(_, arguments)| !arguments.trim().is_empty());
            let floating = toolchain_action
                || names_a_toolchain
                || line.contains("rustup default")
                || line.contains("rustup update");
            assert!(!floating, "{}:{}: {line}", path.display(), number + 1);
        }
    }
}
