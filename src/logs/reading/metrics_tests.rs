//! Observe actual trace opens at the Command-details and Snapshot seams.
use std::ffi::CString;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStrExt;

use super::*;

struct Opens {
    file: fs::File,
    paths: BTreeMap<i32, PathBuf>,
}

impl Opens {
    fn new(paths: &[PathBuf]) -> Self {
        // SAFETY: the successful descriptor is owned exactly once by File.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(fd >= 0, "{}", std::io::Error::last_os_error());
        let file = unsafe { fs::File::from_raw_fd(fd) };
        let paths = paths
            .iter()
            .map(|path| {
                let name = CString::new(path.as_os_str().as_bytes()).unwrap();
                // SAFETY: name is NUL terminated and fd remains owned above.
                let wd = unsafe { libc::inotify_add_watch(fd, name.as_ptr(), libc::IN_OPEN) };
                assert!(wd >= 0, "{}", std::io::Error::last_os_error());
                (wd, path.clone())
            })
            .collect();
        Self { file, paths }
    }

    fn take(&mut self) -> BTreeSet<PathBuf> {
        let mut opened = BTreeSet::new();
        let mut bytes = [0_u8; 8192];
        loop {
            match self.file.read(&mut bytes) {
                Ok(0) => break,
                Ok(len) => {
                    let mut offset = 0;
                    while offset < len {
                        // SAFETY: inotify returns complete events; byte arrays
                        // need not have the alignment of inotify_event.
                        let event = unsafe {
                            bytes
                                .as_ptr()
                                .add(offset)
                                .cast::<libc::inotify_event>()
                                .read_unaligned()
                        };
                        if event.mask & libc::IN_OPEN != 0 {
                            opened.insert(self.paths[&event.wd].clone());
                        }
                        offset += std::mem::size_of::<libc::inotify_event>() + event.len as usize;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
            }
        }
        opened
    }
}

fn historical(root: &Path, number: u64) -> PathBuf {
    let log = root.join(format!(
        "acme/widgets/commands/issue/{number}-trace.logs.jsonl"
    ));
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, "{\"resourceLogs\":[]}\n").unwrap();
    let mut past = record(number);
    past["process_id"] = 0.into();
    past["outcome"] = "merged".into();
    past["ended_at"] = "2026-10-19T11:01:00-04:00".into();
    past["stages"] = json!([{
        "name":"implement", "began_at":"2026-10-19T11:00:00-04:00", "session_log":log
    }]);
    write(root, &format!("{number}-fixture.command.json"), &past);
    log
}

#[test]
fn command_details_only_read_selected_historical_session_traces() {
    for count in [1, 2, 562] {
        let dir = tempfile::tempdir().unwrap();
        let traces: Vec<_> = (1..=count).map(|n| historical(dir.path(), n)).collect();
        let mut opens = Opens::new(&traces);
        let mut config =
            crate::config::UserConfig::parse("", &dir.path().join("config.toml"), dir.path())
                .unwrap();
        config.logs_dir = dir.path().into();
        let inspection = crate::logs::stop::inspect(&config, "widgets#1").unwrap();
        assert_eq!(
            inspection.details["run_metrics"]["session_signals"]["implement"]["observed_sessions"],
            1
        );
        assert_eq!(
            opens.take(),
            [traces[0].clone()].into(),
            "Command details read unrelated historical Session traces with {count} records"
        );
    }
}

#[test]
fn metrics_reuse_unchanged_session_traces_and_refresh_changed_traces() {
    use std::io::Write;

    let dir = tempfile::tempdir().unwrap();
    let traces: Vec<_> = (1..=4).map(|n| historical(dir.path(), n)).collect();
    // The source can exceed the entire cache budget; only its compact derived
    // signals should count toward that budget.
    let mut large = fs::File::create(&traces[3]).unwrap();
    let padding = vec![b' '; 1024 * 1024];
    for _ in 0..65 {
        large.write_all(&padding).unwrap();
    }
    large.write_all(b"{\"resourceLogs\":[]}\n").unwrap();
    drop(large);
    let mut opens = Opens::new(&traces);
    let first = Snapshot::load_metrics(dir.path(), now()).unwrap();
    assert_eq!(opens.take(), traces.iter().cloned().collect());
    let second = Snapshot::load_metrics(dir.path(), now()).unwrap();
    assert!(
        opens.take().is_empty(),
        "unchanged Session traces were reparsed"
    );
    assert_eq!(
        serde_json::to_value(first.resolve("widgets#1").unwrap().observation).unwrap(),
        serde_json::to_value(second.resolve("widgets#1").unwrap().observation).unwrap()
    );

    let amendment = |version: &str| {
        serde_json::to_string(&json!({
            "resourceLogs": [{"resource": {"attributes": [
                {"key": "thirdshift.harness.version", "value": {"stringValue": version}}
            ]}, "scopeLogs": []}]
        }))
        .unwrap()
            + "\n"
    };
    fs::OpenOptions::new()
        .append(true)
        .open(&traces[0])
        .unwrap()
        .write_all(amendment("1.0").as_bytes())
        .unwrap();
    opens.take();
    let appended = Snapshot::load_metrics(dir.path(), now()).unwrap();
    assert_eq!(opens.take(), [traces[0].clone()].into());
    let signals = |snapshot: &Snapshot| {
        serde_json::to_value(snapshot.resolve("widgets#1").unwrap().observation).unwrap()
            ["run_metrics"]["stages"][0]["session_signals"].clone()
    };
    assert_eq!(signals(&appended)["harness_version"], "1.0");

    // Same length and mtime, but a new inode must invalidate the old signals.
    let modified = fs::metadata(&traces[0]).unwrap().modified().unwrap();
    let replacement = dir.path().join("replacement.logs.jsonl");
    fs::write(
        &replacement,
        "{\"resourceLogs\":[]}\n".to_owned() + &amendment("2.0"),
    )
    .unwrap();
    fs::File::open(&replacement)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    fs::rename(replacement, &traces[0]).unwrap();
    let mut replaced_opens = Opens::new(&traces);
    let replaced = Snapshot::load_metrics(dir.path(), now()).unwrap();
    assert_eq!(replaced_opens.take(), [traces[0].clone()].into());
    assert_eq!(signals(&replaced)["harness_version"], "2.0");
}

#[test]
fn command_details_keep_dispatch_and_child_signals_and_other_commands_health() {
    for (selected, expected) in [("widgets#1", vec![1, 2, 3]), ("widgets#2", vec![2, 3])] {
        let dir = tempfile::tempdir().unwrap();
        let traces: Vec<_> = (1..=5).map(|n| historical(dir.path(), n)).collect();
        let path = |n| {
            dir.path().join(format!(
                "acme/widgets/commands/issue/{n}-fixture.command.json"
            ))
        };
        let read = |n| serde_json::from_slice::<Value>(&fs::read(path(n)).unwrap()).unwrap();
        let mut outer = read(1);
        outer["dispatched"] = json!([read(2)]);
        fs::write(path(1), serde_json::to_vec(&outer).unwrap()).unwrap();
        fs::remove_file(path(2)).unwrap();
        let mut child = read(3);
        child["parent_record"] = json!(path(1));
        child["parent_dispatch"] = "https://github.com/acme/widgets/issues/2".into();
        fs::write(path(3), serde_json::to_vec(&child).unwrap()).unwrap();
        let mut live = record(5);
        live["stages"] = json!([{"name": "implement", "began_at": "2026-10-19T11:00:00-04:00"}]);
        fs::write(path(5), serde_json::to_vec(&live).unwrap()).unwrap();

        let before = Snapshot::load(dir.path(), now()).unwrap();
        let mut opens = Opens::new(&traces);
        let details = Snapshot::load_details(
            dir.path(),
            now(),
            std::time::Duration::from_secs(600),
            selected,
        )
        .unwrap();
        assert_eq!(
            opens.take(),
            expected.iter().map(|n| traces[n - 1].clone()).collect()
        );
        for number in [1, 2, 3] {
            let value = serde_json::to_value(
                details
                    .resolve(&format!("widgets#{number}"))
                    .unwrap()
                    .observation,
            )
            .unwrap();
            if expected.contains(&number) {
                assert_eq!(
                    value["run_metrics"]["session_signals"]["implement"]["observed_sessions"],
                    1
                );
            }
        }
        let observation = |snapshot: &Snapshot| {
            serde_json::to_value(snapshot.resolve("widgets#5").unwrap().observation).unwrap()
        };
        assert_eq!(
            observation(&before)["health"],
            observation(&details)["health"]
        );
        assert_eq!(
            observation(&before)["usual_ranges"],
            observation(&details)["usual_ranges"]
        );
        assert!(!observation(&before)["usual_ranges"]["implement"].is_null());
    }
}

#[test]
fn saved_session_signals_do_not_read_the_historical_trace() {
    let dir = tempfile::tempdir().unwrap();
    let trace = historical(dir.path(), 1);
    let snapshot = Snapshot::load_metrics(dir.path(), now()).unwrap();
    let value = serde_json::to_value(snapshot.resolve("widgets#1").unwrap().observation).unwrap();
    write(dir.path(), "1-fixture.command.json", &value);
    fs::write(&trace, "invalid trace, saved signals must win\n").unwrap();
    let mut opens = Opens::new(&[trace]);
    let snapshot = Snapshot::load_metrics(dir.path(), now()).unwrap();
    assert!(opens.take().is_empty());
    let saved = serde_json::to_value(snapshot.resolve("widgets#1").unwrap().observation).unwrap();
    assert_eq!(saved["run_metrics"], value["run_metrics"]);
}
