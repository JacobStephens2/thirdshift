use super::*;

#[test]
fn console_header_shows_host_admission_and_reloads_limits() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let config = home.path().join(".thirdshift/config.toml");
    let settings = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        format!("{settings}\n[resources]\nmin_available_memory_mb = 9007199254740991\n"),
    )
    .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let output = Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(["resources", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let snapshot: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snapshot["ready"], false);
    for path in ["/", "/resources", "/cronjobs", "/account", "/console-log"] {
        let html = console.get(path).text().unwrap();
        let header = html.split("</header>").next().unwrap();
        assert!(
            header.contains("Host busy · new sessions wait"),
            "{path}: {header}"
        );
        assert!(
            header.contains("below 9007199254740991 MB"),
            "{path}: {header}"
        );
        assert!(header.contains("href=\"/resources\""), "{path}: {header}");
    }
    fs::write(
        &config,
        format!("{settings}\n[resources]\nmin_available_memory_mb = 1\n"),
    )
    .unwrap();
    let response = console.get("/resources/status");
    assert_eq!(response.status(), 200);
    assert!(
        response
            .text()
            .unwrap()
            .contains("Host ready for another session")
    );
    fs::write(&config, settings).unwrap();
    assert!(
        console
            .get("/resources/status")
            .text()
            .unwrap()
            .contains("Host limits off")
    );
}

#[test]
fn resources_page_shows_host_measurements_limits_and_admission_then_reloads_config() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let config = home.path().join(".thirdshift/config.toml");
    let settings = fs::read_to_string(&config).unwrap();
    fs::write(
        &config,
        format!("{settings}\n[resources]\nmin_available_memory_mb = 9007199254740991\n"),
    )
    .unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    assert!(
        console
            .get("/")
            .text()
            .unwrap()
            .contains("href=\"/resources\"")
    );
    let response = console.get("/resources");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    for expected in [
        "Resources",
        "Available memory",
        "Logical CPUs",
        "One-minute load",
        "9007199254740991",
        "Waiting for host resources",
        "thirdshift resources --json",
        "Reload",
    ] {
        assert!(html.contains(expected), "missing {expected}: {html}");
    }
    fs::write(config, settings).unwrap();
    assert!(
        console
            .get("/resources")
            .text()
            .unwrap()
            .contains("Ready to start another session")
    );
}
