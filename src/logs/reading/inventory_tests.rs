//! Exercise one Log inventory capture through its Snapshot projections,
//! counting the discovery each projection performs.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use super::*;
use crate::logs::reading::inventory::{Files, Native, Type};

const STAMP: &str = "20261019T110000-0400";

/// Delegate to the real filesystem, counting each effect by exact path.
#[derive(Default)]
struct Counting {
    calls: Mutex<BTreeMap<(&'static str, PathBuf), usize>>,
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
    fn list(&self, path: &Path) -> std::io::Result<Vec<(PathBuf, Type)>> {
        self.count("list", path);
        Native.list(path)
    }

    fn metadata(&self, path: &Path) -> std::io::Result<fs::Metadata> {
        self.count("metadata", path);
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
        let logs = catalog::list(&snapshot, root, None).unwrap();
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
    assert!(
        rounds[0].values().all(|count| *count == 1),
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
fn a_fresh_capture_observes_added_sessions_and_revised_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, &format!("7-{STAMP}~seven.command.json"), &record(7));
    let implement = session(root, &format!("7-{STAMP}~seven-implement.logs.jsonl"), 1);
    let review = session(root, &format!("7-{STAMP}-review.logs.jsonl"), 2);
    let log = folder(root).join(format!("7-{STAMP}~seven.log"));
    evidence(&review, Some(&log));
    let files = Arc::new(Counting::default());
    let first = capture(root, &files);
    let selected = first.resolve("widgets#7").unwrap().observation;
    assert_eq!(
        selected.sessions().unwrap(),
        [implement.clone(), review.clone()]
    );

    let resume = session(
        root,
        &format!("7-{STAMP}~seven-implement-resume.logs.jsonl"),
        3,
    );
    evidence(&review, None);
    fs::remove_file(&implement).unwrap();
    assert_eq!(
        selected.sessions().unwrap(),
        [implement.clone(), review.clone()],
        "one capture stays coherent"
    );
    assert_eq!(selected.session("implement").unwrap(), implement);

    let fresh = capture(root, &files);
    let selected = fresh.resolve("widgets#7").unwrap().observation;
    assert_eq!(selected.sessions().unwrap(), std::slice::from_ref(&resume));
    assert_eq!(selected.session("implement").unwrap(), resume);
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
