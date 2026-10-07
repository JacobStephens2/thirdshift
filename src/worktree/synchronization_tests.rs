//! Synchronization through Worktree, with real local Git and a bare origin.

use super::*;

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
