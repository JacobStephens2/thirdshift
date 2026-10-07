//! Failed run preservation through Worktree, with real local Git and origin.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::*;
use crate::interrupt;
use crate::test_support::with_recorded_signal;

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
