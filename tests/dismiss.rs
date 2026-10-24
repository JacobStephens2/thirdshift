mod support;

use std::fs::{self, File, FileTimes};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use support::{HeldRun, Scenario};

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";
const ACTIVITY: &str = "home/.thirdshift/logs/acme/widgets/activity.log";

fn record_path(scenario: &Scenario, number: u32) -> PathBuf {
    scenario
        .path(LOGS)
        .join(format!("{number}-fixture.command.json"))
}

fn write_record(scenario: &Scenario, number: u32, record: &Value) {
    fs::create_dir_all(scenario.path(LOGS)).unwrap();
    fs::write(
        record_path(scenario, number),
        serde_json::to_vec(record).unwrap(),
    )
    .unwrap();
}

fn record_value(scenario: &Scenario, number: u32, started: &str, ended: Option<&str>) -> Value {
    json!({
        "version": 1, "kind": "Run", "repository": "acme/widgets",
        "name": format!("acme/widgets#{number}"),
        "issue": {"number": number, "url": scenario.issue_url(number), "title": "Fixture"},
        "hostname": "factory", "process_id": 0, "process_start": "gone",
        "harness": "claude", "model": null, "effort": null,
        "started_at": started, "ended_at": ended, "stages": [], "session_log": null,
        "outcome": ended.map(|_| "failed"), "cause": null,
        "issue_branch": null, "pull_request": null
    })
}

fn write_failed(scenario: &Scenario, number: u32, days_ago: i64, cause: Option<&str>) {
    let time = (chrono::Local::now() - chrono::Duration::days(days_ago)).to_rfc3339();
    let mut record = record_value(scenario, number, &time, Some(&time));
    record["cause"] = cause.into();
    write_record(scenario, number, &record);
}

fn write_ready(scenario: &Scenario, number: u32) {
    let now = chrono::Local::now().to_rfc3339();
    let mut record = record_value(scenario, number, &now, Some(&now));
    record["outcome"] = "ready for review".into();
    record["pull_request"] = format!("https://github.com/acme/widgets/pull/{number}").into();
    write_record(scenario, number, &record);
}

fn read_status(scenario: &Scenario) -> Value {
    let result = scenario.run(&["status", "--json"]);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    serde_json::from_str(&result.stdout).unwrap()
}

fn groups(status: &Value) -> Vec<(u64, String)> {
    status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| {
            (
                command["issue"]["number"].as_u64().unwrap(),
                command["group"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn activity(scenario: &Scenario) -> String {
    fs::read_to_string(scenario.path(ACTIVITY)).unwrap()
}

#[test]
fn dismiss_failed_row_moves_it_out_saying_who_and_leaves_the_command_alone() {
    let scenario = Scenario::new();
    write_failed(&scenario, 7, 0, None);
    assert_eq!(
        groups(&read_status(&scenario)),
        vec![(7, "needs you".into())]
    );
    let record = fs::read(record_path(&scenario, 7)).unwrap();
    let labels = scenario.gh_state()["labels"].clone();
    let calls = scenario.gh_calls().len();

    let dismissed = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    assert!(
        dismissed
            .stdout
            .contains("dismissed acme/widgets#7 from Needs you"),
        "{}",
        dismissed.stdout
    );

    let status = read_status(&scenario);
    assert_eq!(groups(&status), vec![(7, "done".into())]);
    let dismissal = &status["commands"][0]["dismissal"];
    assert!(!dismissal["who"].as_str().unwrap().is_empty(), "{status}");
    assert_eq!(dismissal["source"], "CLI");
    assert_eq!(dismissal["kind"], "Failed");
    assert!(
        !dismissal["dismissed_at"].as_str().unwrap().is_empty(),
        "{status}"
    );
    let text = scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(
        text.stdout.contains(&format!(
            "dismissed by {} from the CLI at",
            dismissal["who"].as_str().unwrap()
        )),
        "{}",
        text.stdout
    );
    assert!(
        activity(&scenario).contains(&format!(
            "Run #7 dismissed from the CLI by {}",
            dismissal["who"].as_str().unwrap()
        )),
        "{}",
        activity(&scenario)
    );
    assert_eq!(fs::read(record_path(&scenario, 7)).unwrap(), record);
    assert_eq!(scenario.gh_state()["labels"], labels);
    assert_eq!(scenario.gh_calls().len(), calls);
}

#[test]
fn dismiss_ready_row_leaves_ready_for_review() {
    let scenario = Scenario::new();
    write_ready(&scenario, 8);
    assert_eq!(
        groups(&read_status(&scenario)),
        vec![(8, "ready for review".into())]
    );

    let dismissed = scenario.run(&["dismiss", "widgets#8", "--json"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    let output: Value = serde_json::from_str(&dismissed.stdout).unwrap();
    assert_eq!(output["version"], 1);
    assert_eq!(output["name"], "acme/widgets#8");
    assert_eq!(output["group"], "ready for review");
    assert_eq!(output["dismissal"]["source"], "CLI");
    assert_eq!(output["dismissal"]["kind"], "Ready");

    let status = read_status(&scenario);
    assert_eq!(groups(&status), vec![(8, "done".into())]);
    assert_eq!(status["commands"][0]["dismissal"]["kind"], "Ready");
}

#[test]
fn dismiss_old_row_holds_it_in_resolved_after_the_dismissal() {
    let scenario = Scenario::new();
    write_failed(&scenario, 7, 6, None);
    assert_eq!(
        groups(&read_status(&scenario)),
        vec![(7, "needs you".into())]
    );

    let dismissed = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);

    let status = read_status(&scenario);
    assert_eq!(groups(&status), vec![(7, "resolved".into())]);
    assert_eq!(status["commands"][0]["dismissal"]["kind"], "Failed");
}

#[test]
fn dismiss_names_a_command_as_stop_and_show_do() {
    for form in ["url", "stamp", "path"] {
        let scenario = Scenario::new();
        write_failed(&scenario, 7, 0, None);
        let input = match form {
            "url" => scenario.issue_url(7),
            "stamp" => "widgets#7".to_owned(),
            _ => record_path(&scenario, 7).to_str().unwrap().to_owned(),
        };
        let dismissed = scenario.run(&["dismiss", &input]);
        assert_eq!(dismissed.code, Some(0), "{form}: {}", dismissed.stderr);
        assert_eq!(groups(&read_status(&scenario)), vec![(7, "done".into())]);
    }
    let scenario = Scenario::new();
    let now = chrono::Local::now().to_rfc3339();
    write_record(
        &scenario,
        7,
        &json!({
            "version": 1, "kind": "Pickup run", "repository": "acme/widgets",
            "name": "pickup-20261018T120000-0400", "issue": null,
            "hostname": "factory", "process_id": 0, "process_start": "gone",
            "harness": "claude", "model": null, "effort": null,
            "started_at": now, "ended_at": now, "stages": [], "session_log": null,
            "outcome": "failed", "cause": null,
            "issue_branch": null, "pull_request": null
        }),
    );
    let dismissed = scenario.run(&["dismiss", "pickup-20261018T120000-0400"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    let status = read_status(&scenario);
    assert_eq!(status["commands"][0]["group"], "done");
}

#[test]
fn dismiss_refuses_rows_outside_needs_you_and_ready_for_review() {
    let scenario = Scenario::new();
    scenario.issue_labelled(7, &["ready-for-agent"]);
    scenario.agent_does(&format!(
        "touch '{}'\nwhile true; do sleep 0.05; done\n",
        scenario.path("started").display()
    ));
    let mut held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
    let refused = scenario.run(&["dismiss", "widgets#7"]);
    assert_ne!(refused.code, Some(0));
    assert!(
        refused.stderr.contains("not Needs you or Ready for review"),
        "{}",
        refused.stderr
    );
    held.signal("TERM");
    held.finish();
}

#[test]
fn dismiss_refuses_a_row_without_a_command_record_and_names_it() {
    let scenario = Scenario::new();
    fs::create_dir_all(scenario.path(LOGS)).unwrap();
    let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S%z").to_string();
    let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let log = format!("commands/issue/7-{stamp}.log");
    fs::write(
        scenario.path(ACTIVITY),
        format!("{time} Run #7 started: {log}, on claude\n{time} Run #7 ended: failed: fixture\n"),
    )
    .unwrap();
    let status = read_status(&scenario);
    assert_eq!(status["commands"][0]["source"], "activity_log");
    assert_eq!(status["commands"][0]["group"], "needs you");

    let calls = scenario.gh_calls().len();
    let refused = scenario.run(&["dismiss", "widgets#7"]);
    assert_ne!(refused.code, Some(0));
    assert!(
        refused
            .stderr
            .contains("cannot dismiss widgets#7: row has no Command record"),
        "{}",
        refused.stderr
    );
    assert_eq!(scenario.gh_calls().len(), calls);
}

#[test]
fn dismiss_collapsed_row_covers_every_command_it_stands_for() {
    let scenario = Scenario::new();
    write_failed(&scenario, 7, 0, Some("boom"));
    write_failed(&scenario, 8, 0, Some("boom"));
    let status = read_status(&scenario);
    assert_eq!(status["commands"].as_array().unwrap().len(), 1, "{status}");
    assert_eq!(status["commands"][0]["count"], 2);
    assert_eq!(status["commands"][0]["group"], "needs you");

    let dismissed = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    let status = read_status(&scenario);
    assert!(
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|command| command["group"] != "needs you"),
        "{status}"
    );

    write_failed(&scenario, 9, 0, Some("boom"));
    let status = read_status(&scenario);
    assert_eq!(status["commands"].as_array().unwrap().len(), 2, "{status}");
    let waiting: Vec<_> = groups(&status)
        .into_iter()
        .filter(|(_, group)| group == "needs you")
        .collect();
    assert_eq!(waiting.len(), 1, "{status}");
    let row = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|command| command["group"] == "needs you")
        .unwrap();
    assert_eq!(row["count"], 1);
    assert!(row["dismissal"].is_null(), "{row}");
}

#[test]
fn undo_takes_the_dismissal_back_wherever_the_row_is() {
    let scenario = Scenario::new();
    write_failed(&scenario, 7, 0, None);
    let dismissed = scenario.run(&["dismiss", "widgets#7", "--json"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    assert_eq!(groups(&read_status(&scenario)), vec![(7, "done".into())]);

    let undone = scenario.run(&["dismiss", "widgets#7", "--undo", "--json"]);
    assert_eq!(undone.code, Some(0), "{}", undone.stderr);
    assert_eq!(
        serde_json::from_str::<Value>(&undone.stdout).unwrap(),
        json!({"version": 1, "name": "acme/widgets#7", "undone": true})
    );
    let status = read_status(&scenario);
    assert_eq!(groups(&status), vec![(7, "needs you".into())]);
    assert!(status["commands"][0]["dismissal"].is_null(), "{status}");
    assert!(
        activity(&scenario).contains("dismissal taken back from the CLI by"),
        "{}",
        activity(&scenario)
    );

    let again = scenario.run(&["dismiss", "widgets#7", "--undo"]);
    assert_ne!(again.code, Some(0));
    assert!(again.stderr.contains("not dismissed"), "{}", again.stderr);
}

struct SilentRun {
    scenario: Scenario,
    held: Option<HeldRun>,
    path: PathBuf,
    record: Value,
}

impl SilentRun {
    fn new() -> Self {
        let scenario = Scenario::new();
        scenario.agent_does(&format!(
            "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\nexit 1\n",
            scenario.path("started").display(),
            scenario.path("release").display()
        ));
        let held = scenario.run_until(&[&scenario.issue_url(7)], &[], "started");
        let path = fs::read_dir(scenario.path(LOGS))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().ends_with(".command.json"))
            .unwrap();
        let record = scenario.command_record(LOGS);
        Self {
            scenario,
            held: Some(held),
            path,
            record,
        }
    }

    fn go_silent(&mut self, stage: &str) {
        let began = (chrono::Local::now() - chrono::Duration::minutes(60)).to_rfc3339();
        self.record["stages"] = json!([{
            "name": stage, "began_at": began, "session_log": null
        }]);
        fs::write(&self.path, serde_json::to_vec(&self.record).unwrap()).unwrap();
        self.quiet_logs_for(20);
    }

    fn quiet_logs_for(&self, minutes: u64) {
        let log = self.path.with_file_name(
            self.path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .replace(".command.json", ".log"),
        );
        for path in [
            log,
            PathBuf::from(self.record["session_log"].as_str().unwrap()),
        ] {
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(
                    FileTimes::new()
                        .set_modified(SystemTime::now() - Duration::from_secs(minutes * 60)),
                )
                .unwrap();
        }
    }

    fn usual_implement_times(&self) {
        for number in 20..23 {
            let ended = chrono::Local::now() - chrono::Duration::hours(2);
            let began = ended - chrono::Duration::minutes(10);
            let mut record = record_value(
                &self.scenario,
                number,
                &began.to_rfc3339(),
                Some(&ended.to_rfc3339()),
            );
            record["outcome"] = "merged".into();
            record["stages"] = json!([{
                "name": "implement", "began_at": began.to_rfc3339(), "session_log": null
            }]);
            write_record(&self.scenario, number, &record);
        }
    }

    fn row(&self) -> Value {
        read_status(&self.scenario)["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["issue"]["number"] == 7)
            .unwrap()
            .clone()
    }
}

impl Drop for SilentRun {
    fn drop(&mut self) {
        fs::write(self.scenario.path("release"), "").unwrap();
        self.held.take().unwrap().finish();
    }
}

#[test]
fn dismiss_returning_health_kind_accumulates_coverage_for_the_whole_row() {
    for first in ["Slow", "Silent"] {
        let mut run = SilentRun::new();
        run.go_silent("implement");
        run.usual_implement_times();
        let second = if first == "Slow" { "Silent" } else { "Slow" };
        for (kind, covered) in [(first, vec![first]), (second, vec![first, second])] {
            run.quiet_logs_for(if kind == "Silent" { 20 } else { 0 });
            let row = run.row();
            assert_eq!(row["group"], "needs you", "{row}");
            assert_eq!(row["needs_you_kind"], kind.to_lowercase());
            let dismissed = run.scenario.run(&["dismiss", "widgets#7", "--json"]);
            assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
            let row = run.row();
            assert_eq!(row["group"], "working", "{row}");
            assert_eq!(row["dismissal"]["kinds"], json!(covered), "{row}");
        }
        // Both kinds have now counted once, even after another transition.
        run.quiet_logs_for(if first == "Silent" { 20 } else { 0 });
        let row = run.row();
        assert_eq!(row["group"], "working", "{row}");
        assert_eq!(row["dismissal"]["kind"], first);
        let undone = run.scenario.run(&["dismiss", "widgets#7", "--undo"]);
        assert_eq!(undone.code, Some(0), "{}", undone.stderr);
        assert_eq!(run.row()["group"], "needs you");
    }
}

#[test]
fn dismissed_security_failure_returns_only_for_findings_still_waiting() {
    for waiting in [true, false] {
        let scenario = Scenario::new();
        let now = chrono::Local::now().to_rfc3339();
        let mut record = record_value(&scenario, 7, &now, Some(&now));
        record["kind"] = "Security run".into();
        record["issue"] = Value::Null;
        record["name"] = "security-20261023T120000-0400".into();
        record["security_findings_waiting"] = waiting.into();
        write_record(&scenario, 7, &record);
        let path = record_path(&scenario, 7);
        let name = path.to_str().unwrap();
        assert_eq!(
            read_status(&scenario)["commands"][0]["needs_you_kind"],
            "failed"
        );
        let dismissed = scenario.run(&["dismiss", name]);
        assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
        let status = read_status(&scenario);
        let row = &status["commands"][0];
        if waiting {
            assert_eq!(row["group"], "needs you", "{row}");
            assert_eq!(row["needs_you_kind"], "finding");
            let dismissed = scenario.run(&["dismiss", name]);
            assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
            let status = read_status(&scenario);
            let row = &status["commands"][0];
            assert_eq!(row["group"], "done", "{row}");
            assert_eq!(row["dismissal"]["kinds"], json!(["Failed", "Finding"]));
            // A finding ceasing to wait reveals the covered failure, not news.
            record["security_findings_waiting"] = false.into();
            write_record(&scenario, 7, &record);
            let status = read_status(&scenario);
            assert_eq!(status["commands"][0]["group"], "done", "{status}");
            assert_eq!(status["commands"][0]["dismissal"]["kind"], "Failed");
            let undone = scenario.run(&["dismiss", name, "--undo"]);
            assert_eq!(undone.code, Some(0), "{}", undone.stderr);
            assert_eq!(
                read_status(&scenario)["commands"][0]["needs_you_kind"],
                "failed"
            );
        } else {
            assert_eq!(row["group"], "done", "{row}");
            assert_eq!(row["dismissal"]["kinds"], json!(["Failed"]));
        }
    }
}

#[test]
fn dismissed_silence_holds_while_logging_and_through_later_silences() {
    let mut run = SilentRun::new();
    run.go_silent("implement");
    let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    // Growing the trace clears Silent, without forgetting the dismissal.
    let session = run.record["session_log"].as_str().unwrap();
    fs::write(
        session,
        support::session_trace::records("{\"type\":\"result\"}"),
    )
    .unwrap();
    let row = run.row();
    assert_eq!(row["health"]["state"], "ok", "{row}");
    assert_eq!(row["group"], "working");
    assert_eq!(row["dismissal"]["kinds"], json!(["Silent"]));
    run.quiet_logs_for(20);
    let row = run.row();
    assert_eq!(row["health"]["state"], "silent");
    assert_eq!(row["group"], "working");
    assert_eq!(row["dismissal"]["kind"], "Silent");
}

#[test]
fn dismissed_live_command_returns_when_it_dies_fails_or_ends_ready() {
    for kind in ["Died", "Failed", "Ready"] {
        let mut run = SilentRun::new();
        run.go_silent("implement");
        let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
        assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
        if kind == "Died" {
            run.record["process_id"] = 0.into();
            run.record["process_start"] = "gone".into();
        } else {
            run.record["ended_at"] = chrono::Local::now().to_rfc3339().into();
            run.record["outcome"] = if kind == "Ready" {
                "ready for review"
            } else {
                "failed"
            }
            .into();
            if kind == "Ready" {
                run.record["pull_request"] = "https://github.com/acme/widgets/pull/7".into();
            }
        }
        fs::write(&run.path, serde_json::to_vec(&run.record).unwrap()).unwrap();
        let row = run.row();
        assert_eq!(
            row["group"],
            if kind == "Ready" {
                "ready for review"
            } else {
                "needs you"
            },
            "{row}"
        );
        assert!(row["dismissal"].is_null(), "{row}");
        if kind != "Ready" {
            assert_eq!(row["needs_you_kind"], kind.to_lowercase());
        }
        let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
        assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
        let row = run.row();
        assert_eq!(
            row["group"],
            if kind == "Died" { "resolved" } else { "done" },
            "{row}"
        );
        assert_eq!(row["dismissal"]["kinds"], json!(["Silent", kind]));
    }
}

#[test]
fn dismissed_spec_silence_covers_a_different_ticket_run() {
    let mut run = SilentRun::new();
    run.go_silent("implement");
    let mut child = run.record.clone();
    child["parent_record"] = run.path.to_str().unwrap().into();
    child["issue"]["number"] = 21.into();
    child["issue"]["url"] = run.scenario.issue_url(21).into();
    child["name"] = "acme/widgets#21".into();
    let first = run.path.with_extension("issue-21.child.json");
    fs::write(&first, serde_json::to_vec(&child).unwrap()).unwrap();
    run.record["kind"] = "Spec run".into();
    run.record["stages"] = json!([]);
    fs::write(&run.path, serde_json::to_vec(&run.record).unwrap()).unwrap();
    assert_eq!(run.row()["needs_you_kind"], "silent");
    let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    // First Ticket finishes; a different Ticket now goes Silent in this row.
    child["ended_at"] = chrono::Local::now().to_rfc3339().into();
    child["outcome"] = "merged".into();
    fs::write(&first, serde_json::to_vec(&child).unwrap()).unwrap();
    child["issue"]["number"] = 22.into();
    child["issue"]["url"] = run.scenario.issue_url(22).into();
    child["name"] = "acme/widgets#22".into();
    child["ended_at"] = Value::Null;
    child["outcome"] = Value::Null;
    fs::write(
        run.path.with_extension("issue-22.child.json"),
        serde_json::to_vec(&child).unwrap(),
    )
    .unwrap();
    let row = run.row();
    assert_eq!(row["children"][1]["health"]["state"], "silent", "{row}");
    assert_eq!(row["group"], "working");
    assert_eq!(row["dismissal"]["kinds"], json!(["Silent"]));
    let text = run.scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(text.stdout.contains("dismissed by "), "{}", text.stdout);
    assert!(text.stdout.contains("silent 20 min"), "{}", text.stdout);
}

#[test]
fn single_kind_dismissal_files_keep_their_kind_when_another_is_added() {
    let mut run = SilentRun::new();
    run.go_silent("implement");
    let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    let path = run.path.with_extension("dismiss.json");
    let mut file: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    file["rows"][0].as_object_mut().unwrap().remove("kinds");
    fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
    assert_eq!(run.row()["dismissal"]["kinds"], json!(["Silent"]));
    run.usual_implement_times();
    run.quiet_logs_for(0);
    assert_eq!(run.row()["needs_you_kind"], "slow");
    let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    assert_eq!(run.row()["dismissal"]["kinds"], json!(["Silent", "Slow"]));
}

#[test]
fn dismiss_live_silent_row_moves_it_to_working_and_keeps_it_running() {
    let mut run = SilentRun::new();
    run.go_silent("implement");
    let status = read_status(&run.scenario);
    let row = &status["commands"][0];
    assert_eq!(row["group"], "needs you");
    assert_eq!(row["needs_you_kind"], "silent");

    let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);

    let status = read_status(&run.scenario);
    let row = &status["commands"][0];
    assert_eq!(row["group"], "working");
    assert_eq!(row["state"], "live");
    assert_eq!(row["dismissal"]["kind"], "Silent");
    let text = run.scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(text.stdout.contains("dismissed by "), "{}", text.stdout);
    assert!(text.stdout.contains("silent 20 min"), "{}", text.stdout);
}

#[test]
fn dismiss_live_landing_row_moves_it_to_landing() {
    let mut run = SilentRun::new();
    run.go_silent("CI watch");
    let status = read_status(&run.scenario);
    assert_eq!(status["commands"][0]["group"], "needs you");
    assert_eq!(status["commands"][0]["landing"], true);

    let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);

    let status = read_status(&run.scenario);
    assert_eq!(status["commands"][0]["group"], "landing");
    assert_eq!(status["commands"][0]["dismissal"]["kind"], "Silent");
    let text = run.scenario.run(&["status"]);
    assert_eq!(text.code, Some(0), "{}", text.stderr);
    assert!(text.stdout.contains("dismissed by "), "{}", text.stdout);
    assert!(text.stdout.contains("silent 20 min"), "{}", text.stdout);
}

#[test]
fn dismiss_refuses_a_working_row_using_the_configured_silent_interval() {
    let mut run = SilentRun::new();
    run.scenario
        .user_config_is("[inbox]\nsilent_minutes = 30\n");
    run.go_silent("implement");
    let status = read_status(&run.scenario);
    assert_eq!(status["commands"][0]["group"], "working", "{status}");
    let refused = run.scenario.run(&["dismiss", "widgets#7"]);
    assert_ne!(refused.code, Some(0), "{}", refused.stdout);
    assert!(
        refused.stderr.contains("not Needs you or Ready for review"),
        "{}",
        refused.stderr
    );
}

#[test]
fn dismiss_refuses_a_row_working_under_configured_silence() {
    let mut run = SilentRun::new();
    run.scenario
        .user_config_is("[inbox]\nsilent_minutes = 30\n");
    run.go_silent("implement");
    let status = read_status(&run.scenario);
    assert_eq!(status["commands"][0]["group"], "working", "{status}");

    let dismissed = run.scenario.run(&["dismiss", "widgets#7"]);
    assert_ne!(dismissed.code, Some(0), "{}", dismissed.stdout);
    assert!(
        dismissed
            .stderr
            .contains("not Needs you or Ready for review"),
        "{}",
        dismissed.stderr
    );

    // A shorter configured interval also makes the visible Silent row
    // dismissible before the default ten minutes have elapsed.
    run.scenario.user_config_is("[inbox]\nsilent_minutes = 5\n");
    run.quiet_logs_for(6);
    let status = read_status(&run.scenario);
    assert_eq!(status["commands"][0]["group"], "needs you", "{status}");
    assert_eq!(status["commands"][0]["needs_you_kind"], "silent");
    let dismissed = run.scenario.run(&["dismiss", "widgets#7", "--json"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);
    let status = read_status(&run.scenario);
    assert_eq!(status["commands"][0]["group"], "working", "{status}");
    assert_eq!(status["commands"][0]["dismissal"]["kind"], "Silent");
}

#[test]
fn dismiss_dispatched_row_covers_only_that_row_of_the_record() {
    let scenario = Scenario::new();
    let now = chrono::Local::now().to_rfc3339();
    let mut outer = record_value(&scenario, 7, &now, Some(&now));
    outer["kind"] = "Pickup run".into();
    outer["name"] = "pickup-20261018T120000-0400".into();
    outer["issue"] = Value::Null;
    outer["outcome"] = "merged".into();
    let mut dispatched = record_value(&scenario, 7, &now, Some(&now));
    dispatched["outcome"] = "ready for review".into();
    dispatched["pull_request"] = "https://github.com/acme/widgets/pull/7".into();
    outer["dispatched"] = json!([dispatched]);
    let folder = scenario.path("home/.thirdshift/logs/acme/widgets/commands/pickup");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("pickup-20261018T120000-0400-fixture.command.json"),
        serde_json::to_vec(&outer).unwrap(),
    )
    .unwrap();

    let status = read_status(&scenario);
    assert_eq!(status["commands"].as_array().unwrap().len(), 2, "{status}");

    let dismissed = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);

    let status = read_status(&scenario);
    assert!(
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|command| command["group"] != "needs you"),
        "{status}"
    );
    let rows: Vec<_> = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|command| command["dismissal"].is_object())
        .collect();
    assert_eq!(rows.len(), 1, "{status}");
    assert_eq!(rows[0]["name"], "acme/widgets#7");
}

#[test]
fn undo_collapsed_row_takes_back_every_command_it_stood_for() {
    let scenario = Scenario::new();
    write_failed(&scenario, 7, 0, Some("boom"));
    write_failed(&scenario, 8, 0, Some("boom"));
    let dismissed = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);

    let undone = scenario.run(&["dismiss", "widgets#8", "--undo"]);
    assert_eq!(undone.code, Some(0), "{}", undone.stderr);

    let status = read_status(&scenario);
    assert_eq!(status["commands"].as_array().unwrap().len(), 1, "{status}");
    assert_eq!(status["commands"][0]["group"], "needs you");
    assert_eq!(status["commands"][0]["count"], 2);
}

#[test]
fn dismissal_retention_is_measured_from_dismissal() {
    let scenario = Scenario::new();
    // Eight-day-old failure dismissed two days ago, while still in Needs you.
    write_failed(&scenario, 7, 8, None);
    let path = record_path(&scenario, 7);
    let at = (chrono::Local::now() - chrono::Duration::days(2)).to_rfc3339();
    fs::write(
        path.with_extension("dismiss.json"),
        serde_json::to_vec(&json!({"rows": [{
            "selector": format!("{}::acme/widgets#7", path.display()),
            "kind": "Failed", "who": "reviewer", "at": at,
            "from_console": false
        }]}))
        .unwrap(),
    )
    .unwrap();
    let status = read_status(&scenario);
    assert_eq!(groups(&status), vec![(7, "resolved".into())], "{status}");
    assert_eq!(status["commands"][0]["dismissal"]["who"], "reviewer");

    // Once the dismissal's own retention ends, History still says who
    // dismissed it, and undo works there too.
    let mut file: Value =
        serde_json::from_slice(&fs::read(path.with_extension("dismiss.json")).unwrap()).unwrap();
    file["rows"][0]["at"] = (chrono::Local::now() - chrono::Duration::days(8))
        .to_rfc3339()
        .into();
    fs::write(
        path.with_extension("dismiss.json"),
        serde_json::to_vec(&file).unwrap(),
    )
    .unwrap();
    assert!(groups(&read_status(&scenario)).is_empty());
    let all = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(all.code, Some(0), "{}", all.stderr);
    let all: Value = serde_json::from_str(&all.stdout).unwrap();
    assert_eq!(all["commands"][0]["group"], "history");
    assert_eq!(all["commands"][0]["dismissal"]["who"], "reviewer");
    let undone = scenario.run(&["dismiss", "widgets#7", "--undo"]);
    assert_eq!(undone.code, Some(0), "{}", undone.stderr);
    let all = scenario.run(&["status", "--all", "--json"]);
    assert_eq!(all.code, Some(0), "{}", all.stderr);
    let all: Value = serde_json::from_str(&all.stdout).unwrap();
    assert!(all["commands"][0]["dismissal"].is_null(), "{all}");
}

#[test]
fn dismissal_retention_survives_the_original_wait_cutoff() {
    let scenario = Scenario::new();
    write_failed(&scenario, 7, 6, None);
    let dismissed = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);

    // Represent two days passing: the Command ended eight days ago,
    // and its Dismissal was made two days ago.
    write_failed(&scenario, 7, 8, None);
    let sidecar = record_path(&scenario, 7).with_extension("dismiss.json");
    let mut file: Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    file["rows"][0]["at"] = (chrono::Local::now() - chrono::Duration::days(2))
        .to_rfc3339()
        .into();
    fs::write(sidecar, serde_json::to_vec(&file).unwrap()).unwrap();

    let status = read_status(&scenario);
    assert_eq!(groups(&status), vec![(7, "resolved".into())], "{status}");
    assert_eq!(status["commands"][0]["dismissal"]["kind"], "Failed");

    let original_start = status["commands"][0]["started_at"].clone();
    let now = chrono::Local::now().to_rfc3339();
    let mut later = record_value(&scenario, 7, &now, Some(&now));
    later["outcome"] = "merged".into();
    write_record(&scenario, 8, &later);
    let status = read_status(&scenario);
    let original = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["started_at"] == original_start)
        .unwrap();
    assert_eq!(original["group"], "resolved");
    assert_eq!(original["dismissal"]["kind"], "Failed");
}

#[cfg(unix)]
#[test]
fn undo_reports_sidecar_removal_failure() {
    use std::os::unix::fs::PermissionsExt;

    let scenario = Scenario::new();
    write_failed(&scenario, 7, 0, None);
    let dismissed = scenario.run(&["dismiss", "widgets#7"]);
    assert_eq!(dismissed.code, Some(0), "{}", dismissed.stderr);

    let record = record_path(&scenario, 7);
    let directory = record.parent().unwrap();
    let activity_before = activity(&scenario);
    let original = fs::metadata(directory).unwrap().permissions();
    fs::set_permissions(directory, fs::Permissions::from_mode(0o555)).unwrap();
    let undone = scenario.run(&["dismiss", "widgets#7", "--undo", "--json"]);
    fs::set_permissions(directory, original).unwrap();

    assert!(record.with_extension("dismiss.json").exists());
    assert_ne!(undone.code, Some(0), "{}", undone.stdout);
    assert_eq!(activity(&scenario), activity_before);
    assert!(
        undone.stderr.contains("could not remove the dismissal"),
        "{}",
        undone.stderr
    );
}
