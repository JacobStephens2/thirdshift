use super::*;

#[test]
fn security_fix_peek_links_advisories_and_marks_the_parent_dispatch_for_every_ending() {
    for outcome in [
        Some("ready for review"),
        Some("failed"),
        Some("interrupted"),
        None,
    ] {
        let home = tempfile::tempdir().unwrap();
        configure(home.path());
        let mut parent = record(home.path(), 7, outcome, 0);
        let mut fix = parent.clone();
        fix["kind"] = json!("Run");
        fix["name"] = json!("acme/widgets#7");
        let findings = json!([
            {"title": "Bound <input>", "severity": "medium",
                "url": "https://github.com/acme/widgets/security/advisories/GHSA-first"},
            {"title": "Bound headers", "severity": null,
                "url": "https://github.com/acme/widgets/security/advisories/GHSA-second"}
        ]);
        fix["security_fix_findings"] = findings.clone();
        parent["kind"] = json!("Security run");
        parent["name"] = json!("secure-fixture");
        parent["issue"] = Value::Null;
        parent["security_findings"] = json!([
            findings[0], findings[1],
            {"title": "Audit only", "severity": "low",
                "url": "https://github.com/acme/widgets/security/advisories/GHSA-other"}
        ]);
        parent["dispatched"] = json!([fix]);
        write_record(home.path(), 7, &parent);
        let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
        let console = Console::start(binary, home.path());
        for (command, label) in [
            ("acme/widgets%237", "Fixes advisory"),
            ("secure-fixture", "Fix dispatched for advisory"),
        ] {
            let response = console.get(&format!("/peek?command={command}"));
            assert_eq!(response.status(), 200);
            let html = response.text().unwrap();
            for (id, title, severity) in [
                ("first", "Bound &#60;input&#62;", "medium"),
                ("second", "Bound headers", "ungraded"),
            ] {
                let line = format!(
                    "<p>{label}: <a href=\"https://github.com/acme/widgets/security/advisories/GHSA-{id}\">{title}</a> · {severity}</p>"
                );
                assert!(html.contains(&line), "outcome={outcome:?}: {html}");
            }
            assert!(!html.contains(">Audit only</a>"), "{html}");
        }
        let shown = Command::new(binary)
            .args(["show", "acme/widgets#7", "--json"])
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(shown.status.success());
        let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
        assert_eq!(shown["command"]["security_fix_findings"], findings);
    }
}

#[test]
fn ordinary_and_legacy_records_show_no_advisory_line_or_backfill() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 7, Some("ready for review"), 0);
    let mut legacy = record(home.path(), 8, Some("failed"), 0);
    legacy["kind"] = json!("Security run");
    legacy["name"] = json!("secure-legacy");
    legacy["issue"] = Value::Null;
    legacy["security_findings"] = json!([{
        "title": "Legacy advisory", "severity": "high",
        "url": "https://github.com/acme/widgets/security/advisories/GHSA-legacy"
    }]);
    legacy["dispatched"] = json!([record(home.path(), 9, Some("failed"), 0)]);
    fs::remove_file(
        home.path()
            .join(".thirdshift/logs/acme/widgets/commands/issue/9-fixture.command.json"),
    )
    .unwrap();
    write_record(home.path(), 8, &legacy);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    for command in ["acme/widgets%237", "secure-legacy", "acme/widgets%239"] {
        let response = console.get(&format!("/peek?command={command}"));
        assert_eq!(response.status(), 200);
        let html = response.text().unwrap();
        assert!(!html.contains("Fixes advisory:"), "{html}");
        assert!(!html.contains("Fix dispatched for advisory:"), "{html}");
    }
    let shown = Command::new(binary)
        .args(["show", "acme/widgets#7", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(shown.status.success());
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert!(shown["command"].get("security_fix_findings").is_none());
}
