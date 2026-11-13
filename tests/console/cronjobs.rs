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

fn changing_crontab(home: &Path) {
    checkout(home, "first", "https://github.com/acme/first.git");
    checkout(home, "second", "https://github.com/acme/second.git");
    support::write_executable(
        &home.join("bin/crontab"),
        "#!/bin/sh\nn=0\n[ ! -f \"$HOME/reads\" ] || n=$(cat \"$HOME/reads\")\nn=$((n+1))\necho $n > \"$HOME/reads\"\nif [ $n = 1 ]; then repo=first; else repo=second; fi\necho \"@daily cd ~/$repo && thirdshift pickup\"\n",
    );
}

#[test]
fn start_options_share_one_crontab_capture_and_refresh_on_the_next_request() {
    let home = fixture();
    changing_crontab(home.path());
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    for (read, repo) in [(1, "first"), (2, "second")] {
        let options = api::json(console.get("/api/v1/start-options"));
        assert_eq!(
            fs::read_to_string(home.path().join("reads"))
                .unwrap()
                .trim(),
            read.to_string()
        );
        assert_eq!(
            options["form"]["repositories"][0]["repository"],
            format!("acme/{repo}")
        );
        let request: Value =
            serde_json::from_str(options["passes"][0]["request"].as_str().unwrap()).unwrap();
        assert_eq!(request["repository"], format!("acme/{repo}"));
    }
}

#[test]
fn local_cron_page_shares_its_capture_with_run_now() {
    let home = fixture();
    changing_crontab(home.path());
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    for (read, repo) in [(1, "first"), (2, "second")] {
        let html = console.get("/cronjobs").text().unwrap();
        let document = scraper::Html::parse_document(&html);
        let commands = scraper::Selector::parse(".cron-command").unwrap();
        let buttons = scraper::Selector::parse("[data-start-request]").unwrap();
        assert_eq!(
            document
                .select(&commands)
                .next()
                .unwrap()
                .text()
                .collect::<String>(),
            format!("cd ~/{repo} && thirdshift pickup")
        );
        let request: Value = serde_json::from_str(
            document
                .select(&buttons)
                .next()
                .unwrap()
                .attr("data-start-request")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(request["repository"], format!("acme/{repo}"));
        assert_eq!(
            fs::read_to_string(home.path().join("reads"))
                .unwrap()
                .trim(),
            read.to_string()
        );
    }
}

#[test]
fn cron_page_keeps_repository_order_while_start_options_keep_crontab_order() {
    let home = fixture();
    checkout(home.path(), "a", "https://github.com/acme/a.git");
    checkout(home.path(), "b", "https://github.com/acme/b.git");
    fs::write(home.path().join("crontab"), "@daily cd ~/b && thirdshift pickup\n@hourly cd ~/a && thirdshift architect\n@daily cd ~/b && thirdshift secure\n").unwrap();
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    let options = api::json(console.get("/api/v1/start-options"));
    assert_eq!(
        options["passes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pass| pass["command"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "cd ~/b && thirdshift pickup",
            "cd ~/a && thirdshift architect",
            "cd ~/b && thirdshift secure"
        ]
    );
    let html = console.get("/cronjobs").text().unwrap();
    let document = scraper::Html::parse_document(&html);
    let buttons = scraper::Selector::parse("[data-start-request]").unwrap();
    let commands: Vec<_> = document
        .select(&buttons)
        .map(|button| {
            let request: Value =
                serde_json::from_str(button.attr("data-start-request").unwrap()).unwrap();
            request["command"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(commands, ["architect", "pickup", "secure"]);
}

#[test]
fn aliases_and_duplicate_entries_share_successful_and_failed_origin_observations() {
    let home = fixture();
    let valid = checkout(home.path(), "valid", "https://github.com/acme/widgets.git");
    let invalid = checkout(
        home.path(),
        "invalid",
        "https://gitlab.com/acme/widgets.git",
    );
    std::os::unix::fs::symlink(&valid, home.path().join("alias")).unwrap();
    std::os::unix::fs::symlink(&invalid, home.path().join("bad-alias")).unwrap();
    fs::write(
        home.path().join(".thirdshift/config.toml"),
        "launch_directories = ['~/alias', '~/valid']\n[console]\nport = 0\n",
    )
    .unwrap();
    fs::write(home.path().join("crontab"), "@daily cd ~/alias && thirdshift pickup\n@daily cd ~/valid && thirdshift pickup\n@hourly cd ~/alias && thirdshift secure\n@daily cd ~/invalid && thirdshift pickup\n@daily cd ~/bad-alias && thirdshift pickup\n").unwrap();
    support::write_executable(
        &home.path().join("bin/git"),
        "#!/bin/sh\nif [ \"$1 $2 $3\" = 'config --get remote.origin.url' ]; then echo \"$PWD\" >> \"$HOME/origin-reads\"; fi\nexec /usr/bin/git \"$@\"\n",
    );
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    let options = api::json(console.get("/api/v1/start-options"));
    assert_eq!(options["form"]["repositories"].as_array().unwrap().len(), 1);
    assert_eq!(options["passes"].as_array().unwrap().len(), 3);
    let reads = fs::read_to_string(home.path().join("origin-reads")).unwrap();
    assert_eq!(
        reads.lines().collect::<Vec<_>>(),
        [valid.to_str().unwrap(), invalid.to_str().unwrap()]
    );
}

#[test]
fn invalid_configured_directory_keeps_cron_inventory_and_reports_start_error() {
    let home = fixture();
    checkout(home.path(), "repo", "https://github.com/acme/widgets.git");
    fs::write(
        home.path().join("crontab"),
        "@daily cd ~/repo && thirdshift pickup\n",
    )
    .unwrap();
    fs::write(
        home.path().join(".thirdshift/config.toml"),
        "launch_directories = ['~/missing']\n[console]\nport = 0\n",
    )
    .unwrap();
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    assert_eq!(
        api::json(console.get("/api/v1/cronjobs"))["jobs"][0]["repository"],
        "acme/widgets"
    );
    assert_eq!(console.get("/api/v1/launch-directories").status(), 422);
    assert_eq!(console.get("/api/v1/start-options").status(), 422);
    let page = console.get("/cronjobs");
    assert_eq!(page.status(), 200);
    let html = page.text().unwrap();
    assert!(html.contains("acme/widgets"), "{html}");
    assert!(
        html.contains("Could not load startable Passes: invalid configured Launch directory"),
        "{html}"
    );
    assert!(
        console
            .get("/")
            .text()
            .unwrap()
            .contains("invalid configured Launch directory")
    );
}

#[test]
fn owning_machine_projections_keep_crontab_error_rules() {
    let home = fixture();
    checkout(home.path(), "repo", "https://github.com/acme/widgets.git");
    fs::write(
        home.path().join(".thirdshift/config.toml"),
        "launch_directories = ['~/repo']\n[console]\nport = 0\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("/usr/bin/git", home.path().join("bin/git")).unwrap();
    let mut start = command(home.path());
    start
        .env("PATH", home.path().join("bin"))
        .args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    for (script, strict, discovery) in [
        (Some("#!/bin/sh\nexit 0\n"), 200, 200),
        (
            Some("#!/bin/sh\necho 'crontab: no crontab for owner' >&2\nexit 1\n"),
            200,
            200,
        ),
        (None, 422, 200),
        (
            Some("#!/bin/sh\necho 'crontab: permission denied' >&2\nexit 1\n"),
            422,
            422,
        ),
        (Some("#!/bin/sh\nprintf '\\377'\n"), 422, 422),
    ] {
        let executable = home.path().join("bin/crontab");
        if let Some(script) = script {
            support::write_executable(&executable, script);
        } else {
            fs::remove_file(&executable).unwrap();
        }
        for path in ["/api/v1/cronjobs", "/cronjobs"] {
            let status = if path == "/cronjobs" && strict == 422 {
                500
            } else {
                strict
            };
            assert_eq!(console.get(path).status(), status, "{script:?}: {path}");
        }
        for path in ["/api/v1/launch-directories", "/api/v1/start-options"] {
            let response = console.get(path);
            assert_eq!(response.status(), discovery, "{script:?}: {path}");
            if discovery == 200 {
                let value = api::json(response);
                if path.ends_with("start-options") {
                    assert_eq!(
                        value["form"]["repositories"][0]["repository"],
                        "acme/widgets"
                    );
                    assert!(value["passes"].as_array().unwrap().is_empty());
                } else {
                    assert_eq!(value["directories"][0]["repository"], "acme/widgets");
                }
            }
        }
        for (args, expected) in [
            (vec!["cronjobs", "--json"], strict),
            (vec!["launch-directories", "--json"], discovery),
        ] {
            let output = command(home.path())
                .env("PATH", home.path().join("bin"))
                .args(args)
                .output()
                .unwrap();
            assert_eq!(
                output.status.success(),
                expected == 200,
                "{script:?}: {output:?}"
            );
        }
    }
}

#[test]
fn cron_job_projections_agree_without_conflating_kind_directory_and_start() {
    let home = fixture();
    let repo = checkout(home.path(), "repo", "https://github.com/acme/widgets.git");
    checkout(
        home.path(),
        "elsewhere",
        "https://gitlab.com/acme/widgets.git",
    );
    fs::write(home.path().join("crontab"), "").unwrap();
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    for (text, kind, resolved, offered) in [
        ("thirdshift pickup", Some("Pickup run"), false, false),
        (
            "cd ~/repo && thirdshift pickup",
            Some("Pickup run"),
            true,
            true,
        ),
        ("cd ~/repo && env X=1 thirdshift pickup", None, true, false),
        (
            "cd ~/repo && /usr/bin/env X=1 thirdshift pickup",
            None,
            true,
            false,
        ),
        ("cd ~/repo && thirdshift status --json", None, true, false),
        (
            "cd ~/repo && thirdshift pickup --unknown",
            None,
            true,
            false,
        ),
        (
            "cd ~/repo && thirdshift pickup base '$BASE'",
            None,
            true,
            false,
        ),
        (
            "cd ~/repo && thirdshift pickup base '`branch`'",
            None,
            true,
            false,
        ),
        (
            "cd ~/missing && thirdshift pickup",
            Some("Pickup run"),
            false,
            false,
        ),
        (
            "cd ~/elsewhere && thirdshift pickup",
            Some("Pickup run"),
            false,
            false,
        ),
        (
            "cd ~/repo && command -p -- cd /elsewhere && thirdshift pickup",
            None,
            false,
            false,
        ),
        (
            "cd ~/repo && echo ready && thirdshift pickup && echo done",
            None,
            true,
            true,
        ),
        (
            "cd ~/repo && thirdshift pickup && echo done",
            None,
            true,
            true,
        ),
        (
            "cd ~/repo && thirdshift fix https://github.com/acme/widgets/pull/7",
            Some("Fix run"),
            true,
            false,
        ),
        (
            "cd ~/repo && thirdshift review https://github.com/acme/widgets/pull/7",
            Some("Review run"),
            true,
            false,
        ),
        (
            "cd ~/repo && thirdshift triage",
            Some("Triage run"),
            true,
            false,
        ),
        ("cd '~/repo && thirdshift pickup", None, false, false),
    ] {
        fs::write(home.path().join("crontab"), format!("@daily {text}\n")).unwrap();
        let inventory = command(home.path())
            .args(["cronjobs", "--json"])
            .output()
            .unwrap();
        assert!(inventory.status.success(), "{inventory:?}");
        let inventory: Value = serde_json::from_slice(&inventory.stdout).unwrap();
        assert_eq!(
            inventory["jobs"],
            json!([{
                "schedule": "@daily", "command": text, "command_kind": kind,
                "repository": resolved.then_some("acme/widgets"),
            }]),
            "{text}"
        );
        assert_eq!(
            api::json(console.get("/api/v1/cronjobs")),
            inventory,
            "{text}"
        );
        let directories = command(home.path())
            .args(["launch-directories", "--json"])
            .output()
            .unwrap();
        assert!(directories.status.success(), "{directories:?}");
        let directories: Value = serde_json::from_slice(&directories.stdout).unwrap();
        let expected = if resolved {
            json!({"directories": [{"repository": "acme/widgets", "path": repo}]})
        } else {
            json!({"directories": []})
        };
        assert_eq!(directories, expected, "{text}");
        assert_eq!(
            api::json(console.get("/api/v1/launch-directories")),
            expected,
            "{text}"
        );
        let options = api::json(console.get("/api/v1/start-options"));
        let passes = options["passes"].as_array().unwrap();
        assert_eq!(passes.len(), usize::from(offered), "{text}: {options}");
        if offered {
            assert_eq!(passes[0]["command"], text);
            let request: Value =
                serde_json::from_str(passes[0]["request"].as_str().unwrap()).unwrap();
            assert_eq!(request["command"], "pickup");
            assert_eq!(request["repository"], "acme/widgets");
        }
    }
}

#[test]
fn cronjobs_classifies_literal_commands_without_executing_them() {
    let home = fixture();
    let repo = checkout(
        home.path(),
        "widgets",
        "https://github.com/acme/widgets.git",
    );
    let examples = [
        ("pickup base main", Some("Pickup run")),
        ("secure base main", Some("Security run")),
        ("architect base main", Some("Architect run")),
        ("triage", Some("Triage run")),
        ("fix", Some("Fix run")),
        ("review --author acme", Some("Review run")),
        ("process inspect", Some("Custom process")),
        ("release 1.2.3 --no-email", Some("Release run")),
        ("https://github.com/acme/widgets/issues/7", None),
        ("pickup > cron.log && thirdshift secure", None),
        ("status --json", None),
        ("pickup base \"$BASE\"", None),
        ("unknown-command", None),
    ];
    fs::write(
        home.path().join("crontab"),
        examples
            .iter()
            .map(|(args, _)| {
                format!(
                    "@daily cd '{}' && thirdshift {args} >> cron.log 2>&1\n",
                    repo.display()
                )
            })
            .collect::<String>(),
    )
    .unwrap();
    let output = command(home.path())
        .args(["cronjobs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let inventory: Value = serde_json::from_slice(&output.stdout).unwrap();
    for (job, (_, expected)) in inventory["jobs"].as_array().unwrap().iter().zip(examples) {
        assert_eq!(job.get("command_kind"), Some(&json!(expected)), "{job}");
    }
    let mut start = command(home.path());
    start.args(["console", "--no-sign-in"]);
    let console = Console::start_command(start);
    assert_eq!(api::json(console.get("/api/v1/cronjobs")), inventory);
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
        {"repository": "acme/widgets", "command_kind":"Architect run", "schedule": "0 2 * * *", "command": alpha_command},
        {"repository": "acme/widgets", "command_kind":null, "schedule": "*/10 * * * *", "command": another_alpha},
        {"repository": "zeta/widgets", "command_kind":"Pickup run", "schedule": "*/5 * * * *", "command": zulu_command},
        {"repository": "zeta/widgets", "command_kind":"Pickup run", "schedule": "*/5 * * * *", "command": zulu_command},
        {"repository": null, "command_kind":"Security run", "schedule": "@reboot", "command": "thirdshift secure base main"}
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
fn cronjobs_fleet_combines_inventories_and_isolates_failures() {
    let output = Command::new("python3")
        .args([
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/console/cronjobs_fleet.py"
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
    let commands: Vec<_> = [
        format!("cd '{}'", second.display()),
        "cd ../second".into(),
        format!("pushd '{}'", second.display()),
        "popd".into(),
        format!("command cd '{}'", second.display()),
        "command -p -- cd ../second".into(),
        "builtin cd ../second".into(),
        "CDPATH=.. cd second".into(),
        "! cd ../second".into(),
        "eval 'cd ../second'".into(),
        "source ../change-directory.sh".into(),
        ". ../change-directory.sh".into(),
    ]
    .iter()
    .map(|change| {
        format!(
            "cd '{}' && {change} && thirdshift pickup base main",
            first.display()
        )
    })
    .collect();
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
            &json!({"repository": null, "command_kind":null, "schedule": "@daily", "command": expected})
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
        json!({"repository":"acme/widgets", "command_kind":null, "schedule":"@daily", "command":literal})
    );
    assert_eq!(
        jobs[1],
        json!({"repository":"acme/widgets", "command_kind":null, "schedule":"@hourly", "command":display_only})
    );
    let kinds = [
        Some("Pickup run"),
        Some("Pickup run"),
        Some("Pickup run"),
        None,
        None,
        None,
    ];
    for ((job, expected), kind) in jobs[2..].iter().zip(unresolved).zip(kinds) {
        assert_eq!(
            job,
            &json!({"repository":null, "command_kind":kind, "schedule":"@reboot", "command":expected})
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
fn cronjobs_does_not_print_an_origin_newline_as_a_schedule_line() {
    let home = fixture();
    let checkout = checkout(
        home.path(),
        "checkout",
        "https://github.com/acme/widgets\n@reboot echo forged.git",
    );
    let command_line = format!("cd '{}' && thirdshift pickup base main", checkout.display());
    fs::write(
        home.path().join("crontab"),
        format!("@daily {command_line}\n"),
    )
    .unwrap();
    let text = command(home.path()).arg("cronjobs").output().unwrap();
    assert!(text.status.success(), "{text:?}");
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains(&command_line), "{text}");
    assert!(
        !text.contains("echo forged"),
        "origin text was printed as a schedule line:\n{text}"
    );
    let output = command(home.path())
        .args(["cronjobs", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let inventory: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        inventory["jobs"][0]["repository"],
        Value::Null,
        "{inventory}"
    );
    assert_eq!(inventory["jobs"][0]["command"], command_line);
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
            {"repository": null, "command_kind":"Pickup run", "schedule": "*/5 * * * *", "command": "cd /srv/repo && /home/me/.cargo/bin/thirdshift pickup base main >> /tmp/cron.log 2>&1"},
            {"repository": null, "command_kind":"Security run", "schedule": "@reboot", "command": "thirdshift secure base main # <script>alert(1)</script>"}
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
            {"repository": null, "command_kind":null, "schedule": "* * * * *", "command": "/usr/local/bin/thirdshift%stdin"}
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
