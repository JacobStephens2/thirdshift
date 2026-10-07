//! Acquisition through Worktree, with real local Git and origin.

use super::*;
use std::os::unix::fs::PermissionsExt;

const BRANCH: &str = "issue-7";

#[derive(Clone, Copy, Debug)]
enum AcquisitionKind {
    Fresh,
    ContinuationAbsent,
    ContinuationEqual,
    Review,
}

const KINDS: [AcquisitionKind; 4] = [
    AcquisitionKind::Fresh,
    AcquisitionKind::ContinuationAbsent,
    AcquisitionKind::ContinuationEqual,
    AcquisitionKind::Review,
];

impl AcquisitionKind {
    fn prepare(self) -> (tempfile::TempDir, Git, String) {
        let (temp, launch, issue_head) = continuation();
        if !matches!(self, Self::ContinuationEqual) {
            launch.run(&["branch", "-D", BRANCH]).unwrap();
        }
        let start = if matches!(self, Self::Fresh | Self::Review) {
            launch
                .run(&["rev-parse", "refs/remotes/origin/main"])
                .unwrap()
        } else {
            issue_head
        };
        (temp, launch, start)
    }

    fn acquire(self, launch: &Git) -> Result<()> {
        match self {
            Self::Fresh => Worktree::create_fresh(launch, "work", BRANCH, "main").map(drop),
            Self::ContinuationAbsent | Self::ContinuationEqual => {
                Worktree::continue_existing(launch, "work", BRANCH, "main").map(drop)
            }
            Self::Review => ReviewWorktree::create(launch, "work", "main").map(drop),
        }
    }

    fn path(self, temp: &tempfile::TempDir) -> PathBuf {
        temp.path().join(if matches!(self, Self::Review) {
            "work-architect"
        } else {
            "work-issue-7"
        })
    }
}

fn checkout_hook(temp: &tempfile::TempDir, script: &str) {
    let path = temp.path().join("hooks/post-checkout");
    std::fs::write(
        &path,
        format!("#!/bin/sh\nset -e\n{script}\necho 'checkout hook refused' >&2\nexit 1\n"),
    )
    .unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn clean_hook_failures_remove_only_the_attempts_checkout_and_branch() {
    for kind in KINDS {
        let (temp, launch, start) = kind.prepare();
        let registrations = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
        let refs = Git::new(temp.path().join("origin.git"))
            .run(&["show-ref"])
            .unwrap();
        let reflog_path = launch.dir().join(".git/logs/refs/heads/issue-7");
        let reflog = std::fs::read(&reflog_path).ok();
        checkout_hook(&temp, "");

        let error = kind.acquire(&launch).unwrap_err().to_string();

        assert!(error.starts_with("git worktree add"), "{kind:?}: {error}");
        assert!(error.contains("checkout hook refused"), "{kind:?}: {error}");
        assert!(!kind.path(&temp).exists(), "{kind:?}: {error}");
        assert_eq!(
            launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
            registrations
        );
        assert_eq!(
            Git::new(temp.path().join("origin.git"))
                .run(&["show-ref"])
                .unwrap(),
            refs
        );
        if matches!(kind, AcquisitionKind::ContinuationEqual) {
            assert_eq!(
                local_head(&launch, BRANCH).unwrap().as_deref(),
                Some(start.as_str())
            );
            assert_eq!(std::fs::read(reflog_path).ok(), reflog);
        } else {
            assert!(local_head(&launch, BRANCH).unwrap().is_none());
        }
    }
}

#[test]
fn ignored_hook_work_is_retained_and_named() {
    for kind in KINDS {
        let (temp, launch, start) = kind.prepare();
        std::fs::write(launch.dir().join(".git/info/exclude"), "ignored.txt\n").unwrap();
        checkout_hook(&temp, "echo 'ignored work' > ignored.txt");

        let error = kind.acquire(&launch).unwrap_err().to_string();

        let path = kind.path(&temp);
        assert_eq!(
            std::fs::read_to_string(path.join("ignored.txt")).unwrap(),
            "ignored work\n",
            "{kind:?}: {error}"
        );
        assert!(
            launch
                .run(&["worktree", "list", "--porcelain"])
                .unwrap()
                .contains(path.canonicalize().unwrap().to_str().unwrap())
        );
        assert!(
            error.contains("retaining checkout")
                && error.contains(&start)
                && error.contains("contains work"),
            "{kind:?}: {error}"
        );
        if !matches!(kind, AcquisitionKind::Review) {
            assert_eq!(
                local_head(&launch, BRANCH).unwrap().as_deref(),
                Some(start.as_str())
            );
            assert!(error.contains("refs/heads/issue-7"), "{error}");
        }
    }
}

fn continuation() -> (tempfile::TempDir, Git, String) {
    let (temp, launch) = super::tests::launch_directory();
    std::fs::write(launch.dir().join("tracked.txt"), "baseline\n").unwrap();
    launch.run(&["add", "tracked.txt"]).unwrap();
    launch
        .run(&["commit", "-q", "-m", "Tracked baseline"])
        .unwrap();
    launch.run(&["push", "-q", "origin", "main"]).unwrap();
    launch.run(&["checkout", "-q", "-b", BRANCH]).unwrap();
    launch
        .run(&["commit", "-q", "--allow-empty", "-m", "Earlier work"])
        .unwrap();
    launch.run(&["push", "-q", "origin", BRANCH]).unwrap();
    let head = launch.run(&["rev-parse", "HEAD"]).unwrap();
    launch.run(&["checkout", "-q", "main"]).unwrap();
    (temp, launch, head)
}

#[test]
fn tracked_staged_and_untracked_hook_work_survives_failed_acquisition() {
    for kind in KINDS {
        for (script, file, expected_status) in [
            (
                "echo 'hook work' > tracked.txt",
                "tracked.txt",
                " M tracked.txt",
            ),
            (
                "echo 'hook work' > tracked.txt\ngit add tracked.txt",
                "tracked.txt",
                "M  tracked.txt",
            ),
            (
                "echo 'hook work' > untracked.txt",
                "untracked.txt",
                "?? untracked.txt",
            ),
        ] {
            let (temp, launch, start) = kind.prepare();
            let origin = Git::new(temp.path().join("origin.git"));
            let refs = origin.run(&["show-ref"]).unwrap();
            checkout_hook(&temp, script);

            let error = kind.acquire(&launch).unwrap_err().to_string();

            let path = kind.path(&temp);
            assert_eq!(
                std::fs::read_to_string(path.join(file)).unwrap(),
                "hook work\n"
            );
            assert!(
                Git::new(&path)
                    .run(&["status", "--porcelain"])
                    .unwrap()
                    .contains(expected_status.trim()),
                "{kind:?}: {error}"
            );
            assert!(
                error.contains("retaining checkout")
                    && error.contains(&start)
                    && error.contains("contains work"),
                "{kind:?}: {error}"
            );
            assert_eq!(origin.run(&["show-ref"]).unwrap(), refs);
            if !matches!(kind, AcquisitionKind::Review) {
                assert_eq!(
                    local_head(&launch, BRANCH).unwrap().as_deref(),
                    Some(start.as_str())
                );
            }
        }
    }
}

#[test]
fn hook_commits_survive_including_detached_review_commits() {
    for kind in KINDS {
        let (temp, launch, start) = kind.prepare();
        checkout_hook(
            &temp,
            "echo 'committed hook work' > tracked.txt\ngit commit -q -am 'Hook work'",
        );

        let error = kind.acquire(&launch).unwrap_err().to_string();

        let git = Git::new(kind.path(&temp));
        let head = git.run(&["rev-parse", "HEAD"]).unwrap();
        assert_ne!(head, start);
        assert_eq!(git.run(&["log", "-1", "--format=%s"]).unwrap(), "Hook work");
        assert_eq!(
            std::fs::read_to_string(git.dir().join("tracked.txt")).unwrap(),
            "committed hook work\n"
        );
        assert!(
            error.contains(&head) && error.contains("HEAD changed"),
            "{kind:?}: {error}"
        );
        if !matches!(kind, AcquisitionKind::Review) {
            assert_eq!(
                local_head(&launch, BRANCH).unwrap().as_deref(),
                Some(head.as_str())
            );
        }
    }
}

#[test]
fn locked_or_unexpected_checkouts_survive_and_are_named() {
    for kind in KINDS {
        for (script, reason) in [
            (
                "git worktree lock \"$PWD\" --reason 'hook owns it'",
                "checkout is locked",
            ),
            (
                "git symbolic-ref HEAD refs/heads/main",
                "unexpected checkout identity",
            ),
        ] {
            let (temp, launch, start) = kind.prepare();
            checkout_hook(&temp, script);

            let error = kind.acquire(&launch).unwrap_err().to_string();

            let path = kind.path(&temp);
            assert!(path.exists(), "{kind:?}: {error}");
            assert!(
                launch
                    .run(&["worktree", "list", "--porcelain"])
                    .unwrap()
                    .contains(path.canonicalize().unwrap().to_str().unwrap())
            );
            assert!(
                error.contains("retaining checkout") && error.contains(reason),
                "{kind:?}: {error}"
            );
            if !matches!(kind, AcquisitionKind::Review) {
                assert_eq!(
                    local_head(&launch, BRANCH).unwrap().as_deref(),
                    Some(start.as_str())
                );
            }
        }
    }
}

#[test]
fn pre_existing_empty_directories_and_symlinks_retain_partial_registrations() {
    for kind in KINDS {
        for symlink in [false, true] {
            let (temp, launch, start) = kind.prepare();
            let path = kind.path(&temp);
            if symlink {
                let target = temp.path().join("existing-directory");
                std::fs::create_dir(&target).unwrap();
                std::os::unix::fs::symlink(target, &path).unwrap();
            } else {
                std::fs::create_dir(&path).unwrap();
            }
            checkout_hook(&temp, "");

            let error = kind.acquire(&launch).unwrap_err().to_string();

            assert!(path.exists(), "{kind:?}, symlink {symlink}: {error}");
            assert_eq!(
                std::fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                symlink
            );
            assert!(
                launch
                    .run(&["worktree", "list", "--porcelain"])
                    .unwrap()
                    .contains(path.canonicalize().unwrap().to_str().unwrap()),
                "{kind:?}, symlink {symlink}: {error}"
            );
            assert!(
                error.contains("retaining checkout")
                    && error.contains(&start)
                    && error.contains("existed before"),
                "{kind:?}, symlink {symlink}: {error}"
            );
            if !matches!(kind, AcquisitionKind::Review) {
                assert_eq!(
                    local_head(&launch, BRANCH).unwrap().as_deref(),
                    Some(start.as_str())
                );
            }
        }
    }
}

#[test]
fn inspection_and_removal_failures_retain_resources_without_replacing_the_add_error() {
    for kind in KINDS {
        for (script, reason) in [
            (
                "printf broken > \"$(git rev-parse --git-path index)\"",
                "index file smaller",
            ),
            ("chmod a-w .", "worktree remove"),
        ] {
            let (temp, launch, start) = kind.prepare();
            checkout_hook(&temp, script);

            let error = kind.acquire(&launch).unwrap_err();

            let path = kind.path(&temp);
            // Restore permission for the fixture's eventual TempDir cleanup.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            let message = error.to_string();
            assert!(path.join("tracked.txt").exists(), "{kind:?}: {message}");
            assert!(
                message.starts_with("git worktree add")
                    && message.contains("checkout hook refused")
                    && message.contains("retaining checkout")
                    && message.contains(reason)
                    && message.contains(&start),
                "{kind:?}: {message}"
            );
            assert!(
                error
                    .root_cause()
                    .to_string()
                    .contains("checkout hook refused")
            );
            if !matches!(kind, AcquisitionKind::Review) {
                assert_eq!(
                    local_head(&launch, BRANCH).unwrap().as_deref(),
                    Some(start.as_str())
                );
            }
        }
    }
}

#[test]
fn a_new_issue_branch_still_registered_elsewhere_is_retained_after_checkout_cleanup() {
    for kind in [AcquisitionKind::Fresh, AcquisitionKind::ContinuationAbsent] {
        let (temp, launch, start) = kind.prepare();
        checkout_hook(
            &temp,
            "git -c core.hooksPath=/dev/null worktree add --force ../manual-worktree issue-7",
        );

        let error = kind.acquire(&launch).unwrap_err().to_string();

        assert!(!kind.path(&temp).exists(), "{kind:?}: {error}");
        assert!(temp.path().join("manual-worktree/tracked.txt").exists());
        assert_eq!(
            local_head(&launch, BRANCH).unwrap().as_deref(),
            Some(start.as_str())
        );
        assert!(
            error.contains("retaining local branch issue-7")
                && error.contains("still registered at")
                && error.contains("manual-worktree"),
            "{kind:?}: {error}"
        );
    }
}

#[test]
fn a_rejected_branch_removal_keeps_the_branch_and_original_acquisition_cause() {
    let (temp, launch, start) = AcquisitionKind::Fresh.prepare();
    checkout_hook(&temp, "");
    let hook = temp.path().join("hooks/reference-transaction");
    std::fs::write(&hook, r#"#!/bin/sh
if test "$1" = prepared; then
  while read old new reference; do
    if test "$reference" = refs/heads/issue-7 && test "$new" = 0000000000000000000000000000000000000000; then
      echo 'ref cleanup refused' >&2
      exit 1
    fi
  done
fi
"#).unwrap();
    std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let error = AcquisitionKind::Fresh.acquire(&launch).unwrap_err();

    assert!(!AcquisitionKind::Fresh.path(&temp).exists());
    assert_eq!(
        local_head(&launch, BRANCH).unwrap().as_deref(),
        Some(start.as_str())
    );
    let message = error.to_string();
    assert!(
        message.contains("conditional removal failed") && message.contains("ref cleanup refused"),
        "{message}"
    );
    assert!(
        error
            .root_cause()
            .to_string()
            .contains("checkout hook refused")
    );
}

#[test]
fn acquisition_inspection_fails_before_add_when_a_registration_cannot_be_identified() {
    for kind in KINDS {
        let (temp, launch, start) = kind.prepare();
        let uncertain = launch.common_dir().unwrap().join("worktrees/uncertain");
        std::fs::create_dir_all(&uncertain).unwrap();
        std::fs::write(uncertain.join("HEAD"), "ref: refs/heads/issue-7\n").unwrap();
        // Missing gitdir: Git omits this registration from its listing. A lock
        // keeps fetch's automatic maintenance from pruning it before inspection.
        std::fs::write(uncertain.join("locked"), "retain damaged registration\n").unwrap();
        checkout_hook(&temp, "touch ../hook-ran");

        let error = kind.acquire(&launch).unwrap_err().to_string();

        assert!(!temp.path().join("hook-ran").exists(), "{kind:?}: {error}");
        assert!(!kind.path(&temp).exists());
        assert!(error.contains("cannot inspect"), "{kind:?}: {error}");
        assert_eq!(
            std::fs::read_to_string(uncertain.join("HEAD")).unwrap(),
            "ref: refs/heads/issue-7\n"
        );
        if matches!(kind, AcquisitionKind::ContinuationEqual) {
            assert_eq!(
                local_head(&launch, BRANCH).unwrap().as_deref(),
                Some(start.as_str())
            );
        } else {
            assert!(local_head(&launch, BRANCH).unwrap().is_none());
        }
    }
}

#[test]
fn a_path_inspection_error_occurs_before_any_add_mutation() {
    for kind in KINDS {
        let (temp, launch, _) = kind.prepare();
        let path = kind.path(&temp);
        std::os::unix::fs::symlink(&path, &path).unwrap();
        let refs = launch.run(&["show-ref"]).unwrap();
        let registered = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
        checkout_hook(&temp, "touch ../hook-ran");

        let error = kind.acquire(&launch).unwrap_err().to_string();

        assert!(error.contains("cannot resolve"), "{kind:?}: {error}");
        assert!(!temp.path().join("hook-ran").exists());
        assert!(
            std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        assert_eq!(
            launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
            registered
        );
    }
}

#[test]
fn a_ref_moved_between_inspection_and_deletion_survives() {
    const NAME: &str =
        "worktree::acquisition_tests::a_ref_moved_between_inspection_and_deletion_survives";
    if std::env::var_os("THIRDSHIFT_REAL_GIT").is_none() {
        // Isolate PATH in a child test. The shim performs one real local ref
        // movement immediately before forwarding the real deletion to Git.
        let temp = tempfile::TempDir::new().unwrap();
        let real_git = std::process::Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap();
        assert!(real_git.status.success());
        let shim = temp.path().join("git");
        std::fs::write(&shim, r#"#!/bin/sh
set -e
if test "$1" = update-ref && test "$2" = --no-deref && test "$3" = -d && test ! -e "$THIRDSHIFT_RACE_MARKER"; then
  touch "$THIRDSHIFT_RACE_MARKER"
  changed=$(printf 'Ref moved before deletion\n' | "$THIRDSHIFT_REAL_GIT" commit-tree "$5^{tree}" -p "$5")
  "$THIRDSHIFT_REAL_GIT" update-ref "$4" "$changed" "$5"
fi
exec "$THIRDSHIFT_REAL_GIT" "$@"
"#).unwrap();
        std::fs::set_permissions(shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let search = std::env::var_os("PATH").unwrap();
        let path = std::env::join_paths(
            std::iter::once(temp.path().to_path_buf()).chain(std::env::split_paths(&search)),
        )
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME, "--nocapture"])
            .env("PATH", path)
            .env(
                "THIRDSHIFT_REAL_GIT",
                String::from_utf8(real_git.stdout).unwrap().trim(),
            )
            .env("THIRDSHIFT_RACE_MARKER", temp.path().join("moved"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(temp.path().join("moved").exists());
        return;
    }
    let (temp, launch, start) = AcquisitionKind::Fresh.prepare();
    checkout_hook(&temp, "");

    let error = AcquisitionKind::Fresh.acquire(&launch).unwrap_err();

    assert!(!AcquisitionKind::Fresh.path(&temp).exists());
    let head = local_head(&launch, BRANCH)
        .unwrap()
        .expect("moved ref was deleted");
    assert_ne!(head, start);
    assert_eq!(
        launch.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
        "Ref moved before deletion"
    );
    assert!(
        error.to_string().contains("conditional removal failed")
            && error.to_string().contains(&head),
        "{error:#}"
    );
    assert!(
        error
            .root_cause()
            .to_string()
            .contains("checkout hook refused")
    );
}

#[test]
fn initialized_clean_submodules_are_retained_on_failure() {
    for kind in KINDS {
        let (temp, launch, _) = kind.prepare();
        let source = if matches!(kind, AcquisitionKind::Fresh | AcquisitionKind::Review) {
            "main"
        } else {
            BRANCH
        };
        if matches!(kind, AcquisitionKind::ContinuationAbsent) {
            launch
                .run(&[
                    "checkout",
                    "-q",
                    "-b",
                    BRANCH,
                    "refs/remotes/origin/issue-7",
                ])
                .unwrap();
        } else {
            launch.run(&["checkout", "-q", source]).unwrap();
        }
        launch
            .run(&[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                temp.path().join("origin.git").to_str().unwrap(),
                "child",
            ])
            .unwrap();
        launch
            .run(&["commit", "-q", "-am", "Add submodule"])
            .unwrap();
        launch.run(&["push", "-q", "origin", source]).unwrap();
        let start = launch.run(&["rev-parse", "HEAD"]).unwrap();
        launch.run(&["checkout", "-q", "main"]).unwrap();
        if matches!(kind, AcquisitionKind::ContinuationAbsent) {
            launch.run(&["branch", "-D", BRANCH]).unwrap();
        }
        checkout_hook(
            &temp,
            "git -c protocol.file.allow=always submodule update --init",
        );

        let error = kind.acquire(&launch).unwrap_err().to_string();

        let path = kind.path(&temp);
        assert!(path.join("child/tracked.txt").exists(), "{kind:?}: {error}");
        assert!(
            Git::new(&path)
                .run(&["status", "--porcelain"])
                .unwrap()
                .is_empty()
        );
        assert!(
            error.contains("initialized submodules") && error.contains(&start),
            "{kind:?}: {error}"
        );
    }
}

#[test]
fn tracked_work_hidden_by_index_flags_or_fsmonitor_survives_failure() {
    for kind in KINDS {
        for hiding in [
            "assume-unchanged",
            "skip-worktree",
            "ignorestat",
            "fsmonitor",
        ] {
            let (temp, launch, start) = kind.prepare();
            let script = match hiding {
                "ignorestat" => {
                    launch.run(&["config", "core.ignoreStat", "true"]).unwrap();
                    "echo 'hook work' > tracked.txt".to_string()
                }
                "fsmonitor" => {
                    let monitor = temp.path().join("hooks/fsmonitor");
                    std::fs::write(&monitor, "#!/bin/sh\nprintf 'token\\0'\n").unwrap();
                    std::fs::set_permissions(&monitor, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                    launch
                        .run(&["config", "core.fsmonitor", monitor.to_str().unwrap()])
                        .unwrap();
                    "git status --porcelain >/dev/null\necho 'hook work' > tracked.txt".to_string()
                }
                flag => {
                    format!("git update-index --{flag} tracked.txt\necho 'hook work' > tracked.txt")
                }
            };
            checkout_hook(&temp, &script);

            let error = kind.acquire(&launch).unwrap_err().to_string();

            assert_eq!(
                std::fs::read_to_string(kind.path(&temp).join("tracked.txt")).unwrap(),
                "hook work\n",
                "{kind:?}, {hiding}: {error}"
            );
            assert!(
                error.contains("retaining checkout") && error.contains(&start),
                "{kind:?}, {hiding}: {error}"
            );
            if !matches!(kind, AcquisitionKind::Review) {
                assert_eq!(
                    local_head(&launch, BRANCH).unwrap().as_deref(),
                    Some(start.as_str())
                );
            }
        }
    }
}

fn interrupted_add(test_name: &str, kind: AcquisitionKind, work: &str) {
    crate::test_support::with_recorded_signal(test_name, |signal| {
        let (temp, launch, start) = kind.prepare();
        let reflog_path = launch.dir().join(".git/logs/refs/heads/issue-7");
        let reflog = std::fs::read(&reflog_path).ok();
        checkout_hook(
            &temp,
            &format!(
                "{work}\nkill -{signal} {}\nexec sleep 3",
                std::process::id()
            ),
        );

        let error = kind.acquire(&launch).unwrap_err().to_string();

        assert!(error.starts_with("interrupted"), "{error}");
        assert!(crate::interrupt::requested());
        assert_eq!(
            launch.run(&["rev-parse", "HEAD"]).unwrap_err().to_string(),
            "interrupted"
        );
        let completion = launch.completion();
        if work.is_empty() {
            assert!(!kind.path(&temp).exists(), "{kind:?}: {error}");
            assert_eq!(
                completion
                    .run(&["worktree", "list", "--porcelain"])
                    .unwrap()
                    .matches("worktree ")
                    .count(),
                1
            );
            if matches!(kind, AcquisitionKind::ContinuationEqual) {
                assert_eq!(
                    local_head(&completion, BRANCH).unwrap().as_deref(),
                    Some(start.as_str())
                );
                assert_eq!(std::fs::read(reflog_path).ok(), reflog);
            } else {
                assert!(local_head(&completion, BRANCH).unwrap().is_none());
            }
        } else {
            assert!(kind.path(&temp).exists(), "{kind:?}: {error}");
            assert!(
                error.contains("retaining checkout") && error.contains(&start),
                "{kind:?}: {error}"
            );
            if !matches!(kind, AcquisitionKind::Review) {
                assert_eq!(
                    local_head(&completion, BRANCH).unwrap().as_deref(),
                    Some(start.as_str())
                );
            }
            if work.contains("hook work") {
                assert_eq!(
                    std::fs::read_to_string(kind.path(&temp).join("tracked.txt")).unwrap(),
                    "hook work\n"
                );
            }
        }
    });
}

#[test]
fn interrupted_fresh_add_cleans_up() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_fresh_add_cleans_up",
        AcquisitionKind::Fresh,
        "",
    );
}

#[test]
fn interrupted_absent_continuation_add_cleans_up() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_absent_continuation_add_cleans_up",
        AcquisitionKind::ContinuationAbsent,
        "",
    );
}

#[test]
fn interrupted_equal_continuation_add_cleans_up() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_equal_continuation_add_cleans_up",
        AcquisitionKind::ContinuationEqual,
        "",
    );
}

#[test]
fn interrupted_review_add_cleans_up() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_review_add_cleans_up",
        AcquisitionKind::Review,
        "",
    );
}

#[test]
fn interrupted_fresh_add_retains_work() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_fresh_add_retains_work",
        AcquisitionKind::Fresh,
        "echo 'hook work' > tracked.txt",
    );
}

#[test]
fn interrupted_absent_continuation_add_retains_work() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_absent_continuation_add_retains_work",
        AcquisitionKind::ContinuationAbsent,
        "echo 'hook work' > tracked.txt",
    );
}

#[test]
fn interrupted_equal_continuation_add_retains_work() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_equal_continuation_add_retains_work",
        AcquisitionKind::ContinuationEqual,
        "echo 'hook work' > tracked.txt",
    );
}

#[test]
fn interrupted_review_add_retains_work() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_review_add_retains_work",
        AcquisitionKind::Review,
        "echo 'hook work' > tracked.txt",
    );
}

#[test]
fn interrupted_add_retains_uncertain_checkout() {
    interrupted_add(
        "worktree::acquisition_tests::interrupted_add_retains_uncertain_checkout",
        AcquisitionKind::Fresh,
        "printf broken > \"$(git rev-parse --git-path index)\"",
    );
}

#[test]
fn continuation_refuses_local_only_work_without_selection_and_preserves_its_head() {
    let (temp, launch, origin_head) = continuation();
    launch.run(&["checkout", "-q", BRANCH]).unwrap();
    launch
        .run(&["commit", "-q", "--allow-empty", "-m", "Local only"])
        .unwrap();
    let local_head = launch.run(&["rev-parse", "HEAD"]).unwrap();
    launch.run(&["checkout", "-q", "main"]).unwrap();

    let result = Worktree::continue_existing(&launch, "work", BRANCH, "main");

    assert_eq!(launch.run(&["rev-parse", BRANCH]).unwrap(), local_head);
    assert!(!temp.path().join("work-issue-7").exists());
    assert_eq!(
        Git::new(temp.path().join("origin.git"))
            .run(&["rev-parse", BRANCH])
            .unwrap(),
        origin_head
    );
    assert_eq!(
        result.err().expect("acquisition should refuse").to_string(),
        "the local branch issue-7 differs from origin/issue-7; push, reset or delete it first"
    );
}

#[test]
fn fresh_acquisition_refuses_an_existing_local_branch_even_at_the_base_head() {
    let (temp, launch) = super::tests::launch_directory();
    let head = launch.run(&["rev-parse", "HEAD"]).unwrap();
    launch.run(&["branch", BRANCH]).unwrap();
    let worktrees = launch.run(&["worktree", "list", "--porcelain"]).unwrap();

    let result = Worktree::create_fresh(&launch, "work", BRANCH, "main");

    assert_eq!(launch.run(&["rev-parse", BRANCH]).unwrap(), head);
    assert!(!temp.path().join("work-issue-7").exists());
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        worktrees
    );
    assert_eq!(
        result.err().expect("acquisition should refuse").to_string(),
        "the local branch issue-7 is not on origin; push, rename or delete it first"
    );
}

#[test]
fn continuation_refuses_a_local_commit_added_after_successful_preflight() {
    let (temp, launch, origin_head) = continuation();
    check_local_branch(
        BRANCH,
        local_head(&launch, BRANCH).unwrap().as_deref(),
        Some(&origin_head),
    )
    .unwrap();
    launch.run(&["checkout", "-q", BRANCH]).unwrap();
    launch
        .run(&["commit", "-q", "--allow-empty", "-m", "After preflight"])
        .unwrap();
    let local_head = launch.run(&["rev-parse", "HEAD"]).unwrap();
    launch.run(&["checkout", "-q", "main"]).unwrap();
    let worktrees = launch.run(&["worktree", "list", "--porcelain"]).unwrap();

    let result = Worktree::continue_existing(&launch, "work", BRANCH, "main");

    assert_eq!(launch.run(&["rev-parse", BRANCH]).unwrap(), local_head);
    assert_eq!(
        launch.run(&["rev-parse", "origin/issue-7"]).unwrap(),
        origin_head
    );
    assert!(!temp.path().join("work-issue-7").exists());
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        worktrees
    );
    assert_eq!(
        result.err().expect("acquisition should refuse").to_string(),
        "the local branch issue-7 differs from origin/issue-7; push, reset or delete it first"
    );
}

#[test]
fn continuation_refuses_a_local_ancestor_when_origin_advances_after_preflight() {
    let (temp, launch, local_head) = continuation();
    check_local_branch(BRANCH, Some(&local_head), Some(&local_head)).unwrap();
    launch
        .run(&["checkout", "-q", "-b", "other-machine", BRANCH])
        .unwrap();
    launch
        .run(&["commit", "-q", "--allow-empty", "-m", "Origin advanced"])
        .unwrap();
    let origin_head = launch.run(&["rev-parse", "HEAD"]).unwrap();
    launch.run(&["checkout", "-q", "main"]).unwrap();
    let origin = Git::new(temp.path().join("origin.git"));
    origin
        .run(&[
            "fetch",
            launch.dir().to_str().unwrap(),
            "other-machine:issue-7",
        ])
        .unwrap();
    assert_eq!(
        launch.run(&["rev-parse", "origin/issue-7"]).unwrap(),
        local_head
    );
    let worktrees = launch.run(&["worktree", "list", "--porcelain"]).unwrap();

    let result = Worktree::continue_existing(&launch, "work", BRANCH, "main");

    assert_eq!(launch.run(&["rev-parse", BRANCH]).unwrap(), local_head);
    assert_eq!(
        launch.run(&["rev-parse", "origin/issue-7"]).unwrap(),
        origin_head
    );
    assert_eq!(origin.run(&["rev-parse", BRANCH]).unwrap(), origin_head);
    assert_eq!(
        launch.run(&["rev-parse", "other-machine"]).unwrap(),
        origin_head
    );
    assert!(!temp.path().join("work-issue-7").exists());
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        worktrees
    );
    assert_eq!(
        result.err().expect("acquisition should refuse").to_string(),
        "the local branch issue-7 differs from origin/issue-7; push, reset or delete it first"
    );
}

#[test]
fn continuation_attaches_an_equal_local_branch_without_resetting_and_cleans_up() {
    let (temp, launch, head) = continuation();
    let reflog = launch
        .run(&["reflog", "show", "--format=%H %gs", BRANCH])
        .unwrap();

    let worktree = Worktree::continue_existing(&launch, "work", BRANCH, "main").unwrap();

    assert_eq!(worktree.head().unwrap(), head);
    assert_eq!(worktree.branch(), BRANCH);
    assert_eq!(
        worktree.path(),
        temp.path().join("work-issue-7").canonicalize().unwrap()
    );
    assert_eq!(launch.run(&["rev-parse", BRANCH]).unwrap(), head);
    assert_eq!(
        launch
            .run(&["reflog", "show", "--format=%H %gs", BRANCH])
            .unwrap(),
        reflog
    );
    drop(worktree);
    assert!(!temp.path().join("work-issue-7").exists());
    assert!(local_head(&launch, BRANCH).unwrap().is_none());
    assert_eq!(
        Git::new(temp.path().join("origin.git"))
            .run(&["rev-parse", BRANCH])
            .unwrap(),
        head
    );
}

#[test]
fn absent_issue_branches_are_created_at_the_fetched_start_and_cleaned_up() {
    for continuing in [false, true] {
        let (temp, launch, head) = continuation();
        launch.run(&["branch", "-D", BRANCH]).unwrap();
        let expected = if continuing {
            head
        } else {
            launch.run(&["rev-parse", "origin/main"]).unwrap()
        };

        let worktree = if continuing {
            Worktree::continue_existing(&launch, "work", BRANCH, "main")
        } else {
            Worktree::create_fresh(&launch, "work", BRANCH, "main")
        }
        .unwrap();

        assert_eq!(worktree.head().unwrap(), expected);
        assert_eq!(launch.run(&["rev-parse", BRANCH]).unwrap(), expected);
        assert_eq!(
            Git::new(worktree.path())
                .run(&["symbolic-ref", "--short", "HEAD"])
                .unwrap(),
            BRANCH
        );
        assert_eq!(
            worktree.path(),
            temp.path().join("work-issue-7").canonicalize().unwrap()
        );
        drop(worktree);
        assert!(!temp.path().join("work-issue-7").exists());
        assert!(local_head(&launch, BRANCH).unwrap().is_none());
    }
}

#[test]
fn occupied_paths_survive_add_failure_with_pre_existing_branch_heads_untouched() {
    for (continuing, local_exists) in [(false, false), (true, false), (true, true)] {
        let (temp, launch, issue_head) = continuation();
        let reflog_path = launch.dir().join(".git/logs/refs/heads/issue-7");
        let reflog = std::fs::read(&reflog_path).unwrap();
        if !local_exists {
            launch.run(&["branch", "-D", BRANCH]).unwrap();
        }
        let main_head = launch.run(&["rev-parse", "main"]).unwrap();
        let worktrees = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
        let occupied = temp.path().join("work-issue-7");
        std::fs::create_dir(&occupied).unwrap();
        std::fs::write(occupied.join("local.txt"), "keep this work\n").unwrap();

        let result = if continuing {
            Worktree::continue_existing(&launch, "work", BRANCH, "main")
        } else {
            Worktree::create_fresh(&launch, "work", BRANCH, "main")
        };

        assert_eq!(launch.run(&["rev-parse", "main"]).unwrap(), main_head);
        if local_exists {
            assert_eq!(launch.run(&["rev-parse", BRANCH]).unwrap(), issue_head);
            assert_eq!(std::fs::read(reflog_path).unwrap(), reflog);
        } else {
            assert!(local_head(&launch, BRANCH).unwrap().is_none());
        }
        assert_eq!(
            Git::new(temp.path().join("origin.git"))
                .run(&["rev-parse", BRANCH])
                .unwrap(),
            issue_head
        );
        assert_eq!(
            std::fs::read_to_string(occupied.join("local.txt")).unwrap(),
            "keep this work\n"
        );
        assert_eq!(std::fs::read_dir(&occupied).unwrap().count(), 1);
        assert_eq!(
            launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
            worktrees
        );
        let error = result
            .err()
            .expect("occupied path should refuse")
            .to_string();
        assert!(error.starts_with("git worktree add"), "{error}");
        assert!(error.contains("already exists"), "{error}");
    }
}

#[test]
fn a_branch_in_another_worktree_is_refused_without_touching_its_edits() {
    let (temp, launch, head) = continuation();
    let existing = temp.path().join("manual-worktree");
    launch
        .run(&["worktree", "add", existing.to_str().unwrap(), BRANCH])
        .unwrap();
    std::fs::write(existing.join("wip.txt"), "manual work\n").unwrap();
    let worktrees = launch.run(&["worktree", "list", "--porcelain"]).unwrap();

    let result = Worktree::continue_existing(&launch, "work", BRANCH, "main");

    assert_eq!(launch.run(&["rev-parse", BRANCH]).unwrap(), head);
    assert_eq!(
        Git::new(&existing).run(&["rev-parse", "HEAD"]).unwrap(),
        head
    );
    assert_eq!(
        std::fs::read_to_string(existing.join("wip.txt")).unwrap(),
        "manual work\n"
    );
    assert!(!temp.path().join("work-issue-7").exists());
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        worktrees
    );
    let error = result
        .err()
        .expect("checked-out branch should refuse")
        .to_string();
    assert!(error.starts_with("git worktree add"), "{error}");
    assert!(
        error.contains("already checked out") || error.contains("already used by worktree"),
        "{error}"
    );
}

#[test]
fn acquisition_pins_origin_even_when_local_branches_shadow_remote_tracking_names() {
    let (temp, launch, issue_head) = continuation();
    let base_head = launch.run(&["rev-parse", "main"]).unwrap();
    launch.run(&["branch", "origin/issue-7", "main"]).unwrap();
    launch.run(&["branch", "origin/main", BRANCH]).unwrap();

    let continued = Worktree::continue_existing(&launch, "work", BRANCH, "main").unwrap();
    assert_eq!(continued.head().unwrap(), issue_head);
    drop(continued);

    let fresh = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    assert_eq!(fresh.head().unwrap(), base_head);
    drop(fresh);

    let review = ReviewWorktree::create(&launch, "work", "main").unwrap();
    assert_eq!(
        Git::new(review.path()).run(&["rev-parse", "HEAD"]).unwrap(),
        base_head
    );
    drop(review);

    assert_eq!(
        launch
            .run(&["rev-parse", "refs/heads/origin/issue-7"])
            .unwrap(),
        base_head
    );
    assert_eq!(
        launch
            .run(&["rev-parse", "refs/heads/origin/main"])
            .unwrap(),
        issue_head
    );
    assert!(!temp.path().join("work-issue-7").exists());
    assert!(local_head(&launch, BRANCH).unwrap().is_none());
}
