//! Development assets through the real Console binary and HTTP routes.
use super::*;

fn start(home: &Path, assets: Option<&Path>) -> Console {
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command.args(["console", "--no-sign-in"]).env("HOME", home);
    command.env_remove("THIRDSHIFT_CONSOLE_ASSETS");
    if let Some(assets) = assets {
        command.env("THIRDSHIFT_CONSOLE_ASSETS", assets);
    }
    Console::start_command(command)
}

#[test]
fn development_assets_are_debug_only_and_missing_files_do_not_fall_back() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let assets = tempfile::tempdir().unwrap();
    fs::write(assets.path().join("console.css"), "body { color: red; }").unwrap();
    let embedded = include_str!("../../src/console/assets/console.css");
    let normal = start(home.path(), None);
    let csp = normal.get("/").headers()["content-security-policy"].clone();
    assert_eq!(normal.get("/assets/console.css").text().unwrap(), embedded);
    drop(normal);
    let console = start(home.path(), Some(assets.path()));
    assert_eq!(console.get("/").headers()["content-security-policy"], csp);
    let response = console.get("/assets/console.css");
    assert_eq!(
        response.headers()["content-type"],
        "text/css; charset=utf-8"
    );
    if cfg!(debug_assertions) {
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.text().unwrap(), "body { color: red; }");
        assert_eq!(console.get("/assets/console.js").status(), 404);
        fs::write(assets.path().join("console.css"), "body { color: blue; }").unwrap();
        assert_eq!(
            console.get("/assets/console.css").text().unwrap(),
            "body { color: blue; }"
        );
    } else {
        assert_eq!(response.text().unwrap(), embedded);
        fs::write(assets.path().join("console.css"), "body { color: blue; }").unwrap();
        assert_eq!(console.get("/assets/console.css").text().unwrap(), embedded);
        assert_eq!(console.get("/assets/console.js").status(), 200);
    }
}

#[cfg(debug_assertions)]
#[test]
fn development_assets_refuse_traversal_absolute_paths_and_symlinks() {
    use std::io::Write;
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let assets = home.path().join("assets");
    fs::create_dir_all(assets.join("nested")).unwrap();
    fs::write(assets.join("nested/inside.css"), "safe").unwrap();
    fs::write(home.path().join("secret.css"), "private").unwrap();
    fs::create_dir(home.path().join("private")).unwrap();
    fs::write(home.path().join("private/secret.css"), "private").unwrap();
    std::os::unix::fs::symlink(home.path().join("secret.css"), assets.join("escape.css")).unwrap();
    std::os::unix::fs::symlink(home.path().join("private"), assets.join("outside")).unwrap();
    fs::write(assets.join("user-themes.css"), "forged theme").unwrap();
    let console = start(home.path(), Some(&assets));
    assert_eq!(
        console.get("/assets/nested/inside.css").text().unwrap(),
        "safe"
    );
    assert!(
        !console
            .get("/assets/user-themes.css")
            .text()
            .unwrap()
            .contains("forged")
    );
    // Use HTTP directly so a URL client cannot normalize away the attack.
    for path in [
        "/assets/../secret.css",
        "/assets/%2e%2e/secret.css",
        "/assets/%2Fetc%2Fpasswd",
        "/assets//etc/passwd",
        "/assets/C:%5Csecret.css",
        "/assets/escape.css",
        "/assets/outside/secret.css",
        "/assets/nested",
    ] {
        let mut socket =
            std::net::TcpStream::connect(console.address.trim_start_matches("http://")).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            socket,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut status = String::new();
        BufReader::new(socket).read_line(&mut status).unwrap();
        assert!(
            status.starts_with("HTTP/1.1 404") || status.starts_with("HTTP/1.1 400"),
            "{path}: {status}"
        );
    }
}

#[test]
fn embedded_mode_does_not_send_asset_events_for_disk_edits() {
    use std::io::Read;
    for configured in [false, true] {
        if configured && cfg!(debug_assertions) {
            continue;
        }
        let home = tempfile::tempdir().unwrap();
        configure(home.path());
        let assets = tempfile::tempdir().unwrap();
        fs::write(assets.path().join("console.css"), "before").unwrap();
        let console = start(home.path(), configured.then(|| assets.path()));
        let embedded = include_str!("../../src/console/assets/console.css");
        assert_eq!(console.get("/assets/console.css").text().unwrap(), embedded);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let mut stream = client
            .get(format!("{}/events", console.address))
            .send()
            .unwrap();
        fs::write(assets.path().join("console.css"), "after").unwrap();
        assert_eq!(console.get("/assets/console.css").text().unwrap(), embedded);
        let mut notices = String::new();
        assert!(
            stream.read_to_string(&mut notices).is_err(),
            "SSE must stay open"
        );
        assert!(notices.contains("event: commands"), "{notices}");
        assert!(!notices.contains("event: assets"), "{notices}");
    }
}
