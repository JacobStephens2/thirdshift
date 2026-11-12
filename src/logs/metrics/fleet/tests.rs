use super::*;
use crate::args::MetricsWindow;
use serde_json::json;

#[path = "capture_tests.rs"]
mod capture;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 12).unwrap()
}

fn window() -> Window {
    Window::of(
        &MetricsArgs {
            window: Some(MetricsWindow::Days7),
            ..MetricsArgs::default()
        },
        today(),
    )
}

fn tally(succeeded: usize, failed: usize) -> Value {
    let n = succeeded + failed;
    json!({
        "runs": succeeded + failed, "succeeded": succeeded, "failed": failed,
        "paused": 0, "stopped": 0, "live": 0, "n": succeeded + failed,
        "observed": succeeded + failed, "missing": 0,
        "success_rate": (n > 0).then(|| succeeded as f64 / n as f64),
        "sub_outcomes": {"succeeded": {"merged": succeeded}, "failed": {"failed": failed}}
    })
}

/// One machine's `metrics --window 7d --json`, as its API returns it.
fn summary(name: &str, current: Span, succeeded: usize, failed: usize) -> Summary {
    let mut bucket = tally(succeeded, failed);
    bucket["bucket"] = json!("2026-11-10");
    bucket["account"] = json!("jacob");
    bucket["kind"] = json!("Run");
    bucket["failed_commands"] = json!([format!("widgets#{name}")]);
    let mut stale = tally(1, 0);
    stale["bucket"] = json!("2026-10-01");
    stale["account"] = Value::Null;
    stale["kind"] = json!("Run");
    let prior = Span {
        from: current.from - chrono::Days::new(7),
        through: current.from - chrono::Days::new(1),
    };
    serde_json::from_value(json!({
        "version": 1, "hostname": "same-host", "outcomes": [bucket, stale],
        "comparison": {"current": current, "prior": prior, "rows": [{
            "account": "jacob", "kind": "Run",
            "current": tally(succeeded, failed), "prior": tally(1, 1),
            "delta": {"success_rate": null, "runs": 0, "succeeded": 0, "failed": 0, "paused": 0, "stopped": 0}
        }]},
        "where_runs_failed": {"failed": failed, "observed": failed, "missing": 0, "rows": [{
            "stopped_at": "implement", "category": "stage_failed", "recorded": true,
            "failed": failed,
            "commands": (0..failed).map(|index| json!({
                "name": format!("{name}-{index}"), "kind": "Run", "account": "jacob",
                "started_at": "2026-11-10T00:00:00Z", "selector": format!("widgets#{index}")
            })).collect::<Vec<_>>()
        }]},
        "versions": {"rows": [], "transitions": []}
    }))
    .unwrap()
}

fn view(query: &str, filters: &str) -> View {
    View {
        query: query.into(),
        filters: filters.into(),
        weekly: false,
    }
}

fn machine(name: &str, status: Status) -> Machine {
    Machine {
        name: name.into(),
        address: None,
        local: name == "here",
        status,
        unavailable_accounts: BTreeSet::new(),
        last_seen: None,
        pushed_at: None,
        error: None,
    }
}

#[test]
fn a_capture_keeps_saved_sources_until_the_next_operation() {
    let logs = tempfile::tempdir().unwrap();
    let mut config = UserConfig::parse("", &logs.path().join("config.toml"), logs.path()).unwrap();
    config.logs_dir = logs.path().to_owned();
    let asked = MetricsArgs {
        machine: Some("retired".into()),
        window: Some(MetricsWindow::Days7),
        ..MetricsArgs::default()
    };
    let window = Window::of(&asked, Utc::now().date_naive());
    save(
        logs.path(),
        "retired",
        Slot::Read(None),
        &View::of(&asked, &window),
        &summary("retired", window.current().unwrap(), 2, 0),
        Utc::now(),
    )
    .unwrap();
    let capture = Capture::new(&config, None);
    assert!(capture.default_view());
    assert_eq!(capture.others(), ["retired"]);
    std::fs::remove_file(path(logs.path(), "retired", Slot::Read(None))).unwrap();
    assert_eq!(capture.others(), ["retired"]);
    let loaded = capture.load(&asked).unwrap();
    assert_eq!(loaded.machines[0].name, "retired");
    assert_eq!(loaded.machines[0].status, Status::Cached);
    let fresh = Capture::new(&config, None);
    assert!(!fresh.default_view());
    assert!(fresh.others().is_empty());
    assert!(
        fresh
            .load(&asked)
            .err()
            .unwrap()
            .to_string()
            .contains("unknown Fleet machine")
    );
}

#[test]
fn fleet_totals_add_each_machine_once_and_keep_its_own_rows() {
    let current = window().current().unwrap();
    let fleet = merge(
        None,
        window(),
        Filters::of(&MetricsArgs::default()),
        vec![
            (
                machine("here", Status::Ok),
                Some(summary("here", current, 3, 1)),
            ),
            (
                machine("east", Status::Ok),
                Some(summary("east", current, 1, 2)),
            ),
            (machine("west", Status::Unavailable), None),
        ],
    );
    // Two machines' buckets on the same day stay two rows, each tagged; the
    // stale bucket outside today's window is not shown.
    let rows: Vec<_> = fleet
        .outcomes
        .iter()
        .map(|bucket| (bucket.machine.as_str(), bucket.row.tally.runs))
        .collect();
    assert_eq!(rows, [("here", 4), ("east", 3)]);
    let comparison = fleet.comparison.unwrap();
    assert_eq!(comparison.fleet.len(), 1);
    let total = &comparison.fleet[0];
    assert_eq!((total.current.succeeded, total.current.failed), (4, 3));
    assert_eq!((total.prior.succeeded, total.prior.failed), (2, 2));
    assert_eq!(total.current.rate(), "57% (4/7)");
    assert_eq!(total.delta.rate(), "+7 pts");
    assert_eq!(
        total.current.sub_outcomes[&super::super::Class::Failed]["failed"],
        3
    );
    assert_eq!(comparison.rows.len(), 2);
    assert_eq!(comparison.rows[1].machine.as_deref(), Some("east"));
    assert_eq!(comparison.rows[1].delta.rate(), "-17 pts");
    let failed = &fleet.where_runs_failed;
    assert_eq!((failed.failed, failed.observed, failed.missing), (3, 3, 0));
    assert_eq!(failed.rows.len(), 1);
    assert_eq!(failed.rows[0].failed, 3);
    assert_eq!(
        failed.rows[0].machines,
        BTreeMap::from([("east".into(), 2), ("here".into(), 1)])
    );
    assert_eq!(
        fleet.machines.iter().map(|m| m.status).collect::<Vec<_>>(),
        [Status::Ok, Status::Ok, Status::Unavailable]
    );
}

#[test]
fn a_cached_comparison_for_other_dates_is_left_out_and_said() {
    let current = window().current().unwrap();
    let older = Span {
        from: current.from - chrono::Days::new(2),
        through: current.through - chrono::Days::new(2),
    };
    let fleet = merge(
        None,
        window(),
        Filters::of(&MetricsArgs::default()),
        vec![(
            machine("east", Status::Cached),
            Some(summary("east", older, 1, 1)),
        )],
    );
    let comparison = fleet.comparison.unwrap();
    assert!(comparison.rows.is_empty() && comparison.fleet.is_empty());
    assert_eq!(
        fleet.outcomes.len(),
        1,
        "its buckets in the window still show"
    );
    assert_eq!(fleet.where_runs_failed.failed, 1);
    assert_eq!(
        fleet.machines[0].error.as_deref(),
        Some("its saved comparison covers other dates")
    );
}

#[test]
fn a_saved_snapshot_keeps_each_view_and_only_the_newest_views() {
    let logs = tempfile::tempdir().unwrap();
    let current = window().current().unwrap();
    let start = Utc::now();
    for index in 0..=VIEWS {
        let at = start + chrono::Duration::seconds(index as i64);
        let view = View {
            query: format!("window={index}"),
            filters: String::new(),
            weekly: false,
        };
        save(
            logs.path(),
            "east/one",
            Slot::Read(None),
            &view,
            &summary("east", current, index, 0),
            at,
        )
        .unwrap();
    }
    let saved = read_saved(&path(logs.path(), "east/one", Slot::Read(None))).unwrap();
    assert_eq!(saved.machine, "east/one");
    assert_eq!(saved.views.len(), VIEWS);
    assert!(
        !saved.views.contains_key("window=0"),
        "the oldest view went"
    );
    assert_eq!(
        saved.last_seen,
        start + chrono::Duration::seconds(VIEWS as i64)
    );
    let mut cached_machine = machine("east/one", Status::Cached);
    let read = read_saved(&path(logs.path(), "east/one", Slot::Read(None))).unwrap();
    let summary = cached(
        &mut cached_machine,
        &read,
        &view(&format!("window={VIEWS}"), "x"),
        &window(),
    )
    .unwrap();
    assert_eq!(summary.outcomes[0].tally.succeeded, VIEWS);
    assert!(cached_machine.error.is_none());
    let read = read_saved(&path(logs.path(), "east/one", Slot::Read(None))).unwrap();
    assert!(
        cached(
            &mut cached_machine,
            &read,
            &view("window=never", "other"),
            &window()
        )
        .is_none()
    );
    assert_eq!(
        cached_machine.error.as_deref(),
        Some("no saved snapshot for this view")
    );
    // Only the snapshot file and its lock are left: every write was renamed
    // into place.
    let files: Vec<_> = std::fs::read_dir(logs.path().join(CACHE))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_none_or(|extension| extension != "lock"))
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
}

#[test]
fn the_view_query_is_fixed_whatever_order_the_flags_came_in() {
    let parse = |words: &[&str]| {
        let mut args = vec!["metrics".to_owned()];
        args.extend(words.iter().map(|word| word.to_string()));
        let crate::args::Command::Metrics(asked) = crate::args::parse(&args).unwrap() else {
            unreachable!()
        };
        View::of(&asked, &window()).query
    };
    assert_eq!(
        parse(&["--kind", "Run", "--window", "7d", "--repo", "acme/widgets"]),
        parse(&["--repo", "acme/widgets", "--window", "7d", "--kind", "Run"])
    );
    assert_eq!(
        parse(&["--window", "7d", "--kind", "Spec run"]),
        "window=7d&kind=Spec+run"
    );
    assert_eq!(parse(&[]), "");
}

#[test]
fn a_cached_snapshot_counts_only_failures_in_todays_window() {
    let current = window().current().unwrap();
    let mut old = summary("east", current, 0, 1);
    old.where_runs_failed.rows[0].commands[0].started_at = "2026-10-01T00:00:00Z".into();
    let fleet = merge(
        None,
        window(),
        Filters::of(&MetricsArgs::default()),
        vec![(machine("east", Status::Cached), Some(old))],
    );
    assert_eq!(fleet.where_runs_failed.failed, 0);
}

#[test]
fn another_windows_snapshot_stands_in_regrouped_by_week() {
    let logs = tempfile::tempdir().unwrap();
    let current = window().current().unwrap();
    let daily = View::of(
        &MetricsArgs {
            window: Some(MetricsWindow::Days7),
            ..MetricsArgs::default()
        },
        &window(),
    );
    save(
        logs.path(),
        "east",
        Slot::Read(None),
        &daily,
        &summary("east", current, 2, 1),
        Utc::now(),
    )
    .unwrap();
    let weekly = Window::of(&MetricsArgs::default(), today());
    let all = View::of(&MetricsArgs::default(), &weekly);
    let mut east = machine("east", Status::Cached);
    let read = read_saved(&path(logs.path(), "east", Slot::Read(None))).unwrap();
    let stood_in = cached(&mut east, &read, &all, &weekly).unwrap();
    assert!(stood_in.comparison.is_none());
    let buckets: Vec<_> = stood_in
        .outcomes
        .iter()
        .map(|row| (row.bucket.to_string(), row.tally.runs))
        .collect();
    // 2026-11-10 is in the week of Monday 2026-11-09; 2026-10-01 in 09-28's.
    assert_eq!(
        buckets,
        [("2026-09-28".to_owned(), 1), ("2026-11-09".to_owned(), 3)]
    );
    assert_eq!(
        east.error.as_deref(),
        Some("showing its snapshot of another window")
    );
    // A weekly snapshot cannot stand in for a daily view.
    save(
        logs.path(),
        "west",
        Slot::Read(None),
        &all,
        &summary("west", current, 1, 0),
        Utc::now(),
    )
    .unwrap();
    let mut west = machine("west", Status::Cached);
    let read = read_saved(&path(logs.path(), "west", Slot::Read(None))).unwrap();
    assert!(cached(&mut west, &read, &daily, &window()).is_none());
}

#[test]
fn a_reported_snapshot_stands_in_for_everyone_and_a_newer_read_for_its_reader() {
    let logs = tempfile::tempdir().unwrap();
    let current = window().current().unwrap();
    let view = View::of(
        &MetricsArgs {
            window: Some(MetricsWindow::Days7),
            ..MetricsArgs::default()
        },
        &window(),
    );
    let pushed = Utc::now();
    let reported = MetricsReport {
        version: 1,
        machine: "laptop".into(),
        view: view.query.clone(),
        summary: summary("laptop", current, 1, 0),
    };
    reported
        .receive(logs.path(), "laptop", "here", pushed)
        .unwrap();
    let read = pushed + chrono::Duration::seconds(5);
    save(
        logs.path(),
        "laptop",
        Slot::Read(Some("reader@example.test")),
        &view,
        &summary("laptop", current, 2, 0),
        read,
    )
    .unwrap();
    let mut config = UserConfig::parse("", &logs.path().join("config.toml"), logs.path()).unwrap();
    config.logs_dir = logs.path().to_owned();
    let asked = MetricsArgs {
        machine: Some("laptop".into()),
        window: Some(MetricsWindow::Days7),
        ..MetricsArgs::default()
    };
    let everyone = Capture::new(&config, Some("someone@example.test"))
        .load(&asked)
        .unwrap();
    assert_eq!(everyone.machines.len(), 1);
    assert_eq!(everyone.machines[0].status, Status::Reported);
    assert_eq!(everyone.machines[0].pushed_at, Some(pushed));
    let reader = Capture::new(&config, Some("reader@example.test"))
        .load(&asked)
        .unwrap();
    assert_eq!(reader.machines.len(), 1);
    assert_eq!(reader.machines[0].last_seen, Some(read));
    assert_eq!(reader.machines[0].status, Status::Cached);

    // A report naming another machine, or another kind of view, saves nothing.
    let other = MetricsReport {
        version: 1,
        machine: "elsewhere".into(),
        view: view.query.clone(),
        summary: summary("elsewhere", current, 1, 0),
    };
    let refused = other
        .receive(logs.path(), "laptop", "here", pushed)
        .unwrap_err();
    assert!(matches!(
        refused.downcast_ref::<Refusal>(),
        Some(Refusal::Mismatch)
    ));
    let fleet = MetricsReport {
        version: 1,
        machine: "laptop".into(),
        view: "fleet=true".into(),
        summary: summary("laptop", current, 1, 0),
    };
    assert!(matches!(
        fleet
            .receive(logs.path(), "laptop", "here", pushed)
            .unwrap_err()
            .downcast_ref::<Refusal>(),
        Some(Refusal::Invalid(_))
    ));
    assert!(read_saved(&path(logs.path(), "elsewhere", Slot::Reported)).is_none());
    // Nor does one under this Console's own name, which it never shows.
    let own = MetricsReport {
        version: 1,
        machine: "here".into(),
        view: view.query.clone(),
        summary: summary("here", current, 1, 0),
    };
    assert!(matches!(
        own.receive(logs.path(), "here", "here", pushed)
            .unwrap_err()
            .downcast_ref::<Refusal>(),
        Some(Refusal::Invalid(_))
    ));
    assert!(read_saved(&path(logs.path(), "here", Slot::Reported)).is_none());
}

#[test]
fn a_reported_failure_links_only_to_an_https_page() {
    let command = |url: &str| SummaryCommand {
        name: "widgets#1".into(),
        kind: "Ticket run".into(),
        account: None,
        started_at: String::new(),
        selector: String::new(),
        repository: Some("acme/widgets".into()),
        url: Some(url.into()),
    };
    assert_eq!(
        command("https://github.com/acme/widgets/pull/2").link(),
        Some(("https://github.com/acme/widgets/pull/2", "pull request"))
    );
    assert_eq!(
        command("https://github.com/acme/widgets/issues/1").link(),
        Some(("https://github.com/acme/widgets/issues/1", "issue"))
    );
    assert_eq!(command("javascript:alert(1)").link(), None);
}
