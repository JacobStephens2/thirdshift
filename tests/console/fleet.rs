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
