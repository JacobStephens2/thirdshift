use super::*;

#[test]
fn admin_reads_and_sets_config_with_cli_parity_and_console_log_values() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let path = home.path().join(".thirdshift/config.toml");
    let original =
        "# Local defaults\n[console]\nport = 0\n\n[spec]\nparallel = 3 # keep this comment\n";
    fs::write(&path, original).unwrap();
    let admin = tokens::create(home.path(), "machine-owner", &["--scopes", "admin"]);
    let reader = tokens::create(home.path(), "observer", &[]);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for (route, args) in [
        ("/api/v1/config", vec!["config", "--json"]),
        (
            "/api/v1/config/spec.parallel",
            vec!["config", "spec.parallel", "--json"],
        ),
    ] {
        assert_eq!(
            api::json(tokens::request(&console, &admin, route).send().unwrap()),
            api::cli(home.path(), &args)
        );
        let refused = tokens::request(&console, &reader, route).send().unwrap();
        assert_eq!(refused.status(), 403);
        assert!(
            refused.json::<Value>().unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("admin")
        );
    }
    let post = |token: &str, key: &str, value: &str| {
        reqwest::blocking::Client::new()
            .post(format!("{}/api/v1/config/{key}", console.address))
            .bearer_auth(token)
            .form(&[("value", value)])
            .send()
            .unwrap()
    };
    let refused = post(&reader, "spec.parallel", "8");
    assert_eq!(refused.status(), 403);
    assert!(
        refused.json::<Value>().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("admin")
    );
    let saved = api::json(post(&admin, "spec.parallel", "6"));
    assert_eq!(saved["value"], 6);
    assert_eq!(
        saved,
        api::cli(home.path(), &["config", "spec.parallel", "--json"])
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original.replace("parallel = 3", "parallel = 6")
    );
    for key in [
        "console.allowed_emails",
        "console.address_scopes",
        "console.fleet",
    ] {
        let refused = post(&admin, key, "[]");
        assert_eq!(refused.status(), 422);
        let message = refused.json::<Value>().unwrap()["error"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            message.contains("access") && message.contains("machine"),
            "{message}"
        );
    }
    let invalid = post(&admin, "spec.parallel", "0");
    assert_eq!(invalid.status(), 422);
    assert!(
        invalid.json::<Value>().unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("spec.parallel must be a whole number from 1 up")
    );
    let log = api::cli(home.path(), &["console", "log", "--json"]);
    let changes: Vec<_> = log
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["event"] == "config-changed")
        .collect();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["email"], "machine-owner");
    assert_eq!(
        changes[0]["change"],
        json!({"key":"spec.parallel", "old":3, "new":6})
    );
}
