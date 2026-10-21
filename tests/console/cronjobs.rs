use super::*;
use std::os::unix::fs::PermissionsExt;

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
    fs::write(&fake, "#!/bin/sh\n[ \"$#\" = 1 ] && [ \"$1\" = -l ] || exit 99\nif [ -f \"$HOME/cron-error\" ]; then cat \"$HOME/cron-error\" >&2; exit 1; fi\ncat \"$HOME/crontab\"\n").unwrap();
    fs::set_permissions(fake, fs::Permissions::from_mode(0o755)).unwrap();
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
        json!([
            {"schedule": "*/5 * * * *", "command": "cd /srv/repo && /home/me/.cargo/bin/thirdshift pickup base main >> /tmp/cron.log 2>&1"},
            {"schedule": "@reboot", "command": "thirdshift secure base main # <script>alert(1)</script>"}
        ])
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
            json!([])
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
        json!([
            {"schedule": "* * * * *", "command": "/usr/local/bin/thirdshift%stdin"}
        ])
    );
}
