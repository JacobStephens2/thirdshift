//! The installed binary's Release run, using the same real origin as the legacy flow.
use super::*;

fn command(release: &Release, version: &str) -> Command {
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
    fs::write(
        release.root().join("home/.thirdshift/config.toml"),
        "[harness]\ndefault = 'claude'\n",
    )
    .unwrap();
    let mut command = release.command(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &launch);
    command
        .args(["release", version])
        .env_remove("THIRDSHIFT_SESSION")
        .env_remove("THIRDSHIFT_PARENT_RECORD")
        .env("THIRDSHIFT_POLL_MS", "1")
        .env("THIRDSHIFT_CI_GRACE_MS", "100");
    command
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

fn sandbox_summary(release: &Release) {
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
cat >/dev/null
printf '%s\n' '{{"type":"system","subtype":"init","session_id":"release-summary-test"}}'
printf '%s\n' '{{"type":"result","subtype":"success","is_error":false,"result":"A sandboxed release summary (#2)."}}'
"#,
            release.root().display(),
            release.root().display()
        ),
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
    let output = run
        .env("HOST_RELEASE_SECRET", "must stay outside")
        .output()
        .unwrap();
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
            .contains("A sandboxed release summary (#2).")
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
    release.ci_reports(GREEN);
    let output = command(&release, "0.2.0").output().unwrap();
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
    let output = command(&release, "0.2.0")
        .env("THIRDSHIFT_SESSION", "test-session")
        .output()
        .unwrap();
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
        let output = command(&release, version).output().unwrap();
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
    let output = run.output().unwrap();
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
        let output = command(&release, "0.2.0")
            .env(environment, "held")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(stderr(&output).contains("inside a factory Session"));
        assert_eq!(release.origin_refs(), before);
    }
    let release = Release::new();
    let mut run = command(&release, "0.2.0");
    let lock = fs::File::create(release.root().join("home/.thirdshift/release.lock")).unwrap();
    lock.lock().unwrap();
    let before = release.origin_refs();
    let output = run.output().unwrap();
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
    let output = command(&release, "0.2.0").output().unwrap();
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
        let output = command(&release, "0.2.0").output().unwrap();
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
    let output = command(&release, "0.2.0").output().unwrap();
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
        let output = command(&release, "0.2.0")
            .args(["--email-unless-merged", "maintainer@example.test"])
            .env("RESEND_API_KEY", "re_test_release")
            .env("THIRDSHIFT_RESEND_URL", resend.url())
            .output()
            .unwrap();
        assert_eq!(output.status.success(), merge, "{}", stderr(&output));
        assert_eq!(resend.requests().len(), if merge { 0 } else { 1 });
        assert_eq!(
            release.pr("release-0.2.0")["state"],
            if merge { "MERGED" } else { "OPEN" }
        );
        assert!(release.origin_tag("v0.2.0").is_some());
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
    assert!(!child.wait().success());
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
        let output = command(&release, "0.2.0").output().unwrap();
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
    let output = run.output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        !marker.exists(),
        "Release run executed a hook from the Launch directory working tree"
    );
}
