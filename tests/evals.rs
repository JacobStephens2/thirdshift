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

    fn suite(&self, name: &str) -> PathBuf {
        self.home.join(".thirdshift/evals/suites").join(name)
    }

    fn task(&self, session: &str, state: &str) -> PathBuf {
        let suite = self.suite("reviews");
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

fn replay_inputs(fixture: &Fixture) -> PathBuf {
    let replays = fixture.home.join(".thirdshift/replays");
    let forward = replays.join("inputs-2026-10-07");
    let dated = replays.join("2026-10-09");
    for directory in [
        &forward,
        &dated.join("inputs"),
        &dated.join("grading"),
        &dated.join("context/thirdshift"),
    ] {
        fs::create_dir_all(directory).unwrap();
    }
    let pr = serde_json::json!({
        "number": 17, "head_as_merged": fixture.change, "merge_base": fixture.base,
        "head_ref": "issue-16", "implements_issue": 16
    });
    fs::write(forward.join("manifest.json"), serde_json::to_vec(&serde_json::json!({
        "timing_bugs": [{"id": 7, "introducing_pr": pr}],
        "spec_reviews": [{"repo": "thirdshift", "spec_issue": 19,
            "pre_review_head": fixture.change, "merge_base": fixture.base, "spec_branch": "issue-19"}],
        "plans": []
    })).unwrap()).unwrap();
    fs::write(
        dated.join("inputs/manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "codex_bugs": [{"id": 8, "introducing_prs": [pr, {
                "number": 18, "head_as_merged": fixture.change, "merge_base": fixture.base,
                "head_ref": "issue-16", "implements_issue": 16
            }]}], "codex_plans": []
        }))
        .unwrap(),
    )
    .unwrap();
    for (number, title, body, parent, tickets) in [
        (16, "Invented change", "First issue body", Some(19), vec![]),
        (19, "Invented Spec", "First Spec body", None, vec![16]),
    ] {
        fs::write(
            dated.join(format!("context/thirdshift/{number}.json")),
            serde_json::to_vec(&serde_json::json!({"number": number, "title": title,
                "body": body, "parent": parent, "tickets": tickets, "edited": true}))
            .unwrap(),
        )
        .unwrap();
    }
    fs::write(dated.join("grading/eval-import.json"), serde_json::to_vec(&serde_json::json!({
        "bugs": {"7": "Catch the invented lost update.", "8": "Catch the invented duplicate write."},
        "specs": {"thirdshift-19": [
            {"finding": "Invented missing retry", "score": "real"},
            {"finding": "Invented false alarm", "score": "wrong"}
        ]}
    })).unwrap()).unwrap();
    let repository = fixture.home.join("repos/thirdshift");
    fs::create_dir_all(repository.parent().unwrap()).unwrap();
    git(
        fixture.home.as_path(),
        &[
            "clone",
            "-q",
            fixture.source.to_str().unwrap(),
            repository.to_str().unwrap(),
        ],
    );
    replays
}

#[test]
fn imports_replay_bugs_and_scored_specs_once_as_preparable_eval_tasks() {
    let fixture = Fixture::new();
    replay_inputs(&fixture);
    assert_eq!(
        success(fixture.evals(&["import-replays"])),
        serde_json::json!({"imported": 4, "skipped": 0})
    );
    let suites = success(fixture.evals(&["list"]));
    assert_eq!(suites[0]["name"], "code-review");
    assert_eq!(suites[0]["trials_per_arm"], 3);
    assert_eq!(suites[0]["tasks"].as_array().unwrap().len(), 3);
    assert_eq!(suites[1]["name"], "spec-review");
    assert_eq!(suites[1]["trials_per_arm"], 1);
    let bug = fixture
        .suite("code-review")
        .join("tasks/bug-7-pr-17/task.toml");
    let before = fs::read(&bug).unwrap();
    assert!(String::from_utf8_lossy(&before).contains("Catch the invented lost update."));
    let spec = fs::read_to_string(
        fixture
            .suite("spec-review")
            .join("tasks/thirdshift-19/task.toml"),
    )
    .unwrap();
    for expected in [
        "Invented missing retry",
        "real",
        "Invented false alarm",
        "wrong",
    ] {
        assert!(spec.contains(expected));
    }
    assert_eq!(
        success(fixture.evals(&["import-replays"])),
        serde_json::json!({"imported": 0, "skipped": 4})
    );
    assert_eq!(fs::read(&bug).unwrap(), before);
    for (suite, task) in [
        ("code-review", "bug-7-pr-17"),
        ("spec-review", "thirdshift-19"),
    ] {
        let destination = fixture._root.path().join(suite);
        success(fixture.evals(&["prepare", suite, task, destination.to_str().unwrap()]));
        let input = fs::read_to_string(destination.join(".thirdshift-issue.md")).unwrap();
        assert!(input.contains("First issue body"));
        assert!(input.contains("First Spec body"));
        assert!(!input.contains("Invented missing retry"));
        assert!(!input.contains("Catch the invented"));
    }
}

#[test]
fn replay_import_completes_legacy_code_review_trial_inputs_and_preserves_local_edits() {
    let fixture = Fixture::new();
    replay_inputs(&fixture);
    success(fixture.evals(&["import-replays"]));
    let task = fixture
        .suite("code-review")
        .join("tasks/bug-7-pr-17/task.toml");
    let mut document = fs::read_to_string(&task)
        .unwrap()
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
    document.remove("trial");
    document["criteria"] = toml_edit::value("Locally revised catch criterion.");
    fs::write(&task, format!("# Keep my note.\n{document}")).unwrap();

    assert_eq!(
        success(fixture.evals(&["import-replays"])),
        serde_json::json!({"imported": 0, "skipped": 4})
    );

    let contents = fs::read_to_string(&task).unwrap();
    let mut restored = contents.parse::<toml_edit::DocumentMut>().unwrap();
    assert_eq!(
        restored["criteria"].as_str(),
        Some("Locally revised catch criterion.")
    );
    assert!(contents.contains("# Keep my note."));
    assert!(restored.get("trial").is_some(), "{contents}");
    restored["trial"]["branch"] = toml_edit::value("custom-review-branch");
    let before = restored.to_string();
    fs::write(&task, &before).unwrap();
    success(fixture.evals(&["import-replays"]));
    assert_eq!(fs::read_to_string(task).unwrap(), before);
}

#[test]
fn imported_suites_follow_production_review_sources() {
    let fixture = Fixture::new();
    replay_inputs(&fixture);
    success(fixture.evals(&["import-replays"]));
    for path in [
        "skills/thirdshift-code-review/SKILL.md",
        "src/prompt.rs",
        "src/session.rs",
        "src/trial.rs",
        "src/harness/codex.rs",
        "src/config.rs",
        "src/setup.rs",
    ] {
        assert_eq!(
            success(fixture.evals(&["touched", path])),
            serde_json::json!(["code-review", "spec-review"]),
            "{path}"
        );
    }
}

#[test]
fn replay_import_refuses_incomplete_grading_without_publishing_either_suite() {
    let fixture = Fixture::new();
    let replays = replay_inputs(&fixture);
    let path = replays.join("2026-10-09/grading/eval-import.json");
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (pointer, replacement, diagnostic) in [
        ("/bugs", serde_json::json!({}), "missing catch criterion"),
        ("/specs", serde_json::json!({}), "missing Spec grades"),
        ("/bugs/7", serde_json::json!("  "), "blank catch criterion"),
        (
            "/specs/thirdshift-19/0/score",
            serde_json::json!("unsure"),
            "unknown variant",
        ),
    ] {
        let mut data = original.clone();
        *data.pointer_mut(pointer).unwrap() = replacement;
        fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        let output = fixture.evals(&["import-replays"]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!fixture.suite("code-review").exists());
        assert!(!fixture.suite("spec-review").exists());
    }
}

#[test]
fn replay_import_resumes_missing_tasks_and_preserves_existing_edits() {
    let fixture = Fixture::new();
    replay_inputs(&fixture);
    success(fixture.evals(&["import-replays"]));
    let suite_path = fixture.suite("code-review").join("suite.toml");
    let task_path = fixture
        .suite("code-review")
        .join("tasks/bug-7-pr-17/task.toml");
    let suite = format!(
        "{}\n# Locally reviewed dependencies\n",
        fs::read_to_string(&suite_path).unwrap()
    );
    let task = fs::read_to_string(&task_path).unwrap().replace(
        "Catch the invented lost update.",
        "Locally revised catch criterion.",
    );
    fs::write(&suite_path, &suite).unwrap();
    fs::write(&task_path, &task).unwrap();
    fs::remove_dir_all(fixture.suite("spec-review").join("tasks/thirdshift-19")).unwrap();
    assert_eq!(
        success(fixture.evals(&["import-replays"])),
        serde_json::json!({"imported": 1, "skipped": 3})
    );
    assert_eq!(fs::read_to_string(suite_path).unwrap(), suite);
    assert_eq!(fs::read_to_string(task_path).unwrap(), task);
    let listed = success(fixture.evals(&["list"]));
    assert_eq!(listed[1]["tasks"][0]["name"], "thirdshift-19");
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
    fixture.task("code-review", "approved");
    let specs = fixture.suite("specs");
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
    fixture.task("code-review", "approved");
    let manifest = fixture.suite("reviews").join("suite.toml");
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

#[test]
fn preparation_ignores_source_diff_prefix_setting() {
    let fixture = Fixture::new();
    git(&fixture.source, &["config", "diff.noprefix", "true"]);
    fixture.task("code-review", "approved");
    success(fixture.prepare());
    assert_eq!(
        fs::read_to_string(fixture.destination.join("code.txt")).unwrap(),
        "review this\n"
    );
}

#[test]
fn preparation_preserves_changed_submodule_commit() {
    let mut fixture = Fixture::new();
    let sub = fixture._root.path().join("submodule-source");
    fs::create_dir(&sub).unwrap();
    git(&sub, &["init", "-q"]);
    fs::write(sub.join("code.txt"), "before\n").unwrap();
    git(&sub, &["add", "."]);
    git(&sub, &["commit", "-qm", "Submodule base"]);
    let old = git(&sub, &["rev-parse", "HEAD"]);
    fs::write(sub.join("code.txt"), "after\n").unwrap();
    git(&sub, &["commit", "-qam", "Submodule change"]);
    let new = git(&sub, &["rev-parse", "HEAD"]);
    git(
        &fixture.source,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            sub.to_str().unwrap(),
            "module",
        ],
    );
    let checkout = fixture.source.join("module");
    git(&checkout, &["checkout", "-q", &old]);
    git(&fixture.source, &["add", "."]);
    git(&fixture.source, &["commit", "-qm", "Base with module"]);
    fixture.base = git(&fixture.source, &["rev-parse", "HEAD"]);
    git(&checkout, &["checkout", "-q", &new]);
    git(&fixture.source, &["add", "module"]);
    git(&fixture.source, &["commit", "-qm", "Change module"]);
    fixture.change = git(&fixture.source, &["rev-parse", "HEAD"]);
    fixture.task("code-review", "approved");
    success(fixture.prepare());
    let diff = git(
        &fixture.destination,
        &["diff", "HEAD", "--submodule=short", "--", "module"],
    );
    assert!(
        diff.contains(&format!("+Subproject commit {new}")),
        "reviewed submodule change disappeared: {diff:?}"
    );
}

#[test]
fn preparation_ignores_source_zero_context_setting() {
    let mut fixture = Fixture::new();
    fs::write(fixture.source.join("code.txt"), "first\nbefore\nlast\n").unwrap();
    git(&fixture.source, &["add", "code.txt"]);
    git(&fixture.source, &["commit", "-qm", "Base with context"]);
    fixture.base = git(&fixture.source, &["rev-parse", "HEAD"]);
    fs::write(fixture.source.join("code.txt"), "first\nafter\nlast\n").unwrap();
    git(&fixture.source, &["commit", "-qam", "Change middle line"]);
    fixture.change = git(&fixture.source, &["rev-parse", "HEAD"]);
    git(&fixture.source, &["config", "diff.context", "0"]);
    fixture.task("code-review", "approved");
    success(fixture.prepare());
    assert_eq!(
        fs::read_to_string(fixture.destination.join("code.txt")).unwrap(),
        "first\nafter\nlast\n"
    );
}
