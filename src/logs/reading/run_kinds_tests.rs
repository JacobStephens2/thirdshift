use super::*;

const STAMP: &str = "20261019T110000-0400";

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
fn triage_and_review_names_select_their_commands() {
    for (prefix, kind, session) in [
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
    ] {
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
fn triage_and_review_render_in_inbox_history_and_catalog() {
    use crate::logs::status::{ArchivePage, archive, inbox};

    let dir = tempfile::tempdir().unwrap();
    let mut expected = Vec::new();
    for (prefix, kind, session) in [
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
    ] {
        expected.push(command(dir.path(), prefix, kind, &[session]));
    }
    let mut config =
        crate::config::UserConfig::parse("", &dir.path().join("config.toml"), dir.path()).unwrap();
    config.logs_dir = dir.path().to_owned();
    // Keep the fixed fixture in the Inbox as the wall clock advances.
    config.inbox_days = (Local::now() - now()).num_days().max(0) + 7;
    let inbox = inbox(&config).unwrap();
    let rows: Vec<_> = inbox
        .sections
        .iter()
        .flat_map(|section| &section.rows)
        .collect();
    let history = archive(&config, ArchivePage::History, "", "").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(history.rows.len(), 2);
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
