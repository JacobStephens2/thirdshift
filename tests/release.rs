//! The maintainer's release script, `scripts/release.sh`, run as a black box
//! against a bare local origin with the fake `gh` and Harness CLIs on PATH and
//! the user's git configuration kept out, as the site deploy tests run the
//! publish script. Where a step has several cases, one goes through the whole
//! script and the rest call the step's function, with the script sourced.
//!
//! The script, like the fake `gh` it drives here, relies on GNU tools, so its
//! tests run on Linux only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod support;

#[cfg(target_os = "linux")]
#[path = "release/lifetime.rs"]
mod lifetime;

#[cfg(target_os = "linux")]
#[path = "release/experiments.rs"]
mod experiments;

use std::fs;
#[cfg(not(target_os = "linux"))]
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "linux"))]
use std::process::Stdio;
use std::process::{Command, Output};

use serde_json::{Value, json};
use support::write_executable;
use tempfile::TempDir;

const REPO: &str = "JacobStephens2/thirdshift";

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Keeps the machine's git configuration (signing, hooks, default branch) out
/// of the test's git, the script's and the fake `gh`'s.
fn isolated(command: &mut Command) -> &mut Command {
    // Git's inherited -c parameters override GIT_CONFIG_COUNT too. Append to
    // them so unrelated entries survive and inherited conflicts cannot turn
    // maintenance back on when the shared harness's gitconfig is excluded.
    let mut parameters = command
        .get_envs()
        .find(|(name, _)| *name == "GIT_CONFIG_PARAMETERS")
        .map(|(_, value)| value.unwrap_or_default().to_os_string())
        .unwrap_or_else(|| std::env::var_os("GIT_CONFIG_PARAMETERS").unwrap_or_default());
    if !parameters.is_empty() {
        parameters.push(" ");
    }
    parameters.push("'maintenance.auto=false' 'gc.auto=0'");
    command
        .env("GIT_CONFIG_PARAMETERS", parameters)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
}

#[test]
fn isolated_git_disables_automatic_maintenance() {
    let dir = TempDir::new().unwrap();
    for (kind, key, expected) in [
        ("bool", "maintenance.auto", "false\n"),
        ("int", "gc.auto", "0\n"),
    ] {
        let mut command = Command::new("git");
        command
            .current_dir(dir.path())
            .env("GIT_CONFIG_COUNT", "0")
            .env_remove("GIT_CONFIG_PARAMETERS");
        let output = isolated(&mut command)
            .args(["config", &format!("--type={kind}"), "--get", key])
            .output()
            .unwrap();
        assert!(output.status.success(), "{key}: {output:?}");
        assert_eq!(stdout(&output), expected, "{key}");
    }
}

#[test]
fn isolated_git_descendants_preserve_configuration_and_identity() {
    const CHILD: &str = "THIRDSHIFT_RELEASE_ISOLATION_TEST";
    if std::env::var_os(CHILD).is_some() {
        for (parameters, expected) in [
            (None, "inherited parameter"),
            (
                Some(
                    "'thirdshift.parameters=command parameter' 'maintenance.auto=true' 'gc.auto=42'",
                ),
                "command parameter",
            ),
        ] {
            let mut command = Command::new("bash");
            command.args([
                "-c",
                r#"set -eu
test "$GIT_CONFIG_GLOBAL" = /dev/null
test "$GIT_CONFIG_NOSYSTEM" = 1
git config --type=bool --get maintenance.auto
git config --type=int --get gc.auto
git config --get-all thirdshift.count
git config --get thirdshift.parameters
if git config --get thirdshift.host; then exit 1; fi
git var GIT_AUTHOR_IDENT
git var GIT_COMMITTER_IDENT
"#,
            ]);
            if let Some(parameters) = parameters {
                command.env("GIT_CONFIG_PARAMETERS", parameters);
            }
            let output = isolated(&mut command).output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let text = stdout(&output);
            let lines: Vec<_> = text.lines().collect();
            assert_eq!(
                &lines[..5],
                [
                    "false",
                    "0",
                    "first count entry",
                    "second count entry",
                    expected
                ]
            );
            assert_eq!(lines.len(), 7, "{text}");
            for identity in &lines[5..] {
                assert!(identity.starts_with("Test <test@example.com> "), "{text}");
            }
        }
        return;
    }

    // Re-exec so isolated() reads real inherited entries, without mutating
    // the environment of other tests in this process.
    let dir = TempDir::new().unwrap();
    let host_config = dir.path().join("host.gitconfig");
    fs::write(&host_config, "[thirdshift]\n\thost = must stay excluded\n").unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "isolated_git_descendants_preserve_configuration_and_identity",
            "--nocapture",
        ])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env(CHILD, "1")
        .env("GIT_CONFIG_GLOBAL", &host_config)
        .env("GIT_CONFIG_SYSTEM", &host_config)
        .env("GIT_CONFIG_COUNT", "4")
        .env("GIT_CONFIG_KEY_0", "maintenance.auto")
        .env("GIT_CONFIG_VALUE_0", "true")
        .env("GIT_CONFIG_KEY_1", "gc.auto")
        .env("GIT_CONFIG_VALUE_1", "42")
        .env("GIT_CONFIG_KEY_2", "thirdshift.count")
        .env("GIT_CONFIG_VALUE_2", "first count entry")
        .env("GIT_CONFIG_KEY_3", "thirdshift.count")
        .env("GIT_CONFIG_VALUE_3", "second count entry")
        .env(
            "GIT_CONFIG_PARAMETERS",
            "'thirdshift.parameters=inherited parameter' 'maintenance.auto=true' 'gc.auto=42'",
        )
        .env("GIT_AUTHOR_NAME", "Host")
        .env("GIT_AUTHOR_EMAIL", "host@example.com")
        .env("GIT_COMMITTER_NAME", "Host")
        .env("GIT_COMMITTER_EMAIL", "host@example.com")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum BumpChange {
    AppendComment(&'static str),
    AddedFile,
    FileMode,
    WrongVersion,
}

/// A bare origin whose main is at version 0.1.0, tagged `v0.1.0`, with one PR
/// merged before the tag and two after, and a green CI run on main's tip; a
/// contributor's clone that makes them; the maintainer's clone the script
/// runs in; the fake `gh`'s state, where each bump PR merged starts a release
/// workflow run that succeeds; and a fake `claude` that prints
/// `AGENT_SUMMARY`.
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
        support::fakes::install(&release.bin());
        // Execute the built binary through the script's Cargo interface, so
        // release tests do not rebuild it or depend on the machine's rustup.
        let cargo = release.bin().join("cargo");
        write_executable(
            &cargo,
            "#!/bin/sh\nwhile [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$THIRDSHIFT_TEST_BINARY\" \"$@\"\n",
        );
        fs::create_dir(root.join("home")).unwrap();
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
        release.agent_runs(&format!(
            "printf '%s' '{AGENT_SUMMARY}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\""
        ));

        git(root, &["clone", "-q", "origin.git", "maintainer"]);
        release.main_ci("completed", "success");
        release.release_workflow(json!({}));
        release
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    /// Where the fakes are.
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

    /// Records a CI run on main's tip on origin, newer than any before it.
    fn main_ci(&self, status: &str, conclusion: &str) {
        self.main_ci_on(&self.origin(&["rev-parse", "main"]), status, conclusion, 0);
    }

    /// Makes each tag push record a release workflow run on the bump commit,
    /// and records one when preparing a merged-resume fixture: a run that
    /// succeeded, with `fields` set over it.
    fn release_workflow(&self, fields: Value) {
        let mut run = json!({
            "workflow": "release.yml",
            "event": "push",
            "branch": "@TAG@",
            "headSha": "@SHA@",
            "status": "completed",
            "conclusion": "success",
            "jobs": [{"name": "host", "conclusion": "success"}],
        });
        run.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let script = format!(
            "case $FAKE_MERGE_HEAD in release-*)\n\
             run='{run}'\n\
             origin=${{FAKE_GH_STATE%/*}}/origin.git\n\
             if [ -e \"$FAKE_GH_STATE.tag-pushed\" ]; then exit 0; fi\n\
             sha=$(git --git-dir=\"$origin\" rev-parse refs/heads/\"$FAKE_MERGE_HEAD\")\n\
             run=${{run//@SHA@/$sha}}\n\
             gh fake run \"${{run//@TAG@/v${{FAKE_MERGE_HEAD#release-}}}}\"\n\
             esac"
        );
        self.gh(&["fake", "after-merge", &script]);
        write_executable(
            &self.bin().join("git"),
            &format!(
                r#"#!/bin/bash
set -euo pipefail
command -p git "$@"
if [ "$1" = push ]; then
    for argument in "$@"; do
        case $argument in
        *:refs/tags/v*)
            sha=${{argument%:refs/tags/*}}
            tag=${{argument#*:refs/tags/}}
            run='{run}'
            run=${{run//@SHA@/$sha}}
            gh fake run "${{run//@TAG@/$tag}}"
            touch "$FAKE_GH_STATE.tag-pushed"
            ;;
        esac
    done
fi
"#
            ),
        );
    }

    /// The newest release workflow run recorded.
    fn release_run(&self) -> Value {
        self.gh_state()["runs"]
            .as_array()
            .unwrap()
            .iter()
            .rfind(|run| run["workflow"] == "release.yml")
            .expect("no release workflow run")
            .clone()
    }

    /// Records a CI run on main at `sha`, newer than any before it, pending
    /// for `pending_polls` reads before reporting its final status.
    fn main_ci_on(&self, sha: &str, status: &str, conclusion: &str, pending_polls: u32) {
        let run = json!({
            "branch": "main",
            "workflow": "ci.yml",
            "event": "push",
            "headSha": sha,
            "status": status,
            "conclusion": conclusion,
            "pending_polls": pending_polls,
        });
        self.gh(&["fake", "run", &run.to_string()]);
    }

    /// Moves origin's main just after the first CI listing, so the script
    /// must fetch again before using that listing to start the release.
    fn move_main_after_ci_read(&self, sha: &str) {
        let unwrapped = self.bin().join("unwrapped");
        fs::create_dir(&unwrapped).unwrap();
        let gh = self.bin().join("gh");
        fs::rename(&gh, unwrapped.join("gh")).unwrap();
        write_executable(
            &gh,
            &format!(
                r#"#!/bin/bash
set -euo pipefail
"$(dirname "$0")/unwrapped/gh" "$@"
if [ "$1 $2" = 'run list' ] && [[ " $* " == *' --workflow ci.yml '* ]] &&
    [ ! -e "$FAKE_GH_STATE.main-moved" ]; then
    touch "$FAKE_GH_STATE.main-moved"
    git -C "${{FAKE_GH_STATE%/*}}/origin.git" update-ref refs/heads/main {sha}
fi
"#
            ),
        );
    }

    /// Arranges a merge during the first tip's pending CI, with CI on the
    /// new tip pending too before it reaches `conclusion`.
    fn main_moves_while_waiting(&self, conclusion: &str) -> (String, String) {
        let old = self.origin(&["rev-parse", "main"]);
        self.merge_pr("incoming", "Merge while releasing", "");
        let new = self.origin(&["rev-parse", "main"]);
        self.origin(&["update-ref", "refs/heads/main", &old]);
        self.main_ci_on(&new, "completed", conclusion, 20);
        // An older commit's run can be listed newest; select CI by commit.
        self.main_ci_on(&old, "completed", "success", 3);
        self.move_main_after_ci_read(&new);
        (old, new)
    }

    /// Every ref on origin with the commit it points at, to show a refusal
    /// pushed nothing.
    fn origin_refs(&self) -> String {
        self.origin(&["for-each-ref", "--format=%(refname) %(objectname)"])
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

    /// Calls that attempted to merge a PR, oldest first.
    fn merge_calls(&self) -> Vec<Vec<String>> {
        let path = self.root().join("gh-calls.json");
        if !path.exists() {
            return Vec::new();
        }
        let calls: Vec<Vec<String>> =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        calls
            .into_iter()
            .filter(|call| call.starts_with(&["pr".into(), "merge".into()]))
            .collect()
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
            .env("HOME", self.root().join("home"))
            .env("XDG_DATA_HOME", self.root().join("home/.local/share"))
            .env("THIRDSHIFT_TEST_BINARY", env!("CARGO_BIN_EXE_thirdshift"))
            .env("FAKE_GH_STATE", self.root().join("gh-state.json"))
            .env("FAKE_GH_RECORD", self.root().join("gh-calls.json"))
            .env("FAKE_CLAUDE_SCRIPT", self.root().join("claude-script.sh"))
            .env("FAKE_CLAUDE_RECORD", self.root().join("claude-calls.json"))
            .env("FAKE_CODEX_RECORD", self.root().join("codex-calls.json"))
            .env("FAKE_AGY_RECORD", self.root().join("agy-calls.json"))
            .env("FAKE_AGY_CHECK_RECORD", self.root().join("agy-checks.json"))
            .env("FAKE_GROK_RECORD", self.root().join("grok-calls.json"))
            .env("FAKE_MUSE_RECORD", self.root().join("muse-calls.json"))
            .env(
                "FAKE_OPENCODE_RECORD",
                self.root().join("opencode-calls.json"),
            )
            .env("RELEASE_POLL_SECONDS", "0");
        command
    }

    fn run_script(&self, version: &str) -> Output {
        self.run_script_with(&[version], "", &[])
    }

    /// Runs the script with `args`, with `stdin` on its standard input and
    /// `env` added to its environment.
    fn run_script_with(&self, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Output {
        with_input(
            self.command(
                &manifest_dir().join("scripts/release.sh"),
                &self.maintainer(),
            )
            .args(args)
            .envs(env.iter().copied()),
            stdin,
        )
    }

    /// Sources the script in the maintainer's clone, with what `run_script`
    /// runs it with, then runs the bash `code`, which sets what the earlier
    /// steps would have and calls its functions.
    fn with_functions(&self, code: &str) -> Output {
        with_input(
            self.command(Path::new("bash"), &self.maintainer())
                .arg("-c")
                .arg(format!("source \"$1\"\n{code}"))
                .arg("bash")
                .arg(manifest_dir().join("scripts/release.sh")),
            "",
        )
    }

    /// Runs the Release notes script for `tag` in the maintainer's clone,
    /// once it has fetched the tags, with `INSTALL_NOTES` as dist's install
    /// instructions.
    fn run_notes_script(&self, tag: &str) -> Output {
        git(&self.maintainer(), &["fetch", "-q", "--tags", "origin"]);
        with_input(
            self.command(
                &manifest_dir().join("scripts/release-notes.sh"),
                &self.maintainer(),
            )
            .args([tag, INSTALL_NOTES]),
            "",
        )
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
        self.prs_from(head)
            .pop()
            .unwrap_or_else(|| panic!("no PR from {head}"))
    }

    /// The head commit of `head`'s newest PR, as it was merged if it was.
    fn pr_head(&self, head: &str) -> String {
        self.pr(head)["headRefOid"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| self.origin(&["rev-parse", &format!("refs/heads/{head}")]))
    }

    fn pr_body(&self, head: &str) -> String {
        self.pr(head)["body"].as_str().unwrap().to_owned()
    }

    fn pr_opened(&self, head: &str) -> bool {
        !self.prs_from(head).is_empty()
    }

    fn origin_branch(&self, branch: &str) -> bool {
        !self.origin(&["branch", "--list", branch]).is_empty()
    }

    fn origin_tag(&self, tag: &str) -> Option<String> {
        let refs = self.origin(&["tag", "--list", tag]);
        (!refs.is_empty()).then(|| self.origin(&["rev-parse", &format!("{tag}^{{commit}}")]))
    }

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

    /// Pushes one more commit to `version`'s pushed bump branch, making
    /// `change`, and returns that commit.
    fn push_change_to_bump(&self, version: &str, change: BumpChange) -> String {
        let contributor = self.contributor();
        let branch = format!("release-{version}");
        git(&contributor, &["checkout", "-q", &branch]);
        match change {
            BumpChange::AddedFile => fs::write(contributor.join("extra.txt"), "extra\n").unwrap(),
            BumpChange::FileMode => {
                fs::set_permissions(
                    contributor.join("Cargo.toml"),
                    fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            BumpChange::WrongVersion => {
                for file in ["Cargo.toml", "Cargo.lock"] {
                    fs::write(contributor.join(file), with_version(file, "0.3.0")).unwrap();
                }
            }
            BumpChange::AppendComment(file) => {
                let path = contributor.join(file);
                let text = fs::read_to_string(&path).unwrap();
                fs::write(&path, format!("{text}\n# Unrelated change\n")).unwrap();
            }
        }
        git(&contributor, &["add", "-A"]);
        git(&contributor, &["commit", "-q", "-m", "Unrelated change"]);
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

    /// The PRs from `head`, oldest first.
    fn prs_from(&self, head: &str) -> Vec<Value> {
        self.gh_state()["prs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|pr| pr["head"] == head)
            .cloned()
            .collect()
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

/// The stderr of `output`, with each progress line's `HH:MM:SS` stamp checked
/// and taken out, so `release: 18:20:32 opened …` reads `release: opened …`.
/// Other lines are kept as they are.
fn unstamped(output: &Output) -> String {
    stderr(output)
        .lines()
        .map(|line| match stamped(line) {
            Some((_, message)) => format!("release: {message}\n"),
            None => format!("{line}\n"),
        })
        .collect()
}

/// The `HH:MM:SS` stamp of progress `line` and the message after it, or
/// `None` if `line` is not a progress line. Panics on a progress line with no
/// stamp.
fn stamped(line: &str) -> Option<(chrono::NaiveTime, &str)> {
    let rest = line.strip_prefix("release: ")?;
    let (stamp, message) = rest
        .split_once(' ')
        .unwrap_or_else(|| panic!("no stamp in {line:?}"));
    assert_eq!(stamp.len(), 8, "no HH:MM:SS stamp in {line:?}");
    let stamp = chrono::NaiveTime::parse_from_str(stamp, "%H:%M:%S")
        .unwrap_or_else(|_| panic!("no HH:MM:SS stamp in {line:?}"));
    Some((stamp, message))
}

#[cfg(target_os = "linux")]
fn with_input(command: &mut Command, input: &str) -> Output {
    support::child_lifetime::FixtureChild::spawn(command, input).wait(support::WAIT_BOUND)
}

#[cfg(not(target_os = "linux"))]
fn with_input(command: &mut Command, input: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("could not start command");
    // A rejected command may close stdin before a large input has been written.
    let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// Sources `script` from `scripts/` without running it, then runs the bash
/// `code`, which calls its functions, in an empty directory. The cases of a
/// step go here, and one case of each through the whole script.
fn with_functions_of(script: &str, code: &str) -> Output {
    let dir = TempDir::new().unwrap();
    with_input(
        isolated(&mut Command::new("bash"))
            .arg("-c")
            .arg(format!("source \"$1\"\n{code}"))
            .arg("bash")
            .arg(manifest_dir().join("scripts").join(script))
            .current_dir(dir.path()),
        "",
    )
}

/// A release workflow run that failed in the Rocky Linux smoke check.
fn failed_release() -> Value {
    json!({
        "conclusion": "failure",
        "jobs": [
            {"name": "plan", "conclusion": "success"},
            {"name": "custom-release-smoke / rocky", "conclusion": "failure"},
            {"name": "host", "conclusion": "skipped"},
        ],
    })
}

/// Asserts the script exited 1 on the failed release workflow run, naming
/// the failed job and the run and how to rerun it, with `v0.2.0` left on
/// `merge`.
fn assert_release_failed(release: &Release, output: &Output, merge: &str) {
    assert_eq!(output.status.code(), Some(1), "{}", stderr(output));
    let err = stderr(output);
    let run = release.release_run();
    assert!(err.contains("custom-release-smoke / rocky"), "{err}");
    assert!(!err.contains("plan"), "{err}");
    assert!(err.contains(run["url"].as_str().unwrap()), "{err}");
    assert!(
        err.contains(&format!("gh run rerun --failed {}", run["databaseId"])),
        "{err}"
    );
    assert_eq!(release.origin_tag("v0.2.0").as_deref(), Some(merge));
}

/// The GitHub Release's URL for v0.2.0, as the fake `gh` gives it.
fn release_url() -> String {
    format!("https://github.com/{REPO}/releases/tag/v0.2.0")
}

/// Sets what `main` would for 0.2.0 and finds the earlier run, as `main`
/// does before anything else.
const FIND_EARLIER_RUN: &str =
    "version=0.2.0 tag=v0.2.0 branch=release-0.2.0\nfetch_main\nfind_earlier_run";

const GREEN: &str = r#"[{"name": "test", "conclusion": "success", "pending_polls": 2}]"#;
const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;
const AGENT_SUMMARY: &str = "The headline is the frobnicator (#2).";
const INSTALL_NOTES: &str =
    "## Install thirdshift\n\n```sh\ncurl -LsSf https://example.com/install.sh | sh\n```";

#[test]
#[cfg(target_os = "linux")]
fn a_release_bumps_only_the_version_tags_the_bump_and_merges_the_pr() {
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
    assert_eq!(release.origin_tag("v0.2.0"), Some(head));
    release.origin(&["merge-base", "--is-ancestor", "v0.2.0", "main"]);
}

#[test]
#[cfg(target_os = "linux")]
fn a_pending_main_tip_releases_the_newest_already_green_commit_without_waiting() {
    let release = Release::new();
    let green = release.origin(&["rev-parse", "main"]);
    release.merge_pr("pending", "Not yet green", "");
    let pending = release.origin(&["rev-parse", "main"]);
    release.main_ci_on(&pending, "completed", "success", 2);
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.pr_head("release-0.2.0");
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), green);
    assert!(!stderr(&output).contains("waiting for CI on main"));
    assert_eq!(release.origin_tag("v0.2.0"), Some(head));
}

#[test]
#[cfg(target_os = "linux")]
fn a_failed_bump_merge_keeps_the_tag_and_finishes_publishing() {
    let release = Release::new();
    release.ci_reports(GREEN);
    release.gh(&["fake", "fails", "pr merge"]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let pr = release.pr("release-0.2.0");
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(
        release.origin_tag("v0.2.0"),
        Some(release.pr_head("release-0.2.0"))
    );
    let err = stderr(&output);
    assert!(err.contains("release still stands"), "{err}");
    assert!(err.contains(pr["url"].as_str().unwrap()), "{err}");
    assert!(err.contains(&release_url()), "{err}");
}

#[test]
#[cfg(target_os = "linux")]
fn main_moving_during_bump_checks_does_not_change_the_tagged_content() {
    let release = Release::new();
    let base = release.origin(&["rev-parse", "main"]);
    release.merge_pr("concurrent", "Arrived during bump checks", "");
    let moved = release.origin(&["rev-parse", "main"]);
    release.origin(&["update-ref", "refs/heads/main", &base]);
    release.gh(&[
        "fake", "on-ci-read", "1",
        &format!("gh fake checks \"$FAKE_CI_SHA\" '{GREEN}'\ngit --git-dir=\"${{FAKE_GH_STATE%/*}}/origin.git\" update-ref refs/heads/main {moved}"),
    ]);
    release.gh(&[
        "fake",
        "after-merge",
        r#"case $FAKE_MERGE_HEAD in release-*)
        test "$(git --git-dir="${FAKE_GH_STATE%/*}/origin.git" rev-parse refs/tags/v0.2.0)" = \
             "$(git --git-dir="${FAKE_GH_STATE%/*}/origin.git" rev-parse refs/heads/release-0.2.0)"
        ;; esac"#,
    ]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.pr_head("release-0.2.0");
    assert_eq!(release.origin_tag("v0.2.0"), Some(head.clone()));
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), base);
    let merge = release.origin(&["rev-parse", "main"]);
    assert_eq!(release.origin(&["rev-parse", &format!("{merge}^1")]), moved);
    release.origin(&["merge-base", "--is-ancestor", "v0.2.0", "main"]);
}

#[test]
#[cfg(target_os = "linux")]
fn the_release_summary_uses_the_user_configs_default_harness_model_and_effort() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let home = release.root().join("home");
    fs::create_dir_all(home.join(".thirdshift")).unwrap();
    fs::write(
        home.join(".thirdshift/config.toml"),
        "[harness]\ndefault = 'codex'\n[harness.codex]\nmodel = 'GPT-6-Luna'\neffort = 'High'\n",
    )
    .unwrap();
    release.agent_runs(&format!(
        "printf '%s' '{AGENT_SUMMARY}' > \"$FAKE_CLAUDE_FINAL_MESSAGE\""
    ));

    let output = release.run_script_with(&["0.2.0"], "", &[("HOME", home.to_str().unwrap())]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        release.agent_calls().is_empty(),
        "Claude must not be called"
    );
    let calls: Value =
        serde_json::from_str(&fs::read_to_string(release.root().join("codex-calls.json")).unwrap())
            .unwrap();
    assert_eq!(calls.as_array().unwrap().len(), 1, "{calls:#?}");
    let argv = calls[0]["argv"].as_array().unwrap();
    assert!(argv.contains(&json!("--skip-git-repo-check")), "{argv:?}");
    // Contributor-editable release input must not reach tools: no bypass, a
    // read-only sandbox, nothing to approve, and no shell, MCP or browser.
    assert!(
        !argv.contains(&json!("--dangerously-bypass-approvals-and-sandbox")),
        "{argv:?}"
    );
    for pair in [
        ["--sandbox", "read-only"],
        ["-c", "approval_policy=\"never\""],
        ["-c", "web_search=\"disabled\""],
        ["-c", "mcp_servers={}"],
        ["--disable", "shell_tool"],
        ["--disable", "unified_exec"],
        ["--disable", "apps"],
        ["--disable", "plugins"],
        ["--disable", "browser_use"],
        ["--disable", "computer_use"],
        ["--disable", "hooks"],
    ] {
        assert!(
            argv.windows(2)
                .any(|window| window == [json!(pair[0]), json!(pair[1])]),
            "{pair:?} missing from {argv:?}"
        );
    }
    assert_ne!(calls[0]["cwd"], release.maintainer().display().to_string());
    assert!(
        argv.windows(2)
            .any(|pair| pair == [json!("-m"), json!("gpt-6-luna")]),
        "{argv:?}"
    );
    assert!(
        argv.contains(&json!("model_reasoning_effort=\"high\"")),
        "{argv:?}"
    );
    assert_eq!(
        summary_and_rest(&release.pr_body("release-0.2.0")).0,
        format!("{AGENT_SUMMARY}\n")
    );
}

#[test]
#[cfg(target_os = "linux")]
fn every_harness_receives_the_complete_release_input_above_the_argument_limit() {
    let prompt = format!("Summarize the release.\n{}", "x".repeat(150_000));
    for harness in ["claude", "codex", "agy", "grok", "muse", "opencode"] {
        let release = Release::new();
        fs::create_dir(release.root().join("home/.thirdshift")).unwrap();
        fs::write(
            release.root().join("home/.thirdshift/config.toml"),
            format!("[harness]\ndefault = '{harness}'\n"),
        )
        .unwrap();

        let output = with_input(
            release
                .command(
                    Path::new(env!("CARGO_BIN_EXE_thirdshift")),
                    &release.maintainer(),
                )
                .arg("--release-summary"),
            &prompt,
        );

        assert!(output.status.success(), "{harness}: {}", stderr(&output));
        assert_eq!(stdout(&output).trim(), AGENT_SUMMARY, "{harness}");
        let calls: Value = serde_json::from_str(
            &fs::read_to_string(release.root().join(format!("{harness}-calls.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(
            calls.as_array().unwrap().last().unwrap()["prompt"],
            prompt,
            "{harness}"
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn every_other_harness_summarizes_without_its_session_permission_bypass() {
    // Release input is contributor-editable, so no summary may run tools
    // that act on the machine, as Claude's runs with `--tools ''`.
    // Each Harness, the session flags it must drop, and the runs it must add.
    type Case = (
        &'static str,
        &'static [&'static str],
        &'static [&'static [&'static str]],
    );
    let cases: [Case; 4] = [
        (
            "agy",
            &["--dangerously-skip-permissions"],
            &[
                &["-p="],
                &["--model", "gemini-3.8-flash"],
                &["--effort", "low"],
                &["--sandbox"],
                &["--disable-slash-commands"],
                &["--print-timeout", "10m"],
            ],
        ),
        (
            "grok",
            &["--always-approve", "off"],
            &[
                &["--tools", "todo_write"],
                &["--disallowed-tools", "search_tool,use_tool"],
                &["--disable-web-search"],
            ],
        ),
        (
            "muse",
            &["--yolo", "--disable-approval", "--disable-sandbox"],
            &[
                &["--disable-shell"],
                &["--disable-write"],
                &["--disable-web-tools"],
                &["--max-model-steps", "1"],
            ],
        ),
        ("opencode", &["--auto"], &[]),
    ];
    for (harness, forbidden, required) in cases {
        let release = Release::new();
        fs::create_dir(release.root().join("home/.thirdshift")).unwrap();
        let mut config = format!("[harness]\ndefault = '{harness}'\n");
        if harness == "agy" {
            config.push_str("[harness.agy]\nmodel = 'gemini-3.8-flash'\neffort = 'low'\n");
        }
        fs::write(release.root().join("home/.thirdshift/config.toml"), config).unwrap();

        let output = with_input(
            release
                .command(
                    Path::new(env!("CARGO_BIN_EXE_thirdshift")),
                    &release.maintainer(),
                )
                .arg("--release-summary"),
            "Summarize the release.",
        );

        assert!(output.status.success(), "{harness}: {}", stderr(&output));
        let calls: Value = serde_json::from_str(
            &fs::read_to_string(release.root().join(format!("{harness}-calls.json"))).unwrap(),
        )
        .unwrap();
        let call = calls.as_array().unwrap().last().unwrap();
        let argv: Vec<&str> = call["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        for flag in forbidden {
            assert!(!argv.contains(flag), "{harness}: {flag} in {argv:?}");
        }
        for run in required {
            assert!(
                argv.windows(run.len()).any(|window| window == *run),
                "{harness}: {run:?} missing from {argv:?}"
            );
        }
        if harness == "opencode" {
            assert_eq!(
                call["config_content"], r#"{"permission":{"*":"deny"}}"#,
                "{call:#?}"
            );
        }
    }
}

#[test]
#[cfg(target_os = "linux")]
fn every_other_configured_harness_supplies_its_final_text_to_the_release() {
    for harness in ["agy", "grok", "muse", "opencode"] {
        let release = Release::new();
        release.ci_reports(GREEN);
        fs::create_dir(release.root().join("home/.thirdshift")).unwrap();
        fs::write(
            release.root().join("home/.thirdshift/config.toml"),
            format!("[harness]\ndefault = '{harness}'\n"),
        )
        .unwrap();

        let output = release.run_script("0.2.0");

        assert!(output.status.success(), "{harness}: {}", stderr(&output));
        assert!(
            release.agent_calls().is_empty(),
            "{harness}: Claude was called"
        );
        assert_eq!(
            summary_and_rest(&release.pr_body("release-0.2.0")).0,
            format!("{AGENT_SUMMARY}\n"),
            "{harness}: {}",
            stderr(&output)
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_failed_or_blank_selected_harness_falls_back_without_trying_claude() {
    for script in [
        "exit 1",
        "printf ' \n\t\n' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"",
    ] {
        let release = Release::new();
        release.ci_reports(GREEN);
        fs::create_dir(release.root().join("home/.thirdshift")).unwrap();
        fs::write(
            release.root().join("home/.thirdshift/config.toml"),
            "[harness]\ndefault = 'codex'\n",
        )
        .unwrap();
        release.agent_runs(script);

        let output = release.run_script("0.2.0");

        assert!(output.status.success(), "{}", stderr(&output));
        assert!(release.agent_calls().is_empty());
        assert!(unstamped(&output).contains("release: warning: the agent summary is unavailable"));
        let body = release.pr_body("release-0.2.0");
        assert!(
            summary_and_rest(&body)
                .0
                .contains("agent summary was unavailable"),
            "{body}"
        );
        assert!(
            summary_and_rest(&body).0.contains("Add the frobnicator"),
            "{body}"
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_bad_user_config_warns_and_uses_generated_notes_without_calling_a_harness() {
    let release = Release::new();
    release.ci_reports(GREEN);
    fs::create_dir(release.root().join("home/.thirdshift")).unwrap();
    fs::write(
        release.root().join("home/.thirdshift/config.toml"),
        "[harness]\ndefault = 'typo'\n",
    )
    .unwrap();

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(release.agent_calls().is_empty());
    assert!(
        stderr(&output).contains("harness.default must be"),
        "{}",
        stderr(&output)
    );
    let body = release.pr_body("release-0.2.0");
    assert!(
        summary_and_rest(&body)
            .0
            .contains("agent summary was unavailable"),
        "{body}"
    );
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
    assert_eq!(calls[0]["prompt"].as_str().unwrap(), expected);
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
fn when_the_agent_fails_the_summary_is_the_generated_notes() {
    // One way to fail end to end; the summary step's own test takes the rest.
    let release = Release::new();
    release.ci_reports(GREEN);
    release.agent_runs("echo 'Not logged in' >&2; exit 1");

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let err = unstamped(&output);
    assert!(
        err.lines()
            .any(|line| line.starts_with("release: warning:") && line.contains("agent summary")),
        "{err}"
    );
    let body = release.pr_body("release-0.2.0");
    let (summary, rest) = summary_and_rest(&body);
    let (note, notes) = summary.split_once("\n\n").expect(&body);
    assert!(
        !note.contains('\n') && note.contains("agent summary was unavailable"),
        "{body}"
    );
    assert!(notes.starts_with("## What's Changed\n"), "{body}");
    assert!(notes.contains("Add the frobnicator"), "{body}");
    assert!(!notes.contains("Shipped in 0.1.0"), "{body}");
    assert!(rest.contains("```diff"), "{body}");
    assert_eq!(
        release.origin_tag("v0.2.0"),
        Some(release.pr_head("release-0.2.0"))
    );
}

#[test]
#[cfg(target_os = "linux")]
fn the_summary_is_the_agents_unless_it_fails_or_prints_only_blank_space() {
    let notes = "_The agent summary was unavailable, so this is GitHub's generated notes._\n\n\
                 notes: api repos/{owner}/{repo}/releases/generate-notes \
                 -f tag_name=v0.2.0 -f target_commitish=main --jq .body\n";
    for (agent, summary) in [
        ("echo 'Not logged in' >&2; exit 1", notes),
        ("printf '\\n'", notes),
        ("printf ' \\n\\t\\n'", notes),
        ("echo 'The headline.'", "The headline.\n"),
    ] {
        let output = with_functions_of(
            "release.sh",
            &format!(
                "summary_input() {{ echo input; }}\n\
                 gh() {{ echo \"notes: $*\"; }}\n\
                 summary_agent() {{ cat >/dev/null; {agent}; }}\n\
                 release_summary main v0.2.0 diff\n"
            ),
        );

        assert!(output.status.success(), "{agent}: {}", stderr(&output));
        assert_eq!(stdout(&output), summary, "{agent}");
        let warned =
            unstamped(&output).contains("release: warning: the agent summary is unavailable");
        assert_eq!(warned, summary == notes, "{agent}: {}", stderr(&output));
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_progress_line_is_stamped_with_the_local_time_it_was_printed() {
    use chrono::Timelike;

    // Kiritimati keeps UTC+14 all year, so its clock is never the host's or UTC's.
    let kiritimati = chrono::FixedOffset::east_opt(14 * 3600).unwrap();
    let now = || chrono::Utc::now().with_timezone(&kiritimati).time();
    let before = now().with_nanosecond(0).unwrap();
    let output = with_functions_of(
        "release.sh",
        "TZ=Pacific/Kiritimati progress waiting on 100% of '%s' runs",
    );
    let after = now();

    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    let line = err
        .strip_suffix('\n')
        .filter(|line| !line.contains('\n'))
        .unwrap_or_else(|| panic!("not one line: {err:?}"));
    let (stamp, message) = stamped(line).unwrap_or_else(|| panic!("not a progress line: {err:?}"));
    assert_eq!(message, "waiting on 100% of %s runs");
    // A run across midnight wraps the clock, so only a forward span is bounded.
    if before <= after {
        assert!(
            before <= stamp && stamp <= after,
            "{stamp} not in {before}..{after}"
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn each_step_prints_a_progress_line_on_stderr() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.pr_head("release-0.2.0");
    let url = format!("https://github.com/{REPO}/pull/4");
    let expected = [
        "release: writing the summary with the configured Harness".to_owned(),
        format!("release: opened {url}"),
        format!("release: waiting for CI on {url}"),
        format!(
            "release: tagged v0.2.0 on {} and pushed the tag",
            &head[..7]
        ),
        format!("release: merged {url}"),
        format!(
            "release: waiting for the release workflow at {}",
            release.release_run()["url"].as_str().unwrap()
        ),
        format!("release: released v0.2.0 at {}", release_url()),
    ];
    let err = unstamped(&output);
    let lines: Vec<&str> = err.lines().collect();
    let mut at = 0;
    for line in &expected {
        at += lines[at..]
            .iter()
            .position(|l| l == line)
            .unwrap_or_else(|| panic!("no {line:?} in order in {lines:#?}"))
            + 1;
    }
    assert_eq!(at, lines.len(), "{lines:#?}");
    assert!(output.stdout.is_empty());
}

#[test]
#[cfg(target_os = "linux")]
fn after_the_tag_the_script_waits_for_the_release_workflow_run_to_be_listed_and_finish() {
    let release = Release::new();
    release.ci_reports(GREEN);
    release.release_workflow(json!({"hidden_polls": 3, "pending_polls": 2}));

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let run = release.release_run();
    assert_eq!(run["hidden_polls"], 0);
    assert_eq!(run["pending_polls"], 0);
    let err = unstamped(&output);
    let waiting = [
        "release: waiting for the release workflow on v0.2.0 to start".to_owned(),
        format!(
            "release: waiting for the release workflow at {}",
            run["url"].as_str().unwrap()
        ),
    ];
    let lines: Vec<&str> = err.lines().collect();
    assert!(lines.windows(2).any(|pair| pair == waiting), "{lines:#?}");
    assert_eq!(
        err.lines().last(),
        Some(format!("release: released v0.2.0 at {}", release_url()).as_str()),
        "{err}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_failed_release_workflow_run_exits_1_naming_the_failed_jobs_and_leaves_the_tag() {
    let release = Release::new();
    release.ci_reports(GREEN);
    release.release_workflow(failed_release());

    let output = release.run_script("0.2.0");

    assert_release_failed(&release, &output, &release.pr_head("release-0.2.0"));
    assert!(!stderr(&output).contains(&release_url()));
}

#[test]
#[cfg(target_os = "linux")]
fn ci_on_main_for_the_merge_neither_holds_up_nor_fails_the_release() {
    for (status, conclusion) in [("in_progress", ""), ("completed", "failure")] {
        let release = Release::new();
        let head = release.push_bump("0.2.0");
        release.open_bump_pr("0.2.0");
        let merge = release.merge_bump_pr("0.2.0", &head);
        release.main_ci_on(&merge, status, conclusion, 0);

        let output = release.run_script("0.2.0");

        assert!(output.status.success(), "{status}: {}", stderr(&output));
        let err = stderr(&output);
        assert!(err.contains(&release_url()), "{status}: {err}");
    }
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
    assert!(!release.pr_opened("release-0.2.0"));
    assert!(!release.origin_branch("release-0.2.0"));
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

#[test]
#[cfg(target_os = "linux")]
fn review_prints_the_summary_and_yes_carries_on_with_it_as_written() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script_with(&["--review", "0.2.0"], "y\n", &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    let err = unstamped(&output);
    let summary_at = err.find(AGENT_SUMMARY).expect(&err);
    let prompt_at = err
        .find("release: open the PR with this summary? [y]es / [e]dit / [n]o")
        .expect(&err);
    assert!(summary_at < prompt_at, "{err}");
    let body = release.pr_body("release-0.2.0");
    assert_eq!(
        summary_and_rest(&body).0,
        format!("{AGENT_SUMMARY}\n"),
        "{body}"
    );
    assert_eq!(
        release.origin_tag("v0.2.0"),
        Some(release.pr_head("release-0.2.0"))
    );
}

#[test]
#[cfg(target_os = "linux")]
fn review_edit_opens_the_summary_in_the_editor_and_carries_on_with_what_was_saved() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let editor = release.root().join("editor.sh");
    write_executable(&editor, "#!/bin/sh\nsed -i 's/headline/big news/' \"$1\"\n");

    let output = release.run_script_with(
        &["--review", "0.2.0"],
        "e\n",
        &[("EDITOR", editor.to_str().unwrap())],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let body = release.pr_body("release-0.2.0");
    assert_eq!(
        summary_and_rest(&body).0,
        "The big news is the frobnicator (#2).\n",
        "{body}"
    );
    assert_eq!(
        release.origin_tag("v0.2.0"),
        Some(release.pr_head("release-0.2.0"))
    );
}

#[test]
#[cfg(target_os = "linux")]
fn review_no_stops_with_nothing_pushed() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let main_before = release.origin(&["rev-parse", "main"]);

    let output = release.run_script_with(&["--review", "0.2.0"], "n\n", &[]);

    assert!(!output.status.success(), "{}", stderr(&output));
    assert!(!release.origin_branch("release-0.2.0"));
    assert!(!release.pr_opened("release-0.2.0"));
    assert_eq!(release.origin(&["rev-parse", "main"]), main_before);
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

#[test]
#[cfg(target_os = "linux")]
fn without_review_the_script_runs_to_the_tag_without_reading_stdin() {
    let release = Release::new();
    release.ci_reports(GREEN);

    let output = release.run_script_with(&["0.2.0"], "n\n", &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stderr(&output).contains("[y]es"), "{}", stderr(&output));
    assert_eq!(
        release.origin_tag("v0.2.0"),
        Some(release.pr_head("release-0.2.0"))
    );
}

#[test]
#[cfg(target_os = "linux")]
fn the_release_body_is_the_bump_prs_summary_then_the_generated_notes_then_the_install_instructions()
{
    // The release script cuts the release the notes are for, and the notes
    // script runs twice, as the behavior includes that a rerun gives the same
    // body.
    let release = Release::new();
    release.ci_reports(GREEN);
    let output = release.run_script("0.2.0");
    assert!(output.status.success(), "{}", stderr(&output));

    let first = release.run_notes_script("v0.2.0");
    let again = release.run_notes_script("v0.2.0");

    assert!(first.status.success(), "{}", stderr(&first));
    let body = stdout(&first);
    let notes = release.generated_notes("v0.2.0");
    assert!(notes.contains("Add the frobnicator"), "{notes}");
    assert_eq!(
        body,
        format!("{AGENT_SUMMARY}\n\n{notes}\n\n{INSTALL_NOTES}\n")
    );
    assert!(again.status.success(), "{}", stderr(&again));
    assert_eq!(stdout(&again), body);
}

#[test]
#[cfg(target_os = "linux")]
fn without_a_bump_pr_summary_the_release_body_is_the_generated_notes_and_install_instructions() {
    // v0.1.0 is the merge of a PR without a summary section. The bump PR
    // step's own test takes the other ways to have no summary.
    let release = Release::new();

    let output = release.run_notes_script("v0.1.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let notes = release.generated_notes("v0.1.0");
    assert_eq!(stdout(&output), format!("{notes}\n\n{INSTALL_NOTES}\n"));
}

#[test]
#[cfg(target_os = "linux")]
fn the_summary_is_from_the_pr_whose_head_or_merge_is_tagged() {
    const TAGGED: &str = "1111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222";
    let with_summary = "Intro.\n<!-- release-summary:start -->\nThe headline.\n\
                        <!-- release-summary:end -->\nRest.";
    let release = format!("release-0.2.0 {REPO} {REPO}");
    let fork = format!("release-0.2.0 attacker/thirdshift {REPO}");
    for (case, pulls, body, summary) in [
        (
            "the merge of a PR with a summary section",
            format!("{OTHER} {OTHER} other {REPO} {REPO} 4\n{TAGGED} {OTHER} {release} 5"),
            with_summary,
            "The headline.\n",
        ),
        (
            "the merge of a PR without a summary section",
            format!("{TAGGED} {OTHER} {release} 5"),
            "Old news.",
            "",
        ),
        (
            "the head of a PR, not its merge",
            format!("none {TAGGED} {release} 5"),
            with_summary,
            "The headline.\n",
        ),
        (
            "a fork PR listed ahead of the release PR",
            format!("none {TAGGED} {fork} 9\nnone {TAGGED} {release} 5"),
            with_summary,
            "The headline.\n",
        ),
        (
            "only a fork PR whose head is the tagged commit",
            format!("none {TAGGED} {fork} 5"),
            with_summary,
            "",
        ),
        (
            "a PR for another commit",
            format!("{OTHER} {OTHER} {release} 5"),
            with_summary,
            "",
        ),
        ("a commit with no PR", String::new(), with_summary, ""),
    ] {
        let output = with_functions_of(
            "release-notes.sh",
            &format!(
                "gh() {{\n\
                 \tcase $1 in\n\
                 \tapi) printf '%s' '{pulls}' ;;\n\
                 \tpr) [ \"$3\" = 5 ] && printf '%s\\n' '{body}' ;;\n\
                 \tesac\n\
                 }}\n\
                 bump_summary {TAGGED} release-0.2.0\n"
            ),
        );

        assert!(output.status.success(), "{case}: {}", stderr(&output));
        assert_eq!(stdout(&output), summary, "{case}");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_foreign_pr_with_the_tagged_head_does_not_supply_the_release_summary() {
    // The commits API lists every pull request that points at the tagged bump
    // commit, oldest first. A fork can push that same commit and open first.
    const TAGGED: &str = "1111111111111111111111111111111111111111";
    let output = with_functions_of(
        "release-notes.sh",
        &format!(
            r#"gh() {{
	case $1 in
	api)
		jq=
		while [ $# -gt 0 ]; do
			if [ "$1" = --jq ]; then jq=$2; fi
			shift
		done
		case $jq in
		*head.repo.full_name*)
			printf '%s\n' 'null {tagged} release-0.2.0 attacker/thirdshift JacobStephens2/thirdshift 9' \
				'null {tagged} release-0.2.0 JacobStephens2/thirdshift JacobStephens2/thirdshift 5'
			;;
		*head.sha*)
			printf '%s\n' 'null {tagged} 9' 'null {tagged} 5'
			;;
		*)
			printf '%s\n' 'null 9' 'null 5'
			;;
		esac
		;;
	pr)
		case $3 in
		9) printf '%s\n' $'<!-- release-summary:start -->\nFOREIGN-SENTINEL\n<!-- release-summary:end -->' ;;
		5) printf '%s\n' $'<!-- release-summary:start -->\nThe release.\n<!-- release-summary:end -->' ;;
		esac
		;;
	esac
}}
bump_summary {tagged} release-0.2.0
"#,
            tagged = TAGGED,
        ),
    );

    assert!(output.status.success(), "{}", stderr(&output));
    let summary = stdout(&output);
    assert!(
        !summary.contains("FOREIGN-SENTINEL"),
        "foreign summary was published: {summary}"
    );
    assert_eq!(summary, "The release.\n");
}

#[test]
#[cfg(target_os = "linux")]
fn when_the_bump_pr_cannot_be_read_the_release_body_is_the_generated_notes_with_a_warning() {
    // The release script only cuts the release the notes are for.
    let release = Release::new();
    release.ci_reports(GREEN);
    let output = release.run_script("0.2.0");
    assert!(output.status.success(), "{}", stderr(&output));
    let tagged = release.pr_head("release-0.2.0");
    release.gh(&[
        "fake",
        "fails",
        &format!("api repos/{{owner}}/{{repo}}/commits/{tagged}/pulls"),
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
    assert_eq!(stdout(&output), format!("{notes}\n\n{INSTALL_NOTES}\n"));
}

/// Runs the script with `version` and asserts it refused before pushing
/// anything, with a message on stderr containing each of `reasons`.
fn assert_refused(release: &Release, version: &str, reasons: &[&str]) {
    assert_refused_by(release, version, reasons, || release.run_script(version));
}

/// Runs `run`, the script or some of its functions, and asserts it refused
/// the `case` before pushing anything, with a message on stderr containing
/// each of `reasons`.
fn assert_refused_by(
    release: &Release,
    case: &str,
    reasons: &[&str],
    run: impl FnOnce() -> Output,
) {
    let refs_before = release.origin_refs();
    let prs_before = release.gh_state()["prs"].clone();

    let output = run();

    assert_eq!(output.status.code(), Some(1), "{case} was not refused");
    let err = unstamped(&output);
    assert!(
        err.contains("release: refusing to release"),
        "{case}: {err}"
    );
    for reason in reasons {
        assert!(err.contains(reason), "{case}: no {reason:?} in {err}");
    }
    assert_eq!(release.origin_refs(), refs_before, "{case}");
    assert_eq!(release.gh_state()["prs"], prs_before, "{case}");
}

#[test]
#[cfg(target_os = "linux")]
fn a_version_that_is_not_plain_semver_is_refused() {
    // One version end to end; the version check's own test takes the rest.
    let release = Release::new();

    assert_refused(&release, "0.4.0-rc.1", &["not a plain X.Y.Z version"]);
}

#[test]
#[cfg(target_os = "linux")]
fn a_version_not_higher_than_the_one_on_main_is_refused() {
    // One version end to end; the comparison's own test takes the rest.
    let release = Release::new();

    assert_refused(&release, "0.0.10", &["not higher than 0.1.0 on main"]);
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
fn only_plain_x_y_z_versions_with_no_leading_zeros_pass() {
    for (version, plain) in [
        ("0.4.0", true),
        ("10.0.12", true),
        ("v0.4.0", false),
        ("0.4", false),
        ("abc", false),
        ("0.4.0-rc.1", false),
        ("01.4.0", false),
        ("0.4.0.1", false),
        ("", false),
    ] {
        let output = with_functions_of("release.sh", &format!("is_plain_version '{version}'"));

        assert_eq!(output.status.success(), plain, "{version:?}");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_version_is_higher_only_by_its_parts_as_numbers() {
    for (version, higher) in [
        ("0.1.1", true),
        ("0.2.0", true),
        ("0.10.0", true),
        ("1.0.0", true),
        ("0.1.0", false),
        ("0.0.9", false),
        ("0.0.10", false),
    ] {
        let output = with_functions_of("release.sh", &format!("is_higher {version} 0.1.0"));

        assert_eq!(output.status.success(), higher, "{version}");
    }
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
fn the_latest_failed_push_run_is_skipped_for_an_older_green_main_commit() {
    let release = Release::new();
    let older = release.origin(&["rev-parse", "main^1"]);
    release.main_ci_on(&older, "completed", "success", 0);
    // This supersedes main's earlier successful run.
    release.main_ci("completed", "failure");
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.pr_head("release-0.2.0");
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), older);
    assert!(!stderr(&output).contains("waiting for CI on main"));
}

#[test]
#[cfg(target_os = "linux")]
fn with_no_green_commit_since_the_last_tag_the_script_waits_for_success() {
    let release = Release::new();
    let base = release.origin(&["rev-parse", "main"]);
    let previous = release.origin(&["rev-parse", "v0.1.0"]);
    release.main_ci_on(&previous, "completed", "success", 0);
    release.main_ci_on(&base, "completed", "success", 3);
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.find("waiting for CI on main").unwrap() < err.find("bumping the version").unwrap());
    let head = release.pr_head("release-0.2.0");
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), base);
}

#[test]
#[cfg(target_os = "linux")]
fn a_main_whose_ci_appears_after_a_few_polls_is_released() {
    let release = Release::new();
    release.main_ci("completed", "failure");
    release.merge_pr("untested", "Not yet built", "");
    let base = release.origin(&["rev-parse", "main"]);
    release.gh(&[
        "fake",
        "run",
        &json!({
            "branch": "main", "workflow": "ci.yml", "event": "push",
            "headSha": base, "status": "completed", "conclusion": "success", "hidden_polls": 3,
        })
        .to_string(),
    ]);
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("waiting for CI on main"));
    let head = release.pr_head("release-0.2.0");
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), base);
}

#[test]
#[cfg(target_os = "linux")]
fn moving_main_does_not_restart_the_wait_for_an_older_pending_run() {
    for conclusion in ["success", "failure"] {
        let release = Release::new();
        let (old, _) = release.main_moves_while_waiting(conclusion);
        release.ci_reports(GREEN);

        let output = release.run_script("0.2.0");

        assert!(output.status.success(), "{conclusion}: {}", stderr(&output));
        let head = release.pr_head("release-0.2.0");
        assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), old);
        release.origin(&["merge-base", "--is-ancestor", "v0.2.0", "main"]);
    }
}

#[test]
#[cfg(target_os = "linux")]
fn main_moving_during_a_green_ci_read_does_not_discard_that_commit() {
    let release = Release::new();
    let (old, _) = release.main_moves_while_waiting("success");
    release.main_ci_on(&old, "completed", "success", 0);
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.pr_head("release-0.2.0");
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), old);
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_after_the_branch_was_pushed_opens_its_pr_then_merges_and_tags() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.prs_from("release-0.2.0").len(), 1);
    assert_eq!(release.pr_head("release-0.2.0"), head);
    let body = release.pr_body("release-0.2.0");
    assert!(body.contains(AGENT_SUMMARY), "{body}");
    assert!(
        body.contains("-version = \"0.1.0\"\n+version = \"0.2.0\""),
        "{body}"
    );
    let merge = release.origin(&["rev-parse", "main"]);
    assert_eq!(release.origin(&["rev-parse", &format!("{merge}^2")]), head);
    assert_eq!(
        release.origin_tag("v0.2.0"),
        Some(release.pr_head("release-0.2.0"))
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_resumed_bump_pr_from_another_repository_is_refused() {
    // The open PR end to end; finding the earlier run takes the rest.
    for (merged, tagged) in [(false, false), (true, false), (true, true)] {
        let release = Release::new();
        let head = release.push_bump("0.2.0");
        release.open_bump_pr("0.2.0");
        if merged {
            release.merge_bump_pr("0.2.0", &head);
            if tagged {
                release.origin(&["tag", "v0.2.0", &head]);
            }
        }
        release.gh(&["fake", "pr", "release-0.2.0", "isCrossRepository", "true"]);
        release.ci_reports(GREEN);
        let url = release.pr("release-0.2.0")["url"]
            .as_str()
            .unwrap()
            .to_owned();
        let reasons = [url.as_str(), "another repository"];

        if merged {
            assert_refused_by(
                &release,
                &format!("merged {merged}, tagged {tagged}"),
                &reasons,
                || release.with_functions(FIND_EARLIER_RUN),
            );
        } else {
            assert_refused(&release, "0.2.0", &reasons);
        }
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_resumed_bump_pr_with_changes_beyond_the_version_bump_is_refused() {
    // One open PR end to end; checking the bump PR takes the rest.
    for merged in [false, true] {
        for change in [
            BumpChange::AppendComment("README.md"),
            BumpChange::AppendComment("Cargo.toml"),
            BumpChange::AppendComment("Cargo.lock"),
            BumpChange::AddedFile,
            BumpChange::FileMode,
            BumpChange::WrongVersion,
        ] {
            let release = Release::new();
            release.push_bump("0.2.0");
            let head = release.push_change_to_bump("0.2.0", change);
            release.open_bump_pr("0.2.0");
            if merged {
                release.merge_bump_pr("0.2.0", &head);
            }
            release.ci_reports(GREEN);
            let url = release.pr("release-0.2.0")["url"]
                .as_str()
                .unwrap()
                .to_owned();
            let reasons = [url.as_str(), "version bump"];

            if !merged && change == BumpChange::AppendComment("README.md") {
                assert_refused(&release, "0.2.0", &reasons);
            } else {
                assert_refused_by(
                    &release,
                    &format!("merged {merged}, {change:?}"),
                    &reasons,
                    || release.with_functions(&format!("{FIND_EARLIER_RUN}\nvalidate_bump_pr")),
                );
            }
        }
    }
}

#[test]
#[cfg(target_os = "linux")]
fn a_merged_bump_pr_whose_head_is_not_on_origin_is_refused() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.merge_bump_pr("0.2.0", &head);
    let missing = json!("0".repeat(40)).to_string();
    release.gh(&["fake", "pr", "release-0.2.0", "headRefOid", &missing]);
    let url = release.pr("release-0.2.0")["url"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_refused_by(
        &release,
        "missing head",
        &[&url, "could not be fetched"],
        || release.with_functions(&format!("{FIND_EARLIER_RUN}\nvalidate_bump_pr")),
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_after_the_pr_was_opened_merges_and_tags_without_a_second_pr_or_bump() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.ci_reports(GREEN);
    // Branch lookup is ambiguous; only the selected PR's URL may be merged.
    release.gh(&["fake", "fails", "pr merge release-0.2.0"]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.prs_from("release-0.2.0").len(), 1);
    let pr = release.pr("release-0.2.0");
    assert_eq!(pr["state"], "MERGED");
    assert_eq!(pr["body"], "Summary from the first run");
    assert_eq!(release.pr_head("release-0.2.0"), head);
    let merge = release.origin(&["rev-parse", "main"]);
    assert_eq!(release.origin(&["rev-parse", &format!("{merge}^2")]), head);
    assert_eq!(
        release.origin_tag("v0.2.0"),
        Some(release.pr_head("release-0.2.0"))
    );
    let err = unstamped(&output);
    assert!(
        err.contains("release: waiting for CI on https://github.com/"),
        "{err}"
    );
    let merges = release.merge_calls();
    assert_eq!(merges.last().unwrap()[2], pr["url"].as_str().unwrap());
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_after_the_merge_tags_the_bump_commit_and_pushes_the_tag() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.merge_bump_pr("0.2.0", &head);
    release.merge_pr("later", "Merged after the release", "");

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(head.clone()));
    assert_eq!(release.prs_from("release-0.2.0").len(), 1);
    let err = unstamped(&output);
    assert!(
        err.contains(&format!(
            "release: tagged v0.2.0 on {} and pushed the tag",
            &head[..7]
        )),
        "{err}"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_once_the_release_workflow_succeeded_prints_the_release_and_changes_nothing() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.merge_bump_pr("0.2.0", &head);
    release.origin(&["tag", "v0.2.0", &head]);
    let refs_before = release.origin_refs();
    let state_before = release.gh_state();

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let err = unstamped(&output);
    assert!(err.contains("v0.2.0 is already tagged"), "{err}");
    assert_eq!(
        err.lines().last(),
        Some(format!("release: released v0.2.0 at {}", release_url()).as_str()),
        "{err}"
    );
    assert_eq!(release.origin_refs(), refs_before);
    assert_eq!(release.gh_state(), state_before);
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_while_the_release_workflow_runs_waits_for_it_then_succeeds() {
    let release = Release::new();
    release.release_workflow(json!({"pending_polls": 3}));
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.merge_bump_pr("0.2.0", &head);
    release.origin(&["tag", "v0.2.0", &head]);
    let refs_before = release.origin_refs();

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.release_run()["pending_polls"], 0);
    let err = stderr(&output);
    assert!(err.contains("waiting for the release workflow at"), "{err}");
    assert!(err.contains(&release_url()), "{err}");
    assert_eq!(release.origin_refs(), refs_before);
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_after_the_release_workflow_failed_fails_the_same_way() {
    let release = Release::new();
    release.release_workflow(failed_release());
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.merge_bump_pr("0.2.0", &head);
    release.origin(&["tag", "v0.2.0", &head]);
    let refs_before = release.origin_refs();

    let output = release.run_script("0.2.0");

    assert_release_failed(&release, &output, &release.pr_head("release-0.2.0"));
    assert_eq!(release.origin_refs(), refs_before);
}

#[test]
#[cfg(target_os = "linux")]
fn a_rerun_after_the_failed_jobs_were_rerun_reports_the_new_attempt() {
    let release = Release::new();
    release.ci_reports(GREEN);
    release.release_workflow(failed_release());
    let first = release.run_script("0.2.0");
    assert_eq!(first.status.code(), Some(1), "{}", stderr(&first));
    // `gh run rerun --failed`: a new attempt of the same run, in progress for
    // the script's first reads, whose rerun smoke check passes.
    let attempt = json!({
        "databaseId": release.release_run()["databaseId"],
        "conclusion": "success",
        "pending_polls": 2,
        "jobs": [{"name": "custom-release-smoke / rocky", "conclusion": "success"}],
    });
    release.gh(&["fake", "run", &attempt.to_string()]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains(&release_url()),
        "{}",
        stderr(&output)
    );
}

#[test]
#[cfg(target_os = "linux")]
fn an_annotated_tag_on_the_bump_head_is_reported_as_already_tagged() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.merge_bump_pr("0.2.0", &head);
    release.origin(&["tag", "-a", "-m", "Release 0.2.0", "v0.2.0", &head]);
    let refs_before = release.origin_refs();

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("v0.2.0 is already tagged"), "{err}");
    assert!(err.contains(&release_url()), "{err}");
    assert_eq!(release.origin_refs(), refs_before);
}

#[test]
#[cfg(target_os = "linux")]
fn a_tag_that_is_not_on_the_bump_head_is_refused() {
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
fn a_tag_while_the_bump_pr_is_open_resumes_merging_without_retagging() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.origin(&["tag", "v0.2.0", &head]);
    release.gh(&["fake", "fails", "api repos/{owner}/{repo}/actions/runs"]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("v0.2.0 is already tagged"));
    assert_eq!(release.origin_tag("v0.2.0"), Some(head));
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    assert_eq!(release.prs_from("release-0.2.0").len(), 1);
    release.origin(&["merge-base", "--is-ancestor", "v0.2.0", "main"]);
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
    assert_eq!(release.prs_from("release-0.2.0").len(), 0);
}

#[test]
#[cfg(target_os = "linux")]
fn an_open_bump_pr_with_a_different_head_from_origin_is_refused() {
    let release = Release::new();
    release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    // A stale PR head is still a real, fetchable commit, but not the branch tip.
    let stale = release.origin(&["rev-parse", "main"]);
    release.gh(&[
        "fake",
        "pr",
        "release-0.2.0",
        "headRefOid",
        &json!(stale).to_string(),
    ]);
    release.ci_reports(GREEN);
    let url = release.pr("release-0.2.0")["url"]
        .as_str()
        .unwrap()
        .to_owned();
    let merges_before = release.merge_calls();

    assert_refused(&release, "0.2.0", &[&url, "head does not match", "origin"]);
    assert_eq!(release.merge_calls(), merges_before);
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
    assert_eq!(release.prs_from("release-0.2.0").len(), 1);
}

#[test]
#[cfg(target_os = "linux")]
fn a_bump_pr_closed_without_merging_and_its_branch_deleted_starts_over() {
    // Even a closed PR from another repository leaves nothing to resume.
    for cross_repository in ["false", "true"] {
        let release = Release::new();
        let stale = release.push_bump("0.2.0");
        release.open_bump_pr("0.2.0");
        release.gh(&["fake", "pr", "release-0.2.0", "state", "\"CLOSED\""]);
        release.gh(&[
            "fake",
            "pr",
            "release-0.2.0",
            "isCrossRepository",
            cross_repository,
        ]);
        // GitHub keeps a closed PR's head once its branch is deleted.
        let head = format!("\"{stale}\"");
        release.gh(&["fake", "pr", "release-0.2.0", "headRefOid", &head]);
        release.origin(&["branch", "-D", "release-0.2.0"]);
        release.ci_reports(GREEN);

        let output = release.run_script("0.2.0");

        assert!(output.status.success(), "{}", stderr(&output));
        let prs = release.prs_from("release-0.2.0");
        assert_eq!(prs.len(), 2);
        assert_eq!(prs[0]["state"], "CLOSED");
        assert_eq!(prs[1]["state"], "MERGED");
        assert_eq!(
            release.origin_tag("v0.2.0"),
            Some(release.pr_head("release-0.2.0"))
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn review_no_on_a_rerun_after_the_branch_was_pushed_stops_with_no_pr_opened() {
    let release = Release::new();
    release.push_bump("0.2.0");
    release.ci_reports(GREEN);
    let refs_before = release.origin_refs();

    let output = release.run_script_with(&["--review", "0.2.0"], "n\n", &[]);

    assert!(!output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(
        err.contains("stopped at the review, so no PR was opened"),
        "{err}"
    );
    assert!(!release.pr_opened("release-0.2.0"));
    assert_eq!(release.origin_refs(), refs_before);
}

#[test]
#[cfg(target_os = "linux")]
fn review_resumed_branch_without_pr_with_extra_changes_is_refused() {
    let release = Release::new();
    release.push_bump("0.2.0");
    release.push_change_to_bump("0.2.0", BumpChange::AddedFile);
    release.ci_reports(GREEN);
    let refs_before = release.origin_refs();
    let merges_before = release.merge_calls();

    let output = release.run_script("0.2.0");

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("refusing to release"));
    assert_eq!(release.origin_refs(), refs_before);
    assert_eq!(release.merge_calls(), merges_before);
}

#[test]
#[cfg(target_os = "linux")]
fn review_tagged_resumed_pr_with_extra_changes_is_refused() {
    let release = Release::new();
    release.push_bump("0.2.0");
    let head = release.push_change_to_bump("0.2.0", BumpChange::AddedFile);
    release.open_bump_pr("0.2.0");
    release.merge_bump_pr("0.2.0", &head);
    release.origin(&["tag", "v0.2.0", &head]);
    let url = release.pr("release-0.2.0")["url"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_refused(&release, "0.2.0", &[&url, "version bump"]);
}
