use super::*;

#[test]
fn once_every_ticket_lands_the_spec_review_opens_the_delivery_and_the_checklist_goes_back_before_ready()
 {
    let mut outside = Scripted::new(Script {
        tickets: vec![open(21, &[])],
        ..Script::default()
    });

    let reached = reached_of(spec_run(&mut outside, 3));

    let done = checklist(&["[x] #21 landed"]);
    assert_eq!(
        outside.did_only(after_the_loop),
        [
            Did::Show(checklist(&["[ ] #21 running"])),
            Did::Landed(done.clone()),
            Did::Landed(done.clone()),
            spec_review_opening(),
            Did::PutBack(done),
            Did::Ready,
        ]
    );
    assert_eq!(reached.resolution.url(), SPEC_PR);
    assert_eq!(reached.log, Some(PathBuf::from(DELIVERY_LOG)));
    assert_eq!(reached.ticket_lines, ["#21 landed"]);
}

#[test]
fn a_failed_landing_means_no_delivery_and_carries_the_spec_prs_url_and_the_ticket_lines() {
    let mut outside = Scripted::new(Script {
        tickets: vec![open(21, &[]), open(22, &[])],
        endings: VecDeque::from([(21, Ok(failed("claude exited 1")))]),
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 1));

    assert_eq!(
        format!("{:#}", failed.error),
        "Tickets not done: #21 (failed: claude exited 1)"
    );
    assert_eq!(failed.pr_url.as_deref(), Some(SPEC_PR));
    assert_eq!(failed.log, Some(PathBuf::from("/logs/session.jsonl")));
    assert!(!failed.interrupted);
    assert_eq!(
        failed.ticket_lines,
        [
            "#21 failed: claude exited 1 (session trace: /logs/session.jsonl)",
            "#22 landed"
        ]
    );
    assert!(
        !outside
            .did
            .iter()
            .any(|did| matches!(did, Did::Deliver { .. }))
    );
}

#[test]
fn a_spec_pr_closed_under_the_spec_run_starts_no_further_ticket() {
    let mut outside = Scripted::new(Script {
        tickets: vec![open(21, &[]), open(22, &[21]), open(23, &[21])],
        // Checked before #21 starts, after it starts and as it lands,
        // then before #22 starts; closed once #22 has started.
        spec_pr_closed_at_check: Some(5),
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 1));

    assert_eq!(
        format!("{:#}", failed.error),
        format!("the Spec PR {SPEC_PR} was closed under this Spec run")
    );
    assert_eq!(
        outside.did_only(runs),
        [
            Did::Start(21),
            Did::Ended(21),
            Did::Start(22),
            Did::Ended(22)
        ]
    );
    assert!(
        !outside
            .did
            .iter()
            .any(|did| matches!(did, Did::Deliver { .. }))
    );
}

#[test]
fn a_spec_pr_closed_after_an_earlier_error_is_still_the_cause() {
    let mut outside = Scripted::new(Script {
        tickets: vec![open(21, &[]), open(22, &[])],
        endings: VecDeque::from([
            (21, Err(anyhow!("could not wait for #21"))),
            (22, Ok(reached())),
        ]),
        // Checked before each start and after both start; closed as #21
        // ends.
        spec_pr_closed_at_check: Some(4),
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 2));

    assert!(failed.error.is::<SpecPrChanged>(), "{:#}", failed.error);
}

#[test]
fn a_failed_landing_with_no_spec_pr_open_carries_no_pr_url() {
    let mut outside = Scripted::new(Script {
        tickets: vec![labelled(21, "needs-info")],
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 3));

    assert_eq!(failed.pr_url, None);
    assert_eq!(failed.ticket_lines, ["#21 unready: labelled needs-info"]);
}

#[test]
fn a_failed_landing_with_an_interrupt_requested_ends_as_interrupted() {
    // Found not interrupted before #21, the only time the loop asks once
    // its start failed; the interrupt comes as the landing fails.
    let mut outside = Scripted::new(Script {
        tickets: vec![open(21, &[])],
        spec_pr_open: true,
        start_fails: vec![21],
        interrupted_after: Some(1),
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 3));

    assert!(failed.interrupted);
    assert_eq!(format!("{:#}", failed.error), "interrupted");
    assert_eq!(failed.pr_url.as_deref(), Some(SPEC_PR));
    assert_eq!(failed.ticket_lines, ["#21 not started"]);
}

#[test]
fn a_spec_pr_that_cannot_be_opened_after_the_last_landing_means_no_delivery() {
    let mut outside = Scripted::new(Script {
        tickets: vec![closed(21)],
        landed_fails: true,
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 3));

    assert_eq!(format!("{:#}", failed.error), "could not open the Spec PR");
    assert_eq!(failed.pr_url, None);
    assert_eq!(
        outside.did_only(after_the_loop),
        [Did::Landed(checklist(&["[x] #21 done"]))]
    );
}

#[test]
fn a_failed_delivery_shows_the_checklist_again_and_keeps_its_pr_url_log_and_the_ticket_lines() {
    let mut outside = Scripted::new(Script {
        tickets: vec![open(21, &[])],
        delivery_fails: true,
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 3));

    let done = checklist(&["[x] #21 landed"]);
    assert_eq!(
        format!("{:#}", failed.error),
        "checks failed on the Spec PR"
    );
    assert_eq!(
        outside.did_only(after_the_loop),
        [
            Did::Show(checklist(&["[ ] #21 running"])),
            Did::Landed(done.clone()),
            Did::Landed(done.clone()),
            spec_review_opening(),
            Did::PutBack(done.clone()),
            Did::Ready,
            Did::Show(done),
        ]
    );
    assert_eq!(failed.pr_url.as_deref(), Some(SPEC_PR));
    assert_eq!(failed.log, Some(PathBuf::from(DELIVERY_LOG)));
    assert_eq!(failed.ticket_lines, ["#21 landed"]);
}

#[test]
fn a_put_back_that_fails_fails_the_delivery_before_ready() {
    let mut outside = Scripted::new(Script {
        tickets: vec![closed(21)],
        put_back_fails: true,
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 3));

    let done = checklist(&["[x] #21 done"]);
    assert_eq!(
        format!("{:#}", failed.error),
        "could not put the Tickets checklist back"
    );
    assert_eq!(
        outside.did_only(after_the_loop),
        [
            Did::Landed(done.clone()),
            spec_review_opening(),
            Did::PutBack(done.clone()),
            Did::Show(done),
        ]
    );
}

#[test]
fn a_delivery_that_fails_in_the_spec_review_shows_the_checklist_again_without_a_put_back() {
    let mut outside = Scripted::new(Script {
        tickets: vec![closed(21)],
        review_fails: true,
        ..Script::default()
    });

    let failed = failed_of(spec_run(&mut outside, 3));

    let done = checklist(&["[x] #21 done"]);
    assert_eq!(
        format!("{:#}", failed.error),
        "the spec-review session failed"
    );
    assert_eq!(
        outside.did_only(after_the_loop),
        [
            Did::Landed(done.clone()),
            spec_review_opening(),
            Did::Show(done)
        ]
    );
}
