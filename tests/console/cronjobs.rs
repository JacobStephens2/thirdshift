use super::*;

fn command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_thirdshift"));
    command.env("HOME", home).env(
        "PATH",
        format!(
            "{}:{}",
            home.join("bin").display(),
            std::env::var("PATH").unwrap()
        ),
    );
    command
}

fn fixture() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    configure(home.path());
    fs::create_dir(home.path().join("bin")).unwrap();
    let fake = home.path().join("bin/crontab");
    support::write_executable(
        &fake,
        "#!/bin/sh\n[ \"$#\" = 1 ] && [ \"$1\" = -l ] || exit 99\nif [ -f \"$HOME/cron-error\" ]; then cat \"$HOME/cron-error\" >&2; exit 1; fi\ncat \"$HOME/crontab\"\n",
    );
    home
}

#[test]
fn cronjobs_pairs_cli_and_console_without_running_scheduled_commands() {
    let home = fixture();
    fs::write(home.path().join("crontab"), "# thirdshift disabled\nMAILTO=thirdshift@example.test\n0 1 * * * backup\n*/5 * * * * cd /srv/repo && /home/me/.cargo/bin/thirdshift pickup base main >> /tmp/cron.log 2>&1\n@reboot thirdshift secure base main # <script>alert(1)</script>\n").unwrap();
    let output = command(home.path())
        .args(["cronjobs", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let jobs: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        jobs,
        json!({"jobs": [
            {"schedule": "*/5 * * * *", "command": "cd /srv/repo && /home/me/.cargo/bin/thirdshift pickup base main >> /tmp/cron.log 2>&1"},
            {"schedule": "@reboot", "command": "thirdshift secure base main # <script>alert(1)</script>"}
        ], "pass_starts": []})
    );
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    assert!(
        console
            .get("/")
            .text()
            .unwrap()
            .contains("href=\"/cronjobs\"")
    );
    let response = console.get("/cronjobs");
    assert_eq!(response.status(), 200);
    let html = response.text().unwrap();
    assert!(html.contains("*/5 * * * *"));
    assert!(html.contains("@reboot"));
    assert!(html.contains("thirdshift cronjobs --json"));
    assert!(html.contains("&#60;script&#62;"), "{html}");
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(!html.contains("backup"));
    assert!(!html.contains("MAILTO="));
    assert!(!html.contains("thirdshift disabled"));
}

#[test]
fn last_fired_times_pair_both_cli_views_with_the_console_per_repository() {
    let home = fixture();
    fs::write(home.path().join("crontab"), "").unwrap();
    let fired = (chrono::Utc::now() - chrono::Duration::seconds(310))
        .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
    for (repo, kind) in [
        ("acme/widgets", "pickup"),
        ("acme/widgets", "architect"),
        ("acme/other", "pickup"),
    ] {
        let root = home.path().join(".thirdshift/logs").join(repo);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(format!("{kind}.last-fired")), &fired).unwrap();
    }
    let root = home.path().join(".thirdshift/logs/acme/widgets");
    fs::write(
        root.join("activity.log"),
        "2026-01-01 12:00:00 Pickup run skipped: no Ready issue on acme/widgets\n",
    )
    .unwrap();
    let mut observed = Vec::new();
    for view in ["status", "cronjobs"] {
        let output = command(home.path())
            .args([view, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let starts = value["pass_starts"]
            .as_array()
            .unwrap_or_else(|| panic!("{value}"));
        assert_eq!(starts.len(), 3, "{value}");
        let mut identities = Vec::new();
        for start in starts {
            assert_eq!(start["last_fired_at"], fired);
            assert!(start["age_seconds"].as_i64().unwrap() >= 310);
            identities.push((start["repository"].clone(), start["kind"].clone()));
        }
        observed.push(identities);
        let output = command(home.path()).arg(view).output().unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        for line in [
            "Pickup last fired 5 min ago on acme/widgets",
            "Pickup last fired 5 min ago on acme/other",
            "Architect last fired 5 min ago on acme/widgets",
        ] {
            assert!(text.contains(line), "{view}: {text}");
        }
    }
    assert_eq!(observed[0], observed[1]);
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    for path in ["/", "/cronjobs"] {
        let response = console.get(path);
        assert_eq!(response.status(), 200);
        let html = response.text().unwrap();
        for line in [
            format!(
                "Pickup last fired <span data-pass-fired-at=\"{fired}\">5 min</span> ago on acme/widgets"
            ),
            format!(
                "Pickup last fired <span data-pass-fired-at=\"{fired}\">5 min</span> ago on acme/other"
            ),
            format!(
                "Architect last fired <span data-pass-fired-at=\"{fired}\">5 min</span> ago on acme/widgets"
            ),
        ] {
            assert!(html.contains(&line), "{path}: {html}");
        }
    }
}

#[test]
fn cronjobs_distinguishes_empty_crontabs_from_read_failures() {
    let home = fixture();
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    fs::write(home.path().join("crontab"), "# No jobs\n0 1 * * * backup\n").unwrap();
    for diagnostic in [
        "",
        "no crontab for owner\n",
        "crontab: no crontab for owner\n",
    ] {
        if !diagnostic.is_empty() {
            fs::write(home.path().join("cron-error"), diagnostic).unwrap();
        }
        let output = command(home.path())
            .args(["cronjobs", "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{diagnostic}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            json!({"jobs": [], "pass_starts": []})
        );
        let response = console.get("/cronjobs");
        assert_eq!(response.status(), 200);
        assert!(response.text().unwrap().contains("No thirdshift cron jobs"));
    }
    fs::write(
        home.path().join("cron-error"),
        "crontab: permission denied\n",
    )
    .unwrap();
    let output = command(home.path())
        .args(["cronjobs", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("permission denied"));
    let response = console.get("/cronjobs");
    assert_eq!(response.status(), 500);
    assert!(!response.text().unwrap().contains("No thirdshift cron jobs"));
}

#[test]
fn cronjobs_respects_crons_command_and_stdin_separator() {
    let home = fixture();
    fs::write(home.path().join("crontab"), "* * * * * /usr/local/bin/thirdshift%stdin\n* * * * * backup% /usr/local/bin/thirdshift\n* * * * * /usr/local/bin/thirdshift\\%other\n").unwrap();
    let output = command(home.path())
        .args(["cronjobs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"jobs": [
            {"schedule": "* * * * *", "command": "/usr/local/bin/thirdshift%stdin"}
        ], "pass_starts": []})
    );
}
