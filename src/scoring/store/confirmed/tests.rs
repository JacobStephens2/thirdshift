use std::fs;

use serde_json::{Value, json};

use super::{ConfirmedPullRequest, Store};

fn save(root: &std::path::Path, name: &str, value: Value) {
    fs::write(root.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn confirmed_origins_accept_partial_records_without_full_sample_metadata() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_owned());
    save(
        root.path(),
        "minimal.json",
        json!({
            "window_end": "2026-10-25T00:00:00Z",
            "bug_origins": [{
                "issue": "https://github.com/acme/widgets/issues/99",
                "state": "confirmed",
                "settlement": {"pull_request": {"url": "https://github.com/acme/widgets/pull/23"}}
            }]
        }),
    );
    assert!(store.load().is_err(), "full samples still require metadata");
    assert_eq!(
        store.confirmed_origins().unwrap(),
        vec![ConfirmedPullRequest {
            url: "https://github.com/acme/widgets/pull/23".into(),
            local_run: None,
        }]
    );
}

fn origin(issue: &str, state: &str, url: &str) -> Value {
    json!({"issue": issue, "state": state, "settlement": {"pull_request": {"url": url}}})
}

#[test]
fn latest_normalized_bug_wins_before_confirmation_filtering() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_owned());
    let pr = "https://github.com/acme/widgets/pull/23";
    let corrected = "https://github.com/acme/widgets/pull/24";
    let issue = "https://github.com/acme/widgets/issues/99";
    save(
        root.path(),
        "z-old.json",
        json!({
            "window_end": "2026-10-25T00:00:00Z",
            "bug_origins": [origin(issue, "confirmed", pr),
                origin("https://github.com/acme/widgets/issues/100", "confirmed", pr),
                origin("https://github.com/acme/widgets/issues/101", "confirmed", pr)]
        }),
    );
    for state in ["proposed", "rejected", "future_state", "confirmed"] {
        save(
            root.path(),
            "a-new.json",
            json!({
                "window_end": "2026-11-01T00:00:00Z",
                "bug_origins": [origin(issue, "confirmed", pr),
                    origin("https://github.com/ACME/WIDGETS/issues/99///", state, corrected)]
            }),
        );
        let expected = if state == "confirmed" {
            vec![pr, pr, corrected]
        } else {
            vec![pr, pr]
        };
        assert_eq!(
            store
                .confirmed_origins()
                .unwrap()
                .iter()
                .map(|pr| pr.url.as_str())
                .collect::<Vec<_>>(),
            expected,
            "{state}"
        );
    }
}

#[test]
fn missing_storage_and_optional_origins_are_read_only_empty_results() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("absent");
    assert!(
        Store::new(missing.clone())
            .confirmed_origins()
            .unwrap()
            .is_empty()
    );
    assert!(!missing.exists());

    let store = Store::new(root.path().to_owned());
    fs::write(root.path().join("ignored.txt"), b"not JSON").unwrap();
    fs::write(root.path().join("ignored.JSON"), b"not JSON").unwrap();
    fs::create_dir(root.path().join("packets-unread")).unwrap();
    fs::write(root.path().join("packets-unread/packet.json"), b"not JSON").unwrap();
    let records = [
        json!({"window_end": "2026-10-25T00:00:00Z"}),
        json!({"window_end": "2026-10-25T00:00:00Z", "bug_origins": null}),
        json!({"window_end": "2026-10-25T00:00:00Z", "bug_origins": [
            {"issue": "missing-settlement", "state": "confirmed"},
            {"issue": "null-settlement", "state": "confirmed", "settlement": null},
            origin("case-sensitive-state", "Confirmed", "unused")
        ]}),
    ];
    for (index, record) in records.iter().enumerate() {
        save(root.path(), &format!("{index}.json"), record.clone());
    }
    assert!(store.confirmed_origins().unwrap().is_empty());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 6);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(
            fs::read(root.path().join(format!("{index}.json"))).unwrap(),
            serde_json::to_vec(record).unwrap()
        );
    }
}

#[test]
fn attribution_projection_preserves_lexical_local_pairs_and_nonlocal_fallbacks() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_owned());
    let url = "https://github.com/Acme/Widgets/pull/23///";
    for (run, local_run) in [
        (
            json!({"source": "local", "command_record": "relative/../retained.command.json", "name": "acme/widgets#7"}),
            Some((
                "relative/../retained.command.json".into(),
                "acme/widgets#7".into(),
            )),
        ),
        (Value::Null, None),
        (json!({"source": "built_with", "harness": "codex"}), None),
        (
            json!({"source": "future_source", "command_record": false, "name": null}),
            None,
        ),
    ] {
        let mut confirmed = origin("bug", "confirmed", url);
        confirmed["settlement"]["pull_request"]["run"] = run;
        save(
            root.path(),
            "week.json",
            json!({
                "window_end": "2026-10-25T00:00:00Z",
                "sample": {"unrelated": "invalid full sample"},
                "bug_origins": [confirmed]
            }),
        );
        assert_eq!(
            store.confirmed_origins().unwrap(),
            vec![ConfirmedPullRequest {
                url: url.into(),
                local_run
            }]
        );
    }
}

#[test]
fn equal_window_origins_keep_checkpoint_enumeration_order() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_owned());
    for (file, url) in [("a.json", "first-pr"), ("b.json", "second-pr")] {
        save(
            root.path(),
            file,
            json!({
                "window_end": "2026-10-25T00:00:00Z", "bug_origins": [origin("bug", "confirmed", url)]
            }),
        );
    }
    let last_file = fs::read_dir(root.path())
        .unwrap()
        .last()
        .unwrap()
        .unwrap()
        .file_name();
    let expected = if last_file == "a.json" {
        "first-pr"
    } else {
        "second-pr"
    };
    assert_eq!(
        store.confirmed_origins().unwrap(),
        vec![ConfirmedPullRequest {
            url: expected.into(),
            local_run: None
        }]
    );
}

#[test]
fn malformed_required_projection_fields_name_the_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_owned());
    let path = root.path().join("broken.json");
    let valid = json!({"window_end": "2026-10-25T00:00:00Z", "bug_origins": [origin("bug", "confirmed", "pr")]});
    let mut malformed = vec![
        json!({}),
        json!({"window_end": null}),
        json!({"window_end": "not a date"}),
    ];
    for origin in [
        json!({"state": "confirmed"}),
        json!({"issue": "bug"}),
        json!({"issue": "bug", "state": 1}),
        json!({"issue": "bug", "state": "proposed", "settlement": {}}),
        json!({"issue": "bug", "state": "confirmed", "settlement": {"pull_request": {}}}),
    ] {
        let mut value = valid.clone();
        value["bug_origins"] = json!([origin]);
        malformed.push(value);
    }
    for run in [
        json!({"source": "local", "name": "run"}),
        json!({"source": "local", "command_record": "record.json"}),
        json!({"source": "local", "command_record": null, "name": "run"}),
        json!({"source": "local", "command_record": "record.json", "name": false}),
        json!({"name": "missing source"}),
    ] {
        let mut value = valid.clone();
        value["bug_origins"][0]["settlement"]["pull_request"]["run"] = run;
        malformed.push(value);
    }
    let mut invalid_origins = valid;
    invalid_origins["bug_origins"] = json!({});
    malformed.push(invalid_origins);
    for value in malformed {
        save(root.path(), "broken.json", value.clone());
        let error = store.confirmed_origins().unwrap_err().to_string();
        assert!(
            error.contains("can't read Scoring sample"),
            "{value}: {error}"
        );
        assert!(error.contains(path.to_str().unwrap()), "{value}: {error}");
    }
    fs::write(&path, b"{").unwrap();
    let error = store.confirmed_origins().unwrap_err().to_string();
    assert!(error.contains(path.to_str().unwrap()), "{error}");
}

#[test]
fn storage_read_errors_are_visible_instead_of_empty_confirmations() {
    let root = tempfile::tempdir().unwrap();
    let not_directory = root.path().join("not-a-directory");
    fs::write(&not_directory, b"file").unwrap();
    let error = Store::new(not_directory)
        .confirmed_origins()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("could not read Scoring checkpoints"),
        "{error}"
    );
    let unreadable = root.path().join("unreadable.json");
    fs::create_dir(&unreadable).unwrap();
    let error = Store::new(root.path().to_owned())
        .confirmed_origins()
        .unwrap_err()
        .to_string();
    assert!(error.contains("can't read Scoring sample"), "{error}");
    assert!(error.contains(unreadable.to_str().unwrap()), "{error}");
}
