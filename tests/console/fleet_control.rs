use super::*;

fn paired(caller: &Path, owner: &Path, console: &Console, scopes: &str) -> String {
    let token = tokens::create(owner, "caller-console", &["--scopes", scopes]);
    let result = super::super::fleet::add(caller, "owner", &console.address, &token);
    assert!(result.status.success(), "{result:?}");
    token
}

#[test]
fn signed_in_fleet_start_runs_the_owners_harness_and_merges_its_inbox() {
    let scenario = super::super::start::fixture();
    scenario.user_config_is("[console]\nport = 0\nallowed_emails = ['owner@example.test']\n");
    scenario.agent_waits_for_release("exit 1\n");
    struct Release<'a>(&'a support::Scenario);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            let _ = fs::write(self.0.path("release"), "");
            let _ = self.0.run(&["stop", "acme/widgets#7", "--json"]);
        }
    }
    let _release = Release(&scenario);
    let owner = super::super::start::server(&scenario);
    let caller = configured();
    assert!(
        password(caller.path(), None, PASSWORD, PASSWORD)
            .0
            .success()
    );
    paired(caller.path(), &scenario.path("home"), &owner, "read,run");
    // Local privileges are independent of what this person may do on owner.
    assert!(
        cli(caller.path(), &["console", "scopes", "--scopes", "read"])
            .status
            .success()
    );
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        caller.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let look = api::json(get(&console, "/look", &session));
    let page = get(&console, "/", &session).text().unwrap();
    assert!(page.contains("acme/widgets · owner"), "{page}");
    let response = client()
        .post(format!("{}/fleet/runs?machine=owner", console.address))
        .header("cookie", &session)
        .header("x-csrf-token", look["csrf"].as_str().unwrap())
        .json(&json!({"repository":"acme/widgets","issue":7,"email":{"mode":"never"}}))
        .send()
        .unwrap();
    assert_eq!(api::json(response)["name"], "acme/widgets#7");
    super::super::start::wait_for(&scenario.path("started"));
    let record = scenario.command_record("home/.thirdshift/logs/acme/widgets/commands/issue");
    assert_eq!(record["started_by"]["who"], EMAIL);
    assert_eq!(record["started_by"]["console_holder"], "caller-console");
    let activity =
        fs::read_to_string(scenario.path("home/.thirdshift/logs/acme/widgets/activity.log"))
            .unwrap();
    assert!(
        activity.contains("by owner@example.test from the Console on caller-console"),
        "{activity}"
    );
    let inbox = api::cli(caller.path(), &["status", "--json"]);
    assert!(
        inbox["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |command| command["machine_name"] == "owner" && command["name"] == "acme/widgets#7"
            ),
        "{inbox}"
    );
    assert!(
        !caller
            .path()
            .join(".thirdshift/logs/acme/widgets/commands")
            .exists()
    );
}

#[test]
fn fleet_config_uses_owners_admin_scope_and_refusals_survive_forwarding() {
    let owner = configured();
    let caller = configured();
    assert!(
        password(caller.path(), None, PASSWORD, PASSWORD)
            .0
            .success()
    );
    let owner_console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), owner.path());
    paired(caller.path(), owner.path(), &owner_console, "read,admin");
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        caller.path(),
        false,
    );
    let session = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let page = get(&console, "/config?machine=owner", &session)
        .text()
        .unwrap();
    assert!(page.contains("Defaults on owner"), "{page}");
    let token = csrf(&page);
    let response = client()
        .post(format!(
            "{}/config/spec.parallel?machine=owner",
            console.address
        ))
        .header("cookie", &session)
        .form(&[("csrf", token.as_str()), ("value", "5")])
        .send()
        .unwrap();
    assert_eq!(response.status(), 303, "{}", response.text().unwrap());
    assert_eq!(
        response.headers()["location"],
        "/config?machine=owner&saved=spec.parallel#config-spec.parallel"
    );
    let saved = get(
        &console,
        response.headers()["location"].to_str().unwrap(),
        &session,
    )
    .text()
    .unwrap();
    assert!(saved.contains("role=\"status\">Saved <code>spec.parallel = 5</code> on owner."));
    assert_eq!(
        api::cli(owner.path(), &["config", "spec.parallel", "--json"])["value"],
        5
    );
    assert_eq!(
        api::cli(caller.path(), &["config", "spec.parallel", "--json"])["value"],
        3
    );
    let refused = client()
        .post(format!(
            "{}/config/console.allowed_emails?machine=owner",
            console.address
        ))
        .header("cookie", &session)
        .form(&[
            ("csrf", token.as_str()),
            ("value", "['intruder@example.test']"),
        ])
        .send()
        .unwrap();
    assert_eq!(refused.status(), 422);
    assert!(
        cli(
            owner.path(),
            &["console", "token", "revoke", "caller-console"]
        )
        .status
        .success()
    );
    paired(caller.path(), owner.path(), &owner_console, "read");
    let page = get(&console, "/config", &session).text().unwrap();
    assert!(!page.contains("/config?machine=owner"));
    let refused = client()
        .post(format!(
            "{}/config/spec.parallel?machine=owner",
            console.address
        ))
        .header("cookie", &session)
        .form(&[("csrf", token.as_str()), ("value", "6")])
        .send()
        .unwrap();
    assert_eq!(refused.status(), 403);
    let refused = client()
        .post(format!("{}/fleet/runs?machine=owner", console.address))
        .header("cookie", &session)
        .header("x-csrf-token", &token)
        .json(&json!({"repository":"acme/widgets","issue":7}))
        .send()
        .unwrap();
    assert_eq!(refused.status(), 403);
    let inbox = get(&console, "/", &session).text().unwrap();
    assert!(!inbox.contains("acme/widgets · owner"));
    // An API token cannot use a query to borrow the caller's saved admin token.
    let reader = tokens::create(caller.path(), "reader", &[]);
    let refusal = client()
        .post(format!(
            "{}/api/v1/config/spec.parallel?machine=owner",
            console.address
        ))
        .bearer_auth(reader)
        .form(&[("value", "8")])
        .send()
        .unwrap();
    assert_eq!(refusal.status(), 403);
}

#[test]
fn a_cached_peer_snapshot_stays_with_the_person_who_read_it() {
    const OTHER: &str = "other@example.test";
    let owner = configured();
    let mut value = record(owner.path(), 73, Some("failed"), 0);
    value["name"] = json!("widgets#73");
    write_record(owner.path(), 73, &value);
    let caller = configured();
    fs::write(
        caller.path().join(".thirdshift/config.toml"),
        format!("[console]\nport = 0\nallowed_emails = [\"{EMAIL}\", \"{OTHER}\"]\n"),
    )
    .unwrap();
    for email in [EMAIL, OTHER] {
        assert!(
            password(caller.path(), Some(email), PASSWORD, PASSWORD)
                .0
                .success()
        );
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let owner_console = Console::start(binary, owner.path());
    paired(caller.path(), owner.path(), &owner_console, "read");
    let console = Console::start_mode(binary, caller.path(), false);
    let allowed = cookie(&sign_in(&console, EMAIL, PASSWORD));
    let other = cookie(&sign_in(&console, OTHER, PASSWORD));
    let page = get(&console, "/metrics?window=7d", &allowed)
        .text()
        .unwrap();
    assert!(page.contains("widgets#73</a> on owner"), "{page}");
    // The owner does not list the other person, so it refuses them.
    let page = get(&console, "/metrics?window=7d", &other).text().unwrap();
    assert!(!page.contains("widgets#73"), "{page}");
    // Its saved snapshot must not answer for it once it is offline.
    drop(owner_console);
    let page = get(&console, "/metrics?window=7d", &other).text().unwrap();
    assert!(!page.contains("widgets#73"), "{page}");
    let page = get(&console, "/metrics?window=7d", &allowed)
        .text()
        .unwrap();
    assert!(
        page.contains("widgets#73 (cached, acme/widgets, "),
        "{page}"
    );
}
