//! Installation is exercised through the binary and its printed shell script.
#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};

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
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn install(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_thirdshift"))
            .args(["console", "install"])
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
    fs::copy(env!("CARGO_BIN_EXE_thirdshift"), &binary).unwrap();
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
