//! Factory skill linking through its public worktree boundary.

use super::*;
use crate::harness::Harness;
use std::os::unix::fs::symlink;
use std::process::Command;

fn worktree() -> TempDir {
    let worktree = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(worktree.path())
            .status()
            .unwrap()
            .success()
    );
    worktree
}

#[test]
fn refuses_non_directory_skill_paths_without_changing_any_entries() {
    for harness in Harness::ALL {
        let adapter = harness.adapter();
        for depth in [1, 2] {
            for obstruction in ["outside link", "inside link", "dangling link", "file"] {
                let worktree = worktree();
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
                let blocked = worktree.path().join(
                    Path::new(adapter.project_skills())
                        .components()
                        .take(depth)
                        .collect::<PathBuf>(),
                );
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
    for harness in Harness::ALL {
        let adapter = harness.adapter();
        for existing_depth in [0, 1, 2] {
            let worktree = worktree();
            let existing = worktree.path().join(
                Path::new(adapter.project_skills())
                    .components()
                    .take(existing_depth)
                    .collect::<PathBuf>(),
            );
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
fn replacement_and_new_links_use_the_checked_directory_after_its_path_changes() {
    for harness in Harness::ALL {
        let worktree = worktree();
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
