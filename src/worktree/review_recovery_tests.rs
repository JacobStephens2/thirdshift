//! Review retries through acquisition, with real Git and damaged resources.

use std::fs;
use std::os::unix::fs::symlink;

use super::*;

struct StaleReview {
    _temp: tempfile::TempDir,
    launch: Git,
    path: PathBuf,
    admin: PathBuf,
    head: String,
}

impl StaleReview {
    fn create() -> Self {
        let (temp, launch) = super::tests::launch_directory();
        let owner = ReviewWorktree::create(&launch, "work", "main").unwrap();
        let path = owner.path().to_path_buf();
        let git = Git::new(&path);
        let admin = PathBuf::from(git.run(&["rev-parse", "--absolute-git-dir"]).unwrap());
        let head = git.run(&["rev-parse", "HEAD"]).unwrap();
        fs::write(path.join("retained.txt"), "review scratch\n").unwrap();
        // Abandon the owner without scope-exit cleanup, as process death does.
        std::mem::forget(owner);
        Self {
            _temp: temp,
            launch,
            path,
            admin,
            head,
        }
    }

    fn refuses(&self, reason: &str) {
        let registrations = self
            .launch
            .run(&["worktree", "list", "--porcelain"])
            .unwrap();
        let refs = self.launch.run(&["show-ref"]).unwrap();
        let error = ReviewWorktree::create(&self.launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(self.path.to_str().unwrap()) && error.contains(reason),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(self.path.join("retained.txt")).unwrap(),
            "review scratch\n"
        );
        assert_eq!(
            Git::new(&self.path).run(&["rev-parse", "HEAD"]).unwrap(),
            self.head
        );
        assert_eq!(
            self.launch
                .run(&["worktree", "list", "--porcelain"])
                .unwrap(),
            registrations
        );
        assert_eq!(self.launch.run(&["show-ref"]).unwrap(), refs);
    }
}

#[test]
fn incomplete_or_ambiguous_disposal_evidence_retains_successful_review_scratch() {
    for damage in [
        "missing-record",
        "missing-token",
        "malformed",
        "version",
        "duplicate",
        "unknown",
        "missing-field",
        "wrong-type",
        "nonce",
        "record-link",
        "token-link",
        "record-directory",
        "token-fifo",
    ] {
        let fixture = StaleReview::create();
        let record = fixture.admin.join("thirdshift-review.json");
        let token = fixture.path.join(".thirdshift-review-token");
        match damage {
            "missing-record" => fs::remove_file(&record).unwrap(),
            "missing-token" => fs::remove_file(&token).unwrap(),
            "malformed" => fs::write(&record, "unfinished record").unwrap(),
            "duplicate" => {
                let text = fs::read_to_string(&record).unwrap();
                fs::write(&record, text.replacen('{', "{\"version\":1,", 1)).unwrap();
            }
            "version" | "unknown" | "missing-field" | "wrong-type" => {
                // Mutate the filesystem evidence, then observe only acquisition.
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
                match damage {
                    "version" => value["version"] = 999.into(),
                    "unknown" => value["unrecognized"] = true.into(),
                    "wrong-type" => value["root"]["inode"] = "ambiguous".into(),
                    _ => {
                        value.as_object_mut().unwrap().remove("nonce");
                    }
                }
                fs::write(&record, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "nonce" => fs::write(&token, "another acquisition").unwrap(),
            "record-link" | "token-link" => {
                let target = if damage == "record-link" {
                    &record
                } else {
                    &token
                };
                let saved = target.with_extension("saved");
                fs::rename(target, &saved).unwrap();
                symlink(&saved, target).unwrap();
            }
            "record-directory" => {
                fs::remove_file(&record).unwrap();
                fs::create_dir(&record).unwrap();
            }
            "token-fifo" => {
                fs::remove_file(&token).unwrap();
                assert!(
                    std::process::Command::new("mkfifo")
                        .arg(&token)
                        .status()
                        .unwrap()
                        .success()
                );
            }
            _ => unreachable!(),
        }
        fixture.refuses("retaining checkout");
    }
}

#[test]
fn replaced_directories_or_changed_registration_identity_do_not_inherit_disposal() {
    for change in [
        "root",
        "admin",
        "common",
        "locked",
        "attached",
        "git-link",
        "backlink-link",
        "backlink",
        "root-link",
        "admin-link",
        "missing-root",
        "damaged-registration",
    ] {
        let fixture = StaleReview::create();
        let saved = fixture.path.with_extension("saved");
        match change {
            "root" => {
                fs::rename(&fixture.path, &saved).unwrap();
                fs::create_dir(&fixture.path).unwrap();
                // Even copying both Git link and token cannot transfer authority.
                for file in [".git", ".thirdshift-review-token", "retained.txt"] {
                    fs::copy(saved.join(file), fixture.path.join(file)).unwrap();
                }
            }
            "admin" => {
                let saved_admin = fixture.admin.with_extension("saved");
                fs::rename(&fixture.admin, &saved_admin).unwrap();
                fs::create_dir(&fixture.admin).unwrap();
                for entry in fs::read_dir(&saved_admin).unwrap() {
                    let entry = entry.unwrap();
                    if entry.file_type().unwrap().is_file() {
                        fs::copy(entry.path(), fixture.admin.join(entry.file_name())).unwrap();
                    }
                }
                // Keep the displaced admin directory outside Git's listing.
                fs::rename(saved_admin, fixture._temp.path().join("saved-admin")).unwrap();
            }
            "common" => {
                let record = fixture.admin.join("thirdshift-review.json");
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
                value["common"]["inode"] = 0.into();
                fs::write(record, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "locked" => {
                fixture
                    .launch
                    .run(&["worktree", "lock", fixture.path.to_str().unwrap()])
                    .unwrap();
            }
            "attached" => {
                Git::new(&fixture.path)
                    .run(&["checkout", "-q", "-b", "manual-review"])
                    .unwrap();
            }
            "git-link" => {
                fs::rename(fixture.path.join(".git"), fixture.path.join("saved-git")).unwrap();
                symlink(fixture.path.join("saved-git"), fixture.path.join(".git")).unwrap();
            }
            "backlink-link" => {
                let backlink = fixture.admin.join("gitdir");
                let saved = fixture.admin.join("saved-gitdir");
                fs::rename(&backlink, &saved).unwrap();
                symlink(saved, backlink).unwrap();
            }
            "backlink" => fs::write(
                fixture.admin.join("gitdir"),
                fixture.launch.dir().join(".git").to_str().unwrap(),
            )
            .unwrap(),
            "root-link" | "missing-root" => {
                fs::rename(&fixture.path, &saved).unwrap();
                if change == "root-link" {
                    symlink(&saved, &fixture.path).unwrap();
                }
            }
            "admin-link" => {
                let saved_admin = fixture._temp.path().join("saved-admin");
                fs::rename(&fixture.admin, &saved_admin).unwrap();
                symlink(&saved_admin, &fixture.admin).unwrap();
            }
            "damaged-registration" => fs::remove_file(fixture.admin.join("gitdir")).unwrap(),
            _ => unreachable!(),
        }
        if matches!(
            change,
            "missing-root" | "admin-link" | "damaged-registration" | "backlink"
        ) {
            let refs = fixture.launch.run(&["show-ref"]).unwrap();
            let error = ReviewWorktree::create(&fixture.launch, "work", "main")
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("retaining checkout")
                    && error.contains(fixture.path.to_str().unwrap()),
                "{change}: {error}"
            );
            assert!(
                fixture.admin.symlink_metadata().is_ok(),
                "{change}: {error}"
            );
            let preserved = if change == "missing-root" {
                &saved
            } else {
                &fixture.path
            };
            assert_eq!(
                fs::read_to_string(preserved.join("retained.txt")).unwrap(),
                "review scratch\n"
            );
            assert_eq!(fixture.launch.run(&["show-ref"]).unwrap(), refs);
        } else {
            fixture.refuses("retaining checkout");
        }
    }
}

#[test]
fn legacy_paths_and_dangling_links_are_retained_before_add() {
    for legacy in ["directory", "registered", "dangling-link"] {
        let (temp, launch) = super::tests::launch_directory();
        let path = temp.path().join("work-architect");
        match legacy {
            "registered" => {
                launch
                    .run(&[
                        "worktree",
                        "add",
                        "--detach",
                        path.to_str().unwrap(),
                        "HEAD",
                    ])
                    .unwrap();
            }
            "directory" => fs::create_dir(&path).unwrap(),
            _ => symlink(temp.path().join("absent"), &path).unwrap(),
        }
        let registrations = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
        let error = ReviewWorktree::create(&launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(path.to_str().unwrap()) && error.contains("retaining checkout"),
            "{legacy}: {error}"
        );
        assert!(fs::symlink_metadata(&path).is_ok());
        assert_eq!(
            launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
            registrations
        );
    }
}

#[test]
fn live_review_cleanup_does_not_require_restart_evidence() {
    for missing in ["token", "record", "both"] {
        let (_temp, launch) = super::tests::launch_directory();
        let owner = ReviewWorktree::create(&launch, "work", "main").unwrap();
        let path = owner.path().to_path_buf();
        let admin = PathBuf::from(
            Git::new(&path)
                .run(&["rev-parse", "--absolute-git-dir"])
                .unwrap(),
        );
        if missing != "record" {
            fs::remove_file(path.join(".thirdshift-review-token")).unwrap();
        }
        if missing != "token" {
            fs::remove_file(admin.join("thirdshift-review.json")).unwrap();
        }
        fs::write(path.join("scratch.txt"), "disposable scratch\n").unwrap();
        drop(owner);
        assert!(!path.exists());
        assert_eq!(
            launch
                .run(&["worktree", "list", "--porcelain"])
                .unwrap()
                .matches("worktree ")
                .count(),
            1
        );
    }
}

#[test]
fn process_death_with_prepared_markers_does_not_authorize_a_review_retry() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    const NAME: &str = "worktree::review_recovery_tests::process_death_with_prepared_markers_does_not_authorize_a_review_retry";
    if let Some(launch) = std::env::var_os("THIRDSHIFT_REVIEW_CRASH_LAUNCH") {
        let _owner = ReviewWorktree::create(&Git::new(launch), "work", "main").unwrap();
        panic!("fixture failed to stop acquisition before publication");
    }
    let (temp, launch) = super::tests::launch_directory();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(real_git.status.success());
    let gate = temp.path().join("prepared");
    let release = temp.path().join("release");
    let shim = bin.join("git");
    fs::write(&shim, r#"#!/bin/sh
set -e
if test "$1" = rev-parse && test "$2" = --symbolic-full-name && test -f .thirdshift-review-token; then
  printf 'scratch before publication\n' > retained.txt
  touch "$THIRDSHIFT_REVIEW_CRASH_GATE"
  while test ! -f "$THIRDSHIFT_REVIEW_CRASH_RELEASE"; do sleep 0.02; done
fi
exec "$THIRDSHIFT_REAL_GIT" "$@"
"#).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    let search = std::env::var_os("PATH").unwrap();
    let path_env =
        std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(&search))).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture"])
        .env("PATH", path_env)
        .env(
            "THIRDSHIFT_REAL_GIT",
            String::from_utf8(real_git.stdout).unwrap().trim(),
        )
        .env("THIRDSHIFT_REVIEW_CRASH_LAUNCH", launch.dir())
        .env("THIRDSHIFT_REVIEW_CRASH_GATE", &gate)
        .env("THIRDSHIFT_REVIEW_CRASH_RELEASE", &release)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !gate.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let prepared = gate.exists();
    // Release and drain the recorded child even when the barrier was missed.
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
    fs::write(&release, "release\n").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(prepared, "child never prepared markers: {output:?}");
    let path = temp.path().join("work-architect");
    let head = Git::new(&path).run(&["rev-parse", "HEAD"]).unwrap();
    let registrations = launch.run(&["worktree", "list", "--porcelain"]).unwrap();

    let error = ReviewWorktree::create(&launch, "work", "main")
        .err()
        .unwrap()
        .to_string();

    assert!(
        error.contains("retaining checkout") && error.contains(&head),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(path.join("retained.txt")).unwrap(),
        "scratch before publication\n"
    );
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        registrations
    );
}

#[test]
fn recorded_interruption_leaves_an_eligible_stale_review_untouched() {
    crate::test_support::with_recorded_signal(
        "worktree::review_recovery_tests::recorded_interruption_leaves_an_eligible_stale_review_untouched",
        |signal| {
            let fixture = StaleReview::create();
            let registrations = fixture
                .launch
                .run(&["worktree", "list", "--porcelain"])
                .unwrap();
            signal_hook::low_level::raise(signal).unwrap();
            let error = ReviewWorktree::create(&fixture.launch, "work", "main")
                .err()
                .unwrap()
                .to_string();
            assert_eq!(error, "interrupted");
            assert_eq!(
                fs::read_to_string(fixture.path.join("retained.txt")).unwrap(),
                "review scratch\n"
            );
            assert_eq!(
                fixture
                    .launch
                    .completion()
                    .run(&["worktree", "list", "--porcelain"])
                    .unwrap(),
                registrations
            );
            assert!(crate::interrupt::requested());
        },
    );
}

#[test]
fn token_creation_never_overwrites_existing_files_links_or_tracked_tokens() {
    use std::os::unix::fs::PermissionsExt;
    for collision in ["file", "link", "tracked"] {
        let (temp, launch) = super::tests::launch_directory();
        let path = temp.path().join("work-architect");
        let target = temp.path().join("token-target");
        fs::write(&target, "existing token contents\n").unwrap();
        if collision == "tracked" {
            fs::write(
                launch.dir().join(".thirdshift-review-token"),
                "tracked token contents\n",
            )
            .unwrap();
            launch.run(&["add", ".thirdshift-review-token"]).unwrap();
            launch
                .run(&["commit", "-q", "-m", "Tracked marker collision"])
                .unwrap();
            launch.run(&["push", "origin", "main"]).unwrap();
        } else {
            let hook = temp.path().join("hooks/post-checkout");
            let create = if collision == "file" {
                "printf 'existing token contents\\n' > .thirdshift-review-token"
            } else {
                "ln -s ../token-target .thirdshift-review-token"
            };
            fs::write(
                &hook,
                format!("#!/bin/sh\nset -e\n{create}\nprintf 'hook scratch\\n' > retained.txt\n"),
            )
            .unwrap();
            fs::set_permissions(hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let error = ReviewWorktree::create(&launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(if collision == "tracked" {
                "tracked"
            } else {
                "cannot exclusively create review token"
            }),
            "{collision}: {error}"
        );
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "existing token contents\n"
        );
        if collision == "tracked" {
            assert!(!path.exists());
            assert_eq!(
                fs::read_to_string(launch.dir().join(".thirdshift-review-token")).unwrap(),
                "tracked token contents\n"
            );
        } else {
            assert_eq!(
                fs::read_to_string(path.join(".thirdshift-review-token")).unwrap(),
                "existing token contents\n"
            );
            assert_eq!(
                fs::read_to_string(path.join("retained.txt")).unwrap(),
                "hook scratch\n"
            );
            assert_eq!(
                fs::symlink_metadata(path.join(".thirdshift-review-token"))
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                collision == "link"
            );
        }
    }
}

#[test]
fn stale_recovery_and_new_acquisition_wait_for_the_repository_worktree_lock() {
    use std::time::Duration;
    let fixture = StaleReview::create();
    let held = lock_launch(&fixture.launch).unwrap();
    let original_token = fs::read(fixture.path.join(".thirdshift-review-token")).unwrap();
    let launch = Git::new(fixture.launch.dir());
    let acquiring =
        std::thread::spawn(move || ReviewWorktree::create(&launch, "work", "main").unwrap());
    std::thread::sleep(Duration::from_millis(300));
    let retained_while_locked = fs::read_to_string(fixture.path.join("retained.txt")).ok();
    let token_while_locked = fs::read(fixture.path.join(".thirdshift-review-token")).ok();
    drop(held);
    let next = acquiring.join().unwrap();
    assert_eq!(retained_while_locked.as_deref(), Some("review scratch\n"));
    assert_eq!(token_while_locked, Some(original_token));
    assert!(!next.path().join("retained.txt").exists());
    drop(next);
    assert!(!fixture.path.exists());
}

#[test]
fn a_missing_registered_review_reports_its_available_head_and_retains_registration() {
    let fixture = StaleReview::create();
    let saved = fixture.path.with_extension("saved");
    fs::rename(&fixture.path, &saved).unwrap();
    let registrations = fixture
        .launch
        .run(&["worktree", "list", "--porcelain"])
        .unwrap();
    let error = ReviewWorktree::create(&fixture.launch, "work", "main")
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains(&fixture.head), "{error}");
    assert_eq!(
        fs::read_to_string(saved.join("retained.txt")).unwrap(),
        "review scratch\n"
    );
    assert_eq!(
        fixture
            .launch
            .run(&["worktree", "list", "--porcelain"])
            .unwrap(),
        registrations
    );
}

#[test]
fn a_repository_negating_the_token_exclusion_refuses_review_acquisition() {
    let (temp, launch) = super::tests::launch_directory();
    fs::write(
        launch.dir().join(".gitignore"),
        "!/.thirdshift-review-token\n",
    )
    .unwrap();
    launch.run(&["add", ".gitignore"]).unwrap();
    launch
        .run(&["commit", "-q", "-m", "Negate private token exclusion"])
        .unwrap();
    launch.run(&["push", "origin", "main"]).unwrap();
    let registrations = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();

    let error = ReviewWorktree::create(&launch, "work", "main")
        .err()
        .unwrap()
        .to_string();

    assert!(
        error.contains("token") && error.contains("excluded"),
        "{error}"
    );
    assert!(!temp.path().join("work-architect").exists());
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        registrations
    );
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
}
