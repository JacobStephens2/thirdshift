use super::*;

#[test]
fn a_failed_unpublished_release_is_recut_from_the_newest_green_main_commit() {
    let release = Release::failed_unpublished();
    let old = release.pr_head("release-0.2.0");
    release.merge_pr("recovery", "Content added after the failure", "");
    let green = release.origin(&["rev-parse", "main"]);
    release.main_ci("completed", "success");
    release.merge_pr("red-tip", "Latest CI supersedes success", "");
    release.main_ci("completed", "success");
    release.main_ci("completed", "failure");
    release.release_workflow(json!({"pending_polls": 2}));
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    let head = release.pr_head("release-0.2.0");
    assert_ne!(head, old);
    assert_eq!(release.origin(&["rev-parse", &format!("{head}^1")]), green);
    assert_eq!(release.origin_tag("v0.2.0"), Some(head));
    assert_eq!(release.prs_from("release-0.2.0").len(), 1);
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    assert_eq!(release.agent_calls().len(), 2);
    assert!(release.pr_body("release-0.2.0").contains(AGENT_SUMMARY));
    let calls = release.gh_calls();
    let edit = calls
        .iter()
        .position(|call| call.starts_with(&["pr".into(), "edit".into()]))
        .unwrap();
    let checks = calls
        .iter()
        .rposition(|call| {
            call.iter()
                .any(|arg| arg.contains("actions/runs?head_sha="))
        })
        .unwrap();
    assert!(edit < checks);
    assert_eq!(release.release_run()["pending_polls"], 0);
    assert!(stderr(&output).contains(&release_url()));
}

#[test]
fn a_release_view_outage_does_not_move_a_published_tag() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let mut failed = failed_release();
    failed["published"] = json!(true);
    release.release_workflow(failed);
    let first = release.run_script("0.2.0");
    assert_eq!(first.status.code(), Some(1), "{}", stderr(&first));
    let head = release.pr_head("release-0.2.0");
    let branch = release.origin(&["rev-parse", "refs/heads/release-0.2.0"]);
    release.merge_pr("newer", "New content after publication", "");
    release.main_ci("completed", "success");
    release.ci_reports(GREEN);
    // A transport failure is not "release not found". The tag is published.
    release.gh(&["fake", "fails", "release view"]);

    let output = release.run_script("0.2.0");

    assert_eq!(
        release.origin_tag("v0.2.0"),
        Some(head),
        "published tag moved: {}",
        stderr(&output)
    );
    assert_eq!(
        release.origin(&["rev-parse", "refs/heads/release-0.2.0"]),
        branch,
        "published branch moved: {}",
        stderr(&output)
    );
    assert_ne!(output.status.code(), Some(0), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(
        err.contains("could not tell whether v0.2.0 is already published"),
        "{err}"
    );
    assert!(!err.contains("moved unpublished"), "{err}");
}

#[test]
fn a_published_release_is_never_recut_even_after_a_workflow_failure() {
    let release = Release::new();
    release.ci_reports(GREEN);
    let mut failed = failed_release();
    failed["published"] = json!(true);
    release.release_workflow(failed);
    let first = release.run_script("0.2.0");
    assert_eq!(first.status.code(), Some(1), "{}", stderr(&first));
    let head = release.pr_head("release-0.2.0");
    release.merge_pr("newer", "New content after publication", "");
    release.main_ci("completed", "success");

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(head.clone()));
    assert_eq!(release.pr_head("release-0.2.0"), head);
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    assert_eq!(release.agent_calls().len(), 1);
    assert!(
        !release
            .gh_calls()
            .iter()
            .any(|call| call.starts_with(&["run".into(), "rerun".into()]))
    );
}

#[test]
fn an_immediately_failed_recovery_attempt_leaves_the_bump_pr_open() {
    let release = Release::failed_unpublished();
    let head = release.pr_head("release-0.2.0");
    let main = release.origin(&["rev-parse", "main"]);

    let output = release.run_script("0.2.0");

    assert_release_failed(&release, &output, &head);
    assert_eq!(release.release_run()["attempt"], 2);
    assert_eq!(release.pr("release-0.2.0")["state"], "OPEN");
    assert_eq!(release.origin(&["rev-parse", "main"]), main);
}

#[test]
fn recovery_resumes_after_the_recut_bump_checks_failed() {
    let release = Release::failed_unpublished();
    let old = release.pr_head("release-0.2.0");
    release.merge_pr("newer", "New recovery content", "");
    release.main_ci("completed", "success");
    release.ci_reports(RED);
    let stopped = release.run_script("0.2.0");
    assert_eq!(stopped.status.code(), Some(1), "{}", stderr(&stopped));
    let head = release.pr_head("release-0.2.0");
    assert_ne!(head, old);
    assert_eq!(release.origin_tag("v0.2.0"), Some(old));
    release.gh(&["fake", "checks", &head, GREEN]);
    release.release_workflow(json!({}));

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(head));
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
}

#[test]
fn a_pending_release_attempt_finishes_before_the_open_bump_pr_is_merged() {
    let release = Release::failed_unpublished();
    let head = release.pr_head("release-0.2.0");
    release.gh(&[
        "fake",
        "run",
        &json!({
            "databaseId": release.release_run()["databaseId"],
            "conclusion": "success", "pending_polls": 2,
        })
        .to_string(),
    ]);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.2.0"), Some(head));
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    assert_eq!(release.release_run()["pending_polls"], 0);
    let err = unstamped(&output);
    assert!(
        err.find("waiting for the release workflow at").unwrap()
            < err.find("release: merged").unwrap()
    );
}

#[test]
fn an_old_merged_bump_can_be_recut_with_a_new_bump_pr() {
    let release = Release::failed_unpublished();
    let old = release.pr_head("release-0.2.0");
    release.merge_bump_pr("0.2.0", &old);
    release.merge_pr("newer", "Main advanced after the old bump was merged", "");
    release.main_ci("completed", "success");
    let candidate = release.origin(&["rev-parse", "main"]);
    release.release_workflow(json!({}));
    release.ci_reports(GREEN);

    let output = release.run_script("0.2.0");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.prs_from("release-0.2.0").len(), 2);
    let head = release.pr_head("release-0.2.0");
    assert_eq!(
        release.origin(&["rev-parse", &format!("{head}^1")]),
        candidate
    );
    assert_eq!(release.origin_tag("v0.2.0"), Some(head));
    assert_eq!(release.pr("release-0.2.0")["state"], "MERGED");
    let calls = release.agent_calls();
    let prompt = calls[1]["prompt"].as_str().unwrap();
    assert!(
        prompt.contains("Pull requests merged since v0.1.0"),
        "{prompt}"
    );
    assert!(prompt.contains("Add the frobnicator"), "{prompt}");
    assert!(prompt.contains("Tidy the site"), "{prompt}");
    assert!(
        prompt.contains("Main advanced after the old bump was merged"),
        "{prompt}"
    );
}
