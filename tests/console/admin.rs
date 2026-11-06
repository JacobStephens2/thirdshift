use super::*;

#[test]
fn machine_changes_require_admin_and_import_theme_content_without_paths() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let admin = tokens::create(home.path(), "owner", &["--scopes", "admin"]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let client = reqwest::blocking::Client::new();
    for scope in ["read", "act", "run"] {
        let token = tokens::create(home.path(), scope, &["--scopes", scope]);
        for route in ["/update", "/themes", "/evals/import-replays"] {
            let response = client
                .post(format!("{}/api/v1{route}", console.address))
                .bearer_auth(&token)
                .json(&json!({"name":"custom-admin", "base":"focus-auto"}))
                .send()
                .unwrap();
            assert_eq!(response.status(), 403, "{scope}: {route}");
            assert!(
                response.json::<Value>().unwrap()["error"]
                    .as_str()
                    .unwrap()
                    .contains("admin")
            );
        }
    }
    let document = json!({"name":"custom-admin", "base":"focus-auto"});
    let imported = api::json(
        client
            .post(format!("{}/api/v1/themes", console.address))
            .bearer_auth(&admin)
            .json(&document)
            .send()
            .unwrap(),
    );
    assert_eq!(imported["name"], "custom-admin");
    assert_eq!(
        api::cli(home.path(), &["console", "theme", "export", "custom-admin"]),
        imported
    );
    let path = home.path().join("local-theme.json");
    fs::write(
        &path,
        document.to_string().replace("custom-admin", "custom-path"),
    )
    .unwrap();
    for body in [path.display().to_string(), json!({"path":path}).to_string()] {
        let response = client
            .post(format!("{}/api/v1/themes", console.address))
            .bearer_auth(&admin)
            .body(body)
            .send()
            .unwrap();
        assert_eq!(response.status(), 422);
    }
    let log = api::cli(home.path(), &["console", "log", "--json"]);
    assert!(
        log.as_array()
            .unwrap()
            .iter()
            .any(|event| event["event"] == "theme-saved" && event["email"] == "owner")
    );
}

#[test]
fn admin_imports_replay_data_with_cli_parity_and_audit() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    for (path, content) in [
        (
            "inputs-2026-10-07/manifest.json",
            json!({"timing_bugs":[],"spec_reviews":[]}),
        ),
        ("2026-10-09/inputs/manifest.json", json!({"codex_bugs":[]})),
        (
            "2026-10-09/grading/eval-import.json",
            json!({"bugs":{},"specs":{}}),
        ),
    ] {
        let path = home.path().join(".thirdshift/replays").join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content.to_string()).unwrap();
    }
    let token = tokens::create(home.path(), "data-owner", &["--scopes", "admin"]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let result = api::json(
        reqwest::blocking::Client::new()
            .post(format!("{}/api/v1/evals/import-replays", console.address))
            .bearer_auth(&token)
            .send()
            .unwrap(),
    );
    assert_eq!(result, json!({"imported":0,"skipped":0}));
    assert_eq!(result, api::cli(home.path(), &["evals", "import-replays"]));
    let suites = api::cli(home.path(), &["evals", "list"]);
    assert_eq!(suites.as_array().unwrap().len(), 2);
    let log = api::cli(home.path(), &["console", "log", "--json"]);
    assert!(
        log.as_array()
            .unwrap()
            .iter()
            .any(|event| event["email"] == "data-owner"
                && event["endpoint"] == "/api/v1/evals/import-replays"
                && event["outcome"] == 200)
    );
}
