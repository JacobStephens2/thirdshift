use super::*;

/// A release tag on a bump branch, merged after concurrent main content.
/// The concurrent merge is green but does not descend from the release tag.
fn merge_previous_release(release: &Release) -> String {
    let bump = release.push_bump("0.2.0");
    git(&release.contributor(), &["tag", "v0.2.0", &bump]);
    git(&release.contributor(), &["push", "-q", "origin", "v0.2.0"]);
    release.open_bump_pr("0.2.0");
    release.merge_pr("concurrent", "Content shipped before the bump merged", "");
    release.main_ci("completed", "success");
    release.merge_bump_pr("0.2.0", &bump)
}

#[test]
fn a_release_waits_for_green_main_descending_from_the_previous_bump_tag() {
    let release = Release::new();
    let base = merge_previous_release(&release);
    release.main_ci_on(&base, "completed", "success", 3);
    release.ci_reports(GREEN);

    let output = release.run("0.3.0");

    let head = release.pr_head("release-0.3.0");
    assert_eq!(
        release.origin(&["rev-parse", &format!("{head}^1")]),
        base,
        "selected main content predating the previous release merge: {}",
        stderr(&output)
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("waiting for main push CI before releasing"));
    let body = release.pr_body("release-0.3.0");
    assert!(
        body.contains("-version = \"0.2.0\"\n+version = \"0.3.0\""),
        "{body}"
    );
    let calls = release.agent_calls();
    let prompt = calls.last().unwrap()["prompt"].as_str().unwrap();
    assert!(prompt.contains("\"pull_requests\""), "{prompt}");
    assert!(!prompt.contains("Add the frobnicator"), "{prompt}");
    assert_eq!(release.origin_tag("v0.3.0"), Some(head));
}

#[test]
fn a_saved_candidate_is_reselected_after_a_previous_release_merges() {
    let release = Release::new();
    let old = release.origin(&["rev-parse", "main"]);
    git(
        &release.maintainer(),
        &[
            "update-ref",
            "refs/thirdshift/release-candidates/0.3.0",
            &old,
        ],
    );
    let base = merge_previous_release(&release);
    release.main_ci_on(&base, "completed", "success", 2);
    release.ci_reports(GREEN);

    let output = release.run("0.3.0");

    let head = release.pr_head("release-0.3.0");
    assert_eq!(
        release.origin(&["rev-parse", &format!("{head}^1")]),
        base,
        "reused a saved candidate predating the previous release: {}",
        stderr(&output)
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(release.origin_tag("v0.3.0"), Some(head));
}

#[test]
fn an_untagged_branch_or_open_pr_is_recut_after_a_previous_release_merges() {
    for open in [false, true] {
        let release = Release::new();
        let old = release.push_bump("0.3.0");
        if open {
            release.open_bump_pr("0.3.0");
        }
        let base = merge_previous_release(&release);
        release.main_ci_on(&base, "completed", "success", 2);
        release.ci_reports(GREEN);

        let output = release.run("0.3.0");

        let head = release.pr_head("release-0.3.0");
        assert_eq!(
            release.origin(&["rev-parse", &format!("{head}^1")]),
            base,
            "open={open}: resumed a bump predating the previous release: {}",
            stderr(&output)
        );
        assert!(output.status.success(), "open={open}: {}", stderr(&output));
        assert_ne!(head, old);
        assert_eq!(release.prs_from("release-0.3.0").len(), 1);
        assert_eq!(release.pr("release-0.3.0")["state"], "MERGED");
        assert_eq!(release.origin_tag("v0.3.0"), Some(head.clone()));
        let body = release.pr_body("release-0.3.0");
        assert!(
            body.contains("-version = \"0.2.0\"\n+version = \"0.3.0\""),
            "{body}"
        );
        let calls = release.gh_calls();
        assert!(calls.iter().any(|call| {
            call.iter()
                .any(|arg| arg.contains(&format!("commits/{head}/check-runs")))
        }));
        assert!(!calls.iter().any(|call| {
            call.iter()
                .any(|arg| arg.contains(&format!("commits/{old}/check-runs")))
        }));
    }
}

#[test]
fn declining_review_leaves_an_untagged_branch_and_its_open_pr_unchanged() {
    for open in [false, true] {
        let release = Release::new();
        release.push_bump("0.3.0");
        if open {
            release.open_bump_pr("0.3.0");
        }
        let base = merge_previous_release(&release);
        release.main_ci_on(&base, "completed", "success", 0);
        let before = release.origin_refs();

        let output = release.run_on_terminal(&["--review", "0.3.0"], "n\n", &[]);

        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert_eq!(release.origin_refs(), before);
        assert_eq!(release.pr_opened("release-0.3.0"), open);
        if open {
            assert_eq!(
                release.pr_body("release-0.3.0"),
                "Summary from the first run"
            );
        }
        assert!(
            stderr(&output).contains("stopped at the review"),
            "{}",
            stderr(&output)
        );
        assert!(
            !release
                .gh_calls()
                .iter()
                .any(|call| call.starts_with(&["pr".into(), "edit".into()]))
        );
    }
}

#[test]
fn stale_pins_branches_and_prs_refuse_a_version_main_has_overtaken() {
    for stage in ["pin", "branch", "pr"] {
        let release = Release::new();
        if stage == "pin" {
            let old = release.origin(&["rev-parse", "main"]);
            git(
                &release.maintainer(),
                &[
                    "update-ref",
                    "refs/thirdshift/release-candidates/0.3.0",
                    &old,
                ],
            );
        } else {
            release.push_bump("0.3.0");
            if stage == "pr" {
                release.open_bump_pr("0.3.0");
            }
        }
        let previous = release.push_bump("0.4.0");
        release.open_bump_pr("0.4.0");
        let merge = release.merge_bump_pr("0.4.0", &previous);
        // This case covers a release tag landing directly on main.
        release.origin(&["tag", "v0.4.0", &merge]);
        release.main_ci("completed", "success");
        let before = release.origin_refs();

        let output = release.run("0.3.0");

        assert_eq!(
            output.status.code(),
            Some(1),
            "stage={stage}: {}",
            stderr(&output)
        );
        assert!(
            stderr(&output).contains(if stage == "pin" {
                "strictly greater than 0.4.0 on main"
            } else {
                // The last tag is main itself: no newer CI candidate exists
                // from which to re-cut this stale bump.
                "no candidate main commit"
            }),
            "stage={stage}: {}",
            stderr(&output)
        );
        assert_eq!(release.origin_refs(), before);
        if stage == "pr" {
            assert_eq!(
                release.pr_body("release-0.3.0"),
                "Summary from the first run"
            );
        }
    }
}
