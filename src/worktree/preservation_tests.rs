//! Failed run preservation through Worktree, with real local Git and origin.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;
use crate::interrupt;
use crate::test_support::{git_fault, with_recorded_signal};

const BRANCH: &str = "issue-7";

fn fixture() -> (tempfile::TempDir, Git, Worktree) {
    let (temp, launch) = super::tests::launch_directory();
    let worktree = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
    (temp, launch, worktree)
}

fn rejecting_hook(path: &Path) {
    fs::write(path, "#!/bin/sh\necho 'hook says no' >&2\nexit 1\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn unfinished_merge(git: &Git) {
    fs::write(git.dir().join("conflict.txt"), "base\n").unwrap();
    fs::write(git.dir().join("merged.txt"), "base\n").unwrap();
    git.run(&["add", "-A"]).unwrap();
    git.run(&["commit", "-q", "-m", "Shared files"]).unwrap();
    git.run(&["checkout", "-q", "-b", "other"]).unwrap();
    fs::write(git.dir().join("conflict.txt"), "theirs\n").unwrap();
    fs::write(git.dir().join("merged.txt"), "theirs\n").unwrap();
    git.run(&["commit", "-q", "-am", "Theirs"]).unwrap();
    git.run(&["checkout", "-q", BRANCH]).unwrap();
    fs::write(git.dir().join("conflict.txt"), "ours\n").unwrap();
    git.run(&["commit", "-q", "-am", "Ours"]).unwrap();
    assert!(git.run(&["merge", "--no-edit", "other"]).is_err());
    assert!(git.merge_in_progress().unwrap());
    assert_eq!(
        fs::read_to_string(git.dir().join("merged.txt")).unwrap(),
        "theirs\n"
    );
}

fn assert_cleaned_up(launch: &Git, path: &Path) {
    assert!(!path.exists(), "worktree was retained: {}", path.display());
    assert!(
        !launch
            .succeeds(&["show-ref", "--verify", "--quiet", "refs/heads/issue-7"])
            .unwrap(),
        "local Issue branch was retained"
    );
}

fn assert_retained(launch: &Git, path: &Path) {
    assert!(path.exists(), "worktree was removed: {}", path.display());
    assert!(
        launch
            .succeeds(&["show-ref", "--verify", "--quiet", "refs/heads/issue-7"])
            .unwrap(),
        "local Issue branch was removed"
    );
}

fn assert_ownership_error(error: &anyhow::Error, path: &Path, cause: &str) {
    let error = format!("{error:#}");
    assert!(error.contains(path.to_str().unwrap()), "{error}");
    assert!(
        error.contains(&format!("expected Issue branch {BRANCH}")),
        "{error}"
    );
    assert!(error.contains(cause), "{error}");
}

#[test]
fn switched_and_detached_checkouts_refuse_preservation_without_touching_unrelated_work() {
    for detached in [false, true] {
        let (_temp, launch, mut worktree) = fixture();
        let path = worktree.path().to_path_buf();
        let git = Git::new(&path);
        if detached {
            git.run(&["checkout", "-q", "--detach"]).unwrap();
        } else {
            git.run(&["checkout", "-q", "-b", "manual"]).unwrap();
        }
        fs::write(path.join("unrelated.txt"), "manual work\n").unwrap();
        let refs = launch.run(&["show-ref"]).unwrap();
        let status = git.run(&["status", "--porcelain"]).unwrap();

        let error = worktree
            .preserve_failed_run("main", "session failed")
            .unwrap_err();
        assert_ownership_error(&error, &path, "identity changed");
        drop(worktree);

        assert_retained(&launch, &path);
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), status);
        assert_eq!(
            fs::read_to_string(path.join("unrelated.txt")).unwrap(),
            "manual work\n"
        );
        assert!(!launch.on_origin(BRANCH).unwrap());
        assert!(!launch.on_origin("manual").unwrap());
    }
}

#[test]
fn changed_ownership_refuses_preservation_even_with_unchanged_work() {
    for detached in [false, true] {
        let (_temp, launch, mut worktree) = fixture();
        let path = worktree.path().to_path_buf();
        let git = Git::new(&path);
        if detached {
            git.run(&["checkout", "-q", "--detach"]).unwrap();
        } else {
            git.run(&["checkout", "-q", "-b", "manual"]).unwrap();
        }
        let refs = launch.run(&["show-ref"]).unwrap();

        let error = worktree
            .preserve_failed_run("main", "session failed")
            .unwrap_err();
        assert_ownership_error(&error, &path, "identity changed");
        // Restore readable ownership before Drop: retention must come from
        // the failed preservation, rather than cleanup refusing the mismatch.
        git.run(&["checkout", "-q", BRANCH]).unwrap();
        drop(worktree);

        assert_retained(&launch, &path);
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), "");
        assert!(!launch.on_origin(BRANCH).unwrap());
    }
}

#[test]
fn recreated_checkout_on_the_same_branch_and_head_refuses_preservation() {
    let (_temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    let head = git.run(&["rev-parse", "HEAD"]).unwrap();
    let admin = git.run(&["rev-parse", "--absolute-git-dir"]).unwrap();
    launch
        .run(&["worktree", "remove", "--force", path.to_str().unwrap()])
        .unwrap();
    launch
        .run(&["worktree", "add", path.to_str().unwrap(), BRANCH])
        .unwrap();
    assert_eq!(git.run(&["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(
        git.run(&["rev-parse", "--absolute-git-dir"]).unwrap(),
        admin
    );
    fs::write(path.join("replacement.txt"), "replacement work\n").unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();
    let registered = launch.run(&["worktree", "list", "--porcelain"]).unwrap();
    let status = git.run(&["status", "--porcelain"]).unwrap();

    let error = worktree
        .preserve_failed_run("main", "session failed")
        .unwrap_err();
    assert_ownership_error(&error, &path, "replaced");
    drop(worktree);

    assert_retained(&launch, &path);
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
    assert_eq!(
        launch.run(&["worktree", "list", "--porcelain"]).unwrap(),
        registered
    );
    assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), status);
    assert_eq!(
        fs::read_to_string(path.join("replacement.txt")).unwrap(),
        "replacement work\n"
    );
    assert!(!launch.on_origin(BRANCH).unwrap());
}

#[test]
fn locked_or_damaged_ownership_refuses_preservation_and_retains_work_after_evidence_is_repaired() {
    for damage in [
        "locked",
        "missing-backlink",
        "wrong-backlink",
        "unreadable-backlink",
        "duplicate-registration",
    ] {
        let (_temp, launch, mut worktree) = fixture();
        let path = worktree.path().to_path_buf();
        let git = Git::new(&path);
        let admin = PathBuf::from(git.run(&["rev-parse", "--absolute-git-dir"]).unwrap());
        let backlink = admin.join("gitdir");
        let original = fs::read(&backlink).unwrap();
        fs::write(path.join("wip.txt"), "half done\n").unwrap();
        let refs = launch.run(&["show-ref"]).unwrap();
        let status = git.run(&["status", "--porcelain"]).unwrap();
        match damage {
            "locked" => {
                launch
                    .run(&["worktree", "lock", path.to_str().unwrap()])
                    .unwrap();
            }
            "missing-backlink" => fs::remove_file(&backlink).unwrap(),
            "wrong-backlink" => {
                fs::write(&backlink, launch.dir().join(".git").to_str().unwrap()).unwrap()
            }
            "unreadable-backlink" => {
                fs::remove_file(&backlink).unwrap();
                fs::create_dir(&backlink).unwrap();
            }
            _ => {
                let duplicate = admin.with_file_name("duplicate");
                fs::create_dir(&duplicate).unwrap();
                for file in ["HEAD", "gitdir", "commondir"] {
                    fs::copy(admin.join(file), duplicate.join(file)).unwrap();
                }
            }
        }

        let error = worktree
            .preserve_failed_run("main", "session failed")
            .unwrap_err();
        assert_ownership_error(&error, &path, "cannot preserve acquired checkout");
        match damage {
            "locked" => {
                launch
                    .run(&["worktree", "unlock", path.to_str().unwrap()])
                    .unwrap();
            }
            "unreadable-backlink" => fs::remove_dir(&backlink).unwrap(),
            "duplicate-registration" => {
                fs::remove_dir_all(admin.with_file_name("duplicate")).unwrap()
            }
            _ => {}
        }
        fs::write(&backlink, original).unwrap();
        drop(worktree);

        assert_retained(&launch, &path);
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), status);
        assert_eq!(
            fs::read_to_string(path.join("wip.txt")).unwrap(),
            "half done\n"
        );
        assert!(!launch.on_origin(BRANCH).unwrap());
    }
}

#[test]
fn replaced_pinned_directories_refuse_preservation_and_do_not_lock_a_replacement_repository() {
    for directory in ["checkout", "administration", "common"] {
        let (_temp, launch, mut worktree) = fixture();
        let path = worktree.path().to_path_buf();
        let git = Git::new(&path);
        fs::write(path.join("wip.txt"), "half done\n").unwrap();
        let refs = launch.run(&["show-ref"]).unwrap();
        let status = git.run(&["status", "--porcelain"]).unwrap();
        let pinned = match directory {
            "checkout" => path.clone(),
            "administration" => {
                PathBuf::from(git.run(&["rev-parse", "--absolute-git-dir"]).unwrap())
            }
            _ => launch.common_dir().unwrap(),
        };
        let moved = pinned.with_extension("original");
        fs::rename(&pinned, &moved).unwrap();
        fs::create_dir(&pinned).unwrap();

        let error = worktree
            .preserve_failed_run("main", "session failed")
            .unwrap_err();
        assert_ownership_error(&error, &path, "replaced");
        assert_eq!(
            fs::read_dir(&pinned).unwrap().count(),
            0,
            "replacement was mutated"
        );
        fs::remove_dir(&pinned).unwrap();
        fs::rename(&moved, &pinned).unwrap();
        drop(worktree);

        assert_retained(&launch, &path);
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), status);
        assert_eq!(
            fs::read_to_string(path.join("wip.txt")).unwrap(),
            "half done\n"
        );
        assert!(!launch.on_origin(BRANCH).unwrap());
    }
}

#[test]
fn a_successful_retry_on_the_original_checkout_restores_cleanup_eligibility() {
    let (_temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    git.run(&["checkout", "-q", "--detach"]).unwrap();
    assert!(
        worktree
            .preserve_failed_run("main", "session failed")
            .is_err()
    );
    git.run(&["checkout", "-q", BRANCH]).unwrap();

    worktree
        .preserve_failed_run("main", "session failed")
        .unwrap();
    drop(worktree);

    assert_cleaned_up(&launch, &path);
    assert!(!launch.on_origin(BRANCH).unwrap());
}

#[test]
fn preservation_lock_errors_retain_work_even_after_the_locks_are_repaired() {
    for name in ["thirdshift-worktrees.lock", "thirdshift-worktree-refs.lock"] {
        let (_temp, launch, mut worktree) = fixture();
        let path = worktree.path().to_path_buf();
        let git = Git::new(&path);
        fs::write(path.join("wip.txt"), "half done\n").unwrap();
        let refs = launch.run(&["show-ref"]).unwrap();
        let status = git.run(&["status", "--porcelain"]).unwrap();
        let lock = launch.common_dir().unwrap().join(name);
        fs::remove_file(&lock).unwrap();
        fs::create_dir(&lock).unwrap();

        let error = worktree
            .preserve_failed_run("main", "session failed")
            .unwrap_err();
        assert_ownership_error(&error, &path, "can't open");
        fs::remove_dir(&lock).unwrap();
        drop(worktree);

        assert_retained(&launch, &path);
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        assert_eq!(git.run(&["status", "--porcelain"]).unwrap(), status);
        assert_eq!(
            fs::read_to_string(path.join("wip.txt")).unwrap(),
            "half done\n"
        );
        assert!(!launch.on_origin(BRANCH).unwrap());
    }
}

#[test]
fn preservation_waits_for_both_repository_locks_before_changing_work() {
    for name in ["thirdshift-worktrees.lock", "thirdshift-worktree-refs.lock"] {
        let (_temp, launch, mut worktree) = fixture();
        let path = worktree.path().to_path_buf();
        let git = Git::new(&path);
        fs::write(path.join("wip.txt"), "half done\n").unwrap();
        let original_status = git.run(&["status", "--porcelain"]).unwrap();
        let original_refs = launch.run(&["show-ref"]).unwrap();
        let held = launch.lock(name).unwrap();
        let (finished, received) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = worktree.preserve_failed_run("main", "session failed");
            let _ = finished.send(());
            (result, worktree)
        });
        let finished_while_held = received.recv_timeout(Duration::from_millis(500)).is_ok();
        let status = git.run(&["status", "--porcelain"]);
        let refs = launch.run(&["show-ref"]);
        let pushed = launch.on_origin(BRANCH);
        drop(held);
        let (result, worktree) = worker.join().unwrap();

        assert!(!finished_while_held, "preservation bypassed {name}");
        assert_eq!(status.unwrap(), original_status);
        assert_eq!(refs.unwrap(), original_refs);
        assert!(!pushed.unwrap());
        result.unwrap();
        drop(worktree);
        assert_cleaned_up(&launch, &path);
    }
}

#[test]
fn ownership_changes_during_preservation_refuse_the_next_mutation_or_success() {
    if git_fault(
        "worktree::preservation_tests::ownership_changes_during_preservation_refuse_the_next_mutation_or_success",
        r#"
if test -f "$THIRDSHIFT_FAULT_MARKER.gate" && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  gate=$(cat "$THIRDSHIFT_FAULT_MARKER.gate")
  case "$gate:$1:$2" in
    abort:rev-parse:-q|stage:rev-parse:-q|commit:diff:--cached|push:commit:-q|unchanged:diff:--cached)
      status=0
      "$THIRDSHIFT_REAL_GIT" "$@" || status=$?
      "$THIRDSHIFT_REAL_GIT" symbolic-ref HEAD refs/heads/manual
      printf 'unrelated work\n' > unrelated.txt
      touch "$THIRDSHIFT_FAULT_MARKER"
      exit "$status"
      ;;
  esac
fi
"#,
    ).is_some() {
        return;
    }
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    for gate in ["abort", "stage", "commit", "push", "unchanged"] {
        let _ = fs::remove_file(&marker);
        let _ = fs::remove_file(marker.with_extension("gate"));
        let (_temp, launch, mut worktree) = fixture();
        let path = worktree.path().to_path_buf();
        let git = Git::new(&path);
        if gate == "abort" {
            unfinished_merge(&git);
        }
        git.run(&["branch", "manual"]).unwrap();
        let head = git.run(&["rev-parse", "HEAD"]).unwrap();
        let refs = launch.run(&["show-ref"]).unwrap();
        let index = git.run(&["diff", "--cached", "--name-status"]).unwrap();
        if gate != "unchanged" {
            fs::write(path.join("wip.txt"), "half done\n").unwrap();
        }
        fs::write(marker.with_extension("gate"), gate).unwrap();

        let error = worktree
            .preserve_failed_run("main", "session failed")
            .unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains(path.to_str().unwrap()), "{gate}: {error}");
        assert!(error.contains(BRANCH), "{gate}: {error}");
        assert!(error.contains("identity changed"), "{gate}: {error}");
        assert!(
            marker.exists(),
            "ownership change was not exercised: {gate}"
        );
        drop(worktree);

        assert_retained(&launch, &path);
        assert_eq!(
            fs::read_to_string(path.join("unrelated.txt")).unwrap(),
            "unrelated work\n"
        );
        assert_eq!(git.run(&["rev-parse", "manual"]).unwrap(), head);
        assert!(!launch.on_origin(BRANCH).unwrap());
        assert!(!launch.on_origin("manual").unwrap());
        if gate == "push" {
            assert_eq!(
                git.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
                "thirdshift: failed run (session failed)"
            );
            assert_eq!(git.run(&["show", "issue-7:wip.txt"]).unwrap(), "half done");
        } else {
            assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        }
        if matches!(gate, "abort" | "stage" | "unchanged") {
            assert_eq!(
                git.run(&["diff", "--cached", "--name-status"]).unwrap(),
                index
            );
        }
        if gate == "abort" {
            assert!(git.merge_in_progress().unwrap());
            assert_eq!(
                fs::read_to_string(path.join("merged.txt")).unwrap(),
                "theirs\n"
            );
        }
    }
}

#[test]
fn ownership_changing_during_a_successful_push_still_fails_and_holds_both_locks_through_final_inspection()
 {
    if git_fault(
        "worktree::preservation_tests::ownership_changing_during_a_successful_push_still_fails_and_holds_both_locks_through_final_inspection",
        r#"
if test -e "$THIRDSHIFT_FAULT_MARKER.armed"; then
  if test "$1" = push && test "$2" = --no-verify; then
    "$THIRDSHIFT_REAL_GIT" "$@"
    "$THIRDSHIFT_REAL_GIT" symbolic-ref HEAD refs/heads/manual
    printf 'unrelated work\n' > unrelated.txt
    touch "$THIRDSHIFT_FAULT_MARKER.pushed"
    exit 0
  fi
  if test "$1" = worktree && test "$2" = list && test -e "$THIRDSHIFT_FAULT_MARKER.pushed"; then
    touch "$THIRDSHIFT_FAULT_MARKER"
    for _ in $(seq 500); do
      test -e "$THIRDSHIFT_FAULT_MARKER.release" && break
      sleep 0.01
    done
    test -e "$THIRDSHIFT_FAULT_MARKER.release"
  fi
fi
"#,
    ).is_some() {
        return;
    }
    let (temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    git.run(&["branch", "manual"]).unwrap();
    let manual = git.run(&["rev-parse", "manual"]).unwrap();
    fs::write(path.join("wip.txt"), "half done\n").unwrap();
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    let common = launch.common_dir().unwrap();
    fs::write(marker.with_extension("armed"), "armed").unwrap();
    let (finished, received) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = worktree.preserve_failed_run("main", "session failed");
        let _ = finished.send(());
        (result, worktree)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() && received.try_recv().is_err() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let held: Vec<_> = ["thirdshift-worktrees.lock", "thirdshift-worktree-refs.lock"]
        .into_iter()
        .map(|name| {
            let held = File::open(common.join(name))
                .map(|file| matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
            (name, held)
        })
        .collect();
    let released = fs::write(marker.with_extension("release"), "released");
    let (result, worktree) = worker.join().unwrap();
    released.unwrap();

    assert_ownership_error(&result.unwrap_err(), &path, "identity changed");
    assert!(marker.exists(), "final inspection was not exercised");
    for (name, held) in held {
        assert!(held.unwrap(), "{name} was released before final inspection");
    }
    drop(worktree);

    assert_retained(&launch, &path);
    assert_eq!(git.run(&["rev-parse", "manual"]).unwrap(), manual);
    assert_eq!(
        fs::read_to_string(path.join("unrelated.txt")).unwrap(),
        "unrelated work\n"
    );
    assert_eq!(git.run(&["show", "issue-7:wip.txt"]).unwrap(), "half done");
    let origin = Git::new(temp.path().join("origin.git"));
    assert_eq!(
        origin.run(&["show", "issue-7:wip.txt"]).unwrap(),
        "half done"
    );
    assert_eq!(
        origin.run(&["rev-parse", BRANCH]).unwrap(),
        launch.run(&["rev-parse", BRANCH]).unwrap()
    );
    assert!(!launch.on_origin("manual").unwrap());
}

#[test]
fn preservation_pushes_committed_and_uncommitted_work_with_failure_metadata_then_cleans_up() {
    let (temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    fs::write(path.join("feature.txt"), "feature\n").unwrap();
    git.run(&["add", "feature.txt"]).unwrap();
    git.run(&["commit", "-q", "-m", "Add feature"]).unwrap();
    fs::write(path.join("wip.txt"), "half done\n").unwrap();

    worktree
        .preserve_failed_run("main", "session failed")
        .unwrap();
    drop(worktree);

    let origin = Git::new(temp.path().join("origin.git"));
    assert_eq!(
        origin.run(&["show", "issue-7:feature.txt"]).unwrap(),
        "feature"
    );
    assert_eq!(
        origin.run(&["show", "issue-7:wip.txt"]).unwrap(),
        "half done"
    );
    assert_eq!(
        origin.run(&["log", "-2", "--format=%s", BRANCH]).unwrap(),
        "thirdshift: failed run (session failed)\nAdd feature"
    );
    let body = origin.run(&["log", "-1", "--format=%b", BRANCH]).unwrap();
    let (timestamp, rest) = body.split_once(", host ").expect(&body);
    chrono::DateTime::parse_from_rfc3339(timestamp).unwrap();
    assert!(timestamp.ends_with('Z'), "{body}");
    let host = rest
        .strip_suffix(". Uncommitted work at the time of failure is included in this commit.")
        .expect(&body);
    assert!(!host.is_empty(), "{body}");
    assert_cleaned_up(&launch, &path);
}

#[test]
fn a_staging_error_retains_uncommitted_work_and_the_local_branch_after_drop() {
    let (_temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    let index = path.join(git.run(&["rev-parse", "--git-path", "index"]).unwrap());
    fs::remove_file(&index).unwrap();
    fs::create_dir(&index).unwrap();
    fs::write(path.join("wip.txt"), "unstaged work\n").unwrap();

    let error = worktree
        .preserve_failed_run("main", "session failed")
        .unwrap_err();
    assert!(
        error.to_string().starts_with("git add -A failed"),
        "{error:#}"
    );
    drop(worktree);

    assert_retained(&launch, &path);
    assert_eq!(
        fs::read_to_string(path.join("wip.txt")).unwrap(),
        "unstaged work\n"
    );
    assert_eq!(
        launch.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
        "Initial"
    );
}

#[test]
fn unchanged_work_makes_no_failure_commit_or_remote_branch_and_allows_cleanup() {
    let (_temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let head = worktree.head().unwrap();

    worktree
        .preserve_failed_run("main", "session failed")
        .unwrap();

    assert_eq!(worktree.head().unwrap(), head);
    assert!(!launch.on_origin(BRANCH).unwrap());
    drop(worktree);
    assert_cleaned_up(&launch, &path);
}

#[test]
fn committed_work_gets_an_empty_failure_marker_despite_rejecting_local_hooks() {
    let (temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    fs::write(path.join("feature.txt"), "feature\n").unwrap();
    git.run(&["add", "feature.txt"]).unwrap();
    git.run(&["commit", "-q", "-m", "Add feature"]).unwrap();
    let feature = worktree.head().unwrap();
    for hook in ["pre-commit", "commit-msg", "pre-push"] {
        rejecting_hook(&temp.path().join("hooks").join(hook));
    }

    worktree
        .preserve_failed_run("main", "session failed")
        .unwrap();
    drop(worktree);

    let origin = Git::new(temp.path().join("origin.git"));
    assert_eq!(
        origin.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
        "thirdshift: failed run (session failed)"
    );
    assert_eq!(origin.run(&["rev-parse", "issue-7^"]).unwrap(), feature);
    assert!(
        origin
            .succeeds(&["diff", "--quiet", "issue-7^", BRANCH])
            .unwrap()
    );
    assert_eq!(
        origin.run(&["show", "issue-7:feature.txt"]).unwrap(),
        "feature"
    );
    assert_cleaned_up(&launch, &path);
}

#[test]
fn an_unfinished_merge_is_aborted_before_preserving_the_issue_branch() {
    let (temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    unfinished_merge(&git);
    fs::write(path.join("wip.txt"), "half done\n").unwrap();

    worktree
        .preserve_failed_run("main", "session failed")
        .unwrap();

    assert!(!git.merge_in_progress().unwrap());
    drop(worktree);
    let origin = Git::new(temp.path().join("origin.git"));
    assert_eq!(
        origin.run(&["show", "issue-7:conflict.txt"]).unwrap(),
        "ours"
    );
    assert_eq!(origin.run(&["show", "issue-7:merged.txt"]).unwrap(), "base");
    assert_eq!(
        origin.run(&["show", "issue-7:wip.txt"]).unwrap(),
        "half done"
    );
    assert_eq!(
        origin.run(&["log", "-3", "--format=%s", BRANCH]).unwrap(),
        "thirdshift: failed run (session failed)\nOurs\nShared files"
    );
    assert_cleaned_up(&launch, &path);
}

#[test]
fn a_merge_abort_error_retains_the_conflict_and_edits_after_drop() {
    let (_temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    unfinished_merge(&git);
    let conflict = fs::read_to_string(path.join("conflict.txt")).unwrap();
    fs::write(path.join("merged.txt"), "edited after conflict\n").unwrap();

    let error = worktree
        .preserve_failed_run("main", "session failed")
        .unwrap_err();
    assert!(
        error.to_string().starts_with("git merge --abort failed"),
        "{error:#}"
    );
    drop(worktree);

    assert_retained(&launch, &path);
    assert!(git.merge_in_progress().unwrap());
    assert_eq!(
        fs::read_to_string(path.join("conflict.txt")).unwrap(),
        conflict
    );
    assert_eq!(
        fs::read_to_string(path.join("merged.txt")).unwrap(),
        "edited after conflict\n"
    );
    assert_eq!(
        launch.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
        "Ours"
    );
}

#[test]
fn a_commit_error_retains_staged_work_and_the_local_branch_after_drop() {
    let (_temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    git.run(&["config", "user.name", ""]).unwrap();
    fs::write(path.join("wip.txt"), "half done\n").unwrap();

    let error = worktree
        .preserve_failed_run("main", "session failed")
        .unwrap_err();
    assert!(error.to_string().starts_with("git commit"), "{error:#}");
    drop(worktree);

    assert_retained(&launch, &path);
    assert_eq!(git.run(&["show", ":wip.txt"]).unwrap(), "half done");
    assert_eq!(
        fs::read_to_string(path.join("wip.txt")).unwrap(),
        "half done\n"
    );
    assert_eq!(
        launch.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
        "Initial"
    );
}

#[test]
fn rejected_or_unreachable_origin_retains_the_failure_commit_and_work_after_drop() {
    for rejecting_origin in [true, false] {
        let (temp, launch, mut worktree) = fixture();
        let path = worktree.path().to_path_buf();
        fs::write(path.join("wip.txt"), "half done\n").unwrap();
        if rejecting_origin {
            rejecting_hook(&temp.path().join("origin.git/hooks/pre-receive"));
        } else {
            launch
                .run(&[
                    "remote",
                    "set-url",
                    "origin",
                    temp.path().join("unreachable.git").to_str().unwrap(),
                ])
                .unwrap();
        }

        let error = worktree
            .preserve_failed_run("main", "session failed")
            .unwrap_err();
        assert!(error.to_string().starts_with("git push"), "{error:#}");
        drop(worktree);

        assert_retained(&launch, &path);
        let git = Git::new(&path);
        assert_eq!(git.run(&["show", "HEAD:wip.txt"]).unwrap(), "half done");
        assert_eq!(
            fs::read_to_string(path.join("wip.txt")).unwrap(),
            "half done\n"
        );
        assert_eq!(
            launch.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
            "thirdshift: failed run (session failed)"
        );
        let origin = Git::new(temp.path().join("origin.git"));
        assert!(
            !origin
                .succeeds(&["show-ref", "--verify", "--quiet", "refs/heads/issue-7"])
                .unwrap()
        );
    }
}

#[test]
fn recorded_interruption_allows_preservation_and_cleanup_but_still_blocks_ordinary_git() {
    with_recorded_signal(
        "worktree::preservation_tests::recorded_interruption_allows_preservation_and_cleanup_but_still_blocks_ordinary_git",
        |signal| {
            let (temp, launch, mut worktree) = fixture();
            let path = worktree.path().to_path_buf();
            fs::write(path.join("wip.txt"), "half done\n").unwrap();
            signal_hook::low_level::raise(signal).unwrap();

            worktree.preserve_failed_run("main", "interrupted").unwrap();

            assert!(interrupt::requested());
            assert_eq!(worktree.head().unwrap_err().to_string(), "interrupted");
            assert_eq!(worktree.push().unwrap_err().to_string(), "interrupted");
            drop(worktree);
            let origin = Git::new(temp.path().join("origin.git")).completion();
            assert_eq!(
                origin.run(&["show", "issue-7:wip.txt"]).unwrap(),
                "half done"
            );
            assert_eq!(
                origin.run(&["log", "-1", "--format=%s", BRANCH]).unwrap(),
                "thirdshift: failed run (interrupted)"
            );
            assert_cleaned_up(&launch.completion(), &path);
            assert!(interrupt::requested());
            assert_eq!(
                launch.run(&["rev-parse", "HEAD"]).unwrap_err().to_string(),
                "interrupted"
            );
            assert_eq!(
                Git::new(launch.dir())
                    .run(&["rev-parse", "HEAD"])
                    .unwrap_err()
                    .to_string(),
                "interrupted"
            );
        },
    );
}

#[test]
fn successful_preservation_after_an_error_restores_cleanup_even_with_unchanged_work() {
    let (_temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    let index = path.join(git.run(&["rev-parse", "--git-path", "index"]).unwrap());
    let original_index = fs::read(&index).unwrap();
    fs::remove_file(&index).unwrap();
    fs::create_dir(&index).unwrap();
    assert!(
        worktree
            .preserve_failed_run("main", "session failed")
            .is_err()
    );
    fs::remove_dir(&index).unwrap();
    fs::write(index, original_index).unwrap();

    worktree
        .preserve_failed_run("main", "session failed")
        .unwrap();
    drop(worktree);

    assert!(!launch.on_origin(BRANCH).unwrap());
    assert_cleaned_up(&launch, &path);
}

#[test]
fn preservation_compares_with_the_last_fetched_settled_base_without_fetching_again() {
    let (temp, launch) = super::tests::launch_directory();
    launch.run(&["checkout", "-q", "-b", "develop"]).unwrap();
    fs::write(launch.dir().join("develop.txt"), "settled base\n").unwrap();
    launch.run(&["add", "-A"]).unwrap();
    launch.run(&["commit", "-q", "-m", "Develop base"]).unwrap();
    launch.run(&["push", "-q", "origin", "develop"]).unwrap();
    let mut worktree = Worktree::create_fresh(&launch, "work", BRANCH, "develop").unwrap();
    let path = worktree.path().to_path_buf();
    let head = worktree.head().unwrap();
    fs::write(launch.dir().join("later.txt"), "new base work\n").unwrap();
    launch.run(&["add", "-A"]).unwrap();
    launch
        .run(&["commit", "-q", "-m", "Later base work"])
        .unwrap();
    let origin = Git::new(temp.path().join("origin.git"));
    origin
        .run(&["fetch", launch.dir().to_str().unwrap(), "develop:develop"])
        .unwrap();

    worktree
        .preserve_failed_run("develop", "session failed")
        .unwrap();

    assert_eq!(worktree.head().unwrap(), head);
    assert_eq!(
        origin.run(&["show", "develop:later.txt"]).unwrap(),
        "new base work"
    );
    assert!(!launch.on_origin(BRANCH).unwrap());
    drop(worktree);
    assert_cleaned_up(&launch, &path);
}

#[test]
fn a_completed_nonzero_cached_diff_probe_still_makes_and_pushes_the_failure_marker() {
    let (temp, launch, mut worktree) = fixture();
    let path = worktree.path().to_path_buf();
    // A missing comparison ref makes Git's probe exit nonzero even though
    // the index is unchanged. Preservation keeps the existing boolean semantics.
    launch
        .run(&["update-ref", "-d", "refs/remotes/origin/main"])
        .unwrap();

    worktree
        .preserve_failed_run("main", "session failed")
        .unwrap();
    drop(worktree);

    let origin = Git::new(temp.path().join("origin.git"));
    assert_eq!(
        origin.run(&["log", "-2", "--format=%s", BRANCH]).unwrap(),
        "thirdshift: failed run (session failed)\nInitial"
    );
    assert!(
        origin
            .succeeds(&["diff", "--quiet", "main", BRANCH])
            .unwrap()
    );
    assert_cleaned_up(&launch, &path);
}

#[test]
fn unwinding_while_handling_a_preservation_error_keeps_the_local_work() {
    let (_temp, launch, worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let git = Git::new(&path);
    git.run(&["config", "user.name", ""]).unwrap();
    fs::write(path.join("wip.txt"), "half done\n").unwrap();

    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let mut worktree = worktree;
        worktree
            .preserve_failed_run("main", "session failed")
            .unwrap();
    }));

    assert!(unwound.is_err());
    assert_retained(&launch, &path);
    assert_eq!(git.run(&["show", ":wip.txt"]).unwrap(), "half done");
    assert_eq!(
        fs::read_to_string(path.join("wip.txt")).unwrap(),
        "half done\n"
    );
}
