//! The crates.io package: it carries the sources and the Factory skills the
//! binary embeds, and none of the repository's own agent tooling, docs, site
//! or tests. And the binary's dependencies: HTTPS through rustls, never
//! OpenSSL (ADR 0003).

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

/// The toolchain pin is for building in this repository. In the package it
/// would do nothing for `cargo install thirdshift`, which builds with the
/// installer's own toolchain, and it stays out so that stays plain.
#[test]
fn package_leaves_out_the_toolchain_pin() {
    let list = package_list();
    assert!(
        !list.iter().any(|path| path == "rust-toolchain.toml"),
        "rust-toolchain.toml is in {list:?}"
    );
}

#[test]
fn package_carries_the_factory_skills_with_their_license_and_credits() {
    let list = package_list();
    for expected in [
        "skills/LICENSE",
        "skills/pr/CREDITS.md",
        "skills/implement/SKILL.md",
        "skills/improve-codebase-architecture/SKILL.md",
        "skills/improve-codebase-architecture/REPORT.md",
        "skills/to-spec/SKILL.md",
        "skills/to-tickets/SKILL.md",
        "skills/codebase-design/SKILL.md",
        "skills/codebase-design/DEEPENING.md",
        "skills/codebase-design/DESIGN-IT-TWICE.md",
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

#[test]
fn the_binary_never_links_openssl() {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "--target",
            "all",
            "--offline",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("could not run cargo tree");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for package in String::from_utf8(output.stdout).unwrap().lines() {
        for linked in ["openssl-sys ", "openssl ", "native-tls "] {
            assert!(!package.starts_with(linked), "{package} is a dependency");
        }
    }
}
