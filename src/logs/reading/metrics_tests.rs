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
                            bytes.as_ptr().add(offset).cast::<libc::inotify_event>().read_unaligned()
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
    let log = root.join(format!("acme/widgets/commands/issue/{number}-trace.logs.jsonl"));
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
        let mut config = crate::config::UserConfig::parse("", &dir.path().join("config.toml"), dir.path()).unwrap();
        config.logs_dir = dir.path().into();
        let inspection = crate::logs::stop::inspect(&config, "widgets#1").unwrap();
        assert_eq!(inspection.details["run_metrics"]["session_signals"]["implement"]["observed_sessions"], 1);
        assert_eq!(opens.take(), [traces[0].clone()].into(), "Command details read unrelated historical Session traces with {count} records");
    }
}
