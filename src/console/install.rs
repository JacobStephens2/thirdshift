//! Only this command knows about systemd and the machine's proxy layout.
//! It reads local settings and prints or saves a script; it never installs anything.
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::args::shell_command;
use crate::config::{self, UserConfig};

enum Proxy {
    Caddy,
    Apache(ApacheLayout),
}

enum ApacheLayout {
    Debian,
    RedHat,
}

impl Proxy {
    fn detect() -> Option<Self> {
        // Prefer a running server when both packages are installed. On a
        // machine without a running systemd manager, fall back to PATH.
        const CANDIDATES: [(&str, &str, Proxy); 3] = [
            ("caddy", "caddy", Proxy::Caddy),
            ("apache2", "apache2ctl", Proxy::Apache(ApacheLayout::Debian)),
            ("httpd", "httpd", Proxy::Apache(ApacheLayout::RedHat)),
        ];
        for (service, _, proxy) in CANDIDATES {
            if Command::new("systemctl")
                .args(["is-active", "--quiet", service])
                .output()
                .is_ok_and(|output| output.status.success())
            {
                return Some(proxy);
            }
        }
        CANDIDATES
            .into_iter()
            .find(|(_, executable, _)| available(executable))
            .map(|(_, _, proxy)| proxy)
    }
}

pub fn install() -> Result<String> {
    ensure!(
        identity("-u")? != "0",
        "run thirdshift console install as the owner, without sudo; only the printed script needs sudo"
    );
    let owner = identity("-un")?;
    ensure!(
        owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
        "unsupported owner name"
    );
    let config = UserConfig::load()?;
    ensure!(
        config.console.port != 0,
        "console install needs a fixed, nonzero console.port"
    );
    let home = config::home()?;
    ensure!(home.is_absolute(), "HOME must be an absolute path");
    let binary = std::env::current_exe().context("can't locate thirdshift")?;
    let proxy = Proxy::detect();
    let mode = if proxy.is_none() { " --no-sign-in" } else { "" };
    let environment = unit_word(&format!("HOME={}", path_text(&home)?))?;
    let executable = unit_word(path_text(&binary)?)?;
    let mut preflight = String::new();
    let (site, finish) = if let Some(proxy) = proxy {
        let host = config.console.hostname()?;
        ensure!(
            !config.console.allowed_emails.is_empty(),
            "set console.allowed_emails before installing a public Console"
        );
        preflight = format!(
            r#"if ! getent hosts '{host}' >/dev/null 2>&1; then
    printf '%s\n' '{host} has no DNS record. Add an A or AAAA record pointing at this server, then run this script again.' >&2
    exit 1
fi

"#
        );
        match proxy {
            Proxy::Caddy => (
                caddy(host, config.console.port),
                "systemctl restart caddy\n".into(),
            ),
            Proxy::Apache(layout) => apache(host, config.console.port, layout),
        }
    } else {
        (
            String::new(),
            format!(
                "# No Caddy or Apache found. From your own computer, replace SERVER with this machine's SSH host:\n# ssh -N -L {port}:127.0.0.1:{port} {owner}@SERVER\n# Then open http://localhost:{port} (the service is localhost-only, without sign-in).\n",
                port = config.console.port
            ),
        )
    };
    Ok(format!(
        r#"#!/bin/sh
# Read this script, then run it with sudo sh.
set -eu

{preflight}if command -v getenforce >/dev/null 2>&1 && [ "$(getenforce 2>/dev/null)" = Enforcing ]; then
    printf '%s\n' 'Warning: SELinux can block the Console. See https://github.com/JacobStephens2/thirdshift/issues/1078' >&2
fi

if ! command -v curl >/dev/null 2>&1; then
    printf '%s\n' 'Install curl before running this script; it checks the Console localhost port.' >&2
    exit 1
fi

console_failed() {{
    printf '%s\n' 'thirdshift-console.service did not become active and answer on 127.0.0.1:{port}. Last journal lines:' >&2
    journalctl -u thirdshift-console.service -n 50 --no-pager >&2 || true
    exit 1
}}

cat > /etc/systemd/system/thirdshift-console.service <<'THIRDSHIFT_UNIT'
[Unit]
Description=thirdshift Console
After=network.target

[Service]
User={owner}
Environment={environment}
ExecStart={executable} console{mode}
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
THIRDSHIFT_UNIT

systemctl daemon-reload
systemctl enable thirdshift-console.service
systemctl restart thirdshift-console.service || console_failed

attempt=1
# Any HTTP response counts, including the Console sign-in response.
until systemctl is-active --quiet thirdshift-console.service &&
    curl --disable --noproxy '*' --silent --output /dev/null --connect-timeout 1 --max-time 1 http://127.0.0.1:{port}/; do
    [ "$attempt" -lt 15 ] || console_failed
    attempt=$((attempt + 1))
    sleep 1
done

{site}{finish}"#,
        port = config.console.port
    ))
}

/// Save the script for the owner to read; nothing is written when it can't be made.
pub fn install_to(path: &Path) -> Result<String> {
    let script = install()?;
    fs::write(path, script).with_context(|| format!("can't write {}", path.display()))?;
    let run = shell_command(vec!["sudo".into(), "sh".into(), path_text(path)?.into()]);
    Ok(format!(
        "Wrote {}. Read it, then run: {run}",
        path.display()
    ))
}

fn caddy(host: &str, port: u16) -> String {
    format!(
        r#"mkdir -p /etc/caddy/sites
cat > /etc/caddy/sites/thirdshift-console.caddy <<'THIRDSHIFT_SITE'
{host} {{
    reverse_proxy 127.0.0.1:{port} {{
        flush_interval -1
    }}
}}
THIRDSHIFT_SITE
# Include the sites folder once, preserving existing sites.
if ! grep -Eq '^[[:space:]]*import[[:space:]]+(/etc/caddy/)?sites/\*(\.caddy)?[[:space:]]*(#.*)?$' /etc/caddy/Caddyfile; then
    printf '\nimport /etc/caddy/sites/*.caddy\n' >> /etc/caddy/Caddyfile
fi
caddy validate --config /etc/caddy/Caddyfile
"#
    )
}

fn apache(host: &str, port: u16, layout: ApacheLayout) -> (String, String) {
    let (directory, enable, check, service) = match layout {
        ApacheLayout::RedHat => ("/etc/httpd/conf.d", "", "httpd -t", "httpd"),
        ApacheLayout::Debian => (
            "/etc/apache2/sites-available",
            "a2enmod proxy proxy_http headers ssl setenvif\na2ensite thirdshift-console.conf\n",
            "apache2ctl configtest",
            "apache2",
        ),
    };
    let site = format!(
        r#"mkdir -p {directory}
cat > {directory}/thirdshift-console.conf <<'THIRDSHIFT_SITE'
<VirtualHost *:80>
    ServerName {host}
    ProxyPreserveHost On
    RequestHeader set X-Forwarded-Proto "expr=%{{REQUEST_SCHEME}}"
    ProxyPass / http://127.0.0.1:{port}/ flushpackets=on
    ProxyPassReverse / http://127.0.0.1:{port}/
    SetEnvIf Request_URI "^/(events$|logs/[^/]+/stream/)" no-gzip=1
</VirtualHost>
THIRDSHIFT_SITE
{enable}{check}
certbot --apache -d {host}
"#
    );
    (site, format!("systemctl restart {service}\n"))
}

fn available(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| {
            let path = directory.join(name);
            use std::os::unix::fs::PermissionsExt;
            path.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
    })
}

fn identity(flag: &str) -> Result<String> {
    let output = Command::new("id")
        .arg(flag)
        .output()
        .context("can't identify the Console owner")?;
    ensure!(output.status.success(), "can't identify the Console owner");
    let value = String::from_utf8(output.stdout)?;
    ensure!(!value.trim().is_empty(), "can't identify the Console owner");
    Ok(value.trim().to_owned())
}

fn path_text(path: &Path) -> Result<&str> {
    path.to_str()
        .context("console install requires UTF-8 paths")
}

/// systemd quotes are separate from shell quotes; percent expands specifiers.
fn unit_word(value: &str) -> Result<String> {
    ensure!(
        !value.chars().any(char::is_control),
        "console install cannot use control characters in paths"
    );
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}
