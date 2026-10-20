use super::*;

fn cli(home: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_thirdshift"))
        .args(arguments)
        .env("HOME", home)
        .output()
        .unwrap()
}

#[test]
fn custom_theme_cli_round_trip_defaults_validation_and_log() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let source = home.path().join("theme.json");
    fs::write(&source, r##"{"name":"my-night","base":"press-dark","tokens":{"bg":"#123456","font-body":"my-font.woff2, sans-serif"}}"##).unwrap();
    let imported = cli(
        home.path(),
        &["console", "theme", "import", source.to_str().unwrap()],
    );
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let exported = cli(home.path(), &["console", "theme", "export", "my-night"]);
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let theme: Value = serde_json::from_slice(&exported.stdout).unwrap();
    assert_eq!(theme["tokens"]["bg"], "#123456");
    assert_eq!(theme["tokens"]["body-size"], "13px");
    assert_eq!(theme["tokens"]["font-body"], "my-font.woff2, sans-serif");
    assert_eq!(
        serde_json::from_slice::<Value>(
            &fs::read(home.path().join(".thirdshift/console/themes/my-night.json")).unwrap()
        )
        .unwrap(),
        theme
    );
    fs::write(&source, &exported.stdout).unwrap();
    assert!(
        cli(
            home.path(),
            &["console", "theme", "import", source.to_str().unwrap()]
        )
        .status
        .success()
    );
    assert_eq!(
        cli(home.path(), &["console", "theme", "export", "my-night"]).stdout,
        exported.stdout
    );
    assert!(
        cli(home.path(), &["console", "theme", "my-night"])
            .status
            .success()
    );
    let listed = cli(home.path(), &["console", "theme", "list", "--json"]);
    assert!(
        serde_json::from_slice::<Value>(&listed.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&json!("my-night"))
    );
    for (key, value) in [
        ("unknown", "red"),
        ("bg", "url(https://example.test)"),
        ("body-size", "huge"),
        ("activity-motion", "spin 1s infinite"),
    ] {
        fs::write(
            &source,
            serde_json::to_vec(&json!({"name":"bad", "tokens":{key:value}})).unwrap(),
        )
        .unwrap();
        let result = cli(
            home.path(),
            &["console", "theme", "import", source.to_str().unwrap()],
        );
        assert!(!result.status.success(), "accepted {key}={value}");
        assert!(String::from_utf8_lossy(&result.stderr).contains(key));
    }
    let events: Value =
        serde_json::from_slice(&cli(home.path(), &["console", "log", "--json"]).stdout).unwrap();
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event"] == "theme-saved")
    );
}

#[test]
fn custom_theme_http_round_trip_and_fonts_are_confined() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let settings: Value = console.get("/look").json().unwrap();
    let csrf = settings["csrf"].as_str().unwrap();
    let client = reqwest::blocking::Client::new();
    let post = |text: String, token: &str| {
        client
            .post(format!("{}/themes", console.address))
            .header("x-csrf-token", token)
            .header("content-type", "application/json")
            .body(text)
            .send()
            .unwrap()
    };
    let theme = json!({"name":"http-theme","base":"garden-light","tokens":{"font-body":"custom.woff2, serif", "font-head":"Unknown Family, system-ui"}});
    assert_eq!(post(theme.to_string(), "wrong").status(), 403);
    let saved = post(theme.to_string(), csrf);
    assert_eq!(saved.status(), 200);
    let saved: Value = saved.json().unwrap();
    assert_eq!(
        post(saved.to_string(), csrf).json::<Value>().unwrap(),
        saved
    );
    assert_eq!(
        console.get("/themes/http-theme").json::<Value>().unwrap(),
        saved
    );
    assert_eq!(
        serde_json::from_slice::<Value>(
            &cli(home.path(), &["console", "theme", "export", "http-theme"]).stdout
        )
        .unwrap(),
        saved
    );
    let themes = home.path().join(".thirdshift/console/themes");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(themes.join("http-theme.json")).unwrap())
            .unwrap(),
        saved
    );
    assert!(
        console.get("/look").json::<Value>().unwrap()["themes"]
            .as_array()
            .unwrap()
            .contains(&json!("http-theme"))
    );
    for invalid in [
        json!({"name":"../escape"}),
        json!({"name":"invalid", "tokens":{"bg":"red;display:none"}}),
        json!({"name":"invalid", "tokens":{"font-body":"url(evil)"}}),
        json!({"name":"invalid", "unknown":true}),
    ] {
        assert_eq!(post(invalid.to_string(), csrf).status(), 400);
    }
    fs::create_dir_all(themes.join("fonts")).unwrap();
    for (name, kind) in [
        ("custom.woff2", "font/woff2"),
        ("other.woff", "font/woff"),
        ("other.ttf", "font/ttf"),
        ("other.otf", "font/otf"),
    ] {
        fs::write(themes.join("fonts").join(name), b"font fixture").unwrap();
        let response = console.get(&format!("/assets/user-fonts/{name}"));
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], kind);
        assert_eq!(response.bytes().unwrap(), b"font fixture"[..]);
    }
    fs::write(themes.join("fonts/not-font.txt"), "secret").unwrap();
    let outside = home.path().join("outside.woff2");
    fs::write(&outside, "secret").unwrap();
    symlink(&outside, themes.join("fonts/escape.woff2")).unwrap();
    fs::create_dir(themes.join("fonts/directory.woff2")).unwrap();
    for path in [
        "not-font.txt",
        "escape.woff2",
        "directory.woff2",
        "%2e%2e%2foutside.woff2",
        "nested/other.woff",
    ] {
        assert_eq!(
            console.get(&format!("/assets/user-fonts/{path}")).status(),
            404,
            "{path}"
        );
    }
    let css = console.get("/assets/user-themes.css").text().unwrap();
    assert!(css.contains("url(\"/assets/user-fonts/custom.woff2\")"));
    assert!(css.contains("--font-head: system-ui, sans-serif;"));
    assert!(!css.contains("Unknown Family"));
    assert!(css.contains("color-scheme: light;"));
    fs::rename(themes.join("fonts"), home.path().join("moved-fonts")).unwrap();
    symlink(home.path().join("moved-fonts"), themes.join("fonts")).unwrap();
    assert_eq!(console.get("/assets/user-fonts/custom.woff2").status(), 404);
    let events: Value =
        serde_json::from_slice(&cli(home.path(), &["console", "log", "--json"]).stdout).unwrap();
    assert!(
        events
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event"] == "theme-saved"
                && event["email"] == "localhost (no sign-in)")
    );
}

#[test]
fn built_in_theme_exports_can_be_imported_without_replacing_built_ins() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let source = home.path().join("builtin.json");
    for name in ["focus-auto", "garden-light", "factory-dark"] {
        let exported = cli(home.path(), &["console", "theme", "export", name]);
        assert!(exported.status.success());
        fs::write(&source, &exported.stdout).unwrap();
        let imported = cli(
            home.path(),
            &["console", "theme", "import", source.to_str().unwrap()],
        );
        assert!(
            imported.status.success(),
            "{}",
            String::from_utf8_lossy(&imported.stderr)
        );
        assert_eq!(imported.stdout, exported.stdout);
        let mut changed: Value = serde_json::from_slice(&exported.stdout).unwrap();
        changed["tokens"]["bg"] = json!("#123456");
        fs::write(&source, changed.to_string()).unwrap();
        assert!(
            !cli(
                home.path(),
                &["console", "theme", "import", source.to_str().unwrap()]
            )
            .status
            .success()
        );
    }
    let listed: Value =
        serde_json::from_slice(&cli(home.path(), &["console", "theme", "list", "--json"]).stdout)
            .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 18);
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
