use super::{Console, configure, record};
use serde_json::Value;
use std::path::Path;
use std::{fs, process::Command};

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

#[test]
fn taken_up_rows_say_why_they_left_on_the_console_row() {
    use super::write_record;

    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    record(home.path(), 40, Some("failed"), 10);
    record(home.path(), 41, Some("merged"), 0);
    // The later Command took up issue 40, so the ten-day-old failure stays
    // in Resolved instead of going straight to History.
    let path = home
        .path()
        .join(".thirdshift/logs/acme/widgets/commands/issue/41-fixture.command.json");
    let mut later: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    later["issue"] = serde_json::json!({
        "number": 40,
        "url": "https://github.com/acme/widgets/issues/40",
        "title": "Widgets",
    });
    write_record(home.path(), 41, &later);
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start(binary, home.path());
    let html = console.get("/").text().unwrap();
    let resolved = html
        .split("data-group=\"resolved\"")
        .nth(1)
        .unwrap()
        .split("</details>")
        .next()
        .unwrap();
    assert!(resolved.contains(">acme/widgets#40<"), "{resolved}");
    let row = resolved
        .split("<li class=\"command\"")
        .find(|row| row.contains(">acme/widgets#40<"))
        .expect(resolved);
    assert!(
        row.contains("<p class=\"left\">left needs you: taken up by a later Command started at"),
        "{row}"
    );
    // The Console and status agree on the row's group and its reason.
    let status = Command::new(binary)
        .args(["status", "--json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    let item = status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["issue"]["number"] == 40 && item["group"] == "resolved")
        .unwrap();
    assert_eq!(item["left_reason"], "taken up by a later Command", "{item}");
}
