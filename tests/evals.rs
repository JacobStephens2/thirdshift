use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "gc.auto=0",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

struct Fixture {
    _root: TempDir,
    home: PathBuf,
    source: PathBuf,
    destination: PathBuf,
    base: String,
    change: String,
    later: String,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        git(&source, &["init", "-q"]);
        fs::write(source.join("code.txt"), "before\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-qm", "Base"]);
        let base = git(&source, &["rev-parse", "HEAD"]);
        fs::write(source.join("code.txt"), "review this\n").unwrap();
        git(&source, &["commit", "-qam", "Change under review"]);
        let change = git(&source, &["rev-parse", "HEAD"]);
        fs::write(source.join("answer.txt"), "future fix\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-qm", "Later fix"]);
        let later = git(&source, &["rev-parse", "HEAD"]);
        git(&source, &["tag", "answer"]);
        let home = root.path().join("home");
        let destination = root.path().join("prepared");
        Self {
            _root: root,
            home,
            source,
            destination,
            base,
            change,
            later,
        }
    }

    fn task(&self, session: &str, state: &str) -> PathBuf {
        let suite = self.home.join(".thirdshift/evals/suites/reviews");
        let task = suite.join("tasks/example");
        fs::create_dir_all(&task).unwrap();
        fs::write(
            suite.join("suite.toml"),
            r#"format = 1
trials_per_arm = 3
[dependencies]
factory_skills = ["skills/thirdshift-code-review/"]
session_prompts = ["src/prompt.rs"]
harness_defaults = ["src/harness/"]
model_defaults = ["src/config.rs"]
effort_defaults = ["src/setup.rs"]
"#,
        )
        .unwrap();
        fs::write(task.join("spec.md"), "Spec as first published\n").unwrap();
        fs::write(
            task.join("task.toml"),
            format!(
                r#"format = 1
state = "{state}"
session = "{session}"
repository = {repository:?}
base = "{base}"
change = "{change}"
criteria = "Catch the made-up bug without wrong findings."
checks = [["cargo", "test", "regression"]]
[inputs]
".thirdshift-issue.md" = "spec.md"
"#,
                repository = self.source.to_str().unwrap(),
                base = self.base,
                change = self.change
            ),
        )
        .unwrap();
        task
    }

    fn evals(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .arg("evals")
            .args(args)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap()
    }

    fn prepare(&self) -> Output {
        self.evals(&[
            "prepare",
            "reviews",
            "example",
            self.destination.to_str().unwrap(),
        ])
    }
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn prepares_only_base_history_with_reviewed_changes_and_inputs() {
    let fixture = Fixture::new();
    fixture.task("code-review", "approved");
    success(fixture.prepare());
    let repo = &fixture.destination;
    assert_eq!(git(repo, &["rev-parse", "HEAD"]), fixture.base);
    assert_eq!(git(repo, &["log", "--all", "--format=%H"]), fixture.base);
    assert_eq!(git(repo, &["remote"]), "");
    assert_eq!(git(repo, &["for-each-ref", "--format=%(refname)"]), "");
    assert_eq!(git(repo, &["reflog", "--all"]), "");
    assert!(!repo.join(".git/objects/info/alternates").exists());
    for oid in [&fixture.change, &fixture.later] {
        let output = Command::new("git")
            .current_dir(repo)
            .args(["cat-file", "-e", oid])
            .output()
            .unwrap();
        assert!(!output.status.success(), "future object {oid} survived");
    }
    assert_eq!(
        fs::read_to_string(repo.join("code.txt")).unwrap(),
        "review this\n"
    );
    assert!(!repo.join("answer.txt").exists());
    assert_eq!(
        fs::read_to_string(repo.join(".thirdshift-issue.md")).unwrap(),
        "Spec as first published\n"
    );
    assert!(!repo.join("task.toml").exists());
    assert!(!repo.join("spec.md").exists());
}

#[test]
fn proposals_are_listed_but_cannot_be_prepared() {
    let fixture = Fixture::new();
    fixture.task("code-review", "proposed");
    let suites = success(fixture.evals(&["list"]));
    assert_eq!(suites[0]["name"], "reviews");
    assert_eq!(suites[0]["trials_per_arm"], 3);
    assert_eq!(suites[0]["tasks"][0]["name"], "example");
    assert_eq!(suites[0]["tasks"][0]["state"], "proposed");
    let output = fixture.prepare();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("proposed Eval tasks cannot"));
    assert!(!fixture.destination.exists());
}

#[test]
fn suite_declarations_select_changed_paths_across_all_dependency_categories() {
    let fixture = Fixture::new();
    let task = fixture.task("code-review", "approved");
    let specs = task
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("specs");
    fs::create_dir(&specs).unwrap();
    fs::write(
        specs.join("suite.toml"),
        r#"format = 1
trials_per_arm = 1
[dependencies]
factory_skills = []
session_prompts = ["src/prompt.rs"]
harness_defaults = []
model_defaults = []
effort_defaults = []
"#,
    )
    .unwrap();
    for path in [
        "skills/thirdshift-code-review/SKILL.md",
        "src/harness/codex.rs",
        "src/config.rs",
        "src/setup.rs",
    ] {
        assert_eq!(
            success(fixture.evals(&["touched", path])),
            serde_json::json!(["reviews"]),
            "{path}"
        );
    }
    assert_eq!(
        success(fixture.evals(&["touched", "src/prompt.rs", "src/config.rs"])),
        serde_json::json!(["reviews", "specs"])
    );
    assert_eq!(
        success(fixture.evals(&[
            "touched",
            "src/prompt.rs.old",
            "src/harness-old/codex.rs",
            "README.md"
        ])),
        serde_json::json!([])
    );
    assert_eq!(success(fixture.evals(&["touched"])), serde_json::json!([]));
}

#[test]
fn preparation_history_check_refuses_a_planted_later_commit_on_another_branch() {
    let fixture = Fixture::new();
    fixture.task("code-review", "approved");
    success(fixture.prepare());
    success(fixture.evals(&[
        "verify",
        fixture.destination.to_str().unwrap(),
        &fixture.base,
    ]));
    git(
        &fixture.destination,
        &[
            "fetch",
            "--no-tags",
            fixture.source.to_str().unwrap(),
            &format!("{}:refs/heads/planted-answer", fixture.later),
        ],
    );
    assert_eq!(
        git(&fixture.destination, &["rev-parse", "HEAD"]),
        fixture.base
    );
    let output = fixture.evals(&[
        "verify",
        fixture.destination.to_str().unwrap(),
        &fixture.base,
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("history beyond base"));
}

#[test]
fn spec_review_gets_an_independent_bare_origin_and_can_push() {
    let fixture = Fixture::new();
    fixture.task("spec-review", "approved");
    let prepared = success(fixture.prepare());
    let origin = Path::new(prepared["origin"].as_str().unwrap());
    assert_eq!(git(origin, &["rev-parse", "--is-bare-repository"]), "true");
    assert_eq!(git(origin, &["log", "--all", "--format=%H"]), fixture.base);
    assert_eq!(git(origin, &["remote"]), "");
    assert!(!origin.join("objects/info/alternates").exists());
    git(&fixture.destination, &["checkout", "-qb", "spec"]);
    git(&fixture.destination, &["add", "."]);
    git(
        &fixture.destination,
        &["commit", "-qm", "Spec review changes"],
    );
    git(&fixture.destination, &["push", "-u", "origin", "spec"]);
    assert_eq!(
        git(origin, &["rev-parse", "spec"]),
        git(&fixture.destination, &["rev-parse", "HEAD"])
    );
}

#[test]
fn refuses_wildcard_dependencies_instead_of_silently_missing_changes() {
    let fixture = Fixture::new();
    let task = fixture.task("code-review", "approved");
    let manifest = task.parent().unwrap().parent().unwrap().join("suite.toml");
    let text = fs::read_to_string(&manifest).unwrap();
    fs::write(
        manifest,
        text.replace("skills/thirdshift-code-review/", "skills/*/"),
    )
    .unwrap();
    let output = fixture.evals(&["touched", "skills/thirdshift-code-review/SKILL.md"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wildcard"));
}
