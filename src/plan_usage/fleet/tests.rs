use super::*;
use crate::plan_usage::observation::Window;
use crate::plan_usage::store::Account;
use chrono::TimeDelta;
use serde_json::json;

fn row(harness: Harness, account: Option<Account>, observed: Option<DateTime<Utc>>) -> Row {
    Row {
        harness,
        installed: true,
        active: false,
        routes: Vec::new(),
        state: State {
            observed_at: observed,
            account,
            ..State::default()
        },
    }
}

fn view(rows: Vec<Row>) -> View {
    View {
        version: 1,
        harnesses: rows,
        now: Utc::now(),
    }
}

fn claude(email: &str, organization: &str) -> Option<Account> {
    Some(Account {
        signed_in: Some(true),
        label: Some(email.to_owned()),
        organization: Some(organization.to_owned()),
        ..Account::default()
    })
}

fn names(logins: &[Login]) -> Vec<(&str, Vec<&str>)> {
    logins
        .iter()
        .map(|login| {
            (
                login.harness.name(),
                login.machines.iter().map(String::as_str).collect(),
            )
        })
        .collect()
}

#[test]
fn a_shared_login_shows_once_with_its_freshest_observation_and_every_machine() {
    let now = Utc::now();
    let mut older = row(
        Harness::Claude,
        claude("me@example.test", "org-1"),
        Some(now - TimeDelta::hours(2)),
    );
    older.active = true;
    older.state.windows = vec![Window {
        used_percent: Some(10.0),
        ..Window::named("five_hour").unwrap()
    }];
    let mut fresher = row(
        Harness::Claude,
        claude("me@example.test", "org-1"),
        Some(now),
    );
    fresher.state.windows = vec![Window {
        used_percent: Some(70.0),
        ..Window::named("five_hour").unwrap()
    }];
    let logins = group(&[
        ("laptop".into(), view(vec![older])),
        ("server".into(), view(vec![fresher])),
        (
            "desk".into(),
            view(vec![row(
                Harness::Claude,
                claude("me@example.test", "org-1"),
                None,
            )]),
        ),
    ]);
    assert_eq!(
        names(&logins),
        [("claude", vec!["laptop", "server", "desk"])]
    );
    let login = &logins[0];
    assert_eq!(login.observed_on, "server");
    assert_eq!(login.state.windows[0].used_percent, Some(70.0));
    assert_eq!(login.active_on, ["laptop"]);
    assert_eq!(
        login.login.as_deref(),
        Some("organization org-1 · me@example.test")
    );
}

#[test]
fn distinct_logins_and_harnesses_without_an_identity_stay_per_machine() {
    let now = Some(Utc::now());
    let codex = |id: &str| {
        Some(Account {
            id: Some(id.to_owned()),
            ..Account::default()
        })
    };
    let muse = |label: &str| {
        Some(Account {
            label: Some(label.to_owned()),
            ..Account::default()
        })
    };
    let laptop = view(vec![
        row(Harness::Claude, claude("me@example.test", "org-1"), now),
        row(Harness::Codex, codex("acct-1"), now),
        row(Harness::Muse, muse("Personal"), now),
        row(Harness::Agy, None, now),
        row(Harness::Grok, None, now),
    ]);
    let server = view(vec![
        row(Harness::Claude, claude("work@example.test", "org-2"), now),
        row(Harness::Codex, codex("acct-1"), now),
        row(Harness::Muse, muse("Employer"), now),
        row(Harness::Agy, None, now),
        row(Harness::Grok, None, now),
    ]);
    let logins = group(&[("laptop".into(), laptop), ("server".into(), server)]);
    assert_eq!(
        names(&logins),
        [
            ("claude", vec!["laptop"]),
            ("codex", vec!["laptop", "server"]),
            ("muse", vec!["laptop"]),
            ("agy", vec!["laptop"]),
            ("grok", vec!["laptop"]),
            ("claude", vec!["server"]),
            ("muse", vec!["server"]),
            ("agy", vec!["server"]),
            ("grok", vec!["server"]),
        ]
    );
    // A Claude login that names no account stays its machine's own too.
    let logins = group(&[
        ("laptop".into(), view(vec![row(Harness::Claude, None, now)])),
        ("server".into(), view(vec![row(Harness::Claude, None, now)])),
    ]);
    assert_eq!(logins.len(), 2);
}

#[test]
fn routes_keep_the_machine_they_are_configured_on() {
    let route = |position| ConfiguredRoute {
        setting: view::Setting::Routes,
        position: Some(position),
        model: None,
        effort: None,
        max_utilization: 100,
        allow_credits: false,
        availability: view::Availability::Available,
        active: false,
    };
    let mut laptop = row(Harness::Codex, None, None);
    laptop.state.account = Some(Account {
        id: Some("acct-1".into()),
        ..Account::default()
    });
    laptop.routes = vec![route(1)];
    let mut server = laptop.clone();
    server.routes = vec![route(2)];
    let logins = group(&[
        ("laptop".into(), view(vec![laptop])),
        ("server".into(), view(vec![server])),
    ]);
    let routes: Vec<_> = logins[0]
        .routes
        .iter()
        .map(|route| (route.machine.as_str(), route.route.position))
        .collect();
    assert_eq!(routes, [("laptop", Some(1)), ("server", Some(2))]);
}

#[test]
fn another_machines_view_is_refused_when_it_holds_control_text_or_an_unknown_harness() {
    let good = json!({"version": 1, "harnesses": [{"harness": "claude", "installed": true,
        "active": false, "routes": [], "windows": [{"name": "five_hour", "used_percent": 5.0}]}]});
    assert_eq!(foreign(good.clone()).unwrap().harnesses.len(), 1);
    let mut forged = good.clone();
    forged["harnesses"][0]["windows"][0]["name"] = json!("five\u{1b}[2J");
    assert!(foreign(forged).is_err());
    let mut api_key = good.clone();
    api_key["harnesses"][0]["harness"] = json!("opencode");
    assert!(foreign(api_key).is_err());
    let mut newer = good;
    newer["version"] = json!(2);
    assert!(foreign(newer).is_err());
}

#[test]
fn a_report_is_saved_only_in_its_holders_reported_slot() {
    let logs = tempfile::tempdir().unwrap();
    let now = Utc::now();
    let pushed = |machine: &str| Report {
        version: 1,
        machine: machine.to_owned(),
        view: json!({"version": 1, "harnesses": []}),
    };
    let refused = pushed("other")
        .receive(logs.path(), "laptop", "hub", now)
        .unwrap_err();
    assert!(matches!(
        refused.downcast_ref::<Refusal>(),
        Some(Refusal::Mismatch)
    ));
    let own = pushed("hub")
        .receive(logs.path(), "hub", "hub", now)
        .unwrap_err();
    assert!(matches!(
        own.downcast_ref::<Refusal>(),
        Some(Refusal::Invalid(_))
    ));
    let mut grok = row(Harness::Grok, None, Some(now));
    grok.state.windows = vec![Window {
        used_percent: Some(90.0),
        ..Window::named("weekly").unwrap()
    }];
    Report {
        view: serde_json::to_value(view(vec![grok])).unwrap(),
        ..pushed("laptop")
    }
    .receive(logs.path(), "laptop", "hub", now)
    .unwrap();
    let saved = exact(logs.path(), "laptop", Slot::Reported).unwrap();
    assert_eq!(saved.pushed_at, Some(now));
    assert_eq!(
        saved.view.harnesses[0].state.windows[0].used_percent,
        Some(90.0)
    );
    assert!(exact(logs.path(), "laptop", Slot::Read(None)).is_none());
    // Everyone sees a reported view; a person's read only that person.
    assert_eq!(discovered(logs.path(), Some("a@example.test")).len(), 1);
    save(
        logs.path(),
        "server",
        Slot::Read(Some("b@example.test")),
        view(Vec::new()),
        now,
    )
    .unwrap();
    let machines = |person| {
        discovered(logs.path(), person)
            .into_iter()
            .map(|saved| saved.machine)
            .collect::<Vec<_>>()
    };
    assert_eq!(machines(Some("a@example.test")), ["laptop"]);
    assert_eq!(machines(Some("b@example.test")), ["laptop", "server"]);
}
