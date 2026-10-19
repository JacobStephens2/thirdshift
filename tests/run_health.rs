mod support;

use std::fs::{self, File, FileTimes};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};
use support::{HeldRun, Scenario};

const LOGS: &str = "home/.thirdshift/logs/acme/widgets/commands/issue";

struct LiveRun {
    scenario: Scenario,
    held: Option<HeldRun>,
    path: PathBuf,
    record: Value,
}

impl LiveRun {
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

    fn stage_age(&mut self, minutes: i64) {
        let began = (chrono::Local::now() - chrono::Duration::minutes(minutes)).to_rfc3339();
        self.record["started_at"] = began.clone().into();
        self.record["stages"] = json!([{
            "name": "implement", "began_at": began,
            "session_log": self.record["session_log"]
        }]);
        fs::write(&self.path, serde_json::to_vec(&self.record).unwrap()).unwrap();
    }

    fn log_ages(&self, command: u64, session: u64) {
        for (path, minutes) in [
            (
                self.path.with_extension("log").with_file_name(
                    self.path
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace(".command.json", ".log"),
                ),
                command,
            ),
            (
                PathBuf::from(self.record["session_log"].as_str().unwrap()),
                session,
            ),
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

    fn status(&self) -> Value {
        let result = self.scenario.run(&["status", "--json"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let status: Value = serde_json::from_str(&result.stdout).unwrap();
        status["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["state"] == "live")
            .unwrap()
            .clone()
    }

    fn history(&self, number: u32, duration: Option<i64>, minutes_ago: i64, repo: &str) {
        let mut record = self.record.clone();
        let ended = chrono::Local::now() - chrono::Duration::minutes(minutes_ago);
        let began = ended - chrono::Duration::minutes(duration.unwrap_or(1));
        record["started_at"] = began.to_rfc3339().into();
        record["ended_at"] = ended.to_rfc3339().into();
        record["outcome"] = "ready for review".into();
        record["repository"] = repo.into();
        record["issue"]["number"] = number.into();
        record["session_log"] = Value::Null;
        record["stages"] = duration.map_or(json!([]), |_| {
            json!([{
                "name": "implement", "began_at": began.to_rfc3339(), "session_log": null
            }])
        });
        let folder = self
            .scenario
            .path(&format!("home/.thirdshift/logs/{repo}/commands/issue"));
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join(format!("{number}-fixture.command.json")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }
}

impl Drop for LiveRun {
    fn drop(&mut self) {
        fs::write(self.scenario.path("release"), "").unwrap();
        self.held.take().unwrap().finish();
    }
}

#[test]
fn silent_requires_both_logs_and_escalates_in_status_and_show() {
    let mut run = LiveRun::new();
    run.stage_age(60);
    for ages in [(45, 0), (0, 45), (9, 9)] {
        run.log_ages(ages.0, ages.1);
        assert_eq!(run.status()["health"]["state"], "ok");
    }
    for (minutes, step, words) in [(14, 1, "silent 14 min"), (45, 2, "silent 45 min · past 30")] {
        run.log_ages(minutes, minutes);
        let status = run.status();
        assert_eq!(status["health"]["state"], "silent");
        assert_eq!(status["health"]["step"], step);
        for args in [vec!["status"], vec!["show", "widgets#7"]] {
            let output = run.scenario.run(&args);
            assert_eq!(output.code, Some(0), "{}", output.stderr);
            assert!(output.stdout.contains(words), "{}", output.stdout);
        }
    }
    run.scenario
        .user_config_is("[inbox]\nsilent_minutes = 20\n");
    run.log_ages(14, 14);
    assert_eq!(run.status()["health"]["state"], "ok");
}

#[test]
fn slow_uses_the_repositorys_last_thirty_records_and_escalates_at_twice_the_top() {
    let mut run = LiveRun::new();
    run.stage_age(20);
    run.log_ages(0, 0);
    for (number, duration) in [(10, 10), (11, 14)] {
        run.history(number, Some(duration), 100, "acme/widgets");
    }
    assert_eq!(run.status()["health"]["state"], "ok");
    let shown = run.scenario.run(&["show", "widgets#7", "--json"]);
    let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
    assert!(shown["command"]["stages"][0]["usual_range"].is_null());
    run.history(12, Some(20), 100, "acme/widgets");
    for number in 20..46 {
        run.history(number, None, 200, "acme/widgets");
    }
    run.history(90, Some(900), 2000, "acme/widgets");
    for number in 100..103 {
        run.history(number, Some(900), 100, "other/gadgets");
    }
    for (age, step, words) in [
        (20, 1, "slower than usual (12–17 min), still working"),
        (34, 2, "2× usual"),
        (51, 2, "3× usual"),
    ] {
        run.stage_age(age);
        let status = run.status();
        assert_eq!(status["health"]["state"], "slow", "{status}");
        assert_eq!(status["health"]["step"], step);
        let shown = run.scenario.run(&["show", "widgets#7", "--json"]);
        assert_eq!(shown.code, Some(0), "{}", shown.stderr);
        let shown: Value = serde_json::from_str(&shown.stdout).unwrap();
        assert_eq!(
            shown["command"]["stages"][0]["usual_range"],
            json!({"lower_seconds":720.0, "upper_seconds":1020.0})
        );
        assert_eq!(shown["command"]["health"]["state"], "slow");
        for args in [vec!["status"], vec!["show", "widgets#7"]] {
            let output = run.scenario.run(&args);
            assert!(output.stdout.contains(words), "{}", output.stdout);
            assert!(output.stdout.contains("12–17 min"), "{}", output.stdout);
        }
    }
    run.log_ages(45, 45);
    assert_eq!(run.status()["health"]["state"], "silent");
}

#[test]
fn status_warns_when_waiting_lines_are_not_more_frequent_than_silence() {
    let scenario = Scenario::new();
    for waiting in [1, 5] {
        scenario.user_config_is(&format!(
            "[logs]\nwaiting_line_minutes = {waiting}\n[inbox]\nsilent_minutes = 1\n"
        ));
        let output = scenario.run(&["status", "--json"]);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        serde_json::from_str::<Value>(&output.stdout).unwrap();
        assert!(
            output.stderr.contains("logs.waiting_line_minutes"),
            "{}",
            output.stderr
        );
        assert!(
            output.stderr.contains("inbox.silent_minutes"),
            "{}",
            output.stderr
        );
        assert!(output.stderr.contains("30 sec"), "{}", output.stderr);
    }
    scenario.user_config_is("[logs]\nwaiting_line_minutes = 1\n[inbox]\nsilent_minutes = 2\n");
    assert!(!scenario.run(&["status"]).stderr.contains("warning"));
}
