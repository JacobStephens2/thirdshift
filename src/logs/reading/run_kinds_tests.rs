use super::*;

const STAMP: &str = "20261019T110000-0400";
const COMMAND_CASES: [(&str, &str, &str); 3] = [
    (
        "triage",
        "Triage run",
        "triage-20261019T110000-0400~owner-triage",
    ),
    (
        "review",
        "Review run",
        "process-step-1-20261019T110000-0400~owner-code-review",
    ),
    (
        "release",
        "Release run",
        "release-20261019T110000-0400~owner-release-summary",
    ),
];

fn command(root: &Path, prefix: &str, kind: &str, sessions: &[&str]) -> (PathBuf, Vec<PathBuf>) {
    let folder = root.join("acme/widgets/commands").join(prefix);
    fs::create_dir_all(&folder).unwrap();
    let log = folder.join(format!("{STAMP}~owner.log"));
    fs::write(&log, "Command output\n").unwrap();
    let sessions: Vec<_> = sessions
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let path = folder.join(format!("{name}.logs.jsonl"));
            fs::write(&path, "").unwrap();
            fs::File::open(&path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(index as u64))
                .unwrap();
            path
        })
        .collect();
    let mut value = ended(record(0), "succeeded");
    value["kind"] = kind.into();
    value["name"] = format!("{prefix}-{STAMP}").into();
    value["issue"] = Value::Null;
    value["issue_branch"] = Value::Null;
    value["stages"] = json!([{
        "name": "Session", "began_at": "2026-10-19T11:00:00-04:00", "session_log": sessions[0]
    }]);
    let path = log.with_extension("command.json");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    (path, sessions)
}

#[test]
fn triage_discovers_sessions_beyond_stage_references() {
    let dir = tempfile::tempdir().unwrap();
    let (path, sessions) = command(
        dir.path(),
        "triage",
        "Triage run",
        &[
            "triage-20261019T110000-0400~owner-triage",
            "triage-20261019T110000-0400~owner-triage-resume",
        ],
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let selected = snapshot.resolve(path.to_str().unwrap()).unwrap();
    assert_eq!(selected.observation.sessions().unwrap(), sessions);
}

#[test]
fn a_failed_release_leaves_needs_you_when_its_bump_pull_request_closes() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://github.com/acme/widgets/pull/23";
    let mut release = ended(record(0), "failed");
    release["kind"] = "Release run".into();
    release["name"] = format!("release-{STAMP}").into();
    release["issue"] = Value::Null;
    release["pull_request"] = url.into();
    write(dir.path(), "release.command.json", &release);
    let kept = kept_answer(
        dir.path(),
        json!({"version": 2, "subjects": {url: {
            "state": "Closed", "merged_at": null,
            "closed_at": "2026-10-19T11:45:00-04:00"
        }}, "answered_at": {url: "2026-10-19T11:45:00-04:00"}}),
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let rows = snapshot.evaluate_inbox(7, &kept).into_rows(true);
    let row = serde_json::to_value(&rows[0]).unwrap();
    assert_eq!(row["group"], "done");
    assert_eq!(row["left_reason"], "pull request closed");
}

#[test]
fn release_failures_wait_until_a_later_start_or_newer_success_without_a_bump_pull_request() {
    for (start, outcome, group) in [
        ("2026-10-19T11:45:00-04:00", None, "done"),
        ("2026-10-19T11:45:00-04:00", Some("merged"), "done"),
        ("2026-10-19T11:15:00-04:00", None, "needs you"),
        ("2026-10-19T11:15:00-04:00", Some("failed"), "needs you"),
        ("2026-10-19T11:15:00-04:00", Some("merged"), "done"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut release = record(0);
        release["kind"] = "Release run".into();
        release["name"] = format!("release-{STAMP}").into();
        release["issue"] = Value::Null;
        let path = write(
            dir.path(),
            "release-old.command.json",
            &ended(release.clone(), "failed"),
        );
        release["name"] = "release-20261019T114500-0400".into();
        release["started_at"] = start.into();
        if let Some(outcome) = outcome {
            release["outcome"] = outcome.into();
            release["ended_at"] = "2026-10-19T11:50:00-04:00".into();
        }
        write(dir.path(), "release-new.command.json", &release);

        let snapshot = Snapshot::load(dir.path(), now()).unwrap();
        let selected = snapshot.resolve(path.to_str().unwrap()).unwrap();
        let selector = selected.observation.selector().unwrap();
        let row = snapshot
            .evaluate_inbox(7, &crate::logs::github::Kept::default())
            .into_target(&selector)
            .unwrap();
        let row = serde_json::to_value(&row).unwrap();
        assert_eq!(row["group"], group, "start {start}, outcome {outcome:?}");
        if group == "done" {
            assert_eq!(row["left_reason"], "taken up by a later Command");
        } else {
            assert!(row["left_reason"].is_null());
        }
    }
}

#[test]
fn an_interrupted_release_still_needs_the_day_shift_without_a_stop_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut release = ended(record(0), "interrupted");
    release["kind"] = "Release run".into();
    release["issue"] = Value::Null;
    write(dir.path(), "release.command.json", &release);

    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let rows = snapshot
        .evaluate_inbox(7, &crate::logs::github::Kept::default())
        .into_rows(true);
    let row = serde_json::to_value(&rows[0]).unwrap();
    assert_eq!(row["group"], "needs you");
    assert_eq!(row["needs_you_kind"], "failed");
}

#[test]
fn review_discovers_all_steps_and_resumes_for_its_exact_owner() {
    let dir = tempfile::tempdir().unwrap();
    let (path, sessions) = command(
        dir.path(),
        "review",
        "Review run",
        &[
            "process-step-1-20261019T110000-0400~owner-code-review",
            "process-step-1-20261019T110000-0400~owner-code-review-resume",
            "process-step-2-20261019T110000-0400~owner-security-review",
            "process-step-3-20261019T110000-0400~owner-process-step-3",
        ],
    );
    for name in [
        "process-step-1-20261019T110000-0400~other-code-review.logs.jsonl",
        "process-step-1-20261019T110000-0400-code-review.logs.jsonl",
    ] {
        fs::write(path.parent().unwrap().join(name), "unrelated").unwrap();
    }
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let selected = snapshot.resolve(path.to_str().unwrap()).unwrap();
    assert_eq!(selected.observation.sessions().unwrap(), sessions);
}

#[test]
fn session_command_names_select_their_commands() {
    for (prefix, kind, session) in COMMAND_CASES {
        let dir = tempfile::tempdir().unwrap();
        let (path, sessions) = command(dir.path(), prefix, kind, &[session]);
        let snapshot = Snapshot::load(dir.path(), now()).unwrap();
        let name = format!("{prefix}-{STAMP}");
        let selected = snapshot.resolve(&name).unwrap();
        assert_eq!(selected.owner.unwrap().record_path, path);
        assert_eq!(selected.observation.sessions().unwrap(), sessions);
        assert_eq!(
            selected.observation.command_log(),
            Some(
                path.with_file_name("20261019T110000-0400~owner.log")
                    .as_path()
            )
        );
    }
}

#[test]
fn session_commands_render_in_inbox_history_and_catalog() {
    use crate::logs::status::{ArchivePage, Read};

    let dir = tempfile::tempdir().unwrap();
    let mut expected = Vec::new();
    for (prefix, kind, session) in COMMAND_CASES {
        expected.push(command(dir.path(), prefix, kind, &[session]));
    }
    let mut config =
        crate::config::UserConfig::parse("", &dir.path().join("config.toml"), dir.path()).unwrap();
    config.logs_dir = dir.path().to_owned();
    // Keep the fixed fixture in the Inbox as the wall clock advances.
    config.inbox_days = (Local::now() - now()).num_days().max(0) + 7;
    let read = Read::capture(&config, false).unwrap();
    let inbox = read.inbox(&config).unwrap();
    let rows: Vec<_> = inbox
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .collect();
    let history = read
        .archive(&config, ArchivePage::History, "", "", None)
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(history.rows.len(), 3);
    for (_, sessions) in &expected {
        assert!(rows.iter().any(|row| row.sessions == *sessions));
        assert!(history.rows.iter().any(|row| row.sessions == *sessions));
    }

    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let source = crate::logs::accounts::Source {
        account: None,
        root: dir.path().to_owned(),
    };
    for (path, sessions) in expected {
        snapshot
            .resolve(path.to_str().unwrap())
            .unwrap()
            .observation
            .details()
            .unwrap();
        let log = path.with_file_name("20261019T110000-0400~owner.log");
        let owner = source.id(&log).unwrap();
        let listed = catalog::list(&snapshot, &source, Some(&owner)).unwrap();
        let paths: std::collections::BTreeSet<_> = listed.into_iter().map(|log| log.path).collect();
        assert_eq!(paths, sessions.into_iter().chain([log]).collect());
    }
}
