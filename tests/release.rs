//! The maintainer's release script, `scripts/release.sh`, run as a black box
//! against a bare local origin with the fake `gh` and `claude` on PATH and
//! the user's git configuration kept out, as the site deploy tests run the
//! publish script.
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
/// merged before the tag and two after; a contributor's clone that makes
/// them; the maintainer's clone the script runs in; the fake `gh`'s state;
/// and a fake `claude` that prints `AGENT_SUMMARY`.
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

        release.merge_pr("before-tag", "Shipped in 0.1.0", "Old news.");
        git(&contributor, &["pull", "-q", "origin", "main"]);
        git(&contributor, &["tag", "v0.1.0"]);
        git(&contributor, &["push", "-q", "origin", "v0.1.0"]);
        release.merge_pr(
            "after-tag",
            "Add the frobnicator",
            "It frobs.\n\nCloses #7.",
        );
        release.merge_pr("site", "Tidy the site", "");
        release.agent_runs(&format!("printf '%s\\n' '{AGENT_SUMMARY}'"));

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

    /// Opens a PR titled `title` with `body` from a new branch `head` off main
    /// and merges it with a merge commit, through the fake `gh`.
    fn merge_pr(&self, head: &str, title: &str, body: &str) {
        let contributor = self.contributor();
        git(&contributor, &["checkout", "-q", "-b", head, "origin/main"]);
        fs::write(contributor.join(format!("{head}.txt")), head).unwrap();
        git(&contributor, &["add", "-A"]);
        git(&contributor, &["commit", "-q", "-m", title]);
        git(&contributor, &["push", "-q", "origin", head]);
        let sha = git(&contributor, &["rev-parse", "HEAD"]);
        self.gh(&[
            "pr", "create", "--head", head, "--base", "main", "--title", title, "--body", body,
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

    /// Makes the fake `claude` run the bash `script`.
    fn agent_runs(&self, script: &str) {
        fs::write(self.root().join("claude-script.sh"), script).unwrap();
    }

    /// The fake `claude`'s calls, oldest first.
    fn agent_calls(&self) -> Vec<Value> {
        let path = self.root().join("claude-calls.json");
        if !path.exists() {
            return Vec::new();
        }
        serde_json::from_str::<Value>(&fs::read_to_string(path).unwrap())
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
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
            .env("FAKE_CLAUDE_SCRIPT", self.root().join("claude-script.sh"))
            .env("FAKE_CLAUDE_RECORD", self.root().join("claude-calls.json"))
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

    /// Runs the Release notes script for `tag` in the maintainer's clone,
    /// once it has fetched the tags, with `INSTALL_NOTES` as dist's install
    /// instructions.
    fn run_notes_script(&self, tag: &str) -> Output {
        git(&self.maintainer(), &["fetch", "-q", "--tags", "origin"]);
        self.command(
            &manifest_dir().join("scripts/release-notes.sh"),
            &self.maintainer(),
        )
        .args([tag, INSTALL_NOTES])
        .output()
        .expect("could not run release-notes.sh")
    }

    /// GitHub's generated notes for `tag`, from the fake `gh`.
    fn generated_notes(&self, tag: &str) -> String {
        self.gh(&[
            "api",
            "repos/{owner}/{repo}/releases/generate-notes",
            "-f",
            &format!("tag_name={tag}"),
            "--jq",
            ".body",
        ])
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

    fn pr_body(&self, head: &str) -> String {
        self.pr(head)["body"].as_str().unwrap().to_owned()
    }

    fn origin_tag(&self, tag: &str) -> Option<String> {
        let refs = self.origin(&["tag", "--list", tag]);
        (!refs.is_empty()).then(|| self.origin(&["rev-parse", &format!("{tag}^{{commit}}")]))
    }
}

/// The summary section of a bump PR's `body`, between its markers, and what
/// follows it.
fn summary_and_rest(body: &str) -> (&str, &str) {
    let start_marker = "<!-- release-summary:start -->\n";
    let start = body.find(start_marker).expect(body) + start_marker.len();
    let end = body.find("<!-- release-summary:end -->").expect(body);
    (&body[start..end], &body[end..])
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const GREEN: &str = r#"[{"name": "test", "conclusion": "success", "pending_polls": 2}]"#;
const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;
const AGENT_SUMMARY: &str = "The headline is the frobnicator (#2).";
const INSTALL_NOTES: &str =
    "## Install thirdshift\n\n```sh\ncurl -LsSf https://example.com/install.sh | sh\n```";

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
fn the_agent_gets_the_prompt_file_then_the_version_diff_and_the_prs_merged_since_the_last_tag() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let main_before = release.origin(&["rev-parse", "main"]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let calls = release.agent_calls();
    assert_eq!(calls.len(), 1, "{calls:#?}");
    let head = release.pr_head("release-0.2.0");
    let diff = release.origin(&["diff", "--no-color", "--unified=1", &main_before, &head]);
    let prompt = fs::read_to_string(manifest_dir().join("scripts/release-summary.md")).unwrap();
    let expected = format!(
        "{prompt}
## Version diff

```diff
{diff}
```

## Pull requests merged since v0.1.0

### #2: Add the frobnicator

It frobs.

Closes #7.

### #3: Tidy the site
"
    );
    assert_eq!(calls[0]["stdin"].as_str().unwrap(), expected);
}

#[test]
#[cfg(target_os = "linux")]
fn the_agent_runs_in_print_mode_with_tools_disabled() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let argv: Vec<String> = release.agent_calls()[0]["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_owned())
        .collect();
    assert!(argv.contains(&"-p".to_owned()), "{argv:?}");
    let tools = argv
        .iter()
        .position(|arg| arg == "--tools")
        .expect("no --tools");
    assert_eq!(argv[tools + 1], "", "{argv:?}");
    assert!(argv.contains(&"--strict-mcp-config".to_owned()), "{argv:?}");
}

#[test]
#[cfg(target_os = "linux")]
fn the_pr_body_has_the_agents_summary_in_a_marked_section_then_the_version_diff() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let body = release.pr_body("release-0.2.0");
    let (summary, rest) = summary_and_rest(&body);
    assert_eq!(summary, format!("{AGENT_SUMMARY}\n"), "{body}");
    assert!(rest.contains("## Version diff\n\n```diff"), "{body}");
    assert!(
        rest.contains("-version = \"0.1.0\"\n+version = \"0.2.0\""),
        "{body}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn when_the_agent_fails_or_prints_nothing_the_summary_is_the_generated_notes() {
    for (case, script) in [
        ("fails", "echo 'Not logged in' >&2; exit 1"),
        ("prints nothing", "printf '\\n'"),
    ] {
        let release = Release::new();
        release.ci_reports(GREEN);
        release.agent_runs(script);

        let output = release.run_script("0.2.0");

        assert!(output.status.success(), "{case}: {}", stderr(&output));
        let err = stderr(&output);
        assert!(
            err.lines()
                .any(|line| line.starts_with("release: warning:") && line.contains("agent summary")),
            "{case}: {err}"
        );
        let body = release.pr_body("release-0.2.0");
        let (summary, rest) = summary_and_rest(&body);
        let (note, notes) = summary.split_once("\n\n").expect(&body);
        assert!(
            !note.contains('\n') && note.contains("agent summary was unavailable"),
            "{case}: {body}"
        );
        assert!(notes.starts_with("## What's Changed\n"), "{case}: {body}");
        assert!(notes.contains("Add the frobnicator"), "{case}: {body}");
        assert!(!notes.contains("Shipped in 0.1.0"), "{case}: {body}");
        assert!(rest.contains("```diff"), "{case}: {body}");
        let merge = release.origin(&["rev-parse", "main"]);
        assert_eq!(release.origin_tag("v0.2.0"), Some(merge), "{case}");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn each_step_prints_a_progress_line_on_stderr() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let merge = release.origin(&["rev-parse", "main"]);
    let url = format!("https://github.com/{REPO}/pull/4");
    let expected = [
        "release: writing the summary with claude".to_owned(),
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
    release.agent_runs("exit 1");
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

#[test]
#[cfg(target_os = "linux")]
fn the_release_body_is_the_bump_prs_summary_then_the_generated_notes_then_the_install_instructions()
{
    let release = Release::new();
    release.ci_reports(GREEN);
    let output = release.run_script("0.2.0");
    assert!(output.status.success(), "{}", stderr(&output));

    let first = release.run_notes_script("v0.2.0");
    let again = release.run_notes_script("v0.2.0");

    assert!(first.status.success(), "{}", stderr(&first));
    let body = String::from_utf8(first.stdout).unwrap();
    let notes = release.generated_notes("v0.2.0");
    assert!(notes.contains("Add the frobnicator"), "{notes}");
    assert_eq!(
        body,
        format!("{AGENT_SUMMARY}\n\n{notes}\n\n{INSTALL_NOTES}\n")
    );
    assert!(again.status.success(), "{}", stderr(&again));
    assert_eq!(String::from_utf8(again.stdout).unwrap(), body);
}

#[test]
#[cfg(target_os = "linux")]
fn without_a_bump_pr_summary_the_release_body_is_the_generated_notes_and_install_instructions() {
    let release = Release::new();
    // A PR with a summary section whose head commit, not its merge, is tagged.
    release.merge_pr(
        "summarised",
        "Summarised",
        "<!-- release-summary:start -->\nNot this release.\n<!-- release-summary:end -->",
    );
    let contributor = release.contributor();
    git(&contributor, &["tag", "v0.1.2", "origin/summarised"]);
    // A commit pushed straight to main, not through a PR.
    fs::write(contributor.join("hand.txt"), "by hand").unwrap();
    git(&contributor, &["add", "-A"]);
    git(&contributor, &["commit", "-q", "-m", "By hand"]);
    git(&contributor, &["push", "-q", "origin", "HEAD:main"]);
    git(&contributor, &["tag", "v0.1.3"]);
    git(&contributor, &["push", "-q", "origin", "v0.1.2", "v0.1.3"]);

    for (case, tag) in [
        ("the merge of a PR without a summary section", "v0.1.0"),
        ("the head of a PR, not its merge", "v0.1.2"),
        ("a commit with no PR", "v0.1.3"),
    ] {
        let output = release.run_notes_script(tag);

        assert!(output.status.success(), "{case}: {}", stderr(&output));
        let notes = release.generated_notes(tag);
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{notes}\n\n{INSTALL_NOTES}\n"),
            "{case}"
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn when_the_bump_pr_cannot_be_read_the_release_body_is_the_generated_notes_with_a_warning() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let output = release.run_script("0.2.0");
    assert!(output.status.success(), "{}", stderr(&output));
    let merge = release.origin(&["rev-parse", "main"]);
    release.gh(&[
        "fake",
        "fails",
        &format!("api repos/{{owner}}/{{repo}}/commits/{merge}/pulls"),
    ]);

    let output = release.run_notes_script("v0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(
        err.lines()
            .any(|line| line.starts_with("release-notes: warning:") && line.contains("summary")),
        "{err}"
    );
    let notes = release.generated_notes("v0.2.0");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{notes}\n\n{INSTALL_NOTES}\n")
    );
}
