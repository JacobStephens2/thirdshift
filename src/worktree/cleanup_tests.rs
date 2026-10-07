//! Successful lifetime cleanup through acquisition and scope exit, with real Git.

use super::*;
use std::fs;

const BRANCH: &str = "issue-7";

#[test]
fn a_sibling_fetch_waits_for_worktree_creation_to_finish() {
    if isolated(
        "a_sibling_fetch_waits_for_worktree_creation_to_finish",
        r#"
if test "$1" = worktree && test "$2" = add && test "$4" = issue-22; then
  "$THIRDSHIFT_REAL_GIT" "$@"
  common=$("$THIRDSHIFT_REAL_GIT" rev-parse --git-common-dir)
  head="$common/worktrees/work-issue-22/HEAD"
  cp "$head" "$THIRDSHIFT_FAULT_MARKER.saved"
  printf '0000000000000000000000000000000000000000\n' > "$head"
  touch "$THIRDSHIFT_FAULT_MARKER"
  for _ in $(seq 500); do
    test -e "$THIRDSHIFT_FAULT_MARKER.release" && break
    sleep 0.01
  done
  mv "$THIRDSHIFT_FAULT_MARKER.saved" "$head"
  test -e "$THIRDSHIFT_FAULT_MARKER.release"
  exit
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let reader = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    let (finished_during_creation, merged, sibling) = std::thread::scope(|scope| {
        let creator = scope.spawn(|| Worktree::create_fresh(&launch, "work", "issue-22", "main"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (sent, received) = std::sync::mpsc::channel();
        let reader = &reader;
        let fetcher = scope.spawn(move || {
            let merged = reader.merge_base_branch("main");
            sent.send(()).unwrap();
            merged
        });
        let finished = received
            .recv_timeout(std::time::Duration::from_millis(500))
            .is_ok();
        fs::write(marker.with_extension("release"), "released").unwrap();
        let merged = fetcher.join().unwrap();
        let sibling = creator.join().unwrap();
        (finished, merged, sibling)
    });
    sibling.unwrap();
    assert!(
        marker.exists(),
        "creation never reached the controlled transition"
    );
    assert!(
        !finished_during_creation,
        "fetch observed an incomplete Worktree: {merged:?}"
    );
    assert!(matches!(merged.unwrap(), Merge::Clean { .. }));
}

#[test]
fn a_sibling_fetch_waits_for_worktree_disposal_to_finish() {
    if isolated(
        "a_sibling_fetch_waits_for_worktree_disposal_to_finish",
        r#"
if test "$1" = worktree && test "$2" = remove && test "${4##*/}" = work-issue-22; then
  touch "$THIRDSHIFT_FAULT_MARKER"
  for _ in $(seq 500); do
    test -e "$THIRDSHIFT_FAULT_MARKER.release" && break
    sleep 0.01
  done
  test -e "$THIRDSHIFT_FAULT_MARKER.release"
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let reader = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let sibling = Worktree::create_fresh(&launch, "work", "issue-22", "main").unwrap();
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    let (finished_during_disposal, merged) = std::thread::scope(|scope| {
        let disposer = scope.spawn(|| drop(sibling));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let (sent, received) = std::sync::mpsc::channel();
        let reader = &reader;
        let fetcher = scope.spawn(move || {
            let merged = reader.merge_base_branch("main");
            sent.send(()).unwrap();
            merged
        });
        let finished = received
            .recv_timeout(std::time::Duration::from_millis(500))
            .is_ok();
        fs::write(marker.with_extension("release"), "released").unwrap();
        let merged = fetcher.join().unwrap();
        disposer.join().unwrap();
        (finished, merged)
    });
    assert!(
        marker.exists(),
        "disposal never reached the controlled transition"
    );
    assert!(
        !finished_during_disposal,
        "fetch overlapped Worktree disposal: {merged:?}"
    );
    assert!(matches!(merged.unwrap(), Merge::Clean { .. }));
    assert!(local_head(&launch, "issue-22").unwrap().is_none());
}

#[test]
fn an_old_owner_retains_a_recreated_checkout_even_with_the_same_paths_branch_and_head() {
    if isolated(
        "an_old_owner_retains_a_recreated_checkout_even_with_the_same_paths_branch_and_head",
        "",
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let original = Git::new(&path);
    let head = original.run(&["rev-parse", "HEAD"]).unwrap();
    let admin = original.run(&["rev-parse", "--absolute-git-dir"]).unwrap();
    launch
        .run(&["worktree", "remove", "--force", path.to_str().unwrap()])
        .unwrap();
    launch
        .run(&["worktree", "add", path.to_str().unwrap(), BRANCH])
        .unwrap();
    assert_eq!(original.run(&["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(
        original.run(&["rev-parse", "--absolute-git-dir"]).unwrap(),
        admin
    );
    fs::write(path.join("replacement.txt"), "replacement work\n").unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();
    let registrations = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
    expect_warning(&path, Some(BRANCH), &head, "replaced");

    drop(owner);

    assert_eq!(
        fs::read_to_string(path.join("replacement.txt")).unwrap(),
        "replacement work\n"
    );
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        registrations
    );
}

#[test]
fn an_old_review_owner_retains_a_recreated_detached_checkout() {
    if isolated(
        "an_old_review_owner_retains_a_recreated_detached_checkout",
        "",
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = ReviewWorktree::create(&launch, "work", "main").unwrap();
    let path = owner.path().to_path_buf();
    let git = Git::new(&path);
    let head = git.run(&["rev-parse", "HEAD"]).unwrap();
    let admin = git.run(&["rev-parse", "--absolute-git-dir"]).unwrap();
    launch
        .run(&["worktree", "remove", "--force", path.to_str().unwrap()])
        .unwrap();
    launch
        .run(&["worktree", "add", "--detach", path.to_str().unwrap(), &head])
        .unwrap();
    assert_eq!(
        git.run(&["rev-parse", "--absolute-git-dir"]).unwrap(),
        admin
    );
    fs::write(path.join("replacement.txt"), "review replacement\n").unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();
    let registrations = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
    expect_warning(&path, None, &head, "replaced");

    drop(owner);

    assert_eq!(
        fs::read_to_string(path.join("replacement.txt")).unwrap(),
        "review replacement\n"
    );
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        registrations
    );
    let retry = ReviewWorktree::create(&launch, "work", "main")
        .err()
        .unwrap()
        .to_string();
    assert!(
        retry.contains("retaining checkout") && retry.contains(&head),
        "{retry}"
    );
    assert_eq!(
        fs::read_to_string(path.join("replacement.txt")).unwrap(),
        "review replacement\n"
    );
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        registrations
    );
}

#[test]
fn ordinary_cleanup_removes_tracking_configuration_only_for_the_owned_issue_branch() {
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    launch
        .run(&["config", "branch.issue-7.remote", "origin"])
        .unwrap();
    launch
        .run(&["config", "branch.issue-7.merge", "refs/heads/issue-7"])
        .unwrap();
    launch
        .run(&["config", "branch.Issue-7.remote", "Keep case"])
        .unwrap();
    launch
        .run(&["config", "branch.issue-7.child.remote", "Keep nested"])
        .unwrap();

    drop(owner);

    assert!(!path.exists());
    assert!(local_head(&launch, BRANCH).unwrap().is_none());
    assert!(
        launch
            .run_optional(&["config", "--get", "branch.issue-7.remote"])
            .unwrap()
            .is_none()
    );
    assert!(
        launch
            .run_optional(&["config", "--get", "branch.issue-7.merge"])
            .unwrap()
            .is_none()
    );
    assert_eq!(
        launch.run(&["config", "branch.Issue-7.remote"]).unwrap(),
        "Keep case"
    );
    assert_eq!(
        launch
            .run(&["config", "branch.issue-7.child.remote"])
            .unwrap(),
        "Keep nested"
    );
}

/// Isolate PATH and capture progress at the same process seam as fault tests.
/// The shim changes real Git state at a precise observation/mutation boundary.
fn isolated(name: &str, script: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    let name = format!("worktree::cleanup_tests::{name}");
    if std::env::var("THIRDSHIFT_CLEANUP_TEST").as_deref() == Ok(&name) {
        return false;
    }
    let temp = tempfile::TempDir::new().unwrap();
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(real_git.status.success());
    let shim = temp.path().join("git");
    fs::write(
        &shim,
        format!("#!/bin/sh\nset -e\n{script}\nexec \"$THIRDSHIFT_REAL_GIT\" \"$@\"\n"),
    )
    .unwrap();
    fs::set_permissions(shim, fs::Permissions::from_mode(0o755)).unwrap();
    let search = std::env::var_os("PATH").unwrap();
    let path = std::env::join_paths(
        std::iter::once(temp.path().to_path_buf()).chain(std::env::split_paths(&search)),
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env("PATH", path)
        .env("THIRDSHIFT_CLEANUP_TEST", &name)
        .env(
            "THIRDSHIFT_REAL_GIT",
            String::from_utf8(real_git.stdout).unwrap().trim(),
        )
        .env("THIRDSHIFT_FAULT_MARKER", temp.path().join("fault"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    let warnings: Vec<_> = stderr
        .lines()
        .filter(|line| line.contains("warning:"))
        .collect();
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        if let Some(expected) = line.strip_prefix("warning includes: ") {
            assert!(
                warnings
                    .iter()
                    .any(|warning| expected.split('\t').all(|field| warning.contains(field))),
                "missing {expected:?} in {stderr}"
            );
        }
    }
    true
}

fn expect_warning(path: &Path, branch: Option<&str>, head: &str, reason: &str) {
    println!(
        "warning includes: {}\t{}\t{head}\t{reason}",
        path.display(),
        branch.unwrap_or("detached HEAD")
    );
}

fn registration(launch: &Git) -> String {
    launch.run(&["worktree", "list", "--porcelain"]).unwrap()
}

fn assert_retained(launch: &Git, path: &Path, head: &str) {
    assert_eq!(
        fs::read_to_string(path.join("work.txt")).unwrap(),
        "owned work\n"
    );
    assert_eq!(local_head(launch, BRANCH).unwrap().as_deref(), Some(head));
    assert!(registration(launch).contains(path.to_str().unwrap()));
}

#[test]
fn advanced_run_commits_and_detached_review_scratch_are_still_disposable() {
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    fs::write(path.join("committed.txt"), "run commit\n").unwrap();
    let git = Git::new(&path);
    git.run(&["add", "committed.txt"]).unwrap();
    git.run(&["commit", "-qm", "Run work"]).unwrap();
    fs::write(path.join("work.txt"), "scratch\n").unwrap();
    drop(owner);
    assert!(!path.exists());
    assert!(local_head(&launch, BRANCH).unwrap().is_none());
    let owner = ReviewWorktree::create(&launch, "work", "main").unwrap();
    let path = owner.path().to_path_buf();
    let git = Git::new(&path);
    git.run(&["commit", "-q", "--allow-empty", "-m", "Review experiment"])
        .unwrap();
    fs::write(path.join("work.txt"), "scratch\n").unwrap();
    drop(owner);
    assert!(!path.exists());
    assert_eq!(registration(&launch).matches("worktree ").count(), 1);
}

#[test]
fn replaced_checkout_or_administration_directories_are_retained_and_named() {
    if isolated(
        "replaced_checkout_or_administration_directories_are_retained_and_named",
        "",
    ) {
        return;
    }
    for admin_replaced in [false, true] {
        let (temp, launch) = super::tests::launch_directory();
        let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
        let path = owner.path().to_path_buf();
        let head = owner.head().unwrap();
        fs::write(path.join("work.txt"), "owned work\n").unwrap();
        let target = if admin_replaced {
            PathBuf::from(
                Git::new(&path)
                    .run(&["rev-parse", "--absolute-git-dir"])
                    .unwrap(),
            )
        } else {
            path.clone()
        };
        let saved = temp.path().join("original-directory");
        fs::rename(&target, &saved).unwrap();
        copy_directory(&saved, &target);
        let refs = launch.run(&["show-ref"]).unwrap();
        let registered = registration(&launch);
        expect_warning(
            &path,
            Some(BRANCH),
            &head,
            if admin_replaced {
                "administrative directory identity"
            } else {
                "checkout directory identity"
            },
        );

        drop(owner);

        assert_retained(&launch, &path, &head);
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        assert_eq!(registration(&launch), registered);
        assert_eq!(
            fs::read_to_string(if admin_replaced {
                path.join("work.txt")
            } else {
                saved.join("work.txt")
            })
            .unwrap(),
            "owned work\n"
        );
    }
}

fn copy_directory(source: &Path, target: &Path) {
    fs::create_dir(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[test]
fn symlink_substitutions_are_retained_without_touching_the_target() {
    if isolated(
        "symlink_substitutions_are_retained_without_touching_the_target",
        "",
    ) {
        return;
    }
    for substitution in ["checkout", "administration", "git-link"] {
        let (temp, launch) = super::tests::launch_directory();
        let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
        let path = owner.path().to_path_buf();
        let head = owner.head().unwrap();
        fs::write(path.join("work.txt"), "owned work\n").unwrap();
        let target = match substitution {
            "checkout" => path.clone(),
            "administration" => PathBuf::from(
                Git::new(&path)
                    .run(&["rev-parse", "--absolute-git-dir"])
                    .unwrap(),
            ),
            _ => path.join(".git"),
        };
        let saved = temp.path().join("original");
        fs::rename(&target, &saved).unwrap();
        std::os::unix::fs::symlink(&saved, &target).unwrap();
        expect_warning(&path, Some(BRANCH), &head, "symlink");

        drop(owner);

        assert!(
            fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            local_head(&launch, BRANCH).unwrap().as_deref(),
            Some(head.as_str())
        );
        let work = if substitution == "checkout" {
            saved.join("work.txt")
        } else {
            path.join("work.txt")
        };
        assert_eq!(fs::read_to_string(work).unwrap(), "owned work\n");
    }
}

#[test]
fn locked_or_changed_checkout_identity_is_retained() {
    if isolated("locked_or_changed_checkout_identity_is_retained", "") {
        return;
    }
    for change in ["lock", "detach", "branch", "attach-review"] {
        let (_temp, launch) = super::tests::launch_directory();
        let (path, run, review) = if change == "attach-review" {
            let owner = ReviewWorktree::create(&launch, "work", "main").unwrap();
            (owner.path().to_path_buf(), None, Some(owner))
        } else {
            let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
            (owner.path().to_path_buf(), Some(owner), None)
        };
        let git = Git::new(&path);
        let head = git.run(&["rev-parse", "HEAD"]).unwrap();
        fs::write(path.join("work.txt"), "owned work\n").unwrap();
        match change {
            "lock" => {
                launch
                    .run(&["worktree", "lock", path.to_str().unwrap()])
                    .unwrap();
            }
            "detach" => {
                git.run(&["checkout", "-q", "--detach"]).unwrap();
            }
            _ => {
                git.run(&["checkout", "-q", "-b", "manual"]).unwrap();
            }
        }
        let refs = launch.run(&["show-ref"]).unwrap();
        let registered = registration(&launch);
        expect_warning(
            &path,
            if run.is_some() { Some(BRANCH) } else { None },
            &head,
            if change == "lock" {
                "locked"
            } else {
                "identity changed"
            },
        );

        drop(run);
        drop(review);

        assert_eq!(
            fs::read_to_string(path.join("work.txt")).unwrap(),
            "owned work\n"
        );
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        assert_eq!(registration(&launch), registered);
    }
}

#[test]
fn missing_original_checkout_retains_the_ref_without_deleting_by_name() {
    if isolated(
        "missing_original_checkout_retains_the_ref_without_deleting_by_name",
        "",
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    launch
        .run(&["worktree", "remove", "--force", path.to_str().unwrap()])
        .unwrap();
    expect_warning(
        &path,
        Some(BRANCH),
        &head,
        "cannot inspect original directory",
    );

    drop(owner);

    assert!(!path.exists());
    assert_eq!(
        local_head(&launch, BRANCH).unwrap().as_deref(),
        Some(head.as_str())
    );
    assert_eq!(registration(&launch).matches("worktree ").count(), 1);
}

#[test]
fn damaged_or_inconsistent_registration_evidence_retains_local_work() {
    if isolated(
        "damaged_or_inconsistent_registration_evidence_retains_local_work",
        "",
    ) {
        return;
    }
    for damage in [
        "missing-gitdir",
        "wrong-backlink",
        "broken-head",
        "broken-commondir",
        "unlisted-registration",
        "duplicate-registration",
    ] {
        let (_temp, launch) = super::tests::launch_directory();
        let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
        let path = owner.path().to_path_buf();
        let head = owner.head().unwrap();
        fs::write(path.join("work.txt"), "owned work\n").unwrap();
        let admin = PathBuf::from(
            Git::new(&path)
                .run(&["rev-parse", "--absolute-git-dir"])
                .unwrap(),
        );
        match damage {
            "missing-gitdir" => fs::remove_file(admin.join("gitdir")).unwrap(),
            "wrong-backlink" => fs::write(
                admin.join("gitdir"),
                launch
                    .dir()
                    .join(".git")
                    .canonicalize()
                    .unwrap()
                    .to_str()
                    .unwrap(),
            )
            .unwrap(),
            "broken-head" => fs::write(admin.join("HEAD"), "broken\n").unwrap(),
            "broken-commondir" => fs::write(admin.join("commondir"), "missing\n").unwrap(),
            "duplicate-registration" => copy_directory(&admin, &admin.with_file_name("duplicate")),
            _ => {
                let broken = launch.common_dir().unwrap().join("worktrees/uncertain");
                fs::create_dir(&broken).unwrap();
                fs::write(broken.join("HEAD"), "ref: refs/heads/issue-7\n").unwrap();
            }
        }
        expect_warning(
            &path,
            Some(BRANCH),
            &head,
            if damage == "wrong-backlink" || damage == "duplicate-registration" {
                "duplicate registration"
            } else {
                "cannot inspect"
            },
        );

        drop(owner);

        assert_eq!(
            fs::read_to_string(path.join("work.txt")).unwrap(),
            "owned work\n"
        );
        assert_eq!(
            local_head(&launch, BRANCH).unwrap().as_deref(),
            Some(head.as_str())
        );
        assert!(admin.exists());
    }
}

#[test]
fn cleanup_lock_failure_retains_checkout_and_branch() {
    if isolated("cleanup_lock_failure_retains_checkout_and_branch", "") {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    fs::write(path.join("work.txt"), "owned work\n").unwrap();
    let lock = launch
        .common_dir()
        .unwrap()
        .join("thirdshift-worktrees.lock");
    fs::remove_file(&lock).unwrap();
    fs::create_dir(&lock).unwrap();
    expect_warning(
        &path,
        Some(BRANCH),
        &head,
        "cannot establish cleanup worktree lock",
    );

    drop(owner);

    assert_retained(&launch, &path, &head);
    assert!(lock.is_dir());
}

#[test]
fn cleanup_waits_for_the_repository_lock_until_its_owner_releases_it() {
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let held = lock_launch(&launch).unwrap();
    let (finished, completion) = std::sync::mpsc::channel();
    let dropping = std::thread::spawn(move || {
        drop(owner);
        finished.send(()).unwrap();
    });
    assert!(
        completion
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err()
    );
    assert!(path.exists());
    assert!(local_head(&launch, BRANCH).unwrap().is_some());
    drop(held);
    dropping.join().unwrap();
    completion.recv().unwrap();
    assert!(!path.exists());
    assert!(local_head(&launch, BRANCH).unwrap().is_none());
}

#[test]
fn checkout_removal_failure_retains_the_branch_and_configuration() {
    if isolated(
        "checkout_removal_failure_retains_the_branch_and_configuration",
        r#"
if test "$1" = worktree && test "$2" = remove; then
  echo 'checkout removal refused' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    fs::write(path.join("work.txt"), "owned work\n").unwrap();
    launch
        .run(&["config", "branch.issue-7.remote", "origin"])
        .unwrap();
    expect_warning(&path, Some(BRANCH), &head, "checkout removal refused");

    drop(owner);

    assert_retained(&launch, &path, &head);
    assert_eq!(
        launch.run(&["config", "branch.issue-7.remote"]).unwrap(),
        "origin"
    );
}

#[test]
fn unreadable_registration_listing_retains_checkout_and_ref() {
    if isolated(
        "unreadable_registration_listing_retains_checkout_and_ref",
        r#"
if test "$1" = worktree && test "$2" = list && test -e "$THIRDSHIFT_FAULT_MARKER"; then
  echo 'registration inspection refused' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    fs::write(path.join("work.txt"), "owned work\n").unwrap();
    fs::write(
        std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap(),
        "armed",
    )
    .unwrap();
    expect_warning(
        &path,
        Some(BRANCH),
        &head,
        "registration inspection refused",
    );

    drop(owner);

    fs::remove_file(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap()).unwrap();
    assert_retained(&launch, &path, &head);
}

#[test]
fn a_branch_used_elsewhere_survives_successful_checkout_removal() {
    if isolated(
        "a_branch_used_elsewhere_survives_successful_checkout_removal",
        "",
    ) {
        return;
    }
    let (temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    let other = temp.path().join("manual");
    launch
        .run(&[
            "worktree",
            "add",
            "--force",
            other.to_str().unwrap(),
            BRANCH,
        ])
        .unwrap();
    fs::write(other.join("manual.txt"), "manual work\n").unwrap();
    expect_warning(&path, Some(BRANCH), &head, "still registered at");

    drop(owner);

    assert!(!path.exists());
    assert_eq!(
        local_head(&launch, BRANCH).unwrap().as_deref(),
        Some(head.as_str())
    );
    assert!(registration(&launch).contains(other.to_str().unwrap()));
    assert_eq!(
        fs::read_to_string(other.join("manual.txt")).unwrap(),
        "manual work\n"
    );
}

#[test]
fn incomplete_registration_inspection_after_checkout_removal_retains_the_ref() {
    if isolated(
        "incomplete_registration_inspection_after_checkout_removal_retains_the_ref",
        r#"
if test "$1" = worktree && test "$2" = remove; then
  "$THIRDSHIFT_REAL_GIT" "$@"
  admin=$("$THIRDSHIFT_REAL_GIT" rev-parse --git-common-dir)/worktrees/uncertain
  mkdir -p "$admin"
  printf 'ref: refs/heads/issue-7\n' > "$admin/HEAD"
  exit 0
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    expect_warning(
        &path,
        Some(BRANCH),
        &head,
        "cannot inspect every registration",
    );

    drop(owner);

    assert!(!path.exists());
    assert_eq!(
        local_head(&launch, BRANCH).unwrap().as_deref(),
        Some(head.as_str())
    );
    assert!(
        launch
            .common_dir()
            .unwrap()
            .join("worktrees/uncertain")
            .exists()
    );
}

#[test]
fn ref_movement_immediately_before_conditional_deletion_survives() {
    if isolated(
        "ref_movement_immediately_before_conditional_deletion_survives",
        r#"
if test "$1" = update-ref && test "$2" = --no-deref && test "$3" = -d && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  changed=$(printf 'Moved before lifetime deletion\n' | "$THIRDSHIFT_REAL_GIT" commit-tree "$5^{tree}" -p "$5")
  "$THIRDSHIFT_REAL_GIT" update-ref "$4" "$changed" "$5"
  printf '%s' "$changed" > "$THIRDSHIFT_FAULT_MARKER"
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    launch
        .run(&["config", "branch.issue-7.remote", "origin"])
        .unwrap();

    drop(owner);

    let changed = fs::read_to_string(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap()).unwrap();
    assert_ne!(changed, head);
    expect_warning(&path, Some(BRANCH), &changed, "conditional removal failed");
    assert!(!path.exists());
    assert_eq!(
        local_head(&launch, BRANCH).unwrap().as_deref(),
        Some(changed.as_str())
    );
    assert_eq!(
        launch.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
        "Moved before lifetime deletion"
    );
    assert_eq!(
        launch.run(&["config", "branch.issue-7.remote"]).unwrap(),
        "origin"
    );
}

#[test]
fn changed_branch_configuration_survives_ref_deletion() {
    if isolated(
        "changed_branch_configuration_survives_ref_deletion",
        r#"
if test "$1" = update-ref && test "$2" = --no-deref && test "$3" = -d; then
  "$THIRDSHIFT_REAL_GIT" "$@"
  config=$("$THIRDSHIFT_REAL_GIT" rev-parse --git-common-dir)/config
  case "$THIRDSHIFT_CONFIG_CHANGE" in
    newline) "$THIRDSHIFT_REAL_GIT" config branch.issue-7.description 'changed
with newline' ;;
    empty) printf '[branch "issue-7"]\n\tflag =\n' >> "$config" ;;
    valueless) printf '[branch "issue-7"]\n\tflag\n' >> "$config" ;;
    order)
      "$THIRDSHIFT_REAL_GIT" config --unset-all branch.issue-7.remote
      "$THIRDSHIFT_REAL_GIT" config --add branch.issue-7.remote second
      "$THIRDSHIFT_REAL_GIT" config --add branch.issue-7.remote first ;;
  esac
  exit 0
fi
"#,
    ) {
        return;
    }
    for change in ["newline", "empty", "valueless", "order"] {
        // Environment changes are confined to this isolated, single-test child.
        unsafe {
            std::env::set_var("THIRDSHIFT_CONFIG_CHANGE", change);
        }
        let (_temp, launch) = super::tests::launch_directory();
        let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
        let path = owner.path().to_path_buf();
        let head = owner.head().unwrap();
        launch
            .run(&["config", "--add", "branch.issue-7.remote", "first"])
            .unwrap();
        launch
            .run(&["config", "--add", "branch.issue-7.remote", "second"])
            .unwrap();
        expect_warning(
            &path,
            Some(BRANCH),
            &head,
            "configuration changed since cleanup snapshot",
        );

        drop(owner);

        assert!(!path.exists());
        assert!(local_head(&launch, BRANCH).unwrap().is_none());
        let config = launch
            .run(&["config", "--local", "--null", "--list"])
            .unwrap();
        assert!(config.contains("branch.issue-7.remote\n"));
        match change {
            "newline" => {
                assert!(config.contains("branch.issue-7.description\nchanged\nwith newline\0"))
            }
            "empty" => assert!(config.contains("branch.issue-7.flag\n\0")),
            "valueless" => assert!(config.contains("branch.issue-7.flag\0")),
            _ => assert_eq!(
                launch
                    .run(&["config", "--get-all", "branch.issue-7.remote"])
                    .unwrap(),
                "second\nfirst"
            ),
        }
    }
}

#[test]
fn a_ref_recreated_before_configuration_cleanup_keeps_its_configuration() {
    if isolated(
        "a_ref_recreated_before_configuration_cleanup_keeps_its_configuration",
        r#"
if test "$1" = update-ref && test "$2" = --no-deref && test "$3" = -d; then
  "$THIRDSHIFT_REAL_GIT" "$@"
  "$THIRDSHIFT_REAL_GIT" update-ref "$4" "$5"
  touch "$THIRDSHIFT_FAULT_MARKER"
  exit 0
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    launch
        .run(&["config", "branch.issue-7.remote", "origin"])
        .unwrap();
    expect_warning(&path, Some(BRANCH), &head, "ref was recreated");

    drop(owner);

    assert!(!path.exists());
    assert!(PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap()).exists());
    assert_eq!(
        local_head(&launch, BRANCH).unwrap().as_deref(),
        Some(head.as_str())
    );
    assert_eq!(
        launch.run(&["config", "branch.issue-7.remote"]).unwrap(),
        "origin"
    );
}

#[test]
fn ownership_capture_failure_uses_acquisition_recovery_for_every_lifetime() {
    if isolated(
        "ownership_capture_failure_uses_acquisition_recovery_for_every_lifetime",
        r#"
if test "$1" = rev-parse && test "$2" = --absolute-git-dir; then
  if test "$THIRDSHIFT_CAPTURE_WORK" = yes; then printf 'capture work\n' > work.txt; fi
  echo 'ownership capture refused' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    for work in [false, true] {
        unsafe {
            std::env::set_var("THIRDSHIFT_CAPTURE_WORK", if work { "yes" } else { "no" });
        }
        for kind in ["fresh", "absent", "equal", "review"] {
            let (temp, launch) = super::tests::launch_directory();
            let head = launch.run(&["rev-parse", "HEAD"]).unwrap();
            if kind == "absent" || kind == "equal" {
                launch.run(&["branch", BRANCH]).unwrap();
                launch.run(&["push", "origin", BRANCH]).unwrap();
                if kind == "absent" {
                    launch.run(&["branch", "-D", BRANCH]).unwrap();
                }
            }
            let path = temp.path().join(if kind == "review" {
                "work-architect"
            } else {
                "work-issue-7"
            });
            let result = match kind {
                "fresh" => Worktree::create_fresh(&launch, "work", BRANCH, "main").map(drop),
                "review" => ReviewWorktree::create(&launch, "work", "main").map(drop),
                _ => Worktree::continue_existing(&launch, "work", BRANCH, "main").map(drop),
            };
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("ownership capture refused"),
                "{kind}: {error}"
            );
            if work {
                assert_eq!(
                    fs::read_to_string(path.join("work.txt")).unwrap(),
                    "capture work\n"
                );
                assert!(
                    error.contains("retaining checkout") && error.contains(&head),
                    "{error}"
                );
                assert!(registration(&launch).contains(path.to_str().unwrap()));
            } else {
                assert!(!path.exists());
                assert_eq!(registration(&launch).matches("worktree ").count(), 1);
            }
            if (work && kind != "review") || kind == "equal" {
                assert_eq!(
                    local_head(&launch, BRANCH).unwrap().as_deref(),
                    Some(head.as_str())
                );
            } else {
                assert!(local_head(&launch, BRANCH).unwrap().is_none());
            }
        }
    }
}

#[test]
fn recorded_interruption_allows_both_lifetimes_to_finish_cleanup() {
    crate::test_support::with_recorded_signal(
        "worktree::cleanup_tests::recorded_interruption_allows_both_lifetimes_to_finish_cleanup",
        |signal| {
            let (_temp, launch) = super::tests::launch_directory();
            let run = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
            let review = ReviewWorktree::create(&launch, "work", "main").unwrap();
            let run_path = run.path().to_path_buf();
            let review_path = review.path().to_path_buf();
            signal_hook::low_level::raise(signal).unwrap();
            drop(run);
            drop(review);
            assert!(!run_path.exists() && !review_path.exists());
            assert!(local_head(&launch.completion(), BRANCH).unwrap().is_none());
            assert_eq!(
                registration(&launch.completion())
                    .matches("worktree ")
                    .count(),
                1
            );
            assert!(crate::interrupt::requested());
            assert_eq!(
                launch.run(&["rev-parse", "HEAD"]).unwrap_err().to_string(),
                "interrupted"
            );
        },
    );
}

#[test]
fn a_signal_arriving_during_cleanup_does_not_interrupt_completion() {
    if isolated(
        "a_signal_arriving_during_cleanup_does_not_interrupt_completion",
        r#"
if test "$1" = worktree && test "$2" = remove; then
  kill -"$THIRDSHIFT_TEST_SIGNAL" "$PPID"
fi
"#,
    ) {
        return;
    }
    crate::test_support::with_recorded_signal(
        "worktree::cleanup_tests::a_signal_arriving_during_cleanup_does_not_interrupt_completion",
        |_signal| {
            let (_temp, launch) = super::tests::launch_directory();
            let run = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
            let path = run.path().to_path_buf();
            drop(run);
            assert!(!path.exists());
            assert!(local_head(&launch.completion(), BRANCH).unwrap().is_none());
            assert!(crate::interrupt::requested());
            assert_eq!(
                launch.run(&["rev-parse", "HEAD"]).unwrap_err().to_string(),
                "interrupted"
            );
        },
    );
}

#[test]
fn interrupted_cleanup_still_retains_replacements_for_both_lifetimes() {
    crate::test_support::with_recorded_signal(
        "worktree::cleanup_tests::interrupted_cleanup_still_retains_replacements_for_both_lifetimes",
        |signal| {
            let (_temp, launch) = super::tests::launch_directory();
            let run = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
            let review = ReviewWorktree::create(&launch, "work", "main").unwrap();
            let run_path = run.path().to_path_buf();
            let review_path = review.path().to_path_buf();
            let head = run.head().unwrap();
            for (path, branch) in [(&run_path, BRANCH), (&review_path, head.as_str())] {
                launch
                    .run(&["worktree", "remove", "--force", path.to_str().unwrap()])
                    .unwrap();
                let mut args = vec!["worktree", "add"];
                if path == &review_path {
                    args.push("--detach");
                }
                args.extend([path.to_str().unwrap(), branch]);
                launch.run(&args).unwrap();
                fs::write(path.join("work.txt"), "replacement\n").unwrap();
            }
            let registered = registration(&launch);
            let refs = launch.run(&["show-ref"]).unwrap();
            signal_hook::low_level::raise(signal).unwrap();
            drop(run);
            drop(review);
            for path in [&run_path, &review_path] {
                assert_eq!(
                    fs::read_to_string(path.join("work.txt")).unwrap(),
                    "replacement\n"
                );
            }
            assert_eq!(registration(&launch.completion()), registered);
            assert_eq!(launch.completion().run(&["show-ref"]).unwrap(), refs);
            assert!(crate::interrupt::requested());
            assert_eq!(
                launch.run(&["rev-parse", "HEAD"]).unwrap_err().to_string(),
                "interrupted"
            );
        },
    );
}

#[test]
fn the_repository_lock_is_held_through_configuration_cleanup() {
    if isolated(
        "the_repository_lock_is_held_through_configuration_cleanup",
        r#"
if test "$1" = config && test "$4" = --remove-section; then
  touch "$THIRDSHIFT_FAULT_MARKER"
  tries=0
  while test ! -e "$THIRDSHIFT_FAULT_MARKER.release" && test "$tries" -lt 600; do
    sleep 0.05
    tries=$((tries + 1))
  done
  test -e "$THIRDSHIFT_FAULT_MARKER.release"
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    launch
        .run(&["config", "branch.issue-7.remote", "origin"])
        .unwrap();
    let lock_path = launch
        .common_dir()
        .unwrap()
        .join("thirdshift-worktrees.lock");
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    let checking = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            marker.exists(),
            "configuration cleanup never reached the barrier"
        );
        let held = fs::File::open(lock_path).unwrap();
        let result = held.try_lock();
        fs::write(marker.with_extension("release"), "released").unwrap();
        assert!(
            matches!(result, Err(fs::TryLockError::WouldBlock)),
            "cleanup released the worktree lock early"
        );
    });

    drop(owner);
    checking.join().unwrap();

    assert!(!path.exists());
    assert!(local_head(&launch, BRANCH).unwrap().is_none());
    assert!(
        launch
            .run_optional(&["config", "branch.issue-7.remote"])
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_failed_configuration_snapshot_retains_checkout_and_ref() {
    if isolated(
        "a_failed_configuration_snapshot_retains_checkout_and_ref",
        r#"
if test "$1" = config && test "$2" = --local && test "$3" = --no-includes && test "$4" = --null; then
  echo 'local configuration inspection refused' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    fs::write(path.join("work.txt"), "owned work\n").unwrap();
    launch
        .run(&["config", "branch.issue-7.remote", "origin"])
        .unwrap();
    expect_warning(
        &path,
        Some(BRANCH),
        &head,
        "local configuration inspection refused",
    );

    drop(owner);

    assert_retained(&launch, &path, &head);
    assert_eq!(
        launch.run(&["config", "branch.issue-7.remote"]).unwrap(),
        "origin"
    );
}

#[test]
fn cleanup_preserves_included_configuration_and_exact_dotted_subsections() {
    let (temp, launch) = super::tests::launch_directory();
    let branch = "issue-7.part";
    let owner = Worktree::create_fresh(&launch, "work", branch, "main").unwrap();
    let path = owner.path().to_path_buf();
    let included = temp.path().join("included.config");
    let content = "[branch \"issue-7.part\"]\n\tremote = included\n";
    fs::write(&included, content).unwrap();
    launch
        .run(&["config", "include.path", included.to_str().unwrap()])
        .unwrap();
    launch
        .run(&["config", "branch.issue-7.part.remote", "origin"])
        .unwrap();
    launch
        .run(&["config", "branch.issue-7.part.child.remote", "nested"])
        .unwrap();
    // Keep duplicate values, embedded newlines and both valueless/empty keys
    // in the initial snapshot: unchanged complex sections are disposable too.
    launch
        .run(&[
            "config",
            "--add",
            "branch.issue-7.part.description",
            "first\nline",
        ])
        .unwrap();
    launch
        .run(&[
            "config",
            "--add",
            "branch.issue-7.part.description",
            "second",
        ])
        .unwrap();
    use std::io::Write;
    let mut config = fs::OpenOptions::new()
        .append(true)
        .open(launch.common_dir().unwrap().join("config"))
        .unwrap();
    writeln!(config, "[branch \"issue-7.part\"]\n\tvalueless\n\tempty =").unwrap();
    drop(config);

    drop(owner);

    assert!(!path.exists());
    assert!(local_head(&launch, branch).unwrap().is_none());
    assert_eq!(fs::read_to_string(included).unwrap(), content);
    assert_eq!(
        launch
            .run(&["config", "branch.issue-7.part.remote"])
            .unwrap(),
        "included"
    );
    assert_eq!(
        launch
            .run(&["config", "branch.issue-7.part.child.remote"])
            .unwrap(),
        "nested"
    );
    assert!(
        !launch
            .run(&["config", "--local", "--no-includes", "--null", "--list"])
            .unwrap()
            .contains("branch.issue-7.part.remote\n")
    );
}

#[test]
fn a_replaced_common_repository_retains_the_checkout_and_refs() {
    if isolated(
        "a_replaced_common_repository_retains_the_checkout_and_refs",
        "",
    ) {
        return;
    }
    let (temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    fs::write(path.join("work.txt"), "owned work\n").unwrap();
    let common = launch.common_dir().unwrap();
    let saved = temp.path().join("old-common");
    fs::rename(&common, &saved).unwrap();
    copy_directory(&saved, &common);
    let refs = launch.run(&["show-ref"]).unwrap();
    let registered = registration(&launch);
    expect_warning(&path, Some(BRANCH), &head, "replaced");

    drop(owner);

    assert_retained(&launch, &path, &head);
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
    assert_eq!(registration(&launch), registered);
}

#[test]
fn retention_reports_available_checkout_head_when_local_ref_inspection_fails() {
    if isolated(
        "retention_reports_available_checkout_head_when_local_ref_inspection_fails",
        r#"
if test "$1" = for-each-ref && test -e "$THIRDSHIFT_FAULT_MARKER"; then
  echo 'local ref inspection refused' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = owner.head().unwrap();
    fs::write(path.join("work.txt"), "owned work\n").unwrap();
    fs::write(
        std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap(),
        "armed",
    )
    .unwrap();
    expect_warning(&path, Some(BRANCH), &head, "local ref inspection refused");

    drop(owner);

    fs::remove_file(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap()).unwrap();
    assert_retained(&launch, &path, &head);
}

#[test]
fn review_publication_failure_recovers_only_attempt_artifacts_and_preserves_work() {
    if isolated(
        "review_publication_failure_recovers_only_attempt_artifacts_and_preserves_work",
        r#"
if test "$1" = rev-parse && test "$2" = --symbolic-full-name && test -f .thirdshift-review-token && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  touch "$THIRDSHIFT_FAULT_MARKER"
  if test "$THIRDSHIFT_PUBLICATION_WORK" = yes; then printf 'publication work\n' > work.txt; fi
  echo 'final review inspection refused' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    for work in [false, true] {
        unsafe {
            std::env::set_var(
                "THIRDSHIFT_PUBLICATION_WORK",
                if work { "yes" } else { "no" },
            );
        }
        let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
        let _ = fs::remove_file(marker);
        let (temp, launch) = super::tests::launch_directory();
        let path = temp.path().join("work-architect");
        let error = ReviewWorktree::create(&launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("final review inspection refused"), "{error}");
        if work {
            assert_eq!(
                fs::read_to_string(path.join("work.txt")).unwrap(),
                "publication work\n"
            );
            let admin = PathBuf::from(
                Git::new(&path)
                    .run(&["rev-parse", "--absolute-git-dir"])
                    .unwrap(),
            );
            assert!(!admin.join("thirdshift-review.json").exists());
            assert!(!path.join(".thirdshift-review-token").exists());
            let retry = ReviewWorktree::create(&launch, "work", "main")
                .err()
                .unwrap()
                .to_string();
            assert!(retry.contains("retaining checkout"), "{retry}");
            assert_eq!(
                fs::read_to_string(path.join("work.txt")).unwrap(),
                "publication work\n"
            );
        } else {
            assert!(!path.exists(), "{error}");
            let next = ReviewWorktree::create(&launch, "work", "main").unwrap();
            drop(next);
            assert!(!path.exists());
        }
    }
}

#[test]
fn review_record_publication_never_replaces_existing_files_or_links() {
    if isolated(
        "review_record_publication_never_replaces_existing_files_or_links",
        r#"
if test "$1" = rev-parse && test "$2" = --symbolic-full-name && test -f .thirdshift-review-token && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  touch "$THIRDSHIFT_FAULT_MARKER"
  admin=$("$THIRDSHIFT_REAL_GIT" rev-parse --absolute-git-dir)
  printf 'record collision work\n' > work.txt
  if test "$THIRDSHIFT_RECORD_LINK" = yes; then
    printf 'link target\n' > ../record-target
    ln -s "$PWD/../record-target" "$admin/thirdshift-review.json"
  else
    printf 'existing record\n' > "$admin/thirdshift-review.json"
  fi
fi
"#,
    ) {
        return;
    }
    for link in [false, true] {
        unsafe {
            std::env::set_var("THIRDSHIFT_RECORD_LINK", if link { "yes" } else { "no" });
        }
        let _ = fs::remove_file(PathBuf::from(
            std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap(),
        ));
        let (temp, launch) = super::tests::launch_directory();
        let path = temp.path().join("work-architect");
        let error = ReviewWorktree::create(&launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("cannot publish successful review acquisition"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(path.join("work.txt")).unwrap(),
            "record collision work\n"
        );
        let admin = PathBuf::from(
            Git::new(&path)
                .run(&["rev-parse", "--absolute-git-dir"])
                .unwrap(),
        );
        let record = admin.join("thirdshift-review.json");
        assert_eq!(
            fs::read_to_string(&record).unwrap(),
            if link {
                "link target\n"
            } else {
                "existing record\n"
            }
        );
        assert_eq!(
            fs::symlink_metadata(&record)
                .unwrap()
                .file_type()
                .is_symlink(),
            link
        );
        assert!(!path.join(".thirdshift-review-token").exists());
        let retry = ReviewWorktree::create(&launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(retry.contains("retaining checkout"), "{retry}");
        assert_eq!(
            fs::read_to_string(&record).unwrap(),
            if link {
                "link target\n"
            } else {
                "existing record\n"
            }
        );
    }
}

#[test]
fn uncertain_publication_artifacts_are_retained_instead_of_unlinked() {
    if isolated(
        "uncertain_publication_artifacts_are_retained_instead_of_unlinked",
        r#"
if test "$1" = rev-parse && test "$2" = --symbolic-full-name && test -f .thirdshift-review-token && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  touch "$THIRDSHIFT_FAULT_MARKER"
  if test "$THIRDSHIFT_REPLACE_ARTIFACT" = token; then
    artifact="$PWD/.thirdshift-review-token"
  else
    admin=$("$THIRDSHIFT_REAL_GIT" rev-parse --absolute-git-dir)
    for candidate in "$admin"/.thirdshift-review-*; do artifact="$candidate"; done
  fi
  mv "$artifact" ../original-artifact
  printf 'replacement artifact\n' > "$artifact"
  printf '%s\n' "$artifact" > ../replacement-path
fi
"#,
    ) {
        return;
    }
    for kind in ["token", "record"] {
        unsafe {
            std::env::set_var("THIRDSHIFT_REPLACE_ARTIFACT", kind);
        }
        let _ = fs::remove_file(PathBuf::from(
            std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap(),
        ));
        let (temp, launch) = super::tests::launch_directory();
        let path = temp.path().join("work-architect");
        let error = ReviewWorktree::create(&launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("retaining checkout") && error.contains("uncertain ownership"),
            "{kind}: {error}"
        );
        let replacement = fs::read_to_string(temp.path().join("replacement-path")).unwrap();
        assert_eq!(
            fs::read_to_string(replacement.trim()).unwrap(),
            "replacement artifact\n"
        );
        let head = Git::new(&path).run(&["rev-parse", "HEAD"]).unwrap();
        assert!(error.contains(&head), "{error}");
        let registrations = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
        let retry = ReviewWorktree::create(&launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(retry.contains("retaining checkout"), "{retry}");
        assert_eq!(
            fs::read_to_string(replacement.trim()).unwrap(),
            "replacement artifact\n"
        );
        assert_eq!(
            launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
            registrations
        );
    }
}

#[test]
fn stale_review_removal_failure_retains_resources_and_reports_the_available_head() {
    if isolated(
        "stale_review_removal_failure_retains_resources_and_reports_the_available_head",
        r#"
if test "$1" = worktree && test "$2" = remove; then
  echo 'stale review removal refused' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    let (temp, launch) = super::tests::launch_directory();
    let owner = ReviewWorktree::create(&launch, "work", "main").unwrap();
    let path = owner.path().to_path_buf();
    let head = Git::new(&path).run(&["rev-parse", "HEAD"]).unwrap();
    fs::write(path.join("scratch.txt"), "retained scratch\n").unwrap();
    let registrations = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();
    std::mem::forget(owner);
    let error = ReviewWorktree::create(&launch, "work", "main")
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("retaining checkout")
            && error.contains(&head)
            && error.contains("stale review removal refused"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(path.join("scratch.txt")).unwrap(),
        "retained scratch\n"
    );
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        registrations
    );
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
    assert!(temp.path().join("work-architect").exists());
}

#[test]
fn exclusion_errors_never_publish_review_disposal_authority() {
    if isolated(
        "exclusion_errors_never_publish_review_disposal_authority",
        r#"
if test "$1" = ls-files && test "$2" = --error-unmatch; then
  common=$("$THIRDSHIFT_REAL_GIT" rev-parse --git-common-dir)
  rm -f "$common/info/exclude"
  mkdir -p "$common/info/exclude"
  printf 'exclusion failure work\n' > work.txt
fi
"#,
    ) {
        return;
    }
    let (temp, launch) = super::tests::launch_directory();
    let path = temp.path().join("work-architect");
    let error = ReviewWorktree::create(&launch, "work", "main")
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("exclude") && error.contains("retaining checkout"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(path.join("work.txt")).unwrap(),
        "exclusion failure work\n"
    );
    let admin = PathBuf::from(
        Git::new(&path)
            .run(&["rev-parse", "--absolute-git-dir"])
            .unwrap(),
    );
    assert!(!admin.join("thirdshift-review.json").exists());
    fs::remove_dir(launch.dir().join(".git/info/exclude")).unwrap();
    let retry = ReviewWorktree::create(&launch, "work", "main")
        .err()
        .unwrap()
        .to_string();
    assert!(retry.contains("retaining checkout"), "{retry}");
    assert_eq!(
        fs::read_to_string(path.join("work.txt")).unwrap(),
        "exclusion failure work\n"
    );
}

#[test]
fn interrupted_review_publication_finishes_guarded_acquisition_recovery() {
    const NAME: &str = "worktree::cleanup_tests::interrupted_review_publication_finishes_guarded_acquisition_recovery";
    if isolated(
        "interrupted_review_publication_finishes_guarded_acquisition_recovery",
        r#"
if test "$1" = rev-parse && test "$2" = --symbolic-full-name && test -f .thirdshift-review-token && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  touch "$THIRDSHIFT_FAULT_MARKER"
  if test "$THIRDSHIFT_PUBLICATION_WORK" = yes; then printf 'interrupted publication work\n' > work.txt; fi
  kill -"$THIRDSHIFT_TEST_SIGNAL" "$THIRDSHIFT_PUBLICATION_PID"
fi
"#,
    ) {
        return;
    }
    crate::test_support::with_recorded_signal(NAME, |_signal| {
        let _ = fs::remove_file(PathBuf::from(
            std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap(),
        ));
        unsafe {
            std::env::set_var("THIRDSHIFT_PUBLICATION_WORK", "yes");
            std::env::set_var("THIRDSHIFT_PUBLICATION_PID", std::process::id().to_string());
        }
        let (temp, launch) = super::tests::launch_directory();
        let path = temp.path().join("work-architect");
        let error = ReviewWorktree::create(&launch, "work", "main")
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.starts_with("interrupted") && error.contains("retaining checkout"),
            "{error}"
        );
        let admin = PathBuf::from(
            Git::new(&path)
                .completion()
                .run(&["rev-parse", "--absolute-git-dir"])
                .unwrap(),
        );
        assert!(!admin.join("thirdshift-review.json").exists());
        assert!(!path.join(".thirdshift-review-token").exists());
        assert_eq!(
            fs::read_to_string(path.join("work.txt")).unwrap(),
            "interrupted publication work\n"
        );
        assert!(crate::interrupt::requested());
        assert_eq!(
            launch.run(&["rev-parse", "HEAD"]).unwrap_err().to_string(),
            "interrupted"
        );
    });
}

#[test]
fn damaged_review_registration_is_retained_before_a_fetch_can_prune_it() {
    if isolated(
        "damaged_review_registration_is_retained_before_a_fetch_can_prune_it",
        r#"
if test "$1" = fetch; then
  common=$("$THIRDSHIFT_REAL_GIT" rev-parse --git-common-dir)
  if test -d "$common/worktrees/work-architect" && test ! -e "$common/worktrees/work-architect/gitdir"; then
    "$THIRDSHIFT_REAL_GIT" worktree prune
    touch "$THIRDSHIFT_FAULT_MARKER"
  fi
fi
"#,
    ) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let owner = ReviewWorktree::create(&launch, "work", "main").unwrap();
    let path = owner.path().to_path_buf();
    let admin = PathBuf::from(
        Git::new(&path)
            .run(&["rev-parse", "--absolute-git-dir"])
            .unwrap(),
    );
    let record = fs::read(admin.join("thirdshift-review.json")).unwrap();
    fs::write(path.join("retained.txt"), "uncertain review work\n").unwrap();
    std::mem::forget(owner);
    fs::remove_file(admin.join("gitdir")).unwrap();

    let error = ReviewWorktree::create(&launch, "work", "main")
        .err()
        .unwrap()
        .to_string();

    assert!(error.contains("retaining checkout"), "{error}");
    assert_eq!(
        fs::read_to_string(path.join("retained.txt")).unwrap(),
        "uncertain review work\n"
    );
    assert!(
        admin.exists(),
        "fetch pruned the retained registration: {error}"
    );
    assert_eq!(
        fs::read(admin.join("thirdshift-review.json")).unwrap(),
        record
    );
    assert!(!PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap()).exists());
}
