//! Installation is exercised through the binary and its printed shell script.
#![cfg(target_os = "linux")]

mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use support::{copy_executable, write_executable};

struct Machine {
    root: tempfile::TempDir,
}

impl Machine {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        fs::create_dir_all(root.path().join("home/.thirdshift")).unwrap();
        let machine = Self { root };
        // Identity is real; proxy/service commands stay inside this fixture.
        std::os::unix::fs::symlink("/usr/bin/id", machine.path("bin/id")).unwrap();
        machine.configure("https://console.example.test", 9123);
        machine
    }

    fn path(&self, relative: &str) -> std::path::PathBuf {
        self.root.path().join(relative)
    }

    fn configure(&self, address: &str, port: u16) {
        fs::write(
            self.path("home/.thirdshift/config.toml"),
            format!("[console]\naddress = {address:?}\nport = {port}\nallowed_emails = [\"owner@example.test\"]\n"),
        )
        .unwrap();
    }

    fn tool(&self, name: &str, script: &str) {
        let path = self.path(&format!("bin/{name}"));
        write_executable(&path, &format!("#!/bin/sh\n{script}\n"));
    }

    fn install(&self) -> Output {
        self.install_with(&[])
    }

    fn install_with(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .args(["console", "install"])
            .args(extra)
            .current_dir(self.root.path())
            .env("HOME", self.path("home"))
            .env("PATH", self.path("bin"))
            .env("USER", "not-the-owner")
            .output()
            .unwrap()
    }

    fn script(&self) -> String {
        let output = self.install();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let script = String::from_utf8(output.stdout).unwrap();
        let path = self.path("install.sh");
        fs::write(&path, &script).unwrap();
        let syntax = Command::new("/bin/sh")
            .arg("-n")
            .arg(path)
            .output()
            .unwrap();
        assert!(
            syntax.status.success(),
            "{}",
            String::from_utf8_lossy(&syntax.stderr)
        );
        script
    }

    fn run_script(&self, script: &str, tools: &[(&str, &str)]) -> Output {
        // Run the whole printed script without touching the host's /etc or services.
        let script = script.replace("/etc/", &format!("{}/", self.path("etc").display()));
        let path = self.path("run-install.sh");
        fs::write(&path, script).unwrap();
        fs::create_dir_all(self.path("etc/systemd/system")).unwrap();
        for tool in ["cat", "mkdir", "grep"] {
            std::os::unix::fs::symlink(
                format!("/usr/bin/{tool}"),
                self.path(&format!("bin/{tool}")),
            )
            .unwrap();
        }
        for tool in [
            "systemctl",
            "caddy",
            "apache2ctl",
            "httpd",
            "a2enmod",
            "a2ensite",
            "certbot",
            "curl",
            "journalctl",
        ] {
            self.tool(tool, "printf '%s\\n' \"$0 $*\" >> actions");
        }
        // Advance fake time without waiting. A runaway poll must fail the test
        // rather than hang the suite, even if the script loses its retry bound.
        self.tool(
            "sleep",
            r#"n=0
[ ! -f waits ] || read -r n < waits
n=$((n + 1))
printf '%s\n' "$n" > waits
[ "$n" -lt 30 ]"#,
        );
        for (name, body) in tools {
            self.tool(name, body);
        }
        Command::new("/bin/sh")
            .arg(path)
            .current_dir(self.root.path())
            .env("PATH", self.path("bin"))
            .output()
            .unwrap()
    }
}

#[test]
fn inactive_console_fails_installation_with_its_journal_before_proxy_changes() {
    let machine = Machine::new();
    machine.tool("apache2ctl", "exit 0");
    let script = machine.script();
    machine.tool("getent", "exit 0");

    let output = machine.run_script(
        &script,
        &[
            ("systemctl", "[ \"$1\" != is-active ]"),
            (
                "journalctl",
                "printf 'Console failed: permission denied\\n'",
            ),
        ],
    );

    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("thirdshift-console.service"), "{output:?}");
    assert!(
        stderr.contains("Console failed: permission denied"),
        "{output:?}"
    );
    assert!(!machine.path("etc/apache2").exists());
    assert!(
        !fs::read_to_string(machine.path("actions"))
            .unwrap_or_default()
            .contains("certbot")
    );
}

#[test]
fn active_console_must_answer_on_its_configured_port_to_finish_installation() {
    for answers in [false, true] {
        let machine = Machine::new();
        machine.configure("", 9543);
        let script = machine.script();
        let probe = if answers {
            // First connection is refused; the next response is a sign-in page.
            "printf '%s\\n' \"$*\" >> probes\nif [ ! -f listening ]; then : > listening; exit 7; fi\nexit 0"
        } else {
            "printf '%s\\n' \"$*\" >> probes\nexit 7"
        };

        let output = machine.run_script(
            &script,
            &[
                ("curl", probe),
                ("journalctl", "printf 'Console has not bound its port\\n'"),
            ],
        );

        assert_eq!(output.status.success(), answers, "{output:?}");
        let probes = fs::read_to_string(machine.path("probes")).unwrap();
        assert!(probes.contains("http://127.0.0.1:9543/"), "{probes}");
        assert!(probes.lines().count() > 1, "{probes}");
        if answers {
            assert!(output.stderr.is_empty(), "{output:?}");
        } else {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("Console has not bound its port"),
                "{output:?}"
            );
        }
    }
}

#[test]
fn selinux_warning_is_only_printed_when_getenforce_reports_enforcing() {
    for mode in [
        None,
        Some("Enforcing"),
        Some("Permissive"),
        Some("Disabled"),
    ] {
        let machine = Machine::new();
        let script = machine.script();
        if let Some(mode) = mode {
            machine.tool("getenforce", &format!("printf '%s\\n' '{mode}'"));
        }

        let output = machine.run_script(&script, &[]);

        assert!(output.status.success(), "{mode:?}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        if mode == Some("Enforcing") {
            assert!(
                stderr.contains("SELinux can block the Console"),
                "{output:?}"
            );
            assert!(
                stderr.contains("https://github.com/JacobStephens2/thirdshift/issues/1078"),
                "{output:?}"
            );
        } else {
            assert!(stderr.is_empty(), "{mode:?}: {output:?}");
        }
    }
}

#[test]
fn unresolved_console_address_stops_proxy_installation_before_any_changes() {
    for proxy in ["caddy", "apache2ctl", "httpd"] {
        let machine = Machine::new();
        machine.tool(proxy, "exit 0");
        machine.tool("systemctl", "exit 1");
        machine.configure("https://missing.example.invalid", 9123);
        let script = machine.script();
        machine.tool(
            "getent",
            "printf '%s\\n' \"$*\" > lookup\nprintf 'resolver noise\\n'\nexit 2",
        );

        let output = machine.run_script(&script, &[]);

        assert!(!output.status.success(), "{proxy} installed without DNS");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "missing.example.invalid has no DNS record. Add an A or AAAA record pointing at this server, then run this script again.\n",
            "{proxy}"
        );
        assert!(output.stdout.is_empty(), "{proxy}: {output:?}");
        assert_eq!(
            fs::read_to_string(machine.path("lookup")).unwrap(),
            "hosts missing.example.invalid\n"
        );
        assert!(!machine.path("actions").exists(), "{proxy} ran a command");
        assert_eq!(
            fs::read_dir(machine.path("etc/systemd/system"))
                .unwrap()
                .count(),
            0,
            "{proxy} wrote the unit"
        );
        for directory in ["etc/caddy", "etc/apache2", "etc/httpd"] {
            assert!(!machine.path(directory).exists(), "{proxy} wrote its site");
        }
    }
}

#[test]
fn resolving_console_address_continues_proxy_installation() {
    for (proxy, service, site) in [
        ("caddy", "caddy", "etc/caddy/sites/thirdshift-console.caddy"),
        (
            "apache2ctl",
            "apache2",
            "etc/apache2/sites-available/thirdshift-console.conf",
        ),
        ("httpd", "httpd", "etc/httpd/conf.d/thirdshift-console.conf"),
    ] {
        for address in ["203.0.113.10", "2001:db8::10"] {
            let machine = Machine::new();
            machine.tool(proxy, "exit 0");
            machine.tool("systemctl", "exit 1");
            machine.configure("https://ready.example.test/", 9123);
            let script = machine.script();
            machine.tool(
                "getent",
                &format!(
                    "printf '%s\\n' \"$*\" > lookup\nprintf '{address} ready.example.test\\n'"
                ),
            );
            fs::create_dir_all(machine.path("etc/caddy")).unwrap();
            fs::write(machine.path("etc/caddy/Caddyfile"), "# Existing sites\n").unwrap();

            let output = machine.run_script(&script, &[]);

            assert!(output.status.success(), "{proxy}, {address}: {output:?}");
            assert!(output.stdout.is_empty(), "{proxy}, {address}: {output:?}");
            assert!(output.stderr.is_empty(), "{proxy}, {address}: {output:?}");
            assert_eq!(
                fs::read_to_string(machine.path("lookup")).unwrap(),
                "hosts ready.example.test\n"
            );
            assert!(
                machine
                    .path("etc/systemd/system/thirdshift-console.service")
                    .is_file()
            );
            assert!(machine.path(site).is_file());
            let actions = fs::read_to_string(machine.path("actions")).unwrap();
            assert!(
                actions.contains(&format!("systemctl restart {service}\n")),
                "{actions}"
            );
            if proxy != "caddy" {
                assert!(
                    actions.contains("certbot --apache -d ready.example.test\n"),
                    "{actions}"
                );
            }
        }
    }
}

#[test]
fn caddy_script_installs_an_owner_service_and_streaming_site_without_running_them() {
    let machine = Machine::new();
    machine.tool("caddy", "exit 0");
    // Detection may ask about running services, but must never install one.
    machine.tool("systemctl", "[ \"$1\" = is-active ] || exit 99\nexit 1");
    let script = machine.script();
    let owner = Command::new("id").arg("-un").output().unwrap();
    let owner = String::from_utf8(owner.stdout).unwrap();
    assert!(
        script.contains(&format!("User={}\n", owner.trim())),
        "{script}"
    );
    assert!(script.contains("Restart=always"), "{script}");
    assert!(
        script.contains(&format!(
            "ExecStart=\"{}\" console",
            env!("CARGO_BIN_EXE_thirdshift")
        )),
        "{script}"
    );
    assert!(
        script.contains(&format!("HOME={}", machine.path("home").display())),
        "{script}"
    );
    assert!(
        script.contains("/etc/caddy/sites/thirdshift-console.caddy"),
        "{script}"
    );
    assert!(script.contains("console.example.test {"), "{script}");
    assert!(script.contains("reverse_proxy 127.0.0.1:9123"), "{script}");
    assert!(script.contains("flush_interval -1"), "{script}");
    assert!(
        script.contains("systemctl enable thirdshift-console.service"),
        "{script}"
    );
    assert!(script.contains("systemctl restart caddy"), "{script}");
    assert!(!script.contains("systemctl reload"), "{script}");
    assert!(!machine.path("home/.thirdshift/console").exists());
}

#[test]
fn apache_script_preserves_host_and_protocol_and_certifies_the_streaming_site() {
    let machine = Machine::new();
    machine.tool("apache2ctl", "exit 0");
    let script = machine.script();
    for required in [
        "/etc/apache2/sites-available/thirdshift-console.conf",
        "<VirtualHost *:80>",
        "ServerName console.example.test",
        "ProxyPreserveHost On",
        "RequestHeader set X-Forwarded-Proto \"expr=%{REQUEST_SCHEME}\"",
        "ProxyPass / http://127.0.0.1:9123/ flushpackets=on",
        "ProxyPassReverse / http://127.0.0.1:9123/",
        "SetEnvIf Request_URI \"^/(events$|logs/[^/]+/stream/)\" no-gzip=1",
        "a2enmod proxy proxy_http headers ssl setenvif",
        "a2ensite thirdshift-console.conf",
        "certbot --apache -d console.example.test",
        "systemctl restart apache2",
    ] {
        assert!(script.contains(required), "missing {required}:\n{script}");
    }
    let enabled = script
        .find("systemctl enable thirdshift-console.service")
        .unwrap();
    assert!(enabled < script.find("systemctl restart apache2").unwrap());
    assert!(!script.contains("/etc/caddy"));
}

#[test]
fn without_a_proxy_the_service_and_ssh_tunnel_work_without_an_address_or_accounts() {
    let machine = Machine::new();
    fs::write(
        machine.path("home/.thirdshift/config.toml"),
        "[console]\nport = 9123\n",
    )
    .unwrap();
    let script = machine.script();
    assert!(
        script.contains("/etc/systemd/system/thirdshift-console.service"),
        "{script}"
    );
    assert!(script.contains("console --no-sign-in\n"), "{script}");
    assert!(script.contains("ssh -N -L 9123:127.0.0.1:9123"), "{script}");
    assert!(script.contains("http://localhost:9123"), "{script}");
    assert!(!script.contains("certbot"), "{script}");
    assert!(!script.contains("getent"), "{script}");
    assert!(!script.contains("/etc/caddy"), "{script}");
    assert!(!script.contains("/etc/apache2"), "{script}");
}

#[test]
fn a_running_apache_wins_over_an_installed_caddy() {
    let machine = Machine::new();
    machine.tool("caddy", "exit 0");
    machine.tool("apache2ctl", "exit 0");
    machine.tool(
        "systemctl",
        "[ \"$1\" = is-active ] && [ \"$3\" = apache2 ]",
    );
    let script = machine.script();
    assert!(script.contains("systemctl restart apache2"), "{script}");
    assert!(!script.contains("/etc/caddy"), "{script}");
}

#[test]
fn reinstall_restarts_the_console_before_exposing_it_through_the_proxy() {
    let machine = Machine::new();
    machine.tool("caddy", "exit 0");
    let script = machine.script();
    // enable --now leaves an already running tunnel service in --no-sign-in
    // mode. Replacing its unit must apply the new command before public access.
    let restart = script
        .find("systemctl restart thirdshift-console.service")
        .expect("the replaced service must restart even if it was already running");
    assert!(script.find("systemctl daemon-reload").unwrap() < restart);
    assert!(restart < script.find("systemctl restart caddy").unwrap());
}

#[test]
fn reinstall_restarts_console_before_certbot_can_activate_the_public_site() {
    let machine = Machine::new();
    machine.tool("apache2ctl", "exit 0");
    let script = machine.script();
    assert!(
        script
            .find("systemctl restart thirdshift-console.service")
            .unwrap()
            < script.find("certbot --apache").unwrap(),
        "certbot can reload Apache, so the Console must already require sign-in:\n{script}"
    );
}

#[test]
fn rocky_apache_uses_httpd_and_its_conf_directory() {
    let machine = Machine::new();
    machine.tool("httpd", "exit 0");
    let script = machine.script();
    assert!(
        script.contains("/etc/httpd/conf.d/thirdshift-console.conf"),
        "{script}"
    );
    assert!(script.contains("httpd -t"), "{script}");
    assert!(
        script.contains("certbot --apache -d console.example.test"),
        "{script}"
    );
    assert!(script.contains("systemctl restart httpd"), "{script}");
    assert!(!script.contains("a2en"), "{script}");
}

#[test]
fn install_rejects_unusable_public_settings_without_printing_a_partial_script() {
    let machine = Machine::new();
    machine.tool("caddy", "exit 0");
    for address in [
        "",
        "http://console.example.test",
        "https://console.example.test/path",
        "bad;echo injected",
        "https://console.example.test:8443",
    ] {
        machine.configure(address, 9123);
        let output = machine.install();
        assert!(!output.status.success(), "accepted {address}");
        assert!(output.stdout.is_empty(), "partial script for {address}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("console.address"));
    }
    machine.configure("console.example.test", 0);
    let output = machine.install();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("console.port"));
    fs::write(
        machine.path("home/.thirdshift/config.toml"),
        "[console]\naddress = \"console.example.test\"\n",
    )
    .unwrap();
    let output = machine.install();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("console.allowed_emails"));
}

#[test]
fn unit_preserves_literal_paths_with_shell_and_systemd_metacharacters() {
    let machine = Machine::new();
    let home = machine.path("home space%$\"'\\");
    fs::rename(machine.path("home"), &home).unwrap();
    let binary = machine.path("thirdshift space%$binary");
    copy_executable(Path::new(env!("CARGO_BIN_EXE_thirdshift")), &binary);
    let output = Command::new(&binary)
        .args(["console", "install"])
        .env("HOME", &home)
        .env("PATH", machine.path("bin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let script = String::from_utf8(output.stdout).unwrap();
    assert!(script.contains("home space%%$\\\"'\\\\"), "{script}");
    assert!(
        script.contains("thirdshift space%%$binary\" console"),
        "{script}"
    );
    let unit = script
        .split_once("<<'THIRDSHIFT_UNIT'\n")
        .unwrap()
        .1
        .split_once("THIRDSHIFT_UNIT\n")
        .unwrap()
        .0;
    let unit_path = machine.path("thirdshift-console.service");
    fs::write(&unit_path, unit).unwrap();
    if std::path::Path::new("/usr/bin/systemd-analyze").exists() {
        let output = Command::new("/usr/bin/systemd-analyze")
            .args(["verify", "--man=no"])
            .arg(unit_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn output_saves_the_printed_script_and_names_the_sudo_command() {
    let machine = Machine::new();
    machine.tool("caddy", "exit 0");
    let printed = machine.script();
    let output = machine.install_with(&["--output", "install-console.sh"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(machine.path("install-console.sh")).unwrap(),
        printed
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("sudo sh install-console.sh"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A second install replaces the saved script rather than refusing.
    fs::write(machine.path("install-console.sh"), "stale").unwrap();
    assert!(
        machine
            .install_with(&["--output", "install-console.sh"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(machine.path("install-console.sh")).unwrap(),
        printed
    );
}

#[test]
fn output_saves_nothing_when_the_settings_are_unusable() {
    let machine = Machine::new();
    machine.tool("caddy", "exit 0");
    machine.configure("", 9123);
    let output = machine.install_with(&["--output", "install-console.sh"]);
    assert!(!output.status.success());
    assert!(!machine.path("install-console.sh").exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("console.address"));
}

#[test]
fn output_needs_a_file_name() {
    let machine = Machine::new();
    for extra in [&["--output"][..], &["--output", "--output"], &["--json"]] {
        let output = machine.install_with(extra);
        assert!(!output.status.success(), "accepted {extra:?}");
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("install [--output <file>]"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn output_names_a_copyable_sudo_command_for_a_spaced_path() {
    let machine = Machine::new();
    machine.tool("caddy", "exit 0");
    let output = machine.install_with(&["--output", "my script.sh"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("sudo sh 'my script.sh'"), "{stderr}");
    // The saved script must not tell its reader to run some other file.
    let saved = fs::read_to_string(machine.path("my script.sh")).unwrap();
    assert!(!saved.contains("install-console.sh"), "{saved}");
}
