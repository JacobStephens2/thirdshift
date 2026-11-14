//! The installed binary's Release run against a real bare origin and fake `gh` and Harnesses.
#[test]
fn native_pass_quietly_skips_human_blocked_superseded_cuts() {
    for blocker in ["draft", "foreign"] {
        let release = Release::new();
        release.push_bump("0.2.0");
        release.open_bump_pr("0.2.0");
        release.push_bump("1.0.0");
        let contributor = release.contributor();
        git(&contributor, &["checkout", "-q", "release-1.0.0"]);
        git(
            &contributor,
            &[
                "commit",
                "--amend",
                "-q",
                "-m",
                "Release 1.0.0\n\nThirdshift-Supersedes: 0.2.0",
            ],
        );
        git(
            &contributor,
            &["push", "-q", "--force", "origin", "release-1.0.0"],
        );
        if blocker == "draft" {
            release.gh(&["fake", "pr", "release-0.2.0", "isDraft", "true"]);
        } else {
            release.push_change_to_bump("0.2.0", BumpChange::AddedFile);
        }
        let before = release.origin_refs();
        for _ in 0..2 {
            let output = with_input(&mut command(&release, ""), "");
            assert!(output.status.success(), "{blocker}: {}", stderr(&output));
            assert!(output.stdout.is_empty(), "{blocker}: {}", stdout(&output));
            assert!(output.stderr.is_empty(), "{blocker}: {}", stderr(&output));
        }
        assert_eq!(release.origin_refs(), before);
        let activity = fs::read_to_string(
            release
                .root()
                .join("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
        )
        .unwrap();
        assert_eq!(
            activity.matches("Release run skipped:").count(),
            1,
            "{activity}"
        );
        assert!(activity.contains("0.2.0"), "{activity}");
    }
}

#[test]
fn a_forged_supersedes_trailer_cannot_drop_another_unfinished_release() {
    // Branch pushers cannot create v* tags. A Release pass must not treat a
    // commit message as authority to abandon a clean unfinished release and
    // publish the planted one.
    let release = Release::new();
    release.ci_reports(GREEN);
    let kept = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.push_bump("0.3.0");
    let contributor = release.contributor();
    git(&contributor, &["checkout", "-q", "release-0.3.0"]);
    git(
        &contributor,
        &[
            "commit",
            "--amend",
            "-q",
            "-m",
            "Release 0.3.0\n\nThirdshift-Supersedes: 0.2.0",
        ],
    );
    git(
        &contributor,
        &["push", "-q", "--force", "origin", "release-0.3.0"],
    );
    let before = release.origin_refs();

    let output = with_input(&mut command(&release, ""), "");

    assert!(
        release.origin_tag("v0.3.0").is_none() && release.origin_tag("v0.2.0").is_none(),
        "forged trailer published a release: {}",
        stderr(&output)
    );
    assert_eq!(
        release.pr("release-0.2.0")["state"],
        "OPEN",
        "forged trailer closed the in-progress release: {}",
        stderr(&output)
    );
    assert_eq!(
        release.origin(&["rev-parse", "refs/heads/release-0.2.0"]),
        kept
    );
    assert_eq!(release.origin_refs(), before, "{}", stderr(&output));
}

use super::*;

pub(super) fn command(release: &Release, version: &str) -> Command {
    let launch = release.maintainer();
    git(
        &launch,
        &[
            "config",
            "remote.origin.url",
            "https://github.com/JacobStephens2/thirdshift.git",
        ],
    );
    git(
        &launch,
        &[
            "config",
            &format!(
                "url.{}.insteadOf",
                release.root().join("origin.git").display()
            ),
            "https://github.com/JacobStephens2/thirdshift.git",
        ],
    );
    fs::create_dir_all(release.root().join("home/.thirdshift")).unwrap();
    let config = release.root().join("home/.thirdshift/config.toml");
    if !config.exists() {
        fs::write(config, "[harness]\ndefault = 'claude'\n").unwrap();
    }
    let mut command = release.command(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &launch);
    command.arg("release");
    if !version.is_empty() {
        command.arg(version);
    }
    command
        .env_remove("THIRDSHIFT_SESSION")
        .env_remove("THIRDSHIFT_PARENT_RECORD")
        .env("THIRDSHIFT_POLL_MS", "1")
        .env("THIRDSHIFT_CI_GRACE_MS", "5000");
    command
}

#[test]
fn release_review_requires_a_terminal_before_setup_or_git() {
    let release = Release::new();
    let output = with_input(
        command(&release, "0.2.0")
            .arg("--review")
            .stdin(std::process::Stdio::null()),
        "",
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--review requires a terminal"),
        "{}",
        stderr(&output)
    );
    assert!(release.origin_tag("v0.2.0").is_none());
}

#[test]
fn native_resume_adopts_exact_bump_branches_and_open_prs() {
    for open in [false, true] {
        let release = Release::new();
        let head = release.push_bump("0.2.0");
        if open {
            release.open_bump_pr("0.2.0");
        }
        release.ci_reports(GREEN);
        let output = with_input(&mut command(&release, "0.2.0"), "");
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(release.origin_tag("v0.2.0"), Some(head));
        assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
        assert_eq!(release.prs_from("release-0.2.0").len(), 1);
    }
}

#[test]
fn native_pass_skips_human_blocked_releases_and_direct_mode_names_recovery() {
    for blocker in ["draft", "foreign", "closed"] {
        let release = Release::new();
        release.push_bump("0.2.0");
        release.open_bump_pr("0.2.0");
        if blocker == "foreign" {
            release.push_change_to_bump("0.2.0", BumpChange::AddedFile);
        } else if blocker == "draft" {
            release.gh(&["fake", "pr", "release-0.2.0", "isDraft", "true"]);
        } else {
            release.gh(&["fake", "pr", "release-0.2.0", "state", "\"CLOSED\""]);
        }
        let before = release.origin_refs();
        for _ in 0..2 {
            let output = with_input(&mut command(&release, ""), "");
            assert!(output.status.success(), "{}", stderr(&output));
            assert!(output.stdout.is_empty());
            assert!(output.stderr.is_empty(), "{}", stderr(&output));
        }
        let activity = fs::read_to_string(
            release
                .root()
                .join("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
        )
        .unwrap();
        assert_eq!(
            activity.matches("Release run skipped:").count(),
            1,
            "{activity}"
        );
        assert!(activity.contains("0.2.0"), "{activity}");
        let output = with_input(&mut command(&release, "0.2.0"), "");
        assert!(!output.status.success());
        assert!(stderr(&output).contains("0.2.0"), "{}", stderr(&output));
        assert_eq!(release.origin_refs(), before);
    }
}

#[test]
fn native_direct_mode_refuses_a_different_unfinished_release() {
    let release = Release::new();
    release.push_bump("0.2.0");
    let output = with_input(&mut command(&release, "0.3.0"), "");
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("thirdshift release 0.2.0"),
        "{}",
        stderr(&output)
    );
    assert!(release.origin_tag("v0.3.0").is_none());
}

#[test]
fn native_with_experiment_uses_the_engine_without_cargo_on_the_launch_directory() {
    let release = Release::new();
    release.ci_reports(GREEN);
    write_executable(
        &release.bin().join("cargo"),
        "#!/bin/sh\necho 'cargo must not be run' >&2\nexit 93\n",
    );
    let output = with_input(command(&release, "0.2.0").arg("--with-experiment"), "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("no Eval suite touched"),
        "{}",
        stderr(&output)
    );
    let pin = git(
        &release.maintainer(),
        &["rev-parse", "refs/thirdshift/release-candidates/0.2.0"],
    );
    assert_eq!(pin, release.origin(&["rev-parse", "release-0.2.0^"]));
}

#[test]
fn native_failed_unpublished_release_recuts_and_published_tags_never_move() {
    // A draft GitHub Release is unpublished: its tag may still be recut.
    for (published, draft) in [(false, false), (false, true), (true, false)] {
        let release = Release::new();
        release.ci_reports(GREEN);
        let mut failed = failed_release();
        failed["published"] = json!(published);
        failed["draft"] = json!(draft);
        release.release_workflow(failed);
        let output = with_input(&mut command(&release, "0.2.0"), "");
        assert!(!output.status.success(), "{}", stderr(&output));
        let old = release.origin_tag("v0.2.0").unwrap();
        release.merge_pr("newer", "New content after failed workflow", "");
        let base = release.origin(&["rev-parse", "main"]);
        release.main_ci("completed", "success");
        release.release_workflow(json!({}));
        release.ci_reports(GREEN);
        let output = with_input(&mut command(&release, "0.2.0"), "");
        assert!(output.status.success(), "{}", stderr(&output));
        let head = release.origin_tag("v0.2.0").unwrap();
        if published {
            assert_eq!(head, old);
        } else {
            assert_ne!(head, old);
            assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), base);
        }
        assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    }
}

#[test]
fn native_recovery_honors_a_higher_requested_bump() {
    let release = Release::new();
    release.ci_reports(GREEN);
    release.release_workflow(failed_release());
    let output = with_input(&mut command(&release, "0.2.0"), "");
    assert!(!output.status.success(), "{}", stderr(&output));
    release.merge_pr("newer", "New content for recut", "");
    let base = release.origin(&["rev-parse", "main"]);
    release.main_ci("completed", "success");
    release.release_workflow(json!({}));
    release.ci_reports(GREEN);
    let output = with_input(command(&release, "").arg("--major"), "");
    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.origin_tag("v1.0.0").unwrap();
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), base);
    assert_eq!(release.pr("release-1.0.0")["state"], "MERGED");
    assert_eq!(release.pr("release-0.2.0")["state"], "CLOSED");
}

#[test]
fn native_publication_outage_skips_a_pass_and_refuses_direct_recovery() {
    let release = Release::new();
    release.push_bump("0.2.0");
    release.gh(&["fake", "fails", "release view"]);
    let before = release.origin_refs();
    let output = with_input(&mut command(&release, ""), "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
    let output = with_input(&mut command(&release, "0.2.0"), "");
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot verify GitHub Release publication"),
        "{}",
        stderr(&output)
    );
    assert_eq!(release.origin_refs(), before);
}

#[test]
fn native_review_accepts_edits_or_cancels_on_a_terminal() {
    for answer in ["y\n", "e\n", "n\n"] {
        let release = Release::new();
        release.ci_reports(GREEN);
        sandbox_summary(&release);
        let editor = release.bin().join("release-editor");
        write_executable(
            &editor,
            "#!/bin/sh\nprintf 'Edited release summary' > \"$1\"\n",
        );
        let output = release.run_on_terminal(
            &["--review", "0.2.0"],
            answer,
            &[("EDITOR", editor.to_str().unwrap())],
        );
        assert_eq!(
            output.status.success(),
            answer != "n\n",
            "{}\n{}",
            stdout(&output),
            stderr(&output)
        );
        if answer == "n\n" {
            assert!(!release.origin_branch("release-0.2.0"));
            assert!(release.origin_tag("v0.2.0").is_none());
        } else {
            assert!(
                release
                    .pr_body("release-0.2.0")
                    .contains(if answer == "e\n" {
                        "Edited release summary"
                    } else {
                        "A sandboxed release summary"
                    })
            );
        }
    }
}

#[test]
fn native_conflict_runs_one_in_process_fix_and_revalidates_the_repaired_tree() {
    let release = Release::new();
    let (_, repaired) = checks::conflicted_bump(&release);
    release.agent_runs(&format!(
        "git fetch origin prepared-repair\ngit reset --hard {repaired}\ngit push origin HEAD:release-0.2.0\ngh fake pr release-0.2.0 mergeable '\"MERGEABLE\"'\ngh fake checks {repaired} '{GREEN}'\n"
    ));
    let output = with_input(&mut command(&release, "0.2.0"), "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired));
    assert_eq!(
        stderr(&output)
            .matches("starting one targeted Fix run:")
            .count(),
        1
    );
    assert!(stderr(&output).contains("--no-merge --no-base-fix"));
    assert!(
        !release.root().join("gh-state.json.fix-calls").exists(),
        "Fix ran through Cargo"
    );
    assert_eq!(record(&release)["kind"], "Release run");
}

#[test]
fn native_failed_fix_leaves_a_draft_and_no_tag_then_a_pass_skips_it() {
    let release = Release::new();
    release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.ci_reports(RED);
    release.agent_runs("exit 1");
    let output = with_input(&mut command(&release, "0.2.0"), "");
    assert!(!output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("targeted Fix run"),
        "{}",
        stderr(&output)
    );
    assert!(release.origin_tag("v0.2.0").is_none());
    assert_eq!(release.pr("release-0.2.0")["isDraft"], true);
    let output = with_input(&mut command(&release, ""), "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
}

#[test]
fn release_pass_skips_non_shipped_changes_quietly_and_records_each_firing() {
    let release = Release::new();
    let before = release.origin_refs();
    for _ in 0..2 {
        let output = with_input(&mut command(&release, ""), "");
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(release.origin_refs(), before);
        assert!(
            release
                .root()
                .join("home/.thirdshift/logs/JacobStephens2/thirdshift/release.last-fired")
                .exists()
        );
    }
    let root = release
        .root()
        .join("home/.thirdshift/logs/JacobStephens2/thirdshift");
    let activity = fs::read_to_string(root.join("activity.log")).unwrap();
    assert_eq!(
        activity.matches("Release run skipped:").count(),
        1,
        "{activity}"
    );
    assert!(activity.contains("no shipped changes"), "{activity}");
    assert!(!root.join("commands/release").exists());
}

#[test]
fn release_version_and_bump_flags_are_mutually_exclusive_before_startup() {
    for arguments in [
        vec!["1.2.3", "--patch"],
        vec!["--minor", "--patch"],
        vec!["--major", "1.2.3"],
    ] {
        let release = Release::new();
        let mut run = command(&release, "");
        let output = with_input(run.args(arguments), "");
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("mutually exclusive"),
            "{}",
            stderr(&output)
        );
        assert!(!release.root().join("home/.thirdshift/logs").exists());
    }
}

#[test]
fn release_pass_admission_gates_skip_quietly_before_any_cut() {
    for gate in ["local", "origin", "quota", "experiment"] {
        let release = Release::new();
        let mut run = command(&release, "");
        let lock = fs::File::create(release.root().join("home/.thirdshift/release.lock")).unwrap();
        let reason = match gate {
            "local" => {
                lock.lock().unwrap();
                "machine-local release lock"
            }
            "origin" => {
                let token = release.origin(&["rev-parse", "main"]);
                release.origin(&["update-ref", "refs/thirdshift/reservations/release", &token]);
                "origin release reservation"
            }
            "quota" => {
                let path = release.root().join("gh-state.json");
                let mut state: Value =
                    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
                state["rate_limits"] = json!({"core":{"remaining":0,"reset":(chrono::Utc::now() + chrono::Duration::hours(1)).timestamp()}});
                fs::write(path, state.to_string()).unwrap();
                "Quota wait"
            }
            "experiment" => {
                let pid = std::process::id();
                let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
                let ticks = stat
                    .rsplit_once(") ")
                    .unwrap()
                    .1
                    .split_whitespace()
                    .nth(19)
                    .unwrap();
                let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
                let hostname =
                    String::from_utf8(Command::new("hostname").output().unwrap().stdout).unwrap();
                let directory = release
                    .root()
                    .join("home/.thirdshift/logs/acme/other/commands/experiment");
                fs::create_dir_all(&directory).unwrap();
                fs::write(directory.join("live.command.json"), json!({
                    "version":1, "kind":"Experiment", "repository":"acme/other", "hostname":hostname.trim(),
                    "name":"live", "process_id":pid, "process_start":format!("{}:{ticks}", boot.trim()),
                    "harness":"claude", "started_at":chrono::Utc::now().to_rfc3339(), "stages":[]
                }).to_string()).unwrap();
                "Experiment live is live"
            }
            _ => unreachable!(),
        };
        let before = release.origin_refs();
        for _ in 0..2 {
            let output = with_input(&mut run, "");
            assert!(output.status.success(), "{gate}: {}", stderr(&output));
            assert!(
                output.stdout.is_empty() && output.stderr.is_empty(),
                "{gate}: {output:?}"
            );
        }
        assert_eq!(release.origin_refs(), before);
        let root = release
            .root()
            .join("home/.thirdshift/logs/JacobStephens2/thirdshift");
        let activity = fs::read_to_string(root.join("activity.log")).unwrap();
        assert_eq!(
            activity.matches("Release run skipped:").count(),
            1,
            "{activity}"
        );
        assert!(activity.contains(reason), "{activity}");
        assert!(root.join("release.last-fired").exists());
        assert!(!root.join("commands/release").exists());
    }
}

#[test]
fn release_pass_nothing_changed_and_red_shipped_ci_are_quiet_skips() {
    for nothing_changed in [true, false] {
        let release = Release::new();
        let mut run = command(&release, "");
        let reason = if nothing_changed {
            let tag = release.origin(&["rev-parse", "v0.1.0"]);
            release.origin(&["update-ref", "refs/heads/main", &tag]);
            "nothing changed"
        } else {
            shipped(&release);
            release.main_ci("completed", "failure");
            "main push CI (ci.yml)"
        };
        let before = release.origin_refs();
        let output = with_input(&mut run, "");
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "{output:?}"
        );
        assert_eq!(release.origin_refs(), before);
        let activity = fs::read_to_string(
            release
                .root()
                .join("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
        )
        .unwrap();
        assert!(activity.contains(reason), "{activity}");
    }
}

#[test]
fn release_pass_selects_older_green_shipped_content_or_waits_for_pending_ci() {
    for older_green in [true, false] {
        let release = Release::new();
        let green = shipped(&release);
        release.merge_pr("pending-pass", "Pending content", "");
        let tip = release.origin(&["rev-parse", "main"]);
        release.main_ci_on(
            &tip,
            "completed",
            "success",
            if older_green { 100 } else { 2 },
        );
        if !older_green {
            release.main_ci_on(&green, "completed", "failure", 0);
        }
        let output = with_input(command(&release, "").arg("--patch"), "");
        assert!(output.status.success(), "{}", stderr(&output));
        let head = release.pr_head("release-0.1.1");
        assert_eq!(
            release.origin(&["rev-parse", &format!("{head}^")]),
            if older_green { green } else { tip }
        );
        assert_eq!(
            stderr(&output).contains("waiting for main push CI"),
            !older_green
        );
    }
}

#[test]
fn release_pass_reports_pending_main_ci_before_it_finishes() {
    use std::time::{Duration, Instant};
    let release = Release::new();
    shipped(&release);
    release.main_ci_on(
        &release.origin(&["rev-parse", "main"]),
        "completed",
        "success",
        100000,
    );
    let mut run = command(&release, "");
    run.arg("--patch");
    let path = release.root().join("pending-main-output");
    let output = fs::File::create(&path).unwrap();
    run.stdout(output.try_clone().unwrap()).stderr(output);
    let owned = support::child_lifetime::OwnedRuns::new();
    let mut child = owned.spawn(&mut run);
    // This covers binary startup, isolated Git preparation and the first CI read.
    let deadline = Instant::now() + Duration::from_secs(15);
    while !fs::read_to_string(&path)
        .unwrap()
        .contains("waiting for main push CI")
    {
        assert!(
            child.try_wait().is_none(),
            "{}",
            fs::read_to_string(&path).unwrap()
        );
        assert!(
            Instant::now() < deadline,
            "no progress was visible while main CI was pending"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // A waiting Pass is live work: in Working, and reachable by Stop.
    let live = record(&release);
    assert!(live["outcome"].is_null(), "{live}");
    assert_eq!(inbox(&release)["commands"][0]["group"], "working");
    release.main_ci("completed", "success");
    let deadline = Instant::now() + support::WAIT_BOUND;
    loop {
        if let Some(status) = child.try_wait() {
            assert!(status.success(), "{}", fs::read_to_string(&path).unwrap());
            break;
        }
        assert!(Instant::now() < deadline, "Release Pass did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(record(&release)["outcome"], "merged");
}

#[test]
fn a_release_pass_that_waited_for_main_ci_records_its_later_skip() {
    use std::time::{Duration, Instant};
    let release = Release::new();
    shipped(&release);
    release.main_ci_on(
        &release.origin(&["rev-parse", "main"]),
        "completed",
        "failure",
        100000,
    );
    let mut run = command(&release, "");
    run.arg("--patch");
    let path = release.root().join("pending-main-output");
    let output = fs::File::create(&path).unwrap();
    run.stdout(output.try_clone().unwrap()).stderr(output);
    let owned = support::child_lifetime::OwnedRuns::new();
    let mut child = owned.spawn(&mut run);
    // This covers binary startup, isolated Git preparation and the first CI read.
    let deadline = Instant::now() + Duration::from_secs(15);
    while !fs::read_to_string(&path)
        .unwrap()
        .contains("waiting for main push CI")
    {
        assert!(
            child.try_wait().is_none(),
            "{}",
            fs::read_to_string(&path).unwrap()
        );
        assert!(Instant::now() < deadline, "main CI wait never began");
        std::thread::sleep(Duration::from_millis(20));
    }
    release.main_ci("completed", "failure");
    let deadline = Instant::now() + support::WAIT_BOUND;
    loop {
        if let Some(status) = child.try_wait() {
            assert!(status.success(), "{}", fs::read_to_string(&path).unwrap());
            break;
        }
        assert!(Instant::now() < deadline, "Release Pass did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }
    let ended = record(&release);
    assert_eq!(ended["outcome"], "skipped", "{ended}");
    assert!(
        ended["cause"]
            .as_str()
            .is_some_and(|cause| cause.contains("main push CI")),
        "{ended}"
    );
    assert_ne!(inbox(&release)["commands"][0]["group"], "working");
    assert!(
        release
            .origin(&["for-each-ref", "refs/heads/release-*"])
            .is_empty()
    );
}

#[test]
fn release_pass_filters_shipped_sources_assets_skills_and_pipeline_paths() {
    for (path, eligible) in [
        ("src/main.rs", true),
        ("src/console/start_form.html", true),
        ("src/console/assets/start.js", true),
        ("skills/notes/SKILL.md", true),
        ("processes/notes.toml", true),
        (".github/workflows/release.yml", true),
        (".github/workflows/publish-crates.yml", true),
        ("scripts/release-notes.sh", true),
        ("dist-workspace.toml", true),
        ("rust-toolchain.toml", true),
        ("docs/guide.md", false),
        ("site/index.html", false),
        (".agents/skills/notes/SKILL.md", false),
        ("tests/release.rs", false),
        // The Day shift's ruleset wizard and the workflow generator ship nothing.
        ("scripts/setup-release-tags.sh", false),
        ("scripts/generate-release.py", false),
    ] {
        let release = Release::new();
        let contributor = release.contributor();
        let file = contributor.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "Changed\n").unwrap();
        git(&contributor, &["add", "--", path]);
        git(&contributor, &["commit", "-qm", "Change a path"]);
        git(&contributor, &["push", "-q", "origin", "HEAD:main"]);
        release.main_ci("completed", "success");
        release.ci_reports(GREEN);
        let output = with_input(command(&release, "").arg("--patch"), "");
        assert!(output.status.success(), "{path}: {}", stderr(&output));
        assert_eq!(
            release.origin_tag("v0.1.1").is_some(),
            eligible,
            "{path}: {}",
            stderr(&output)
        );
    }
}

fn record(release: &Release) -> Value {
    let directory = release
        .root()
        .join("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/release");
    let path = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".command.json"))
        .unwrap();
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn inbox(release: &Release) -> Value {
    let output = release
        .command(
            Path::new(env!("CARGO_BIN_EXE_thirdshift")),
            &release.maintainer(),
        )
        .args(["status", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    serde_json::from_str(&stdout(&output)).unwrap()
}

fn sandbox_summary(release: &Release) {
    sandbox_reply(release, "A sandboxed release summary (#2).", false);
}

fn sandbox_reply(release: &Release, reply: &str, automatic: bool) {
    let binary = release.bin().join("claude");
    fs::remove_file(&binary).unwrap();
    fs::write(
        release.root().join("home/private-release-secret"),
        "host secret",
    )
    .unwrap();
    write_executable(
        &binary,
        &format!(
            r#"#!/bin/sh
if [ "$1 $2" = 'auth status' ]; then exit 0; fi
test ! -r '{}/home/private-release-secret' || exit 81
test -z "${{HOST_RELEASE_SECRET-}}" || exit 82
test "$HOME" != '{}/home' || exit 83
prompt=$(cat)
{}
printf '%s\n' '{{"type":"system","subtype":"init","session_id":"release-summary-test"}}'
printf '%s\n' '{}'
"#,
            release.root().display(),
            release.root().display(),
            if automatic {
                r#"case "$prompt" in *'"minor":"0.2.0"'*) ;; *) exit 84;; esac
case "$prompt" in *'"patch":"0.1.1"'*) ;; *) exit 85;; esac
case "$prompt" in *'"number":2'*'"number":3'*) ;; *) exit 86;; esac
case "$prompt" in *'Old news.'*) exit 87;; esac"#
            } else {
                r#"case "$prompt" in *'Choose the bump'*) exit 88;; esac"#
            },
            json!({"type":"result", "subtype":"success", "is_error": false, "result": reply})
        ),
    );
}

fn shipped(release: &Release) -> String {
    let contributor = release.contributor();
    git(&contributor, &["checkout", "-q", "--detach", "origin/main"]);
    fs::write(contributor.join("README.md"), "Shipped change\n").unwrap();
    git(&contributor, &["add", "README.md"]);
    git(&contributor, &["commit", "-qm", "Shipped change"]);
    git(&contributor, &["push", "-q", "origin", "HEAD:main"]);
    let sha = release.origin(&["rev-parse", "main"]);
    release.main_ci("completed", "success");
    release.ci_reports(GREEN);
    sha
}

#[test]
fn release_pass_selects_minor_or_patch_and_summary_in_one_sandboxed_session() {
    for (choice, version) in [("minor", "0.2.0"), ("patch", "0.1.1")] {
        let release = Release::new();
        let base = shipped(&release);
        sandbox_reply(
            &release,
            &json!({"bump":choice,"summary":"Automatic summary (#2)."}).to_string(),
            true,
        );
        let output = with_input(
            command(&release, "").env("HOST_RELEASE_SECRET", "stay outside"),
            "",
        );
        assert!(output.status.success(), "{}", stderr(&output));
        let pr = release.pr(&format!("release-{version}"));
        assert!(
            pr["body"]
                .as_str()
                .unwrap()
                .contains("Automatic summary (#2).")
        );
        assert_eq!(
            release.origin(&[
                "rev-parse",
                &format!("{}^", release.pr_head(&format!("release-{version}")))
            ]),
            base
        );
        let record = record(&release);
        let directory = release
            .root()
            .join("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/release");
        let traces: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.to_string_lossy()
                    .ends_with("release-summary.logs.jsonl")
            })
            .collect();
        assert_eq!(traces.len(), 1, "{traces:?}");
        assert!(
            fs::read_to_string(&traces[0])
                .unwrap()
                .contains("release-summary-test")
        );
        assert_eq!(record["outcome"], "merged");
    }
}

#[test]
fn release_pass_automatic_selection_never_chooses_major_and_falls_back_once() {
    for reply in [
        Some(r#"{"bump":"major","summary":"A usable summary (#2)."}"#),
        Some("not JSON"),
        Some(r#"{"bump":"patch","summary":" "}"#),
        None,
    ] {
        let release = Release::new();
        shipped(&release);
        if let Some(reply) = reply {
            sandbox_reply(&release, reply, true);
        } else {
            release.summary_says(" ");
        }
        let output = with_input(&mut command(&release, ""), "");
        assert!(output.status.success(), "{}", stderr(&output));
        let pr = release.pr("release-0.1.1");
        let body = pr["body"].as_str().unwrap();
        if reply.is_some_and(|reply| reply.contains("usable")) {
            assert!(body.contains("A usable summary"), "{body}");
        } else {
            assert_eq!(body.matches("## What's Changed").count(), 1, "{body}");
            assert!(
                !body
                    .split("<!-- release-summary:end -->")
                    .next()
                    .unwrap()
                    .contains("## What's Changed")
            );
        }
        assert!(release.origin_tag("v1.0.0").is_none());
    }
}

#[test]
fn release_pass_pinned_bumps_write_only_a_summary() {
    for (flag, version) in [
        ("--major", "1.0.0"),
        ("--minor", "0.2.0"),
        ("--patch", "0.1.1"),
    ] {
        let release = Release::new();
        shipped(&release);
        sandbox_summary(&release);
        let output = with_input(command(&release, "").arg(flag), "");
        assert!(output.status.success(), "{}", stderr(&output));
        let pr = release.pr(&format!("release-{version}"));
        assert!(
            pr["body"]
                .as_str()
                .unwrap()
                .contains("A sandboxed release summary")
        );
        assert!(release.origin_tag(&format!("v{version}")).is_some());
    }
}

#[test]
fn a_blank_minor_summary_falls_back_to_patch() {
    let release = Release::new();
    shipped(&release);
    sandbox_reply(&release, r#"{"bump":"minor","summary":" "}"#, true);
    let output = with_input(&mut command(&release, ""), "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        release.origin_tag("v0.1.1").is_some(),
        "invalid summary must fall back to patch; minor tag = {:?}",
        release.origin_tag("v0.2.0")
    );
}

#[test]
fn native_release_publishes_green_content_without_reading_the_launch_tree() {
    let release = Release::new();
    release.ci_reports(GREEN);
    sandbox_summary(&release);
    let mut run = command(&release, "0.2.0");
    // A Ticket run or Spec run at the Command limit owns this same slot.
    let slots = release
        .root()
        .join("home/.thirdshift/command-slots/jacobstephens2/thirdshift");
    fs::create_dir_all(&slots).unwrap();
    let occupied = fs::File::create(slots.join("0.slot")).unwrap();
    occupied.lock().unwrap();
    fs::write(release.maintainer().join("Cargo.toml"), "not a manifest\n").unwrap();
    fs::write(release.maintainer().join("untracked.txt"), "leave alone\n").unwrap();
    let before = git(&release.maintainer(), &["status", "--porcelain"]);
    let main = release.origin(&["rev-parse", "main"]);
    let output = with_input(run.env("HOST_RELEASE_SECRET", "must stay outside"), "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        git(&release.maintainer(), &["status", "--porcelain"]),
        before
    );
    assert_eq!(
        fs::read_to_string(release.maintainer().join("Cargo.toml")).unwrap(),
        "not a manifest\n"
    );
    let head = release.pr_head("release-0.2.0");
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^")]), main);
    assert_eq!(release.origin_tag("v0.2.0"), Some(head.clone()));
    // The workflow checks a fetched tag; Release publishes it directly to origin.
    git(
        &release.maintainer(),
        &[
            "fetch",
            "--no-tags",
            "origin",
            "refs/tags/v0.2.0:refs/tags/v0.2.0",
        ],
    );
    let guard = release
        .command(Path::new("python3"), &release.maintainer())
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/release-tag-guard.py"))
        .env("GITHUB_ACTOR", "JacobStephens2")
        .env("GITHUB_REF_NAME", "v0.2.0")
        .env("GITHUB_SHA", &head)
        .output()
        .unwrap();
    assert!(guard.status.success(), "{}", stderr(&guard));
    assert_eq!(
        release.origin(&["diff", "--name-only", &main, &head]),
        "Cargo.lock\nCargo.toml"
    );
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    assert!(
        release.pr("release-0.2.0")["body"]
            .as_str()
            .unwrap()
            .contains("A sandboxed release summary (#2)."),
        "{}",
        stderr(&output)
    );
    assert_eq!(record(&release)["kind"], "Release run");
    assert_eq!(record(&release)["outcome"], "merged");
    assert!(
        release
            .origin(&["for-each-ref", "refs/thirdshift/reservations/release"])
            .is_empty()
    );
    assert!(
        !release
            .gh_calls()
            .iter()
            .any(|call| call.iter().any(|word| word == "update"))
    );
    assert_eq!(
        git(&release.maintainer(), &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        1
    );
    let traces = fs::read_dir(
        release
            .root()
            .join("home/.thirdshift/logs/JacobStephens2/thirdshift/commands/release"),
    )
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|path| {
        path.to_string_lossy()
            .ends_with("release-summary.logs.jsonl")
    })
    .collect::<Vec<_>>();
    assert_eq!(traces.len(), 1, "{traces:?}");
    assert!(
        fs::read_to_string(&traces[0])
            .unwrap()
            .contains("release-summary-test")
    );
}

#[test]
fn native_release_summary_failure_keeps_generated_notes_outside_markers() {
    let release = Release::new();
    release.summary_says(" ");
    release.ci_reports(GREEN);
    let output = with_input(&mut command(&release, "0.2.0"), "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("release summary unavailable"));
    let pr = release.pr("release-0.2.0");
    let body = pr["body"].as_str().unwrap();
    let marked = body
        .split("<!-- release-summary:start -->")
        .nth(1)
        .unwrap()
        .split("<!-- release-summary:end -->")
        .next()
        .unwrap();
    assert!(!marked.contains("## What's Changed"), "{body}");
    assert!(
        body.split("<!-- release-summary:end -->")
            .nth(1)
            .unwrap()
            .contains("## What's Changed"),
        "{body}"
    );
}

#[test]
fn native_release_refuses_factory_sessions_before_any_push() {
    let release = Release::new();
    let before = release.origin_refs();
    let output = with_input(
        command(&release, "0.2.0").env("THIRDSHIFT_SESSION", "test-session"),
        "",
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("inside a factory Session"),
        "{output:?}"
    );
    assert_eq!(release.origin_refs(), before);
}

#[test]
fn native_release_rejects_versions_and_wrong_origins_before_any_push() {
    for version in ["0.1.0", "0.0.9", "01.2.3", "1.2", "v1.2.3", "1.2.3-rc.1"] {
        let release = Release::new();
        let before = release.origin_refs();
        let output = with_input(&mut command(&release, version), "");
        assert!(!output.status.success(), "{version}: {output:?}");
        assert_eq!(release.origin_refs(), before, "{version}");
    }
    let release = Release::new();
    let mut run = command(&release, "0.2.0");
    git(
        &release.maintainer(),
        &[
            "config",
            "remote.origin.url",
            "https://github.com/acme/widgets.git",
        ],
    );
    let before = release.origin_refs();
    let output = with_input(&mut run, "");
    assert!(!output.status.success(), "{output:?}");
    assert!(
        stderr(&output).contains("release requires origin"),
        "{}",
        stderr(&output)
    );
    assert_eq!(release.origin_refs(), before);
}

#[test]
fn native_release_parent_record_and_competing_locks_refuse_cleanly() {
    for environment in ["THIRDSHIFT_SESSION", "THIRDSHIFT_PARENT_RECORD"] {
        let release = Release::new();
        let before = release.origin_refs();
        let output = with_input(command(&release, "0.2.0").env(environment, "held"), "");
        assert!(!output.status.success());
        assert!(stderr(&output).contains("inside a factory Session"));
        assert_eq!(release.origin_refs(), before);
    }
    let release = Release::new();
    let mut run = command(&release, "0.2.0");
    let lock = fs::File::create(release.root().join("home/.thirdshift/release.lock")).unwrap();
    lock.lock().unwrap();
    let before = release.origin_refs();
    let output = with_input(&mut run, "");
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("machine-local release lock"),
        "{}",
        stderr(&output)
    );
    assert_eq!(release.origin_refs(), before);
    drop(lock);
    let token = release.origin(&["rev-parse", "main"]);
    release.origin(&["update-ref", "refs/thirdshift/reservations/release", &token]);
    let before = release.origin_refs();
    let output = with_input(&mut command(&release, "0.2.0"), "");
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("origin release reservation"),
        "{}",
        stderr(&output)
    );
    assert_eq!(release.origin_refs(), before);
}

#[test]
fn native_release_selects_older_green_main_and_waits_for_pending_main() {
    for older_green in [true, false] {
        let release = Release::new();
        let green = release.origin(&["rev-parse", "main"]);
        release.merge_pr("pending-native", "Pending content", "");
        let tip = release.origin(&["rev-parse", "main"]);
        release.main_ci_on(
            &tip,
            "completed",
            "success",
            if older_green { 100 } else { 2 },
        );
        if !older_green {
            release.main_ci_on(&green, "completed", "failure", 0);
        }
        release.ci_reports(GREEN);
        let output = with_input(&mut command(&release, "0.2.0"), "");
        assert!(output.status.success(), "{}", stderr(&output));
        let expected = if older_green { green } else { tip };
        let head = release.pr_head("release-0.2.0");
        assert_eq!(
            release.origin(&["rev-parse", &format!("{head}^")]),
            expected
        );
        assert_eq!(
            stderr(&output).contains("waiting for main push CI"),
            !older_green
        );
    }
}

#[test]
fn native_release_all_red_main_fails_without_publishing_a_branch_or_tag() {
    let release = Release::new();
    release.main_ci("completed", "failure");
    let before = release.origin_refs();
    let output = with_input(&mut command(&release, "0.2.0"), "");
    assert!(!output.status.success(), "{output:?}");
    assert!(
        stderr(&output).contains("no candidate main commit has successful push CI"),
        "{}",
        stderr(&output)
    );
    assert_eq!(release.origin_refs(), before);
    assert_eq!(record(&release)["outcome"], "failed");
}

#[test]
fn native_release_email_unless_merged_suppresses_success_and_emails_unmerged_publication() {
    for merge in [true, false] {
        let release = Release::new();
        release.ci_reports(GREEN);
        if !merge {
            release.gh(&["fake", "refuse-merges", "1", "merge refused"]);
        }
        let resend =
            support::resend::ResendStandIn::replying(200, r#"{"id":"release-notification"}"#);
        let output = with_input(
            command(&release, "0.2.0")
                .args(["--email-unless-merged", "maintainer@example.test"])
                .env("RESEND_API_KEY", "re_test_release")
                .env("THIRDSHIFT_RESEND_URL", resend.url()),
            "",
        );
        assert_eq!(output.status.success(), merge, "{}", stderr(&output));
        assert_eq!(resend.requests().len(), if merge { 0 } else { 1 });
        assert_eq!(
            release.pr("release-0.2.0")["state"],
            if merge { "MERGED" } else { "OPEN" }
        );
        assert!(release.origin_tag("v0.2.0").is_some());
        let status = inbox(&release);
        assert_eq!(
            status["commands"][0]["group"],
            if merge { "done" } else { "needs you" },
            "{status}"
        );
        if !merge {
            assert!(stderr(&output).contains(
                "GitHub Release was published, but the bump pull request was not merged"
            ));
            assert_eq!(record(&release)["outcome"], "failed");
            assert_eq!(
                record(&release)["pull_request"],
                release.pr("release-0.2.0")["url"]
            );
        }
    }
}

#[test]
fn native_release_stop_releases_locks_reservation_and_live_label() {
    use std::time::{Duration, Instant};
    let release = Release::new();
    let mut run = command(&release, "0.2.0");
    let output_path = release.root().join("native-live-output");
    let output = fs::File::create(&output_path).unwrap();
    run.stdout(output.try_clone().unwrap()).stderr(output);
    // Pending head CI keeps the Release run live after it opens its bump PR.
    release.ci_reports(r#"[{"name":"test","conclusion":"success","pending_polls":100000}]"#);
    let owned = support::child_lifetime::OwnedRuns::new();
    let mut child = owned.spawn(&mut run);
    let deadline = Instant::now() + support::WAIT_BOUND;
    loop {
        if fs::read_to_string(&output_path)
            .unwrap()
            .contains("waiting for release bump pull-request checks")
        {
            break;
        }
        assert!(
            child.try_wait().is_none(),
            "{}",
            fs::read_to_string(&output_path).unwrap()
        );
        assert!(
            Instant::now() < deadline,
            "Release run never reached CI wait"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let live = record(&release);
    assert!(live["outcome"].is_null(), "{live}");
    assert_eq!(inbox(&release)["commands"][0]["group"], "working");
    assert!(
        release.pr("release-0.2.0")["labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "in-progress")
    );
    let stopped = release
        .command(
            Path::new(env!("CARGO_BIN_EXE_thirdshift")),
            &release.maintainer(),
        )
        .args(["stop", live["name"].as_str().unwrap(), "--json"])
        .env_remove("THIRDSHIFT_SESSION")
        .env_remove("THIRDSHIFT_PARENT_RECORD")
        .output()
        .unwrap();
    assert!(stopped.status.success(), "{}", stderr(&stopped));
    let deadline = Instant::now() + support::WAIT_BOUND;
    let status = loop {
        if let Some(status) = child.try_wait() {
            break status;
        }
        assert!(Instant::now() < deadline, "stopped Release run did not end");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success());
    assert!(
        release
            .origin(&["for-each-ref", "refs/thirdshift/reservations/release"])
            .is_empty()
    );
    assert!(
        !release.pr("release-0.2.0")["labels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|label| label == "in-progress")
    );
    let lock = fs::File::options()
        .write(true)
        .open(release.root().join("home/.thirdshift/release.lock"))
        .unwrap();
    assert!(lock.try_lock().is_ok());
    assert!(release.origin_tag("v0.2.0").is_none());
}

#[test]
fn native_release_refuses_a_blocking_review_without_a_merge_attempt() {
    for decision in ["REVIEW_REQUIRED", "CHANGES_REQUESTED"] {
        let release = Release::new();
        let initial_merges = release.merge_calls().len();
        release.gh(&[
            "fake", "on-ci-read", "1",
            &format!("gh fake checks \"$FAKE_CI_SHA\" '{GREEN}'\ngh fake pr release-0.2.0 reviewDecision '\"{decision}\"'"),
        ]);
        let output = with_input(&mut command(&release, "0.2.0"), "");
        assert!(!output.status.success(), "{decision}: {}", stderr(&output));
        assert_eq!(
            release.merge_calls().len(),
            initial_merges,
            "{decision}: a blocking review must not issue gh pr merge"
        );
        assert_eq!(release.pr("release-0.2.0")["state"], "OPEN");
    }
}

#[test]
fn native_release_does_not_execute_launch_checkout_hooks() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let mut run = command(&release, "0.2.0");
    let hooks = release.maintainer().join("local-release-hooks");
    fs::create_dir_all(&hooks).unwrap();
    let marker = release.root().join("launch-hook-executed");
    write_executable(
        &hooks.join("post-checkout"),
        &format!("#!/bin/sh\nprintf invoked > '{}'\n", marker.display()),
    );
    git(
        &release.maintainer(),
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    let output = with_input(&mut run, "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        !marker.exists(),
        "Release run executed a hook from the Launch directory working tree"
    );
}

#[test]
fn native_release_does_not_execute_launch_repository_filters() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let mut run = command(&release, "0.2.0");
    let root = release.root();
    let marker = root.join("launch-filter-executed");
    let smudge = root.join("smudge.sh");
    let clean = root.join("clean.sh");
    let fsmonitor = root.join("fsmonitor.sh");
    let record = marker.display().to_string();
    write_executable(
        &smudge,
        &format!("#!/bin/sh\nprintf smudge >> '{record}'\ncat\n"),
    );
    write_executable(
        &clean,
        &format!("#!/bin/sh\nprintf clean >> '{record}'\ncat\n"),
    );
    write_executable(
        &fsmonitor,
        &format!("#!/bin/sh\nprintf fs >> '{record}'\nexit 0\n"),
    );
    let launch = release.maintainer();
    git(
        &launch,
        &["config", "filter.pwn.smudge", smudge.to_str().unwrap()],
    );
    git(
        &launch,
        &["config", "filter.pwn.clean", clean.to_str().unwrap()],
    );
    git(&launch, &["config", "filter.pwn.required", "true"]);
    fs::create_dir_all(launch.join(".git/info")).unwrap();
    fs::write(launch.join(".git/info/attributes"), "* filter=pwn\n").unwrap();
    git(
        &launch,
        &["config", "core.fsmonitor", fsmonitor.to_str().unwrap()],
    );
    assert!(
        !marker.exists(),
        "fixture setup executed a filter or fsmonitor"
    );
    let output = with_input(&mut run, "");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        !marker.exists(),
        "Release run executed a filter or fsmonitor from the Launch directory repository"
    );
}

#[test]
fn a_skipped_release_pass_sends_no_run_notification_even_with_email() {
    let release = Release::new();
    shipped(&release);
    release.main_ci("completed", "failure");
    let resend = support::resend::ResendStandIn::replying(200, r#"{"id":"release-skip"}"#);
    let output = with_input(
        command(&release, "")
            .args(["--email", "maintainer@example.test"])
            .env("RESEND_API_KEY", "re_test_release")
            .env("THIRDSHIFT_RESEND_URL", resend.url()),
        "",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let activity = fs::read_to_string(
        release
            .root()
            .join("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(activity.contains("Release run skipped:"), "{activity}");
    assert!(resend.requests().is_empty());
}
