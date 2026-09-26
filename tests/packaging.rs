//! The crates.io package: it carries the sources and the Factory skills the
//! binary embeds, and none of the repository's own agent tooling, docs, site or
//! tests.

use std::process::Command;

/// The paths `cargo package` would put in the crate.
fn package_list() -> Vec<String> {
    let output = Command::new(env!("CARGO"))
        .args(["package", "--list", "--allow-dirty", "--offline"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("could not run cargo package");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn package_leaves_out_the_repository_tooling() {
    for path in package_list() {
        for excluded in [".agents/", ".grok/", ".claude/", "docs/", "site/", "tests/"] {
            assert!(!path.starts_with(excluded), "{path} is in the package");
        }
    }
}

#[test]
fn package_carries_the_factory_skills_with_their_license_and_credits() {
    let list = package_list();
    for expected in [
        "skills/LICENSE",
        "skills/pr/CREDITS.md",
        "skills/implement/SKILL.md",
        "src/main.rs",
        "README.md",
        "LICENSE",
    ] {
        assert!(
            list.iter().any(|path| path == expected),
            "{expected} is missing from {list:?}"
        );
    }
}
