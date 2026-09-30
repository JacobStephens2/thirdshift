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
/// merged before the tag and one after; a contributor's clone that makes
/// them; the maintainer's clone the script runs in; and the fake `gh`'s state.
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

impl Release {
    /// Pushes a `release-<version>` branch off main with the bump commit the
    /// script makes, as a run interrupted after pushing it leaves it, and
    /// returns that commit.
    fn push_bump(&self, version: &str) -> String {
        let contributor = self.contributor();
        let branch = format!("release-{version}");
        git(&contributor, &["fetch", "-q", "origin"]);
        git(
            &contributor,
            &["checkout", "-q", "-b", &branch, "origin/main"],
        );
        for file in ["Cargo.toml", "Cargo.lock"] {
            fs::write(contributor.join(file), with_version(file, version)).unwrap();
        }
        git(
            &contributor,
            &["commit", "-q", "-a", "-m", &format!("Release {version}")],
        );
        git(&contributor, &["push", "-q", "origin", &branch]);
        let head = git(&contributor, &["rev-parse", "HEAD"]);
        git(&contributor, &["checkout", "-q", "--detach", "origin/main"]);
        head
    }

    /// Opens the bump PR for `version`'s pushed branch.
    fn open_bump_pr(&self, version: &str) {
        self.gh(&[
            "pr",
            "create",
            "--head",
            &format!("release-{version}"),
            "--base",
            "main",
            "--title",
            &format!("Release {version}"),
            "--body",
            "Summary from the first run",
        ]);
    }

    /// Merges the bump PR at `head` with a merge commit and returns it.
    fn merge_bump_pr(&self, version: &str, head: &str) -> String {
        self.gh(&[
            "pr",
            "merge",
            &format!("release-{version}"),
            "--merge",
            "--match-head-commit",
            head,
        ]);
        self.origin(&["rev-parse", "main"])
    }

    fn prs_from(&self, head: &str) -> usize {
        self.gh_state()["prs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|pr| pr["head"] == head)
            .count()
    }

    /// Every ref on origin with the commit it names.
    fn origin_refs(&self) -> String {
        self.origin(&["for-each-ref", "--format=%(refname) %(objectname)"])
    }

    fn gh_calls(&self, subcommand: &[&str]) -> usize {
        let path = self.root().join("gh-calls.json");
        let calls: Value = fs::read_to_string(path)
            .map(|text| serde_json::from_str(&text).unwrap())
            .unwrap_or(json!([]));
        calls
            .as_array()
            .unwrap()
            .iter()
            .filter(|call| {
                subcommand
                    .iter()
                    .enumerate()
                    .all(|(i, word)| call[i] == *word)
            })
            .count()
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_after_the_branch_was_pushed_opens_its_pr_then_merges_and_tags() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.prs_from("release-0.2.0"), 1);
    assert_eq!(release.pr_head("release-0.2.0"), head);
    let body = release.pr("release-0.2.0")["body"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(body.contains("Add the frobnicator"), "{body}");
    assert!(
        body.contains("-version = \"0.1.0\"\n+version = \"0.2.0\""),
        "{body}"
    );
    let merge = release.origin(&["rev-parse", "main"]);
    assert_eq!(release.origin(&["rev-parse", &format!("{merge}^2")]), head);
    assert_eq!(release.origin_tag("v0.2.0"), Some(merge));
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_after_the_pr_was_opened_merges_and_tags_without_a_second_pr_or_bump() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.ci_reports(GREEN);
    let creates_before = release.gh_calls(&["pr", "create"]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.prs_from("release-0.2.0"), 1);
    assert_eq!(release.gh_calls(&["pr", "create"]), creates_before);
    let pr = release.pr("release-0.2.0");
    assert_eq!(pr["state"], "MERGED");
    assert_eq!(pr["body"], "Summary from the first run");
    assert_eq!(release.pr_head("release-0.2.0"), head);
    let merge = release.origin(&["rev-parse", "main"]);
    assert_eq!(release.origin(&["rev-parse", &format!("{merge}^2")]), head);
    assert_eq!(release.origin_tag("v0.2.0"), Some(merge));
    let err = stderr(&output);
    assert!(
        err.contains("release: waiting for CI on https://github.com/"),
        "{err}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_after_the_merge_tags_the_merge_commit_and_pushes_the_tag() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    let merge = release.merge_bump_pr("0.2.0", &head);
    release.merge_pr("later", "Merged after the release");
    let merges_before = release.gh_calls(&["pr", "merge"]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(merge.clone()));
    assert_eq!(release.prs_from("release-0.2.0"), 1);
    assert_eq!(release.gh_calls(&["pr", "merge"]), merges_before);
    let err = stderr(&output);
    assert!(
        err.contains(&format!(
            "release: tagged v0.2.0 on {} and pushed the tag",
            &merge[..7]
        )),
        "{err}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_once_the_tag_is_on_the_merged_bump_reports_it_and_changes_nothing() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    let merge = release.merge_bump_pr("0.2.0", &head);
    release.origin(&["tag", "v0.2.0", &merge]);
    let refs_before = release.origin_refs();
    let state_before = release.gh_state();

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("v0.2.0 is already tagged"), "{err}");
    assert_eq!(release.origin_refs(), refs_before);
    assert_eq!(release.gh_state(), state_before);
}

#[test]
#[cfg(target_os = "linux")]
fn a_tag_that_is_not_on_the_merged_bump_is_refused() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    let merge = release.merge_bump_pr("0.2.0", &head);
    release.origin(&["tag", "v0.2.0", &format!("{merge}^1")]);
    let refs_before = release.origin_refs();

    let output = release.run_script("0.2.0");

    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("v0.2.0 already exists"), "{err}");
    assert_eq!(release.origin_refs(), refs_before);
}

#[test]
#[cfg(target_os = "linux")]
fn a_tag_with_no_bump_pr_is_refused() {
    let release = Release::new();
    release.origin(&["tag", "v0.2.0", "main"]);
    let refs_before = release.origin_refs();

    let output = release.run_script("0.2.0");

    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("v0.2.0 already exists"), "{err}");
    assert_eq!(release.origin_refs(), refs_before);
    assert_eq!(release.prs_from("release-0.2.0"), 0);
}

#[test]
#[cfg(target_os = "linux")]
fn a_bump_pr_closed_without_merging_is_refused() {
    let release = Release::new();
    release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.gh(&["fake", "pr", "release-0.2.0", "state", "\"CLOSED\""]);
    let refs_before = release.origin_refs();

    let output = release.run_script("0.2.0");

    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("closed without merging"), "{err}");
    assert_eq!(release.origin_refs(), refs_before);
    assert_eq!(release.prs_from("release-0.2.0"), 1);
}
