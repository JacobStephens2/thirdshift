use super::*;

const PR: &str = "https://github.com/acme/widgets/pull/23";

fn with_pr(number: u64, started_at: &str) -> Value {
    let mut value = record(number);
    value["pull_request"] = PR.into();
    value["started_at"] = started_at.into();
    value
}

#[test]
fn normalized_pull_requests_select_from_the_captured_commands() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "7.command.json",
        &with_pr(7, "2026-10-19T11:00:00Z"),
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    fs::remove_file(&path).unwrap();
    let attribution = PullRequestAttribution::new(&snapshot);
    let variant = "HTTPS://GITHUB.COM/ACME/WIDGETS/PULL/23///";
    let expected = CommandIdentity {
        record_path: path.clone(),
        name: "acme/widgets#7".into(),
    };
    assert_eq!(attribution.latest(variant), Some(&expected));
    assert_eq!(
        attribution.exact(variant, &path, &expected.name),
        Some(&expected)
    );
    assert_eq!(
        attribution.latest("https://github.com/acme/other/pull/23"),
        None
    );
    assert_eq!(
        run_for_pull_request(dir.path(), PR).unwrap(),
        None,
        "the filesystem adapter takes a fresh capture"
    );
}

#[test]
fn equal_instants_choose_greatest_lexical_path_then_command_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut earlier_path = with_pr(7, "2026-10-19T11:00:00Z");
    earlier_path["name"] = "zzzz".into();
    write(dir.path(), "a.command.json", &earlier_path);
    let mut pass = with_pr(8, "2026-10-19T12:00:00+01:00");
    pass["kind"] = "Pickup run".into();
    pass["name"] = "middle".into();
    let mut zulu = with_pr(20, "2026-10-19T11:00:00Z");
    zulu["name"] = "zulu".into();
    let mut alpha = with_pr(30, "2026-10-19T13:00:00+02:00");
    alpha["name"] = "alpha".into();
    pass["dispatched"] = json!([zulu, alpha]);
    let path = write(dir.path(), "z.command.json", &pass);
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let attribution = PullRequestAttribution::new(&snapshot);
    assert_eq!(
        attribution.latest(PR),
        Some(&CommandIdentity {
            record_path: path.clone(),
            name: "zulu".into()
        })
    );
    for name in ["middle", "alpha", "zulu"] {
        assert_eq!(attribution.exact(PR, &path, name).unwrap().name, name);
    }
}

#[test]
fn invalid_starts_precede_valid_times_and_keep_deterministic_ties() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "y.command.json", &with_pr(7, "zzz-invalid"));
    let invalid = write(dir.path(), "z.command.json", &with_pr(8, ""));
    let valid = write(
        dir.path(),
        "a.command.json",
        &with_pr(9, "2000-01-01T00:00:00Z"),
    );
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    assert_eq!(
        PullRequestAttribution::new(&snapshot)
            .latest(PR)
            .unwrap()
            .record_path,
        valid
    );
    fs::remove_file(valid).unwrap();
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    assert_eq!(
        PullRequestAttribution::new(&snapshot)
            .latest(PR)
            .unwrap()
            .record_path,
        invalid
    );
}

#[test]
fn exact_pins_require_both_lexical_fields_and_matching_pull_request() {
    let dir = tempfile::tempdir().unwrap();
    let pinned = write(dir.path(), "old.command.json", &with_pr(7, "invalid"));
    write(
        dir.path(),
        "new.command.json",
        &with_pr(7, "2026-10-19T11:00:00Z"),
    );
    let mut other = with_pr(8, "2026-10-19T12:00:00Z");
    other["pull_request"] = "https://github.com/acme/widgets/pull/24".into();
    let other = write(dir.path(), "other.command.json", &other);
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let attribution = PullRequestAttribution::new(&snapshot);
    assert_eq!(
        attribution
            .exact(PR, &pinned, "acme/widgets#7")
            .unwrap()
            .record_path,
        pinned
    );
    assert_eq!(attribution.exact(PR, &pinned, "acme/widgets#8"), None);
    assert_eq!(attribution.exact(PR, &other, "acme/widgets#8"), None);
    assert_eq!(
        attribution.exact(
            PR,
            &dir.path().join("missing.command.json"),
            "acme/widgets#7"
        ),
        None
    );
    // A lexically different spelling is not a saved identity, even for the same file.
    let alias = pinned.parent().unwrap().join("../issue/old.command.json");
    assert_eq!(attribution.exact(PR, &alias, "acme/widgets#7"), None);
}

#[test]
fn nested_ticket_and_base_fix_attribution_preserves_dispatch_membership() {
    let dir = tempfile::tempdir().unwrap();
    let mut pass = with_pr(1, "2026-10-19T10:00:00Z");
    pass["kind"] = "Security run".into();
    let mut spec = with_pr(20, "2026-10-19T11:00:00Z");
    spec["kind"] = "Spec run".into();
    pass["dispatched"] = json!([spec, with_pr(30, "2026-10-19T12:00:00Z")]);
    let outer = write(dir.path(), "outer.command.json", &pass);
    let mut ticket = with_pr(21, "2026-10-19T13:00:00Z");
    ticket["parent_record"] = json!(outer);
    ticket["parent_dispatch"] = "https://github.com/acme/widgets/issues/20".into();
    let child = write(dir.path(), "ticket.child.json", &ticket);
    let mut fix = with_pr(22, "2026-10-19T14:00:00Z");
    fix["kind"] = "Base fix".into();
    fix["outcome"] = "failed".into();
    fix["parent_record"] = json!(child);
    let fix = write(dir.path(), "fix.child.json", &fix);
    let mut orphan = with_pr(99, "2099-01-01T00:00:00Z");
    orphan["parent_record"] = json!(dir.path().join("missing.command.json"));
    let orphan_path = write(dir.path(), "orphan.child.json", &orphan);
    orphan["parent_record"] = json!(outer);
    orphan["parent_dispatch"] = "https://github.com/acme/widgets/issues/999".into();
    let unmatched = write(dir.path(), "unmatched.child.json", &orphan);
    let snapshot = Snapshot::load(dir.path(), now()).unwrap();
    let attribution = PullRequestAttribution::new(&snapshot);
    assert_eq!(attribution.latest(PR).unwrap().record_path, fix);
    for (path, name) in [
        (&outer, "acme/widgets#1"),
        (&outer, "acme/widgets#20"),
        (&outer, "acme/widgets#30"),
        (&child, "acme/widgets#21"),
        (&fix, "acme/widgets#22"),
    ] {
        assert_eq!(attribution.exact(PR, path, name).unwrap().name, name);
    }
    assert_eq!(attribution.exact(PR, &orphan_path, "acme/widgets#99"), None);
    assert_eq!(attribution.exact(PR, &unmatched, "acme/widgets#99"), None);
    assert_eq!(
        run_for_pull_request(dir.path(), PR).unwrap(),
        Some((fix, "acme/widgets#22".into()))
    );
}
