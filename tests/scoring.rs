//! Scoring runs through the whole binary, with GitHub at the executable seam.
mod support;

use chrono::{DateTime, Utc};
use serde_json::Value;
use support::Scenario;

const LOGS: &str = "home/.thirdshift/logs/JacobStephens2/thirdshift/commands/scoring";

#[path = "scoring/grading.rs"]
mod grading;

#[path = "scoring/calibration.rs"]
mod calibration;

#[test]
fn packet_diff_preserves_trailing_whitespace_in_the_last_changed_line() {
    let scenario = scenario();
    scenario.origin_has_commit(
        "main",
        "trailing.txt",
        "keep trailing spaces  \n",
        "Whitespace fixture",
    );
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "codex", "model", false)]);
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let week = weeks(&scenario).remove(0);
    let packet: Value = serde_json::from_slice(
        &std::fs::read(
            scenario
                .path("home/.thirdshift/evals/scoring")
                .join(week["packets"].as_str().unwrap())
                .join("0/packet.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let diff = packet["diff"].as_str().unwrap();
    assert!(diff.ends_with("+keep trailing spaces  \n"), "{diff:?}");
}

#[test]
fn packet_blinding_preserves_binary_files_with_valid_utf8_bytes() {
    let scenario = scenario();
    let binary = b"\0binary header\nBuilt with fixture\nbinary tail\0";
    assert!(std::str::from_utf8(binary).is_ok());
    std::fs::write(scenario.launch_dir().join("binary"), binary).unwrap();
    scenario.launch_git(&["add", "binary"]);
    scenario.launch_git(&["commit", "-m", "Binary fixture"]);
    scenario.launch_git(&["push", "origin", "main"]);
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "codex", "model", false)]);
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let week = weeks(&scenario).remove(0);
    let repository = scenario
        .path("home/.thirdshift/evals/scoring")
        .join(week["packets"].as_str().unwrap())
        .join("0/repository");
    assert_eq!(
        std::fs::read(repository.join("binary")).unwrap(),
        binary,
        "blinding rewrote a binary repository blob"
    );
}

#[test]
fn packet_diff_includes_every_commit_in_a_rebased_pull_request() {
    let scenario = scenario();
    scenario.origin_has_commit("main", "first.txt", "first PR change\n", "First PR commit");
    scenario.origin_has_commit(
        "main",
        "second.txt",
        "second PR change\n",
        "Second PR commit",
    );
    let merged = scenario
        .origin_git(&["rev-parse", "main"])
        .trim()
        .to_string();
    let mut candidate = pr(1, "acme/widgets", "codex", "model", false);
    // A two-commit rebase merge points mergeCommit at the second commit.
    candidate["mergeCommit"] = serde_json::json!({"oid": merged});
    candidate["headRefOid"] = merged.clone().into();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([candidate]);
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let week = weeks(&scenario).remove(0);
    let packet_dir = scenario
        .path("home/.thirdshift/evals/scoring")
        .join(week["packets"].as_str().unwrap())
        .join("0");
    let packet: Value =
        serde_json::from_slice(&std::fs::read(packet_dir.join("packet.json")).unwrap()).unwrap();
    assert_eq!(
        std::fs::read_to_string(packet_dir.join("repository/first.txt")).unwrap(),
        "first PR change\n"
    );
    let diff = packet["diff"].as_str().unwrap();
    assert!(diff.contains("+second PR change"), "{diff}");
    assert!(
        diff.contains("+first PR change"),
        "first PR commit missing from merged diff: {diff}"
    );
}

#[test]
fn packet_repository_removes_attribution_for_every_harness() {
    for harness in ["claude", "codex"] {
        let scenario = scenario();
        scenario.origin_has_commit(
            "main",
            "feature.txt",
            &format!("implemented work\nBuilt with {harness} · model · high <!-- thirdshift:built-with -->\nGenerated with Claude Code\n"),
            "Feature",
        );
        let mut state = scenario.gh_state();
        state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", harness, "model", false)]);
        scenario.write_gh_state(&state);
        let run = scenario.run(&["score"]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let week = weeks(&scenario).remove(0);
        let packet_dir = scenario
            .path("home/.thirdshift/evals/scoring")
            .join(week["packets"].as_str().unwrap())
            .join("0");
        let source = std::fs::read_to_string(packet_dir.join("repository/feature.txt")).unwrap();
        assert!(source.contains("implemented work"));
        for forbidden in [
            "Built with",
            "thirdshift:built-with",
            "Generated with Claude Code",
        ] {
            assert!(
                !source.contains(forbidden),
                "{harness} packet repository leaked {forbidden}: {source}"
            );
        }
    }
}

#[test]
fn packet_blinding_preserves_source_binary_files_and_symlinks() {
    use std::os::unix::fs::PermissionsExt;
    let scenario = scenario();
    let source = "const MARKER: &str = \"<!-- thirdshift:built-with -->\";\nconst FOOTER: &str = \"Generated with Claude Code\";\n";
    scenario.commit_locally("source.rs", source, "Source containing attribution syntax");
    let binary = b"\xff\0Built with test\n";
    std::fs::write(scenario.launch_dir().join("binary"), binary).unwrap();
    std::fs::write(scenario.path("outside.txt"), "Built with outside\n").unwrap();
    std::os::unix::fs::symlink(
        scenario.path("outside.txt"),
        scenario.launch_dir().join("link"),
    )
    .unwrap();
    std::fs::write(
        scenario.launch_dir().join("executable"),
        "#!/bin/sh\n# keep me\n",
    )
    .unwrap();
    std::fs::set_permissions(
        scenario.launch_dir().join("executable"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    scenario.launch_git(&["add", "."]);
    scenario.launch_git(&["commit", "-m", "Repository file kinds"]);
    scenario.launch_git(&["push", "origin", "main"]);
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "codex", "model", false)]);
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let week = weeks(&scenario).remove(0);
    let repository = scenario
        .path("home/.thirdshift/evals/scoring")
        .join(week["packets"].as_str().unwrap())
        .join("0/repository");
    assert_eq!(
        std::fs::read_to_string(repository.join("source.rs")).unwrap(),
        source
    );
    assert_eq!(std::fs::read(repository.join("binary")).unwrap(), binary);
    assert_eq!(
        std::fs::read_link(repository.join("link")).unwrap(),
        scenario.path("outside.txt")
    );
    assert_eq!(
        std::fs::read_to_string(scenario.path("outside.txt")).unwrap(),
        "Built with outside\n"
    );
    assert_eq!(
        std::fs::metadata(repository.join("executable"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0o111
    );
}

#[test]
fn packet_diff_excludes_changes_already_on_the_pull_requests_base() {
    let scenario = scenario();
    scenario.launch_git(&["checkout", "-b", "feature"]);
    scenario.commit_locally("feature.txt", "PR change\n", "Feature");
    scenario.launch_git(&["checkout", "main"]);
    scenario.commit_locally("base.txt", "Earlier base change\n", "Base advances");
    let base = scenario
        .launch_git(&["rev-parse", "HEAD"])
        .trim()
        .to_string();
    scenario.launch_git(&["merge", "--no-ff", "feature", "-m", "Merge feature"]);
    scenario.launch_git(&["push", "origin", "main"]);
    let mut candidate = pr(1, "acme/widgets", "codex", "model", false);
    candidate["baseRefOid"] = base.into();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([candidate]);
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let week = weeks(&scenario).remove(0);
    let packet: Value = serde_json::from_slice(
        &std::fs::read(
            scenario
                .path("home/.thirdshift/evals/scoring")
                .join(week["packets"].as_str().unwrap())
                .join("0/packet.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let diff = packet["diff"].as_str().unwrap();
    assert!(diff.contains("+PR change"), "{diff}");
    assert!(!diff.contains("Earlier base change"), "{diff}");
}

fn scenario() -> Scenario {
    let scenario = Scenario::new();
    scenario.user_config_is("[scoring]\nowners = [\"acme\"]\nsample_size = 3\nmodel = 'grader-primary'\nfallback_model = 'grader-secondary'\n");
    grading::grader(&scenario);
    scenario
}

fn weeks(scenario: &Scenario) -> Vec<Value> {
    let result = scenario.run(&["scores", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    serde_json::from_str(&result.stdout).unwrap()
}

// Grading assertions inspect the private checkpoint; public views withhold
// assessments until human Scores exist.
fn saved_weeks(scenario: &Scenario) -> Vec<Value> {
    let mut saved: Vec<Value> = std::fs::read_dir(scenario.path("home/.thirdshift/evals/scoring"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
        .collect();
    saved.sort_by_key(|week| {
        DateTime::parse_from_rfc3339(week["window_end"].as_str().unwrap()).unwrap()
    });
    saved
}

fn rename_sample(scenario: &Scenario) -> std::path::PathBuf {
    let directory = scenario.path("home/.thirdshift/evals/scoring");
    let original = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let renamed = directory.join("arbitrary-retained-sample.json");
    std::fs::rename(original, &renamed).unwrap();
    renamed
}

fn pr(number: u64, repo: &str, harness: &str, model: &str, security: bool) -> Value {
    let merged = Utc::now() - chrono::TimeDelta::days(1);
    serde_json::json!({
        "number": number,
        "url": format!("https://github.com/{repo}/pull/{number}"),
        "body": format!("Closes #7\n\nBuilt with {harness} · {model} · high <!-- thirdshift:built-with -->\n"),
        "mergedAt": merged.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "headRefName": "feature",
        "repository": {"nameWithOwner": repo},
        "closingIssuesReferences": {
            "pageInfo": {"hasNextPage": false},
            "nodes": [{"url": format!("https://github.com/{repo}/issues/7"),
                "labels": {"nodes": if security {serde_json::json!([{"name": "security-fix"}])} else {serde_json::json!([])},
                    "pageInfo": {"hasNextPage": false}}}]
        }
    })
}

#[test]
fn packets_are_blind_and_keep_the_issue_and_repository_at_merge() {
    for harness in ["claude", "codex"] {
        let scenario = scenario();
        scenario.origin_has_commit(
            "main",
            "feature.txt",
            "merged work\n",
            "Secret commit message\n\nCo-Authored-By: Claude <claude@example.com>",
        );
        let merged = scenario
            .origin_git(&["rev-parse", "main"])
            .trim()
            .to_string();
        scenario.origin_has_commit("main", "later-fix.txt", "future fix\n", "Future fix");
        let mut candidate = pr(1, "acme/widgets", harness, "model", false);
        candidate["title"] = "Deliver the feature".into();
        candidate["body"] = format!("Closes #7\nBuilt with {harness} · model · high <!-- thirdshift:built-with -->\n🤖 Generated with [Claude Code](https://claude.ai/code)\nKeep this claim.\n").into();
        candidate["mergeCommit"] = serde_json::json!({"oid": merged});
        candidate["headRefOid"] = merged.clone().into();
        let at = DateTime::parse_from_rfc3339(candidate["mergedAt"].as_str().unwrap()).unwrap();
        let before = (at - chrono::TimeDelta::hours(1)).to_rfc3339();
        let after = (at + chrono::TimeDelta::hours(1)).to_rfc3339();
        let mut state = scenario.gh_state();
        state["scoring_prs"] = serde_json::json!([candidate]);
        state["titles"] = serde_json::json!({"7": "Later title"});
        state["bodies"] = serde_json::json!({"7": "Later body"});
        state["edits"] = serde_json::json!({"7": [
            {"kind":"body", "at":before, "diff":"Original request"},
            {"kind":"body", "at":after, "diff":"Later body"},
            {"kind":"title", "at":after, "previousTitle":"Original title", "currentTitle":"Later title"}
        ]});
        state["checks"] = serde_json::json!({merged: [{"name":"tests", "conclusion":"success", "url":"https://example.com/identifying-job", "id":99}]});
        scenario.write_gh_state(&state);
        let run = scenario.run(&["score"]);
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        let week = weeks(&scenario).remove(0);
        let packet_dir = scenario
            .path("home/.thirdshift/evals/scoring")
            .join(week["packets"].as_str().expect("saved packet directory"))
            .join("0");
        let text = std::fs::read_to_string(packet_dir.join("packet.json")).unwrap();
        let packet: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            packet["issues"],
            serde_json::json!([{"title":"Original title", "body":"Original request"}])
        );
        assert_eq!(packet["pull_request"]["title"], "Deliver the feature");
        assert!(
            packet["pull_request"]["body"]
                .as_str()
                .unwrap()
                .contains("Keep this claim.")
        );
        assert_eq!(
            packet["ci"],
            serde_json::json!([{"name":"tests", "conclusion":"success"}])
        );
        for forbidden in [
            "Built with",
            "thirdshift:built-with",
            "Generated with",
            "Secret commit message",
            "Co-Authored-By",
            "identifying-job",
            "Later body",
            "Later title",
        ] {
            assert!(
                !text.contains(forbidden),
                "packet leaked {forbidden}: {text}"
            );
        }
        assert!(packet["diff"].as_str().unwrap().contains("+merged work"));
        assert_eq!(
            std::fs::read_to_string(packet_dir.join("repository/feature.txt")).unwrap(),
            "merged work\n"
        );
        assert!(!packet_dir.join("repository/later-fix.txt").exists());
        assert!(
            !packet_dir.join("repository/.git").exists(),
            "Git metadata would expose commit messages and remote access"
        );
    }
}

#[test]
fn deleted_merge_time_revision_fails_without_advancing_the_window() {
    let scenario = scenario();
    let candidate = pr(1, "acme/widgets", "codex", "model", false);
    let at = candidate["mergedAt"].as_str().unwrap();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([candidate.clone()]);
    state["edits"] = serde_json::json!({"7": [
        {"kind":"body", "at":"2020-01-01T00:00:00Z", "diff":"Obsolete request"},
        {"kind":"body", "at":at, "diff":null, "deletedAt":at}
    ]});
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(
        run.stderr.contains("revision was deleted"),
        "{}",
        run.stderr
    );
    assert!(weeks(&scenario).is_empty());
    assert_eq!(
        std::fs::read_dir(scenario.path("home/.thirdshift/evals/scoring"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn packet_history_reads_past_a_page_of_later_edits() {
    let scenario = scenario();
    let candidate = pr(1, "acme/widgets", "codex", "model", false);
    let at = DateTime::parse_from_rfc3339(candidate["mergedAt"].as_str().unwrap()).unwrap();
    let mut edits: Vec<_> = (1..=101)
        .rev()
        .map(|hour| {
            serde_json::json!({
                "kind":"body", "at": (at + chrono::TimeDelta::hours(hour)).to_rfc3339(),
                "diff":"Later request"
            })
        })
        .collect();
    edits.push(serde_json::json!({"kind":"body", "at":at.to_rfc3339(), "diff":"Request at merge"}));
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([candidate]);
    state["edits"] = serde_json::json!({"7":edits});
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let week = weeks(&scenario).remove(0);
    let packet: Value = serde_json::from_slice(
        &std::fs::read(
            scenario
                .path("home/.thirdshift/evals/scoring")
                .join(week["packets"].as_str().unwrap())
                .join("0/packet.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(packet["issues"][0]["body"], "Request at merge");
}

#[test]
fn sample_spreads_across_harness_and_model_pairs_and_excludes_security_fixes() {
    let scenario = scenario();
    // Both sampled repositories use this fixture's local origin.
    scenario.set_origin_url("https://github.com/other/repository.git");
    scenario.user_config_is(
        "[scoring]\nowners = [\"acme\", \"other\", \"acme\"]\nsample_size = 3\nmodel = 'grader'\n",
    );
    let mut state = scenario.gh_state();
    let mut prs: Vec<Value> = (1..=12)
        .map(|n| pr(n, "acme/widgets", "claude", "opus", false))
        .collect();
    prs.push(pr(20, "acme/widgets", "claude", "sonnet", false));
    prs.push(pr(21, "other/repository", "codex 1.2.3", "gpt", false));
    prs.push(pr(22, "acme/widgets", "grok", "grok", true));
    prs.push(pr(23, "outsider/repository", "agy", "gemini", false));
    state["scoring_prs"] = prs.into();
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let weeks = weeks(&scenario);
    let sample = weeks[0]["sample"].as_array().unwrap();
    // The first run also includes the historical calibration draw.
    assert_eq!(sample.len(), 14, "{sample:?}");
    assert!(sample.iter().all(|pr| pr["for_you"] == true));
    let pairs: std::collections::BTreeSet<_> = sample
        .iter()
        .map(|pr| {
            (
                pr["harness"].as_str().unwrap(),
                pr["model"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        pairs,
        [("claude", "opus"), ("claude", "sonnet"), ("codex", "gpt")].into()
    );
    assert_eq!(
        sample.iter().find(|pr| pr["harness"] == "codex").unwrap()["harness_version"],
        "1.2.3"
    );
    assert!(
        sample
            .iter()
            .all(|pr| pr["number"] != 22 && pr["number"] != 23)
    );
    assert!(weeks[0]["seed"].as_u64().is_some());
    assert_eq!(weeks[0]["week"].as_str().unwrap().len(), 8);
}

#[test]
fn security_fix_on_an_issue_branch_is_excluded_without_a_closing_issue_link() {
    let scenario = scenario();
    scenario.issue_labelled(7, &["security-fix"]);
    let mut state = scenario.gh_state();
    let mut candidate = pr(10, "acme/widgets", "claude", "opus", false);
    candidate["headRefName"] = "issue-7".into();
    candidate["closingIssuesReferences"]["nodes"] = serde_json::json!([]);
    state["scoring_prs"] = serde_json::json!([candidate]);
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(weeks(&scenario)[0]["sample"], serde_json::json!([]));
}

#[test]
fn a_second_window_starts_at_the_previous_end_and_keeps_the_first_sample() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(1, "acme/widgets", "claude", "opus", false)]);
    scenario.write_gh_state(&state);
    let first_run = scenario.run(&["score"]);
    assert_eq!(first_run.code, Some(0), "{}", first_run.stderr);
    let first = weeks(&scenario).remove(0);
    assert_eq!(first["sample"].as_array().unwrap().len(), 1);
    let second_run = scenario.run(&["score"]);
    assert_eq!(second_run.code, Some(0), "{}", second_run.stderr);
    let retained = weeks(&scenario);
    assert_eq!(retained.len(), 2);
    assert!(retained.contains(&first), "first sample was overwritten");
    let second = retained.iter().find(|week| **week != first).unwrap();
    assert_eq!(second["window_start"], first["window_end"]);
    assert_eq!(second["sample"], serde_json::json!([]));
    let text = scenario.run(&["scores"]);
    assert!(
        text.stdout
            .contains("https://github.com/acme/widgets/pull/1")
    );
    assert!(text.stdout.contains(first["week"].as_str().unwrap()));
}

#[test]
fn a_live_scoring_command_is_visible_stoppable_and_excludes_a_second_command() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["on_scoring_search"] = format!(
        "touch '{}'\nsleep 5",
        scenario.path("search-started").display()
    )
    .into();
    scenario.write_gh_state(&state);
    let mut held = scenario.run_until(&["score"], &[], "search-started");
    let status = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    let command = &status["commands"][0];
    assert_eq!(command["kind"], "Scoring run");
    assert_eq!(command["state"], "live");
    let before = scenario.gh_calls().len();
    let skipped = scenario.run_from(&scenario.path("home"), &["score"]);
    assert_eq!(skipped.code, Some(0), "{}", skipped.stderr);
    assert!(skipped.stderr.contains("another Scoring run"));
    assert_eq!(scenario.gh_calls().len(), before);
    assert_eq!(scenario.log_files(LOGS, "log").len(), 1);
    let stopped = scenario.run(&["stop", command["name"].as_str().unwrap(), "--json"]);
    if stopped.code != Some(0) {
        held.signal("TERM");
    }
    let ended = held.finish();
    assert_eq!(stopped.code, Some(0), "{}", stopped.stderr);
    assert_eq!(ended.code, Some(1), "{}", ended.stderr);
    let stop: Value = serde_json::from_str(&stopped.stdout).unwrap();
    assert_eq!(stop["outcome"], "interrupted");
    assert!(weeks(&scenario).is_empty());
    assert_eq!(scenario.command_record(LOGS)["outcome"], "interrupted");
    let activity = std::fs::read_to_string(
        scenario.path("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(activity.contains("stopped from the CLI by"), "{activity}");
}

#[test]
fn a_failed_scoring_search_lands_in_needs_you_and_advances_no_window() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    state["failing"] = serde_json::json!(["api graphql"]);
    state["failing_stderr"] = "HTTP 401: Bad credentials".into();
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(weeks(&scenario).is_empty());
    assert_eq!(scenario.command_record(LOGS)["outcome"], "failed");
    let status = scenario.run(&["status", "--json"]);
    let status: Value = serde_json::from_str(&status.stdout).unwrap();
    assert_eq!(status["commands"][0]["group"], "needs you");
    assert_eq!(status["commands"][0]["needs_you_kind"], "failed");
}

#[test]
fn search_reads_every_page_before_drawing_the_sample() {
    let scenario = scenario();
    scenario
        .user_config_is("[scoring]\nowners = [\"acme\"]\nsample_size = 200\nmodel = 'grader'\n");
    let mut state = scenario.gh_state();
    state["scoring_prs"] = (1..=105)
        .map(|number| pr(number, "acme/widgets", "claude", "opus", false))
        .collect();
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let retained = weeks(&scenario);
    let sample = retained[0]["sample"].as_array().unwrap();
    assert_eq!(sample.len(), 105);
    assert!(sample.iter().any(|pr| pr["number"] == 105));
}

#[test]
fn incomplete_issue_labels_fail_without_advancing_the_window() {
    let scenario = scenario();
    let mut state = scenario.gh_state();
    let mut candidate = pr(10, "acme/widgets", "claude", "opus", false);
    candidate["closingIssuesReferences"]["nodes"][0]["labels"]["pageInfo"]["hasNextPage"] =
        true.into();
    state["scoring_prs"] = serde_json::json!([candidate]);
    scenario.write_gh_state(&state);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(1), "{}", result.stderr);
    assert!(
        result
            .stderr
            .contains("cannot verify all linked issue labels"),
        "{}",
        result.stderr
    );
    assert!(weeks(&scenario).is_empty());
}

#[test]
fn setup_completes_scoring_defaults_and_preserves_configured_choices() {
    let scenario = Scenario::new();
    scenario.user_config_is("[scoring]\nowners = [\"acme\"] # retain my owners\nsample_size = 4\nharness = 'codex'\nmodel = 'gpt'\nfallback_harness = 'claude'\nfallback_model = 'opus'\n");
    let result = scenario.run(&["setup"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let text = std::fs::read_to_string(scenario.path("home/.thirdshift/config.toml")).unwrap();
    assert!(text.contains("# retain my owners"));
    let parsed: toml::Table = text.parse().unwrap();
    assert_eq!(parsed["scoring"]["sample_size"].as_integer(), Some(4));
    assert_eq!(parsed["scoring"]["harness"].as_str(), Some("codex"));
    assert_eq!(parsed["scoring"]["model"].as_str(), Some("gpt"));
    assert_eq!(
        parsed["scoring"]["fallback_harness"].as_str(),
        Some("claude")
    );
    assert_eq!(parsed["scoring"]["fallback_model"].as_str(), Some("opus"));
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let record = scenario.command_record(LOGS);
    assert_eq!(record["harness"], "codex");
    assert_eq!(record["model"], "gpt");
}

#[test]
fn scoring_samples_and_metrics_agree_on_normalized_chronological_attribution() {
    let scenario = scenario();
    scenario.origin_has_commit("main", "feature", "work\n", "Feature");
    let records = support::scoring::continuations(
        &scenario,
        ["2026-10-19T01:00:00+02:00", "2026-10-19T00:30:00Z"],
    );
    let mut state = scenario.gh_state();
    state["scoring_prs"] = serde_json::json!([pr(23, "acme/widgets", "codex", "model", false)]);
    scenario.write_gh_state(&state);
    let run = scenario.run(&["score"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let mut week = saved_weeks(&scenario).remove(0);
    assert_eq!(
        week["sample"][0]["command_record"],
        records[1].0.to_str().unwrap()
    );
    // The read-only metrics decoder needs only settlement identities from this checkpoint.
    week["bug_origins"] = serde_json::json!([{
        "issue":scenario.issue_url(99), "state":"confirmed", "settlement":{
            "pull_request":{"url":"https://github.com/acme/widgets/pull/23", "run":null}
        }
    }]);
    std::fs::write(rename_sample(&scenario), serde_json::to_vec(&week).unwrap()).unwrap();
    let calls = scenario.gh_calls();
    let run = scenario.run(&["metrics", "--json"]);
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let metrics: Value = serde_json::from_str(&run.stdout).unwrap();
    let rows: Vec<_> = metrics["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["session_kind"].is_null())
        .collect();
    assert_eq!(
        rows.iter()
            .map(|row| row["confirmed_bug_origins"]["sum"].as_f64().unwrap())
            .sum::<f64>(),
        1.0
    );
    assert_eq!(
        rows.iter()
            .find(|row| row["confirmed_bug_origins"]["sum"] == 1.0)
            .unwrap()["week"],
        "2026-10-19"
    );
    assert_eq!(scenario.gh_calls(), calls);
    for (path, bytes) in records {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn scoring_reads_the_built_with_line_written_by_a_factory_run() {
    let scenario = scenario();
    scenario.agent_does(
        r#"
echo feature > feature.txt
git add feature.txt
git commit -q -m 'Add feature'
gh pr create --base main --head issue-7 --title 'Add feature' --body 'Closes #7'
"#,
    );
    let result = scenario.run(&[&scenario.issue_url(7)]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let mut state = scenario.gh_state();
    let mut candidate = pr(1, "acme/widgets", "claude", "opus", false);
    candidate["body"] = state["prs"][0]["body"].clone();
    candidate["url"] = state["prs"][0]["url"].clone();
    state["scoring_prs"] = serde_json::json!([candidate]);
    scenario.write_gh_state(&state);
    grading::grader(&scenario);
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let retained = weeks(&scenario);
    let sample = retained[0]["sample"].as_array().unwrap();
    assert_eq!(sample.len(), 1);
    assert_eq!(sample[0]["harness"], "claude");
    assert_eq!(sample[0]["harness_version"], "9.8.7");
    assert_eq!(sample[0]["model"], "default model");
    assert_eq!(sample[0]["effort"], "default effort");
    let record = sample[0]["command_record"]
        .as_str()
        .expect("local Command record link");
    let record: Value = serde_json::from_slice(&std::fs::read(record).unwrap()).unwrap();
    assert_eq!(record["pull_request"], sample[0]["url"]);
}

#[test]
fn first_window_looks_back_seven_days_and_is_a_recorded_command() {
    let scenario = scenario();
    let result = scenario.run(&["score"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    let weeks = weeks(&scenario);
    assert_eq!(weeks.len(), 1);
    let start = DateTime::parse_from_rfc3339(weeks[0]["window_start"].as_str().unwrap()).unwrap();
    let end = DateTime::parse_from_rfc3339(weeks[0]["window_end"].as_str().unwrap()).unwrap();
    assert_eq!((end - start).num_seconds(), 7 * 24 * 60 * 60);
    assert!((Utc::now() - end.with_timezone(&Utc)).num_seconds() < 60);
    assert!(weeks[0]["seed"].as_u64().is_some());
    assert_eq!(weeks[0]["sample"], serde_json::json!([]));
    let record = scenario.command_record(LOGS);
    assert_eq!(record["kind"], "Scoring run");
    assert_eq!(record["outcome"], "sampled");
    assert_eq!(scenario.log_files(LOGS, "log").len(), 1);
    use std::os::unix::fs::PermissionsExt;
    let sample_dir = scenario.path("home/.thirdshift/evals/scoring");
    assert_eq!(
        std::fs::metadata(&sample_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for entry in std::fs::read_dir(sample_dir).unwrap() {
        let metadata = entry.unwrap().metadata().unwrap();
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            if metadata.is_dir() { 0o700 } else { 0o600 }
        );
    }
    let activity = std::fs::read_to_string(
        scenario.path("home/.thirdshift/logs/JacobStephens2/thirdshift/activity.log"),
    )
    .unwrap();
    assert!(activity.contains("Scoring run started:"), "{activity}");
    assert!(
        activity.contains("Scoring run ended: sampled 0 pull requests"),
        "{activity}"
    );
}
