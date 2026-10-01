//! The crates.io package: it carries the sources and the Factory skills the
//! binary embeds, and none of the repository's own agent tooling, docs, site
//! or tests. And the binary's dependencies, as the lockfile names them: HTTPS
//! through rustls, never OpenSSL (ADR 0003).

use std::fs;
use std::path::Path;
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

/// The packages in a lockfile that would link OpenSSL, as `name version`.
///
/// The lockfile names every package for every target without touching the
/// registry, where `cargo tree --target all --offline` fails unless every
/// platform's crates are already in the cargo cache. It also names the
/// dev-dependencies, so this is stricter than ADR 0003's normal and build
/// dependencies: a test-only OpenSSL is caught too.
fn openssl_packages(lockfile: &str) -> Vec<String> {
    let lockfile: toml::Table = lockfile.parse().expect("the lockfile is not TOML");
    lockfile["package"]
        .as_array()
        .expect("the lockfile has no packages")
        .iter()
        .filter_map(|package| {
            let name = package["name"].as_str().unwrap();
            ["openssl-sys", "openssl", "native-tls"]
                .contains(&name)
                .then(|| format!("{name} {}", package["version"].as_str().unwrap()))
        })
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
    let lockfile = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock"))
        .expect("could not read Cargo.lock");
    let linked = openssl_packages(&lockfile);
    assert!(linked.is_empty(), "{linked:?} are dependencies");
}

#[test]
fn a_lockfile_with_openssl_or_native_tls_in_it_is_caught() {
    let lockfile = r#"
version = 4

[[package]]
name = "native-tls"
version = "0.2.14"
dependencies = [
 "openssl",
 "openssl-probe",
 "openssl-sys",
]

[[package]]
name = "openssl"
version = "0.10.73"

[[package]]
name = "openssl-probe"
version = "0.2.1"

[[package]]
name = "openssl-sys"
version = "0.9.109"

[[package]]
name = "rustls"
version = "0.23.31"
"#;
    assert_eq!(
        openssl_packages(lockfile),
        [
            "native-tls 0.2.14",
            "openssl 0.10.73",
            "openssl-sys 0.9.109"
        ]
    );
}
