//! Skill and Review checkout allocation, each run in a child with its own temp root.

use std::fs;
use std::path::Path;

use super::*;

const CHILD: &str = "THIRDSHIFT_SKILL_ALLOCATION_TEST";
const MISSING_COMMIT: &str = "1234567890123456789012345678901234567890";

/// Run the calling test again in a child whose temp root holds only its
/// fixtures, so allocation leftovers can be counted.
fn ran_in_isolated_child(configure: impl FnOnce(&Path, &mut std::process::Command)) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return false;
    }
    let root = tempfile::tempdir().unwrap();
    let name = std::thread::current().name().unwrap().to_string();
    // The launch fixture's empty clone warns, so judge only the child's status.
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", &name, "--nocapture"])
        .env(CHILD, "1")
        .env("TMPDIR", root.path());
    configure(root.path(), &mut command);
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

fn allocations() -> Vec<String> {
    fs::read_dir(std::env::temp_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.starts_with("thirdshift-review-") || name.starts_with("thirdshift-skill-")
        })
        .collect()
}

#[test]
fn a_failed_skill_checkout_leaves_no_allocation_behind() {
    if ran_in_isolated_child(|_, _| {}) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();

    assert!(SkillWorktree::at(&launch, MISSING_COMMIT).is_err());

    assert_eq!(allocations(), Vec::<String>::new());
}

#[test]
fn a_failed_review_checkout_leaves_no_allocation_behind() {
    if ran_in_isolated_child(|_, _| {}) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();

    assert!(SkillWorktree::on_pull_request(&launch, "feature", MISSING_COMMIT).is_err());

    assert_eq!(allocations(), Vec::<String>::new());
    assert_eq!(local_head(&launch, "feature").unwrap(), None);
}

#[test]
fn a_branch_occupied_after_review_preflight_keeps_the_occupier_and_no_allocation() {
    if ran_in_isolated_child(|root, command| {
        // Occupy the branch once, between preflight and Git's add.
        let real = String::from_utf8(
            std::process::Command::new("sh")
                .args(["-c", "command -v git"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        crate::test_support::write_executable(
            &bin.join("git"),
            &format!(
                "#!/bin/sh\nif [ \"$1 $2 $3\" = 'worktree add -b' ] && [ ! -e '{root}/occupier' ]; then\n  '{real}' worktree add -q -b \"$4\" '{root}/occupier' \"$6\" || exit 97\nfi\nexec '{real}' \"$@\"\n",
                root = root.display(),
                real = real.trim(),
            ),
        );
        let path = std::env::join_paths(
            std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        command.env("PATH", path);
    }) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let start = launch.run(&["rev-parse", "HEAD"]).unwrap();

    let Err(error) = SkillWorktree::on_pull_request(&launch, "feature", &start) else {
        panic!("a branch occupied after preflight was acquired");
    };

    assert!(format!("{error:#}").contains("feature"), "{error:#}");
    assert_eq!(allocations(), Vec::<String>::new());
    let occupier = std::env::temp_dir().join("occupier");
    assert_eq!(
        Git::new(&occupier).run(&["rev-parse", "HEAD"]).unwrap(),
        start
    );
    assert_eq!(local_head(&launch, "feature").unwrap(), Some(start));
}

#[test]
fn a_branch_occupied_before_review_is_refused_before_allocation() {
    if ran_in_isolated_child(|_, _| {}) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let start = launch.run(&["rev-parse", "HEAD"]).unwrap();
    let occupier = std::env::temp_dir().join("occupier");
    launch
        .run(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            occupier.to_str().unwrap(),
        ])
        .unwrap();

    let Err(error) = SkillWorktree::on_pull_request(&launch, "feature", &start) else {
        panic!("an occupied branch was acquired");
    };

    assert!(
        format!("{error:#}").contains("Occupied checkout"),
        "{error:#}"
    );
    assert_eq!(allocations(), Vec::<String>::new());
    assert_eq!(local_head(&launch, "feature").unwrap(), Some(start));
}

#[test]
fn hook_work_from_a_failed_skill_or_review_checkout_survives() {
    if ran_in_isolated_child(|_, _| {}) {
        return;
    }
    let (temp, launch) = super::tests::launch_directory();
    let start = launch.run(&["rev-parse", "HEAD"]).unwrap();
    crate::test_support::write_executable(
        &temp.path().join("hooks/post-checkout"),
        "#!/bin/sh\necho hook work > hook.txt\nexit 1\n",
    );

    assert!(SkillWorktree::at(&launch, &start).is_err());
    assert!(SkillWorktree::on_pull_request(&launch, "feature", &start).is_err());

    let mut retained = allocations();
    retained.sort_by_key(|name| name.starts_with("thirdshift-review-"));
    assert!(
        retained.len() == 2
            && retained[0].starts_with("thirdshift-skill-")
            && retained[1].starts_with("thirdshift-review-"),
        "{retained:?}"
    );
    for name in retained {
        let path = std::env::temp_dir().join(name);
        assert_eq!(
            fs::read_to_string(path.join("hook.txt")).unwrap(),
            "hook work\n"
        );
    }
    assert_eq!(local_head(&launch, "feature").unwrap(), Some(start));
}

#[test]
fn successful_skill_and_review_checkouts_are_cleaned_up() {
    if ran_in_isolated_child(|_, _| {}) {
        return;
    }
    let (_temp, launch) = super::tests::launch_directory();
    let start = launch.run(&["rev-parse", "HEAD"]).unwrap();

    drop(SkillWorktree::at(&launch, &start).unwrap());
    drop(SkillWorktree::on_pull_request(&launch, "feature", &start).unwrap());

    assert_eq!(allocations(), Vec::<String>::new());
}
