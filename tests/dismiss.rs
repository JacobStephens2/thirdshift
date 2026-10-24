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

fn wrote(scenario: &Scenario, number: u32, started: &str, ended: Option<&str>) -> Value {
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
    let mut record = wrote(scenario, number, &time, Some(&time));
    record["cause"] = cause.into();
    write_record(scenario, number, &record);
}

fn write_ready(scenario: &Scenario, number: u32) {
    let now = chrono::Local::now().to_rfc3339();
    let mut record = wrote(scenario, number, &now, Some(&now));
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
                    FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(20 * 60)),
                )
                .unwrap();
        }
    }
}

impl Drop for SilentRun {
    fn drop(&mut self) {
        fs::write(self.scenario.path("release"), "").unwrap();
        self.held.take().unwrap().finish();
    }
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
fn dismiss_dispatched_row_covers_only_that_row_of_the_record() {
    let scenario = Scenario::new();
    let now = chrono::Local::now().to_rfc3339();
    let mut outer = wrote(&scenario, 7, &now, Some(&now));
    outer["kind"] = "Pickup run".into();
    outer["name"] = "pickup-20261018T120000-0400".into();
    outer["issue"] = Value::Null;
    outer["outcome"] = "merged".into();
    let mut dispatched = wrote(&scenario, 7, &now, Some(&now));
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
