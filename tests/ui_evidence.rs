//! Execute the PR skill's publishing recipe against the shared fake GitHub CLI.
mod support;

use std::fs;
use std::os::unix::fs::{MetadataExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::Scenario;

fn recipe() -> String {
    let read = |path| fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
    let extract = |text: String| {
        text.split("```sh\n")
            .filter_map(|part| part.split_once("```"))
            .map(|(block, _)| block)
            .find(|block| block.contains("gh pr create") && block.contains("--attach"))
            .expect("PR skill needs a GitHub attachment publish recipe")
            .to_owned()
    };
    let factory = extract(read("skills/thirdshift-pr/SKILL.md"));
    assert_eq!(factory, extract(read(".agents/skills/pr/SKILL.md")));
    factory
}

fn publish(scenario: &Scenario, action: &str, captures: &[PathBuf]) -> Output {
    let body = scenario.path("body.md");
    let mut markdown = "## Evidence\n\n".to_owned();
    for (index, path) in captures.iter().enumerate() {
        markdown.push_str(&format!(
            "**After — flow {index}, desktop 1440×900**\n\n![Capture {index}]({})\n\n",
            path.display()
        ));
    }
    fs::write(&body, markdown).unwrap();
    publish_with_body(scenario, action, &body, captures)
}

fn publish_with_body(
    scenario: &Scenario,
    action: &str,
    body: &Path,
    captures: &[PathBuf],
) -> Output {
    let environment = scenario.command(&[]);
    Command::new("bash")
        .args(["-c", &recipe(), "publish", action])
        .arg(body)
        .args(captures)
        .current_dir(scenario.launch_dir())
        .env_clear()
        .envs(
            environment
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .env("BASE_BRANCH", "main")
        .env("ISSUE_BRANCH", "issue-7")
        .env("PR_TITLE", "UI verification")
        .env("PR_URL", "https://github.com/acme/widgets/pull/1")
        .env("EVIDENCE_REDACTED", "yes")
        .output()
        .unwrap()
}

#[test]
fn pr_recipe_refuses_unsafe_captures_before_any_upload() {
    for invalid in [
        "symlink",
        "directory",
        "fifo",
        "unowned",
        "bad name.png",
        "bad.gif",
        "empty.png",
        "oversized.png",
    ] {
        let scenario = Scenario::new();
        let valid = scenario.path("good.png");
        fs::write(&valid, "faux image").unwrap();
        let capture = scenario.path(invalid);
        let expected = match invalid {
            "symlink" => {
                symlink(&valid, &capture).unwrap();
                "not a regular file owned"
            }
            "directory" => {
                fs::create_dir(&capture).unwrap();
                "not a regular file owned"
            }
            "fifo" => {
                assert!(
                    Command::new("mkfifo")
                        .arg(&capture)
                        .status()
                        .unwrap()
                        .success()
                );
                "not a regular file owned"
            }
            "unowned" => "not a regular file owned",
            "bad name.png" => {
                fs::write(&capture, "faux").unwrap();
                "filename"
            }
            "bad.gif" => {
                fs::write(&capture, "faux").unwrap();
                "extension"
            }
            "empty.png" => {
                fs::write(&capture, "").unwrap();
                "empty or oversized"
            }
            "oversized.png" => {
                fs::File::create(&capture)
                    .unwrap()
                    .set_len(8_388_609)
                    .unwrap();
                "empty or oversized"
            }
            _ => unreachable!(),
        };
        let capture = if invalid == "unowned" {
            let path = PathBuf::from("/etc/issue");
            if fs::metadata(&path).unwrap().uid() == fs::metadata(&valid).unwrap().uid() {
                continue;
            }
            path
        } else {
            capture
        };
        let output = publish(&scenario, "create", &[valid, capture]);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {invalid}");
        assert!(error.contains(expected), "{invalid}: {error}");
        assert!(
            scenario.gh_calls_of("pr", "create").is_empty(),
            "uploaded before rejecting {invalid}"
        );
    }
}

#[test]
fn pr_recipe_refuses_capture_paths_containing_hash_alt_delimiter() {
    let scenario = Scenario::new();
    let secret = scenario.path("secret-data");
    fs::write(&secret, "SECRET-MARKER-hash-bypass\n").unwrap();
    let planted_target = scenario.path("secret-target");
    symlink(&secret, &planted_target).unwrap();
    let decoy_dir = scenario.path("secret-target#");
    fs::create_dir(&decoy_dir).unwrap();
    let decoy_capture = decoy_dir.join("after.png");
    fs::write(&decoy_capture, "faux image").unwrap();

    let output = publish(&scenario, "create", &[decoy_capture]);
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "recipe accepted capture path containing '#' delimiter (gh --attach reads symlink target before '#')"
    );
    assert!(error.contains("path"), "{error}");
    assert!(
        scenario.gh_calls_of("pr", "create").is_empty(),
        "uploaded before rejecting '#' in capture path"
    );
}

#[test]
fn pr_recipe_refuses_unsafe_body_file_before_publishing() {
    for invalid in ["symlink", "directory", "fifo", "unowned", "stdin"] {
        let scenario = Scenario::new();
        let valid = scenario.path("good.png");
        fs::write(&valid, "faux image").unwrap();
        let secret = scenario.path("secret-body-target");
        fs::write(&secret, "SECRET-MARKER-body-symlink\n").unwrap();
        let body = match invalid {
            "symlink" => {
                let link = scenario.path("pr-body.md");
                symlink(&secret, &link).unwrap();
                link
            }
            "directory" => {
                let dir = scenario.path("body-dir");
                fs::create_dir(&dir).unwrap();
                dir
            }
            "fifo" => {
                let fifo = scenario.path("body-fifo");
                assert!(
                    Command::new("mkfifo")
                        .arg(&fifo)
                        .status()
                        .unwrap()
                        .success()
                );
                fifo
            }
            "unowned" => {
                let path = PathBuf::from("/etc/issue");
                if fs::metadata(&path).unwrap().uid() == fs::metadata(&valid).unwrap().uid() {
                    continue;
                }
                path
            }
            "stdin" => PathBuf::from("-"),
            _ => unreachable!(),
        };

        let output = publish_with_body(&scenario, "create", &body, &[valid]);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {invalid} body file");
        assert!(error.contains("body"), "{invalid}: {error}");
        assert!(
            scenario.gh_calls_of("pr", "create").is_empty(),
            "published PR with unsafe {invalid} body file"
        );
        let state = scenario.gh_state();
        assert!(
            !state.to_string().contains("SECRET-MARKER-body-symlink"),
            "symlink target was published in PR body"
        );
    }
}

#[test]
fn pr_recipe_refuses_symlinked_ancestor_directories_for_body_and_captures() {
    let scenario = Scenario::new();
    let valid_capture = scenario.path("good.png");
    fs::write(&valid_capture, "faux image").unwrap();
    let safe_body = scenario.path("safe-body.md");
    fs::write(&safe_body, "## Evidence\n").unwrap();

    let secret_dir = scenario.path("secret-dir");
    let inner_dir = secret_dir.join("inner");
    fs::create_dir_all(&inner_dir).unwrap();
    fs::write(
        secret_dir.join("credentials.md"),
        "SECRET-MARKER-body-dir-symlink\n",
    )
    .unwrap();
    fs::write(secret_dir.join("after.png"), "faux secret png").unwrap();

    let decoy_dir = scenario.path("decoy-dir");
    symlink(&secret_dir, &decoy_dir).unwrap();
    let nested = scenario.path("nested");
    fs::create_dir(&nested).unwrap();
    // Place a harmless file in `nested` so syntactic `..` normalization would hide the escape.
    fs::write(nested.join("credentials.md"), "harmless\n").unwrap();
    symlink(&inner_dir, nested.join("hop")).unwrap();

    for (label, body) in [
        ("direct-symlink-dir", decoy_dir.join("credentials.md")),
        (
            "dotdot-after-symlink-dir",
            nested.join("hop").join("..").join("credentials.md"),
        ),
    ] {
        let output = publish_with_body(
            &scenario,
            "create",
            &body,
            std::slice::from_ref(&valid_capture),
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "accepted {label} body path: {}",
            body.display()
        );
        assert!(error.contains("body"), "{label}: {error}");
        assert!(
            scenario.gh_calls_of("pr", "create").is_empty(),
            "published PR with {label} body path"
        );
        assert!(
            !scenario
                .gh_state()
                .to_string()
                .contains("SECRET-MARKER-body-dir-symlink"),
            "{label}: symlinked directory target leaked into PR body"
        );
    }

    for (label, capture) in [
        ("direct-symlink-dir", decoy_dir.join("after.png")),
        (
            "dotdot-after-symlink-dir",
            nested.join("hop").join("..").join("after.png"),
        ),
    ] {
        let output = publish_with_body(&scenario, "create", &safe_body, &[capture]);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "accepted {label} capture path");
        assert!(
            error.contains("not a regular file owned"),
            "{label}: {error}"
        );
        assert!(
            scenario.gh_calls_of("pr", "create").is_empty(),
            "uploaded {label} capture path"
        );
    }
}

#[test]
fn pr_recipe_requires_gh_2_103_2_or_later() {
    for version in ["2.96.0", "2.103.1", "2.103.2", "2.104.0", "3.0.0"] {
        let scenario = Scenario::new();
        let mut state = scenario.gh_state();
        state["version"] = version.into();
        scenario.write_gh_state(&state);
        let path = scenario.path("after.png");
        fs::write(&path, "faux").unwrap();
        let output = publish(&scenario, "create", &[path]);
        let supported = !["2.96.0", "2.103.1"].contains(&version);
        assert_eq!(
            output.status.success(),
            supported,
            "{version}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            scenario.gh_calls_of("pr", "create").len(),
            usize::from(supported)
        );
    }
}

#[test]
fn pr_recipe_publishes_all_png_and_mp4_captures_with_visible_labels() {
    let scenario = Scenario::new();
    let before = tempfile::Builder::new()
        .prefix("before-desktop.")
        .suffix(".png")
        .tempfile_in(scenario.path("home"))
        .unwrap();
    let after = tempfile::Builder::new()
        .prefix("after-desktop.")
        .suffix(".mp4")
        .tempfile_in(scenario.path("home"))
        .unwrap();
    // The documented cap is inclusive: exactly 8 MB is accepted.
    before.as_file().set_len(8_388_608).unwrap();
    fs::write(after.path(), "faux MP4").unwrap();
    let paths = [before.path().to_owned(), after.path().to_owned()];
    for action in ["create", "edit"] {
        let output = publish(&scenario, action, &paths);
        assert!(
            output.status.success(),
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let body = scenario.gh_state()["prs"][0]["body"]
            .as_str()
            .unwrap()
            .to_owned();
        for index in 0..2 {
            assert!(
                body.contains(&format!("**After — flow {index}, desktop 1440×900**\n\n")),
                "{body}"
            );
        }
        assert_eq!(
            body.matches("https://github.com/user-attachments/assets/")
                .count(),
            2,
            "{body}"
        );
        assert!(!body.contains("faux MP4"));
        assert!(
            body.contains("\n\nhttps://github.com/user-attachments/assets/fake-1\n"),
            "video must be a native player: {body}"
        );
    }
    let environment = scenario.command(&[]);
    let output = Command::new("gh")
        .args(["pr", "comment", "1", "--body-file"])
        .arg(scenario.path("body.md"))
        .arg("--attach")
        .arg(&paths[0])
        .arg("--attach")
        .arg(&paths[1])
        .env_clear()
        .envs(
            environment
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .current_dir(scenario.launch_dir())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let state = scenario.gh_state();
    let comment = state["prs"][0]["postedComments"][0].as_str().unwrap();
    assert_eq!(
        comment
            .matches("https://github.com/user-attachments/assets/")
            .count(),
        2
    );
}

#[test]
fn day_shift_ui_verification_skills_are_mirrored_for_claude_and_free_of_factory_references() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for skill in [
        "verify-ui",
        "create-verification-skill",
        "maintain-verification-skill",
        "verify-console",
        "verify-site",
    ] {
        let claude_link = root.join(".claude/skills").join(skill);
        let target = fs::read_link(&claude_link)
            .unwrap_or_else(|error| panic!("{}: {error}", claude_link.display()));
        assert_eq!(
            target,
            PathBuf::from(format!("../../.agents/skills/{skill}")),
            "{}",
            claude_link.display()
        );
        assert!(
            claude_link.join("SKILL.md").is_file(),
            "broken link: {}",
            claude_link.display()
        );
    }

    for path in [
        "skills/thirdshift-pr/SKILL.md",
        "skills/thirdshift-verify-ui/SKILL.md",
    ] {
        let text = fs::read_to_string(root.join(path)).unwrap();
        assert!(!text.contains("Day shift:"), "{path} leaks Day shift name");
    }
    for path in [
        ".agents/skills/pr/SKILL.md",
        ".agents/skills/verify-ui/SKILL.md",
    ] {
        let text = fs::read_to_string(root.join(path)).unwrap();
        for factory_only in [
            "thirdshift-verify-ui",
            "Built-in processes",
            "Experiment Arm",
            "wastes a Resume",
            "for the PR skill",
        ] {
            assert!(
                !text.contains(factory_only),
                "{path} retains Factory text: {factory_only}"
            );
        }
    }
}
