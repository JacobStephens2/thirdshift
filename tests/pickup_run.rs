//! Pickup runs: `thirdshift pickup` takes the lowest-numbered Ready issue in
//! the repository, an open issue labelled `ready-for-agent` with no label
//! that makes an Unready Ticket, no Claim and no `base-fix` label, that is
//! not a sub-issue, has no open blocker and was never started or was handed
//! back after its last Claim ended, is not a
//! Spec whose Tickets are all closed, and has settled, and
//! dispatches it as `thirdshift <Issue URL>` would, a Spec run or a Run,
//! ending as that does. Each `ready-for-agent` issue it passes over on the
//! way gets a line saying why. With no Ready issue, with the repository at
//! its Failure limit, or while an Architect run or another Pickup run on the
//! repository is still running, it is skipped. Each pass that gets the lock
//! first sweeps `in-progress` and `agent-failed` off the repository's closed
//! issues, then takes `agent-failed` off each open issue the Day shift has
//! acted on.
//! Asked for a Run notification, a pass that took an issue sends one naming
//! the Pickup run and the kind of run it dispatched; a skipped one sends none.

#[path = "pickup_run/base_ci.rs"]
mod base_ci;
mod support;

use std::fs;

use support::resend::{ResendStandIn, assert_one_issue_line, assert_saved_notification};
use support::{
    HeldRun, REPO, RunResult, Scenario, TimelineEvent, before_command_log, write_executable,
};

/// The label of an issue a Pickup run may take.
const READY_FOR_AGENT: &str = "ready-for-agent";

/// The label of a Claimed issue, which the dispatched run swaps the issue's
/// `ready-for-agent` for.
const IN_PROGRESS: &str = "in-progress";

/// The label of an issue a failed run kept its Claim on, which the Failure
/// limit counts.
const AGENT_FAILED: &str = "agent-failed";

/// Make issue `number` open and labelled `ready-for-agent` on the fake
/// GitHub, with `more` labels after it.
fn ready_issue(scenario: &Scenario, number: u32, more: &[&str]) {
    scenario.issue_is(number, "OPEN");
    scenario.issue_labelled(number, &[&[READY_FOR_AGENT], more].concat());
}

/// A script in which the agent for issue `issue` commits its work and opens
/// its PR into `base`, leaving the pushing to thirdshift.
fn agent_opens_pr(issue: u32, base: &str) -> String {
    format!(
        r#"
echo "{issue}" > issue-{issue}.txt
git add issue-{issue}.txt
git commit -q -m "Work on {issue}"
gh pr create --base {base} --head issue-{issue} --title "Work on {issue}" --body "Closes #{issue}"
"#
    )
}

/// A successful diagnosis followed by PR delivery for a bug-labelled issue.
fn agent_fixes_bug(issue: u32, base: &str) -> String {
    format!(
        "{}echo 'Bug reproduced: fixed' > \"$FAKE_CLAUDE_FINAL_MESSAGE\"\n",
        agent_opens_pr(issue, base),
    )
}

/// Keep fault scripts at their call sites while sharing wrapper installation.
fn install_git_wrapper(scenario: &Scenario, script: impl FnOnce(&str) -> String) {
    let git = std::process::Command::new("which")
        .arg("git")
        .output()
        .unwrap();
    assert!(git.status.success());
    let git = String::from_utf8(git.stdout).unwrap();
    write_executable(&scenario.path("bin/git"), &script(git.trim()));
}

/// The newest pull request from `head` on the fake GitHub.
fn pr_from(scenario: &Scenario, head: &str) -> serde_json::Value {
    let prs = scenario.gh_state()["prs"].clone();
    let from_head = prs
        .as_array()
        .unwrap()
        .iter()
        .rfind(|pr| pr["head"] == head);
    from_head
        .unwrap_or_else(|| panic!("no PR from {head}: {prs}"))
        .clone()
}

/// `stderr` after the line saying the Pickup run is starting.
fn after_start(stderr: &str) -> &str {
    support::after_start(stderr, "Pickup run starting")
}

/// Assert the Pickup run ended with `pr` in `outcome`, as a Run or a Spec
/// run that reached its goal does: exit 0, the PR's URL alone on stdout, and
/// the outcome as the last line on stderr.
fn assert_ended_with_pr(result: &RunResult, pr: &serde_json::Value, outcome: &str) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    let url = pr["url"].as_str().unwrap();
    assert_eq!(result.stdout, format!("{url}\n"));
    assert_eq!(
        result.stderr.lines().last(),
        Some(format!("thirdshift: PR {url} is {outcome}").as_str()),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn the_lower_numbered_of_two_ready_issues_is_run_and_the_other_left_untouched() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 9, &["bug"]);
    ready_issue(&scenario, 7, &["bug"]);
    scenario.agent_does_for(7, &agent_fixes_bug(7, "main"));

    let result = scenario.run(&["pickup"]);

    let pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &pr, "ready for review");
    assert_eq!(pr["base"], "main");
    assert_eq!(pr["state"], "OPEN");
    assert_eq!(scenario.issue_labels(7), ["bug", IN_PROGRESS]);
    assert_eq!(scenario.issue_labels(9), [READY_FOR_AGENT, "bug"]);
    assert!(scenario.origin_log("issue-9").is_none());
    let calls = scenario.claude_calls();
    assert_eq!(calls.len(), 1, "sessions: {calls:?}");
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with(&format!(
            "/thirdshift-diagnosing-bugs {}\n",
            scenario.issue_url(7)
        )),
        "{prompt}"
    );
    scenario.assert_cleaned_up("issue-7");
    assert_eq!(
        scenario.origin_git(&["for-each-ref", "refs/heads/thirdshift/pickup"]),
        ""
    );
}

/// With Ready issues #7 and #9, `pickup` with `extra` takes `taken` and
/// leaves `left` labelled `ready-for-agent` with no branch on origin.
fn pickup_takes(extra: &[&str], taken: u32, left: u32) {
    let scenario = Scenario::new();
    ready_issue(&scenario, 9, &["bug"]);
    ready_issue(&scenario, 7, &["bug"]);
    scenario.agent_does_for(taken, &agent_fixes_bug(taken, "main"));

    let mut command = vec!["pickup"];
    command.extend(extra);
    let result = scenario.run(&command);

    let pr = pr_from(&scenario, &format!("issue-{taken}"));
    assert_ended_with_pr(&result, &pr, "ready for review");
    assert_eq!(
        scenario.issue_labels(left),
        [READY_FOR_AGENT, "bug"],
        "extra: {extra:?}"
    );
    assert!(scenario.origin_log(&format!("issue-{left}")).is_none());
    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "sessions: {:?}",
        scenario.claude_calls()
    );
}

#[test]
fn the_higher_numbered_of_two_ready_issues_is_run_with_highest_first() {
    pickup_takes(&["highest-first"], 9, 7);
}

#[test]
fn the_lower_numbered_of_two_ready_issues_is_run_with_lowest_first() {
    pickup_takes(&["lowest-first"], 7, 9);
}

#[test]
fn pickup_order_in_the_user_config_sets_which_ready_issue_is_run() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 9, &["bug"]);
    ready_issue(&scenario, 7, &["bug"]);
    scenario.agent_does_for(9, &agent_fixes_bug(9, "main"));
    scenario.user_config_is("[pickup]\norder = \"highest-first\"\n");

    let result = scenario.run(&["pickup"]);

    let pr = pr_from(&scenario, "issue-9");
    assert_ended_with_pr(&result, &pr, "ready for review");
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT, "bug"]);
}

#[test]
fn a_pickup_order_flag_beats_pickup_order_in_the_user_config() {
    for (configured, flag, taken, left) in [
        ("lowest-first", "highest-first", 9, 7),
        ("highest-first", "lowest-first", 7, 9),
    ] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 9, &["bug"]);
        ready_issue(&scenario, 7, &["bug"]);
        scenario.agent_does_for(taken, &agent_fixes_bug(taken, "main"));
        scenario.user_config_is(&format!("[pickup]\norder = \"{configured}\"\n"));

        let result = scenario.run(&["pickup", flag]);

        let pr = pr_from(&scenario, &format!("issue-{taken}"));
        assert_ended_with_pr(&result, &pr, "ready for review");
        assert_eq!(
            scenario.issue_labels(left),
            [READY_FOR_AGENT, "bug"],
            "configured {configured}, flag {flag}"
        );
    }
}

#[test]
fn an_invalid_pickup_order_stops_the_pass_naming_the_file_and_key() {
    for order in ["\"lowest\"", "\"highest\"", "\"\"", "\"Lowest-First\""] {
        let scenario = ready_ticket();
        let path = scenario.user_config_is(&format!("[pickup]\norder = {order}\n"));

        let result = scenario.run(&["pickup"]);

        scenario.assert_rejected_before_any_work(
            &result,
            &format!(
                "thirdshift: pickup.order must be lowest-first or highest-first, not {order}, in the User config {}\n",
                path.display()
            ),
        );
        assert_eq!(result.code, Some(1), "{order}: {}", result.stderr);
        assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    }
    for order in ["true", "3"] {
        let scenario = ready_ticket();
        let path = scenario.user_config_is(&format!("[pickup]\norder = {order}\n"));

        let result = scenario.run(&["pickup"]);

        scenario.assert_rejected_before_any_work(
            &result,
            &format!(
                "thirdshift: pickup.order must be a quoted order in the User config {}\n",
                path.display()
            ),
        );
        assert_eq!(result.code, Some(1), "{order}: {}", result.stderr);
        assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    }
}

#[test]
fn a_rejecting_pre_push_hook_does_not_block_reserving_or_releasing_a_ready_issue() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.launch_has_rejecting_pre_push_hook();
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));

    let result = scenario.run(&["pickup"]);

    let pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &pr, "ready for review");
    assert_eq!(scenario.claude_calls().len(), 1, "{}", result.stderr);
    assert_eq!(
        scenario.origin_git(&["for-each-ref", "refs/heads/thirdshift/pickup"]),
        "",
        "{}",
        result.stderr
    );
}

#[test]
fn two_machines_racing_for_a_ready_issue_start_exactly_one_run() {
    for race in [
        PickupRace::Issue,
        PickupRace::Spec,
        PickupRace::SuccessfulWinner,
    ] {
        pickup_race(race);
    }
}

#[test]
fn delayed_selector_after_failed_winner_still_starts_only_one_run() {
    pickup_race(PickupRace::FailedWinner);
}

#[test]
fn stale_listing_must_not_adopt_an_active_claim_generation() {
    pickup_race(PickupRace::ActiveClaimHistory);
}

enum PickupRace {
    Issue,
    Spec,
    SuccessfulWinner,
    FailedWinner,
    ActiveClaimHistory,
}

fn pickup_race(race: PickupRace) {
    let (is_spec, winner_finishes_first, winner_fails, active_history) = match race {
        PickupRace::Issue => (false, false, false, false),
        PickupRace::Spec => (true, false, false, false),
        PickupRace::SuccessfulWinner => (false, true, false, false),
        PickupRace::FailedWinner => (false, true, true, false),
        PickupRace::ActiveClaimHistory => (false, true, true, true),
    };
    let scenario = Scenario::new();
    // The generation check must protect even a repository that disables
    // settling, regardless of timestamp precision or either host's clock.
    scenario.user_config_is("[pickup]\nwait_minutes = 0\n");
    ready_issue(&scenario, 7, &[]);
    if is_spec {
        scenario.spec_has_tickets(7, &[(8, &[])]);
    }
    fs::create_dir_all(scenario.path("machine-2")).unwrap();
    scenario.launch_git(&[
        "clone",
        "-q",
        &scenario.github_url(),
        scenario.path("machine-2/widgets").to_str().unwrap(),
    ]);
    fs::create_dir_all(scenario.path("other-home")).unwrap();
    fs::copy(
        scenario.path("home/.gitconfig"),
        scenario.path("other-home/.gitconfig"),
    )
    .unwrap();
    fs::create_dir_all(scenario.path("other-home/.thirdshift")).unwrap();
    fs::write(
        scenario.path("other-home/.thirdshift/config.toml"),
        "[pickup]\nwait_minutes = 0\n",
    )
    .unwrap();

    // Both hosts read Ready before either makes its Claim. Keep the first
    // host's empty origin listing even after the second has started work.
    install_git_wrapper(&scenario, |git| {
        format!(
            r#"#!/bin/sh
if [ "$HOLD_READY_READ" = 1 ] && [ "$*" = 'ls-remote --heads origin refs/heads/issue-7 refs/heads/issue-7-branch-*' ] && [ ! -f "$TMPDIR/ready-read" ]; then
    "{git}" "$@" > "$TMPDIR/ready-answer" || exit $?
    touch "$TMPDIR/ready-read"
    while [ -d "$TMPDIR" ] && [ ! -f "$TMPDIR/release-ready" ]; do sleep 0.02; done
    cat "$TMPDIR/ready-answer"
    exit 0
fi
exec "{git}" "$@"
"#,
        )
    });
    let ending = if winner_finishes_first && !winner_fails {
        agent_opens_pr(7, "main")
    } else {
        "exit 1\n".to_string()
    };
    scenario.agent_does(&format!("if [ \"$HOLD_READY_READ\" = 1 ]; then exit 1; fi\ntouch \"$TMPDIR/winner-started\"\nwhile [ -d \"$TMPDIR\" ] && [ ! -f \"$TMPDIR/finish-winner\" ]; do sleep 0.02; done\n{ending}"));

    if active_history {
        // Preserve the old listing, then preserve the candidate response
        // sampled while the winner's Claim is active. This barrier lets a
        // correct search reject that history before any Issue branch read.
        fs::create_dir_all(scenario.path("bin/real")).unwrap();
        fs::rename(scenario.path("bin/gh"), scenario.path("bin/real/gh")).unwrap();
        write_executable(
            &scenario.path("bin/gh"),
            r#"#!/bin/sh
if [ "$HOLD_READY_READ" = 1 ]; then
    case "$1 $2 $*" in
    'api --paginate '*ready-for-agent*)
        if [ ! -f "$TMPDIR/listing-read" ]; then
            "$(dirname "$0")/real/gh" "$@" > "$TMPDIR/listing-answer" || exit $?
            touch "$TMPDIR/listing-read"
            while [ -d "$TMPDIR" ] && [ ! -f "$TMPDIR/release-listing" ]; do sleep 0.02; done
            cat "$TMPDIR/listing-answer"
            exit 0
        fi
        ;;
    'api graphql '*timelineItems*)
        if [ ! -f "$TMPDIR/candidate-read" ]; then
            "$(dirname "$0")/real/gh" "$@" > "$TMPDIR/candidate-answer" || exit $?
            touch "$TMPDIR/candidate-read"
            while [ -d "$TMPDIR" ] && [ ! -f "$TMPDIR/release-candidate" ]; do sleep 0.02; done
            cat "$TMPDIR/candidate-answer"
            exit 0
        fi
        ;;
    esac
fi
exec "$(dirname "$0")/real/gh" "$@"
"#,
        );
    }
    let started = if active_history {
        "tmp/listing-read"
    } else {
        "tmp/ready-read"
    };
    let stale = scenario.run_until(&["pickup"], &[("HOLD_READY_READ", "1")], started);
    let other_home = scenario.path("other-home");
    let winner = std::panic::catch_unwind(|| {
        scenario.run_from_until(
            &scenario.path("machine-2/widgets"),
            &["pickup"],
            &[("HOME", other_home.to_str().unwrap())],
            "tmp/winner-started",
        )
    });
    let winner = match winner {
        Ok(winner) => winner,
        Err(panic) => {
            fs::write(scenario.path("tmp/release-ready"), "").unwrap();
            fs::write(scenario.path("tmp/release-listing"), "").unwrap();
            fs::write(scenario.path("tmp/release-candidate"), "").unwrap();
            fs::write(scenario.path("tmp/finish-winner"), "").unwrap();
            stale.finish();
            std::panic::resume_unwind(panic);
        }
    };
    let candidate_seen = if active_history {
        fs::write(scenario.path("tmp/release-listing"), "").unwrap();
        let deadline = std::time::Instant::now() + support::WAIT_BOUND;
        while !scenario.path("tmp/candidate-read").exists() && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        scenario.path("tmp/candidate-read").exists()
    } else {
        true
    };
    let mut winner = Some(winner);
    let winner_result = if winner_finishes_first {
        fs::write(scenario.path("tmp/finish-winner"), "").unwrap();
        Some(winner.take().unwrap().finish())
    } else {
        None
    };
    let labels = scenario.issue_labels(7);
    let branches = scenario.origin_git(&["show-ref"]);
    let prs = scenario.gh_state()["prs"].clone();
    fs::write(scenario.path("tmp/release-ready"), "").unwrap();
    fs::write(scenario.path("tmp/release-candidate"), "").unwrap();
    let stale = stale.finish();
    let labels_after = scenario.issue_labels(7);
    let branches_after = scenario.origin_git(&["show-ref"]);
    let prs_after = scenario.gh_state()["prs"].clone();
    fs::write(scenario.path("tmp/finish-winner"), "").unwrap();
    if let Some(winner) = winner {
        winner.finish();
    }

    assert!(
        candidate_seen,
        "stale selector never sampled active Claim history"
    );
    assert_eq!(stale.code, Some(0), "{}", stale.stderr);
    if let Some(winner) = winner_result {
        assert_eq!(
            winner.code,
            Some(if winner_fails { 1 } else { 0 }),
            "{}",
            winner.stderr
        );
    }
    assert!(stale.stdout.is_empty(), "{}", stale.stdout);
    assert_eq!(scenario.claude_calls().len(), 1, "{}", stale.stderr);
    assert_eq!(labels_after, labels);
    assert_eq!(branches_after, branches);
    assert_eq!(prs_after, prs);
}

#[test]
fn pickup_reservation_records_the_host_holding_it() {
    let scenario = Scenario::new();
    let hostname = std::process::Command::new("hostname").output().unwrap();
    assert!(hostname.status.success());
    let hostname = String::from_utf8(hostname.stdout).unwrap();
    let held = pickup_run_held_on_issue_7(&scenario);
    let message = scenario.origin_git(&[
        "show",
        "-s",
        "--format=%B",
        "refs/heads/thirdshift/pickup/7",
    ]);
    release(&scenario);
    held.finish();

    assert!(
        message
            .lines()
            .any(|line| line == format!("Host: {}", hostname.trim())),
        "reservation did not identify its host: {message}"
    );
}

#[test]
fn pickup_reservation_does_not_publish_unrelated_local_head() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.launch_git(&["checkout", "-qb", "private-local-work"]);
    scenario.commit_locally(
        "private-local.txt",
        "local-only-content",
        "Unpublished local work",
    );
    scenario.agent_does(AGENT_WAITS_FOR_RELEASE);
    let held = scenario.run_until(&["pickup", "base", "main"], &[], "started");
    let published = scenario.origin_git(&["ls-tree", "-r", "refs/heads/thirdshift/pickup/7"]);
    release(&scenario);
    held.finish();
    assert!(
        !published.contains("private-local.txt"),
        "reservation published local file: {published}"
    );
}

#[test]
fn a_legacy_reservation_without_a_host_is_not_stolen() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let tree = scenario.launch_git(&["mktree"]);
    let token = scenario.launch_git(&[
        "commit-tree",
        tree.trim(),
        "-m",
        "thirdshift Pickup reservation for https://github.com/acme/widgets/issues/7\n\nToken: old-token\nLaunch: /old/widgets\nProcess: 123\nStarted: 2026-10-01 00:00:00 UTC",
    ]);
    scenario.launch_git(&[
        "push",
        "origin",
        &format!("{}:refs/heads/thirdshift/pickup/7", token.trim()),
    ]);
    let before = scenario.origin_git(&["show-ref"]);
    let github = scenario.gh_state();

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        result.stderr.contains("reserved by another Pickup run"),
        "{}",
        result.stderr
    );
    assert!(result.stdout.is_empty());
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(scenario.origin_git(&["show-ref"]), before);
    assert_eq!(scenario.gh_state(), github);
}

#[test]
fn ending_a_pickup_does_not_delete_a_replaced_reservation() {
    let scenario = Scenario::new();
    let held = pickup_run_held_on_issue_7(&scenario);
    let replacement = scenario.origin_git(&["rev-parse", "main"]);
    scenario.origin_git(&[
        "update-ref",
        "refs/heads/thirdshift/pickup/7",
        replacement.trim(),
    ]);

    release(&scenario);
    let result = held.finish();

    assert!(
        result
            .stderr
            .contains("could not release Pickup reservation"),
        "{}",
        result.stderr
    );
    assert_eq!(
        scenario.origin_git(&["rev-parse", "refs/heads/thirdshift/pickup/7"]),
        replacement
    );
}

#[test]
fn a_reservation_push_rejection_fails_before_any_claim_or_session() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let hook = scenario.origin_dir().join("hooks/pre-receive");
    write_executable(
        &hook,
        "#!/bin/sh\necho 'reservations forbidden' >&2\nexit 1\n",
    );
    let before = scenario.origin_git(&["show-ref"]);
    let github = scenario.gh_state();

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("could not reserve refs/heads/thirdshift/pickup/7"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(scenario.origin_git(&["show-ref"]), before);
    assert_eq!(scenario.gh_state(), github);
}

#[test]
fn an_uncertain_successful_push_retains_the_reservation_without_dispatching() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    install_git_wrapper(&scenario, |git| {
        format!(
            r#"#!/bin/sh
if [ "$1" = push ]; then
    "{git}" "$@" || exit $?
    echo 'lost push acknowledgement' >&2
    exit 1
fi
exec "{git}" "$@"
"#,
        )
    });

    let failed = scenario.run(&["pickup"]);
    let reservation = scenario.origin_git(&["rev-parse", "refs/heads/thirdshift/pickup/7"]);
    let retried = scenario.run(&["pickup"]);

    assert_eq!(failed.code, Some(1), "{}", failed.stderr);
    assert!(
        failed.stderr.contains("check origin by hand"),
        "{}",
        failed.stderr
    );
    assert_eq!(retried.code, Some(0), "{}", retried.stderr);
    assert!(retried.stderr.contains("reserved by another Pickup run"));
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    assert_eq!(
        scenario.origin_git(&["rev-parse", "refs/heads/thirdshift/pickup/7"]),
        reservation
    );
    assert!(scenario.origin_log("issue-7").is_none());
}

#[test]
fn an_interrupted_pickup_releases_its_reservation_after_ending_its_claim() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does(AGENT_WAITS_FOR_RELEASE);
    let mut held = scenario.run_until(&["pickup"], &[], "started");
    assert!(
        !scenario
            .origin_git(&["show-ref", "--heads", "thirdshift/pickup/7"])
            .is_empty()
    );

    held.signal("INT");
    release(&scenario);
    let result = held.finish();

    assert_ne!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(
        scenario.origin_git(&["for-each-ref", "refs/heads/thirdshift/pickup"]),
        ""
    );
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
}

/// What a skipped Pickup run says when the scenario's repository has no
/// Ready issue.
const NO_READY_ISSUE: &str = "thirdshift: no Ready issue on acme/widgets\n";

/// What a Pickup run says when it passed over each `ready-for-agent` issue
/// with one of `lines`, in order, and so found no Ready issue.
fn no_ready_issue_after(lines: &[&str]) -> String {
    let lines: String = lines
        .iter()
        .map(|line| format!("thirdshift: {line}\n"))
        .collect();
    lines + NO_READY_ISSUE
}

/// Assert the Pickup run was skipped with `reason` as its one line: exit 0,
/// that line alone on stderr after the one saying it is starting, nothing on stdout, and no session started.
fn assert_skipped(scenario: &Scenario, result: &RunResult, reason: &str) {
    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(after_start(&result.stderr), reason);
    assert_eq!(result.stdout, "");
    assert!(scenario.claude_calls().is_empty(), "a session was started");
}

#[test]
fn an_issue_without_ready_for_agent_is_never_taken_and_with_no_ready_issue_the_pass_is_skipped() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["bug"]);
    scenario.issue_is(8, "OPEN");
    let github = scenario.gh_state();

    let result = scenario.run(&["pickup"]);

    assert_skipped(&scenario, &result, NO_READY_ISSUE);
    assert_eq!(scenario.gh_state(), github);
}

#[test]
fn a_closed_issue_labelled_ready_for_agent_is_not_taken() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.issue_is(7, "CLOSED");

    let result = scenario.run(&["pickup"]);

    assert_skipped(&scenario, &result, NO_READY_ISSUE);
}

#[test]
fn an_issue_that_also_has_a_label_that_makes_an_unready_ticket_is_not_taken() {
    // Whatever its case: GitHub's label names are case-insensitive.
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &["Needs-Info"]);

    let result = scenario.run(&["pickup"]);

    // The line names the label as thirdshift spells it.
    let passed_over = no_ready_issue_after(&["#7 labelled needs-info"]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT, "Needs-Info"]);
}

#[test]
fn an_issue_with_an_issue_branch_on_origin_is_not_taken() {
    for branch in ["issue-7", "issue-7-branch-2"] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.origin_has_branch(branch, "main", &["Earlier work"]);

        let result = scenario.run(&["pickup"]);

        let line = format!(
            "#7 started: {branch} is on origin; no ready-for-agent hand-back after the last Claim ended"
        );
        assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
        assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT], "{branch}");
    }
}

#[test]
fn an_issue_with_a_pull_request_from_an_issue_branch_in_any_state_is_not_taken() {
    for state in ["OPEN", "MERGED", "CLOSED"] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        let pr = scenario.github_has_pr("issue-7", "main", state);

        let result = scenario.run(&["pickup"]);

        let line =
            format!("#7 started: PR {pr}; no ready-for-agent hand-back after the last Claim ended");
        assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
        assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT], "{state}");
    }
}

#[test]
fn when_a_later_issue_is_ready_the_earlier_ones_lines_are_printed_and_it_is_taken() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let pr = scenario.github_has_pr("issue-7", "main", "CLOSED");
    ready_issue(&scenario, 8, &["wontfix"]);
    ready_issue(&scenario, 9, &[IN_PROGRESS]);
    // Neither #7's Issue branch nor its pull request is one of #70's.
    ready_issue(&scenario, 70, &[]);
    scenario.agent_does_for(70, &agent_opens_pr(70, "main"));
    // Never looked at: the pass ends with the first Ready issue.
    ready_issue(&scenario, 71, &["needs-info"]);

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-70"), "ready for review");
    let url = scenario.issue_url(70);
    let lines = format!(
        "thirdshift: #7 started: PR {pr}; no ready-for-agent hand-back after the last Claim ended\n\
         thirdshift: #8 labelled wontfix\n\
         thirdshift: #9 labelled in-progress\n\
         thirdshift: taking Ready issue #70 \"Issue 70\", as thirdshift {url} would\n"
    );
    assert!(
        after_start(&result.stderr).starts_with(&lines),
        "stderr: {}",
        result.stderr
    );
    assert!(!result.stderr.contains("#71"), "stderr: {}", result.stderr);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    assert_eq!(scenario.issue_labels(70), [IN_PROGRESS]);
}

#[test]
fn a_spec_that_is_a_ready_issue_is_taken_and_its_tickets_get_no_lines() {
    // #7 is a Ticket of the Spec #20, numbered below it, and #21 one numbered
    // above it: both labelled `ready-for-agent`, as the Spec is.
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    ready_issue(&scenario, 20, &[]);
    ready_issue(&scenario, 21, &[]);
    scenario.spec_has_tickets(20, &[(7, &[]), (21, &[])]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "issue-20"));
    scenario.agent_does_for(21, &agent_opens_pr(21, "issue-20"));

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-20"), "ready for review");
    let taking = format!(
        "thirdshift: taking Ready issue #20 \"Issue 20\", as thirdshift {} would\n",
        scenario.issue_url(20)
    );
    assert!(
        after_start(&result.stderr).starts_with(&taking),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(20), [IN_PROGRESS]);
    for ticket in [7, 21] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}"));
        assert_eq!(pr["base"], "issue-20");
        assert_eq!(pr["state"], "MERGED");
    }
}

#[test]
fn an_issue_with_an_open_blocker_is_not_taken_and_with_that_blocker_closed_it_is() {
    let scenario = ready_ticket();
    scenario.issue_blocked_by(7, &[5, 6]);
    scenario.issue_is(5, "CLOSED");
    scenario.issue_is(6, "OPEN");

    let blocked = scenario.run(&["pickup"]);

    let passed_over = no_ready_issue_after(&["#7 blocked by #6"]);
    assert_skipped(&scenario, &blocked, &passed_over);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);

    scenario.issue_is(6, "CLOSED");
    let unblocked = scenario.run(&["pickup"]);

    assert_ended_with_pr(
        &unblocked,
        &pr_from(&scenario, "issue-7"),
        "ready for review",
    );
}

/// What a Pickup run says of #7 when it was labelled `ready-for-agent` too
/// recently to have settled.
const LABELLED_TOO_RECENTLY: &str =
    "#7 not settled: labelled ready-for-agent less than 5 minutes ago";

#[test]
fn an_issue_labelled_ready_for_agent_less_than_five_minutes_ago_is_not_taken() {
    let scenario = ready_ticket();
    // Its other labels, however recent, and an earlier `ready-for-agent`
    // don't count: only the last time `ready-for-agent` was applied does.
    scenario.issue_timeline(
        7,
        &[
            (TimelineEvent::Labelled(READY_FOR_AGENT), 60),
            (TimelineEvent::Labelled(READY_FOR_AGENT), 4),
            (TimelineEvent::Labelled("bug"), 1),
        ],
    );

    let result = scenario.run(&["pickup"]);

    let passed_over = no_ready_issue_after(&[LABELLED_TOO_RECENTLY]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
}

#[test]
fn an_issue_labelled_ready_for_agent_more_than_five_minutes_ago_is_taken() {
    let scenario = ready_ticket();
    scenario.issue_timeline(
        7,
        &[
            (TimelineEvent::Labelled(READY_FOR_AGENT), 6),
            (TimelineEvent::Labelled("bug"), 1),
        ],
    );

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
}

#[test]
fn pickup_wait_in_the_user_config_shortens_lengthens_or_disables_settling() {
    for (wait, ago, taken) in [(1, 2, true), (30, 6, false), (0, 0, true)] {
        for (event, shaped) in [
            (
                TimelineEvent::Labelled(READY_FOR_AGENT),
                "labelled ready-for-agent",
            ),
            (TimelineEvent::SubIssueAdded, "a sub-issue added or removed"),
            (
                TimelineEvent::SubIssueRemoved,
                "a sub-issue added or removed",
            ),
            (
                TimelineEvent::BlockedByAdded,
                "a \"blocked by\" link added or removed",
            ),
            (
                TimelineEvent::BlockedByRemoved,
                "a \"blocked by\" link added or removed",
            ),
        ] {
            let scenario = ready_ticket();
            scenario.user_config_is(&format!("[pickup]\nwait_minutes = {wait}\n"));
            scenario.issue_timeline(
                7,
                &[
                    (TimelineEvent::Labelled(READY_FOR_AGENT), 600),
                    (event, ago),
                ],
            );

            let result = scenario.run(&["pickup"]);

            if taken {
                assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
            } else {
                let line = format!("#7 not settled: {shaped} less than {wait} minutes ago");
                assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
                assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
            }
        }
    }
}

#[test]
fn zero_pickup_wait_ignores_server_timestamps_ahead_of_local_clock() {
    let scenario = ready_ticket();
    scenario.user_config_is("[pickup]\nwait_minutes = 0\n");
    scenario.issue_timeline(7, &[(TimelineEvent::Labelled(READY_FOR_AGENT), -1)]);

    let result = scenario.run(&["pickup"]);

    assert_eq!(
        scenario.claude_calls().len(),
        1,
        "Pickup did not dispatch with zero wait:\n{}",
        result.stderr
    );
    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
}

#[test]
fn a_spec_labelled_long_ago_whose_sub_issues_or_blockers_changed_less_than_five_minutes_ago_is_not_taken()
 {
    for (event, changed) in [
        (TimelineEvent::SubIssueAdded, "a sub-issue"),
        (TimelineEvent::SubIssueRemoved, "a sub-issue"),
        (TimelineEvent::BlockedByAdded, "a \"blocked by\" link"),
        (TimelineEvent::BlockedByRemoved, "a \"blocked by\" link"),
    ] {
        let scenario = ready_spec("");
        scenario.issue_timeline(
            7,
            &[
                (TimelineEvent::Labelled(READY_FOR_AGENT), 600),
                (TimelineEvent::SubIssueAdded, 590),
                (event, 4),
            ],
        );

        let result = scenario.run(&["pickup"]);

        let line = format!("#7 not settled: {changed} added or removed less than 5 minutes ago");
        assert_skipped(&scenario, &result, &no_ready_issue_after(&[&line]));
        assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    }
}

/// What a Pickup run says of #7 when it is a Spec whose Tickets are all
/// closed, with nothing started on it.
const EVERY_TICKET_CLOSED: &str = "#7 every Ticket is closed";

/// An open issue labelled `ready-for-agent`, #7, that is a Spec whose
/// Tickets, #5 and #6, are both closed, with no Spec branch.
fn spec_with_every_ticket_closed() -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.spec_has_tickets(7, &[(5, &[]), (6, &[])]);
    scenario.issue_is(5, "CLOSED");
    scenario.issue_is(6, "CLOSED");
    scenario
}

#[test]
fn a_spec_whose_tickets_are_all_closed_is_passed_over_and_the_next_ready_issue_is_taken() {
    let scenario = spec_with_every_ticket_closed();
    ready_issue(&scenario, 9, &[]);
    scenario.agent_does_for(9, &agent_opens_pr(9, "main"));

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-9"), "ready for review");
    let lines = format!(
        "thirdshift: {EVERY_TICKET_CLOSED}\n\
         thirdshift: taking Ready issue #9 \"Issue 9\", as thirdshift {} would\n",
        scenario.issue_url(9)
    );
    assert!(
        after_start(&result.stderr).starts_with(&lines),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    assert_eq!(scenario.issue_labels(9), [IN_PROGRESS]);
    assert_eq!(sessions(&scenario), ["9"]);
}

#[test]
fn with_only_a_spec_whose_tickets_are_all_closed_the_pass_is_skipped_changing_nothing() {
    let scenario = spec_with_every_ticket_closed();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);
    let github = scenario.gh_state();

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    let passed_over = no_ready_issue_after(&[EVERY_TICKET_CLOSED]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.gh_state(), github);
    assert!(resend.requests().is_empty());
}

#[test]
fn a_spec_with_one_ticket_still_open_is_taken() {
    let scenario = ready_spec("");
    scenario.issue_is(8, "CLOSED");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
    assert_eq!(sessions(&scenario), ["9", "7"]);
}

/// An open issue labelled `ready-for-agent`, #7, that is a Spec whose only
/// Ticket, #8, is labelled `needs-info`, and whose session would open its PR
/// into the Spec branch.
fn spec_with_an_unready_ticket() -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.spec_has_tickets(7, &[(8, &[])]);
    scenario.issue_labelled(8, &["needs-info"]);
    scenario.agent_does_for(8, &agent_opens_pr(8, "issue-7"));
    scenario
}

#[test]
fn a_spec_whose_only_open_ticket_is_unready_is_passed_over_changing_nothing() {
    let scenario = spec_with_an_unready_ticket();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);
    let github = scenario.gh_state();

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    let passed_over =
        no_ready_issue_after(&["#7 has no Ticket that can start: #8 labelled needs-info"]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.gh_state(), github);
    assert_eq!(
        scenario.origin_log("issue-7"),
        None,
        "the Spec branch was pushed"
    );
    assert!(resend.requests().is_empty());
}

#[test]
fn a_spec_whose_open_tickets_are_all_blocked_names_why_each_cant_start() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.spec_has_tickets(7, &[(8, &[9]), (9, &[]), (10, &[30])]);
    scenario.issue_labelled(9, &["ready-for-human"]);
    let github = scenario.gh_state();

    let result = scenario.run(&["pickup"]);

    let passed_over = no_ready_issue_after(&[
        "#7 has no Ticket that can start: #8 blocked by #9; #9 labelled ready-for-human; \
         #10 blocked by #30 (outside the Spec)",
    ]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.gh_state(), github);
}

#[test]
fn a_spec_with_one_startable_ticket_and_one_unready_ticket_is_taken() {
    let scenario = ready_spec("");
    scenario.issue_labelled(8, &[READY_FOR_AGENT, "needs-info"]);

    let result = scenario.run(&["pickup"]);

    assert!(
        result.stderr.contains("taking Ready issue #7 \"Issue 7\""),
        "stderr: {}",
        result.stderr
    );
    // Ticket #8 can't start, so the Spec run fails and keeps its Claim.
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS, AGENT_FAILED]);
    assert_eq!(sessions(&scenario).first().map(String::as_str), Some("9"));
}

#[test]
fn a_handed_back_spec_with_no_startable_ticket_is_passed_over() {
    let scenario = spec_with_an_unready_ticket();
    scenario.origin_has_branch("issue-7", "main", &["Earlier Spec run"]);
    scenario.issue_timeline(
        7,
        &[
            (TimelineEvent::Labelled(IN_PROGRESS), 90),
            (TimelineEvent::Unlabelled(IN_PROGRESS), 61),
            (TimelineEvent::Labelled(READY_FOR_AGENT), 60),
        ],
    );
    let github = scenario.gh_state();

    let result = scenario.run(&["pickup"]);

    let passed_over =
        no_ready_issue_after(&["#7 has no Ticket that can start: #8 labelled needs-info"]);
    assert_skipped(&scenario, &result, &passed_over);
    assert_eq!(scenario.gh_state(), github);
}

#[test]
fn once_its_ticket_is_no_longer_unready_the_next_pass_takes_the_spec() {
    let scenario = spec_with_an_unready_ticket();
    let passed_over = scenario.run(&["pickup"]);
    assert_eq!(
        after_start(&passed_over.stderr),
        no_ready_issue_after(&["#7 has no Ticket that can start: #8 labelled needs-info"])
    );

    scenario.issue_labelled(8, &[READY_FOR_AGENT]);
    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
    assert_eq!(sessions(&scenario), ["8", "7"]);
}

/// A Ready issue, #7, that is a Spec with two Tickets that don't block each
/// other, #8 and #9. Each Ticket's session opens its PR into the Spec branch,
/// #9's after doing `before_ticket_9`.
fn ready_spec(before_ticket_9: &str) -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[])]);
    scenario.agent_does_for(8, &agent_opens_pr(8, "issue-7"));
    scenario.agent_does_for(
        9,
        &format!("{before_ticket_9}\n{}", agent_opens_pr(9, "issue-7")),
    );
    scenario
}

/// A Ready issue, #7, with no sub-issues, whose session opens its PR into
/// `main`.
fn ready_ticket() -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));
    scenario
}

/// The issue number each agent session was for, the first Issue URL its
/// prompt names, in order.
fn sessions(scenario: &Scenario) -> Vec<String> {
    scenario
        .claude_calls()
        .iter()
        .map(|call| {
            let prompt = call["prompt"].as_str().unwrap();
            let (_, after) = prompt.split_once("/issues/").unwrap();
            after.chars().take_while(char::is_ascii_digit).collect()
        })
        .collect()
}

#[test]
fn a_ready_issue_with_sub_issues_is_run_as_a_spec_run_and_ends_as_that_spec_run_does() {
    let scenario = ready_spec("");

    let result = scenario.run(&["pickup"]);

    let spec_pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &spec_pr, "ready for review");
    assert_eq!(spec_pr["base"], "main");
    assert_eq!(spec_pr["state"], "OPEN");
    assert_eq!(spec_pr["isDraft"], false);
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
    let mut sessions = sessions(&scenario);
    assert_eq!(sessions.pop().as_deref(), Some("7"), "the Spec review");
    sessions.sort();
    assert_eq!(sessions, ["8", "9"]);
    for ticket in [8, 9] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}"));
        assert_eq!(pr["base"], "issue-7");
        assert_eq!(pr["state"], "MERGED");
    }
    scenario.assert_cleaned_up("issue-7");
}

#[test]
fn a_dispatched_run_that_fails_fails_the_pickup_run_as_a_failed_run_does() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does_for(7, "echo work > work.txt\nexit 1");

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(result.stdout, "");
    let last = *before_command_log(&result.stderr).last().unwrap();
    assert!(
        last.starts_with("thirdshift: session trace: ") && last.ends_with("-implement.logs.jsonl"),
        "stderr: {}",
        result.stderr
    );
    assert!(scenario.origin_log("issue-7").is_some(), "work not pushed");
}

#[test]
fn parallel_reaches_a_dispatched_spec_run() {
    // #9's session only succeeds if #8 landed on the Spec branch before it
    // started, as it has only when the Tickets run one at a time.
    let scenario = ready_spec("test -f issue-8.txt");

    let result = scenario.run(&["pickup", "parallel", "1"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(sessions(&scenario), ["8", "9", "7"]);
}

#[test]
fn parallel_is_ignored_without_error_for_an_issue_that_is_not_a_spec() {
    for flag in ["parallel", "--parallel"] {
        let scenario = ready_ticket();

        let result = scenario.run(&["pickup", flag, "2"]);

        assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    }
}

#[test]
fn merge_reaches_the_dispatched_run() {
    let scenario = ready_ticket();

    let result = scenario.run(&["pickup", "merge"]);

    let pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &pr, "merged");
    assert_eq!(pr["state"], "MERGED");
}

const RED: &str = r#"[{"name": "test", "conclusion": "failure"}]"#;

const GREEN: &str = r#"[{"name": "test", "conclusion": "success"}]"#;

/// A Ready issue, #7, whose Run opens its PR with `test` red on its head and
/// on `main`: an Inherited failure. The agent for the Base fix issue, #8, the
/// next issue, fixes it.
fn ready_ticket_that_inherits_a_failure() -> Scenario {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.agent_does_for(
        7,
        &format!(
            r#"{}
gh fake checks "$(git rev-parse HEAD)" '{RED}'
gh fake checks "$(git rev-parse origin/main)" '{RED}'
"#,
            agent_opens_pr(7, "main")
        ),
    );
    scenario.agent_does_for(
        8,
        &format!(
            r#"
echo "fixed" > ci-fix.txt
git add ci-fix.txt
git commit -q -m "Fix CI on main"
gh pr create --base main --head issue-8 --title "Fix CI on main" --body "Closes #8"
gh fake checks "$(git rev-parse HEAD)" '{GREEN}'
"#
        ),
    );
    scenario
}

#[test]
fn base_fix_reaches_the_dispatched_run() {
    let scenario = ready_ticket_that_inherits_a_failure();

    let result = scenario.run(&["pickup", "base-fix"]);

    assert!(
        result.stderr.contains(
            "thirdshift: starting Base fix #8 into main: https://github.com/acme/widgets/issues/8\n",
        ),
        "stderr: {}",
        result.stderr
    );
    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(pr_from(&scenario, "issue-8")["state"], "MERGED");
}

#[test]
fn a_run_that_fails_on_an_inherited_failure_offers_the_command_that_retries_its_issue_by_hand() {
    let scenario = ready_ticket_that_inherits_a_failure();

    let result = scenario.run(&["pickup", "merge", "parallel", "2", "base", "main"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    // Not a Spec, so without the `parallel` that would fail its Run by hand.
    let retry = format!("thirdshift {} merge base-fix\n", scenario.issue_url(7));
    assert!(result.stderr.contains(&retry), "stderr: {}", result.stderr);
}

#[test]
fn base_makes_the_named_branch_the_dispatched_runs_base_branch_whatever_is_checked_out() {
    for (flag, detached) in [("base", false), ("--base", true)] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.agent_does_for(7, &agent_opens_pr(7, "develop"));
        scenario.origin_has_branch("develop", "main", &["Develop work"]);
        if detached {
            scenario.launch_git(&["checkout", "-q", "--detach"]);
        }
        let checked_out = scenario.launch_git(&["branch", "--show-current"]);

        let result = scenario.run(&["pickup", flag, "develop"]);

        let pr = pr_from(&scenario, "issue-7");
        assert_ended_with_pr(&result, &pr, "ready for review");
        assert_eq!(pr["base"], "develop", "detached: {detached}");
        assert_eq!(
            scenario.origin_log("issue-7").unwrap(),
            ["Work on 7", "Develop work", "Initial commit"],
            "detached: {detached}"
        );
        assert_eq!(
            scenario.launch_git(&["branch", "--show-current"]),
            checked_out
        );
        scenario.assert_cleaned_up("issue-7");
    }
}

#[test]
fn base_makes_the_named_branch_the_dispatched_spec_runs_base_branch() {
    let scenario = ready_spec("");
    scenario.origin_has_branch("develop", "main", &["Develop work"]);

    let result = scenario.run(&["pickup", "base", "develop", "merge"]);

    let spec_pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &spec_pr, "merged");
    assert_eq!(spec_pr["base"], "develop");
    assert_eq!(
        scenario.origin_file("develop", "issue-8.txt").as_deref(),
        Some("8\n")
    );
    assert!(scenario.origin_file("main", "issue-8.txt").is_none());
}

/// Assert the Pickup run was stopped by a preflight check with `message`,
/// exit 1, before any work: no session, and no call to GitHub, so no label
/// was read or changed.
fn assert_stopped_by_preflight(scenario: &Scenario, result: &RunResult, message: &str) {
    scenario.assert_rejected_before_any_work(result, message);
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
}

#[test]
fn a_detached_head_without_base_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();
    scenario.launch_git(&["checkout", "-q", "--detach"]);

    let result = scenario.run(&["pickup"]);

    assert_stopped_by_preflight(
        &scenario,
        &result,
        "thirdshift: HEAD is detached; check out the branch the work should be based on, \
         or name it with base <branch>\n",
    );
}

#[test]
fn a_base_branch_ahead_of_origin_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();
    scenario.commit_locally("local.txt", "local\n", "Local work");

    let result = scenario.run(&["pickup"]);

    assert_stopped_by_preflight(
        &scenario,
        &result,
        "local main is 1 commit(s) ahead of origin/main; push them first",
    );
}

#[test]
fn a_named_branch_that_is_not_on_origin_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();

    let result = scenario.run(&["pickup", "base", "nowhere"]);

    assert_stopped_by_preflight(
        &scenario,
        &result,
        "base branch nowhere does not exist on origin or locally; \
         create or obtain that branch first, or select a valid Base branch",
    );
}

#[test]
fn a_missing_git_identity_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();
    scenario.git_email_is(None);

    let result = scenario.run(&["pickup"]);

    assert_stopped_by_preflight(&scenario, &result, "git user.email is not set");
}

#[test]
fn an_origin_that_is_not_on_github_stops_the_pass_before_any_label_changes() {
    let scenario = ready_ticket();
    scenario.set_origin_url("https://gitlab.com/acme/widgets.git");

    let result = scenario.run(&["pickup"]);

    assert_stopped_by_preflight(
        &scenario,
        &result,
        "origin https://gitlab.com/acme/widgets.git is not a GitHub repository",
    );
}

/// What a skipped Pickup run or Architect run says when another of either is
/// running on the scenario's repository.
const ALREADY_RUNNING: &str = "thirdshift: another Pass is already running on acme/widgets\n";

/// A script in which the agent touches `started` in the scenario root, then
/// waits there until the test touches `release`, or until the scenario is
/// gone, as after a test that failed while holding it.
const AGENT_WAITS_FOR_RELEASE: &str = r#"
root=$(dirname "$FAKE_CLAUDE_RECORD")
touch "$root/started"
while [ -d "$root" ] && [ ! -e "$root/release" ]; do sleep 0.05; done
"#;

/// Let an agent held by [`AGENT_WAITS_FOR_RELEASE`] go on.
fn release(scenario: &Scenario) {
    fs::write(scenario.path("release"), "").unwrap();
}

#[test]
fn a_pickup_run_is_skipped_while_an_architect_run_on_the_repository_is_still_running() {
    let scenario = Scenario::new();
    scenario.agent_does(AGENT_WAITS_FOR_RELEASE);
    let architect = scenario.run_until(&["architect", "--plan-only"], &[], "started");
    // Labelled only now, as a Ready issue would have skipped the Architect
    // run.
    ready_issue(&scenario, 7, &[]);
    let (github, gh_calls) = (scenario.gh_state(), scenario.gh_calls());

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(after_start(&result.stderr), ALREADY_RUNNING);
    assert_eq!(result.stdout, "");
    assert_eq!(scenario.claude_calls().len(), 1, "a session was started");
    assert_eq!(scenario.gh_state(), github);
    assert_eq!(scenario.gh_calls(), gh_calls);
    assert_eq!(scenario.entries("work"), [REPO, "widgets-architect"]);
    release(&scenario);
    architect.finish();
}

#[test]
fn a_pickup_run_or_an_architect_run_is_skipped_while_a_pickup_runs_dispatched_run_is_still_going() {
    for (second, starting) in [
        ("pickup", "Pickup run starting"),
        ("architect", "Architect run starting"),
    ] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        ready_issue(&scenario, 9, &[]);
        scenario.agent_does_for(
            7,
            &format!("{AGENT_WAITS_FOR_RELEASE}{}", agent_opens_pr(7, "main")),
        );
        let first = scenario.run_until(&["pickup"], &[], "started");
        let (github, gh_calls) = (scenario.gh_state(), scenario.gh_calls());

        let skipped = scenario.run(&[second]);

        assert_eq!(skipped.code, Some(0), "stderr: {}", skipped.stderr);
        assert_eq!(
            support::after_start(&skipped.stderr, starting),
            ALREADY_RUNNING,
            "{second}"
        );
        assert_eq!(skipped.stdout, "");
        assert_eq!(scenario.claude_calls().len(), 1, "a session was started");
        assert_eq!(scenario.gh_state(), github);
        assert_eq!(scenario.gh_calls(), gh_calls);
        release(&scenario);
        let first = first.finish();
        assert_ended_with_pr(&first, &pr_from(&scenario, "issue-7"), "ready for review");
        assert_eq!(scenario.issue_labels(9), [READY_FOR_AGENT]);
    }
}

#[test]
fn once_a_pickup_run_has_ended_the_next_takes_the_next_ready_issue() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    ready_issue(&scenario, 9, &[]);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));
    scenario.agent_does_for(9, &agent_opens_pr(9, "main"));
    let first = scenario.run(&["pickup"]);
    assert_ended_with_pr(&first, &pr_from(&scenario, "issue-7"), "ready for review");

    let second = scenario.run(&["pickup"]);

    assert_ended_with_pr(&second, &pr_from(&scenario, "issue-9"), "ready for review");
    let third = scenario.run(&["pickup"]);
    assert_eq!(third.code, Some(0), "stderr: {}", third.stderr);
    assert_eq!(after_start(&third.stderr), NO_READY_ISSUE);
    assert_eq!(third.stdout, "");
    assert_eq!(sessions(&scenario), ["7", "9"]);
}

const KEY: &str = "re_test_123";

const ACCEPTED: &str = r#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#;

/// The environment for a Pickup run against `resend`, with a Resend API key.
fn resend_env(resend: &ResendStandIn) -> [(&str, &str); 2] {
    [
        ("THIRDSHIFT_RESEND_URL", resend.url()),
        ("RESEND_API_KEY", KEY),
    ]
}

/// Run thirdshift with `args` against `resend`, with a Resend API key in the
/// environment.
fn run_with_resend(scenario: &Scenario, resend: &ResendStandIn, args: &[&str]) -> RunResult {
    scenario.run_with_env(args, &resend_env(resend))
}

/// The subject and the text of the one Run notification `resend` received.
fn the_one_notification(resend: &ResendStandIn) -> (String, String) {
    let requests = resend.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let body = &requests[0].body;
    let part = |name: &str| body[name].as_str().unwrap().to_string();
    let text = part("text");
    assert_one_issue_line(&text);
    (part("subject"), text)
}

/// A Ready issue, #7, titled, with no sub-issues, whose session opens its PR
/// into `main`.
fn titled_ready_ticket() -> Scenario {
    let scenario = ready_ticket();
    scenario.issue_titled(7, "Sharpen the widgets");
    scenario
}

/// The User config of a machine where every Run and every Pickup run that
/// takes an issue sends a Run notification.
const EMAIL_ALWAYS: &str = "[email]\nalways = true\nto = \"config@example.com\"\n";

#[test]
fn a_pass_that_took_an_issue_sends_one_notification_when_asked() {
    let scenario = titled_ready_ticket();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    let requests = resend.requests();
    // Exactly one: the dispatched run sends none of its own.
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].body["to"], "config@example.com");
    assert_saved_notification(&scenario, "pickup", &requests[0]);
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands/issue")
            .exists()
    );
}

#[test]
fn the_notification_of_a_run_left_ready_for_review_is_that_runs() {
    let scenario = titled_ready_ticket();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

    let pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &pr, "ready for review");
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen the widgets: ready for review"
    );
    let logs = scenario.log_files(
        "home/.thirdshift/logs/acme/widgets/commands/pickup",
        "jsonl",
    );
    assert_eq!(logs.len(), 1, "logs: {logs:?}");
    let log = scenario
        .path("home/.thirdshift/logs/acme/widgets/commands/pickup")
        .join(&logs[0]);
    let commands = scenario.log_files("home/.thirdshift/logs/acme/widgets/commands/pickup", "log");
    assert_eq!(commands.len(), 1, "Command logs: {commands:?}");
    let command_log = scenario
        .path("home/.thirdshift/logs/acme/widgets/commands/pickup")
        .join(&commands[0]);
    assert!(
        text.starts_with(&format!(
            "Result:       ready for review\n\
             Issue:        https://github.com/acme/widgets/issues/7\n\
             Title:        Sharpen the widgets\n\
             Command:      Pickup run, which dispatched a Ticket run\n\
             Pull request: {}\n\
             Session trace:  {}\n\
             Command log:  {}\n\
             Built with claude 9.8.7 · default model · default effort (configured)\n\
             Sent by:      thirdshift\n\
             Host:         ",
            pr["url"].as_str().unwrap(),
            log.display(),
            command_log.display()
        )),
        "{text}"
    );
    assert!(text.contains("\nTook:         "), "{text}");
}

#[test]
fn the_notification_of_a_merge_run_says_it_merged() {
    let scenario = titled_ready_ticket();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "merge"]);

    let pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &pr, "merged");
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen the widgets: merged"
    );
    let pull_request = format!(
        "Result:       merged\nIssue:        https://github.com/acme/widgets/issues/7\nTitle:        Sharpen the widgets\nCommand:      Pickup run, which dispatched a Ticket run\nPull request: {}\n",
        pr["url"].as_str().unwrap()
    );
    assert!(text.starts_with(&pull_request), "{text}");
}

#[test]
fn the_notification_of_a_failed_run_has_its_pull_request_and_its_cause() {
    let scenario = titled_ready_ticket();
    scenario.agent_does_for(7, &format!("{}exit 3\n", agent_opens_pr(7, "main")));
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "--email", "me@example.com"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen the widgets: failed"
    );
    assert!(
        text.starts_with(&format!(
            "Result:       failed\n\
             Issue:        https://github.com/acme/widgets/issues/7\n\
             Title:        Sharpen the widgets\n\
             Command:      Pickup run, which dispatched a Ticket run\n\
             Pull request: {}\n\
             Cause:        claude exited 3\n\
             Session trace:  ",
            pr_from(&scenario, "issue-7")["url"].as_str().unwrap()
        )),
        "{text}"
    );
}

#[test]
fn the_notification_of_an_interrupted_run_says_it_was_interrupted() {
    let scenario = titled_ready_ticket();
    scenario.agent_does_for(
        7,
        r#"touch "$(dirname "$FAKE_CLAUDE_RECORD")/started"
sleep 60"#,
    );
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = scenario.run_and_signal_with_env(
        &["pickup", "--email", "me@example.com"],
        &resend_env(&resend),
        "started",
        "TERM",
    );

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen the widgets: interrupted"
    );
    assert!(
        text.starts_with(
            "Result:       interrupted\nIssue:        https://github.com/acme/widgets/issues/7\n"
        ),
        "{text}"
    );
    assert!(!text.contains("Cause:"), "{text}");
}

#[test]
fn the_notification_of_a_spec_run_is_that_spec_runs_with_a_line_per_ticket() {
    let scenario = ready_spec("");
    scenario.issue_titled(7, "Sharpen every widget");
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    let spec_pr = pr_from(&scenario, "issue-7");
    assert_ended_with_pr(&result, &spec_pr, "ready for review");
    // Exactly one: neither the Spec run nor a Ticket's Run sends its own.
    let (subject, text) = the_one_notification(&resend);
    assert_eq!(
        subject,
        "[thirdshift] acme/widgets#7 Sharpen every widget: ready for review"
    );
    let pull_request = format!(
        "Result:       ready for review\nIssue:        https://github.com/acme/widgets/issues/7\nTitle:        Sharpen every widget\nCommand:      Pickup run, which dispatched a Spec run\nPull request: {}\n",
        spec_pr["url"].as_str().unwrap()
    );
    assert!(text.starts_with(&pull_request), "{text}");
    let (_, tickets) = text.split_once("\nTickets:\n").expect(&text);
    for ticket in [8, 9] {
        let pr = pr_from(&scenario, &format!("issue-{ticket}"));
        let line = format!("#{ticket} landed with {}\n", pr["url"].as_str().unwrap());
        assert!(tickets.contains(&line), "expected {line:?} in: {text}");
    }
}

#[test]
fn the_notification_of_a_failed_spec_run_has_each_tickets_outcome() {
    let scenario = ready_spec("exit 3");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert!(subject.ends_with(": failed"), "{subject}");
    assert!(
        text.starts_with(
            "Result:       failed\nIssue:        https://github.com/acme/widgets/issues/7\n"
        ),
        "{text}"
    );
    let (_, tickets) = text.split_once("\nTickets:\n").expect(&text);
    let landed = pr_from(&scenario, "issue-8");
    let landed = format!("#8 landed with {}\n", landed["url"].as_str().unwrap());
    assert!(tickets.contains(&landed), "expected {landed:?} in: {text}");
    assert!(tickets.contains("#9 failed: "), "{text}");
}

#[test]
fn a_spec_notification_promotes_exhausted_usage_and_keeps_other_failures() {
    let scenario = ready_spec(
        "echo 'API error 429: Subscription quota exhausted. Your usage window resets at 2026-10-24T03:28:16Z.' > \"$FAKE_MUSE_ERROR\"\nexit 1",
    );
    scenario.spec_has_tickets(7, &[(8, &[]), (9, &[]), (10, &[9]), (11, &[])]);
    scenario.agent_does_for(
        11,
        "echo 'stream protocol error' > \"$FAKE_MUSE_ERROR\"\nexit 1",
    );
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(
        &scenario,
        &resend,
        &["pickup", "harness", "muse", "email", "me@example.com"],
    );

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    let (subject, text) = the_one_notification(&resend);
    assert!(subject.ends_with(": failed"), "{subject}");
    let (summary, tickets) = text.split_once("\nTickets:\n").expect(&text);
    assert!(
        summary.contains("Usage limit:  Harness usage allowance exhausted in Tickets #9. Retry when usage is available.\n"),
        "{text}"
    );
    for detail in [
        "#8 landed with ",
        "#9 failed: muse exited 1: API error 429: Subscription quota exhausted.",
        "Your usage window resets at 2026-10-24T03:28:16Z.",
        "#10 blocked by #9",
        "#11 failed: muse exited 1: stream protocol error",
    ] {
        assert!(tickets.contains(detail), "{text}");
    }
    assert_saved_notification(&scenario, "pickup", &resend.requests()[0]);
}

#[test]
fn a_pass_skipped_for_having_no_ready_issue_sends_no_notification() {
    let scenario = Scenario::new();
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    assert_skipped(&scenario, &result, NO_READY_ISSUE);
    assert!(resend.requests().is_empty());
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands")
            .exists()
    );
}

/// Start a Pickup run that takes #7, made a Ready issue here, and holds the
/// repository's lock until [`release`], its session waiting once started.
fn pickup_run_held_on_issue_7(scenario: &Scenario) -> HeldRun {
    ready_issue(scenario, 7, &[]);
    scenario.agent_does_for(
        7,
        &format!("{AGENT_WAITS_FOR_RELEASE}{}", agent_opens_pr(7, "main")),
    );
    scenario.run_until(&["pickup"], &[], "started")
}

#[test]
fn a_pass_skipped_for_the_lock_sends_no_notification() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 9, &[]);
    let first = pickup_run_held_on_issue_7(&scenario);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let skipped = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

    assert_eq!(skipped.code, Some(0), "stderr: {}", skipped.stderr);
    assert_eq!(after_start(&skipped.stderr), ALREADY_RUNNING);
    assert_eq!(skipped.stdout, "");
    release(&scenario);
    first.finish();
    assert!(resend.requests().is_empty());
}

#[test]
fn asked_for_a_notification_with_no_address_known_the_pass_stops_before_any_work() {
    let scenario = ready_ticket();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "--email"]);

    assert_stopped_by_preflight(&scenario, &result, "no email address");
    assert!(resend.requests().is_empty());
}

#[test]
fn asked_for_a_notification_with_no_resend_api_key_the_pass_stops_before_any_work() {
    let scenario = ready_ticket();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = scenario.run_with_env(
        &["pickup", "email", "me@example.com"],
        &[("THIRDSHIFT_RESEND_URL", resend.url())],
    );

    assert_stopped_by_preflight(&scenario, &result, "no Resend API key");
    assert!(resend.requests().is_empty());
}

#[test]
fn a_pass_that_would_be_skipped_for_having_no_ready_issue_still_stops_on_the_notifications_checks()
{
    let scenario = Scenario::new();
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "--email"]);

    scenario.assert_rejected_before_any_work(&result, "no email address");
    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    assert!(resend.requests().is_empty());
}

#[test]
fn a_pass_that_would_be_skipped_for_the_lock_still_stops_on_the_notifications_checks() {
    let scenario = Scenario::new();
    let first = pickup_run_held_on_issue_7(&scenario);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let stopped = run_with_resend(&scenario, &resend, &["pickup", "--email"]);

    assert_eq!(stopped.code, Some(1), "stderr: {}", stopped.stderr);
    assert!(
        stopped.stderr.contains("no email address"),
        "stderr: {}",
        stopped.stderr
    );
    assert_eq!(stopped.stdout, "");
    release(&scenario);
    first.finish();
    assert!(resend.requests().is_empty());
}

#[test]
fn the_notifications_checks_come_before_the_checks_on_the_clone() {
    let scenario = ready_ticket();
    scenario.launch_git(&["checkout", "-q", "--detach"]);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "--email"]);

    assert_stopped_by_preflight(&scenario, &result, "no email address");
    assert!(
        !result.stderr.contains("HEAD is detached"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn a_pass_that_cant_tell_whether_its_issue_is_a_spec_fails_before_saying_it_took_it() {
    let scenario = titled_ready_ticket();
    scenario.gh_fails("api graphql");
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        !result.stderr.contains("taking Ready issue"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    assert!(scenario.claude_calls().is_empty(), "a session was started");
    assert!(resend.requests().is_empty());
}

#[test]
fn a_notification_that_cant_be_sent_is_a_warning_that_changes_neither_the_exit_code_nor_stdout() {
    for (script, code) in [(agent_opens_pr(7, "main"), 0), ("exit 3".to_string(), 1)] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.agent_does_for(7, &script);
        let unasked = scenario.run(&["pickup"]);
        assert_eq!(unasked.code, Some(code), "stderr: {}", unasked.stderr);

        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.agent_does_for(7, &script);
        let resend = ResendStandIn::replying(500, "upstream exploded");

        let result = run_with_resend(&scenario, &resend, &["pickup", "email", "me@example.com"]);

        assert_eq!(result.code, Some(code), "stderr: {}", result.stderr);
        assert_eq!(result.stdout, unasked.stdout);
        assert_eq!(resend.requests().len(), 1);
        let last = result.stderr.lines().last().unwrap();
        assert!(
            last.contains("warning: could not send the Run notification")
                && last.contains("upstream exploded"),
            "stderr: {}",
            result.stderr
        );
    }
}

/// Make each of `issues` open and labelled `in-progress` and `agent-failed`
/// on the fake GitHub: an issue a failed run kept its Claim on.
fn failed_issues(scenario: &Scenario, issues: &[u32]) {
    for issue in issues {
        scenario.issue_is(*issue, "OPEN");
        scenario.issue_labelled(*issue, &[IN_PROGRESS, AGENT_FAILED]);
    }
}

#[test]
fn a_failure_limit_skip_names_every_failure_lowest_first_without_extra_github_requests() {
    let scenario = Scenario::new();
    failed_issues(&scenario, &[953, 924, 936]);

    let result = scenario.run(&["pickup"]);

    let listing = |state: &str, label: &str| {
        vec![
            "api".to_string(),
            "--paginate".into(),
            "--jq".into(),
            ".[]".into(),
            format!(
                "repos/acme/widgets/issues?state={state}&labels={label}&sort=created&direction=desc&per_page=100"
            ),
        ]
    };
    assert_eq!(
        scenario.gh_calls(),
        [
            vec![
                "api".to_string(),
                "repos/acme/widgets/actions/runs?branch=main&event=push&per_page=1".into(),
            ],
            listing("closed", IN_PROGRESS),
            listing("closed", AGENT_FAILED),
            listing("open", AGENT_FAILED),
        ]
    );
    assert_skipped(
        &scenario,
        &result,
        "thirdshift: at the Failure limit on acme/widgets: #924, #936 and #953 are labelled \
         agent-failed, pickup.failure_limit is 3\n",
    );
}

#[test]
fn a_pass_skipped_for_the_failure_limit_sends_no_notification() {
    let scenario = ready_ticket();
    failed_issues(&scenario, &[1, 2, 3]);
    scenario.user_config_is(EMAIL_ALWAYS);
    let resend = ResendStandIn::replying(200, ACCEPTED);

    let result = run_with_resend(&scenario, &resend, &["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        "thirdshift: at the Failure limit on acme/widgets: #1, #2 and #3 are labelled \
         agent-failed, pickup.failure_limit is 3\n",
    );
    assert!(resend.requests().is_empty());
}

#[test]
fn pickup_failure_limit_in_the_user_config_raises_and_lowers_the_failure_limit() {
    // Raised, three failures no longer stop the pass.
    let scenario = ready_ticket();
    failed_issues(&scenario, &[1, 2, 3]);
    scenario.user_config_is("[pickup]\nfailure_limit = 4\n");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");

    // Lowered, one failure stops it.
    let scenario = ready_ticket();
    failed_issues(&scenario, &[1]);
    scenario.user_config_is("[pickup]\nfailure_limit = 1\n");

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        "thirdshift: at the Failure limit on acme/widgets: #1 is labelled agent-failed, \
         pickup.failure_limit is 1\n",
    );
}

#[test]
fn claims_whose_pull_requests_are_ready_draft_or_merged_never_count() {
    let scenario = ready_ticket();
    for (number, state, draft) in [(1, "OPEN", false), (2, "OPEN", true), (3, "MERGED", false)] {
        scenario.issue_is(number, "OPEN");
        scenario.issue_labelled(number, &[IN_PROGRESS]);
        scenario.github_has_pr(&format!("issue-{number}"), "main", state);
        if draft {
            let mut github = scenario.gh_state();
            github["prs"].as_array_mut().unwrap().last_mut().unwrap()["isDraft"] = true.into();
            scenario.write_gh_state(&github);
        }
    }
    scenario.user_config_is("[pickup]\nfailure_limit = 1\n");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    for number in [1, 2, 3] {
        assert_eq!(scenario.issue_labels(number), [IN_PROGRESS]);
    }
}

#[test]
fn a_closed_issue_labelled_agent_failed_does_not_count_and_is_swept() {
    let scenario = ready_ticket();
    failed_issues(&scenario, &[1, 2, 3]);
    scenario.issue_is(3, "CLOSED");
    scenario.issue_labelled(3, &["bug", IN_PROGRESS, AGENT_FAILED]);

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert!(
        result
            .stderr
            .contains("thirdshift: taking in-progress and agent-failed off #3, which is closed\n"),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(3), ["bug"]);
}

#[test]
fn a_failure_the_day_shift_handed_back_or_took_over_loses_agent_failed_and_does_not_count() {
    let scenario = ready_ticket();
    failed_issues(&scenario, &[11, 12, 13, 14]);
    // Handed back, with or without the Claim taken off: Ready, but above #7.
    scenario.issue_labelled(11, &[IN_PROGRESS, AGENT_FAILED, READY_FOR_AGENT]);
    scenario.issue_labelled(12, &[AGENT_FAILED, READY_FOR_AGENT]);
    // Taken over: no Claim and no needs-triage left.
    scenario.issue_labelled(13, &[AGENT_FAILED, "bug"]);
    // Still waiting for triage, so still counted.
    scenario.issue_labelled(14, &[AGENT_FAILED, "needs-triage"]);
    scenario.user_config_is("[pickup]\nfailure_limit = 2\n");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    for (number, acted) in [(11, "handed back"), (12, "handed back"), (13, "took over")] {
        let line =
            format!("thirdshift: taking agent-failed off #{number}, which the Day shift {acted}\n");
        assert!(result.stderr.contains(&line), "stderr: {}", result.stderr);
    }
    assert_eq!(scenario.issue_labels(11), [IN_PROGRESS, READY_FOR_AGENT]);
    assert_eq!(scenario.issue_labels(12), [READY_FOR_AGENT]);
    assert_eq!(scenario.issue_labels(13), ["bug"]);
    assert_eq!(scenario.issue_labels(14), [AGENT_FAILED, "needs-triage"]);
}

#[test]
fn a_failure_whose_label_cannot_be_taken_off_is_a_warning_and_is_not_counted() {
    let scenario = ready_ticket();
    scenario.issue_is(1, "OPEN");
    scenario.issue_labelled(1, &[AGENT_FAILED, "bug"]);
    scenario.user_config_is("[pickup]\nfailure_limit = 1\n");
    scenario.gh_fails("api --method DELETE repos/acme/widgets/issues/1/labels/agent-failed");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert!(
        result
            .stderr
            .contains("thirdshift: warning: could not take agent-failed off #1: "),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(1), [AGENT_FAILED, "bug"]);
}

#[test]
fn a_pickup_failure_limit_that_is_not_a_whole_number_from_1_up_stops_the_pass_naming_the_file_and_key()
 {
    for limit in ["0", "-1", "1.5", "\"3\"", "true"] {
        let scenario = ready_ticket();
        let path = scenario.user_config_is(&format!("[pickup]\nfailure_limit = {limit}\n"));

        let result = scenario.run(&["pickup"]);

        scenario.assert_rejected_before_any_work(
            &result,
            &format!(
                "thirdshift: pickup.failure_limit must be a whole number from 1 up in the User \
                 config {}\n",
                path.display()
            ),
        );
        assert_eq!(result.code, Some(1), "{limit}: {}", result.stderr);
        assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    }
}

#[test]
fn an_old_pickup_limit_is_ignored_with_a_warning_naming_pickup_failure_limit() {
    let scenario = ready_ticket();
    failed_issues(&scenario, &[1]);
    scenario.issue_is(2, "OPEN");
    scenario.issue_labelled(2, &[IN_PROGRESS]);
    let path = scenario.user_config_is("[pickup]\nlimit = 1\n");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    let warning = format!(
        "thirdshift: warning: pickup.limit is ignored: a Pickup run now counts the issues it \
         failed on, up to pickup.failure_limit; remove pickup.limit from the User config {}\n",
        path.display()
    );
    assert_eq!(
        result.stderr.matches(&warning).count(),
        1,
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn an_invalid_pickup_wait_stops_the_pass_naming_the_file_and_key() {
    for wait in ["-1", "1.5", "\"30\"", "true", "9223372036854775807"] {
        let scenario = ready_ticket();
        let path = scenario.user_config_is(&format!("[pickup]\nwait_minutes = {wait}\n"));

        let result = scenario.run(&["pickup"]);

        scenario.assert_rejected_before_any_work(
            &result,
            &format!(
                "thirdshift: pickup.wait_minutes must be a whole number of minutes from 0 up \
                 within the supported range in the User config {}\n",
                path.display()
            ),
        );
        assert_eq!(result.code, Some(1), "{wait}: {}", result.stderr);
        assert_eq!(scenario.gh_calls(), Vec::<Vec<String>>::new());
    }
}

/// Make issue `number` closed on the fake GitHub, still labelled
/// `in-progress`, after the labels `before` it: an issue merged by hand,
/// which no Self-merge took the Claim off.
fn closed_issue_in_progress(scenario: &Scenario, number: u32, before: &[&str]) {
    scenario.issue_is(number, "CLOSED");
    scenario.issue_labelled(number, &[before, &[IN_PROGRESS]].concat());
}

/// What the Sweep says as it takes `in-progress` off closed issue #3.
const SWEEPING_3: &str = "thirdshift: taking in-progress off #3, which is closed\n";

#[test]
fn the_sweep_does_not_run_on_a_pass_skipped_for_the_lock() {
    let scenario = Scenario::new();
    closed_issue_in_progress(&scenario, 3, &["bug"]);
    scenario.agent_does(AGENT_WAITS_FOR_RELEASE);
    let architect = scenario.run_until(&["architect", "--plan-only"], &[], "started");
    // Labelled only now, as a Ready issue would have skipped the Architect
    // run.
    ready_issue(&scenario, 7, &[]);

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert_eq!(after_start(&result.stderr), ALREADY_RUNNING);
    assert_eq!(scenario.issue_labels(3), ["bug", IN_PROGRESS]);
    release(&scenario);
    architect.finish();
}

#[test]
fn a_sweep_that_fails_prints_a_warning_and_the_pass_carries_on() {
    let scenario = ready_ticket();
    closed_issue_in_progress(&scenario, 3, &["bug"]);
    closed_issue_in_progress(&scenario, 4, &[]);
    scenario.gh_fails("api --method DELETE");

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    for closed in [3, 4] {
        let warning = format!(
            "thirdshift: warning: could not take in-progress off #{closed}: gh api --method \
             DELETE repos/acme/widgets/issues/{closed}/labels/in-progress --silent failed: \
             HTTP 502"
        );
        assert!(
            result.stderr.contains(&warning),
            "stderr: {}",
            result.stderr
        );
    }
    assert_eq!(scenario.issue_labels(3), ["bug", IN_PROGRESS]);
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
}

#[test]
fn the_sweep_takes_the_label_off_as_the_closed_issue_spells_it() {
    let scenario = Scenario::new();
    scenario.issue_is(3, "CLOSED");
    scenario.issue_labelled(3, &["In-Progress", "bug"]);

    let result = scenario.run(&["pickup"]);

    assert_skipped(&scenario, &result, &format!("{SWEEPING_3}{NO_READY_ISSUE}"));
    assert_eq!(scenario.issue_labels(3), ["bug"]);
    let deleted = scenario.gh_calls_of("api", "--method");
    assert_eq!(deleted.len(), 1, "{deleted:?}");
    assert_eq!(
        deleted[0][3],
        "repos/acme/widgets/issues/3/labels/In-Progress"
    );
}

/// The arguments of each call to `claude` that was an agent session, which
/// streams JSON, and of each that was the test call that checks a Model.
fn sessions_and_test_calls(scenario: &Scenario) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    scenario
        .claude_calls()
        .iter()
        .map(|call| {
            let args = call["argv"].as_array().unwrap().iter();
            args.map(|arg| arg.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .partition(|args| args.iter().any(|arg| arg == "--output-format"))
}

#[test]
fn the_model_reaches_the_dispatched_run_once_the_pass_has_checked_it() {
    let scenario = ready_ticket();

    let result = scenario.run(&["pickup", "--model", "opus"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    let (sessions, test_calls) = sessions_and_test_calls(&scenario);
    assert_eq!(test_calls, [["-p", "--model", "opus"]]);
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    assert!(
        sessions[0]
            .windows(2)
            .any(|window| window == ["--model", "opus"]),
        "{sessions:?}"
    );
}

#[test]
fn a_model_claude_refuses_fails_the_pass_once_it_has_taken_an_issue_before_any_work() {
    let scenario = ready_ticket();
    scenario.agent_does_in_session(1, "echo 'no such model: Opus 5.5'\nexit 1");

    let result = scenario.run(&["pickup", "model", "Opus 5.5"]);

    assert_eq!(result.code, Some(1), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(
            "thirdshift: claude refused a test call on the Model Opus 5.5: no such model: Opus 5.5\n"
        ),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(
        !scenario
            .path("home/.thirdshift/logs/acme/widgets/commands")
            .exists(),
        "a Command log was kept"
    );
}

#[test]
fn an_issue_written_by_an_untrusted_account_is_passed_over() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let mut state = scenario.gh_state();
    state["author_associations"] = serde_json::json!({"7": "NONE"});
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        &no_ready_issue_after(&["#7 issue author is not a Trusted account"]),
    );
    assert!(scenario.claude_calls().is_empty());
}

#[test]
fn an_issue_with_no_ready_labelling_history_is_passed_over() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let mut state = scenario.gh_state();
    state["trust_events"] = serde_json::json!({"7": []});
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        &no_ready_issue_after(&["#7 has never been labelled ready-for-agent"]),
    );
}

#[test]
fn ready_labels_are_read_from_label_events_and_their_actors_trusted_by_membership() {
    for (association, actor, owner_type, collaborators, members) in [
        ("COLLABORATOR", "reader", "User", vec!["reader"], vec![]),
        ("MEMBER", "member", "Organization", vec![], vec!["member"]),
    ] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.agent_does_for(7, &agent_opens_pr(7, "main"));
        let mut state = scenario.gh_state();
        state["author_associations"] = serde_json::json!({"7": association});
        state["owner_type"] = serde_json::json!(owner_type);
        state["collaborators"] = serde_json::json!(collaborators);
        state["org_members"] = serde_json::json!(members);
        state["trust_events"] = serde_json::json!({"7": [
            {"created_at": "2020-01-01T00:00:00Z", "event": "labeled", "label": {"name": "ready-for-agent"}, "actor": {"login": "outsider"}},
            {"created_at": "2020-01-01T00:00:00Z", "event": "unlabeled", "label": {"name": "ready-for-agent"}, "actor": {"login": "acme"}},
            {"created_at": "2020-01-01T00:00:00Z", "event": "labeled", "label": {"name": "READY-for-AGENT"}, "actor": {"login": actor}},
            {"created_at": "2020-01-01T00:00:00Z", "event": "labeled", "label": {"name": "bug"}, "actor": {"login": "outsider"}}
        ]});
        scenario.write_gh_state(&state);

        let result = scenario.run(&["pickup"]);

        assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
        assert!(!result.stderr.contains("warning:"), "{}", result.stderr);
    }
}

#[test]
fn an_edit_whose_account_github_no_longer_knows_is_untrusted() {
    for kind in ["body", "title"] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        let mut state = scenario.gh_state();
        state["edits"] = serde_json::json!({"7": [
            {"kind": kind, "by": null, "at": "2020-01-03T00:00:00Z"}
        ]});
        scenario.write_gh_state(&state);

        let result = scenario.run(&["pickup"]);

        assert_skipped(
            &scenario,
            &result,
            &no_ready_issue_after(&[&format!(
                "#7 {kind} edited after it was marked ready by an account that is not a Trusted account"
            )]),
        );
    }
}

#[test]
fn an_untrusted_edit_is_found_before_a_later_edit_with_no_time() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let mut state = scenario.gh_state();
    state["edits"] = serde_json::json!({"7": [
        {"kind": "body", "by": "outsider", "at": "2020-01-03T00:00:00Z"},
        {"kind": "body", "by": "acme", "at": null}
    ]});
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        &no_ready_issue_after(&[
            "#7 body edited after it was marked ready by an account that is not a Trusted account",
        ]),
    );
}

#[test]
fn a_ready_labelling_whose_account_github_no_longer_knows_is_untrusted() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let mut state = scenario.gh_state();
    state["trust_events"] = serde_json::json!({"7": [
        {"created_at": "2020-01-01T00:00:00Z", "event": "labeled", "label": {"name": "ready-for-agent"}, "actor": null}
    ]});
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        &no_ready_issue_after(&[
            "#7 most recent ready-for-agent label actor is not a Trusted account",
        ]),
    );
}

#[test]
fn an_untrusted_edit_past_the_first_page_of_edit_history_is_found() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let mut state = scenario.gh_state();
    let mut edits: Vec<_> = (0..100)
        .map(|_| serde_json::json!({"kind": "body", "by": "acme", "at": "2020-01-02T00:00:00Z"}))
        .collect();
    edits.push(serde_json::json!(
        {"kind": "body", "by": "outsider", "at": "2020-01-03T00:00:00Z"}
    ));
    state["edits"] = serde_json::json!({ "7": edits });
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        &no_ready_issue_after(&[
            "#7 body edited after it was marked ready by an account that is not a Trusted account",
        ]),
    );
}

#[test]
fn a_failed_label_actor_membership_lookup_starts_no_work() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    let mut state = scenario.gh_state();
    state["trust_events"] = serde_json::json!({"7": [
        {"created_at": "2020-01-01T00:00:00Z", "event": "labeled", "label": {"name": "ready-for-agent"}, "actor": {"login": "reader"}}
    ]});
    state["failing"] = serde_json::json!(["api --include repos/acme/widgets/collaborators/reader"]);
    scenario.write_gh_state(&state);

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("could not check Trusted account membership"),
        "{}",
        result.stderr
    );
    assert!(scenario.claude_calls().is_empty());
    assert_eq!(scenario.issue_labels(7), [READY_FOR_AGENT]);
}

#[test]
fn a_handed_back_branch_is_continued_without_losing_interrupted_work() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.origin_has_branch("issue-7", "main", &["Interrupted work"]);
    scenario.issue_timeline(
        7,
        &[
            (TimelineEvent::Labelled(IN_PROGRESS), 90),
            (TimelineEvent::Unlabelled(IN_PROGRESS), 61),
            (TimelineEvent::Labelled(READY_FOR_AGENT), 60),
        ],
    );
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.claude_calls()[0]["branch"], "issue-7");
    assert!(scenario.first_prompt().contains("You are continuing work"));
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Work on 7".to_string(),
            "Interrupted work".to_string(),
            "Initial commit".to_string(),
        ])
    );
}

#[test]
fn a_same_second_hand_back_across_history_pages_continues_the_branch() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.origin_has_branch("issue-7", "main", &["Interrupted work"]);
    let mut events = vec![(TimelineEvent::Labelled(IN_PROGRESS), 90)];
    events.extend((0..98).map(|_| (TimelineEvent::Labelled("bug"), 65)));
    // The ready label ends one page; the Claim removal starts the next.
    events.extend([
        (TimelineEvent::Labelled(READY_FOR_AGENT), 60),
        (TimelineEvent::Unlabelled(IN_PROGRESS), 60),
    ]);
    scenario.issue_timeline(7, &events);
    scenario.agent_does_for(7, &agent_opens_pr(7, "main"));

    let result = scenario.run(&["pickup"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(scenario.claude_calls()[0]["branch"], "issue-7");
    assert!(scenario.first_prompt().contains("You are continuing work"));
}

#[test]
fn a_handed_back_open_pr_is_continued_using_its_base() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.origin_has_branch("develop", "main", &["Develop work"]);
    scenario.launch_checks_out("develop");
    scenario.origin_has_branch("issue-7", "main", &["Interrupted work"]);
    let pr = scenario.github_has_pr("issue-7", "main", "OPEN");
    // These operations can all occur in the same second. Timeline order
    // still establishes the Claim ending before the hand-back.
    scenario.issue_timeline(
        7,
        &[
            (TimelineEvent::Labelled("In-Progress"), 60),
            (TimelineEvent::Unlabelled("IN-PROGRESS"), 60),
            (TimelineEvent::Labelled("Ready-For-Agent"), 60),
        ],
    );
    scenario.agent_does_for(
        7,
        "echo more > more.txt\ngit add more.txt\ngit commit -q -m Continued",
    );

    let result = scenario.run(&["pickup", "base", "develop"]);

    assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
    assert_eq!(result.stdout, format!("{pr}\n"));
    assert_eq!(scenario.gh_state()["prs"].as_array().unwrap().len(), 1);
    assert!(scenario.first_prompt().contains("You are continuing work"));
    assert!(scenario.first_prompt().contains("The base branch is main."));
    assert_eq!(
        scenario.origin_log("issue-7"),
        Some(vec![
            "Continued".to_string(),
            "Interrupted work".to_string(),
            "Initial commit".to_string(),
        ])
    );
}

#[test]
fn a_recent_hand_back_on_a_later_history_page_waits_for_settling() {
    let scenario = Scenario::new();
    ready_issue(&scenario, 7, &[]);
    scenario.origin_has_branch("issue-7", "main", &["Interrupted work"]);
    let mut events = vec![(TimelineEvent::Labelled(IN_PROGRESS), 90)];
    events.extend((0..101).map(|_| (TimelineEvent::Labelled("bug"), 65)));
    events.extend([
        (TimelineEvent::Unlabelled(IN_PROGRESS), 2),
        (TimelineEvent::Labelled(READY_FOR_AGENT), 1),
    ]);
    scenario.issue_timeline(7, &events);

    let result = scenario.run(&["pickup"]);

    assert_skipped(
        &scenario,
        &result,
        &no_ready_issue_after(&[LABELLED_TOO_RECENTLY]),
    );
}

#[test]
fn label_history_is_read_past_one_page_before_accepting_or_rejecting_a_hand_back() {
    for handed_back in [true, false] {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.origin_has_branch("issue-7", "main", &["Interrupted work"]);
        let mut events = vec![
            (TimelineEvent::Labelled(IN_PROGRESS), 90),
            (TimelineEvent::Unlabelled(IN_PROGRESS), 80),
            (TimelineEvent::Labelled(READY_FOR_AGENT), 70),
        ];
        events.extend((0..205).map(|_| (TimelineEvent::Labelled("bug"), 65)));
        events.extend([
            (TimelineEvent::Labelled(IN_PROGRESS), 62),
            (TimelineEvent::Unlabelled(IN_PROGRESS), 61),
        ]);
        if handed_back {
            events.push((TimelineEvent::Labelled(READY_FOR_AGENT), 60));
            scenario.agent_does_for(7, &agent_opens_pr(7, "main"));
        }
        scenario.issue_timeline(7, &events);

        let result = scenario.run(&["pickup"]);

        if handed_back {
            assert_ended_with_pr(&result, &pr_from(&scenario, "issue-7"), "ready for review");
        } else {
            assert_skipped(
                &scenario,
                &result,
                &no_ready_issue_after(&[
                    "#7 started: issue-7 is on origin; no ready-for-agent hand-back after the last Claim ended",
                ]),
            );
        }
    }
}

#[test]
fn started_work_requires_a_hand_back_after_the_last_claim_ended() {
    use TimelineEvent::{Labelled as On, Unlabelled as Off};
    let claim = On(IN_PROGRESS);
    let end = Off(IN_PROGRESS);
    let ready = On(READY_FOR_AGENT);
    let histories = [
        vec![],
        vec![ready],
        vec![end, ready],
        vec![claim, ready],
        vec![claim, ready, end],
        vec![claim, end],
        vec![claim, end, ready, claim, end],
        vec![claim, end, ready, claim, ready, end],
        vec![claim, end, ready, Off(READY_FOR_AGENT)],
    ];
    for history in histories {
        let scenario = Scenario::new();
        ready_issue(&scenario, 7, &[]);
        scenario.origin_has_branch("issue-7", "main", &["Human or interrupted work"]);
        // These histories require a ready label strictly before the Claim
        // ends; a same-second swap is now a valid hand-back.
        let events: Vec<_> = history
            .into_iter()
            .enumerate()
            .map(|(index, event)| (event, 90 - index as i64))
            .collect();
        scenario.issue_timeline(7, &events);

        let result = scenario.run(&["pickup"]);

        assert_skipped(
            &scenario,
            &result,
            &no_ready_issue_after(&[
                "#7 started: issue-7 is on origin; no ready-for-agent hand-back after the last Claim ended",
            ]),
        );
    }
}

#[test]
fn an_unusable_security_harness_falls_back_before_the_pickup_dispatches_its_run() {
    let scenario = ready_ticket();
    scenario.user_config_is(
        r#"
[harness]
default = "codex"

[harness.claude]
model = "claude-opus-5-5"

[security]
harness = "claude"
review = true
"#,
    );
    scenario.agent_does_in_session(1, "echo 'no such model'\nexit 1");
    scenario.agent_does_for_in_session(7, 2, r#"
report=$(printf '%s' "$FAKE_CLAUDE_PROMPT" | sed -n 's/^Private report file: `\(.*\)`\.$/\1/p')
test -n "$report"
printf '[]\n' > "$report"
printf '%s\n' 'Security review: {"unaddressed_count":0,"findings":[],"pre_existing_count":0}' > "$FAKE_CLAUDE_FINAL_MESSAGE"
"#);

    let result = scenario.run(&["pickup"]);

    assert_eq!(result.code, Some(0), "stderr: {}", result.stderr);
    assert!(
        result.stderr.contains(
            "warning: Security review on claude can't be used (claude refused a test call on the Model claude-opus-5-5: no such model); reviewing on codex, the default Harness\n"
        ),
        "stderr: {}",
        result.stderr
    );
    assert_eq!(scenario.issue_labels(7), [IN_PROGRESS]);
    assert_eq!(scenario.codex_calls().len(), 2);
    assert_eq!(scenario.claude_calls().len(), 1);
    assert!(
        pr_from(&scenario, "issue-7")["body"]
            .as_str()
            .unwrap()
            .contains("No unaddressed introduced findings.")
    );
}
