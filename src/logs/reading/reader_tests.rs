//! Exercise the resolved-reader interface with retained files and native identities.
use std::os::unix::ffi::OsStrExt;

use super::*;
use crate::logs::reading::Framing;

#[test]
fn disappeared_dispatched_session_retains_its_captured_writer() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("acme/widgets/commands/issue");
    let session = folder.join("30-20261019T110000-0400~removed-implement.logs.jsonl");

    let mut dispatch = record(30);
    dispatch["session_log"] = json!(session);
    let mut parent = record(7);
    parent["dispatched"] = json!([dispatch]);
    let record_path = write(
        dir.path(),
        "7-20261019T110000-0400~removed.command.json",
        &parent,
    );
    fs::write(&session, b"first\nunfinished").unwrap();

    let reader = exact(dir.path(), &session);
    let first = reader.read(0, Framing::Lines, 100).unwrap();
    assert_eq!(first.bytes, b"first\n");
    assert!(!first.done);

    parent["dispatched"] = json!([]);
    fs::write(&record_path, serde_json::to_vec(&parent).unwrap()).unwrap();

    let pending = reader.read(first.offset, Framing::Lines, 100).unwrap();
    assert!(
        !pending.done,
        "removing the observing record cannot prove captured writer exit"
    );
    assert!(pending.bytes.is_empty());
    assert_eq!(pending.offset, first.offset);
}

fn config(root: &Path) -> crate::config::UserConfig {
    let mut config = crate::config::UserConfig::parse("", &root.join("config.toml"), root).unwrap();
    config.logs_dir = root.to_owned();
    config
}

fn listed(root: &Path, path: &Path) -> Result<catalog::Log> {
    catalog::find(
        &config(root),
        &catalog::id(path.strip_prefix(root).unwrap()),
    )
}

fn exact(root: &Path, path: &Path) -> Reader {
    listed(root, path).unwrap().reader
}

#[test]
fn inventory_reads_unowned_files_by_path_bytes_and_refuses_excluded_storage() {
    let dir = tempfile::tempdir().unwrap();
    let commands = dir.path().join("acme/widgets/commands");
    let unnamed = commands.join(std::ffi::OsStr::from_bytes(b"kind-\xff"));
    fs::create_dir_all(commands.join("issue")).unwrap();
    fs::create_dir_all(&unnamed).unwrap();
    for path in [
        commands.join("issue/notes.log"),
        commands.join("issue/draft.logs.jsonl"),
        unnamed.join("notes.log"),
    ] {
        fs::write(&path, b"unowned\n").unwrap();
        let batch = exact(dir.path(), &path)
            .read(0, Framing::Lines, 100)
            .unwrap();
        assert_eq!(batch.bytes, b"unowned\n", "{}", path.display());
        assert!(batch.done, "{}", path.display());
    }
    let sessions = dir.path().join("acme/widgets/sessions");
    fs::create_dir_all(&sessions).unwrap();
    for path in [
        sessions.join("7-20261019T110000-0400-implement.logs.jsonl"),
        commands.join("issue/7-20261019T110000-0400-implement.jsonl"),
        commands.join("issue/7-20261019T110000-0400-implement.spans.jsonl"),
        commands.join("issue/7-20261019T110000-0400.notification.txt"),
        commands.join("issue/.log"),
        commands.join("loose.log"),
    ] {
        fs::write(&path, b"excluded\n").unwrap();
        assert!(listed(dir.path(), &path).is_err(), "{}", path.display());
    }
}

#[test]
fn removing_or_symlinking_an_enumerated_entry_ends_inventory_reads() {
    for replaced in [
        "acme",
        "acme/widgets",
        "acme/widgets/commands/issue",
        "acme/widgets/commands/issue/7-20261019T110000-0400~gone.log",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let record_path = write(
            dir.path(),
            "7-20261019T110000-0400~gone.command.json",
            &record(7),
        );
        let path = record_path.with_file_name("7-20261019T110000-0400~gone.log");
        fs::write(&path, b"kept\n").unwrap();
        let reader = exact(dir.path(), &path);
        assert_eq!(
            reader.read(0, Framing::Lines, 100).unwrap().bytes,
            b"kept\n"
        );
        let entry = dir.path().join(replaced);
        let moved = elsewhere.path().join("moved");
        fs::rename(&entry, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &entry).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"kept\n", "{replaced}");
        let error = reader.read(0, Framing::Lines, 100).unwrap_err();
        assert!(
            error.to_string().contains("not in the current inventory"),
            "{replaced}: {error:#}"
        );
        assert!(listed(dir.path(), &path).is_err(), "{replaced}");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "notes.log", &Value::Null);
    let reader = exact(dir.path(), &path);
    assert!(reader.read(0, Framing::Raw, 100).unwrap().done);
    fs::remove_file(&path).unwrap();
    assert!(reader.read(0, Framing::Raw, 100).is_err());
}

#[test]
fn configured_root_and_commands_aliases_stay_readable() {
    let dir = tempfile::tempdir().unwrap();
    let stored = dir.path().join("stored-commands");
    fs::create_dir_all(stored.join("issue")).unwrap();
    fs::create_dir_all(dir.path().join("real/acme/widgets")).unwrap();
    let commands = dir.path().join("real/acme/widgets/commands");
    std::os::unix::fs::symlink(&stored, &commands).unwrap();
    let root = dir.path().join("logs");
    std::os::unix::fs::symlink(dir.path().join("real"), &root).unwrap();
    let path = root.join("acme/widgets/commands/issue/notes.log");
    fs::write(&path, b"through aliases\n").unwrap();
    let reader = exact(&root, &path);
    assert_eq!(
        reader.read(0, Framing::Lines, 100).unwrap().bytes,
        b"through aliases\n"
    );
    // Listing follows a retargeted `commands` alias, so reading does too.
    let moved = dir.path().join("moved-commands");
    fs::rename(&stored, &moved).unwrap();
    fs::remove_file(&commands).unwrap();
    std::os::unix::fs::symlink(&moved, &commands).unwrap();
    let batch = reader.read(0, Framing::Lines, 100).unwrap();
    assert_eq!(batch.bytes, b"through aliases\n");
    assert!(batch.done);
}

#[test]
fn a_selected_read_ignores_an_unrelated_malformed_subtree() {
    let dir = tempfile::tempdir().unwrap();
    let record_path = write(
        dir.path(),
        "7-20261019T110000-0400~local.command.json",
        &record(7),
    );
    let path = record_path.with_file_name("7-20261019T110000-0400~local.log");
    fs::write(&path, b"first\n").unwrap();
    let reader = exact(dir.path(), &path);
    let first = reader.read(0, Framing::Lines, 100).unwrap();
    assert!(!first.done);
    fs::create_dir_all(dir.path().join("acme/other")).unwrap();
    fs::write(dir.path().join("acme/other/commands"), b"not a directory").unwrap();
    assert!(
        catalog::load(&config(dir.path()), None).is_err(),
        "whole-store loading fails on this fixture"
    );
    fs::write(&path, b"first\nsecond\n").unwrap();
    let next = reader.read(first.offset, Framing::Lines, 100).unwrap();
    assert_eq!(next.bytes, b"second\n");
    assert!(!next.done);
}

#[test]
fn raw_and_line_framing_keep_byte_offsets_and_hold_live_unfinished_lines() {
    let dir = tempfile::tempdir().unwrap();
    let record_path = write(
        dir.path(),
        "7-20261019T110000-0400~framing.command.json",
        &record(7),
    );
    let path = record_path.with_file_name("7-20261019T110000-0400~framing.log");
    fs::write(&path, b"caf\xc3\xa9\n\xfftail").unwrap();
    let reader = exact(dir.path(), &path);
    let raw = reader.read(0, Framing::Raw, 4).unwrap();
    assert_eq!(raw.bytes, b"caf\xc3");
    assert_eq!(raw.offset, 4);
    assert!(!raw.done);
    let raw = reader.read(raw.offset, Framing::Raw, 100).unwrap();
    assert_eq!(raw.bytes, b"\xa9\n\xfftail");
    assert_eq!(raw.offset, 11);
    let line = reader.read(0, Framing::Lines, 1).unwrap();
    assert_eq!(line.bytes, "café\n".as_bytes());
    assert_eq!(line.offset, 6);
    let pending = reader.read(line.offset, Framing::Lines, 100).unwrap();
    assert!(pending.bytes.is_empty());
    assert_eq!(pending.offset, 6);
    assert!(!pending.done);
    assert!(reader.read(12, Framing::Raw, 100).is_err());
    fs::write(&path, "short").unwrap();
    assert!(
        reader
            .read(6, Framing::Lines, 100)
            .unwrap_err()
            .to_string()
            .contains("truncated")
    );
}

#[test]
fn completed_files_drain_whole_lines_across_batches_including_the_final_tail() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "7-20261019T110000-0400~retained.log",
        &Value::Null,
    );
    fs::write(&path, b"first\nsecond\nfinal").unwrap();
    let reader = exact(dir.path(), &path);
    let first = reader.read(0, Framing::Lines, 2).unwrap();
    assert_eq!(first.bytes, b"first\n");
    assert_eq!(first.offset, 6);
    assert!(!first.done);
    let second = reader.read(first.offset, Framing::Lines, 7).unwrap();
    assert_eq!(second.bytes, b"second\n");
    assert_eq!(second.offset, 13);
    assert!(!second.done);
    let last = reader.read(second.offset, Framing::Lines, 5).unwrap();
    assert_eq!(last.bytes, b"final");
    assert_eq!(last.offset, 18);
    assert!(last.done);
    let end = reader.read(last.offset, Framing::Lines, 100).unwrap();
    assert!(end.bytes.is_empty() && end.done);
}

#[test]
fn captured_writer_ignores_outcome_and_new_commands_but_rejects_a_reused_pid() {
    let dir = tempfile::tempdir().unwrap();
    let mut fixture = record(7);
    fixture["outcome"] = "ready for review".into();
    let record_path = write(
        dir.path(),
        "7-20261019T110000-0400~owner.command.json",
        &fixture,
    );
    let path = record_path.with_file_name("7-20261019T110000-0400~owner.log");
    fs::write(&path, b"tail").unwrap();
    let reader = exact(dir.path(), &path);
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let selected = snapshot.resolve("widgets#7").unwrap().reader(path.clone());
    assert!(!reader.read(0, Framing::Raw, 100).unwrap().done);
    fixture["process_start"] = "reused PID".into();
    write(
        dir.path(),
        "7-20261019T110000-0400~owner.command.json",
        &fixture,
    );
    write(
        dir.path(),
        "7-20261019T110000-0400~another.command.json",
        &record(7),
    );
    assert!(!reader.read(0, Framing::Lines, 100).unwrap().done);
    assert!(!selected.read(0, Framing::Raw, 100).unwrap().done);
    let mismatched = exact(dir.path(), &path)
        .read(0, Framing::Lines, 100)
        .unwrap();
    assert_eq!(mismatched.bytes, b"tail");
    assert!(mismatched.done);
    fixture["process_start"] = "".into();
    write(
        dir.path(),
        "7-20261019T110000-0400~owner.command.json",
        &fixture,
    );
    assert!(
        exact(dir.path(), &path)
            .read(0, Framing::Raw, 100)
            .unwrap()
            .done
    );
    fs::remove_file(&path).unwrap();
    assert!(reader.read(0, Framing::Raw, 100).is_err());
}

#[test]
fn exact_session_tracks_its_current_reference_while_command_selection_keeps_outer_lifetime() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("acme/widgets/commands/issue");
    let original = folder.join("7-20261019T110000-0400~session-implement.jsonl");
    let converted = crate::logs::session::path(&original);
    let mut fixture = record(7);
    fixture["session_log"] = json!(original);
    fixture["outcome"] = "ready for review".into();
    let record_path = write(
        dir.path(),
        "7-20261019T110000-0400~session.command.json",
        &fixture,
    );
    fs::write(&converted, b"finished event\nlast event").unwrap();
    let reader = exact(dir.path(), &converted);
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let selected = snapshot
        .resolve("widgets#7")
        .unwrap()
        .reader(converted.clone());
    assert_eq!(
        reader.read(0, Framing::Lines, 100).unwrap().bytes,
        b"finished event\n"
    );
    // Losing a record cannot prove that its captured writer has stopped.
    fs::remove_file(&record_path).unwrap();
    assert!(!reader.read(0, Framing::Lines, 100).unwrap().done);
    fixture["session_log"] =
        json!(folder.join("7-20261019T110000-0400~session-implement-resume.logs.jsonl"));
    fs::write(&record_path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    let final_batch = reader.read(15, Framing::Lines, 100).unwrap();
    assert_eq!(final_batch.bytes, b"last event");
    assert!(final_batch.done);
    assert!(!selected.read(0, Framing::Raw, 100).unwrap().done);
    fs::write(&record_path, b"unreadable record").unwrap();
    assert!(reader.read(0, Framing::Lines, 100).is_err());
}

#[test]
fn exact_child_session_uses_its_own_writer_and_dispatched_sessions_use_the_pass() {
    let dir = tempfile::tempdir().unwrap();
    let mut parent = record(7);
    parent["process_start"] = "gone".into();
    let parent_path = write(
        dir.path(),
        "7-20261019T110000-0400~tree.command.json",
        &parent,
    );
    let log = parent_path.with_file_name("7-20261019T110000-0400~tree.log");
    fs::write(&log, b"outer tail").unwrap();
    let session = parent_path.with_file_name("21-20261019T110000-0400~tree-implement.logs.jsonl");
    let mut child = record(21);
    child["parent_record"] = json!(parent_path);
    child["session_log"] = json!(session);
    write(
        dir.path(),
        "7-20261019T110000-0400~tree.issue-21.child.json",
        &child,
    );
    fs::write(&session, b"child tail").unwrap();
    assert!(
        !exact(dir.path(), &session)
            .read(0, Framing::Raw, 100)
            .unwrap()
            .done
    );
    assert!(
        exact(dir.path(), &log)
            .read(0, Framing::Raw, 100)
            .unwrap()
            .done
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    assert!(
        snapshot
            .resolve("widgets#21")
            .unwrap()
            .reader(session.clone())
            .read(0, Framing::Raw, 100)
            .unwrap()
            .done
    );

    let dispatch_session =
        parent_path.with_file_name("30-20261019T110000-0400~tree-implement.logs.jsonl");
    let mut dispatch = record(30);
    dispatch["session_log"] = json!(dispatch_session);
    parent = record(7);
    parent["dispatched"] = json!([dispatch]);
    fs::write(&parent_path, serde_json::to_vec(&parent).unwrap()).unwrap();
    fs::write(&dispatch_session, b"dispatch tail").unwrap();
    let reader = exact(dir.path(), &dispatch_session);
    assert!(!reader.read(0, Framing::Lines, 100).unwrap().done);
    parent["dispatched"][0]["session_log"] = Value::Null;
    fs::write(&parent_path, serde_json::to_vec(&parent).unwrap()).unwrap();
    assert!(reader.read(0, Framing::Lines, 100).unwrap().done);
}
