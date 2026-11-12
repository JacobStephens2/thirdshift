use super::*;
use std::cell::{Cell, RefCell};
use std::io::{Error, ErrorKind};

#[derive(Default)]
struct ScriptedFiles {
    entries: Option<Vec<PathBuf>>,
    contents: RefCell<BTreeMap<PathBuf, Result<Vec<u8>, ErrorKind>>>,
    listings: Cell<usize>,
    reads: RefCell<BTreeMap<PathBuf, usize>>,
}

impl SavedFiles for ScriptedFiles {
    fn list(&self, _: &Path) -> std::io::Result<Vec<PathBuf>> {
        self.listings.set(self.listings.get() + 1);
        self.entries
            .clone()
            .ok_or_else(|| ErrorKind::PermissionDenied.into())
    }

    fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        *self.reads.borrow_mut().entry(path.to_owned()).or_default() += 1;
        self.contents
            .borrow()
            .get(path)
            .cloned()
            .unwrap_or(Err(ErrorKind::NotFound))
            .map_err(Error::from)
    }
}

fn config(logs: &Path, names: &[&str]) -> UserConfig {
    let mut config = UserConfig::parse("", &logs.join("config.toml"), logs).unwrap();
    config.logs_dir = logs.to_owned();
    config.console.fleet = names
        .iter()
        .map(|name| Entry::link((*name).to_owned(), "http://127.0.0.1:1".into()))
        .collect();
    config
}

fn bytes(name: &str, slot: Slot<'_>, at: DateTime<Utc>, view: &View) -> Vec<u8> {
    serde_json::to_vec(&Saved {
        version: 1,
        machine: name.into(),
        person: slot.person().map(str::to_owned),
        last_seen: at,
        pushed_at: slot.reported().then_some(at),
        views: BTreeMap::from([(
            view.query.clone(),
            SavedView {
                fetched_at: at,
                filters: Some(view.filters.clone()),
                weekly: Some(view.weekly),
                summary: serde_json::to_value(summary(name, window().current().unwrap(), 2, 0))
                    .unwrap(),
            },
        )]),
    })
    .unwrap()
}

fn asked(name: &str) -> MetricsArgs {
    MetricsArgs {
        machine: Some(name.into()),
        window: Some(crate::args::MetricsWindow::Days7),
        ..MetricsArgs::default()
    }
}

#[test]
fn joint_projections_observe_each_path_once_including_negative_results() {
    let logs = tempfile::tempdir().unwrap();
    let config = config(
        logs.path(),
        &[
            "capture-good",
            "capture-missing",
            "capture-denied",
            "capture-broken",
            "capture-version",
        ],
    );
    let at = today().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let good = path(logs.path(), "capture-good", Slot::Reported);
    let retired = logs.path().join(CACHE).join("unexpected.json");
    let ignored = logs.path().join(CACHE).join("ignored.lock");
    let mut entries = vec![good.clone(), good.clone(), retired.clone(), ignored.clone()];
    let files = ScriptedFiles::default();
    files.contents.borrow_mut().insert(
        good.clone(),
        Ok(bytes(
            "capture-good",
            Slot::Reported,
            at,
            &view("window=7d", ""),
        )),
    );
    files.contents.borrow_mut().insert(
        retired.clone(),
        Ok(bytes(
            "retired",
            Slot::Read(None),
            at,
            &view("window=7d", ""),
        )),
    );
    for (name, result) in [
        ("capture-denied", Err(ErrorKind::PermissionDenied)),
        ("capture-broken", Ok(b"{broken".to_vec())),
        ("capture-version", {
            let mut value: Value = serde_json::from_slice(&bytes(
                "capture-version",
                Slot::Reported,
                at,
                &view("window=7d", ""),
            ))
            .unwrap();
            value["version"] = serde_json::json!(2);
            Ok(serde_json::to_vec(&value).unwrap())
        }),
    ] {
        let path = path(logs.path(), name, Slot::Reported);
        entries.push(path.clone());
        files.contents.borrow_mut().insert(path, result);
    }
    let files = ScriptedFiles {
        entries: Some(entries),
        ..files
    };
    let capture = Capture::with_filesystem(&config, None, &files);
    assert!(capture.default_view());
    assert_eq!(
        capture.others(),
        [
            "capture-good",
            "capture-missing",
            "capture-denied",
            "capture-broken",
            "capture-version",
            "retired"
        ]
    );
    // Replace successful and failed observations after the first projections.
    files
        .contents
        .borrow_mut()
        .insert(good, Err(ErrorKind::NotFound));
    files
        .contents
        .borrow_mut()
        .insert(retired, Err(ErrorKind::NotFound));
    for name in [
        "capture-missing",
        "capture-denied",
        "capture-broken",
        "capture-version",
    ] {
        files.contents.borrow_mut().insert(
            path(logs.path(), name, Slot::Reported),
            Ok(bytes(name, Slot::Reported, at, &view("window=7d", ""))),
        );
        assert_eq!(
            capture.load(&asked(name)).unwrap().machines[0].status,
            Status::LinkOnly
        );
    }
    for name in ["capture-good", "retired"] {
        let loaded = capture.load(&asked(name)).unwrap();
        assert_eq!(loaded.machines[0].last_seen, Some(at));
    }
    assert!(capture.load(&asked("unknown")).is_err());
    assert_eq!(files.listings.get(), 1);
    let reads = files.reads.borrow().clone();
    assert_eq!(reads.len(), 11); // ten exact slots plus the unexpected filename
    assert!(reads.values().all(|count| *count == 1), "{reads:?}");
    assert!(!reads.contains_key(&ignored));
    let fresh = Capture::with_filesystem(&config, None, &files);
    assert_eq!(
        fresh.load(&asked("capture-good")).unwrap().machines[0].status,
        Status::LinkOnly
    );
    assert!(fresh.load(&asked("retired")).is_err());
    for name in [
        "capture-missing",
        "capture-denied",
        "capture-broken",
        "capture-version",
    ] {
        assert_eq!(
            fresh.load(&asked(name)).unwrap().machines[0].status,
            Status::Reported
        );
    }
    assert_eq!(files.listings.get(), 2);
    assert!(files.reads.borrow().values().all(|count| *count == 2));
}

#[test]
fn exact_slots_work_when_listing_fails_without_becoming_discovery() {
    let logs = tempfile::tempdir().unwrap();
    let config = config(logs.path(), &["capture-link"]);
    let at = today().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let files = ScriptedFiles::default();
    files.contents.borrow_mut().insert(
        path(logs.path(), "capture-link", Slot::Reported),
        Ok(bytes(
            "capture-link",
            Slot::Reported,
            at,
            &view("window=7d", ""),
        )),
    );
    files.contents.borrow_mut().insert(
        path(logs.path(), "capture-link", Slot::Read(None)),
        Ok(bytes(
            "capture-link",
            Slot::Read(None),
            at + chrono::Duration::seconds(1),
            &view("window=7d", ""),
        )),
    );
    let capture = Capture::with_filesystem(&config, None, &files);
    assert!(
        !capture.default_view(),
        "exact reads alone do not satisfy the discovery predicate"
    );
    assert_eq!(capture.others(), ["capture-link"]);
    let loaded = capture.load(&asked("capture-link")).unwrap();
    assert_eq!(loaded.machines[0].status, Status::Reported);
    assert_eq!(
        loaded.machines[0].last_seen,
        Some(at),
        "a link uses its report even with a newer personal read"
    );
    assert_eq!(files.listings.get(), 1);
    assert_eq!(files.reads.borrow().len(), 2);
    assert!(files.reads.borrow().values().all(|count| *count == 1));
}

#[test]
fn discovery_ties_keep_enumeration_order_and_choose_the_slot_before_the_view() {
    let logs = tempfile::tempdir().unwrap();
    let config = config(logs.path(), &["capture-configured"]);
    let at = today().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let first = logs.path().join(CACHE).join("first.json");
    let second = logs.path().join(CACHE).join("second.json");
    let unrelated = logs.path().join(CACHE).join("unrelated.json");
    let local = crate::console::fleet_control::local_name(&config);
    let local_path = logs.path().join(CACHE).join("local.json");
    let mismatched = path(logs.path(), "capture-configured", Slot::Reported);
    let mut files = ScriptedFiles {
        entries: Some(vec![
            first.clone(),
            second.clone(),
            unrelated.clone(),
            local_path.clone(),
            mismatched.clone(),
        ]),
        ..ScriptedFiles::default()
    };
    files.contents.get_mut().extend([
        (
            first.clone(),
            Ok(bytes(
                "z-retired",
                Slot::Read(None),
                at,
                &view("window=7d", ""),
            )),
        ),
        (
            second.clone(),
            Ok(bytes(
                "z-retired",
                Slot::Reported,
                at,
                &view("window=30d", "kind=Run"),
            )),
        ),
        (
            unrelated,
            Ok(bytes(
                "hidden",
                Slot::Read(Some("another@example.test")),
                at,
                &view("window=7d", ""),
            )),
        ),
        (
            local_path,
            Ok(bytes(&local, Slot::Reported, at, &view("window=7d", ""))),
        ),
        (
            mismatched,
            Ok(bytes(
                "a-retired",
                Slot::Reported,
                at,
                &view("window=7d", ""),
            )),
        ),
    ]);
    let capture = Capture::with_filesystem(&config, None, &files);
    assert_eq!(
        capture.others(),
        ["capture-configured", "a-retired", "z-retired"]
    );
    assert!(capture.load(&asked("hidden")).is_err());
    assert!(capture.load(&asked(&local)).is_err());
    assert_eq!(
        capture.load(&asked("capture-configured")).unwrap().machines[0].status,
        Status::LinkOnly,
        "an unexpected payload cannot fill an exact configured slot"
    );
    assert_eq!(
        capture.load(&asked("a-retired")).unwrap().machines[0].status,
        Status::Reported
    );
    assert_eq!(
        capture.load(&asked("z-retired")).unwrap().machines[0].status,
        Status::Cached
    );
    files.entries.as_mut().unwrap().swap(0, 1);
    let reversed = Capture::with_filesystem(&config, None, &files)
        .load(&asked("z-retired"))
        .unwrap();
    assert_eq!(reversed.machines[0].status, Status::Reported);
    assert!(reversed.outcomes.is_empty());
    assert!(
        reversed.machines[0]
            .error
            .as_deref()
            .unwrap()
            .contains("no saved snapshot for this view")
    );
    // Newer wins regardless of enumeration order or an older compatible view.
    files.contents.get_mut().insert(
        second,
        Ok(bytes(
            "z-retired",
            Slot::Reported,
            at + chrono::Duration::seconds(1),
            &view("window=30d", "kind=Run"),
        )),
    );
    files.entries.as_mut().unwrap().swap(0, 1);
    let newer = Capture::with_filesystem(&config, None, &files)
        .load(&asked("z-retired"))
        .unwrap();
    assert_eq!(newer.machines[0].status, Status::Reported);
    assert!(newer.outcomes.is_empty());
}

#[test]
fn concurrent_reports_reload_the_slot_and_a_new_capture_keeps_both_views() {
    let logs = tempfile::tempdir().unwrap();
    let config = config(logs.path(), &[]);
    let before = Capture::new(&config, None);
    let at = today().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|threads| {
        let handles: Vec<_> = ["window=7d", "window=30d"]
            .into_iter()
            .map(|query| {
                let barrier = &barrier;
                let logs = logs.path();
                threads.spawn(move || {
                    barrier.wait();
                    MetricsReport {
                        version: 1,
                        machine: "reporter".into(),
                        view: query.into(),
                        summary: summary("reporter", window().current().unwrap(), 2, 0),
                    }
                    .receive(logs, "reporter", "here", at)
                    .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    assert!(before.others().is_empty());
    assert!(before.load(&asked("reporter")).is_err());
    let after = Capture::new(&config, None);
    for preset in [
        crate::args::MetricsWindow::Days7,
        crate::args::MetricsWindow::Days30,
    ] {
        let mut asked = asked("reporter");
        asked.window = Some(preset);
        let loaded = after.load(&asked).unwrap();
        assert_eq!(loaded.machines[0].status, Status::Reported);
        assert!(
            !loaded.machines[0]
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("snapshot"),
            "both exact views survive the concurrent writes; comparison dates may differ"
        );
    }
}
