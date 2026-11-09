//! The crates.io package: it carries sources, Factory skills and Built-in processes the
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
/// dev-dependencies, so this is stricter than the binary's own normal and
/// build dependencies, which is all ADR 0003 needs: a test-only OpenSSL is
/// caught too.
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
fn package_carries_the_built_in_process_folder() {
    let list = package_list();
    for expected in [
        "processes/README.md",
        "processes/ticket-run.toml",
        "processes/spec-run.toml",
        "processes/architect-run.toml",
        "processes/security-run.toml",
    ] {
        assert!(
            list.iter().any(|path| path == expected),
            "{expected} is missing"
        );
    }
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
fn non_linux_compilation_reports_only_the_platform_error() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let temp = tempfile::TempDir::new().unwrap();
    for entry in fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "main.rs" {
            std::os::unix::fs::symlink(entry.path(), temp.path().join(entry.file_name())).unwrap();
        }
    }
    // Make only the crate root see a non-Linux configuration. Keeping the
    // host's std avoids a cross toolchain; withholding external dependencies
    // makes any module that escapes the platform gate produce extra errors.
    let main = fs::read_to_string(source.join("main.rs")).unwrap();
    let root = temp.path().join("main.rs");
    fs::write(
        &root,
        main.replace("target_os = \"linux\"", "thirdshift_linux"),
    )
    .unwrap();
    for test in [false, true] {
        let mut command = Command::new(std::env::var_os("RUSTC").unwrap_or("rustc".into()));
        command.args(["--edition=2024", "--emit=metadata", "--error-format=json"]);
        if test {
            command.arg("--test");
        }
        let output = command
            .arg(&root)
            .arg("-o")
            .arg(temp.path().join("main.rmeta"))
            .output()
            .expect("could not compile the crate root");
        assert!(!output.status.success(), "unsupported build succeeded");
        let diagnostics: Vec<serde_json::Value> = String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let messages: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| matches!(diagnostic["level"].as_str(), Some("error" | "warning")))
            .map(|diagnostic| diagnostic["message"].as_str().unwrap())
            .filter(|message| !message.starts_with("aborting due to"))
            .collect();
        assert!(messages.contains(&"thirdshift supports Linux only. On macOS, run it inside an Ubuntu 24.04 Linux VM (such as Lima). On Windows, use WSL2."), "missing platform error: {messages:?}");
        assert_eq!(
            messages.len(),
            1,
            "test={test}: extra diagnostics: {:?}",
            &messages[..messages.len().min(5)]
        );
    }
}

#[test]
fn package_carries_the_factory_skills_with_their_license_and_credits() {
    let list = package_list();
    for expected in [
        "skills/LICENSE",
        "skills/thirdshift-pr/CREDITS.md",
        "skills/thirdshift-implement/SKILL.md",
        "skills/thirdshift-diagnosing-bugs/SKILL.md",
        "skills/thirdshift-improve-codebase-architecture/SKILL.md",
        "skills/thirdshift-improve-codebase-architecture/REPORT.md",
        "skills/thirdshift-to-spec/SKILL.md",
        "skills/thirdshift-to-tickets/SKILL.md",
        "skills/thirdshift-codebase-design/SKILL.md",
        "skills/thirdshift-codebase-design/DEEPENING.md",
        "skills/thirdshift-codebase-design/DESIGN-IT-TWICE.md",
        "skills/thirdshift-security-audit/SKILL.md",
        "skills/thirdshift-security-audit/LICENSE",
        "skills/thirdshift-security-audit/CREDITS.md",
        "skills/thirdshift-security-audit/report-schema.json",
        "skills/thirdshift-security-audit/validate-findings.cjs",
        "skills/thirdshift-security-audit/validate-findings.test.cjs",
        "skills/thirdshift-security-audit/validate-coverage-ledger.cjs",
        "skills/thirdshift-security-audit/validate-coverage-ledger.test.cjs",
        "src/main.rs",
        "askama.toml",
        "src/console/inbox.html",
        "src/console/row.html",
        "src/console/assets/console.css",
        "src/console/assets/console.js",
        "src/console/assets/vendor/htmx-2.0.7.min.js",
        "src/console/assets/vendor/htmx-ext-sse-2.2.3.js",
        "src/console/assets/vendor/idiomorph-0.7.4.min.js",
        "src/console/assets/fonts/ibm-plex-sans-latin.woff2",
        "src/console/assets/fonts/ibm-plex-mono-latin.woff2",
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
