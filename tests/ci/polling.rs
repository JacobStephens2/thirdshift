//! Real Command/GitHub observations with the existing shortened poll interval.

use super::{AGENT_OPENS_PR, Scenario};
use std::fs;

fn waiting_scenario() -> Scenario {
    let scenario = Scenario::new();
    fs::create_dir(scenario.path("fake")).unwrap();
    fs::rename(scenario.path("bin/gh"), scenario.path("fake/gh")).unwrap();
    super::support::write_executable(
        &scenario.path("bin/gh"),
        r#"#!/bin/bash
set -eu
if [[ " $* " == *' .check_runs[] '* ]]; then
    now=${EPOCHREALTIME/./}
    if [[ ! -f "$POLL_CLOCK" ]]; then echo "$now" > "$POLL_CLOCK"; fi
    start=$(cat "$POLL_CLOCK")
    now=$((now - start))
    echo "$now" >> "$POLL_READS"
    sha=${!#}
    sha=${sha%/check-runs*}
    sha=${sha##*/}
    pending=1
    if (( now >= 4500000 )); then pending=0; fi
    "$POLL_GH" fake checks "$sha" "[{\"name\":\"test\",\"conclusion\":\"success\",\"pending_polls\":$pending}]"
fi
exec "$POLL_GH" "$@"
"#,
    );
    scenario.agent_does(AGENT_OPENS_PR);
    scenario
}

fn run_wait(scenario: &Scenario) -> super::support::RunResult {
    scenario.run_with_env(
        &[&scenario.issue_url(7)],
        &[
            ("POLL_CLOCK", scenario.path("poll-clock").to_str().unwrap()),
            ("POLL_READS", scenario.path("poll-reads").to_str().unwrap()),
            ("POLL_GH", scenario.path("fake/gh").to_str().unwrap()),
            // 25ms represents the initial ten seconds: thirty minutes is 4.5s.
            ("THIRDSHIFT_POLL_MS", "25"),
        ],
    )
}

#[test]
fn thirty_minutes_of_pending_ci_uses_about_forty_polls_and_reports_green() {
    let scenario = waiting_scenario();
    let result = run_wait(&scenario);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(result.stderr.contains("CI passed"), "{}", result.stderr);
    let reads = fs::read_to_string(scenario.path("poll-reads")).unwrap();
    let polls = reads.lines().count();
    assert!(polls <= 40, "{polls} CI polls over 30 simulated minutes");
}

#[test]
fn a_pending_ci_poll_observes_the_pr_once() {
    let scenario = Scenario::new();
    scenario.agent_does(&format!(
        "{AGENT_OPENS_PR}{}",
        super::checks_on_head(r#"[{"name":"test","conclusion":"success","pending_polls":3}]"#)
    ));
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let calls = scenario.gh_calls();
    let reads: Vec<_> = calls
        .iter()
        .enumerate()
        .filter(|(_, call)| call.iter().any(|arg| arg == ".check_runs[]"))
        .map(|(index, _)| index)
        .collect();
    // Skip the first read that discovers CI; inspect a complete pending poll.
    let observations = calls[reads[1] + 1..reads[2]]
        .iter()
        .filter(|call| call.starts_with(&["pr".into(), "view".into()]))
        .count();
    assert_eq!(observations, 1, "PR observations between pending CI reads");
}
