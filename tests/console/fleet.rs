use super::*;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::Output;

fn add(home: &Path, name: &str, address: &str, token: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["console", "fleet", "add", name, address])
        .env("HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = writeln!(child.stdin.take().unwrap(), "{token}");
    child.wait_with_output().unwrap()
}

fn stand_in() -> (std::net::TcpListener, String) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    (listener, address)
}

fn inbox_request(listener: &std::net::TcpListener, all: bool) -> std::net::TcpStream {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line, format!("GET /api/v1/status?all={all} HTTP/1.1\r\n"));
                loop {
                    line.clear();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Inbox request never arrived"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    }
}

fn reply(stream: &mut std::net::TcpStream, code: u16, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {code} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}

#[test]
fn status_merges_paired_inboxes_without_regrouping_their_rows() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    configure(local_home.path());
    configure(remote_home.path());
    let mut local_record = record(local_home.path(), 71, Some("failed"), 0);
    local_record["issue"] = Value::Null;
    write_record(local_home.path(), 71, &local_record);
    let mut remote_record = record(remote_home.path(), 72, Some("merged"), 0);
    remote_record["issue"] = Value::Null;
    write_record(remote_home.path(), 72, &remote_record);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start(binary, remote_home.path());
    let _local = Console::start(binary, local_home.path());
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    let paired = add(local_home.path(), "remote", &remote.address, &token);
    assert!(paired.status.success(), "{paired:?}");
    let status = api::cli(local_home.path(), &["status", "--json"]);
    let rows = status["commands"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{status}");
    assert_eq!(status["version"], 2);
    let local = rows
        .iter()
        .find(|row| row["machine_name"] == status["hostname"])
        .unwrap();
    assert_eq!(local["group"], "needs you");
    let other = rows
        .iter()
        .find(|row| row["machine_name"] == "remote")
        .unwrap();
    assert_eq!(
        other["machine"],
        api::json(remote.get("/api/v1/whoami"))["hostname"]
    );
    assert_eq!(other["group"], "done");
    let machines = status["machines"].as_array().unwrap();
    assert_eq!(machines.len(), 2, "{status}");
    for machine in machines {
        assert_eq!(machine["reachability"], "ok");
        assert!(
            chrono::DateTime::parse_from_rfc3339(machine["fetched_at"].as_str().unwrap()).is_ok()
        );
        assert!(machine["error"].is_null());
    }
}

#[test]
fn status_scopes_remote_rows_and_shows_the_fleet_active_run_breakdown() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    for home in [local_home.path(), remote_home.path()] {
        configure(home);
        api::freeze_clock(home);
    }
    let local_row = super::watching::live_record(local_home.path(), 81, std::process::id());
    write_record(local_home.path(), 81, &local_row);
    let mut remote_row = super::watching::live_record(remote_home.path(), 82, std::process::id());
    remote_row["kind"] = json!("Run");
    remote_row["stages"][0]["name"] = json!("CI watch");
    write_record(remote_home.path(), 82, &remote_row);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start_command({
        let mut command = Command::new(binary);
        command
            .args(["console", "--no-sign-in"])
            .env("HOME", remote_home.path())
            .env("LD_PRELOAD", remote_home.path().join("api-clock.so"));
        command
    });
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    assert!(
        add(local_home.path(), "remote", &remote.address, &token)
            .status
            .success()
    );
    let selected = api::cli(
        local_home.path(),
        &["status", "--machine", "remote", "--json"],
    );
    assert_eq!(
        selected["commands"].as_array().unwrap().len(),
        1,
        "{selected}"
    );
    assert_eq!(selected["commands"][0]["machine_name"], "remote");
    assert_eq!(selected["commands"][0]["group"], "landing");
    assert_eq!(selected["active_runs"], 1);
    assert_eq!(selected["machines"].as_array().unwrap().len(), 1);
    let plain = api::cli(local_home.path(), &["status", "--json"]);
    let hostname = plain["hostname"].as_str().unwrap();
    let scoped_self = api::cli(
        local_home.path(),
        &["status", "--json", "--machine", hostname],
    );
    // Fresh fetch times are per read; all other facts must agree.
    let without_fetch_times = |mut value: Value| {
        for machine in value["machines"].as_array_mut().unwrap() {
            machine.as_object_mut().unwrap().remove("fetched_at");
        }
        value
    };
    assert_eq!(
        without_fetch_times(plain.clone()),
        without_fetch_times(scoped_self)
    );
    assert_eq!(plain["active_runs"], 2, "{plain}");
    for machine in plain["machines"].as_array().unwrap() {
        assert_eq!(machine["active_runs"], 1, "{plain}");
    }
    let text = Command::new(binary)
        .args(["status"])
        .env("HOME", local_home.path())
        .output()
        .unwrap();
    assert!(text.status.success(), "{text:?}");
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("2 active runs"), "{text}");
    assert!(text.contains("remote: 1 active run"), "{text}");
    assert!(
        text.contains(&format!("{hostname}: 1 active run")),
        "{text}"
    );
    assert!(
        text.contains("[remote] Ticket run acme/widgets#82"),
        "{text}"
    );
    assert!(
        text.contains(&format!("[{hostname}] Spec run acme/widgets#81")),
        "{text}"
    );
}

#[test]
fn status_reports_failed_machines_without_losing_the_local_inbox() {
    let local_home = tempfile::tempdir().unwrap();
    let remote_home = tempfile::tempdir().unwrap();
    configure(local_home.path());
    configure(remote_home.path());
    let mut fixture = record(local_home.path(), 91, Some("merged"), 0);
    fixture["issue"] = Value::Null;
    write_record(local_home.path(), 91, &fixture);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start(binary, remote_home.path());
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    assert!(
        add(local_home.path(), "remote", &remote.address, &token)
            .status
            .success()
    );
    let assert_state = |expected: &str| {
        let started = std::time::Instant::now();
        let status = api::cli(local_home.path(), &["status", "--json"]);
        assert!(started.elapsed() < Duration::from_secs(10), "{status}");
        assert_eq!(status["commands"].as_array().unwrap().len(), 1, "{status}");
        assert_eq!(status["commands"][0]["machine"], status["hostname"]);
        let machine = status["machines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|machine| machine["name"] == "remote")
            .unwrap();
        assert_eq!(machine["reachability"], expected, "{status}");
        assert!(machine["error"].as_str().is_some(), "{status}");
        assert!(!status.to_string().contains(&token));
    };
    let revoke = Command::new(binary)
        .args(["console", "token", "revoke", "watcher"])
        .env("HOME", remote_home.path())
        .output()
        .unwrap();
    assert!(revoke.status.success(), "{revoke:?}");
    assert_state("refused");
    drop(remote);
    assert_state("unreachable");

    for (code, body, expected) in [
        (404, "", "needs update"),
        (200, r#"{"version":999}"#, "needs update"),
        (500, "internal failure", "unreachable"),
        (302, "redirect", "refused"),
    ] {
        let (server, address) = stand_in();
        fs::write(
            local_home.path().join(".thirdshift/config.toml"),
            format!(
                "[console]\nport = 0\nfleet = [{{ name = 'remote', address = '{address}' }}]\n"
            ),
        )
        .unwrap();
        std::thread::scope(|scope| {
            let reply = scope.spawn(move || {
                reply(&mut inbox_request(&server, false), code, body);
            });
            assert_state(expected);
            reply.join().unwrap();
        });
    }
    let (server, address) = stand_in();
    fs::write(
        local_home.path().join(".thirdshift/config.toml"),
        format!("[console]\nport = 0\nfleet = [{{ name = 'remote', address = '{address}' }}]\n"),
    )
    .unwrap();
    std::thread::scope(|scope| {
        let (release, wait) = mpsc::channel();
        let reply = scope.spawn(move || {
            let _request = inbox_request(&server, false);
            // Hold the response until status has returned, or a generous cleanup bound.
            let _ = wait.recv_timeout(Duration::from_secs(15));
        });
        assert_state("unreachable");
        release.send(()).unwrap();
        reply.join().unwrap();
    });
}

#[test]
fn status_preserves_remote_accounts_children_and_history_without_copying_logs() {
    let local_home = tempfile::tempdir().unwrap();
    configure(local_home.path());
    let machine = other_accounts::Machine::new();
    machine.setting("conductor");
    let remote_home = machine.home("serving");
    for home in [local_home.path(), remote_home.as_path()] {
        let mut row = record(home, 101, Some("failed"), 0);
        row["issue"] = Value::Null;
        row["cause"] = json!("same cause on different machines");
        write_record(home, 101, &row);
    }
    let directory = remote_home.join(".thirdshift/logs/acme/widgets/commands/issue");
    let mut child = record(&remote_home, 102, Some("merged"), 0);
    child["parent_record"] = json!(directory.join("101-fixture.command.json"));
    child["kind"] = json!("Run");
    write_record(&remote_home, 102, &child);
    record(&machine.home("conductor"), 103, Some("merged"), 0);
    record(&remote_home, 104, Some("merged"), 90);
    let remote = Console::start_command({
        let mut command = machine.command();
        command.args(["console", "--no-sign-in"]);
        command
    });
    let token = tokens::create(&remote_home, "watcher", &[]);
    assert!(
        add(local_home.path(), "remote", &remote.address, &token)
            .status
            .success()
    );
    let status = api::cli(local_home.path(), &["status", "--all", "--json"]);
    let rows = status["commands"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "{status}");
    let remote_rows: Vec<_> = rows
        .iter()
        .filter(|row| row["machine_name"] == "remote")
        .collect();
    assert_eq!(remote_rows.len(), 3, "{status}");
    let other_account = remote_rows
        .iter()
        .find(|row| row["account"] == "conductor")
        .unwrap();
    assert_eq!(other_account["issue"]["number"], 103);
    assert!(other_account["machine"].as_str().is_some());
    let parent = remote_rows
        .iter()
        .find(|row| row["group"] == "needs you")
        .unwrap();
    assert_eq!(parent["children"][0]["issue"]["number"], 102);
    assert_eq!(parent["children"][0]["machine_name"], "remote");
    assert_eq!(parent["count"], 1);
    assert!(
        remote_rows
            .iter()
            .any(|row| row["group"] == "history" && row["issue"]["number"] == 104)
    );
    let text = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["status", "--machine", "remote"])
        .env("HOME", local_home.path())
        .output()
        .unwrap();
    assert!(text.status.success(), "{text:?}");
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains("[remote] Spec run acme/widgets#103") && text.contains("[conductor]"),
        "{text}"
    );
    assert!(
        text.contains("  [remote] Ticket run acme/widgets#102"),
        "{text}"
    );
    let local_records: Vec<_> = fs::read_dir(
        local_home
            .path()
            .join(".thirdshift/logs/acme/widgets/commands/issue"),
    )
    .unwrap()
    .map(|entry| entry.unwrap().file_name())
    .collect();
    assert_eq!(local_records, ["101-fixture.command.json"]);
}

#[test]
fn status_keeps_fleet_order_and_rows_whose_entry_name_matches_the_local_hostname() {
    for case in ["order", "name-collision"] {
        let output = Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/console/fleet_regressions.py"
            ))
            .args([case, env!("CARGO_BIN_EXE_thirdshift")])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn local_status_is_the_console_api_twin_when_a_fleet_is_paired() {
    let output = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/console/fleet_api_parity.py"
        ))
        .arg(env!("CARGO_BIN_EXE_thirdshift"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn pairs_two_consoles_and_removes_the_entry_and_owner_only_token() {
    let remote_home = tempfile::tempdir().unwrap();
    let local_home = tempfile::tempdir().unwrap();
    configure(remote_home.path());
    configure(local_home.path());
    fs::write(
        local_home.path().join(".thirdshift/config.toml"),
        "[console]\nport = 0\n",
    )
    .unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let remote = Console::start(binary, remote_home.path());
    let local = Console::start(binary, local_home.path());
    let token = tokens::create(remote_home.path(), "local", &["--scopes", "read,act"]);
    let result = add(local_home.path(), "remote", &remote.address, &token);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains(&token));
    let list = api::cli(local_home.path(), &["console", "fleet", "list", "--json"]);
    assert_eq!(list[0]["name"], "remote");
    assert_eq!(list[0]["address"], remote.address);
    assert_eq!(list[0]["live"], true);
    assert_eq!(list[0]["scopes"], json!(["read", "act"]));
    assert_eq!(list[0]["thirdshift_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        list[0]["hostname"],
        api::json(remote.get("/api/v1/whoami"))["hostname"]
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(list[0]["last_contact"].as_str().unwrap()).is_ok()
    );
    assert!(!list.to_string().contains(&token));
    let root = local_home.path().join(".thirdshift");
    let config = fs::read_to_string(root.join("config.toml")).unwrap();
    assert!(!config.contains(&token));
    assert!(!config.contains("other_consoles"));
    let credentials = fs::read_to_string(root.join("credentials.toml")).unwrap();
    let credentials: toml::Table = credentials.parse().unwrap();
    assert_eq!(
        credentials["fleet"]["remote"].as_str(),
        Some(token.as_str())
    );
    assert_eq!(
        fs::metadata(root.join("credentials.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let header = local.get("/").text().unwrap();
    assert!(header.contains(&format!("href=\"{}\">remote</a>", remote.address)));
    let removed = Command::new(binary)
        .args(["console", "fleet", "remove", "remote"])
        .env("HOME", local_home.path())
        .output()
        .unwrap();
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert_eq!(
        api::cli(local_home.path(), &["console", "fleet", "list", "--json"]),
        json!([])
    );
    assert!(
        !fs::read_to_string(root.join("credentials.toml"))
            .unwrap()
            .contains(&token)
    );
    assert!(
        !fs::read_to_string(root.join("config.toml"))
            .unwrap()
            .contains(&remote.address)
    );
}

#[test]
fn legacy_links_migrate_on_console_writes_including_inline_settings() {
    for text in [
        "[console]\nother_consoles = ['https://other.example.test/path?view=inbox#top', 'http://other.example.test:9000']\n# keep this\ntheme = 'focus-auto'\n",
        "console = { other_consoles = ['https://other.example.test/path?view=inbox#top', 'http://other.example.test:9000'], theme = 'focus-auto' }\n",
    ] {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join(".thirdshift");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("config.toml"), text).unwrap();
        let listed = api::cli(home.path(), &["console", "fleet", "list", "--json"]);
        assert_eq!(listed[0]["name"], "other.example.test");
        assert_eq!(
            listed[0]["address"],
            "https://other.example.test/path?view=inbox#top"
        );
        assert_eq!(listed[0]["live"], false);
        assert_eq!(listed[1]["name"], "other.example.test-2");
        assert_eq!(listed[1]["live"], false);
        assert_eq!(
            fs::read_to_string(root.join("config.toml")).unwrap(),
            text,
            "reading does not write"
        );
        api::cli(home.path(), &["console", "theme", "press-dark", "--json"]);
        let saved = fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(!saved.contains("other_consoles"), "{saved}");
        assert!(saved.contains("fleet"));
        if text.contains("# keep this") {
            assert!(saved.contains("# keep this"));
        }
        assert_eq!(
            api::cli(home.path(), &["console", "fleet", "list", "--json"]),
            listed
        );
        assert!(!root.join("credentials.toml").exists());
        let setup = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .arg("setup")
            .env("HOME", home.path())
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            setup.status.success(),
            "{}",
            String::from_utf8_lossy(&setup.stderr)
        );
        assert_eq!(
            api::cli(home.path(), &["console", "fleet", "list", "--json"]),
            listed
        );
        let complete = fs::read_to_string(root.join("config.toml")).unwrap();
        assert!(!complete.contains("other_consoles"));
        assert!(complete.contains("hidden_sections"));
    }
}

#[test]
fn refused_unreachable_and_old_machines_save_nothing() {
    let remote_home = tempfile::tempdir().unwrap();
    configure(remote_home.path());
    let remote = Console::start(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        remote_home.path(),
    );
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let token = tokens::create(remote_home.path(), "watcher", &[]);
    let paired = add(home.path(), "saved", &remote.address, &token);
    assert!(
        paired.status.success(),
        "{}",
        String::from_utf8_lossy(&paired.stderr)
    );
    let root = home.path().join(".thirdshift");
    let config = fs::read(root.join("config.toml")).unwrap();
    let credentials = fs::read(root.join("credentials.toml")).unwrap();
    let refused = add(home.path(), "saved", &remote.address, "invalid-token");
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("refused"));
    let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", unused.local_addr().unwrap());
    drop(unused);
    let unreachable = add(home.path(), "saved", &address, "invalid-token");
    assert!(!unreachable.status.success());
    assert!(String::from_utf8_lossy(&unreachable.stderr).contains("unreachable"));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stand_in = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    let mut request = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    request.read_line(&mut line).unwrap();
                    assert!(line.starts_with("GET /api/v1/whoami "));
                    loop {
                        line.clear();
                        assert!(request.read_line(&mut line).unwrap() > 0);
                        if line == "\r\n" {
                            break;
                        }
                    }
                    stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "whoami request never arrived"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        }
    });
    let old = add(home.path(), "saved", &address, "invalid-token");
    stand_in.join().unwrap();
    assert!(!old.status.success());
    assert!(String::from_utf8_lossy(&old.stderr).contains("too old"));
    for result in [&refused, &unreachable, &old] {
        assert!(!String::from_utf8_lossy(&result.stderr).contains("invalid-token"));
    }
    assert_eq!(fs::read(root.join("config.toml")).unwrap(), config);
    assert_eq!(
        fs::read(root.join("credentials.toml")).unwrap(),
        credentials
    );
}

#[test]
fn token_input_is_hidden_on_a_terminal_and_echo_is_restored() {
    let remote_home = tempfile::tempdir().unwrap();
    configure(remote_home.path());
    let remote = Console::start(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        remote_home.path(),
    );
    let token = tokens::create(remote_home.path(), "terminal", &[]);
    let scenario = support::Scenario::new();
    let result = scenario.run_terminal(
        &["console", "fleet", "add", "remote", &remote.address],
        &[],
        &[support::TerminalStep::line("Fleet token: ", &token)],
    );
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(
        !result.stderr.contains(&token),
        "terminal echoed the Fleet token"
    );
    assert!(!result.stdout.contains(&token));
    assert!(result.terminal_restored);
    let saved: toml::Table = scenario.credentials().unwrap().parse().unwrap();
    assert_eq!(saved["fleet"]["remote"].as_str(), Some(token.as_str()));
}
