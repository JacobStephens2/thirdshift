use super::*;

fn set_pr(release: &Release, field: &str, value: Value) {
    release.gh(&["fake", "pr", "release-0.2.0", field, &value.to_string()]);
}

fn simulate_fix(release: &Release, script: &str) {
    fs::write(release.root().join("gh-state.json.fix"), script).unwrap();
}

fn fix_calls(release: &Release) -> Vec<String> {
    fs::read_to_string(release.root().join("gh-state.json.fix-calls"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn run(release: &Release, args: &[&str], env: &[(&str, &str)]) -> Output {
    // The shared release bound covers script startup, Git/GitHub reads, fake
    // Fix/summary/Experiment work, publication/merge and output collection on
    // a loaded runner; the guardian owns all fake Fix children too.
    release.run_script_with(args, "", env)
}

/// Prepare the exact reported pattern: a bump based on older main, followed
/// by a competing version change. Keep a real conflict-resolution merge ready
/// for the fake Fix command to push, with the requested version retained.
fn conflicted_bump(release: &Release) -> (String, String) {
    let old = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    let contributor = release.contributor();
    for file in ["Cargo.toml", "Cargo.lock"] {
        fs::write(contributor.join(file), with_version(file, "0.1.1")).unwrap();
    }
    git(
        &contributor,
        &["commit", "-q", "-a", "-m", "Competing version"],
    );
    git(&contributor, &["push", "-q", "origin", "HEAD:main"]);
    git(&contributor, &["checkout", "-q", "release-0.2.0"]);
    let conflict = isolated(&mut Command::new("git"))
        .current_dir(&contributor)
        .args(["merge", "--no-edit", "origin/main"])
        .output()
        .unwrap();
    assert_eq!(conflict.status.code(), Some(1));
    assert!(stdout(&conflict).contains("CONFLICT"));
    for file in ["Cargo.toml", "Cargo.lock"] {
        fs::write(contributor.join(file), with_version(file, "0.2.0")).unwrap();
    }
    git(&contributor, &["add", "Cargo.toml", "Cargo.lock"]);
    git(
        &contributor,
        &["commit", "-q", "-m", "Resolve bump conflict"],
    );
    let repaired = git(&contributor, &["rev-parse", "HEAD"]);
    git(
        &contributor,
        &["push", "-q", "origin", "HEAD:prepared-repair"],
    );
    set_pr(release, "mergeable", json!("CONFLICTING"));
    (old, repaired)
}

fn adopt_script(repaired: &str) -> String {
    format!(
        r#"set -euo pipefail
git --git-dir="${{FAKE_GH_STATE%/*}}/origin.git" update-ref refs/heads/release-0.2.0 {repaired}
gh fake pr release-0.2.0 mergeable '"MERGEABLE"'
gh fake checks {repaired} '{GREEN}'
"#
    )
}

#[test]
fn a_conflict_without_workflows_runs_one_targeted_fix_and_releases_the_repaired_head() {
    let release = Release::new();
    fs::create_dir_all(release.root().join("home/.thirdshift")).unwrap();
    fs::write(
        release.root().join("home/.thirdshift/config.toml"),
        "[merge]\nalways = true\n[base]\nfix = true\n",
    )
    .unwrap();
    let (old, repaired) = conflicted_bump(&release);
    simulate_fix(&release, &adopt_script(&repaired));

    let output = run(&release, &["0.2.0"], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired.clone()));
    assert_ne!(repaired, old);
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    assert_eq!(
        fix_calls(&release),
        [format!(
            "fix {} --no-merge --no-base-fix",
            release.pr("release-0.2.0")["url"].as_str().unwrap()
        )]
    );
    assert!(
        release
            .pr_body("release-0.2.0")
            .contains("-version = \"0.1.1\"")
    );
}

#[test]
fn missing_workflows_expire_with_the_pr_head_and_recovery_command() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");

    let output = run(
        &release,
        &["0.2.0"],
        &[("RELEASE_WORKFLOW_STARTUP_SECONDS", "0")],
    );

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    for text in [
        "no workflows appeared",
        &head,
        "pull/4",
        "scripts/release.sh 0.2.0",
    ] {
        assert!(stderr(&output).contains(text), "{}", stderr(&output));
    }
    assert!(fix_calls(&release).is_empty());
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

#[test]
fn pending_and_healthy_ci_never_start_a_fix_even_after_startup_grace() {
    for resume in [false, true] {
        let release = Release::new();
        if resume {
            release.push_bump("0.2.0");
            release.open_bump_pr("0.2.0");
        }
        release.ci_reports(GREEN);

        let output = run(
            &release,
            &["0.2.0"],
            &[("RELEASE_WORKFLOW_STARTUP_SECONDS", "0")],
        );

        assert!(output.status.success(), "{}", stderr(&output));
        assert!(fix_calls(&release).is_empty());
    }
}

#[test]
fn completed_failing_ci_uses_the_same_fix_path() {
    let release = Release::new();
    release.ci_reports(RED);
    simulate_fix(
        &release,
        &format!(
            "gh fake checks \"$(git ls-remote origin refs/heads/release-0.2.0 | cut -f1)\" '{GREEN}'"
        ),
    );

    let output = run(&release, &["0.2.0"], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fix_calls(&release).len(), 1);
    assert!(stderr(&output).contains("CI failed (test)"));
}

#[test]
fn failed_interrupted_or_ineffective_fixes_leave_no_tag_and_are_not_retried() {
    for script in ["exit 1", "exit 130", "exit 0"] {
        let release = Release::new();
        conflicted_bump(&release);
        simulate_fix(&release, script);

        let output = run(&release, &["0.2.0"], &[]);

        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert_eq!(fix_calls(&release).len(), 1);
        assert_eq!(release.origin_tag("v0.2.0"), None);
        assert!(stderr(&output).contains("thirdshift fix"));
        assert!(stderr(&output).contains("scripts/release.sh 0.2.0"));
    }
}

#[test]
fn a_signal_interrupted_fix_reports_recovery_without_tagging() {
    let release = Release::new();
    let (head, _) = conflicted_bump(&release);
    // Signal the release shell while its foreground Fix also dies by signal,
    // as Ctrl-C can do during Cargo startup before thirdshift's handler runs.
    // Name only these two fixture processes, keeping the guardian alive.
    simulate_fix(&release, "kill -INT \"$PPID\"\nkill -INT \"$$\"\n");

    let output = run(&release, &["0.2.0"], &[]);

    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{err}");
    assert!(err.contains(&head), "{err}");
    assert!(err.contains("Recover with thirdshift fix"), "{err}");
    assert!(err.contains("scripts/release.sh 0.2.0"), "{err}");
    assert_eq!(fix_calls(&release).len(), 1);
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

#[test]
fn a_published_draft_pr_explains_how_to_finish_merging_without_moving_the_tag() {
    let release = Release::new();
    let head = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.origin(&["tag", "v0.2.0", &head]);
    set_pr(&release, "isDraft", json!(true));
    release.gh(&["fake", "fails", "pr merge"]);
    let refs = release.origin_refs();

    let output = run(&release, &["0.2.0"], &[]);

    let err = stderr(&output);
    assert!(output.status.success(), "{err}");
    assert!(err.contains("release still stands"), "{err}");
    assert!(err.contains("draft"), "{err}");
    assert!(
        err.contains(&format!(
            "gh pr ready {}",
            release.pr("release-0.2.0")["url"].as_str().unwrap()
        )),
        "{err}"
    );
    assert_eq!(release.origin_refs(), refs);
    assert_eq!(release.pr("release-0.2.0")["state"], "OPEN");
    assert!(fix_calls(&release).is_empty());
}

#[test]
fn a_second_ci_failure_after_a_successful_fix_stops_without_another_fix() {
    let release = Release::new();
    let (_, repaired) = conflicted_bump(&release);
    simulate_fix(&release, &adopt_script(&repaired).replace(GREEN, RED));

    let output = run(&release, &["0.2.0"], &[]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("CI failed (test) after the automatic Fix run"));
    assert_eq!(fix_calls(&release).len(), 1);
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

#[test]
fn an_external_head_change_during_a_green_read_is_adopted_without_a_fix() {
    let release = Release::new();
    let (old, repaired) = conflicted_bump(&release);
    set_pr(&release, "mergeable", json!("MERGEABLE"));
    release.gh(&[
        "fake",
        "on-ci-read",
        "1",
        &format!(
            "{}\ngh fake checks {old} '{GREEN}'",
            adopt_script(&repaired)
        ),
    ]);

    let output = run(&release, &["0.2.0"], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired));
    assert!(fix_calls(&release).is_empty());
}

#[test]
fn identity_mutations_after_fix_or_external_updates_refuse_publication() {
    for external in [false, true] {
        for (field, value) in [
            ("base", json!("elsewhere")),
            ("head", json!("elsewhere")),
            ("state", json!("CLOSED")),
            ("state", json!("MERGED")),
            ("url", json!("https://github.com/other/repo/pull/4")),
            ("isCrossRepository", json!(true)),
            ("isDraft", json!(true)),
        ] {
            let release = Release::new();
            release.push_bump("0.2.0");
            release.open_bump_pr("0.2.0");
            let mutation = format!("gh fake pr release-0.2.0 {field} '{value}'");
            if external {
                release.gh(&["fake", "on-ci-read", "1", &mutation]);
            } else {
                set_pr(&release, "mergeable", json!("CONFLICTING"));
                simulate_fix(&release, &mutation);
            }

            let output = run(&release, &["0.2.0"], &[]);

            assert_eq!(
                output.status.code(),
                Some(1),
                "{field}: {}",
                stderr(&output)
            );
            assert_eq!(release.origin_tag("v0.2.0"), None, "{field}");
            assert!(
                stderr(&output).contains("identity/state/branch"),
                "{field}: {}",
                stderr(&output)
            );
            assert_eq!(fix_calls(&release).len(), usize::from(!external));
        }
    }
}

#[test]
fn non_bump_changes_after_fix_or_external_updates_refuse_publication() {
    for external in [false, true] {
        for change in [
            BumpChange::WrongVersion,
            BumpChange::AddedFile,
            BumpChange::AppendComment("Cargo.lock"),
            BumpChange::FileMode,
        ] {
            let release = Release::new();
            let old = release.push_bump("0.2.0");
            release.open_bump_pr("0.2.0");
            let changed = release.push_change_to_bump("0.2.0", change);
            release.origin(&["update-ref", "refs/heads/release-0.2.0", &old]);
            if external {
                release.gh(&["fake", "on-ci-read", "1", &adopt_script(&changed)]);
            } else {
                set_pr(&release, "mergeable", json!("CONFLICTING"));
                simulate_fix(&release, &adopt_script(&changed));
            }

            let output = run(&release, &["0.2.0"], &[]);

            assert_eq!(
                output.status.code(),
                Some(1),
                "{change:?}: {}",
                stderr(&output)
            );
            assert_eq!(release.origin_tag("v0.2.0"), None);
            assert!(
                stderr(&output).contains("changes more than the version bump"),
                "{change:?}: {}",
                stderr(&output)
            );
        }
    }
}

#[test]
fn a_resumed_repaired_merge_checks_the_experiment_on_its_content_base() {
    let release = Release::new();
    let (_, repaired) = conflicted_bump(&release);
    let base = release.origin(&["rev-parse", "main"]);
    release.origin(&["update-ref", "refs/heads/release-0.2.0", &repaired]);
    set_pr(&release, "mergeable", json!("MERGEABLE"));
    release.ci_reports(GREEN);
    experiments::cargo_reports(&release, &[experiments::verdict("pass")]);

    let output = run(&release, &["--with-experiment", "0.2.0"], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(experiments::calls(&release)[0]["args"][2], base);
}

#[test]
fn a_repair_refreshes_the_experiment_summary_and_diff_for_its_new_content_base() {
    let release = Release::new();
    let (old, repaired) = conflicted_bump(&release);
    let base = release.origin(&["rev-parse", "main"]);
    let old_base = release.origin(&["rev-parse", &format!("{old}^")]);
    experiments::cargo_reports(
        &release,
        &[experiments::verdict("pass"), experiments::verdict("pass")],
    );
    simulate_fix(&release, &adopt_script(&repaired));

    let output = run(&release, &["--with-experiment", "0.2.0"], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    let calls = experiments::calls(&release);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["args"][2], old_base);
    assert_eq!(calls[1]["args"][2], base);
    assert!(
        release
            .pr_body("release-0.2.0")
            .contains("-version = \"0.1.1\"")
    );
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired));
}

#[test]
fn a_new_content_base_cannot_publish_under_an_obsolete_experiment() {
    let release = Release::new();
    let (_, repaired) = conflicted_bump(&release);
    experiments::cargo_reports(
        &release,
        &[experiments::verdict("pass"), experiments::verdict("fail")],
    );
    simulate_fix(&release, &adopt_script(&repaired));

    let output = run(&release, &["--with-experiment", "0.2.0"], &[]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("Experiment fail; no release override given"));
    assert_eq!(experiments::calls(&release).len(), 2);
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

#[test]
fn continued_external_head_churn_is_bounded_without_starting_fix_runs() {
    let release = Release::new();
    let old = release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    let contributor = release.contributor();
    git(&contributor, &["checkout", "-q", "release-0.2.0"]);
    let mut heads = vec![old.clone()];
    for _ in 0..7 {
        git(
            &contributor,
            &["commit", "-q", "--allow-empty", "-m", "Another head"],
        );
        heads.push(git(&contributor, &["rev-parse", "HEAD"]));
    }
    git(
        &contributor,
        &["push", "-q", "origin", "HEAD:prepared-churn"],
    );
    let mut script = String::from("case $FAKE_CI_SHA in\n");
    for pair in heads.windows(2) {
        script.push_str(&format!(
            "{}) git --git-dir=\"${{FAKE_GH_STATE%/*}}/origin.git\" update-ref refs/heads/release-0.2.0 {} ;;\n",
            pair[0], pair[1]
        ));
    }
    script.push_str("esac\n");
    release.gh(&["fake", "on-ci-read", "10", &script]);

    let output = run(&release, &["0.2.0"], &[]);

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stderr(&output).contains("head changed more than five times"));
    assert_eq!(release.origin_tag("v0.2.0"), None);
    assert!(fix_calls(&release).is_empty());
}

#[test]
fn spec_resume_after_interrupted_fix_refreshes_release_notes() {
    let release = Release::new();
    let (_, repaired) = conflicted_bump(&release);
    set_pr(
        &release,
        "body",
        json!("Old release notes: version diff starts at 0.1.0"),
    );
    simulate_fix(&release, &(adopt_script(&repaired) + "\nexit 130\n"));
    let interrupted = run(&release, &["0.2.0"], &[]);
    assert_eq!(
        interrupted.status.code(),
        Some(1),
        "{}",
        stderr(&interrupted)
    );
    assert_eq!(release.origin_tag("v0.2.0"), None);
    assert_eq!(
        release.pr_body("release-0.2.0"),
        "Old release notes: version diff starts at 0.1.0"
    );

    let resumed = run(&release, &["0.2.0"], &[]);
    assert!(resumed.status.success(), "{}", stderr(&resumed));
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired));
    assert!(
        release
            .pr_body("release-0.2.0")
            .contains("-version = \"0.1.1\""),
        "Published repaired content with stale release notes: {}",
        release.pr_body("release-0.2.0")
    );
}

#[test]
fn a_notes_marker_detects_an_external_repair_from_another_checkout() {
    let release = Release::new();
    let (old, repaired) = conflicted_bump(&release);
    let old_base = release.origin(&["rev-parse", &format!("{old}^")]);
    set_pr(
        &release,
        "body",
        json!(format!(
            "Old release notes\n<!-- release-content-base: {old_base} -->"
        )),
    );
    release.origin(&["update-ref", "refs/heads/release-0.2.0", &repaired]);
    set_pr(&release, "mergeable", json!("MERGEABLE"));
    release.ci_reports(GREEN);

    let output = run(&release, &["0.2.0"], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired));
    assert!(fix_calls(&release).is_empty());
    assert!(
        release
            .pr_body("release-0.2.0")
            .contains("-version = \"0.1.1\"")
    );
}

#[test]
fn recovery_after_a_repaired_tag_resumes_between_branch_and_tag_pushes() {
    let release = Release::new();
    let (_, repaired) = conflicted_bump(&release);
    simulate_fix(&release, &adopt_script(&repaired));
    release.release_workflow(failed_release());
    let failed = run(&release, &["0.2.0"], &[]);
    assert_release_failed(&release, &failed, &repaired);
    release.merge_pr("newer-content", "Recovery content", "");
    release.main_ci("completed", "success");
    release.ci_reports(RED);
    simulate_fix(&release, "exit 1");
    let stopped = run(&release, &["0.2.0"], &[]);
    assert_eq!(stopped.status.code(), Some(1), "{}", stderr(&stopped));
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired));
    let head = release.pr_head("release-0.2.0");
    release.gh(&["fake", "checks", &head, GREEN]);
    release.release_workflow(json!({}));
    let resumed = run(&release, &["0.2.0"], &[]);
    assert!(resumed.status.success(), "{}", stderr(&resumed));
    assert_eq!(release.origin_tag("v0.2.0"), Some(head));
}

/// The Experiment is a yield: the cargo stand-in moves the release branch to
/// an exact bump of newer main and rewrites the notes marker to that base
/// before the script observes the pull request again.
fn forge_new_base_during_experiment(release: &Release, repaired: &str, new_base: &str) {
    experiments::cargo_reports(
        release,
        &[experiments::verdict("pass"), experiments::verdict("pass")],
    );
    let body = format!("Old release notes\n<!-- release-content-base: {new_base} -->");
    fs::write(
        release.root().join("during-experiment.sh"),
        format!(
            r#"set -euo pipefail
git --git-dir="${{FAKE_GH_STATE%/*}}/origin.git" update-ref refs/heads/release-0.2.0 {repaired}
gh fake pr release-0.2.0 mergeable '"MERGEABLE"'
gh fake pr release-0.2.0 body '{body}'
gh fake checks {repaired} '[{{"name": "test", "conclusion": "success"}}]'
"#,
            repaired = repaired,
            body = serde_json::to_string(&body).unwrap(),
        ),
    )
    .unwrap();
}

#[test]
fn a_base_change_during_the_experiment_is_measured_before_tagging() {
    let release = Release::new();
    let (old, repaired) = conflicted_bump(&release);
    let old_base = release.origin(&["rev-parse", &format!("{old}^")]);
    let new_base = release.origin(&["rev-parse", "main"]);
    assert_ne!(old_base, new_base);
    set_pr(
        &release,
        "body",
        json!(format!(
            "Old release notes\n<!-- release-content-base: {old_base} -->"
        )),
    );
    set_pr(&release, "mergeable", json!("MERGEABLE"));
    release.gh(&[
        "fake",
        "checks",
        &old,
        r#"[{"name": "test", "conclusion": "success"}]"#,
    ]);
    forge_new_base_during_experiment(&release, &repaired, &new_base);

    let output = run(&release, &["--with-experiment", "0.2.0"], &[]);

    let tag = release.origin_tag("v0.2.0");
    let calls = experiments::calls(&release);
    let measured: Vec<&str> = calls
        .iter()
        .filter_map(|call| call["args"].get(2).and_then(Value::as_str))
        .collect();
    // The hook rewrote the marker. Publishing that head anyway requires a
    // measurement of its base and notes that describe the new diff.
    assert!(
        release
            .pr_body("release-0.2.0")
            .contains(&format!("release-content-base: {new_base}"))
            || measured.contains(&new_base.as_str()),
        "experiment hook did not run: {}\n{}",
        release.pr_body("release-0.2.0"),
        stderr(&output)
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(tag.as_deref(), Some(repaired.as_str()));
    assert!(
        measured.contains(&new_base.as_str()),
        "tagged {repaired} under an experiment for {old_base}, not {new_base}: {measured:?}\n{}",
        stderr(&output)
    );
    assert!(
        release
            .pr_body("release-0.2.0")
            .contains("-version = \"0.1.1\""),
        "published repaired content with the pre-change notes: {}",
        release.pr_body("release-0.2.0")
    );
}

#[test]
fn an_unmarked_legacy_pr_repaired_before_resume_refreshes_its_notes() {
    let release = Release::new();
    let (_, repaired) = conflicted_bump(&release);
    set_pr(
        &release,
        "body",
        json!("Legacy notes: version diff starts at 0.1.0"),
    );
    release.origin(&["update-ref", "refs/heads/release-0.2.0", &repaired]);
    set_pr(&release, "mergeable", json!("MERGEABLE"));
    release.ci_reports(GREEN);
    let output = run(&release, &["0.2.0"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(repaired));
    assert!(
        release
            .pr_body("release-0.2.0")
            .contains("-version = \"0.1.1\""),
        "Published repaired legacy content with stale release notes: {}",
        release.pr_body("release-0.2.0")
    );
}

#[test]
fn green_ci_with_mergeability_stuck_unknown_stops_instead_of_waiting_forever() {
    let release = Release::new();
    release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    set_pr(&release, "mergeable", json!("UNKNOWN"));
    release.ci_reports(GREEN);
    let output = run(
        &release,
        &["0.2.0"],
        &[("RELEASE_WORKFLOW_STARTUP_SECONDS", "0")],
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("did not report the PR mergeable"),
        "{}",
        stderr(&output)
    );
    assert!(fix_calls(&release).is_empty());
    assert_eq!(release.origin_tag("v0.2.0"), None);
}

#[test]
fn a_pr_closed_during_ci_does_not_recommend_a_fix_run() {
    let release = Release::new();
    release.push_bump("0.2.0");
    release.open_bump_pr("0.2.0");
    release.gh(&[
        "fake",
        "on-ci-read",
        "1",
        "gh fake pr release-0.2.0 state '\"CLOSED\"'",
    ]);
    let output = run(&release, &["0.2.0"], &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        !stderr(&output).contains("left open"),
        "{}",
        stderr(&output)
    );
    assert!(
        !stderr(&output).contains("thirdshift fix"),
        "{}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("rerun scripts/release.sh 0.2.0"),
        "{}",
        stderr(&output)
    );
    assert_eq!(release.origin_tag("v0.2.0"), None);
}
