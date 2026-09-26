//! The thirdshift.app deploy configuration in `site/deploy/`: the publish
//! script run against a local origin and a temporary web root, and the Caddy
//! block and systemd units checked with their own tools where installed.
//!
//! The publish script targets the Linux server and swaps its symlink with GNU
//! `mv -T`, so its tests run on Linux only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use serde_json::Value;
use tempfile::TempDir;

fn deploy_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("site/deploy")
}

/// Keeps the machine's git configuration (signing, hooks, default branch) out
/// of the test's git and the publish script's.
fn without_user_git_config(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = without_user_git_config(&mut Command::new("git"))
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("could not run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// Whether `tool` is on PATH; the Caddy and systemd checks skip without it.
fn installed(tool: &str) -> bool {
    Command::new(tool).arg("version").output().is_ok()
}

/// A bare origin, a contributor's clone that pushes to it, the deploy
/// checkout the publish script fetches into, and an empty web root.
struct Deploy {
    temp: TempDir,
    /// The commit on main before the test adds any.
    first_commit: String,
}

impl Deploy {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        git(
            temp.path(),
            &["init", "-q", "--bare", "-b", "main", "origin.git"],
        );
        git(temp.path(), &["clone", "-q", "origin.git", "contributor"]);
        let mut deploy = Deploy {
            temp,
            first_commit: String::new(),
        };
        deploy.first_commit = deploy.commit(&[
            ("site/index.html", "home v1"),
            ("site/prompts/index.html", "prompts v1"),
            ("site/deploy/publish.sh", "not for the web"),
            ("README.md", "readme v1"),
        ]);
        git(
            deploy.temp_dir(),
            &["clone", "-q", "origin.git", "checkout"],
        );
        fs::create_dir(deploy.web_root()).unwrap();
        deploy
    }

    fn temp_dir(&self) -> &Path {
        self.temp.path()
    }

    fn checkout(&self) -> PathBuf {
        self.temp_dir().join("checkout")
    }

    fn web_root(&self) -> PathBuf {
        self.temp_dir().join("www")
    }

    /// Commits `files` in the contributor's clone and pushes them to main.
    fn commit(&self, files: &[(&str, &str)]) -> String {
        let clone = self.temp_dir().join("contributor");
        for (path, contents) in files {
            let path = clone.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        git(&clone, &["add", "-A"]);
        git(&clone, &["commit", "-q", "-m", "change"]);
        git(&clone, &["push", "-q", "origin", "HEAD:main"]);
        git(&clone, &["rev-parse", "HEAD"])
    }

    fn publish(&self) {
        let output = without_user_git_config(&mut Command::new(deploy_dir().join("publish.sh")))
            .arg(self.checkout())
            .arg(self.web_root())
            .output()
            .expect("could not run publish.sh");
        assert!(
            output.status.success(),
            "publish.sh failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The directory the web root's `current` symlink points at.
    fn current(&self) -> PathBuf {
        fs::canonicalize(self.web_root().join("current")).unwrap()
    }

    fn served(&self, path: &str) -> String {
        fs::read_to_string(self.web_root().join("current").join(path)).unwrap()
    }

    /// Every entry under the web root, with modification times, so a run
    /// that changes anything at all shows up as a difference.
    fn snapshot(&self) -> Vec<(PathBuf, SystemTime)> {
        fn walk(dir: &Path, out: &mut Vec<(PathBuf, SystemTime)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let meta = fs::symlink_metadata(entry.path()).unwrap();
                out.push((entry.path(), meta.modified().unwrap()));
                if meta.is_dir() {
                    walk(&entry.path(), out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.web_root(), &mut out);
        out.sort();
        out
    }
}

#[test]
#[cfg(target_os = "linux")]
fn publishing_serves_the_site_without_the_deploy_directory_and_records_the_commit() {
    let deploy = Deploy::new();

    deploy.publish();

    assert_eq!(deploy.served("index.html"), "home v1");
    assert_eq!(deploy.served("prompts/index.html"), "prompts v1");
    assert_eq!(
        deploy.served("commit.txt"),
        format!("{}\n", deploy.first_commit)
    );
    assert!(!deploy.current().join("deploy").exists());
    assert!(!deploy.current().join("README.md").exists());
}

#[test]
#[cfg(target_os = "linux")]
fn publishing_again_with_no_new_commits_changes_nothing() {
    let deploy = Deploy::new();
    deploy.publish();
    let before = deploy.snapshot();

    deploy.publish();

    assert_eq!(deploy.snapshot(), before);
}

#[test]
#[cfg(target_os = "linux")]
fn a_commit_outside_the_served_site_changes_nothing_served() {
    let deploy = Deploy::new();
    deploy.publish();
    let before = deploy.snapshot();
    let head = deploy.commit(&[
        ("README.md", "readme v2"),
        ("site/deploy/publish.sh", "still not for the web"),
    ]);

    deploy.publish();

    assert_eq!(deploy.snapshot(), before);
    // The deploy checkout still follows main, so the units and Caddy block a
    // human installs from it are current.
    assert_eq!(git(&deploy.checkout(), &["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read_to_string(deploy.checkout().join("site/deploy/publish.sh")).unwrap(),
        "still not for the web"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_commit_to_the_site_swaps_in_a_new_copy() {
    let deploy = Deploy::new();
    deploy.publish();
    let old = deploy.current();

    let head = deploy.commit(&[("site/index.html", "home v2")]);
    deploy.publish();

    assert_ne!(deploy.current(), old);
    assert!(
        fs::symlink_metadata(deploy.web_root().join("current"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(deploy.served("index.html"), "home v2");
    assert_eq!(deploy.served("commit.txt"), format!("{head}\n"));
    // The copy it replaced is left whole for requests still reading it.
    assert_eq!(
        fs::read_to_string(old.join("index.html")).unwrap(),
        "home v1"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn only_the_live_and_previous_copies_are_kept() {
    let deploy = Deploy::new();
    deploy.publish();
    for n in 2..=4 {
        deploy.commit(&[("site/index.html", &format!("home v{n}"))]);
        deploy.publish();
    }

    let kept = fs::read_dir(deploy.web_root().join("copies"))
        .unwrap()
        .count();

    assert_eq!(kept, 2);
    assert_eq!(deploy.served("index.html"), "home v4");
}

#[test]
fn the_caddy_block_adapts() {
    if !installed("caddy") {
        eprintln!("skipping: caddy is not installed");
        return;
    }
    let output = Command::new("caddy")
        .args(["adapt", "--adapter", "caddyfile", "--config"])
        .arg(deploy_dir().join("Caddyfile"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "caddy adapt: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config: Value = serde_json::from_slice(&output.stdout).unwrap();

    let site = caddy_route(&config, "thirdshift.app");
    assert!(site.contains(r#""handler":"file_server""#), "{site}");
    assert!(site.contains(&redirect(
        302,
        "https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh"
    )), "{site}");
    let www = caddy_route(&config, "www.thirdshift.app");
    assert!(
        www.contains(&redirect(301, "https://thirdshift.app{http.request.uri}")),
        "{www}"
    );
}

/// The adapted Caddy route that matches `host`, as JSON text.
fn caddy_route(config: &Value, host: &str) -> String {
    let routes = &config["apps"]["http"]["servers"]["srv0"]["routes"];
    routes
        .as_array()
        .unwrap()
        .iter()
        .find(|route| route["match"][0]["host"][0] == host)
        .unwrap_or_else(|| panic!("no route for {host} in {routes}"))
        .to_string()
}

/// How a redirect handler reads in adapted Caddy JSON.
fn redirect(status: u16, location: &str) -> String {
    format!(r#""headers":{{"Location":["{location}"]}},"status_code":{status}"#)
}

#[test]
fn the_systemd_units_verify() {
    if !installed("systemd-analyze") {
        eprintln!("skipping: systemd-analyze is not installed");
        return;
    }
    let output = Command::new("systemd-analyze")
        .args(["verify", "--man=no"])
        .arg(deploy_dir().join("thirdshift-site-publish.service"))
        .arg(deploy_dir().join("thirdshift-site-publish.timer"))
        .output()
        .unwrap();
    assert!(
        output.status.success() && output.stderr.is_empty(),
        "systemd-analyze verify: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
