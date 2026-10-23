//! Native process-exit updates through HTTP, on Linux and macOS.
use super::*;

struct FixtureProcess(Child);

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(target_os = "linux")]
fn start(pid: u32) -> String {
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
fn start(pid: u32) -> String {
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

#[test]
fn process_death_finishes_the_existing_reader_without_a_record_write() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut work = FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
    let mut fixture = record(home.path(), 81, None, 0);
    fixture["process_id"] = json!(work.0.id());
    fixture["process_start"] = json!(start(work.0.id()));
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
    let mut fixture = record(home.path(), 81, None, 0);
    fixture["process_id"] = json!(std::process::id());
    fixture["process_start"] = json!(start(std::process::id()));
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
