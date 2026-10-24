use super::{Console, configure, record};
use serde_json::Value;
use std::path::Path;
use std::{fs, process::Command};

#[test]
fn ended_rows_distinguish_finish_age_from_run_duration() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let end = chrono::Utc::now() - chrono::Duration::minutes(52);
    let mut fixture = record(home.path(), 7, Some("merged"), 0);
    fixture["started_at"] = json!((end - chrono::Duration::minutes(140)).to_rfc3339());
    fixture["ended_at"] = json!(end.to_rfc3339());
    write_record(home.path(), 7, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    assert!(html.contains("52 min ago"), "{html}");
    assert!(
        html.contains(&format!("<time datetime=\"{}\">Ended ", end.to_rfc3339())),
        "{html}"
    );
    assert!(html.contains(" UTC</time>"), "{html}");
    assert!(!html.contains("2 hr 20 min"), "{html}");
}

#[test]
fn unavailable_finish_times_are_not_invented_from_other_observations() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    for (number, outcome, end) in [
        (1, Some("merged"), Value::Null),
        (2, Some("failed"), json!("not a timestamp")),
        (3, None, Value::Null),
    ] {
        let mut fixture = record(home.path(), number, outcome, 0);
        fixture["ended_at"] = end;
        write_record(home.path(), number, &fixture);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    for page in ["/", "/?page=history"] {
        let html = console.get(page).text().unwrap();
        assert_eq!(html.matches("End time unknown").count(), 3, "{html}");
        assert!(!html.contains("<time datetime="), "{html}");
        assert!(!html.contains("ago</span>"), "{html}");
    }
}

#[test]
fn finish_instants_survive_offsets_and_nested_history() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut outer = record(home.path(), 7, Some("merged"), 0);
    outer["kind"] = json!("Pickup run");
    outer["ended_at"] = json!("2026-10-23T19:03:18.964097412-04:00");
    let mut child = outer.clone();
    child["kind"] = json!("Run");
    child["name"] = json!("acme/widgets#8");
    child["issue"]["number"] = json!(8);
    child["ended_at"] = json!("2026-10-23T19:03:17.705023444-04:00");
    outer["dispatched"] = json!([child]);
    write_record(home.path(), 7, &outer);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/?page=history").text().unwrap();
    assert!(html.contains("<time datetime=\"2026-10-23T19:03:18.964097412-04:00\">Ended Oct 23, 2026 at 23:03:18 UTC</time>"), "{html}");
    let nested = html.split("<ul class=\"children\">").nth(1).unwrap();
    assert!(nested.contains("<time datetime=\"2026-10-23T19:03:17.705023444-04:00\">Ended Oct 23, 2026 at 23:03:17 UTC</time>"), "{nested}");
    assert!(!nested.contains("23:03:18 UTC"), "{nested}");
}

#[test]
fn shared_ended_states_show_each_commands_finish() {
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let now = chrono::Utc::now();
    for (number, outcome, days) in [
        (1, "failed", 0),
        (2, "ready for review", 0),
        (3, "merged", 0),
        (4, "merged", 2),
    ] {
        // A slightly future finish (clock skew) must clamp to a recent zero age.
        let end = (now - chrono::Duration::days(days)
            + chrono::Duration::seconds(if days == 0 { 30 } else { 0 }))
        .to_rfc3339();
        let mut fixture = record(home.path(), number, Some(outcome), days);
        fixture["ended_at"] = json!(end);
        write_record(home.path(), number, &fixture);
    }
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    for group in ["needs you", "ready for review", "done", "resolved"] {
        let section = html
            .split(&format!("data-group=\"{group}\""))
            .nth(1)
            .unwrap()
            .split("</details>")
            .next()
            .unwrap();
        assert!(section.contains("<time datetime="), "{group}: {section}");
        assert!(section.contains(" UTC</time> · "), "{group}: {section}");
        assert!(section.contains(" ago</span>"), "{group}: {section}");
    }
    assert!(html.contains("0 sec ago"), "{html}");
    assert!(html.contains("48 hr 0 min ago"), "{html}");
}

#[test]
fn retained_activity_finishes_show_dates_and_missing_finishes_stay_unknown() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let root = home.path().join(".thirdshift/logs/acme/widgets");
    fs::create_dir_all(root.join("commands/issue")).unwrap();
    fs::write(
        root.join("commands/issue/8-20260101T230000-0400.log"),
        "Old Command output",
    )
    .unwrap();
    fs::write(root.join("activity.log"), "2026-01-01 23:00:00 Run #8 started: commands/issue/8-20260101T230000-0400.log\n2026-01-01 23:59:00 Run #8 ended: merged\n").unwrap();
    let session = root.join("commands/issue/9-20260102T120000+0000-implement.logs.jsonl");
    super::support::session_trace::write(&session, "Session without a Command record");
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/?page=history").text().unwrap();
    assert!(
        html.contains(
            "<time datetime=\"2026-01-01T23:59:00-04:00\">Ended Jan 2, 2026 at 03:59:00 UTC</time>"
        ),
        "{html}"
    );
    assert_eq!(html.matches("<time datetime=").count(), 1, "{html}");
    assert_eq!(html.matches("End time unknown").count(), 1, "{html}");
}

#[cfg(unix)]
#[test]
fn live_rows_keep_start_age_without_claiming_to_have_ended() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let mut fixture = super::watching::live_record(home.path(), 7, std::process::id());
    fixture["started_at"] =
        serde_json::json!((chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339());
    super::write_record(home.path(), 7, &fixture);
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    assert!(html.contains("data-live=\"true\""), "{html}");
    assert!(html.contains("<span class=\"age\">5 min</span>"), "{html}");
    assert!(!html.contains("<time datetime="), "{html}");
    assert!(!html.contains("End time unknown"), "{html}");
    assert!(!html.contains("ago</span>"), "{html}");
}

#[test]
fn idle_inbox_explains_missing_activity_before_the_folded_sections() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    let notice = html
        .split("aria-label=\"No running Commands\"")
        .nth(1)
        .expect(&html);
    assert!(
        notice.contains("No Commands are running on this machine."),
        "{html}"
    );
    assert!(
        notice.contains("No Pass is currently recorded as skipped."),
        "{html}"
    );
    assert!(notice.contains("href=\"/cronjobs\""), "{html}");
    assert!(notice.contains("Activity logs"), "{html}");
    assert!(notice.contains("thirdshift status"), "{html}");
    assert!(html.find("No running Commands").unwrap() < html.find("id=\"group-1\"").unwrap());
    for page in ["history", "security"] {
        let html = console.get(&format!("/?page={page}")).text().unwrap();
        assert!(!html.contains("No running Commands"), "{page}: {html}");
    }

    // Ended work does not make the machine active.
    record(home.path(), 1, Some("merged"), 0);
    let html = console.get("/").text().unwrap();
    assert!(html.contains("No running Commands"), "{html}");
}

#[test]
fn idle_inbox_surfaces_recorded_skip_reasons_and_waiting_work() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 1, Some("failed"), 0);
    record(home.path(), 2, Some("ready for review"), 0);
    let activity = home
        .path()
        .join(".thirdshift/logs/acme/widgets/activity.log");
    fs::write(&activity, "2026-01-01 12:00:00 Pickup run skipped: Claim limit reached <script>unsafe</script>\n2026-01-01 12:01:00 Architect run skipped: plan still open https://github.com/acme/widgets/issues/7\n").unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/").text().unwrap();
    let notice = html
        .split("aria-label=\"No running Commands\"")
        .nth(1)
        .expect(&html)
        .split("</section>")
        .next()
        .unwrap();
    assert!(notice.contains("Last recorded skipped Passes"), "{notice}");
    let status = Command::new(binary)
        .args(["status", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["idle_passes"].as_array().unwrap().len(), 2);
    assert!(notice.contains("Pickup run idle on acme/widgets: Claim limit reached &#60;script&#62;unsafe&#60;/script&#62;"), "{notice}");
    assert!(
        notice.contains("Architect run idle on acme/widgets: plan still open"),
        "{notice}"
    );
    assert!(
        notice.contains("href=\"https://github.com/acme/widgets/issues/7\""),
        "{notice}"
    );
    assert!(!notice.contains("<script>unsafe</script>"));
    assert!(!notice.contains("No Pass is currently recorded as skipped."));
    assert!(notice.contains("href=\"#group-1\""), "{notice}");
    assert!(notice.contains("href=\"#group-2\""), "{notice}");
    assert!(
        !html.contains("aria-label=\"Idle Passes\""),
        "skip reasons should appear only once"
    );
}

#[test]
fn idle_inbox_does_not_deny_skips_recorded_before_a_later_run() {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let repo = home.path().join(".thirdshift/logs/acme/widgets");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("activity.log"), "2026-01-01 12:00:00 Pickup run skipped: no Ready issue on acme/widgets\n2026-01-01 12:01:00 Pickup run #7 started: commands/pickup/7-fixture.log, on codex\n2026-01-01 12:02:00 Pickup run #7 ended: merged\n").unwrap();
    let console = Console::start(Path::new(env!("CARGO_BIN_EXE_thirdshift")), home.path());
    let html = console.get("/").text().unwrap();
    assert!(
        html.contains("No Pass is currently recorded as skipped."),
        "{html}"
    );
    assert!(
        !html.contains("No skipped Pass reason has been recorded."),
        "{html}"
    );
}

#[test]
fn live_commands_never_show_the_idle_notice_even_in_needs_you_or_hidden_sections() {
    use super::{HeldFixture, support};
    use serde_json::json;
    use std::time::{Duration, SystemTime};

    let scenario = support::Scenario::new();
    scenario.user_config_is("[console]\nport = 0\nhidden_sections = [\"working\", \"landing\"]\n");
    scenario.agent_does(&format!(
        "touch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\nexit 1\n",
        scenario.path("started").display(),
        scenario.path("release").display()
    ));
    let _held = HeldFixture {
        run: Some(scenario.run_until(&[&scenario.issue_url(7)], &[], "started")),
        release: scenario.path("release"),
    };
    let dir = scenario.path("home/.thirdshift/logs/acme/widgets/commands/issue");
    let path = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".command.json"))
        .unwrap();
    let mut fixture: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let console = Console::start(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        &scenario.path("home"),
    );
    for stage in ["implement", "CI watch"] {
        fixture["stages"][0]["name"] = json!(stage);
        fs::write(&path, serde_json::to_vec(&fixture).unwrap()).unwrap();
        let html = console.get("/").text().unwrap();
        assert!(html.contains("data-live=\"true\""), "{html}");
        assert!(!html.contains("No running Commands"), "{stage}: {html}");
    }
    // Silence moves the live Command to Needs you, leaving both active groups empty.
    fixture["stages"][0]["began_at"] =
        json!((chrono::Local::now() - chrono::Duration::minutes(20)).to_rfc3339());
    fs::write(&path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    for file in fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "log" || ext == "jsonl")
        })
    {
        fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(20 * 60)),
            )
            .unwrap();
    }
    let html = console.get("/").text().unwrap();
    assert!(html.contains("data-state=\"Silent\""), "{html}");
    assert!(!html.contains("No running Commands"), "{html}");
}

#[cfg(unix)]
#[test]
fn landing_ready_and_stopping_rows_carry_their_state_in_the_console_and_status() {
    use super::watching::{FixtureProcess, live_record};
    use super::write_record;
    use serde_json::json;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    let work = FixtureProcess(Command::new("sleep").arg("60").spawn().unwrap());
    let pid = work.0.id();
    for (number, stage) in [(1, "CI watch"), (2, "implement"), (3, "implement")] {
        let mut fixture = live_record(home.path(), number, pid);
        fixture["stages"][0]["name"] = json!(stage);
        write_record(home.path(), number, &fixture);
    }
    record(home.path(), 4, Some("ready for review"), 0);
    record(home.path(), 5, Some("failed"), 0);
    let dir = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue");
    fs::write(
        dir.join("3-fixture.command.stop.json"),
        r#"{"who":"jacob","requeue":false}"#,
    )
    .unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/").text().unwrap();
    let status = |args: &[&str]| {
        let output = Command::new(binary)
            .args(args)
            .env("HOME", home.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let json: Value = serde_json::from_str(&status(&["status", "--json"])).unwrap();
    let text = status(&["status"]);
    for (number, group, state, live, stopping) in [
        (1, "landing", "Landing", true, false),
        (2, "working", "live", true, false),
        (3, "working", "Stopping", true, true),
        (4, "ready for review", "Ready", false, false),
        (5, "needs you", "Failed", false, false),
    ] {
        let row = html
            .split("<li class=\"command\"")
            .find(|row| row.contains(&format!(">acme/widgets#{number}<")))
            .expect(&html);
        assert!(row.contains(&format!("data-live=\"{live}\"")), "{row}");
        assert!(row.contains(&format!("data-state=\"{state}\"")), "{row}");
        assert!(
            row.contains(&format!("<span class=\"state\">{state}</span>")),
            "{row}"
        );
        let section = html
            .split(&format!("data-group=\"{group}\""))
            .nth(1)
            .unwrap()
            .split("</details>")
            .next()
            .unwrap();
        assert!(
            section.contains(&format!(">acme/widgets#{number}<")),
            "{section}"
        );
        let item = json["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["issue"]["number"] == number)
            .unwrap();
        assert_eq!(item["group"], group, "{item}");
        assert_eq!(item["stopping"], stopping, "{item}");
        assert!(
            text.contains(&format!(
                "Spec run acme/widgets#{number} Widgets <script>alert(1)</script>"
            )) && text
                .lines()
                .any(|line| line.contains(&format!("acme/widgets#{number} "))
                    && line.contains(&format!(" · {state} · "))),
            "{text}"
        );
    }
}
