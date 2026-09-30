//! The maintainer's release script, `scripts/release.sh`, run as a black box
//! against a bare local origin with the fake `gh` on PATH and the user's git
//! configuration kept out, as the site deploy tests run the publish script.
//!
//! The script, like the fake `gh` it drives here, relies on GNU tools, so its
//! tests run on Linux only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

const REPO: &str = "JacobStephens2/thirdshift";

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Keeps the machine's git configuration (signing, hooks, default branch) out
/// of the test's git, the script's and the fake `gh`'s.
fn isolated(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = isolated(&mut Command::new("git"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("could not run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// The manifest or lockfile from this repo, with thirdshift's own version
/// set to `version`, so the fixture has the real files' shape.
fn with_version(file: &str, version: &str) -> String {
    let text = fs::read_to_string(manifest_dir().join(file)).unwrap();
    let anchor = "name = \"thirdshift\"\nversion = \"";
    let start = text.find(anchor).expect("no thirdshift version") + anchor.len();
    let end = start + text[start..].find('"').unwrap();
    format!("{}{version}{}", &text[..start], &text[end..])
}

/// A bare origin whose main is at version 0.1.0, tagged `v0.1.0`, with one PR
/// merged before the tag and one after, and a green CI run on main's tip; a
/// contributor's clone that makes them; the maintainer's clone the script
/// runs in; and the fake `gh`'s state.
struct Release {
    temp: TempDir,
}

impl Release {
    fn new() -> Self {
        let release = Release {
            temp: TempDir::new().unwrap(),
        };
        let root = release.root();
        git(root, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        fs::create_dir(release.bin()).unwrap();
        for name in ["gh", "claude"] {
            let fake = release.bin().join(name);
            fs::copy(manifest_dir().join("tests/fakes").join(name), &fake).unwrap();
            fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(
            root.join("gh-state.json"),
            json!({"repo": REPO, "issues": {}, "prs": [], "checks": {}, "statuses": {}})
                .to_string(),
        )
        .unwrap();
        git(root, &["clone", "-q", "origin.git", "contributor"]);
        let contributor = release.contributor();
        fs::write(
            contributor.join("Cargo.toml"),
            with_version("Cargo.toml", "0.1.0"),
        )
        .unwrap();
        fs::write(
            contributor.join("Cargo.lock"),
            with_version("Cargo.lock", "0.1.0"),
        )
        .unwrap();
        fs::write(contributor.join("README.md"), "readme\n").unwrap();
        git(&contributor, &["add", "-A"]);
        git(&contributor, &["commit", "-q", "-m", "Initial"]);
        git(&contributor, &["push", "-q", "origin", "HEAD:main"]);

        release.merge_pr("before-tag", "Shipped in 0.1.0");
        git(&contributor, &["pull", "-q", "origin", "main"]);
        git(&contributor, &["tag", "v0.1.0"]);
        git(&contributor, &["push", "-q", "origin", "v0.1.0"]);
        release.merge_pr("after-tag", "Add the frobnicator");

        git(root, &["clone", "-q", "origin.git", "maintainer"]);
        release.main_ci("completed", "success");
        release
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    /// The fakes, copied out executable.
    fn bin(&self) -> PathBuf {
        self.root().join("bin")
    }

    fn contributor(&self) -> PathBuf {
        self.root().join("contributor")
    }

    fn maintainer(&self) -> PathBuf {
        self.root().join("maintainer")
    }

    fn origin(&self, args: &[&str]) -> String {
        git(&self.root().join("origin.git"), args)
    }

    fn gh(&self, args: &[&str]) -> String {
        let output = self
            .command(&self.bin().join("gh"), self.root())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "gh {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    /// Opens a PR titled `title` from a new branch `head` off main and merges
    /// it with a merge commit, through the fake `gh`.
    fn merge_pr(&self, head: &str, title: &str) {
        let contributor = self.contributor();
        git(&contributor, &["checkout", "-q", "-b", head, "origin/main"]);
        fs::write(contributor.join(format!("{head}.txt")), head).unwrap();
        git(&contributor, &["add", "-A"]);
        git(&contributor, &["commit", "-q", "-m", title]);
        git(&contributor, &["push", "-q", "origin", head]);
        let sha = git(&contributor, &["rev-parse", "HEAD"]);
        self.gh(&[
            "pr", "create", "--head", head, "--base", "main", "--title", title, "--body", "",
        ]);
        self.gh(&["pr", "merge", head, "--merge", "--match-head-commit", &sha]);
        git(&contributor, &["fetch", "-q", "origin"]);
        git(&contributor, &["checkout", "-q", "--detach", "origin/main"]);
    }

    /// Makes the next head commit CI reads report `checks`, a JSON list.
    fn ci_reports(&self, checks: &str) {
        self.gh(&[
            "fake",
            "on-ci-read",
            "1",
            &format!("gh fake checks \"$FAKE_CI_SHA\" '{checks}'"),
        ]);
    }

    /// Records a CI run on main's tip on origin, newer than any before it.
    fn main_ci(&self, status: &str, conclusion: &str) {
        let run = json!({
            "branch": "main",
            "workflow": "ci.yml",
            "event": "push",
            "headSha": self.origin(&["rev-parse", "main"]),
            "status": status,
            "conclusion": conclusion,
        });
        self.gh(&["fake", "run", &run.to_string()]);
    }

    /// Every ref on origin with the commit it points at, to show a refusal
    /// pushed nothing.
    fn origin_refs(&self) -> String {
        self.origin(&["for-each-ref", "--format=%(refname) %(objectname)"])
    }

    fn command(&self, program: &Path, dir: &Path) -> Command {
        let path = format!(
            "{}:{}",
            self.bin().display(),
            std::env::var("PATH").unwrap()
        );
        let mut command = Command::new(program);
        isolated(&mut command)
            .current_dir(dir)
            .env("PATH", path)
            .env("FAKE_GH_STATE", self.root().join("gh-state.json"))
            .env("FAKE_GH_RECORD", self.root().join("gh-calls.json"))
            .env("RELEASE_POLL_SECONDS", "0");
        command
    }

    fn run_script(&self, version: &str) -> Output {
        self.command(
            &manifest_dir().join("scripts/release.sh"),
            &self.maintainer(),
        )
        .arg(version)
        .output()
        .expect("could not run release.sh")
    }

    fn gh_state(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(self.root().join("gh-state.json")).unwrap())
            .unwrap()
    }

    fn pr(&self, head: &str) -> Value {
        self.gh_state()["prs"]
            .as_array()
            .unwrap()
            .iter()
            .rfind(|pr| pr["head"] == head)
            .unwrap_or_else(|| panic!("no PR from {head}"))
            .clone()
    }

    /// The head commit of `head`'s newest PR, as it was merged if it was.
    fn pr_head(&self, head: &str) -> String {
        self.pr(head)["headRefOid"].as_str().unwrap().to_owned()
    }

    fn origin_tag(&self, tag: &str) -> Option<String> {
        let refs = self.origin(&["tag", "--list", tag]);
        (!refs.is_empty()).then(|| self.origin(&["rev-parse", &format!("{tag}^{{commit}}")]))
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const GREEN: &str = r#"[{"name": "test", "conclusion": "success", "pending_polls": 2}]"#;
const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;

#[test]
#[cfg(target_os = "linux")]
fn a_release_bumps_only_the_version_merges_the_pr_and_tags_the_merge_commit() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let main_before = release.origin(&["rev-parse", "main"]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.pr_head("release-0.2.0");
    assert_eq!(
        release.origin(&["diff", "--numstat", &main_before, &head]),
        "1\t1\tCargo.lock\n1\t1\tCargo.toml"
    );
    let diff = release.origin(&["diff", "-U0", &main_before, &head]);
    assert!(
        diff.contains("-version = \"0.1.0\"\n+version = \"0.2.0\""),
        "{diff}"
    );
    assert_eq!(
        release.origin(&["rev-list", "--count", &format!("{main_before}..{head}")]),
        "1"
    );

    let pr = release.pr("release-0.2.0");
    assert_eq!(pr["base"], "main");
    assert_eq!(pr["title"], "Release 0.2.0");
    assert_eq!(pr["state"], "MERGED");

    let merge = release.origin(&["rev-parse", "main"]);
    assert_eq!(
        release.origin(&["rev-list", "--parents", "-n", "1", &merge]),
        format!("{merge} {main_before} {head}")
    );
    assert_eq!(release.origin_tag("v0.2.0"), Some(merge));
}

#[test]
#[cfg(target_os = "linux")]
fn the_pr_body_has_a_marked_summary_of_the_generated_notes_then_the_version_diff() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let body = release.pr("release-0.2.0")["body"]
        .as_str()
        .unwrap()
        .to_owned();
    let start = body.find("<!-- release-summary:start -->").expect(&body);
    let end = body.find("<!-- release-summary:end -->").expect(&body);
    let summary = &body[start..end];
    assert!(summary.contains("Add the frobnicator"), "{body}");
    assert!(!summary.contains("Shipped in 0.1.0"), "{body}");
    let diff = &body[end..];
    assert!(diff.contains("```diff"), "{body}");
    assert!(
        diff.contains("-version = \"0.1.0\"\n+version = \"0.2.0\""),
        "{body}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn each_step_prints_a_progress_line_on_stderr() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let merge = release.origin(&["rev-parse", "main"]);
    let url = format!("https://github.com/{REPO}/pull/3");
    let expected = [
        format!("release: opened {url}"),
        format!("release: waiting for CI on {url}"),
        format!("release: merged {url} as {}", &merge[..7]),
        format!(
            "release: tagged v0.2.0 on {} and pushed the tag",
            &merge[..7]
        ),
    ];
    let err = stderr(&output);
    let lines: Vec<&str> = err.lines().collect();
    let mut at = 0;
    for line in &expected {
        at += lines[at..]
            .iter()
            .position(|l| l == line)
            .unwrap_or_else(|| panic!("no {line:?} in order in {lines:#?}"))
            + 1;
    }
    assert!(output.stdout.is_empty());
}

#[test]
#[cfg(target_os = "linux")]
fn failing_checks_leave_the_pr_open_with_no_tag() {
    let release = Release::new();
    release.ci_reports(RED);
    let main_before = release.origin(&["rev-parse", "main"]);

    let output = release.run_script("0.2.0");

    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("CI failed") && err.contains("test"), "{err}");
    assert!(err.contains("left open"), "{err}");
    assert_eq!(release.pr("release-0.2.0")["state"], "OPEN");
    assert_eq!(release.origin(&["rev-parse", "main"]), main_before);
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

#[test]
#[cfg(target_os = "linux")]
fn the_maintainers_checkout_neither_affects_the_release_nor_is_changed() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let maintainer = release.maintainer();
    git(&maintainer, &["checkout", "-q", "-b", "wip"]);
    fs::write(maintainer.join("Cargo.toml"), "local edit\n").unwrap();
    fs::write(maintainer.join("scratch.txt"), "untracked\n").unwrap();
    git(
        &maintainer,
        &["commit", "-q", "--allow-empty", "-m", "Unpushed"],
    );
    let status_before = git(&maintainer, &["status", "--porcelain"]);
    let branches_before = git(&maintainer, &["branch", "--list"]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.pr_head("release-0.2.0");
    let diff = release.origin(&["diff", "--name-only", &format!("{head}^"), &head]);
    assert_eq!(diff, "Cargo.lock\nCargo.toml");
    assert!(
        !release
            .origin(&["log", "--format=%s", "main"])
            .contains("Unpushed")
    );

    assert_eq!(
        git(&maintainer, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "wip"
    );
    assert_eq!(git(&maintainer, &["status", "--porcelain"]), status_before);
    assert_eq!(
        fs::read_to_string(maintainer.join("Cargo.toml")).unwrap(),
        "local edit\n"
    );
    assert_eq!(git(&maintainer, &["branch", "--list"]), branches_before);
    assert_eq!(git(&maintainer, &["worktree", "list"]).lines().count(), 1);
}

#[test]
#[cfg(target_os = "linux")]
fn a_failure_to_generate_the_notes_stops_before_the_pr_is_opened() {
    let release = Release::new();
    release.ci_reports(GREEN);
    release.gh(&[
        "fake",
        "fails",
        "api repos/{owner}/{repo}/releases/generate-notes",
    ]);

    let output = release.run_script("0.2.0");

    assert!(!output.status.success());
    let prs = release.gh_state()["prs"].clone();
    assert!(
        prs.as_array()
            .unwrap()
            .iter()
            .all(|pr| pr["head"] != "release-0.2.0"),
        "{prs}"
    );
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

/// Runs the script with `version` and asserts it refused before pushing
/// anything, with a message on stderr containing each of `reasons`.
fn assert_refused(release: &Release, version: &str, reasons: &[&str]) {
    let refs_before = release.origin_refs();
    let prs_before = release.gh_state()["prs"].clone();

    let output = release.run_script(version);

    assert!(!output.status.success(), "{version} was not refused");
    let err = stderr(&output);
    assert!(err.contains("refusing to release"), "{version}: {err}");
    for reason in reasons {
        assert!(err.contains(reason), "{version}: no {reason:?} in {err}");
    }
    assert_eq!(release.origin_refs(), refs_before, "{version}");
    assert_eq!(release.gh_state()["prs"], prs_before, "{version}");
}

#[test]
#[cfg(target_os = "linux")]
fn a_version_that_is_not_plain_semver_is_refused() {
    let release = Release::new();

    for version in [
        "v0.4.0",
        "0.4",
        "abc",
        "0.4.0-rc.1",
        "01.4.0",
        "0.4.0.1",
        "",
    ] {
        assert_refused(&release, version, &["not a plain X.Y.Z version"]);
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_version_not_higher_than_the_one_on_main_is_refused() {
    let release = Release::new();

    for version in ["0.1.0", "0.0.9", "0.0.10"] {
        assert_refused(&release, version, &["not higher than 0.1.0 on main"]);
    }
}

#[test]
#[cfg(target_os = "linux")]
fn versions_are_compared_as_numbers_not_text() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script("0.10.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(release.origin_tag("v0.10.0").is_some());
}

#[test]
#[cfg(target_os = "linux")]
fn a_tag_that_exists_locally_is_refused() {
    let release = Release::new();
    git(&release.maintainer(), &["tag", "v0.2.0"]);

    assert_refused(&release, "0.2.0", &["v0.2.0 already exists locally"]);
}

#[test]
#[cfg(target_os = "linux")]
fn a_tag_that_exists_on_origin_is_refused() {
    let release = Release::new();
    // On a commit main never reaches, so fetching main doesn't bring it in.
    let contributor = release.contributor();
    git(
        &contributor,
        &["checkout", "-q", "-b", "stray", "origin/main"],
    );
    git(
        &contributor,
        &["commit", "-q", "--allow-empty", "-m", "Stray"],
    );
    git(&contributor, &["tag", "v0.2.0"]);
    git(&contributor, &["push", "-q", "origin", "v0.2.0"]);

    assert_refused(&release, "0.2.0", &["v0.2.0 already exists on origin"]);
    assert!(git(&release.maintainer(), &["tag", "--list", "v0.2.0"]).is_empty());
}

#[test]
#[cfg(target_os = "linux")]
fn a_tag_on_main_that_exists_on_origin_is_refused_as_on_origin() {
    let release = Release::new();
    let contributor = release.contributor();
    git(&contributor, &["tag", "v0.2.0", "origin/main"]);
    git(&contributor, &["push", "-q", "origin", "v0.2.0"]);

    assert_refused(&release, "0.2.0", &["v0.2.0 already exists on origin"]);
}

#[test]
#[cfg(target_os = "linux")]
fn a_red_ci_run_on_main_is_refused() {
    let release = Release::new();
    release.main_ci("completed", "failure");

    assert_refused(&release, "0.2.0", &["CI on main", "failure"]);
}

#[test]
#[cfg(target_os = "linux")]
fn an_unfinished_ci_run_on_main_is_refused() {
    let release = Release::new();
    release.main_ci("in_progress", "");

    assert_refused(&release, "0.2.0", &["CI on main", "in_progress"]);
}

#[test]
#[cfg(target_os = "linux")]
fn a_main_whose_tip_ci_has_not_run_on_is_refused() {
    let release = Release::new();
    release.merge_pr("untested", "Not yet built");

    assert_refused(&release, "0.2.0", &["CI has not run on main at"]);
}
