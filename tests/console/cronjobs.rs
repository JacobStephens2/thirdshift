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

fn checkout(home: &Path, name: &str, origin: &str) -> std::path::PathBuf {
    let path = home.join(name);
    fs::create_dir_all(&path).unwrap();
    for arguments in [vec!["init", "-q"], vec!["remote", "add", "origin", origin]] {
        let output = Command::new("git")
            .current_dir(&path)
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    path
}

#[test]
fn cronjobs_groups_repositories_stably_in_cli_and_api() {
    let home = fixture();
    let zulu = checkout(home.path(), "zulu", "git@github.com:zeta/widgets.git");
    let alpha = checkout(
        home.path(),
        "alpha checkout",
        "https://github.com/acme/widgets.git",
    );
    checkout(
        home.path(),
        "another-alpha",
        "git@github.com:acme/widgets.git",
    );
    let zulu_command = format!("cd '{}' && thirdshift pickup base main", zulu.display());
    let alpha_command = format!("cd '{}'&&thirdshift architect base main", alpha.display());
    let another_alpha = "cd ~/another-alpha && env NO_COLOR=1 thirdshift secure base main";
    fs::write(home.path().join("crontab"), format!(
        "@reboot thirdshift secure base main\n*/5 * * * * {zulu_command}\n0 2 * * * {alpha_command}\n*/10 * * * * {another_alpha}\n*/5 * * * * {zulu_command}\n"
    )).unwrap();
    let output = command(home.path())
        .args(["cronjobs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let inventory: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        inventory,
        json!({"jobs": [
        {"repository": "acme/widgets", "schedule": "0 2 * * *", "command": alpha_command},
        {"repository": "acme/widgets", "schedule": "*/10 * * * *", "command": another_alpha},
        {"repository": "zeta/widgets", "schedule": "*/5 * * * *", "command": zulu_command},
        {"repository": "zeta/widgets", "schedule": "*/5 * * * *", "command": zulu_command},
        {"repository": null, "schedule": "@reboot", "command": "thirdshift secure base main"}
    ], "pass_starts": []})
    );
    let text = command(home.path()).arg("cronjobs").output().unwrap();
    assert!(text.status.success());
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        format!(
            "acme/widgets\n0 2 * * *  {alpha_command}\n*/10 * * * *  {another_alpha}\nzeta/widgets\n*/5 * * * *  {zulu_command}\n*/5 * * * *  {zulu_command}\nOther\n@reboot  thirdshift secure base main\nNo Pass start times recorded on this host.\n"
        )
    );
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    assert_eq!(api::json(console.get("/api/v1/cronjobs")), inventory);
}

#[test]
fn cronjobs_remote_and_legacy_inventories_use_the_owning_machines_metadata() {
    let output = Command::new("python3")
        .args([
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/console/cronjobs_remote.py"
            ),
            env!("CARGO_BIN_EXE_thirdshift"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn cronjobs_keeps_directory_changing_compounds_under_other() {
    let home = fixture();
    let first = checkout(home.path(), "first", "https://github.com/acme/first.git");
    let second = checkout(home.path(), "second", "https://github.com/acme/second.git");
    let commands = [
        format!(
            "cd '{}' && cd '{}' && thirdshift pickup base main",
            first.display(),
            second.display()
        ),
        format!(
            "cd '{}' && cd ../second && thirdshift pickup base main",
            first.display()
        ),
        format!(
            "cd '{}' && pushd '{}' && thirdshift pickup base main",
            first.display(),
            second.display()
        ),
        format!(
            "cd '{}' && popd && thirdshift pickup base main",
            first.display()
        ),
    ];
    fs::write(
        home.path().join("crontab"),
        commands
            .iter()
            .map(|command| format!("@daily {command}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let output = command(home.path())
        .args(["cronjobs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let inventory: Value = serde_json::from_slice(&output.stdout).unwrap();
    let jobs = inventory["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), commands.len());
    for (job, expected) in jobs.iter().zip(commands) {
        assert_eq!(
            job,
            &json!({"repository": null, "schedule": "@daily", "command": expected})
        );
    }
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    assert_eq!(api::json(console.get("/api/v1/cronjobs")), inventory);
    let html = console.get("/cronjobs").text().unwrap();
    assert!(html.contains("<h2>Other</h2>"));
    assert!(!html.contains("<h2>acme/"));
}

#[test]
fn cronjobs_keeps_unresolved_entries_and_never_evaluates_shell_text() {
    let home = fixture();
    checkout(
        home.path(),
        "quoted checkout",
        "https://github.com/acme/widgets.git",
    );
    let elsewhere = checkout(
        home.path(),
        "elsewhere",
        "https://gitlab.com/acme/widgets.git",
    );
    let sentinel = home.path().join("executed");
    let unresolved = [
        "thirdshift pickup base main".to_string(),
        format!(
            "cd '{}/missing' && thirdshift pickup",
            home.path().display()
        ),
        format!("cd '{}' && thirdshift pickup", elsewhere.display()),
        format!("cd $(touch {}) && thirdshift pickup", sentinel.display()),
        format!("cd `touch {}` && thirdshift pickup", sentinel.display()),
        "cd $HOME/'quoted checkout' && thirdshift pickup".to_string(),
    ];
    let literal = "cd '~/quoted checkout' && thirdshift status --json%stdin";
    let display_only = "cd \"~/quoted checkout\" && thirdshift pickup base \"$BASE\"";
    fs::write(
        home.path().join("crontab"),
        format!(
            "{}\n@daily {literal}\n@hourly {display_only}\n",
            unresolved
                .iter()
                .map(|command| format!("@reboot {command}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();
    let output = command(home.path())
        .args(["cronjobs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let inventory: Value = serde_json::from_slice(&output.stdout).unwrap();
    let jobs = inventory["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 8, "{inventory}");
    assert_eq!(
        jobs[0],
        json!({"repository":"acme/widgets", "schedule":"@daily", "command":literal})
    );
    assert_eq!(
        jobs[1],
        json!({"repository":"acme/widgets", "schedule":"@hourly", "command":display_only})
    );
    for (job, expected) in jobs[2..].iter().zip(unresolved) {
        assert_eq!(
            job,
            &json!({"repository":null, "schedule":"@reboot", "command":expected})
        );
    }
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    assert_eq!(api::json(console.get("/api/v1/cronjobs")), inventory);
    let html = console.get("/cronjobs").text().unwrap();
    assert!(html.find("<h2>acme/widgets</h2>").unwrap() < html.find("<h2>Other</h2>").unwrap());
    assert!(
        !sentinel.exists(),
        "Inventory discovery must never execute cron text"
    );
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
            {"repository": null, "schedule": "*/5 * * * *", "command": "cd /srv/repo && /home/me/.cargo/bin/thirdshift pickup base main >> /tmp/cron.log 2>&1"},
            {"repository": null, "schedule": "@reboot", "command": "thirdshift secure base main # <script>alert(1)</script>"}
        ], "pass_starts": []})
    );
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    assert_eq!(api::json(console.get("/api/v1/cronjobs")), jobs);
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
            {"repository": null, "schedule": "* * * * *", "command": "/usr/local/bin/thirdshift%stdin"}
        ], "pass_starts": []})
    );
}

#[test]
fn run_scope_cannot_read_cron_inventory_through_start_options() {
    let home = fixture();
    fs::write(
        home.path().join("crontab"),
        "0 2 * * * thirdshift console token list # cron-inventory-secret\n*/5 * * * * thirdshift pickup base main\n",
    )
    .unwrap();
    let fired = chrono::Utc::now().to_rfc3339();
    let root = home.path().join(".thirdshift/logs/hidden/cron-history");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("pickup.last-fired"), &fired).unwrap();
    let reader = super::tokens::create(home.path(), "reader", &["--scopes", "read"]);
    let runner = super::tokens::create(home.path(), "runner", &["--scopes", "run"]);
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    let readable = super::tokens::request(&console, &reader, "/api/v1/cronjobs")
        .send()
        .unwrap();
    assert_eq!(readable.status(), 200);
    let readable = readable.text().unwrap();
    assert!(readable.contains("cron-inventory-secret"), "{readable}");
    assert!(readable.contains("hidden/cron-history"), "{readable}");
    let refused = super::tokens::request(&console, &runner, "/api/v1/cronjobs")
        .send()
        .unwrap();
    assert_eq!(refused.status(), 403);
    let options = super::tokens::request(&console, &runner, "/api/v1/start-options")
        .send()
        .unwrap();
    assert_eq!(options.status(), 200);
    let body = options.text().unwrap();
    assert!(!body.contains("cron-inventory-secret"), "{body}");
    assert!(!body.contains("console token list"), "{body}");
    assert!(!body.contains("hidden/cron-history"), "{body}");
    assert!(!body.contains("\"inventory\""), "{body}");
}
