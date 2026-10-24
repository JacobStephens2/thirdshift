//! Native process-exit updates through HTTP, on Linux and macOS.
use super::*;

pub(super) struct FixtureProcess(pub(super) Child);

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(target_os = "linux")]
pub(super) fn start(pid: u32) -> String {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let ticks = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    format!("{}:{ticks}", boot.trim())
}

#[cfg(target_os = "macos")]
pub(super) fn start(pid: u32) -> String {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: the fixture owns this process and supplies the exact buffer size.
    assert_eq!(
        unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        },
        size
    );
    let info = unsafe { info.assume_init() };
    format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec)
}

pub(super) fn live_record(home: &Path, number: u64, pid: u32) -> Value {
    let mut fixture = record(home, number, None, 0);
    fixture["process_id"] = json!(pid);
    fixture["process_start"] = json!(start(pid));
    fixture
}

#[test]
fn outcome_does_not_finish_sse_or_polling_before_the_writer_exits() {
    use std::io::Write;
    for polling in [false, true] {
        let home = tempfile::tempdir().unwrap();
        configure(home.path());
        let mut work = FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
        let mut fixture = live_record(home.path(), 81, work.0.id());
        write_record(home.path(), 81, &fixture);
        let path = home
            .path()
            .join(".thirdshift/logs/acme/widgets/commands/issue/81-fixture.log");
        fs::write(&path, "first line\n").unwrap();
        let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
        let link = console.log_link("81-fixture.log");
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(4))
            .build()
            .unwrap();
        let poll = |offset| -> Value {
            client
                .get(format!("{}{link}/stream/{offset}", console.address))
                .header("accept", "application/json")
                .send()
                .unwrap()
                .json()
                .unwrap()
        };
        let mut stream = (!polling).then(|| {
            BufReader::new(
                client
                    .get(format!("{}{link}/stream/0", console.address))
                    .send()
                    .unwrap(),
            )
        });
        let first = if let Some(stream) = &mut stream {
            sse_event(stream)
        } else {
            poll(0).to_string()
        };
        assert!(
            first.contains("first line") && first.contains("\"done\":false"),
            "{first}"
        );
        fixture["outcome"] = json!("ready for review");
        write_record(home.path(), 81, &fixture);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"later output\nfinal unterminated output")
            .unwrap();
        let later = if let Some(stream) = &mut stream {
            sse_event(stream)
        } else {
            poll(11).to_string()
        };
        assert!(
            later.contains("later output") && later.contains("\"done\":false"),
            "{later}"
        );
        assert!(!later.contains("final unterminated output"), "{later}");
        let initial = console.get(&link).text().unwrap();
        assert!(!initial.contains("final unterminated output"), "{initial}");
        assert!(initial.contains("data-live=\"true\""), "{initial}");
        // Let the observer settle after the append, then exit without another record write.
        std::thread::sleep(Duration::from_millis(200));
        let began = std::time::Instant::now();
        work.0.kill().unwrap();
        work.0.wait().unwrap();
        let last = if let Some(stream) = &mut stream {
            sse_event(stream)
        } else {
            poll(24).to_string()
        };
        assert!(
            last.contains("final unterminated output") && last.contains("\"done\":true"),
            "{last}"
        );
        assert!(!last.contains("later output"), "{last}");
        assert!(began.elapsed() < Duration::from_secs(2));
    }
}

#[test]
fn process_death_finishes_the_existing_reader_without_a_record_write() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut work = FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
    let fixture = live_record(home.path(), 81, work.0.id());
    write_record(home.path(), 81, &fixture);
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/81-fixture.log");
    fs::write(&path, "first line\nfinal unterminated line").unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let link = console.log_link("81-fixture.log");
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
        .get(format!("{}{link}/stream/0", console.address))
        .send()
        .unwrap();
    let mut stream = BufReader::new(response);
    let first = sse_event(&mut stream);
    assert!(first.contains("first line") && first.contains("\"done\":false"));
    std::thread::sleep(Duration::from_millis(200));
    let began = std::time::Instant::now();
    work.0.kill().unwrap();
    work.0.wait().unwrap();
    let last = sse_event(&mut stream);
    assert!(last.contains("final unterminated line") && last.contains("\"done\":true"));
    assert!(began.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_silent_transition_wakes_the_inbox_without_a_file_change() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    fs::write(
        home.path().join(".thirdshift/config.toml"),
        "[console]\nport = 0\n[inbox]\nsilent_minutes = 1\n",
    )
    .unwrap();
    let mut fixture = live_record(home.path(), 81, std::process::id());
    fixture["stages"][0]["began_at"] =
        json!((chrono::Local::now() - chrono::Duration::minutes(2)).to_rfc3339());
    write_record(home.path(), 81, &fixture);
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/81-fixture.log");
    fs::write(&path, "recent output\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() - Duration::from_secs(58)),
        )
        .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    assert!(
        !console
            .get("/")
            .text()
            .unwrap()
            .contains("data-state=\"Silent\"")
    );
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .unwrap()
        .get(format!("{}/events", console.address))
        .send()
        .unwrap();
    let mut stream = BufReader::new(response);
    let began = std::time::Instant::now();
    loop {
        assert!(sse_event(&mut stream).contains("event: commands"));
        if console
            .get("/")
            .text()
            .unwrap()
            .contains("data-state=\"Silent\"")
        {
            break;
        }
    }
    assert!(began.elapsed() < Duration::from_secs(3));
}

#[test]
fn elapsed_time_updates_inbox_groups_and_retention_without_a_file_change() {
    for (outcome, days, remains) in [
        ("merged", 7, true),
        ("failed", 1, false),
        ("ready for review", 1, false),
    ] {
        let home = tempfile::tempdir().unwrap();
        configure(home.path());
        fs::write(
            home.path().join(".thirdshift/config.toml"),
            format!("[console]\nport = 0\n[inbox]\ndays = {days}\n"),
        )
        .unwrap();
        let mut fixture = record(home.path(), 81, Some(outcome), 0);
        fixture["issue"]["title"] = json!("Elapsed boundary fixture");
        fixture["ended_at"] = json!(
            (chrono::Local::now() - chrono::Duration::days(1) + chrono::Duration::seconds(2))
                .to_rfc3339()
        );
        write_record(home.path(), 81, &fixture);
        let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
        assert!(
            console
                .get("/")
                .text()
                .unwrap()
                .contains("Elapsed boundary fixture")
        );
        let response = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .get(format!("{}/events", console.address))
            .send()
            .unwrap();
        let mut stream = BufReader::new(response);
        let began = std::time::Instant::now();
        loop {
            assert!(sse_event(&mut stream).contains("event: commands"));
            let html = console.get("/").text().unwrap();
            match html.find("Elapsed boundary fixture") {
                None if !remains => break,
                Some(at) if remains => {
                    let prefix = &html[..at];
                    let group = &prefix[prefix.rfind("data-group=\"").unwrap()..];
                    if group.starts_with("data-group=\"resolved\"") {
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(began.elapsed() < Duration::from_secs(4));
    }
}
