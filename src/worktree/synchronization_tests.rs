//! Synchronization through Worktree, with real local Git and a bare origin.

use super::*;
use crate::test_support::{git_fault, with_recorded_signal};
use std::fs;
use std::os::unix::fs::symlink;
use std::time::Duration;

fn commit(git: &Git, message: &str) -> String {
    git.run(&["commit", "-q", "--allow-empty", "-m", message])
        .unwrap();
    git.run(&["rev-parse", "HEAD"]).unwrap()
}

fn fixture() -> (tempfile::TempDir, Git, Worktree) {
    let (temp, launch) = super::tests::launch_directory();
    let worktree = Worktree::create_fresh(&launch, "work", "issue-7", "main").unwrap();
    (temp, launch, worktree)
}

fn foreign_origin() -> (tempfile::TempDir, Git, Worktree, String) {
    let (temp, launch, worktree) = fixture();
    launch
        .run(&["checkout", "-q", "-b", "other-machine"])
        .unwrap();
    let foreign = commit(&launch, "Foreign commit");
    launch
        .run(&["push", "-q", "origin", "HEAD:issue-7"])
        .unwrap();
    (temp, launch, worktree, foreign)
}

fn clean_commit(merge: Merge) -> String {
    match merge {
        Merge::Clean { commit } => commit,
        Merge::Conflicted(_) => panic!("merge should be clean"),
    }
}

fn contains(worktree: &Worktree, commit: &str) -> bool {
    Git::new(worktree.path())
        .succeeds(&["merge-base", "--is-ancestor", commit, "HEAD"])
        .unwrap()
}

fn conflicting_merge(
    upstream: &str,
) -> (
    tempfile::TempDir,
    Git,
    Worktree,
    PendingMerge,
    String,
    String,
) {
    let (temp, launch) = super::tests::launch_directory();
    std::fs::write(launch.dir().join("conflict.txt"), "shared\n").unwrap();
    launch.run(&["add", "conflict.txt"]).unwrap();
    let shared = commit(&launch, "Shared file");
    launch.run(&["push", "-q", "origin", "main"]).unwrap();
    let worktree = Worktree::create_fresh(&launch, "work", "issue-7", "main").unwrap();
    let git = Git::new(worktree.path());
    std::fs::write(worktree.path().join("conflict.txt"), "ours\n").unwrap();
    git.run(&["add", "conflict.txt"]).unwrap();
    commit(&git, "Own conflicting work");
    std::fs::write(launch.dir().join("conflict.txt"), "theirs\n").unwrap();
    launch.run(&["add", "conflict.txt"]).unwrap();
    let selected = commit(&launch, "Conflicting origin work");
    launch
        .run(&["push", "-q", "origin", &format!("HEAD:{upstream}")])
        .unwrap();
    let merged = if upstream == "main" {
        worktree.merge_base_branch("main").unwrap()
    } else {
        worktree
            .merge_new_commits(worktree.new_commits_on_origin().unwrap())
            .unwrap()
    };
    let Merge::Conflicted(pending) = merged else {
        panic!("merge should conflict");
    };
    assert_eq!(git.run(&["rev-parse", "MERGE_HEAD"]).unwrap(), selected);
    (temp, launch, worktree, pending, selected, shared)
}

#[test]
fn a_base_merge_refuses_a_switched_branch_without_advancing_unrelated_work() {
    let (temp, launch, worktree) = fixture();
    let git = Git::new(worktree.path());
    let initial = worktree.head().unwrap();
    git.run(&["checkout", "-q", "-b", "manual"]).unwrap();
    std::fs::write(worktree.path().join("manual.txt"), "manual work\n").unwrap();
    git.run(&["add", "manual.txt"]).unwrap();
    commit(&launch, "Base advances");
    launch.run(&["push", "-q", "origin", "main"]).unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();
    let index = git.run(&["diff", "--cached"]).unwrap();
    let origin = Git::new(temp.path().join("origin.git"));
    let remote_refs = origin.run(&["show-ref"]).unwrap();

    let error = worktree.merge_base_branch("main").unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains(worktree.path().to_str().unwrap()), "{error}");
    assert!(error.contains("expected Issue branch issue-7"), "{error}");
    assert!(error.contains("identity changed"), "{error}");
    drop(worktree);

    assert_eq!(git.run(&["rev-parse", "HEAD"]).unwrap(), initial);
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
    assert_eq!(git.run(&["diff", "--cached"]).unwrap(), index);
    assert_eq!(origin.run(&["show-ref"]).unwrap(), remote_refs);
    assert_eq!(
        std::fs::read_to_string(git.dir().join("manual.txt")).unwrap(),
        "manual work\n"
    );
}

#[test]
fn a_base_merge_refuses_a_recreated_checkout_on_the_same_branch_and_head() {
    let (temp, launch, worktree) = fixture();
    let path = worktree.path().to_path_buf();
    let initial = worktree.head().unwrap();
    launch
        .run(&["worktree", "remove", "--force", path.to_str().unwrap()])
        .unwrap();
    launch
        .run(&["worktree", "add", path.to_str().unwrap(), "issue-7"])
        .unwrap();
    let replacement = Git::new(&path);
    assert_eq!(replacement.run(&["rev-parse", "HEAD"]).unwrap(), initial);
    fs::write(path.join("replacement.txt"), "replacement work\n").unwrap();
    replacement.run(&["add", "replacement.txt"]).unwrap();
    commit(&launch, "Base advances");
    launch.run(&["push", "-q", "origin", "main"]).unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();
    let index = replacement.run(&["diff", "--cached"]).unwrap();
    let origin = Git::new(temp.path().join("origin.git"));
    let remote_refs = origin.run(&["show-ref"]).unwrap();

    let error = worktree.merge_base_branch("main").unwrap_err();
    assert!(format!("{error:#}").contains("replaced"), "{error:#}");
    drop(worktree);

    assert!(path.exists());
    assert_eq!(replacement.run(&["rev-parse", "HEAD"]).unwrap(), initial);
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
    assert_eq!(replacement.run(&["diff", "--cached"]).unwrap(), index);
    assert_eq!(origin.run(&["show-ref"]).unwrap(), remote_refs);
    assert_eq!(
        fs::read_to_string(path.join("replacement.txt")).unwrap(),
        "replacement work\n"
    );
}

#[test]
fn every_ordinary_operation_refuses_changed_or_uncertain_ownership_even_without_new_commits() {
    for damage in [
        "switch",
        "detach",
        "lock",
        "git-link",
        "backlink-link",
        "common-link",
        "missing-backlink",
        "wrong-backlink",
        "duplicate-registration",
        "root",
        "admin",
        "common",
        "missing-root",
    ] {
        let (temp, launch, worktree) = fixture();
        worktree.push().unwrap();
        let observed = worktree.new_commits_on_origin().unwrap();
        let path = worktree.path().to_path_buf();
        let git = Git::new(&path);
        git.run(&["branch", "manual"]).unwrap();
        fs::write(path.join("manual.txt"), "manual work\n").unwrap();
        git.run(&["add", "manual.txt"]).unwrap();
        let admin = PathBuf::from(git.run(&["rev-parse", "--absolute-git-dir"]).unwrap());
        let common = launch.common_dir().unwrap();
        let refs = launch.run(&["show-ref"]).unwrap();
        let index = fs::read(admin.join("index")).unwrap();
        let origin = Git::new(temp.path().join("origin.git"));
        let remote_refs = origin.run(&["show-ref"]).unwrap();
        let target = match damage {
            "root" | "missing-root" => path.clone(),
            "admin" => admin.clone(),
            "common" => common.clone(),
            "git-link" => path.join(".git"),
            "common-link" => admin.join("commondir"),
            _ => admin.join("gitdir"),
        };
        let saved = target.with_extension("saved");
        match damage {
            "switch" => {
                git.run(&["checkout", "-q", "manual"]).unwrap();
            }
            "detach" => {
                git.run(&["checkout", "-q", "--detach"]).unwrap();
            }
            "lock" => {
                launch
                    .run(&["worktree", "lock", path.to_str().unwrap()])
                    .unwrap();
            }
            "duplicate-registration" => {
                fs::create_dir(admin.with_file_name("duplicate")).unwrap();
                for file in ["HEAD", "gitdir", "commondir"] {
                    fs::copy(
                        admin.join(file),
                        admin.with_file_name("duplicate").join(file),
                    )
                    .unwrap();
                }
            }
            _ => {
                fs::rename(&target, &saved).unwrap();
                match damage {
                    "git-link" | "backlink-link" | "common-link" => {
                        symlink(&saved, &target).unwrap()
                    }
                    "wrong-backlink" => {
                        fs::write(&target, launch.dir().join(".git").to_str().unwrap()).unwrap()
                    }
                    "missing-backlink" | "missing-root" => {}
                    _ => fs::create_dir(&target).unwrap(),
                }
            }
        }

        for result in [
            worktree.push(),
            worktree.head().map(|_| ()),
            worktree.merge_base_branch("main").map(|_| ()),
            worktree.new_commits_on_origin().map(|_| ()),
            worktree.merge_new_commits(observed).map(|_| ()),
            worktree.fast_forward_to_origin(),
            worktree.base_branch_moved("main").map(|_| ()),
        ] {
            let error = format!("{:#}", result.unwrap_err());
            assert!(error.contains(path.to_str().unwrap()), "{damage}: {error}");
            assert!(
                error.contains("expected Issue branch issue-7"),
                "{damage}: {error}"
            );
        }
        match damage {
            "switch" | "detach" => {
                git.run(&["checkout", "-q", "issue-7"]).unwrap();
            }
            "lock" => {
                launch
                    .run(&["worktree", "unlock", path.to_str().unwrap()])
                    .unwrap();
            }
            "duplicate-registration" => {
                fs::remove_dir_all(admin.with_file_name("duplicate")).unwrap()
            }
            _ => {
                match damage {
                    "root" | "admin" | "common" => {
                        assert_eq!(
                            fs::read_dir(&target).unwrap().count(),
                            0,
                            "replacement mutated: {damage}"
                        );
                        fs::remove_dir(&target).unwrap();
                    }
                    "missing-backlink" | "missing-root" => {}
                    _ => fs::remove_file(&target).unwrap(),
                }
                fs::rename(&saved, &target).unwrap();
            }
        }
        assert_eq!(launch.run(&["show-ref"]).unwrap(), refs, "{damage}");
        assert_eq!(fs::read(admin.join("index")).unwrap(), index, "{damage}");
        assert_eq!(origin.run(&["show-ref"]).unwrap(), remote_refs, "{damage}");
        assert_eq!(
            fs::read_to_string(path.join("manual.txt")).unwrap(),
            "manual work\n"
        );
    }
}

#[test]
fn pending_merge_validation_refuses_a_switched_checkout_after_a_valid_repair() {
    let (_temp, _launch, worktree, pending, _, _) = conflicting_merge("main");
    let git = Git::new(worktree.path());
    fs::write(worktree.path().join("conflict.txt"), "resolved\n").unwrap();
    git.run(&["add", "conflict.txt"]).unwrap();
    git.run(&["commit", "-q", "--no-edit"]).unwrap();
    worktree.ensure_merged(&pending).unwrap();
    git.run(&["checkout", "-q", "-b", "manual"]).unwrap();
    let refs = git.run(&["show-ref"]).unwrap();

    let error = worktree.ensure_merged(&pending).unwrap_err();

    assert!(
        format!("{error:#}").contains("expected Issue branch issue-7"),
        "{error:#}"
    );
    assert_eq!(git.run(&["show-ref"]).unwrap(), refs);
}

#[test]
fn confirmed_merge_deletion_uses_the_captured_repository_despite_a_different_checkout_origin() {
    for replaced in [false, true] {
        let (temp, launch, worktree) = fixture();
        worktree.push().unwrap();
        let (other_temp, other_launch) = super::tests::launch_directory();
        other_launch
            .run(&["checkout", "-q", "-b", "issue-7"])
            .unwrap();
        other_launch
            .run(&["push", "-q", "origin", "issue-7"])
            .unwrap();
        let other_origin = other_temp.path().join("origin.git");
        let path = worktree.path().to_path_buf();
        if replaced {
            fs::rename(&path, path.with_extension("saved")).unwrap();
            launch
                .run(&[
                    "clone",
                    "-q",
                    "--branch",
                    "issue-7",
                    other_origin.to_str().unwrap(),
                    path.to_str().unwrap(),
                ])
                .unwrap();
        } else {
            launch
                .run(&["config", "extensions.worktreeConfig", "true"])
                .unwrap();
            let git = Git::new(&path);
            git.run(&["checkout", "-q", "-b", "manual"]).unwrap();
            git.run(&[
                "config",
                "--worktree",
                "remote.origin.url",
                other_origin.to_str().unwrap(),
            ])
            .unwrap();
        }
        fs::write(path.join("manual.txt"), "unrelated work\n").unwrap();
        let other_refs = Git::new(&other_origin).run(&["show-ref"]).unwrap();

        worktree.delete_from_origin().unwrap();
        drop(worktree);

        assert!(
            !Git::new(temp.path().join("origin.git"))
                .succeeds(&["show-ref", "--verify", "refs/heads/issue-7"])
                .unwrap()
        );
        assert_eq!(
            Git::new(other_origin).run(&["show-ref"]).unwrap(),
            other_refs
        );
        assert_eq!(
            fs::read_to_string(path.join("manual.txt")).unwrap(),
            "unrelated work\n"
        );
    }
}

#[test]
fn completion_refuses_a_replaced_common_repository_before_any_remote_command() {
    let (temp, launch, worktree) = fixture();
    worktree.push().unwrap();
    let common = launch.common_dir().unwrap();
    let saved = common.with_extension("saved");
    let origin = Git::new(temp.path().join("origin.git"));
    let refs = origin.run(&["show-ref"]).unwrap();
    fs::rename(&common, &saved).unwrap();
    fs::create_dir(&common).unwrap();

    let result = worktree.delete_from_origin();
    let replacement_entries = fs::read_dir(&common).unwrap().count();
    fs::remove_dir(&common).unwrap();
    fs::rename(&saved, &common).unwrap();

    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("expected Issue branch issue-7"), "{error}");
    assert!(error.contains(worktree.path().to_str().unwrap()), "{error}");
    assert!(error.contains("common repository identity"), "{error}");
    assert_eq!(replacement_entries, 0);
    assert_eq!(origin.run(&["show-ref"]).unwrap(), refs);
}

#[test]
fn completion_rechecks_repository_identity_after_deletion_and_absent_branch_observation() {
    if git_fault(
        "worktree::synchronization_tests::completion_rechecks_repository_identity_after_deletion_and_absent_branch_observation",
        r#"
if test -f "$THIRDSHIFT_FAULT_MARKER.command" && test ! -e "$THIRDSHIFT_FAULT_MARKER" && test "$1" = "$(cat "$THIRDSHIFT_FAULT_MARKER.command")"; then
  status=0
  "$THIRDSHIFT_REAL_GIT" "$@" || status=$?
  common=$("$THIRDSHIFT_REAL_GIT" rev-parse --git-common-dir)
  mv "$common" "$common.saved"
  mkdir "$common"
  touch "$THIRDSHIFT_FAULT_MARKER"
  exit "$status"
fi
"#,
    ).is_some() { return; }
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    for command in ["push", "ls-remote"] {
        let _ = fs::remove_file(&marker);
        let _ = fs::remove_file(marker.with_extension("command"));
        let (temp, launch, worktree) = fixture();
        if command == "push" {
            worktree.push().unwrap();
        }
        let common = launch.common_dir().unwrap();
        fs::write(marker.with_extension("command"), command).unwrap();

        let result = worktree.delete_from_origin();
        let replacement_entries = fs::read_dir(&common).unwrap().count();
        fs::remove_dir(&common).unwrap();
        fs::rename(common.with_extension("saved"), &common).unwrap();
        fs::remove_file(marker.with_extension("command")).unwrap();

        assert!(
            marker.exists(),
            "completion command not exercised: {command}"
        );
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("expected Issue branch issue-7"), "{error}");
        assert!(error.contains("common repository identity"), "{error}");
        assert_eq!(replacement_entries, 0);
        assert!(
            !Git::new(temp.path().join("origin.git"))
                .succeeds(&["show-ref", "--verify", "refs/heads/issue-7"])
                .unwrap()
        );
    }
}

#[test]
fn completion_refuses_a_launch_directory_that_no_longer_belongs_to_the_captured_repository() {
    let (temp, launch) = super::tests::launch_directory();
    let linked_path = temp.path().join("launch");
    launch
        .run(&[
            "worktree",
            "add",
            "-b",
            "launch-branch",
            linked_path.to_str().unwrap(),
            "main",
        ])
        .unwrap();
    let linked_launch = Git::new(&linked_path);
    let worktree = Worktree::create_fresh(&linked_launch, "launch", "issue-7", "main").unwrap();
    worktree.push().unwrap();
    let (other_temp, other_launch) = super::tests::launch_directory();
    other_launch
        .run(&["push", "-q", "origin", "HEAD:issue-7"])
        .unwrap();
    fs::rename(&linked_path, linked_path.with_extension("saved")).unwrap();
    launch
        .run(&[
            "clone",
            "-q",
            other_temp.path().join("origin.git").to_str().unwrap(),
            linked_path.to_str().unwrap(),
        ])
        .unwrap();
    let original = Git::new(temp.path().join("origin.git"));
    let replacement = Git::new(other_temp.path().join("origin.git"));
    let original_refs = original.run(&["show-ref"]).unwrap();
    let replacement_refs = replacement.run(&["show-ref"]).unwrap();

    let error = worktree.delete_from_origin().unwrap_err();

    assert!(
        format!("{error:#}").contains("launch no longer belongs"),
        "{error:#}"
    );
    assert_eq!(original.run(&["show-ref"]).unwrap(), original_refs);
    assert_eq!(replacement.run(&["show-ref"]).unwrap(), replacement_refs);
}

#[test]
fn confirmed_merge_deletion_and_absent_branch_reconciliation_finish_after_interruption() {
    with_recorded_signal(
        "worktree::synchronization_tests::confirmed_merge_deletion_and_absent_branch_reconciliation_finish_after_interruption",
        |signal| {
            let (temp, launch, worktree) = fixture();
            let absent = Worktree::create_fresh(&launch, "work", "issue-8", "main").unwrap();
            worktree.push().unwrap();
            Git::new(worktree.path())
                .run(&["checkout", "-q", "-b", "manual"])
                .unwrap();
            signal_hook::low_level::raise(signal).unwrap();

            worktree.delete_from_origin().unwrap();
            absent.delete_from_origin().unwrap();

            assert!(
                !Git::new(temp.path().join("origin.git"))
                    .completion()
                    .succeeds(&["show-ref", "--verify", "refs/heads/issue-7"])
                    .unwrap()
            );
            assert!(crate::interrupt::requested());
            assert_eq!(worktree.head().unwrap_err().to_string(), "interrupted");
        },
    );
}

#[test]
fn ownership_changes_during_commands_refuse_later_mutations_and_successful_results() {
    if git_fault(
        "worktree::synchronization_tests::ownership_changes_during_commands_refuse_later_mutations_and_successful_results",
        r#"
if test -f "$THIRDSHIFT_FAULT_MARKER.command" && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  command=$(cat "$THIRDSHIFT_FAULT_MARKER.command")
  if test "$1" = "$command"; then
    status=0
    "$THIRDSHIFT_REAL_GIT" "$@" || status=$?
    "$THIRDSHIFT_REAL_GIT" symbolic-ref HEAD refs/heads/manual
    printf 'unrelated work\n' > unrelated.txt
    touch "$THIRDSHIFT_FAULT_MARKER"
    exit "$status"
  fi
fi
"#,
    ).is_some() { return; }
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    for command in [
        "fetch",
        "push",
        "rev-list",
        "merge",
        "conflict",
        "merge-base",
    ] {
        let _ = fs::remove_file(&marker);
        let _ = fs::remove_file(marker.with_extension("command"));
        let (temp, launch, worktree) = fixture();
        let git = Git::new(worktree.path());
        fs::write(worktree.path().join("conflict.txt"), "ours\n").unwrap();
        git.run(&["add", "conflict.txt"]).unwrap();
        let own_head = commit(&git, "Own work");
        git.run(&["branch", "manual"]).unwrap();
        if command != "push" {
            worktree.push().unwrap();
        }
        if matches!(command, "fetch" | "merge" | "conflict") {
            if command == "conflict" {
                fs::write(launch.dir().join("conflict.txt"), "theirs\n").unwrap();
                launch.run(&["add", "conflict.txt"]).unwrap();
            }
            commit(&launch, "Base advances");
            launch.run(&["push", "-q", "origin", "main"]).unwrap();
        }
        let refs = launch.run(&["show-ref"]).unwrap();
        let index = git.run(&["diff", "--cached"]).unwrap();
        fs::write(
            marker.with_extension("command"),
            if command == "conflict" {
                "merge"
            } else {
                command
            },
        )
        .unwrap();

        let result = match command {
            "push" => worktree.push(),
            "rev-list" => worktree.new_commits_on_origin().map(|_| ()),
            "merge-base" => worktree.base_branch_moved("main").map(|_| ()),
            _ => worktree.merge_base_branch("main").map(|_| ()),
        };

        assert!(marker.exists(), "fault not exercised: {command}");
        fs::remove_file(marker.with_extension("command")).unwrap();
        let error = format!("{:#}", result.unwrap_err());
        assert!(
            error.contains("expected Issue branch issue-7"),
            "{command}: {error}"
        );
        assert_eq!(git.run(&["rev-parse", "manual"]).unwrap(), own_head);
        assert_eq!(
            fs::read_to_string(worktree.path().join("unrelated.txt")).unwrap(),
            "unrelated work\n"
        );
        assert!(!launch.on_origin("manual").unwrap());
        if !matches!(command, "merge" | "conflict") {
            assert_eq!(git.run(&["rev-parse", "issue-7"]).unwrap(), own_head);
            assert_eq!(git.run(&["diff", "--cached"]).unwrap(), index);
        }
        if matches!(command, "rev-list" | "merge-base") {
            assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
        }
        if command == "push" {
            assert_eq!(
                Git::new(temp.path().join("origin.git"))
                    .run(&["rev-parse", "issue-7"])
                    .unwrap(),
                own_head
            );
        }
        let lock = fs::File::open(
            launch
                .common_dir()
                .unwrap()
                .join("thirdshift-worktrees.lock"),
        )
        .unwrap();
        lock.try_lock()
            .expect("operation did not release its lock on ownership failure");
        drop(lock);
        drop(worktree);
        assert!(git.dir().exists(), "invalid checkout was disposed of");
    }
}

#[test]
fn ownership_changing_after_a_git_lock_failure_refuses_the_retry_before_it_merges_unrelated_work() {
    if git_fault(
        "worktree::synchronization_tests::ownership_changing_after_a_git_lock_failure_refuses_the_retry_before_it_merges_unrelated_work",
        r#"
if test "$1" = merge && test -e "$THIRDSHIFT_FAULT_MARKER.armed" && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  "$THIRDSHIFT_REAL_GIT" symbolic-ref HEAD refs/heads/manual
  touch "$THIRDSHIFT_FAULT_MARKER"
  printf "fatal: Unable to create '%s/index.lock': File exists\n" "$PWD" >&2
  exit 1
fi
"#,
    ).is_some() { return; }
    let (_temp, launch, worktree) = fixture();
    let git = Git::new(worktree.path());
    let initial = worktree.head().unwrap();
    git.run(&["branch", "manual"]).unwrap();
    fs::write(worktree.path().join("manual.txt"), "unrelated work\n").unwrap();
    git.run(&["add", "manual.txt"]).unwrap();
    commit(&launch, "Base advances");
    launch.run(&["push", "-q", "origin", "main"]).unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();
    let index = git.run(&["diff", "--cached"]).unwrap();
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    fs::write(marker.with_extension("armed"), "armed").unwrap();

    let error = worktree.merge_base_branch("main").unwrap_err();

    assert!(marker.exists(), "lock failure did not occur");
    assert!(
        format!("{error:#}").contains("expected Issue branch issue-7"),
        "{error:#}"
    );
    assert_eq!(
        git.run(&["rev-parse", "manual"]).unwrap(),
        initial,
        "merge retry mutated unrelated branch"
    );
    assert_eq!(launch.run(&["show-ref"]).unwrap(), refs);
    assert_eq!(git.run(&["diff", "--cached"]).unwrap(), index);
    assert_eq!(
        fs::read_to_string(worktree.path().join("manual.txt")).unwrap(),
        "unrelated work\n"
    );
}

#[test]
fn ordinary_operations_wait_for_ownership_and_refs_locks_before_changing_work() {
    for name in ["thirdshift-worktrees.lock", "thirdshift-worktree-refs.lock"] {
        let (_temp, launch, worktree) = fixture();
        let initial = worktree.head().unwrap();
        commit(&launch, "Base advances");
        launch.run(&["push", "-q", "origin", "main"]).unwrap();
        let held = launch.lock(name).unwrap();
        let (finished, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = worktree.merge_base_branch("main");
            let _ = finished.send(());
            (result, worktree)
        });
        let finished_while_held = received.recv_timeout(Duration::from_millis(300)).is_ok();
        let before_release = launch.run(&["rev-parse", "issue-7"]);
        drop(held);
        let joined = worker.join();

        assert!(!finished_while_held, "operation bypassed {name}");
        assert_eq!(before_release.unwrap(), initial);
        let (result, worktree) = joined.unwrap();
        assert!(matches!(result.unwrap(), Merge::Clean { .. }));
        assert_eq!(
            worktree.head().unwrap(),
            launch.run(&["rev-parse", "HEAD"]).unwrap()
        );
    }
}

#[test]
fn ownership_changing_while_fetch_waits_for_refs_refuses_the_fetch_before_it_updates_refs() {
    let (_temp, launch, worktree) = fixture();
    let git = Git::new(worktree.path());
    git.run(&["branch", "manual"]).unwrap();
    commit(&launch, "Base advances after acquisition");
    launch.run(&["push", "-q", "origin", "main"]).unwrap();
    // Rewind the tracking ref so an erroneously executed fetch is observable.
    let initial = worktree.head().unwrap();
    launch
        .run(&["update-ref", "refs/remotes/origin/main", &initial])
        .unwrap();
    let refs = launch.run(&["show-ref"]).unwrap();
    let held = launch.lock_worktree_refs().unwrap();
    let lock = fs::File::open(
        launch
            .common_dir()
            .unwrap()
            .join("thirdshift-worktrees.lock"),
    )
    .unwrap();
    let (started, starting) = std::sync::mpsc::channel();
    let (finished, received) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = started.send(());
        let result = worktree.merge_base_branch("main");
        let _ = finished.send(());
        (result, worktree)
    });
    let started = starting.recv_timeout(Duration::from_secs(2)).is_ok();
    // Confirm this operation holds ownership while waiting for the refs lock.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let waiting = loop {
        match lock.try_lock() {
            Err(fs::TryLockError::WouldBlock) => break true,
            Ok(()) => {
                if lock.unlock().is_err() {
                    break false;
                }
            }
            Err(_) => break false,
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let blocked_on_refs = received.recv_timeout(Duration::from_millis(500)).is_err();
    let switched = git.run(&["checkout", "-q", "manual"]);
    drop(held);
    let joined = worker.join();

    assert!(
        started && waiting && blocked_on_refs,
        "fetch did not wait for refs while holding ownership"
    );
    switched.unwrap();
    let (result, worktree) = joined.unwrap();
    let error = format!("{:#}", result.unwrap_err());
    assert!(error.contains("expected Issue branch issue-7"), "{error}");
    assert_eq!(
        launch.run(&["show-ref"]).unwrap(),
        refs,
        "fetch ran after ownership changed"
    );
    assert_eq!(git.run(&["rev-parse", "manual"]).unwrap(), initial);
    drop(worktree);
}

#[test]
fn an_ordinary_operation_serializes_other_checkouts_until_its_git_command_finishes() {
    if git_fault(
        "worktree::synchronization_tests::an_ordinary_operation_serializes_other_checkouts_until_its_git_command_finishes",
        r#"
if test "$1" = fetch && test -e "$THIRDSHIFT_FAULT_MARKER.armed"; then
  "$THIRDSHIFT_REAL_GIT" "$@"
  touch "$THIRDSHIFT_FAULT_MARKER"
  for _ in $(seq 500); do
    test -e "$THIRDSHIFT_FAULT_MARKER.release" && break
    sleep 0.01
  done
  test -e "$THIRDSHIFT_FAULT_MARKER.release"
  exit 0
fi
"#,
    ).is_some() { return; }
    let (_temp, launch, worktree) = fixture();
    let other = Worktree::create_fresh(&launch, "work", "issue-8", "main").unwrap();
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    fs::write(marker.with_extension("armed"), "armed").unwrap();
    let (finished, received) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = worktree.merge_base_branch("main");
        let _ = finished.send(());
        (result, worktree)
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !marker.exists() && received.try_recv().is_err() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let (started, starting) = std::sync::mpsc::channel();
    let (observed, observing) = std::sync::mpsc::channel();
    let observer = std::thread::spawn(move || {
        let _ = started.send(());
        let result = other.head();
        let _ = observed.send(());
        (result, other)
    });
    let observer_started = starting.recv_timeout(Duration::from_secs(2)).is_ok();
    let observed_while_fetching = observing.recv_timeout(Duration::from_millis(300)).is_ok();
    let released = fs::write(marker.with_extension("release"), "released");
    let merged = worker.join();
    let observation = observer.join();

    released.unwrap();
    assert!(marker.exists(), "controlled fetch did not start");
    assert!(observer_started);
    assert!(
        !observed_while_fetching,
        "ordinary observation overlapped another checkout's operation"
    );
    let (result, worktree) = merged.unwrap();
    let (head, _other) = observation.unwrap();
    assert!(matches!(result.unwrap(), Merge::Clean { .. }));
    assert_eq!(head.unwrap(), worktree.head().unwrap());
}

#[test]
fn interruption_while_waiting_for_refs_releases_the_operation_lock() {
    interrupted_lock_wait(
        "worktree::synchronization_tests::interruption_while_waiting_for_refs_releases_the_operation_lock",
        "thirdshift-worktree-refs.lock",
    );
}

#[test]
fn interruption_while_waiting_for_ownership_stops_the_operation() {
    interrupted_lock_wait(
        "worktree::synchronization_tests::interruption_while_waiting_for_ownership_stops_the_operation",
        "thirdshift-worktrees.lock",
    );
}

fn interrupted_lock_wait(test_name: &str, lock_name: &str) {
    with_recorded_signal(test_name, |signal| {
        let (_temp, launch, worktree) = fixture();
        let initial = worktree.head().unwrap();
        let held = launch.lock(lock_name).unwrap();
        let (finished, received) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = worktree.merge_base_branch("main");
            let _ = finished.send(());
            (result, worktree)
        });
        let finished_before_signal = received.recv_timeout(Duration::from_millis(300)).is_ok();
        let raised = signal_hook::low_level::raise(signal);
        let stopped_while_held = received.recv_timeout(Duration::from_secs(3)).is_ok();
        drop(held);
        let joined = worker.join();

        raised.unwrap();
        assert!(!finished_before_signal);
        assert!(stopped_while_held, "ordinary fetch ignored interruption");
        let (result, worktree) = joined.unwrap();
        assert_eq!(result.unwrap_err().to_string(), "interrupted");
        let completion = launch.completion();
        assert_eq!(completion.run(&["rev-parse", "issue-7"]).unwrap(), initial);
        let lock = fs::File::open(
            completion
                .common_dir()
                .unwrap()
                .join("thirdshift-worktrees.lock"),
        )
        .unwrap();
        lock.try_lock().unwrap();
        drop(lock);
        drop(worktree);
        assert!(crate::interrupt::requested());
    });
}

#[test]
fn legitimate_commits_can_advance_on_the_acquired_issue_branch() {
    let (temp, launch, worktree) = fixture();
    let advanced = commit(
        &Git::new(worktree.path()),
        "Session commits legitimate work",
    );

    assert_eq!(worktree.head().unwrap(), advanced);
    worktree.push().unwrap();
    assert!(
        worktree
            .new_commits_on_origin()
            .unwrap()
            .commits()
            .is_empty()
    );
    worktree.fast_forward_to_origin().unwrap();
    assert!(!worktree.base_branch_moved("main").unwrap());
    assert_eq!(
        clean_commit(worktree.merge_base_branch("main").unwrap()),
        launch.run(&["rev-parse", "HEAD"]).unwrap()
    );
    assert_eq!(worktree.head().unwrap(), advanced);
    assert_eq!(
        Git::new(temp.path().join("origin.git"))
            .run(&["rev-parse", "issue-7"])
            .unwrap(),
        advanced
    );
}

#[test]
fn a_base_merge_keeps_its_selected_commit_when_the_shared_ref_rewinds() {
    let (_temp, launch) = super::tests::launch_directory();
    let initial = launch.run(&["rev-parse", "HEAD"]).unwrap();
    let worktree = Worktree::create_fresh(&launch, "work", "issue-7", "main").unwrap();
    let base = commit(&launch, "Base advances");
    launch.run(&["push", "-q", "origin", "main"]).unwrap();

    let Merge::Clean { commit } = worktree.merge_base_branch("main").unwrap() else {
        panic!("Base branch merge should be clean");
    };
    launch
        .run(&["update-ref", "refs/remotes/origin/main", &initial])
        .unwrap();

    assert_eq!(commit, base);
    assert_eq!(worktree.head().unwrap(), base);
}

#[test]
fn a_foreign_commit_merge_consumes_only_the_batch_observed_before_the_ref_advanced() {
    let (_temp, launch) = super::tests::launch_directory();
    let worktree = Worktree::create_fresh(&launch, "work", "issue-7", "main").unwrap();
    let own_head = worktree.head().unwrap();
    launch
        .run(&["checkout", "-q", "-b", "other-machine"])
        .unwrap();
    let first = commit(&launch, "First Foreign commit");
    let second = commit(&launch, "Second Foreign commit");
    launch
        .run(&["push", "-q", "origin", "HEAD:issue-7"])
        .unwrap();

    let observed = worktree.new_commits_on_origin().unwrap();
    assert_eq!(observed.commits(), [first, second.clone()]);
    assert_eq!(observed.origin_head(), second);
    assert_eq!(observed.local_head(), own_head);
    assert_eq!(observed.upstream(), "origin/issue-7");
    assert_eq!(
        worktree.head().unwrap(),
        own_head,
        "observation is read-only"
    );

    let late = commit(&launch, "Later Foreign commit");
    launch
        .run(&["push", "-q", "origin", "HEAD:issue-7"])
        .unwrap();
    assert!(matches!(
        worktree.merge_new_commits(observed).unwrap(),
        Merge::Clean { .. }
    ));

    assert_eq!(worktree.head().unwrap(), second);
    assert_eq!(worktree.new_commits_on_origin().unwrap().commits(), [late]);
}

#[test]
fn the_base_branch_moved_check_ignores_a_shadowing_local_branch() {
    let (_temp, launch) = super::tests::launch_directory();
    let worktree = Worktree::create_fresh(&launch, "work", "issue-7", "main").unwrap();
    launch.run(&["branch", "origin/main", "HEAD"]).unwrap();
    commit(&launch, "Base advances");
    launch.run(&["push", "-q", "origin", "main"]).unwrap();

    assert!(worktree.base_branch_moved("main").unwrap());
}

#[test]
fn a_no_op_base_merge_returns_its_selected_commit_without_changing_the_local_head() {
    let (_temp, launch, worktree) = fixture();
    let own = commit(&Git::new(worktree.path()), "Own work");
    let base = launch.run(&["rev-parse", "HEAD"]).unwrap();

    assert_eq!(
        clean_commit(worktree.merge_base_branch("main").unwrap()),
        base
    );
    assert_eq!(worktree.head().unwrap(), own);
}

#[test]
fn base_merges_ignore_shadowing_branch_and_tag_names() {
    for kind in ["branch", "tag"] {
        let (_temp, launch, worktree) = fixture();
        let base = commit(&launch, "Real Base branch advance");
        launch.run(&["push", "-q", "origin", "main"]).unwrap();
        launch.run(&["checkout", "-q", "-b", "shadow"]).unwrap();
        let shadow = commit(&launch, "Unintended commit");
        launch.run(&[kind, "origin/main", &shadow]).unwrap();

        assert!(worktree.base_branch_moved("main").unwrap());
        assert_eq!(
            clean_commit(worktree.merge_base_branch("main").unwrap()),
            base
        );
        assert_eq!(worktree.head().unwrap(), base);
        assert!(!contains(&worktree, &shadow));
        assert!(!worktree.base_branch_moved("main").unwrap());
    }
}

#[test]
fn foreign_observation_and_merge_ignore_shadowing_branch_and_tag_names() {
    for kind in ["branch", "tag"] {
        let (_temp, launch, worktree, foreign) = foreign_origin();
        let own = worktree.head().unwrap();
        launch.run(&[kind, "origin/issue-7", &own]).unwrap();

        let observed = worktree.new_commits_on_origin().unwrap();
        assert_eq!(observed.origin_head(), foreign);
        assert_eq!(observed.commits(), std::slice::from_ref(&foreign));
        assert_eq!(observed.local_head(), own);
        assert_eq!(
            clean_commit(worktree.merge_new_commits(observed).unwrap()),
            foreign
        );
        assert_eq!(worktree.head().unwrap(), foreign);
    }
}

#[test]
fn a_foreign_merge_keeps_its_sample_when_the_shared_ref_is_rewound_or_replaced() {
    for replacement in [false, true] {
        let (_temp, launch, worktree, foreign) = foreign_origin();
        let own = worktree.head().unwrap();
        let observed = worktree.new_commits_on_origin().unwrap();
        let later_ref = if replacement {
            launch.run(&["checkout", "-q", "main"]).unwrap();
            commit(&launch, "Unrelated replacement")
        } else {
            own
        };
        launch
            .run(&["update-ref", "refs/remotes/origin/issue-7", &later_ref])
            .unwrap();

        assert_eq!(
            clean_commit(worktree.merge_new_commits(observed).unwrap()),
            foreign
        );
        assert_eq!(worktree.head().unwrap(), foreign);
        if replacement {
            assert!(!contains(&worktree, &later_ref));
        }
        assert!(
            worktree
                .new_commits_on_origin()
                .unwrap()
                .commits()
                .is_empty()
        );
    }
}

#[test]
fn another_worktree_cannot_consume_a_foreign_observation() {
    let (_temp, launch, worktree, foreign) = foreign_origin();
    let observed = worktree.new_commits_on_origin().unwrap();
    let other = Worktree::create_fresh(&launch, "work", "issue-8", "main").unwrap();
    let before = other.head().unwrap();

    let error = other.merge_new_commits(observed).unwrap_err();

    assert_eq!(
        error.to_string(),
        "the Foreign commit observation belongs to another worktree"
    );
    assert_eq!(other.head().unwrap(), before);
    assert!(!contains(&other, &foreign));
}

#[test]
fn a_recreated_worktree_at_the_same_path_cannot_consume_an_earlier_observation() {
    let (_temp, launch, worktree, foreign) = foreign_origin();
    let observed = worktree.new_commits_on_origin().unwrap();
    let path = worktree.path().to_path_buf();
    let before = worktree.head().unwrap();
    drop(worktree);
    let replacement = Worktree::create_fresh(&launch, "work", "issue-7", "main").unwrap();
    assert_eq!(replacement.path(), path);

    let error = replacement.merge_new_commits(observed).unwrap_err();

    assert_eq!(
        error.to_string(),
        "the Foreign commit observation belongs to another worktree"
    );
    assert_eq!(replacement.head().unwrap(), before);
    assert!(!contains(&replacement, &foreign));
}

#[test]
fn a_changed_local_head_refuses_a_foreign_observation_before_merging() {
    let (_temp, _launch, worktree, foreign) = foreign_origin();
    let observed = worktree.new_commits_on_origin().unwrap();
    let before = commit(&Git::new(worktree.path()), "Own work after observation");

    let error = worktree.merge_new_commits(observed).unwrap_err();

    assert_eq!(
        error.to_string(),
        "the local head changed since observing origin/issue-7"
    );
    assert_eq!(worktree.head().unwrap(), before);
    assert!(!contains(&worktree, &foreign));
    assert!(!Git::new(worktree.path()).merge_in_progress().unwrap());
}

#[test]
fn spec_catch_up_ignores_shadowing_branch_and_tag_names() {
    for kind in ["branch", "tag"] {
        let (_temp, launch, worktree, foreign) = foreign_origin();
        launch
            .run(&[kind, "origin/issue-7", &worktree.head().unwrap()])
            .unwrap();

        worktree.fast_forward_to_origin().unwrap();

        assert_eq!(worktree.head().unwrap(), foreign);
    }
}

#[test]
fn spec_catch_up_refuses_divergence_without_changing_the_local_head() {
    let (_temp, _launch, worktree, foreign) = foreign_origin();
    let before = commit(&Git::new(worktree.path()), "Divergent own work");

    let error = worktree.fast_forward_to_origin().unwrap_err().to_string();

    assert!(error.contains("Not possible to fast-forward"), "{error}");
    assert_eq!(worktree.head().unwrap(), before);
    assert!(!contains(&worktree, &foreign));
}

#[test]
fn a_foreign_merge_remains_a_merge_when_user_config_demands_fast_forward_only() {
    let (_temp, _launch, worktree, foreign) = foreign_origin();
    let git = Git::new(worktree.path());
    let own = commit(&git, "Divergent own work");
    git.run(&["config", "merge.ff", "only"]).unwrap();
    let observed = worktree.new_commits_on_origin().unwrap();

    assert_eq!(
        clean_commit(worktree.merge_new_commits(observed).unwrap()),
        foreign
    );

    assert_eq!(git.run(&["rev-parse", "HEAD^1"]).unwrap(), own);
    assert_eq!(git.run(&["rev-parse", "HEAD^2"]).unwrap(), foreign);
    assert_eq!(
        git.run(&["log", "-1", "--format=%s"]).unwrap(),
        "Merge remote-tracking branch 'origin/issue-7' into issue-7"
    );
}

#[test]
fn a_completed_conflict_is_validated_against_its_selected_commit_after_origin_advances() {
    for upstream in ["main", "issue-7"] {
        let (_temp, launch, worktree, pending, selected, _shared) = conflicting_merge(upstream);
        let later = commit(&launch, "Origin advances during Repair");
        launch
            .run(&["push", "-q", "origin", &format!("HEAD:{upstream}")])
            .unwrap();
        let git = Git::new(worktree.path());
        std::fs::write(worktree.path().join("conflict.txt"), "ours\ntheirs\n").unwrap();
        git.run(&["add", "conflict.txt"]).unwrap();
        git.run(&["commit", "-q", "--no-edit"]).unwrap();

        worktree.ensure_merged(&pending).unwrap();

        assert!(contains(&worktree, &selected));
        assert!(!contains(&worktree, &later));
        assert_eq!(
            git.run(&["log", "-1", "--format=%s"]).unwrap(),
            format!("Merge remote-tracking branch 'origin/{upstream}' into issue-7")
        );
    }
}

#[test]
fn unfinished_and_aborted_conflicts_fail_validation_even_after_the_ref_rewinds() {
    for upstream in ["main", "issue-7"] {
        let (_temp, launch, worktree, pending, selected, shared) = conflicting_merge(upstream);
        launch
            .run(&[
                "update-ref",
                &format!("refs/remotes/origin/{upstream}"),
                &shared,
            ])
            .unwrap();

        let error = worktree.ensure_merged(&pending).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("the merge of origin/{upstream} is still in progress")
        );

        Git::new(worktree.path())
            .run(&["merge", "--abort"])
            .unwrap();
        let error = worktree.ensure_merged(&pending).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("origin/{upstream} is not merged into issue-7")
        );
        assert!(!contains(&worktree, &selected));
    }
}

#[test]
fn ordinary_merge_errors_are_reported_without_a_pending_conflict() {
    let (_temp, launch, worktree) = fixture();
    std::fs::write(launch.dir().join("new.txt"), "origin\n").unwrap();
    launch.run(&["add", "new.txt"]).unwrap();
    let base = commit(&launch, "Base adds a file");
    launch.run(&["push", "-q", "origin", "main"]).unwrap();
    std::fs::write(worktree.path().join("new.txt"), "untracked local work\n").unwrap();
    let before = worktree.head().unwrap();

    let error = worktree.merge_base_branch("main").unwrap_err().to_string();

    assert!(error.contains("would be overwritten by merge"), "{error}");
    assert_eq!(worktree.head().unwrap(), before);
    assert!(!contains(&worktree, &base));
    assert!(!Git::new(worktree.path()).merge_in_progress().unwrap());
}
