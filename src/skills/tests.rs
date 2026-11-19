//! Factory skill linking through its public worktree boundary.

use super::*;
use crate::harness::Harness;
use crate::test_support::scratch_worktree;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn ui_verification_is_available_to_every_harness_and_standalone_skill_sessions() {
    let _skills = testing::FactorySkills::hold();
    for harness in Harness::ALL {
        let worktree = scratch_worktree();
        let adapter = harness.adapter();
        link_into(worktree.path(), adapter).unwrap();
        let skill = find(worktree.path(), "verify-ui").unwrap();
        assert_eq!(skill.name, "thirdshift-verify-ui");
        assert_eq!(
            fs::read(
                worktree
                    .path()
                    .join(adapter.project_skills())
                    .join("thirdshift-verify-ui/SKILL.md")
            )
            .unwrap(),
            fs::read(skill.path).unwrap(),
            "{} must receive the standalone Factory skill",
            harness.name()
        );
    }
}

#[test]
fn code_review_reports_count_only_actionable_findings() {
    let _skills = testing::FactorySkills::hold();
    let worktree = scratch_worktree();
    let adapter = Harness::Codex.adapter();
    link_into(worktree.path(), adapter).unwrap();
    let skill = fs::read_to_string(
        worktree
            .path()
            .join(adapter.project_skills())
            .join("thirdshift-code-review/SKILL.md"),
    )
    .unwrap();
    for rule in [
        "Report only actionable findings that warrant a change or human follow-up",
        "Omit declined, rejected or accepted-as-is observations from the findings list",
        "nonnegative integer actionable finding count",
        "`0` when no change or human follow-up is needed",
        "Exclude declined or accepted observations that require no change",
        // #2231: a review that outlives its session's turn is stopped partway.
        "never end your turn while one is still running",
        "Start both sub-agents in the foreground",
    ] {
        assert!(skill.contains(rule), "missing {rule:?} in Factory skill");
    }
    assert!(!skill.contains("including findings later fixed or declined"));
}

fn skills_prefix(adapter: &dyn Adapter, depth: usize) -> PathBuf {
    Path::new(adapter.project_skills())
        .components()
        .take(depth)
        .collect()
}

#[test]
fn overlapping_test_users_share_skills_until_the_last_finishes() {
    const CHILD: &str = "THIRDSHIFT_SKILLS_LIFETIME_TEST";
    if std::env::var_os(CHILD).is_none() {
        let tmp = tempfile::tempdir().unwrap();
        crate::test_support::assert_test_without_warnings(
            "skills::tests::overlapping_test_users_share_skills_until_the_last_finishes",
            |command| {
                command.env(CHILD, "1").env("TMPDIR", tmp.path());
            },
        );
        return;
    }

    let first = testing::FactorySkills::hold();
    let worktree = scratch_worktree();
    let adapter = Harness::Codex.adapter();
    link_into(worktree.path(), adapter).unwrap();
    let skill = Path::new(adapter.project_skills()).join("thirdshift-implement");
    let target = fs::read_link(worktree.path().join(&skill)).unwrap();

    let (ready, started) = std::sync::mpsc::channel();
    let (finish, finishing) = std::sync::mpsc::channel();
    let second = std::thread::spawn(move || {
        let _skills = testing::FactorySkills::hold();
        let worktree = scratch_worktree();
        link_into(worktree.path(), adapter).unwrap();
        ready
            .send(fs::read_link(worktree.path().join(&skill)).unwrap())
            .unwrap();
        finishing.recv().unwrap();
        assert!(fs::read_to_string(worktree.path().join(skill).join("SKILL.md")).is_ok());
    });
    let shared = started
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    // Check readability while the second user still holds the extraction.
    drop(first);
    let retained = fs::read_to_string(target.join("SKILL.md"));
    finish.send(()).unwrap();
    second.join().unwrap();
    assert_eq!(
        shared, target,
        "overlapping tests must share one extraction"
    );
    assert!(retained.is_ok(), "the second test still needed the skills");
    assert!(!target.exists(), "the last test must remove the extraction");
}

#[test]
fn refuses_non_directory_skill_paths_without_changing_any_entries() {
    for harness in Harness::ALL {
        let adapter = harness.adapter();
        for depth in [1, 2] {
            for obstruction in ["outside link", "inside link", "dangling link", "file"] {
                let worktree = scratch_worktree();
                let outside = tempfile::tempdir().unwrap();
                let inside = worktree.path().join("ordinary");
                fs::create_dir(&inside).unwrap();
                for target in [outside.path(), inside.as_path()] {
                    fs::create_dir(target.join("skills")).unwrap();
                    fs::write(target.join("thirdshift-implement"), "keep skill").unwrap();
                    fs::write(
                        target.join("skills/thirdshift-implement"),
                        "keep nested skill",
                    )
                    .unwrap();
                }
                let blocked = worktree.path().join(skills_prefix(adapter, depth));
                fs::create_dir_all(blocked.parent().unwrap()).unwrap();
                match obstruction {
                    "outside link" => symlink(outside.path(), &blocked).unwrap(),
                    "inside link" => symlink(&inside, &blocked).unwrap(),
                    "dangling link" => symlink(outside.path().join("absent"), &blocked).unwrap(),
                    "file" => fs::write(&blocked, "keep obstruction").unwrap(),
                    _ => unreachable!(),
                }
                let before = fs::read(worktree.path().join(".git/info/exclude")).unwrap();
                let error = link_into(worktree.path(), adapter).expect_err(&format!(
                    "{} must refuse {obstruction} at {}",
                    harness.name(),
                    blocked.display()
                ));
                assert!(format!("{error:#}").contains(&blocked.display().to_string()));
                assert_eq!(
                    fs::read(worktree.path().join(".git/info/exclude")).unwrap(),
                    before
                );
                for target in [outside.path(), inside.as_path()] {
                    assert_eq!(
                        fs::read_to_string(target.join("thirdshift-implement")).unwrap(),
                        "keep skill"
                    );
                    assert_eq!(
                        fs::read_to_string(target.join("skills/thirdshift-implement")).unwrap(),
                        "keep nested skill"
                    );
                    assert_eq!(fs::read_dir(target).unwrap().count(), 2);
                    assert_eq!(fs::read_dir(target.join("skills")).unwrap().count(), 1);
                }
                if obstruction == "file" {
                    assert_eq!(fs::read_to_string(&blocked).unwrap(), "keep obstruction");
                } else {
                    assert!(
                        fs::symlink_metadata(&blocked)
                            .unwrap()
                            .file_type()
                            .is_symlink()
                    );
                }
            }
        }
    }
}

#[test]
fn links_factory_skills_in_existing_and_missing_directories_for_every_adapter() {
    let _skills = testing::FactorySkills::hold();
    for harness in Harness::ALL {
        let adapter = harness.adapter();
        for existing_depth in [0, 1, 2] {
            let worktree = scratch_worktree();
            let existing = worktree.path().join(skills_prefix(adapter, existing_depth));
            fs::create_dir_all(existing).unwrap();
            let dir = worktree.path().join(adapter.project_skills());
            let outside = tempfile::tempdir().unwrap();
            fs::write(outside.path().join("original"), "keep target").unwrap();
            if existing_depth == 2 {
                fs::write(dir.join("thirdshift-implement"), "replace entry").unwrap();
                symlink(outside.path().join("original"), dir.join("thirdshift-tdd")).unwrap();
                fs::write(dir.join("repository-skill"), "keep repository skill").unwrap();
            }

            link_into(worktree.path(), adapter).unwrap();
            // Repeated session starts also replace existing Factory links.
            link_into(worktree.path(), adapter).unwrap();
            for skill in SKILLS.dirs() {
                let link = dir.join(skill.path());
                assert!(
                    fs::symlink_metadata(&link)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                assert!(
                    fs::read_to_string(link.join("SKILL.md"))
                        .unwrap()
                        .contains("thirdshift-")
                );
            }
            assert_eq!(
                fs::read_to_string(outside.path().join("original")).unwrap(),
                "keep target"
            );
            if existing_depth == 2 {
                assert_eq!(
                    fs::read_to_string(dir.join("repository-skill")).unwrap(),
                    "keep repository skill"
                );
            }
        }
    }
}

#[test]
fn links_in_ordinary_directories_without_directory_read_permission() {
    let _skills = testing::FactorySkills::hold();
    for harness in Harness::ALL {
        let adapter = harness.adapter();
        for restricted_depth in [0, 1, 2] {
            let worktree = scratch_worktree();
            let dir = worktree.path().join(adapter.project_skills());
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("thirdshift-implement"), "replace entry").unwrap();
            let restricted = worktree
                .path()
                .join(skills_prefix(adapter, restricted_depth));
            fs::set_permissions(&restricted, fs::Permissions::from_mode(0o300)).unwrap();
            let result = link_into(worktree.path(), adapter)
                .and_then(|()| link_into(worktree.path(), adapter));
            // Restore listing permission even on failure so the fixture cleans up.
            fs::set_permissions(&restricted, fs::Permissions::from_mode(0o700)).unwrap();
            result.unwrap();
            assert!(
                fs::read_to_string(dir.join("thirdshift-implement/SKILL.md"))
                    .unwrap()
                    .contains("thirdshift-implement")
            );
        }
    }
}

#[test]
fn replacement_and_new_links_use_the_checked_directory_after_its_path_changes() {
    for harness in Harness::ALL {
        let worktree = scratch_worktree();
        let relative = Path::new(harness.adapter().project_skills());
        let dir = worktree.path().join(relative);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("thirdshift-implement"), "replace checked entry").unwrap();
        let checked = CheckedDirectory::check(worktree.path(), relative)
            .unwrap()
            .create_missing()
            .unwrap();
        let retained = worktree.path().join("checked-skills");
        fs::rename(&dir, &retained).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(
            outside.path().join("thirdshift-implement"),
            "keep outside entry",
        )
        .unwrap();
        symlink(outside.path(), &dir).unwrap();

        let target = worktree.path().join("target");
        fs::create_dir(&target).unwrap();
        for name in ["thirdshift-implement", "thirdshift-tdd"] {
            checked.link(&target, Path::new(name)).unwrap();
            assert_eq!(fs::read_link(retained.join(name)).unwrap(), target);
        }
        assert_eq!(
            fs::read_to_string(outside.path().join("thirdshift-implement")).unwrap(),
            "keep outside entry"
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
    }
}
