use super::*;
use crate::args::MetricsWindow;
use serde_json::json;

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

fn machine(name: &str, status: Status) -> Machine {
    Machine {
        name: name.into(),
        address: None,
        local: name == "here",
        status,
        last_seen: None,
        error: None,
    }
}

#[test]
fn fleet_totals_add_each_machine_once_and_keep_its_own_rows() {
    let current = window().current().unwrap();
    let fleet = merge(
        "fleet".into(),
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
        "fleet".into(),
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
        let query = format!("window={index}");
        save(
            logs.path(),
            "east/one",
            &query,
            &summary("east", current, index, 0),
            at,
        )
        .unwrap();
    }
    let saved = saved(logs.path());
    assert_eq!(saved.len(), 1);
    let saved = &saved[0];
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
    let read = read_saved(&path(logs.path(), "east/one")).unwrap();
    let summary = cached(&mut cached_machine, read, &format!("window={VIEWS}")).unwrap();
    assert_eq!(summary.outcomes[0].tally.succeeded, VIEWS);
    assert!(cached_machine.error.is_none());
    let read = read_saved(&path(logs.path(), "east/one")).unwrap();
    assert!(cached(&mut cached_machine, read, "window=never").is_none());
    assert_eq!(
        cached_machine.error.as_deref(),
        Some("no saved snapshot for this view")
    );
    // Only the snapshot file is left: every write was renamed into place.
    let files: Vec<_> = std::fs::read_dir(logs.path().join(CACHE))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
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
        query(&asked)
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
