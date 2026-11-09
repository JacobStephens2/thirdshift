//! Exercise one Log inventory capture through its Snapshot projections,
//! counting the discovery each projection performs.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use super::*;
use crate::logs::reading::inventory::{EntryKind, Files, Native};

const STAMP: &str = "20261019T110000-0400";

/// Delegate to the real filesystem, counting each effect by exact path.
#[derive(Default)]
struct Counting {
    calls: Mutex<BTreeMap<(&'static str, PathBuf), usize>>,
    unavailable: Mutex<BTreeSet<PathBuf>>,
}

impl Counting {
    fn count(&self, effect: &'static str, path: &Path) {
        *self
            .calls
            .lock()
            .unwrap()
            .entry((effect, path.to_owned()))
            .or_default() += 1;
    }

    fn calls(&self) -> BTreeMap<(&'static str, PathBuf), usize> {
        self.calls.lock().unwrap().clone()
    }
}

impl Files for Counting {
    fn list(&self, path: &Path) -> std::io::Result<Vec<(PathBuf, EntryKind)>> {
        self.count("list", path);
        Native.list(path)
    }

    fn metadata(&self, path: &Path) -> std::io::Result<fs::Metadata> {
        self.count("metadata", path);
        if self.unavailable.lock().unwrap().contains(path) {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        Native.metadata(path)
    }

    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        self.count("read", path);
        Native.read(path)
    }
}

fn capture(root: &Path, files: &Arc<Counting>) -> Snapshot {
    Snapshot::capture(files.clone(), root, now(), Duration::from_secs(600)).unwrap()
}

fn folder(root: &Path) -> PathBuf {
    root.join("acme/widgets/commands/issue")
}

/// A Session trace whose modification time orders it within its Command.
fn session(root: &Path, name: &str, second: u64) -> PathBuf {
    let path = folder(root).join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "").unwrap();
    fs::File::open(&path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(second))
        .unwrap();
    path
}

fn evidence(session: &Path, command_log: Option<&Path>) {
    fs::write(
        crate::logs::owner::evidence_file(session),
        serde_json::to_vec(&json!({"command_log": command_log})).unwrap(),
    )
    .unwrap();
}

fn records(snapshot: &Snapshot) -> Vec<&Command> {
    fn walk<'a>(command: &'a Command, found: &mut Vec<&'a Command>) {
        found.push(command);
        for child in command.dispatched.iter().chain(&command.children) {
            walk(child, found);
        }
    }
    let mut found = Vec::new();
    for entry in snapshot.entries() {
        if let Entry::Record(command) = entry {
            walk(command, &mut found);
        }
    }
    found
}

fn sessions_by_name(snapshot: &Snapshot) -> BTreeMap<String, Vec<String>> {
    let name = |path: &PathBuf| path.file_name().unwrap().to_string_lossy().into_owned();
    let mut found = BTreeMap::new();
    for command in records(snapshot) {
        found.insert(
            command.record.name.clone(),
            command.sessions().unwrap().iter().map(name).collect(),
        );
    }
    for entry in snapshot.entries() {
        if let Entry::History(command) = entry {
            found.insert(
                format!("history {}", name(&command.session_logs[0])),
                Observation::History(command)
                    .sessions()
                    .unwrap()
                    .iter()
                    .map(name)
                    .collect(),
            );
        }
    }
    found
}

/// A Pickup run whose dispatched Spec run has a child Ticket, beside two
/// direct Runs, all sharing one folder and one start stamp.
fn shared_folder(root: &Path) {
    // Dispatched and child Runs reference their Sessions, as recorded Runs
    // do; Sessions sharing the outer Command's subject are discovered.
    let plan = session(root, &format!("20-{STAMP}~pass-plan.logs.jsonl"), 2);
    let implement = session(root, &format!("21-{STAMP}~pass-implement.logs.jsonl"), 3);
    let resume = session(
        root,
        &format!("21-{STAMP}~pass-implement-resume.logs.jsonl"),
        4,
    );
    session(root, &format!("pickup-{STAMP}~pass-pickup.logs.jsonl"), 1);
    let mut pass = record(0);
    pass["kind"] = "Pickup run".into();
    pass["name"] = format!("pickup-{STAMP}").into();
    pass["issue"] = Value::Null;
    let mut spec = record(20);
    spec["kind"] = "Spec run".into();
    spec["stages"] =
        json!([{"name": "plan", "began_at": "2026-10-19T11:00:00-04:00", "session_log": plan}]);
    pass["dispatched"] = json!([spec]);
    let pass_path = write(root, &format!("{STAMP}~pass.command.json"), &pass);
    let mut ticket = record(21);
    ticket["parent_record"] = json!(pass_path);
    ticket["parent_dispatch"] = "https://github.com/acme/widgets/issues/20".into();
    ticket["stages"] = json!([{"name": "implement", "began_at": "2026-10-19T11:00:00-04:00", "session_log": implement}]);
    ticket["session_log"] = json!(resume);
    write(
        root,
        &format!("{STAMP}~pass.command.dispatch-20-issue-21.child.json"),
        &ticket,
    );

    write(root, &format!("7-{STAMP}~seven.command.json"), &record(7));
    session(root, &format!("7-{STAMP}~seven-implement.logs.jsonl"), 5);
    // Explicit Model evidence attaches a tokenless Session; equal stamps alone
    // and a disjoint owner token at the same stamp do not.
    let review = session(root, &format!("7-{STAMP}-review.logs.jsonl"), 5);
    evidence(
        &review,
        Some(&folder(root).join(format!("7-{STAMP}~seven.log"))),
    );
    session(root, &format!("7-{STAMP}~other-implement.logs.jsonl"), 6);
    let mut eight = record(8);
    // A reference stays visible even when its file is missing.
    eight["stages"] = json!([{"name": "implement", "began_at": "2026-10-19T11:00:00-04:00",
        "session_log": folder(root).join("missing-implement.logs.jsonl")}]);
    write(root, &format!("8-{STAMP}~eight.command.json"), &eight);
    session(root, &format!("8-{STAMP}~eight-implement.logs.jsonl"), 7);
    session(root, &format!("8-{STAMP}-review.logs.jsonl"), 8);
    fs::write(folder(root).join("notes.log"), "unowned\n").unwrap();
}

#[test]
fn command_activity_is_shared_by_health_growth_and_watching_until_the_next_capture() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = dir.path();
    let log = folder(root).join(format!("{STAMP}~pass.log"));
    let pass_session = outside.path().join("pass.logs.jsonl");
    let spec_session = outside.path().join("spec.logs.jsonl");
    let ticket_session = outside.path().join("ticket.logs.jsonl");
    let missing = outside.path().join("missing.logs.jsonl");
    let previous = outside.path().join("previous.logs.jsonl");
    let next = outside.path().join("next.logs.jsonl");
    let mut pass = record(0);
    pass["kind"] = "Pickup run".into();
    pass["name"] = format!("pickup-{STAMP}").into();
    pass["issue"] = Value::Null;
    pass["session_log"] = json!(pass_session);
    pass["stages"] = json!([{"name": "dispatched Spec run", "began_at": pass["started_at"]}]);
    let mut spec = record(20);
    spec["kind"] = "Spec run".into();
    spec["session_log"] = json!(outside.path().join("spec.spans.jsonl"));
    spec["stages"] = json!([{"name": "Tickets", "began_at": spec["started_at"]}]);
    pass["dispatched"] = json!([spec]);
    let pass_path = write(root, &format!("{STAMP}~pass.command.json"), &pass);
    let mut ticket = record(21);
    ticket["parent_record"] = json!(pass_path);
    ticket["parent_dispatch"] = "https://github.com/acme/widgets/issues/20".into();
    // A retained raw-file reference resolves to the records file, not spans.
    ticket["session_log"] = json!(outside.path().join("ticket.jsonl"));
    ticket["stages"] =
        json!([{"name": "implement", "began_at": ticket["started_at"], "session_log": previous}]);
    let ticket_path = write(root, "ticket-21.child.json", &ticket);
    let mut sibling = ticket.clone();
    sibling["issue"] = record(22)["issue"].clone();
    sibling["name"] = "acme/widgets#22".into();
    write(root, "ticket-22.child.json", &sibling);
    let mut base_fix = record(23);
    base_fix["kind"] = "Base fix".into();
    base_fix["parent_record"] = json!(ticket_path);
    base_fix["session_log"] = json!(missing);
    base_fix["stages"] = json!([{"name": "implement", "began_at": "2026-10-19T11:55:00-04:00"}]);
    write(root, "base-fix-23.child.json", &base_fix);
    let mut ended = record(99);
    ended["outcome"] = "merged".into();
    ended["ended_at"] = "2026-10-19T11:30:00-04:00".into();
    let ended_path = write(root, "ended.command.json", &ended);
    for (path, text, age) in [
        (&log, "abc", 1200),
        (&pass_session, "pass", 1800),
        (&spec_session, "spec!", 900),
        (&ticket_session, "ticket!", 1800),
        (&previous, "old session", -60),
        (&outside.path().join("ticket.spans.jsonl"), "span", -60),
    ] {
        activity_file(path, text, age);
    }
    let files = Arc::new(Counting::default());
    let snapshot = capture(root, &files);
    assert!(
        !files
            .calls()
            .contains_key(&("metadata", command_log(&ended_path)))
    );
    let observed = snapshot
        .resolve(&format!("pickup-{STAMP}"))
        .unwrap()
        .owner
        .unwrap();
    let spec = &observed.dispatched[0];
    let ticket = &spec.children[0];
    let sibling = &spec.children[1];
    let base_fix = &ticket.children[0];
    assert!(
        matches!(observed.health, Some(Health::Ok)),
        "dispatch stages stay excluded"
    );
    assert!(matches!(
        spec.health,
        Some(Health::Silent { seconds: 900, .. })
    ));
    assert!(matches!(
        ticket.health,
        Some(Health::Silent { seconds: 1200, .. })
    ));
    assert!(matches!(base_fix.health, Some(Health::Ok)));
    assert_eq!(
        ticket.next_health_change,
        Some(now() + chrono::Duration::seconds(600))
    );
    let serialized = serde_json::to_value(ticket).unwrap();
    assert_eq!(serialized["command_log"], json!(command_log(&ticket_path)));
    assert!(serialized.get("activity").is_none());

    // Health sampled first. File and record edits cannot alter later projections.
    activity_file(&log, "abcdefgh", -60);
    fs::remove_file(&spec_session).unwrap();
    activity_file(&ticket_session, "changed", -60);
    activity_file(&missing, "available", -60);
    activity_file(&next, "new session", -60);
    let mut revised: Value = serde_json::from_slice(&fs::read(&ticket_path).unwrap()).unwrap();
    revised["session_log"] = json!(next);
    fs::write(&ticket_path, serde_json::to_vec(&revised).unwrap()).unwrap();
    for (command, session, lengths, quiet) in [
        (observed, &pass_session, (3, 4), 1200),
        (spec, &spec_session, (3, 5), 900),
        (ticket, &ticket_session, (3, 7), 1200),
        (sibling, &ticket_session, (3, 7), 1200),
        (base_fix, &missing, (3, 0), 300),
    ] {
        assert_eq!(command.activity_lengths(), lengths);
        assert_eq!(command.quiet_age(), quiet);
        assert_eq!(
            command.activity_paths().collect::<Vec<_>>(),
            [&log, session]
        );
    }
    for path in [
        &log,
        &pass_session,
        &spec_session,
        &ticket_session,
        &missing,
    ] {
        assert_eq!(
            files.calls().get(&("metadata", path.clone())),
            Some(&1),
            "{}",
            path.display()
        );
    }
    assert!(!files.calls().contains_key(&("metadata", previous)));
    assert!(
        !files
            .calls()
            .contains_key(&("metadata", outside.path().join("ticket.spans.jsonl")))
    );

    let fresh = capture(root, &files);
    let observed = fresh
        .resolve(&format!("pickup-{STAMP}"))
        .unwrap()
        .owner
        .unwrap();
    let spec = &observed.dispatched[0];
    let ticket = &spec.children[0];
    for (command, session, lengths) in [
        (observed, &pass_session, (8, 4)),
        (spec, &spec_session, (8, 0)),
        (ticket, &next, (8, 11)),
        (&spec.children[1], &ticket_session, (8, 7)),
        (&ticket.children[0], &missing, (8, 9)),
    ] {
        assert_eq!(command.activity_lengths(), lengths);
        assert_eq!(command.quiet_age(), 0, "future times clamp to zero");
        assert_eq!(
            command.activity_paths().collect::<Vec<_>>(),
            [&log, session]
        );
        assert!(matches!(command.health, Some(Health::Ok)));
    }
    assert_eq!(files.calls().get(&("metadata", log)), Some(&2));
}

fn activity_file(path: &Path, text: &str, age_seconds: i64) {
    fs::write(path, text).unwrap();
    fs::File::open(path)
        .unwrap()
        .set_modified(std::time::SystemTime::from(
            now() - chrono::Duration::seconds(age_seconds),
        ))
        .unwrap();
}

#[test]
fn unavailable_activity_uses_stage_age_and_zero_lengths_until_a_fresh_capture() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let session = outside.path().join("current.logs.jsonl");
    let mut run = record(7);
    run["session_log"] = json!(session);
    run["stages"] = json!([{"name": "implement", "began_at": run["started_at"]}]);
    let path = write(dir.path(), "7-fixture.command.json", &run);
    let log = command_log(&path);
    activity_file(&log, "abc", -60);
    activity_file(&session, "trace", -60);
    let files = Arc::new(Counting::default());
    files
        .unavailable
        .lock()
        .unwrap()
        .extend([log.clone(), session.clone()]);
    let snapshot = capture(dir.path(), &files);
    let command = snapshot.resolve("widgets#7").unwrap().owner.unwrap();
    assert!(matches!(
        command.health,
        Some(Health::Silent {
            step: 2,
            seconds: 3600,
            ..
        })
    ));
    files.unavailable.lock().unwrap().clear();
    for _ in 0..2 {
        assert_eq!(command.activity_lengths(), (0, 0));
        assert_eq!(command.quiet_age(), 3600);
        assert_eq!(
            command.activity_paths().collect::<Vec<_>>(),
            [&log, &session]
        );
    }
    for path in [&log, &session] {
        assert_eq!(files.calls().get(&("metadata", path.clone())), Some(&1));
    }
    let fresh = capture(dir.path(), &files);
    let command = fresh.resolve("widgets#7").unwrap().owner.unwrap();
    assert_eq!(command.activity_lengths(), (3, 5));
    assert_eq!(command.quiet_age(), 0);
    assert!(matches!(command.health, Some(Health::Ok)));
    for path in [&log, &session] {
        assert_eq!(files.calls().get(&("metadata", path.clone())), Some(&2));
    }
}

#[test]
fn one_capture_serves_every_session_projection_without_discovering_again() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    shared_folder(root);
    let files = Arc::new(Counting::default());
    let snapshot = capture(root, &files);
    let loaded = files.calls();
    let listed: Vec<_> = loaded
        .keys()
        .filter(|(effect, _)| *effect == "list")
        .map(|(_, path)| path.strip_prefix(root).unwrap().to_owned())
        .collect();
    assert_eq!(
        listed,
        [
            "",
            "acme",
            "acme/widgets/commands",
            "acme/widgets/commands/issue"
        ]
        .map(PathBuf::from)
    );

    let expected: BTreeMap<String, Vec<String>> = [
        (
            format!("pickup-{STAMP}"),
            vec![format!("pickup-{STAMP}~pass-pickup.logs.jsonl")],
        ),
        (
            "acme/widgets#20".into(),
            vec![format!("20-{STAMP}~pass-plan.logs.jsonl")],
        ),
        (
            "acme/widgets#21".into(),
            vec![
                format!("21-{STAMP}~pass-implement.logs.jsonl"),
                format!("21-{STAMP}~pass-implement-resume.logs.jsonl"),
            ],
        ),
        (
            "acme/widgets#7".into(),
            vec![
                format!("7-{STAMP}-review.logs.jsonl"),
                format!("7-{STAMP}~seven-implement.logs.jsonl"),
            ],
        ),
        (
            "acme/widgets#8".into(),
            vec![
                "missing-implement.logs.jsonl".into(),
                format!("8-{STAMP}~eight-implement.logs.jsonl"),
            ],
        ),
        (
            format!("history 7-{STAMP}~other-implement.logs.jsonl"),
            vec![format!("7-{STAMP}~other-implement.logs.jsonl")],
        ),
        (
            format!("history 8-{STAMP}-review.logs.jsonl"),
            vec![format!("8-{STAMP}-review.logs.jsonl")],
        ),
    ]
    .into();
    let mut rounds = Vec::new();
    for _ in 0..3 {
        assert_eq!(sessions_by_name(&snapshot), expected);
        for command in records(&snapshot) {
            Observation::Record(command).details().unwrap();
        }
        for entry in snapshot.entries() {
            match entry {
                Entry::Record(command) => {
                    crate::logs::trace::ancestry::of(command, command).unwrap();
                }
                Entry::History(command) => {
                    Observation::History(command).details().unwrap();
                }
            }
        }
        let logs = catalog::list(
            &snapshot,
            &crate::logs::accounts::Source {
                account: None,
                root: root.to_owned(),
            },
            None,
        )
        .unwrap();
        let names: Vec<_> = logs.iter().map(|log| log.name.as_str()).collect();
        assert!(
            names.contains(&"acme/widgets/commands/issue/notes.log"),
            "{names:?}"
        );
        assert!(
            !names.iter().any(|name| name.contains("missing")),
            "{names:?}"
        );
        rounds.push(files.calls());
    }
    assert_eq!(rounds[0], rounds[2], "projections repeat no discovery");
    let projected: Vec<_> = rounds[0]
        .iter()
        .filter(|(call, _)| !loaded.contains_key(call))
        .collect();
    assert!(
        projected.iter().all(|((effect, _), _)| *effect != "list"),
        "{projected:?}"
    );
    // The current Session has one health activity sample as well as its
    // independent Session-ordering lookup. Discovery still happens once.
    let current = folder(root).join(format!("21-{STAMP}~pass-implement-resume.logs.jsonl"));
    assert!(
        rounds[0].iter().all(|((effect, path), count)| {
            *count
                == if *effect == "metadata" && path == &current {
                    2
                } else {
                    1
                }
        }),
        "{:#?}",
        rounds[0]
    );
    // Every Session decodes once, with one optional Model-evidence read.
    assert_eq!(
        rounds[0]
            .keys()
            .filter(|(effect, _)| *effect == "read")
            .count(),
        9
    );
}

#[test]
fn a_fresh_capture_observes_added_removed_and_retargeted_sessions_and_revised_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, &format!("7-{STAMP}~seven.command.json"), &record(7));
    let implement = session(root, &format!("7-{STAMP}~seven-implement.logs.jsonl"), 1);
    let review = session(root, &format!("7-{STAMP}-review.logs.jsonl"), 2);
    let repair = session(root, &format!("7-{STAMP}~seven-repair-1.logs.jsonl"), 4);
    let log = folder(root).join(format!("7-{STAMP}~seven.log"));
    evidence(&review, Some(&log));
    let files = Arc::new(Counting::default());
    let first = capture(root, &files);
    let selected = first.resolve("widgets#7").unwrap().observation;
    let listed = |snapshot: &Snapshot| -> Vec<PathBuf> {
        catalog::list(
            snapshot,
            &crate::logs::accounts::Source {
                account: None,
                root: root.to_owned(),
            },
            None,
        )
        .unwrap()
        .into_iter()
        .map(|log| log.path)
        .collect()
    };
    assert_eq!(
        selected.sessions().unwrap(),
        [implement.clone(), review.clone(), repair.clone()]
    );
    assert_eq!(
        listed(&first),
        [review.clone(), implement.clone(), repair.clone()]
    );

    let resume = session(
        root,
        &format!("7-{STAMP}~seven-implement-resume.logs.jsonl"),
        3,
    );
    evidence(&review, None);
    fs::remove_file(&implement).unwrap();
    // A retargeted entry becomes a symlink, which the inventory excludes.
    let elsewhere = tempfile::tempdir().unwrap();
    let moved = elsewhere.path().join("moved.logs.jsonl");
    fs::rename(&repair, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &repair).unwrap();
    assert_eq!(
        selected.sessions().unwrap(),
        [implement.clone(), review.clone(), repair.clone()],
        "one capture stays coherent"
    );
    assert_eq!(
        listed(&first),
        [review.clone(), implement.clone(), repair.clone()]
    );
    assert_eq!(selected.session("implement").unwrap(), implement);

    let fresh = capture(root, &files);
    let selected = fresh.resolve("widgets#7").unwrap().observation;
    assert_eq!(selected.sessions().unwrap(), std::slice::from_ref(&resume));
    assert_eq!(selected.session("implement").unwrap(), resume);
    assert!(selected.session("repair").is_err());
    assert_eq!(listed(&fresh), [review.clone(), resume.clone()]);
    // Without evidence the tokenless Session is recovered on its own.
    let Some(Entry::History(history)) = fresh
        .entries()
        .iter()
        .find(|entry| matches!(entry, Entry::History(_)))
    else {
        panic!("tokenless Session is historical")
    };
    assert_eq!(history.session_logs, [review]);
}
