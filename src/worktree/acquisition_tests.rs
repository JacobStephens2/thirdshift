//! Acquisition through Worktree, with real local Git and origin.

use super::*;

const BRANCH: &str = "issue-7";

fn continuation() -> (tempfile::TempDir, Git, String) {
    let (temp, launch) = super::tests::launch_directory();
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
