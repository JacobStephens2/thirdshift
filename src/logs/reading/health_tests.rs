use super::*;
use std::time::Duration;

fn history(root: &Path, stage: &str, seconds: [i64; 3]) {
    for (number, seconds) in (1..=3).zip(seconds) {
        let mut past = record(number);
        let ended = now() - chrono::Duration::hours(2);
        let began = ended - chrono::Duration::seconds(seconds);
        past["started_at"] = began.to_rfc3339().into();
        past["ended_at"] = ended.to_rfc3339().into();
        past["outcome"] = "merged".into();
        past["stages"] =
            json!([{"name": stage, "began_at": began.to_rfc3339(), "session_log": null}]);
        write(root, &format!("{number}-past.command.json"), &past);
    }
}

#[test]
fn slow_boundaries_keep_minor_overruns_in_working_or_landing() {
    for stage in ["implement", "CI watch", "Self-merge"] {
        for (samples, lower, upper, threshold, suspended, quota_wait) in [
            ([2, 4, 20], 3, 12, 312, 0, false),
            ([2, 4, 20], 3, 12, 312, 1800, false),
            ([2, 4, 20], 3, 12, 312, 1800, true),
            ([2, 4, 20], 3, 12, 312, 3600, false),
            ([600, 840, 1200], 720, 1020, 2040, 0, false),
            ([600, 840, 1200], 720, 1020, 2040, 1800, false),
            ([600, 840, 1200], 720, 1020, 2040, 1800, true),
            ([600, 840, 1200], 720, 1020, 2040, 3600, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            history(dir.path(), stage, samples);
            let began = now() - chrono::Duration::hours(1);
            let mut live = record(7);
            live["started_at"] = began.to_rfc3339().into();
            live["stages"] = json!([{
                "name": stage,
                "began_at": began.to_rfc3339(),
                "session_log": null,
                "suspended_seconds": suspended,
            }]);
            if quota_wait {
                live["quota_wait_until"] = "2026-10-19T12:01:37-04:00".into();
            }
            let path = write(dir.path(), "7-live.command.json", &live);
            fs::write(path.with_file_name("7-live.log"), "still working\n").unwrap();
            let original = fs::read(&path).unwrap();
            let mut ages = vec![upper, upper + 1, threshold - 1, threshold];
            if upper == 12 {
                ages.push(60);
            }
            for age in ages {
                let at = began + chrono::Duration::seconds(age + suspended);
                let snapshot =
                    Snapshot::load_with_silence(dir.path(), at, Duration::from_secs(3600)).unwrap();
                let selected = snapshot.resolve("widgets#7").unwrap();
                let Observation::Record(command) = selected.observation else {
                    panic!("record-backed Command")
                };
                let value = serde_json::to_value(command).unwrap();
                assert_eq!(
                    value["usual_ranges"][stage],
                    json!({"lower_seconds": lower as f64, "upper_seconds": upper as f64})
                );
                if age == upper {
                    assert_eq!(value["health"]["state"], "ok");
                } else {
                    assert_eq!(value["health"]["state"], "slow");
                    assert_eq!(
                        value["health"]["step"],
                        if age == threshold { 2 } else { 1 }
                    );
                }
                if age < threshold {
                    assert_eq!(
                        command.next_health_change,
                        Some(
                            began
                                + chrono::Duration::seconds(
                                    suspended + if age == upper { upper + 1 } else { threshold },
                                )
                        )
                    );
                }
                let rows = snapshot
                    .evaluate_inbox(7, &crate::logs::github::Kept::default())
                    .into_rows(true);
                let rows = serde_json::to_value(rows).unwrap();
                let row = rows
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["issue"]["number"] == 7)
                    .unwrap();
                assert_eq!(
                    row["group"],
                    if age == threshold {
                        "needs you"
                    } else if stage == "implement" || quota_wait {
                        "working"
                    } else {
                        "landing"
                    },
                    "stage {stage}, age {age}: {row}"
                );
                assert_eq!(
                    row["needs_you_kind"],
                    if age == threshold {
                        json!("slow")
                    } else {
                        Value::Null
                    }
                );
            }
            assert_eq!(fs::read(path).unwrap(), original);
        }
    }
}

#[test]
fn parents_enter_needs_you_only_for_a_step_two_slow_descendant() {
    for tree in ["Spec run", "Pass with Spec run", "Pass with Ticket run"] {
        for (age, expected) in [(20, "working"), (34, "needs you")] {
            let dir = tempfile::tempdir().unwrap();
            history(dir.path(), "implement", [600, 840, 1200]);
            let mut parent = record(20);
            parent["kind"] = if tree == "Spec run" {
                "Spec run"
            } else {
                "Pickup run"
            }
            .into();
            let path = write(dir.path(), "20-parent.command.json", &parent);
            let mut ticket = record(21);
            let began = now() - chrono::Duration::minutes(age);
            ticket["started_at"] = began.to_rfc3339().into();
            ticket["stages"] =
                json!([{"name": "implement", "began_at": began.to_rfc3339(), "session_log": null}]);
            if tree == "Pass with Ticket run" {
                parent["dispatched"] = json!([ticket]);
            } else {
                ticket["parent_record"] = json!(path);
                if tree == "Pass with Spec run" {
                    let mut spec = record(30);
                    spec["kind"] = "Spec run".into();
                    ticket["parent_dispatch"] = spec["issue"]["url"].clone();
                    parent["dispatched"] = json!([spec]);
                }
                let child_path =
                    write(dir.path(), "20-parent.command.issue-21.child.json", &ticket);
                fs::write(
                    child_path.with_file_name("20-parent.command.issue-21.child.log"),
                    "still working\n",
                )
                .unwrap();
            }
            fs::write(&path, serde_json::to_vec(&parent).unwrap()).unwrap();
            fs::write(path.with_file_name("20-parent.log"), "still working\n").unwrap();
            let snapshot = Snapshot::load(dir.path(), now()).unwrap();
            let rows = snapshot
                .evaluate_inbox(7, &crate::logs::github::Kept::default())
                .into_rows(true);
            let rows = serde_json::to_value(rows).unwrap();
            let row = rows
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["issue"]["number"] == 20)
                .unwrap();
            assert_eq!(row["group"], expected, "{tree}, age {age}: {row}");
            assert_eq!(
                row["needs_you_kind"],
                if age == 34 {
                    json!("slow")
                } else {
                    Value::Null
                }
            );
        }
    }
}
