use super::*;

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
