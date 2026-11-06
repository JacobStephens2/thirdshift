use super::*;

fn fixture(home: &Path) -> String {
    configure(home);
    let mut parent = record(home, 7, Some("failed"), 0);
    parent["started_at"] = json!("2026-10-01T12:00:00Z");
    parent["ended_at"] = json!("2026-10-01T12:01:00Z");
    parent["stages"] = json!([{"name": "Tickets", "began_at": parent["started_at"]}]);
    write_record(home, 7, &parent);
    let dir = home.join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::write(
        dir.join("7-fixture.log"),
        "Remote Command output <unsafe>\n",
    )
    .unwrap();
    let session = dir.join("99-20261001T120000+0000-implement.logs.jsonl");
    fs::write(&session, include_str!("../fixtures/waterfall-logs.jsonl")).unwrap();
    let mut child = parent.clone();
    child["kind"] = json!("Run");
    child["issue"]["number"] = json!(99);
    child["parent_record"] = json!(dir.join("7-fixture.command.json"));
    child["stages"] =
        json!([{"name": "implement", "began_at": "2026-10-01T12:00:10Z", "session_log": session}]);
    child["session_log"] = json!(session);
    fs::write(
        dir.join("7-fixture.issue-99.child.json"),
        serde_json::to_vec(&child).unwrap(),
    )
    .unwrap();
    session.to_string_lossy().into_owned()
}

fn pair(caller: &Path, owner: &Console, owner_home: &Path, scopes: &str) {
    let token = tokens::create(owner_home, scopes, &["--scopes", scopes]);
    assert!(
        super::fleet::add(caller, "remote", &owner.address, &token)
            .status
            .success()
    );
}

#[test]
fn hostname_alias_details_stay_remote_while_api_and_cli_stay_local() {
    let caller_home = tempfile::tempdir().unwrap();
    let owner_home = tempfile::tempdir().unwrap();
    configure(caller_home.path());
    let mut local = record(caller_home.path(), 7, Some("failed"), 0);
    local["cause"] = json!("LOCAL_COMMAND");
    write_record(caller_home.path(), 7, &local);
    fixture(owner_home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let owner = Console::start(binary, owner_home.path());
    let hostname = api::cli(caller_home.path(), &["console", "whoami", "--json"])["hostname"]
        .as_str()
        .unwrap()
        .to_owned();
    let token = tokens::create(owner_home.path(), "caller", &["--scopes", "read,security"]);
    assert!(
        super::fleet::add(caller_home.path(), &hostname, &owner.address, &token)
            .status
            .success()
    );
    let caller = Console::start(binary, caller_home.path());
    let selector = owner_home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.command.json")
        .to_string_lossy()
        .into_owned();
    for route in ["/peek", "/waterfall", "/spans", "/log"] {
        let mut owning_url = reqwest::Url::parse(&owner.address)
            .unwrap()
            .join(route)
            .unwrap();
        owning_url
            .query_pairs_mut()
            .append_pair("command", &selector);
        assert_eq!(
            reqwest::blocking::get(owning_url).unwrap().status(),
            200,
            "owning {route}"
        );
        let mut url = reqwest::Url::parse(&caller.address)
            .unwrap()
            .join(route)
            .unwrap();
        url.query_pairs_mut()
            .append_pair("command", &selector)
            .append_pair("machine", &hostname);
        let response = reqwest::blocking::get(url).unwrap();
        assert_eq!(
            response.status(),
            200,
            "{route}: {}",
            response.text().unwrap()
        );
    }
    let reader = tokens::create(caller_home.path(), "local-reader", &["--scopes", "read"]);
    let mut url = reqwest::Url::parse(&caller.address)
        .unwrap()
        .join("/api/v1/peek")
        .unwrap();
    url.query_pairs_mut()
        .append_pair("command", "widgets#7")
        .append_pair("machine", &hostname);
    let details: Value = reqwest::blocking::Client::new()
        .get(url)
        .bearer_auth(reader)
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(details["command"]["cause"], "LOCAL_COMMAND", "{details}");
    assert_eq!(
        api::cli(
            caller_home.path(),
            &["show", "widgets#7", "--machine", &hostname, "--json"]
        )["command"]["cause"],
        "LOCAL_COMMAND"
    );
}

#[test]
fn remote_command_details_render_locally_and_fetch_fresh_without_writing_content() {
    let caller_home = tempfile::tempdir().unwrap();
    let owner_home = tempfile::tempdir().unwrap();
    configure(caller_home.path());
    fixture(owner_home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let owner = Console::start(binary, owner_home.path());
    pair(
        caller_home.path(),
        &owner,
        owner_home.path(),
        "read,security",
    );
    let caller = Console::start(binary, caller_home.path());
    let response = caller.get("/peek?machine=remote&command=widgets%237");
    assert_eq!(
        response.status(),
        200,
        "remote Command must open on the caller"
    );
    let html = response.text().unwrap();
    for text in [
        "Command details",
        "Tickets",
        "implement",
        "99",
        "Remote Command output &#60;unsafe&#62;",
    ] {
        assert!(html.contains(text), "missing {text}: {html}");
    }
    assert!(html.contains(&owner.address), "owning Console link: {html}");
    assert!(html.contains("machine=remote"), "remote links: {html}");
    let command_log = owner_home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/7-fixture.log");
    fs::write(
        &command_log,
        format!("{}\nRemote tail <unsafe>\n", "🦀".repeat(1500)),
    )
    .unwrap();
    let preview = |html: String| {
        html.split("aria-label=\"Recent Command output\"")
            .nth(1)
            .unwrap()
            .split("<pre>")
            .nth(1)
            .unwrap()
            .split("</pre>")
            .next()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        preview(
            caller
                .get("/peek?machine=remote&command=widgets%237")
                .text()
                .unwrap()
        ),
        preview(owner.get("/peek?command=widgets%237").text().unwrap()),
        "remote preview matches the owning Console's bounded tail"
    );
    fs::write(
        &command_log,
        format!("{}\nlast remote line\n", "x".repeat(1024 * 1024)),
    )
    .unwrap();
    let tail = api::json(owner.get("/api/v1/log?command=widgets%237&tail=true"));
    assert_eq!(tail["contents"].as_str().unwrap().len(), 4096);
    assert_eq!(
        tail,
        api::cli(
            owner_home.path(),
            &["logs", "widgets#7", "--tail", "--json"]
        )
    );
    assert!(
        caller
            .get("/peek?machine=remote&command=widgets%237")
            .text()
            .unwrap()
            .contains("last remote line")
    );
    fs::write(command_log, "Fresh remote output\n").unwrap();
    assert!(
        caller
            .get("/peek?machine=remote&command=widgets%237")
            .text()
            .unwrap()
            .contains("Fresh remote output")
    );
    fn check(root: &Path) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                check(&path);
            } else {
                let text = fs::read(&path).unwrap();
                let text = String::from_utf8_lossy(&text);
                assert!(
                    !text.contains("Remote Command output")
                        && !text.contains("Fresh remote output")
                        && !text.contains("first <unsafe>"),
                    "remote content in {}",
                    path.display()
                );
            }
        }
    }
    check(caller_home.path());
}

#[test]
fn remote_waterfall_and_log_links_keep_the_owning_command() {
    let caller_home = tempfile::tempdir().unwrap();
    let owner_home = tempfile::tempdir().unwrap();
    configure(caller_home.path());
    let session = fixture(owner_home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let owner = Console::start(binary, owner_home.path());
    pair(
        caller_home.path(),
        &owner,
        owner_home.path(),
        "read,security",
    );
    let caller = Console::start(binary, caller_home.path());
    let tree: Value = owner.get("/spans?command=widgets%237").json().unwrap();
    let response = caller.get("/waterfall?machine=remote&command=widgets%237");
    assert_eq!(
        response.status(),
        200,
        "remote Waterfall opens on the caller"
    );
    let html = response.text().unwrap();
    let data: Value = serde_json::from_str(
        html.split("id=\"span-data\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        data["spans"].as_array().unwrap().len(),
        tree["spans"].as_array().unwrap().len()
    );
    assert!(
        data["spans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|span| span["kind"] == "tool")
    );
    for span in data["spans"].as_array().unwrap() {
        assert!(span["command_url"].as_str().unwrap().starts_with("/peek?"));
        assert!(
            span["command_url"]
                .as_str()
                .unwrap()
                .contains("machine=remote")
        );
    }
    for query in [
        "command=widgets%237".to_owned(),
        format!(
            "command=widgets%2399&session={}",
            reqwest::Url::parse("http://localhost")
                .unwrap()
                .query_pairs_mut()
                .append_pair("s", &session)
                .finish()
                .query()
                .unwrap()
                .strip_prefix("s=")
                .unwrap()
        ),
    ] {
        let local = owner.get(&format!("/log?{query}")).text().unwrap();
        let response = caller.get(&format!("/log?machine=remote&{query}"));
        assert_eq!(response.status(), 200, "remote log {query}");
        let remote = response.text().unwrap();
        // Both pages use the same escaped Command/conversation renderer.
        let content = |html: &str| {
            html.split("data-offset=\"")
                .nth(1)
                .unwrap()
                .split_once('>')
                .unwrap()
                .1
                .split("</section>")
                .next()
                .unwrap()
                .to_owned()
        };
        assert_eq!(content(&remote), content(&local));
        assert!(remote.contains("machine=remote"));
        assert!(remote.contains(&owner.address));
    }
}

#[test]
fn details_cli_machine_returns_the_owning_json_and_local_hostname_keeps_local_behavior() {
    let caller_home = tempfile::tempdir().unwrap();
    let owner_home = tempfile::tempdir().unwrap();
    configure(caller_home.path());
    fixture(owner_home.path());
    api::freeze_clock(owner_home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let owner = Console::start_command({
        let mut command = Command::new(binary);
        command
            .args(["console", "--no-sign-in"])
            .env("HOME", owner_home.path())
            .env("LD_PRELOAD", owner_home.path().join("api-clock.so"));
        command
    });
    pair(
        caller_home.path(),
        &owner,
        owner_home.path(),
        "read,security",
    );
    for args in [
        vec!["show", "widgets#7", "--json"],
        vec!["show", "widgets#7", "--spans", "--json"],
        vec!["logs", "widgets#7", "--json"],
        vec!["logs", "widgets#7", "--tail", "--json"],
        vec!["logs", "widgets#99", "--session", "implement", "--json"],
    ] {
        let mut expected = api::cli(owner_home.path(), &args);
        let mut remote = args.clone();
        remote.extend(["--machine", "remote"]);
        let mut actual = api::cli(caller_home.path(), &remote);
        if args.contains(&"--spans") {
            // Capture instants differ, while every retained span fact agrees.
            expected.as_object_mut().unwrap().remove("now");
            actual.as_object_mut().unwrap().remove("now");
        }
        assert_eq!(actual, expected, "{args:?}");
    }
    let tree = api::cli(
        owner_home.path(),
        &["show", "widgets#7", "--spans", "--json"],
    );
    let span = tree["spans"][0]["id"].as_str().unwrap();
    assert_eq!(
        api::cli(
            caller_home.path(),
            &[
                "show",
                "widgets#7",
                "--span",
                span,
                "--machine",
                "remote",
                "--json"
            ]
        ),
        api::cli(
            owner_home.path(),
            &["show", "widgets#7", "--span", span, "--json"]
        )
    );
    let hostname = api::cli(owner_home.path(), &["console", "whoami", "--json"])["hostname"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        api::cli(
            owner_home.path(),
            &["show", "widgets#7", "--machine", &hostname, "--json"]
        ),
        api::cli(owner_home.path(), &["show", "widgets#7", "--json"])
    );
    for flags in [
        vec!["--machine"],
        vec!["--machine", "missing"],
        vec!["--machine", "remote", "--machine", "remote"],
    ] {
        let failed = Command::new(binary)
            .args(["show", "widgets#7"])
            .args(&flags)
            .env("HOME", caller_home.path())
            .output()
            .unwrap();
        assert!(!failed.status.success(), "{flags:?}: {failed:?}");
    }
}

#[test]
fn remote_security_details_and_dispatched_traces_require_security_on_the_owning_machine() {
    let caller_home = tempfile::tempdir().unwrap();
    let owner_home = tempfile::tempdir().unwrap();
    configure(caller_home.path());
    fixture(owner_home.path());
    let dir = owner_home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    let mut security: Value =
        serde_json::from_slice(&fs::read(dir.join("7-fixture.command.json")).unwrap()).unwrap();
    security["kind"] = json!("Security run");
    security["security_findings"] = json!([{"title": "Private remote finding <unsafe>", "url": "https://github.com/acme/widgets/security/advisories/GHSA-test", "severity": "high"}]);
    write_record(owner_home.path(), 7, &security);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let owner = Console::start(binary, owner_home.path());
    pair(caller_home.path(), &owner, owner_home.path(), "read");
    let caller = Console::start(binary, caller_home.path());
    let paths = [
        "/peek?machine=remote&command=widgets%237",
        "/waterfall?machine=remote&command=widgets%237",
        "/spans?machine=remote&command=widgets%237",
        "/log?machine=remote&command=widgets%237",
        "/log?machine=remote&command=widgets%2399&session=implement",
    ];
    for path in paths {
        let denied = caller.get(path);
        assert_eq!(denied.status(), 403, "{path}");
        let html = denied.text().unwrap();
        assert!(
            html.contains("needs the `security` scope on `remote`"),
            "{path}: {html}"
        );
        assert!(!html.contains("Private remote finding"), "{html}");
    }
    let failed = Command::new(binary)
        .args(["show", "widgets#7", "--machine", "remote", "--json"])
        .env("HOME", caller_home.path())
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("needs the `security` scope on `remote`")
    );
    pair(
        caller_home.path(),
        &owner,
        owner_home.path(),
        "read,security",
    );
    for path in paths {
        let response = caller.get(path);
        assert_eq!(response.status(), 200, "{path}");
        if path.starts_with("/peek") {
            assert!(
                response
                    .text()
                    .unwrap()
                    .contains("Private remote finding &#60;unsafe&#62;")
            );
        }
    }
}

#[test]
fn details_api_does_not_relay_a_read_token_through_broader_fleet_credentials() {
    let caller_home = tempfile::tempdir().unwrap();
    let owner_home = tempfile::tempdir().unwrap();
    configure(caller_home.path());
    fixture(owner_home.path());
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let owner = Console::start(binary, owner_home.path());
    pair(
        caller_home.path(),
        &owner,
        owner_home.path(),
        "read,security",
    );
    let reader = tokens::create(caller_home.path(), "reader", &[]);
    let caller = Console::start(binary, caller_home.path());
    // The versioned API reads its owning machine, as status already does.
    // Fleet clients call the selected machine's API directly with its token.
    for path in [
        "/api/v1/peek?machine=remote&command=widgets%237",
        "/api/v1/waterfall?machine=remote&command=widgets%237",
        "/api/v1/spans?machine=remote&command=widgets%237",
        "/api/v1/log?machine=remote&command=widgets%2399&session=implement",
    ] {
        let response = tokens::request(&caller, &reader, path).send().unwrap();
        assert_eq!(
            response.status(),
            400,
            "{path}: the owning API must not proxy another machine"
        );
        let error: Value = response.json().unwrap();
        assert!(error["error"].as_str().unwrap().contains("owning machine"));
    }
}
