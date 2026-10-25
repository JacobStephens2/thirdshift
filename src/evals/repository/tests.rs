use serde_json::json;

use super::*;

#[test]
fn spec_review_accepts_branch_below_base_seed() {
    let fixture = Fixture::new("spec-review", "stable", "base/issue-7");
    fixture.prepare().unwrap();
}

struct Fixture {
    root: tempfile::TempDir,
    task: Task,
    later: String,
}

impl Fixture {
    fn new(session: &str, base_branch: &str, issue_branch: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        git(&source, &["init", "--template="]).unwrap();
        git(&source, &["config", "user.name", "Source"]).unwrap();
        git(&source, &["config", "user.email", "source@example.invalid"]).unwrap();
        fs::write(source.join("code.txt"), "Base code\n").unwrap();
        git(&source, &["add", "."]).unwrap();
        git(&source, &["commit", "-m", "Base"]).unwrap();
        let base = git(&source, &["rev-parse", "HEAD"]).unwrap();
        fs::write(source.join("code.txt"), "Reviewed code\n").unwrap();
        git(&source, &["commit", "-am", "Original change"]).unwrap();
        let change = git(&source, &["rev-parse", "HEAD"]).unwrap();
        fs::write(source.join("answer.txt"), "Later answer\n").unwrap();
        git(&source, &["add", "."]).unwrap();
        git(&source, &["commit", "-m", "Later fix"]).unwrap();
        let later = git(&source, &["rev-parse", "HEAD"]).unwrap();
        fs::write(root.path().join("issue.md"), "Original issue\n").unwrap();
        fs::write(root.path().join("overlay.txt"), "Trial input overlay\n").unwrap();
        let task = serde_json::from_value(json!({
            "format": 1, "state": "approved", "session": session,
            "repository": "source", "base": base, "change": change,
            "criteria": "Review the change", "inputs": {
                ".thirdshift-issue.md": "issue.md", "code.txt": "overlay.txt"
            },
            "trial": {"issue_url": "https://github.com/acme/widgets/issues/7",
                "base": base_branch, "branch": issue_branch,
                "pr_url": "https://github.com/acme/widgets/pull/8"}
        }))
        .unwrap();
        Self { root, task, later }
    }

    fn destination(&self) -> PathBuf {
        // Canonical, as the prepared repository is: macOS temp dirs sit
        // under the /var symlink.
        self.root.path().canonicalize().unwrap().join("prepared")
    }

    fn prepare(&self) -> Result<Prepared> {
        prepare_trial(self.root.path(), &self.task, &self.destination())
    }

    fn assert_unpublished(&self) {
        assert!(!self.destination().exists());
        assert!(fs::read_dir(self.root.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".thirdshift-eval-")
        }));
    }
}

#[test]
fn complete_trial_repository_preserves_reviewed_patch_inputs_and_custom_branches() {
    for session in ["code-review", "spec-review"] {
        for (base_branch, issue_branch) in [
            ("release/stable", "changes/issue-7"),
            ("stable", "base"),
            ("base/stable", "changes/issue-7"),
            ("stable", "base/issue-7"),
        ] {
            let fixture = Fixture::new(session, base_branch, issue_branch);
            let prepared = fixture.prepare().unwrap();
            assert_eq!(prepared.repository, fixture.destination());
            let repo = &prepared.repository;
            assert_eq!(
                git(repo, &["branch", "--show-current"]).unwrap(),
                issue_branch
            );
            assert_eq!(
                git(repo, &["rev-parse", base_branch]).unwrap(),
                fixture.task.base
            );
            assert_eq!(
                git(repo, &["rev-parse", "HEAD^"]).unwrap(),
                fixture.task.base
            );
            assert_eq!(
                git(
                    repo,
                    &["rev-list", "--count", &format!("{base_branch}..HEAD")]
                )
                .unwrap(),
                "1"
            );
            assert_eq!(
                git(repo, &["show", "HEAD:code.txt"]).unwrap(),
                "Reviewed code"
            );
            assert_eq!(
                git(repo, &["ls-tree", "--name-only", "HEAD"]).unwrap(),
                "code.txt"
            );
            assert_eq!(git(repo, &["diff", "--cached"]).unwrap(), "");
            assert_eq!(
                fs::read_to_string(repo.join("code.txt")).unwrap(),
                "Trial input overlay\n"
            );
            assert_eq!(
                fs::read_to_string(repo.join(".thirdshift-issue.md")).unwrap(),
                "Original issue\n"
            );
            assert_eq!(
                git(repo, &["check-ignore", ".thirdshift-issue.md"]).unwrap(),
                ".thirdshift-issue.md"
            );
            assert_eq!(
                git(repo, &["show", "-s", "--format=%an <%ae>%n%B", "HEAD"]).unwrap(),
                "Eval <eval@example.invalid>\nEval change"
            );
            for oid in [&fixture.task.change, &fixture.later] {
                assert!(
                    git(repo, &["cat-file", "-e", oid]).is_err(),
                    "source commit survived: {oid}"
                );
            }
            if session == "code-review" {
                assert!(prepared.origin.is_none());
                assert_eq!(git(repo, &["remote"]).unwrap(), "");
            } else {
                let origin = prepared.origin.as_ref().unwrap();
                assert_eq!(origin, &fixture.destination().join(".git/eval-origin.git"));
                assert_eq!(
                    git(repo, &["remote", "get-url", "origin"]).unwrap(),
                    origin.to_str().unwrap()
                );
                assert_eq!(
                    git(origin, &["rev-parse", "--is-bare-repository"]).unwrap(),
                    "true"
                );
                for branch in [base_branch, issue_branch] {
                    let reference = format!("refs/heads/{branch}");
                    assert_eq!(
                        git(origin, &["rev-parse", &reference]).unwrap(),
                        git(repo, &["rev-parse", &reference]).unwrap()
                    );
                }
                assert_eq!(
                    git(origin, &["symbolic-ref", "HEAD"]).unwrap(),
                    format!("refs/heads/{base_branch}")
                );
                fs::remove_dir_all(fixture.root.path().join("source")).unwrap();
                git(
                    repo,
                    &[
                        "-c",
                        "user.name=Eval",
                        "-c",
                        "user.email=eval@example.invalid",
                        "commit",
                        "--allow-empty",
                        "-m",
                        "Later local review",
                    ],
                )
                .unwrap();
                git(repo, &["push", "origin", issue_branch]).unwrap();
                assert_eq!(
                    git(origin, &["rev-parse", issue_branch]).unwrap(),
                    git(repo, &["rev-parse", "HEAD"]).unwrap()
                );
                for oid in [&fixture.task.change, &fixture.later] {
                    assert!(git(origin, &["cat-file", "-e", oid]).is_err());
                }
            }
        }
    }
}

#[test]
fn trial_preparation_rejects_missing_invalid_and_identical_branch_inputs() {
    for session in ["code-review", "spec-review"] {
        for (base_branch, issue_branch) in [
            ("bad..base", "issue-7"),
            ("stable", "bad..issue"),
            ("same", "same"),
        ] {
            let fixture = Fixture::new(session, base_branch, issue_branch);
            assert!(fixture.prepare().is_err());
            fixture.assert_unpublished();
        }
        let mut fixture = Fixture::new(session, "stable", "issue-7");
        fixture.task.trial = None;
        assert!(
            fixture
                .prepare()
                .err()
                .unwrap()
                .to_string()
                .contains("missing Trial inputs")
        );
        fixture.assert_unpublished();
    }
}

#[test]
fn trial_preparation_cleans_up_failures_and_refuses_existing_destinations() {
    for session in ["code-review", "spec-review"] {
        let mut fixture = Fixture::new(session, "stable", "issue-7");
        fixture
            .task
            .inputs
            .insert("missing.txt".into(), "absent.txt".into());
        assert!(fixture.prepare().is_err());
        fixture.assert_unpublished();

        // Both names are valid, but Git cannot create overlapping refs. This
        // fails after import and verification, while still in staging.
        let fixture = Fixture::new(session, "review", "review/issue-7");
        assert!(fixture.prepare().is_err());
        fixture.assert_unpublished();

        let fixture = Fixture::new(session, "stable", "issue-7");
        fs::create_dir(fixture.destination()).unwrap();
        let sentinel = fixture.destination().join("sentinel");
        fs::write(&sentinel, "Keep this directory\n").unwrap();
        assert!(
            fixture
                .prepare()
                .err()
                .unwrap()
                .to_string()
                .contains("already exists")
        );
        assert_eq!(
            fs::read_to_string(sentinel).unwrap(),
            "Keep this directory\n"
        );
    }
}
