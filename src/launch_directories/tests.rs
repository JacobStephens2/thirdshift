use super::*;
use std::fs;
use std::os::unix::fs::symlink;

fn checkout(home: &Path, name: &str, origin: &str) -> PathBuf {
    let path = home.join(name);
    fs::create_dir_all(&path).unwrap();
    let git = Git::new(&path);
    git.run(&["init", "-q"]).unwrap();
    git.run(&["remote", "add", "origin", origin]).unwrap();
    path.canonicalize().unwrap()
}

fn config(home: &Path) -> config::UserConfig {
    config::UserConfig::parse("", &home.join("config.toml"), home).unwrap()
}

#[test]
fn captured_projections_preserve_entry_associations_order_and_literal_logs() {
    let home = tempfile::tempdir().unwrap();
    let a = checkout(home.path(), "a", "https://github.com/acme/z.git");
    let b = checkout(home.path(), "b", "https://github.com/acme/a.git");
    let alias = home.path().join("alias");
    symlink(&a, &alias).unwrap();
    let mut config = config(home.path());
    config.launch_directories = vec![alias.clone(), a.clone()];
    let text = "@daily cd ~/alias && thirdshift pickup\n@hourly cd ~/a && thirdshift secure >> later.log\n@daily cd ~/b && thirdshift architect > first.log\n@daily cd ~/b && thirdshift architect > first.log\n";
    let captured = Captured::new(&config, home.path(), Ok(cronjobs::parse(text)));
    let entries = captured.entries().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].directory.as_ref().unwrap().path, a);
    assert!(entries[0].launch.as_ref().unwrap().command.is_some());
    assert_eq!(
        entries[1].launch.as_ref().unwrap().output_log,
        Some(a.join("later.log"))
    );
    let directories = &captured.directories().unwrap().directories;
    assert_eq!(
        directories.iter().map(|d| &d.path).collect::<Vec<_>>(),
        [&a, &b]
    );
    assert_eq!(directories[0].output_log, None);
    assert_eq!(directories[1].output_log, Some(b.join("first.log")));
    let jobs = &captured.cronjobs().unwrap().jobs;
    assert_eq!(
        jobs.iter()
            .map(|job| job.schedule.as_str())
            .collect::<Vec<_>>(),
        ["@daily", "@daily", "@daily", "@hourly"]
    );
    assert_eq!(jobs[0].repository.as_deref(), Some("acme/a"));
    assert_eq!(jobs[2].repository.as_deref(), Some("acme/z"));

    let text = "@daily cd ~/alias && thirdshift pickup >> output.log\n@daily cd ~/a && thirdshift pickup\n";
    let captured = Captured::new(&config, home.path(), Ok(cronjobs::parse(text)));
    assert_eq!(
        captured.directories().unwrap().directories[0].output_log,
        Some(alias.join("output.log"))
    );
    // A captured checkout remains an observation; the next operation sees edits.
    Git::new(&a)
        .run(&[
            "remote",
            "set-url",
            "origin",
            "https://github.com/acme/changed.git",
        ])
        .unwrap();
    assert_eq!(
        captured.cronjobs().unwrap().jobs[0].repository.as_deref(),
        Some("acme/z")
    );
    let fresh = Captured::new(&config, home.path(), Ok(cronjobs::parse(text)));
    assert_eq!(
        fresh.cronjobs().unwrap().jobs[0].repository.as_deref(),
        Some("acme/changed")
    );
}

#[test]
fn unavailable_checkout_observations_are_shared_with_configured_validation() {
    let home = tempfile::tempdir().unwrap();
    let invalid = checkout(
        home.path(),
        "invalid",
        "https://gitlab.com/acme/widgets.git",
    );
    let missing = home.path().join("missing");
    symlink(&invalid, home.path().join("alias")).unwrap();
    let mut config = config(home.path());
    let text =
        "@daily cd ~/missing && thirdshift pickup\n@daily cd ~/invalid && thirdshift pickup\n";
    config.launch_directories = vec![missing.clone()];
    let captured = Captured::new(&config, home.path(), Ok(cronjobs::parse(text)));
    checkout(
        home.path(),
        "missing",
        "https://github.com/acme/widgets.git",
    );
    assert!(
        captured
            .cronjobs()
            .unwrap()
            .jobs
            .iter()
            .all(|job| job.repository.is_none())
    );
    assert!(
        captured
            .directories()
            .err()
            .unwrap()
            .to_string()
            .contains("invalid configured Launch directory")
    );

    config.launch_directories = vec![home.path().join("alias")];
    let captured = Captured::new(&config, home.path(), Ok(cronjobs::parse(text)));
    Git::new(&invalid)
        .run(&[
            "remote",
            "set-url",
            "origin",
            "https://github.com/acme/widgets.git",
        ])
        .unwrap();
    assert!(
        captured
            .directories()
            .err()
            .unwrap()
            .to_string()
            .contains("not on GitHub")
    );
    assert_eq!(captured.cronjobs().unwrap().jobs.len(), 2);
    let fresh = Captured::new(&config, home.path(), Ok(cronjobs::parse(text)));
    assert_eq!(fresh.directories().unwrap().directories.len(), 2);
}

#[test]
fn projection_errors_preserve_strict_cron_and_missing_tool_discovery() {
    let home = tempfile::tempdir().unwrap();
    let path = checkout(home.path(), "repo", "https://github.com/acme/widgets.git");
    let mut config = config(home.path());
    config.launch_directories = vec![path];
    for kind in [
        std::io::ErrorKind::NotFound,
        std::io::ErrorKind::PermissionDenied,
    ] {
        let error =
            Err(std::io::Error::new(kind, "fixture crontab error")).context("can't read cron jobs");
        let captured = Captured::new(&config, home.path(), error);
        assert!(captured.cronjobs().is_err());
        assert_eq!(
            captured.directories().is_ok(),
            kind == std::io::ErrorKind::NotFound
        );
        if kind == std::io::ErrorKind::NotFound {
            assert!(captured.entries().unwrap().is_empty());
            assert_eq!(captured.directories().unwrap().directories.len(), 1);
        }
    }
    let captured = Captured::new(&config, home.path(), Ok(Vec::new()));
    assert!(captured.cronjobs().unwrap().jobs.is_empty());
    assert_eq!(captured.directories().unwrap().directories.len(), 1);
}
