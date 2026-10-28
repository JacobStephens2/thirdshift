use std::fs;
use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};

use super::Store;

fn payload() -> Value {
    json!({
        "week": "2026-W43",
        "window_start": "2026-10-18T00:00:00Z",
        "window_end": "2026-10-25T00:00:00Z",
        "seed": 42,
        "sample": [],
        "packets": "packets-original",
        "bug_origins": [],
        "initial_calibration": true
    })
}

#[test]
fn loaded_sample_commits_to_its_exact_file_without_reading_history_again() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_owned());
    let path = root.path().join("arbitrary-retained-name.json");
    let other = root.path().join("same-seed-and-window.json");
    let original = serde_json::to_vec(&payload()).unwrap();
    fs::write(&path, &original).unwrap();
    let mut other_payload = payload();
    other_payload["week"] = "other".into();
    fs::write(&other, serde_json::to_vec(&other_payload).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let mut retained = store.load().unwrap();
    // Both payloads have the same seed and window. Each handle must still
    // commit to its own file, regardless of enumeration order or payload edits.
    for (index, sample) in retained.iter_mut().enumerate() {
        sample.week.week = format!("edited-{}", sample.week.week);
        sample.week.seed = 100 + index as u64;
    }
    fs::write(root.path().join("unrelated.json"), b"malformed").unwrap();
    for sample in &retained {
        sample.commit().unwrap();
    }
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let other_saved: Value = serde_json::from_slice(&fs::read(&other).unwrap()).unwrap();
    assert_eq!(saved["week"], "edited-2026-W43");
    assert_eq!(other_saved["week"], "edited-other");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let mut expected = payload();
    expected["week"] = saved["week"].clone();
    expected["seed"] = saved["seed"].clone();
    assert_eq!(saved, expected, "unrelated payload fields must survive");
}

#[test]
fn same_second_publication_retains_distinct_samples_and_private_packets() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("scoring");
    let store = Store::new(directory.clone());
    for seed in [42, 43] {
        let staging = store.stage_packets().unwrap();
        let packet_directory = staging.path().to_owned();
        fs::create_dir(packet_directory.join("0")).unwrap();
        fs::write(packet_directory.join("0/packet.json"), b"complete").unwrap();
        let mut week = payload();
        week["seed"] = seed.into();
        let retained = staging
            .publish(serde_json::from_value(week).unwrap())
            .unwrap();
        assert_eq!(retained.packet(0).unwrap(), packet_directory.join("0"));
        assert_eq!(
            fs::read(retained.packet(0).unwrap().join("packet.json")).unwrap(),
            b"complete"
        );
        assert_eq!(
            fs::metadata(&packet_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    let samples = store.load().unwrap();
    assert_eq!(samples.len(), 2);
    let mut seeds: Vec<_> = samples.iter().map(|sample| sample.week.seed).collect();
    seeds.sort();
    assert_eq!(seeds, [42, 43]);
    assert_eq!(
        fs::metadata(directory.join("20261025T000000Z.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(directory.join("20261025T000000Z-43.json").is_file());
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn failed_no_clobber_publication_preserves_samples_and_cleans_staged_packets() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().to_owned());
    let original = serde_json::to_vec(&payload()).unwrap();
    for name in ["20261025T000000Z.json", "20261025T000000Z-42.json"] {
        fs::write(root.path().join(name), &original).unwrap();
    }
    let staging = store.stage_packets().unwrap();
    let staged_path = staging.path().to_owned();
    fs::write(staged_path.join("prepared-packet"), b"private").unwrap();
    assert!(
        staging
            .publish(serde_json::from_value(payload()).unwrap())
            .is_err()
    );
    assert!(
        !staged_path.exists(),
        "failed publication retained its packets"
    );
    for name in ["20261025T000000Z.json", "20261025T000000Z-42.json"] {
        assert_eq!(fs::read(root.path().join(name)).unwrap(), original);
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn retained_reader_keeps_legacy_defaults_ordering_and_contextual_errors() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("absent");
    let store = Store::new(directory.clone());
    assert!(store.load().unwrap().is_empty());
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("ignored.txt"), b"not JSON").unwrap();
    fs::create_dir(directory.join("packets-old")).unwrap();
    for (name, end) in [
        ("z.json", "2026-10-24T00:00:00Z"),
        ("a.json", "2026-10-25T00:00:00Z"),
        ("b.json", "2026-10-25T00:00:00Z"),
    ] {
        let mut value = payload();
        value["week"] = name.into();
        value["window_end"] = end.into();
        for optional in ["packets", "bug_origins", "initial_calibration"] {
            value.as_object_mut().unwrap().remove(optional);
        }
        fs::write(directory.join(name), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    // Equal-window samples keep enumeration order, just as the old reader did.
    let equal_windows: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name == "a.json" || name == "b.json")
        .collect();
    let samples = store.load().unwrap();
    assert_eq!(samples[0].week.week, "z.json");
    assert_eq!(
        samples[1..]
            .iter()
            .map(|sample| sample.week.week.clone())
            .collect::<Vec<_>>(),
        equal_windows
    );
    assert!(samples[0].week.bug_origins.is_none());
    assert!(!samples[0].week.initial_calibration);
    assert!(samples[0].packet(0).is_err());
    let malformed = directory.join("broken.json");
    fs::write(&malformed, b"not JSON").unwrap();
    let error = store.load().err().unwrap().to_string();
    assert!(error.contains("can't read Scoring sample"), "{error}");
    assert!(error.contains(malformed.to_str().unwrap()), "{error}");
}
