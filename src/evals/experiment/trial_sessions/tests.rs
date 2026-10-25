use std::fs;

use super::*;
use crate::evals::experiment::trial_results::files::{Native, observed::Observed};

#[test]
fn discovery_scopes_share_ordered_owned_session_families_without_reading_contents() {
    let root = tempfile::tempdir().unwrap();
    let trial = root.path().join("trials/baseline");
    let older = trial.join("attempt-01");
    let later = trial.join("attempt-02");
    for attempt in [&later, &older] {
        fs::create_dir_all(attempt.join("session")).unwrap();
        fs::write(attempt.join("started.json"), "unrelated malformed metadata").unwrap();
    }
    fs::write(trial.join("pending.json"), "malformed").unwrap();
    for name in [
        "stdout.jsonl",
        "stdout.logs.jsonl",
        "stdout.spans.jsonl",
        "resume-1.jsonl",
        "orphan.spans.jsonl",
        "ignored.txt",
    ] {
        fs::write(older.join("session").join(name), "not decoded").unwrap();
    }
    fs::write(later.join("session/stdout.logs.jsonl"), "converted only").unwrap();
    fs::create_dir_all(trial.join("not-an-attempt/session")).unwrap();
    std::os::unix::fs::symlink(&older, trial.join("attempt-symlink")).unwrap();
    let files = Observed::default();
    let captured = experiment(&files, root.path()).unwrap();
    assert_eq!(
        captured.iter().map(|a| &a.path).collect::<Vec<_>>(),
        vec![&older, &later]
    );
    let counts = files.counts.borrow().clone();
    assert!(
        counts
            .iter()
            .all(|((operation, _), count)| *operation == "entries" && *count == 1)
    );

    // Facts own the lexical identities and selections, even after deletion.
    fs::remove_file(older.join("session/stdout.logs.jsonl")).unwrap();
    fs::write(older.join("session/resume-2.spans.jsonl"), "later").unwrap();
    let sessions = &captured[0].sessions;
    assert_eq!(
        sessions
            .iter()
            .map(|s| s.original.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["orphan.jsonl", "resume-1.jsonl", "stdout.jsonl"]
    );
    assert_eq!(sessions[0].readable(), None);
    assert_eq!(
        sessions[0].spans.as_deref(),
        Some(older.join("session/orphan.spans.jsonl").as_path())
    );
    assert_eq!(
        sessions[1].readable(),
        Some(older.join("session/resume-1.jsonl").as_path())
    );
    assert_eq!(sessions[1].spans, None);
    assert_eq!(
        sessions[2].readable(),
        Some(older.join("session/stdout.logs.jsonl").as_path())
    );
    assert_eq!(
        sessions[2].raw.as_deref(),
        Some(older.join("session/stdout.jsonl").as_path())
    );
    assert_eq!(
        sessions[2].spans.as_deref(),
        Some(older.join("session/stdout.spans.jsonl").as_path())
    );
    assert_eq!(*files.counts.borrow(), counts);

    let fresh = self::trial(&Native, &trial).unwrap();
    assert_eq!(fresh.len(), 2);
    assert_eq!(fresh[0].sessions.len(), 4);
    assert_eq!(
        fresh[0].sessions[3].readable(),
        Some(older.join("session/stdout.jsonl").as_path())
    );
    let local = attempt(&Native, &older).unwrap();
    assert_eq!(local, fresh[0]);
    assert_eq!(captured[1], fresh[1]);
}

#[test]
fn readable_files_and_span_files_keep_their_distinct_type_rules() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("session");
    fs::create_dir(&folder).unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), folder.join("raw.jsonl")).unwrap();
    std::os::unix::fs::symlink(outside.path(), folder.join("linked.spans.jsonl")).unwrap();
    fs::create_dir(folder.join("directory.spans.jsonl")).unwrap();
    fs::create_dir(folder.join("ignored.logs.jsonl")).unwrap();
    let captured = attempt(&Native, root.path()).unwrap();
    assert_eq!(captured.sessions.len(), 2);
    assert!(captured.sessions.iter().all(|s| s.readable().is_none()));
    assert_eq!(
        captured.sessions[0].spans.as_deref(),
        Some(folder.join("directory.spans.jsonl").as_path())
    );
    assert_eq!(
        captured.sessions[1].spans.as_deref(),
        Some(folder.join("linked.spans.jsonl").as_path())
    );
}

#[test]
fn missing_folders_are_sampled_once_and_other_directory_errors_propagate() {
    let root = tempfile::tempdir().unwrap();
    let files = Observed::default();
    *files.after.borrow_mut() = Some(Box::new(|operation, path| {
        if operation == "entries" {
            fs::create_dir_all(path.join("trial/attempt-new/session")).unwrap();
        }
    }));
    let captured = experiment(&files, root.path()).unwrap();
    assert!(captured.is_empty());
    assert_eq!(files.counts.borrow().len(), 1);
    assert!(files.counts.borrow().values().all(|count| *count == 1));
    assert_eq!(experiment(&Native, root.path()).unwrap().len(), 1);
    fs::write(root.path().join("trials/not-a-directory"), "invalid layout").unwrap();
    assert!(experiment(&Native, root.path()).is_err());
}
