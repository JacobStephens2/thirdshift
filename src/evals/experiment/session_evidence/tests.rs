use std::fs;
use std::io;

use serde_json::json;

use super::*;
use crate::evals::experiment::Bar;
use crate::evals::experiment::trial_results::files::observed::Observed;

const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
// SHA-256 of the retained protocol's original relative path:
// trials/ordinary/attempt-old/session/stdout.jsonl
const SESSION: &str = "3c4ce803b0461ee7b8a1391b704c16dc0952d0c3bf9ea5070e3886054db0cd93";

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join(ID);
    let setup = json!({"commit": "a", "harness": "claude", "model": null, "effort": null});
    fs::create_dir_all(directory.join("trials/ordinary/attempt-old/session")).unwrap();
    fs::write(
        directory.join("experiment.json"),
        serde_json::to_vec(&json!({
            "format": 1, "id": ID, "baseline": setup, "candidate": setup,
            "suites": [], "bar": Bar::default(), "prepared": null,
            "state": "completed", "created_at": "2026-10-24T10:00:00Z", "tasks": []
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        directory.join("trials/ordinary/attempt-old/session/stdout.jsonl"),
        "raw λ\n",
    )
    .unwrap();
    root
}

#[test]
fn selected_contents_need_only_saved_metadata_and_one_captured_inventory() {
    let root = fixture();
    let directory = root.path().join(ID);
    let unrelated = directory.join("trials/unrelated--baseline--0/attempt-other");
    for relative in [
        "session/stdout.jsonl",
        "session/stdout.logs.jsonl",
        "session/stdout.spans.jsonl",
        "started.json",
        "../pending.json",
        "../active.json",
    ] {
        let path = unrelated.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "malformed").unwrap();
    }
    fs::create_dir_all(directory.join("results")).unwrap();
    fs::write(directory.join("results/bad.json"), "malformed").unwrap();
    assert_eq!(
        read_from(&Native, root.path(), ID, SESSION)
            .unwrap()
            .contents,
        "raw λ\n"
    );

    let files = Observed::default();
    let metadata = directory.join("experiment.json");
    let trials = directory.join("trials");
    *files.after.borrow_mut() = Some(Box::new(move |operation, path| {
        if operation == "read" && path == metadata {
            fs::write(path, "changed after sampling").unwrap();
        } else if operation == "entries" && path == trials {
            fs::create_dir_all(path.join("later/attempt-new/session")).unwrap();
        }
    }));
    let selected = read_from(&files, root.path(), ID, SESSION).unwrap();
    assert_eq!(selected.id, SESSION);
    assert!(!selected.trace);
    assert_eq!(selected.contents, "raw λ\n");
    let counts = files.counts.borrow();
    assert!(counts.values().all(|count| *count == 1), "{counts:?}");
    let reads: Vec<_> = counts
        .keys()
        .filter(|(operation, _)| *operation == "read")
        .map(|(_, path)| path.clone())
        .collect();
    assert_eq!(
        reads,
        vec![
            directory.join("experiment.json"),
            directory.join("trials/ordinary/attempt-old/session/stdout.jsonl"),
        ]
    );
    assert!(counts.keys().all(|(operation, _)| *operation != "open"));
    assert!(!counts.contains_key(&("entries", directory.join("trials/later"))));
}

#[test]
fn converted_records_keep_the_original_id_and_span_only_families_are_unreadable() {
    let root = fixture();
    let folder = root
        .path()
        .join(ID)
        .join("trials/ordinary/attempt-old/session");
    let raw = read_from(&Native, root.path(), ID, SESSION).unwrap();
    assert!(!raw.trace);
    fs::write(folder.join("stdout.logs.jsonl"), "converted\n").unwrap();
    fs::write(
        folder.join("stdout.spans.jsonl"),
        "malformed but irrelevant",
    )
    .unwrap();
    let converted = read_from(&Native, root.path(), ID, &raw.id).unwrap();
    assert!(converted.trace);
    assert_eq!(converted.contents, "converted\n");
    fs::remove_file(folder.join("stdout.jsonl")).unwrap();
    assert_eq!(
        read_from(&Native, root.path(), ID, SESSION)
            .unwrap()
            .contents,
        "converted\n"
    );
    fs::remove_file(folder.join("stdout.logs.jsonl")).unwrap();
    assert!(read_from(&Native, root.path(), ID, SESSION).is_err());
    assert!(read_from(&Native, root.path(), ID, "not-listed").is_err());
}

#[test]
fn selected_disappearance_read_errors_and_invalid_utf8_never_fall_back_or_rescan() {
    for failure in [
        io::ErrorKind::NotFound,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::InvalidData,
    ] {
        let root = fixture();
        let folder = root
            .path()
            .join(ID)
            .join("trials/ordinary/attempt-old/session");
        let converted = folder.join("stdout.logs.jsonl");
        fs::write(
            &converted,
            if failure == io::ErrorKind::InvalidData {
                &[0xff][..]
            } else {
                b"converted"
            },
        )
        .unwrap();
        let files = Observed::default();
        let removed = converted.clone();
        *files.after.borrow_mut() = Some(Box::new(move |operation, path| {
            if operation == "canonicalize" && path == removed && failure == io::ErrorKind::NotFound
            {
                fs::remove_file(path).unwrap();
            }
        }));
        let files = if failure == io::ErrorKind::PermissionDenied {
            Observed {
                read_error: Some((converted.clone(), failure)),
                ..files
            }
        } else {
            files
        };
        assert!(read_from(&files, root.path(), ID, SESSION).is_err());
        let counts = files.counts.borrow();
        assert_eq!(counts.get(&("read", converted)), Some(&1));
        assert!(!counts.contains_key(&("read", folder.join("stdout.jsonl"))));
        assert!(counts.values().all(|count| *count == 1), "{counts:?}");
    }
}

#[test]
fn saved_experiment_validation_is_shared_with_ordinary_loads() {
    let root = fixture();
    for invalid in ["../escape", "not-an-id", &"g".repeat(64), &"b".repeat(64)] {
        assert!(read_from(&Native, root.path(), invalid, SESSION).is_err());
        assert!(Stored::load(root.path(), invalid, false).is_err());
    }
    let path = root.path().join(ID).join("experiment.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (field, value) in [("format", json!(2)), ("id", json!("b".repeat(64)))] {
        let mut corrupt = original.clone();
        corrupt[field] = value;
        fs::write(&path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
        assert!(read_from(&Native, root.path(), ID, SESSION).is_err());
        assert!(Stored::load(root.path(), ID, false).is_err());
    }
    fs::remove_file(&path).unwrap();
    assert!(read_from(&Native, root.path(), ID, SESSION).is_err());
}

#[test]
fn escaped_session_directories_and_files_are_unavailable() {
    let root = fixture();
    let folder = root
        .path()
        .join(ID)
        .join("trials/ordinary/attempt-old/session");
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("stdout.jsonl"), "secret").unwrap();
    let files = Observed::default();
    let escaped = folder.clone();
    let target = outside.path().join("stdout.jsonl");
    // Capture a regular file's type, then replace it before confinement.
    *files.after.borrow_mut() = Some(Box::new(move |operation, path| {
        if operation == "entries" && path == escaped {
            fs::remove_file(path.join("stdout.jsonl")).unwrap();
            std::os::unix::fs::symlink(&target, path.join("stdout.jsonl")).unwrap();
        }
    }));
    assert!(read_from(&files, root.path(), ID, SESSION).is_err());
    assert!(
        !files
            .counts
            .borrow()
            .contains_key(&("read", folder.join("stdout.jsonl")))
    );
    fs::remove_dir_all(&folder).unwrap();
    std::os::unix::fs::symlink(outside.path(), &folder).unwrap();
    assert!(read_from(&Native, root.path(), ID, SESSION).is_err());
}

#[test]
fn lexical_trial_entries_and_real_attempt_directories_keep_their_membership_rules() {
    let root = fixture();
    let directory = root.path().join(ID);
    let trial = directory.join("trials/ordinary");
    let retained = directory.join("retained-trial");
    fs::rename(&trial, &retained).unwrap();
    // Trial entries have never required a particular name or a real directory.
    // A lexical link still hashes to the original ID when its contents stay confined.
    std::os::unix::fs::symlink(&retained, &trial).unwrap();
    assert_eq!(
        read_from(&Native, root.path(), ID, SESSION)
            .unwrap()
            .contents,
        "raw λ\n"
    );
    fs::rename(retained.join("attempt-old"), retained.join("other")).unwrap();
    std::os::unix::fs::symlink(retained.join("other"), retained.join("attempt-old")).unwrap();
    assert!(read_from(&Native, root.path(), ID, SESSION).is_err());
}

#[test]
fn disappeared_discoveries_are_skipped_and_other_directory_errors_propagate() {
    let root = fixture();
    let folder = root
        .path()
        .join(ID)
        .join("trials/ordinary/attempt-old/session");
    let files = Observed::default();
    let removed = folder.clone();
    *files.after.borrow_mut() = Some(Box::new(move |operation, path| {
        if operation == "entries" && path == removed {
            fs::remove_file(path.join("stdout.jsonl")).unwrap();
        }
    }));
    assert_eq!(
        read_from(&files, root.path(), ID, SESSION)
            .err()
            .unwrap()
            .to_string(),
        "Session not found"
    );
    assert!(files.counts.borrow().values().all(|count| *count == 1));
    assert!(
        !files
            .counts
            .borrow()
            .contains_key(&("read", folder.join("stdout.jsonl")))
    );
    fs::write(
        root.path().join(ID).join("trials/not-a-directory"),
        "invalid layout",
    )
    .unwrap();
    let error = read_from(&Native, root.path(), ID, SESSION).err().unwrap();
    assert!(error.downcast_ref::<io::Error>().is_some());
}
