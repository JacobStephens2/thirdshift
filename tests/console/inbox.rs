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
