//! Process IO is the user-visible regression: an idle Console must not reread
//! the log store, regardless of its size.
use super::*;

fn rchar(console: &Console) -> u64 {
    fs::read_to_string(format!("/proc/{}/io", console.child.id()))
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("rchar: "))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn idle_console_does_not_reread_the_store() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 81, Some("merged"), 0);
    fixture["cause"] = json!("x".repeat(1024 * 1024));
    write_record(home.path(), 81, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    assert_eq!(console.get("/").status(), 200);
    let before = rchar(&console);
    std::thread::sleep(Duration::from_secs(3));
    let bytes = rchar(&console) - before;
    eprintln!("idle Console rchar over 3s: {bytes} bytes");
    assert!(bytes < 48 * 1024, "idle Console reread {bytes} bytes");
}

#[test]
fn unchanged_requests_reuse_retained_records_but_replacements_are_visible() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = record(home.path(), 81, Some("merged"), 0);
    fixture["cause"] = json!("x".repeat(1024 * 1024));
    write_record(home.path(), 81, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    assert!(console.get("/").text().unwrap().contains("acme/widgets#81"));
    let before = rchar(&console);
    assert!(console.get("/").text().unwrap().contains("acme/widgets#81"));
    let bytes = rchar(&console) - before;
    assert!(bytes < 48 * 1024, "unchanged request reread {bytes} bytes");
    fixture["issue"]["title"] = json!("Changed title");
    write_record(home.path(), 81, &fixture);
    assert!(console.get("/").text().unwrap().contains("Changed title"));
}

fn live_fixture(home: &Path, pid: u32) {
    let mut fixture = record(home, 81, None, 0);
    fixture["process_id"] = json!(pid);
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let ticks = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap();
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
    fixture["process_start"] = json!(format!("{}:{ticks}", boot.trim()));
    write_record(home, 81, &fixture);
}

#[test]
fn idle_viewers_sleep_and_log_appends_and_completion_wake_the_reader() {
    use std::io::Write;
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    live_fixture(home.path(), std::process::id());
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/81-fixture.log");
    fs::write(&path, "first line\n").unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let _viewers: Vec<_> = (0..3).map(|_| console.get("/events")).collect();
    let link = console.log_link("81-fixture.log");
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap()
        .get(format!("{}{link}/stream/0", console.address))
        .send()
        .unwrap();
    let mut reader = BufReader::new(response);
    assert!(sse_event(&mut reader).contains("first line"));
    std::thread::sleep(Duration::from_millis(200));
    let before = rchar(&console);
    std::thread::sleep(Duration::from_secs(3));
    let bytes = rchar(&console) - before;
    assert!(bytes < 48 * 1024, "idle viewers reread {bytes} bytes");
    let began = std::time::Instant::now();
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"second line\n")
        .unwrap();
    assert!(sse_event(&mut reader).contains("second line"));
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "log append was not delivered promptly"
    );
    let mut fixture = record(home.path(), 81, Some("merged"), 0);
    fixture["ended_at"] = json!(chrono::Local::now().to_rfc3339());
    write_record(home.path(), 81, &fixture);
    assert!(sse_event(&mut reader).contains("\"done\":true"));
}

#[test]
fn replacing_the_log_root_keeps_future_inbox_changes_live() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 81, Some("merged"), 0);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let root = home.path().join(".thirdshift/logs");
    fs::rename(&root, home.path().join("old-logs")).unwrap();
    let mut fixture = record(home.path(), 82, Some("merged"), 0);
    std::thread::sleep(Duration::from_millis(400));
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap()
        .get(format!("{}/events", console.address))
        .send()
        .unwrap();
    let mut stream = BufReader::new(response);
    assert!(sse_event(&mut stream).contains("event: commands"));
    fixture["issue"]["title"] = json!("Changed after replacing the root");
    write_record(home.path(), 82, &fixture);
    assert!(sse_event(&mut stream).contains("event: commands"));
    assert!(
        console
            .get("/")
            .text()
            .unwrap()
            .contains("Changed after replacing the root")
    );
}
