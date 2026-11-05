//! Factory skill linking through its public worktree boundary.

use super::*;
use crate::harness::Harness;
use crate::test_support::scratch_worktree;
use std::os::unix::fs::{PermissionsExt, symlink};

fn skills_prefix(adapter: &dyn Adapter, depth: usize) -> PathBuf {
    Path::new(adapter.project_skills())
        .components()
        .take(depth)
        .collect()
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

/// The publish recipe must not upload a symlink target. Literal `/tmp`
/// capture names are rewritten into this fixture so the proof does not use
/// the shared predictable paths.
#[test]
fn pr_evidence_publish_recipe_does_not_upload_symlink_targets() {
    let factory = publish_recipe("skills/thirdshift-pr/SKILL.md");
    let day_shift = publish_recipe(".agents/skills/pr/SKILL.md");
    assert_eq!(
        factory, day_shift,
        "factory and Day shift publish recipes diverged"
    );
    let fixture = tempfile::tempdir().unwrap();
    let secret = fixture.path().join("secret-token");
    fs::write(&secret, b"SECRET-MARKER-pr-evidence\n").unwrap();
    let planted = fixture.path().join("before-desktop.png");
    symlink(&secret, &planted).unwrap();
    let after = fixture.path().join("after-desktop.png");
    fs::write(&after, b"AFTER-REGULAR-ok\n").unwrap();
    let published = fixture.path().join("published-symlink");
    fs::create_dir(&published).unwrap();
    let output = run_publish_recipe(&factory, &planted, &after, &published);
    let dumped = format!(
        "status {}\nstdout {}\nstderr {}\npublished {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        published_text(&published)
    );
    assert!(
        !published_text(&published).contains("SECRET-MARKER-pr-evidence"),
        "symlink target was published\n{dumped}"
    );
    assert!(
        !output.status.success(),
        "recipe accepted a symlink capture\n{dumped}"
    );
    assert_eq!(
        fs::read(&secret).unwrap(),
        b"SECRET-MARKER-pr-evidence\n",
        "capture path was written through"
    );

    let ok = tempfile::tempdir().unwrap();
    let before = ok.path().join("before-desktop.A1b2C3.png");
    let after = ok.path().join("after-desktop.A1b2C3.png");
    fs::write(&before, b"BEFORE-REGULAR-ok\n").unwrap();
    fs::write(&after, b"AFTER-REGULAR-ok\n").unwrap();
    let published = ok.path().join("published-regular");
    fs::create_dir(&published).unwrap();
    let output = run_publish_recipe(&factory, &before, &after, &published);
    let text = published_text(&published);
    assert!(
        output.status.success(),
        "regular captures were refused\nstatus {}\nstdout {}\nstderr {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("BEFORE-REGULAR-ok"), "{text}");
    assert!(text.contains("AFTER-REGULAR-ok"), "{text}");
    assert!(!text.contains("SECRET-MARKER-pr-evidence"), "{text}");

    for relative in [
        "skills/thirdshift-implement/SKILL.md",
        "skills/thirdshift-diagnosing-bugs/SKILL.md",
        ".agents/skills/implement/SKILL.md",
        ".agents/skills/diagnosing-bugs/SKILL.md",
    ] {
        let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
            .unwrap_or_else(|error| panic!("{relative}: {error}"));
        assert!(
            text.contains("mktemp"),
            "{relative} does not send captures through mktemp"
        );
    }
}

fn publish_recipe(relative: &str) -> String {
    let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative))
        .unwrap_or_else(|error| panic!("{relative}: {error}"));
    let mut lines = text.lines();
    let mut found = None;
    while let Some(line) = lines.next() {
        if line != "```sh" {
            continue;
        }
        let mut block = String::new();
        for line in lines.by_ref() {
            if line == "```" {
                break;
            }
            block.push_str(line);
            block.push('\n');
        }
        if block.contains("gh gist create") {
            found = Some(block);
        }
    }
    found.unwrap_or_else(|| panic!("{relative} has no Gist publish recipe"))
}

fn run_publish_recipe(
    recipe: &str,
    before: &Path,
    after: &Path,
    publish: &Path,
) -> std::process::Output {
    let bin = tempfile::tempdir().unwrap();
    let gitconfig = bin.path().join("gitconfig");
    fs::write(&gitconfig, "[user]\nname = PoC\nemail = poc@example.com\n").unwrap();
    fs::write(bin.path().join("git"), GIT_MOCK).unwrap();
    fs::write(bin.path().join("gh"), GH_MOCK).unwrap();
    for name in ["git", "gh"] {
        let path = bin.path().join(name);
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
    }
    let script = recipe
        .replace("/tmp/before-desktop.png", before.to_str().unwrap())
        .replace("/tmp/after-desktop.png", after.to_str().unwrap());
    std::process::Command::new("bash")
        .arg("-c")
        .arg(&script)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.path().display()))
        .env("HOME", bin.path())
        .env("PUBLISH_DIR", publish)
        .env("BEFORE_DESKTOP", before)
        .env("AFTER_DESKTOP", after)
        .env("GIT_CONFIG_GLOBAL", &gitconfig)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "PoC")
        .env("GIT_AUTHOR_EMAIL", "poc@example.com")
        .env("GIT_COMMITTER_NAME", "PoC")
        .env("GIT_COMMITTER_EMAIL", "poc@example.com")
        .output()
        .unwrap()
}

fn published_text(dir: &Path) -> String {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    names.sort();
    let mut text = String::new();
    for path in names {
        if path.is_file() {
            text.push_str(&fs::read_to_string(&path).unwrap_or_default());
        }
    }
    text
}

const GIT_MOCK: &str = r#"#!/bin/bash
real=/usr/bin/git
args=("$@")
cmd=""
c_dir=""
i=0
while [ "$i" -lt "${#args[@]}" ]; do
  a=${args[$i]}
  case "$a" in
    -C)
      i=$((i + 1))
      c_dir=${args[$i]}
      ;;
    -c)
      i=$((i + 1))
      ;;
    -*)
      ;;
    *)
      cmd=$a
      break
      ;;
  esac
  i=$((i + 1))
done
if [ "$cmd" = "clone" ]; then
  dest=${args[$((i + 2))]}
  mkdir -p -- "$dest"
  "$real" init -q "$dest"
  exit 0
fi
if [ "$cmd" = "push" ]; then
  root=${c_dir:-.}
  mkdir -p -- "$PUBLISH_DIR"
  find "$root" -mindepth 1 -maxdepth 1 -type f -exec cp -P -- {} "$PUBLISH_DIR/" \;
  printf 'pushed\n' > "$PUBLISH_DIR/PUSHED"
  exit 0
fi
exec "$real" "$@"
"#;

const GH_MOCK: &str = r#"#!/bin/bash
case "$1" in
  gist) printf '%s\n' 'https://gist.github.com/0123456789abcdef0123456789abcdef' ;;
  api) printf '%s\n' 'poc-owner' ;;
  *) printf 'unexpected gh\n' >&2; exit 1 ;;
esac
"#;
