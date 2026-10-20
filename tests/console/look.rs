use super::*;

fn cli(home: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(arguments)
        .env("HOME", home)
        .output()
        .unwrap()
}

#[test]
fn look_cli_preserves_other_settings_and_comments() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let path = home.path().join(".thirdshift/config.toml");
    let original = "# my machine\n[console] # reader\nport = 0 # local\nallowed_emails = [\"owner@example.test\"] # access\ntheme = \"focus-auto\" # look\n\n[merge]\nalways = true # keep\n";
    fs::write(&path, original).unwrap();
    let output = cli(home.path(), &["console", "theme", "garden-dark"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original.replace("focus-auto", "garden-dark")
    );
    let output = cli(
        home.path(),
        &[
            "console", "sections", "--hide", "done", "--hide", "resolved",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = cli(
        home.path(),
        &["console", "sections", "--show", "done", "--json"],
    );
    assert!(output.status.success());
    let settings: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(settings["hidden_sections"], json!(["resolved"]));
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("allowed_emails = [\"owner@example.test\"] # access"));
    assert!(text.contains("always = true # keep"));
    let output = cli(home.path(), &["console", "log", "--json"]);
    assert!(output.status.success());
    let events: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 3);
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["email"].as_str().unwrap().starts_with("CLI:"))
    );
    let output = cli(home.path(), &["console", "theme", "list", "--json"]);
    let themes: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(themes.as_array().unwrap().len(), 18);
}

#[test]
fn page_can_only_write_look_keys_with_csrf() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let response = console.get("/look");
    assert_eq!(response.status(), 200);
    let look: Value = response.json().unwrap();
    let client = reqwest::blocking::Client::new();
    let post = |path: &str, body: &str, csrf: &str| {
        client
            .post(format!("{}{path}", console.address))
            .header("x-csrf-token", csrf)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.to_owned())
            .send()
            .unwrap()
    };
    assert_eq!(post("/theme", "theme=press-dark", "wrong").status(), 403);
    let csrf = look["csrf"].as_str().unwrap();
    let before = fs::read_to_string(home.path().join(".thirdshift/config.toml")).unwrap();
    for (path, body) in [
        (
            "/theme",
            "theme=press-dark&allowed_emails=attacker%40example.test",
        ),
        ("/sections", "hide=done&port=9000"),
        ("/theme", "theme=unknown-dark"),
        ("/sections", "hide=unknown"),
    ] {
        assert!(
            post(path, body, csrf).status().is_client_error(),
            "{path}: {body}"
        );
        assert_eq!(
            fs::read_to_string(home.path().join(".thirdshift/config.toml")).unwrap(),
            before
        );
    }
    assert_eq!(post("/theme", "theme=press-dark", csrf).status(), 200);
    assert_eq!(post("/sections", "hide=done,resolved", csrf).status(), 200);
    let look: Value = console.get("/look").json().unwrap();
    assert_eq!(look["theme"], "press-dark");
    assert_eq!(look["hidden_sections"], json!(["done", "resolved"]));
    let output = cli(home.path(), &["console", "log", "--json"]);
    let events: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(events.as_array().unwrap().len(), 2);
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["email"] == "localhost (no sign-in)")
    );
}
