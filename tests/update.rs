//! `thirdshift update`, offline: the install receipt lives in the scenario's
//! `$HOME`, and GitHub is a local fake serving the Releases API.

mod support;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::json;
use support::{RunResult, Scenario};
use tempfile::TempDir;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A local stand-in for the GitHub API and its release downloads.
struct FakeReleases {
    server: Child,
    root: TempDir,
    url: String,
}

impl FakeReleases {
    /// Serve an empty directory: no releases yet.
    fn start() -> Self {
        let root = TempDir::new().unwrap();
        let mut server = Command::new("python3")
            .args(["-u", "-m", "http.server", "0", "--bind", "127.0.0.1"])
            .arg("--directory")
            .arg(root.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // "Serving HTTP on 127.0.0.1 port <port> (http://127.0.0.1:<port>/) ..."
        let mut line = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let url = line
            .split_whitespace()
            .find_map(|word| word.strip_prefix('(')?.strip_suffix(')'))
            .unwrap_or_else(|| panic!("unexpected http.server output: {line}"))
            .to_string();
        FakeReleases { server, root, url }
    }

    /// Publish `version` as the latest stable release, with an installer
    /// that runs `installer` (sh).
    fn publish(&self, version: &str, installer: &str) {
        let api = self
            .root
            .path()
            .join("api/v3/repos/JacobStephens2/thirdshift/releases");
        fs::create_dir_all(&api).unwrap();
        let tag = format!("v{version}");
        fs::write(
            api.join("latest"),
            json!({
                "tag_name": tag,
                "name": tag,
                "url": format!("{}api/v3/repos/JacobStephens2/thirdshift/releases/1", self.url),
                "prerelease": false,
                "assets": [{
                    "name": "thirdshift-installer.sh",
                    "url": format!("{}installer", self.url),
                    "browser_download_url": format!("{}installer", self.url),
                }],
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            self.root.path().join("installer"),
            format!("#!/bin/sh\n{installer}\n"),
        )
        .unwrap();
    }

    /// Run `thirdshift update` against this fake GitHub.
    fn update(&self, scenario: &Scenario) -> RunResult {
        update_against(scenario, &self.url)
    }
}

impl Drop for FakeReleases {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}

/// Run `thirdshift update` with the GitHub at `url` in place of github.com.
fn update_against(scenario: &Scenario, url: &str) -> RunResult {
    scenario.run_with_env(&["update"], &[("THIRDSHIFT_INSTALLER_GHE_BASE_URL", url)])
}

/// Write an install receipt, as the shell installer does, saying thirdshift
/// was installed into `install_prefix`.
fn write_install_receipt(scenario: &Scenario, install_prefix: &Path) {
    let receipt_dir = scenario.path("home/.config/thirdshift");
    fs::create_dir_all(&receipt_dir).unwrap();
    fs::write(
        receipt_dir.join("thirdshift-receipt.json"),
        json!({
            "binaries": ["thirdshift"],
            "binary_aliases": {},
            "cdylibs": [],
            "cstaticlibs": [],
            "install_layout": "flat",
            "install_prefix": install_prefix,
            "modify_path": true,
            "provider": { "source": "cargo-dist", "version": "0.33.0" },
            "source": {
                "app_name": "thirdshift",
                "name": "thirdshift",
                "owner": "JacobStephens2",
                "release_type": "github"
            },
            "version": VERSION
        })
        .to_string(),
    )
    .unwrap();
}

/// A scenario whose install receipt is for the binary under test, so that
/// `update` goes on to ask GitHub for the latest release.
fn installed_scenario() -> Scenario {
    let scenario = Scenario::new();
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_thirdshift"));
    write_install_receipt(&scenario, binary.parent().unwrap());
    scenario
}

/// Assert that `update` refused to touch this copy of thirdshift: exit 1,
/// nothing on stdout, and stderr naming the commands that update the other
/// kinds of install.
fn assert_update_refused(result: &RunResult) {
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    for hint in [
        "cargo install --path .",
        "cargo install thirdshift",
        "thirdshift-installer.sh | sh",
    ] {
        assert!(
            result.stderr.contains(hint),
            "expected {hint:?} in stderr: {}",
            result.stderr
        );
    }
}

#[test]
fn without_an_install_receipt_update_refuses_with_a_reinstall_hint() {
    let scenario = Scenario::new();
    let empty = TempDir::new().unwrap();
    let empty = empty.path().to_str().unwrap();

    let result = scenario.run_with_env(&["update"], &[("HOME", empty), ("XDG_CONFIG_HOME", empty)]);

    assert_update_refused(&result);
    assert!(
        result.stderr.contains("no install receipt"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn update_refuses_a_copy_the_install_receipt_is_not_for() {
    let scenario = Scenario::new();
    let install_prefix = scenario.path("home/.local/bin");
    fs::create_dir_all(&install_prefix).unwrap();
    write_install_receipt(&scenario, &install_prefix);

    let result = scenario.run(&["update"]);

    assert_update_refused(&result);
    assert!(
        result
            .stderr
            .contains(&install_prefix.display().to_string()),
        "expected the receipt's install location in stderr: {}",
        result.stderr
    );
}

#[test]
fn on_the_latest_release_update_says_so_and_installs_nothing() {
    let scenario = installed_scenario();
    let github = FakeReleases::start();
    let ran = scenario.path("installer-ran");
    github.publish(VERSION, &format!("touch '{}'", ran.display()));

    let result = github.update(&scenario);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(
        result.stderr,
        format!("thirdshift: thirdshift {VERSION} is the latest release\n")
    );
    assert!(!ran.exists(), "the installer ran");
}

#[test]
fn with_a_newer_release_update_runs_its_installer_and_keeps_stdout_empty() {
    let scenario = installed_scenario();
    let github = FakeReleases::start();
    let ran = scenario.path("installer-ran");
    github.publish(
        "99.0.0",
        &format!(
            "echo 'installing thirdshift 99.0.0'\ntouch '{}'",
            ran.display()
        ),
    );

    let result = github.update(&scenario);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(ran.exists(), "the installer didn't run");
    assert!(
        result.stderr.ends_with(&format!(
            "thirdshift: updated thirdshift {VERSION} to 99.0.0\n"
        )),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_failing_installer_makes_update_fail_with_its_output_on_stderr() {
    let scenario = installed_scenario();
    let github = FakeReleases::start();
    github.publish("99.0.0", "echo 'download failed'\nexit 1");

    let result = github.update(&scenario);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result.stderr.contains("download failed"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn with_no_release_on_github_update_fails_with_an_explanation() {
    let scenario = installed_scenario();
    let github = FakeReleases::start();

    let result = github.update(&scenario);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result
            .stderr
            .starts_with("thirdshift: can't update from GitHub Releases: "),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn when_github_is_unreachable_update_fails_with_an_explanation() {
    let scenario = installed_scenario();

    // Nothing listens on port 1, so the request fails without a network.
    let result = update_against(&scenario, "http://127.0.0.1:1/");

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    assert!(
        result
            .stderr
            .starts_with("thirdshift: can't update from GitHub Releases: ")
            && result.stderr.contains("127.0.0.1:1"),
        "stderr: {}",
        result.stderr
    );
}
