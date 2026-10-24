//! A Trial sandbox's network: an allowlisting CONNECT proxy.
//!
//! Each sandbox gets a network of its own, holding only its own loopback, so
//! nothing inside can reach the host or the internet directly. The proxy runs
//! outside the sandbox, listening on a Unix socket that is bound into it. A
//! relay inside turns a local port into that socket, and the Harness reaches
//! its provider through `HTTPS_PROXY`. What the proxy allows is only that
//! Harness's provider hosts, from #1009's live-checked lists. It refuses names
//! that resolve to loopback or to the host's own addresses, and anything it
//! does not allow never resolves: the allowlist is checked before DNS.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::harness::Harness;

/// One provider's hosts, from #1009's live-checked lists: the hosts each
/// Harness contacted through `HTTPS_PROXY` with a fake credential. Codex's
/// plugin-sync hosts (`github.com`, `api.github.com`) and `files.openai.com`
/// stay refused: with them refused, Codex carries on to its API. The
/// subscription-login hosts the checks could not reach come from Codex's own
/// source (`chatgpt.com`, `auth.openai.com`) and Claude's sandbox docs
/// (`claude.ai`, `platform.claude.com`).
const ANTHROPIC_HOSTS: &[&str] = &["api.anthropic.com", "claude.ai", "platform.claude.com"];
const OPENAI_HOSTS: &[&str] = &["api.openai.com", "chatgpt.com", "auth.openai.com"];
const XAI_HOSTS: &[&str] = &["api.x.ai"];
const META_HOSTS: &[&str] = &["api.meta.ai"];
const AGY_HOSTS: &[&str] = &["antigravity-unleash.goog", "play.googleapis.com"];

/// One provider's hosts by the name an OpenCode Model uses for it.
fn provider_hosts(provider: &str) -> Option<&'static [&'static str]> {
    match provider {
        "anthropic" => Some(ANTHROPIC_HOSTS),
        "openai" => Some(OPENAI_HOSTS),
        "xai" => Some(XAI_HOSTS),
        "meta" => Some(META_HOSTS),
        _ => None,
    }
}

/// The provider an OpenCode Model names as `<provider>/<model>`, with an
/// optional `#effort` suffix on the model.
pub(crate) fn opencode_provider(model: Option<&str>) -> Option<&str> {
    model
        .and_then(|model| model.split('#').next())
        .and_then(|model| model.split_once('/').map(|(provider, _)| provider))
        .filter(|provider| !provider.is_empty())
}

/// What the proxy allows, per Harness: that Harness's provider hosts.
pub(crate) fn allowed_hosts(harness: Harness, model: Option<&str>) -> Result<Vec<String>> {
    let hosts: Vec<&str> = match harness {
        Harness::Claude => ANTHROPIC_HOSTS.to_vec(),
        Harness::Codex => OPENAI_HOSTS.to_vec(),
        Harness::Grok => XAI_HOSTS.to_vec(),
        Harness::Muse => META_HOSTS.to_vec(),
        Harness::Agy => AGY_HOSTS.to_vec(),
        Harness::OpenCode => {
            let mut hosts = vec!["opencode.ai", "models.opencode.ai"];
            hosts.extend(opencode_provider_hosts(model)?);
            hosts
        }
    };
    let mut kept = Vec::new();
    for host in hosts {
        if !kept.contains(&host.to_string()) {
            kept.push(host.to_string());
        }
    }
    Ok(kept)
}

/// An OpenCode Trial's provider hosts: the provider its Model names, looked up
/// in the same lists. An OpenCode session without a provider answers on a free
/// model, so a Trial must name its provider; one this lookup does not know
/// fails before its session starts rather than reaching further than its list.
fn opencode_provider_hosts(model: Option<&str>) -> Result<Vec<&'static str>> {
    match opencode_provider(model).and_then(provider_hosts) {
        Some(hosts) => Ok(hosts.to_vec()),
        None => match opencode_provider(model) {
            Some(provider) => bail!(
                "OpenCode Trial login failed: unknown provider {provider}; \
                 the Trial's Model must name a provider this proxy allows"
            ),
            None => bail!(
                "OpenCode Trial login failed: the Model must name its provider as \
                 <provider>/<model>, so the Trial's proxy can allow that provider's hosts"
            ),
        },
    }
}

/// Whether `host` names an allowed host: an exact match, case-insensitive,
/// with a trailing dot ignored. Checked before DNS, so anything refused here
/// never resolves.
pub(crate) fn allowed_name(host: &str, allowed: &[String]) -> bool {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    !normalized.is_empty() && allowed.contains(&normalized)
}

/// Why the proxy refuses resolved addresses: loopback (or the unspecified
/// address, which dials as loopback) or one of this machine's own addresses.
/// `None` allows the connection.
pub(crate) fn address_refusal(resolved: &[IpAddr], local: &[IpAddr]) -> Option<&'static str> {
    if resolved.is_empty() {
        return Some("the name did not resolve");
    }
    if resolved
        .iter()
        .any(|address| address.is_loopback() || address.is_unspecified())
    {
        return Some("the name resolves to loopback");
    }
    if resolved.iter().any(|address| local.contains(address)) {
        return Some("the name resolves to this machine");
    }
    None
}

/// Resolve `host` the way the proxy's dial will: `getaddrinfo`, which reads
/// `/etc/hosts` before DNS.
fn resolve(host: &str, port: u16) -> Vec<IpAddr> {
    let mut addresses = Vec::new();
    if let Ok(resolved) = (host, port).to_socket_addrs() {
        for socket in resolved {
            if !addresses.contains(&socket.ip()) {
                addresses.push(socket.ip());
            }
        }
    }
    addresses
}

/// The proxy outside one Trial's sandbox: `allowed` is that Trial Harness's
/// provider hosts. Serves its socket until the guard drops, which stops the
/// thread and removes the socket directory. Every refused CONNECT is logged
/// with its reason, the way #1009's checks read the log.
pub(crate) struct Guard {
    dir: tempfile::TempDir,
    shutdown: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Guard {
    /// The socket path to bind into the sandbox: a short directory of its
    /// own under `/tmp`, since a Unix socket's path is limited to 108 bytes
    /// and never fits deep under an Experiment's folder.
    pub(crate) fn socket_path(&self) -> std::path::PathBuf {
        self.dir.path().join("proxy.sock")
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

/// Serve the allowlisting proxy for one Trial's sandbox.
pub(crate) fn start(allowed: &[String]) -> Result<Guard> {
    let dir = tempfile::Builder::new()
        .prefix("thirdshift-proxy-")
        .tempdir_in("/tmp")
        .context("sandbox failed to start: could not create the Trial proxy's directory")?;
    let socket_path = dir.path().join("proxy.sock");
    let listener = UnixListener::bind(&socket_path).with_context(|| {
        format!(
            "sandbox failed to start: could not listen on {}",
            socket_path.display()
        )
    })?;
    listener
        .set_nonblocking(true)
        .context("sandbox failed to start: could not listen without blocking")?;
    let shared = Arc::new(Shared {
        allowed: allowed.to_vec(),
        local: local_addresses(),
    });
    let shutdown = Arc::new(AtomicBool::new(false));
    let thread = std::thread::Builder::new()
        .name("thirdshift-trial-proxy".into())
        .spawn({
            let shutdown = Arc::clone(&shutdown);
            move || serve(listener, shared, &shutdown)
        })
        .context("sandbox failed to start: could not start the Trial proxy")?;
    Ok(Guard {
        dir,
        shutdown,
        thread: Some(thread),
    })
}

/// What every proxy connection shares: the Harness's allowed hosts and this
/// machine's own addresses, read once when the proxy starts.
struct Shared {
    allowed: Vec<String>,
    local: Vec<IpAddr>,
}

/// Accept connections until `shutdown` is set.
fn serve(listener: UnixListener, shared: Arc<Shared>, shutdown: &AtomicBool) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let shared = Arc::clone(&shared);
                std::thread::Builder::new()
                    .name("thirdshift-trial-proxy-conn".into())
                    .spawn(move || serve_one(stream, &shared))
                    .ok();
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                if !shutdown.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}

/// Serve one relay connection: a CONNECT request, then bytes both ways. The
/// 200 goes out only once the tunnel is dialed: a client that reads success
/// knows its bytes will move.
fn serve_one(mut stream: std::os::unix::net::UnixStream, shared: &Shared) {
    // A relay probe connects and closes without asking: no request, no
    // refusal to log.
    let Some(head) = read_head(&mut stream) else {
        return;
    };
    let decision = decide(&head, shared);
    if let Decision::Refuse { status, message } = &decision {
        eprintln!("thirdshift proxy: refused CONNECT {message} ({status})");
    }
    let tunnel = match &decision {
        Decision::Refuse { .. } => None,
        Decision::Tunnel(request) => dial(request),
    };
    let (status, body) = match (&decision, &tunnel) {
        (Decision::Tunnel(_), Some(_)) => ("200 Connection Established".to_string(), None),
        (Decision::Tunnel(request), None) => (
            "502 Bad Gateway".to_string(),
            Some(format!(
                "CONNECT {}:{} could not connect",
                request.host, request.port
            )),
        ),
        (Decision::Refuse { status, message }, _) => (status.clone(), Some(message.clone())),
    };
    let mut response = format!("HTTP/1.1 {status}\r\n");
    match body {
        None => response.push_str("\r\n"),
        Some(body) => response.push_str(&format!(
            "Content-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )),
    }
    if stream.write_all(response.as_bytes()).is_err() {
        return;
    }
    if let Some(server) = tunnel {
        relay(stream, server);
    }
}

/// What the proxy does with a request head.
enum Decision {
    Tunnel(Request),
    Refuse { status: String, message: String },
}

/// A CONNECT request the proxy will dial.
struct Request {
    host: String,
    port: u16,
    addresses: Vec<IpAddr>,
}

/// Dial the request's resolved addresses in order, holding each at most ten
/// seconds.
fn dial(request: &Request) -> Option<TcpStream> {
    request
        .addresses
        .iter()
        .filter_map(|address| {
            TcpStream::connect_timeout(
                &std::net::SocketAddr::new(*address, request.port),
                Duration::from_secs(10),
            )
            .ok()
        })
        .next()
}

/// Read the request head: lines until the first blank one, at most 8 KiB.
/// `None` is an empty connection or an unreadable one.
fn read_head(stream: &mut std::os::unix::net::UnixStream) -> Option<String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut head = Vec::new();
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = Vec::new();
        // A zero-byte read is the relay's probe closing: not a request.
        if reader.read_until(b'\n', &mut line).ok()? == 0 || head.len() + line.len() > 8192 {
            return None;
        }
        head.extend_from_slice(&line);
        if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
            break;
        }
    }
    String::from_utf8(head).ok()
}

/// Decide a request head. The allowlist runs before DNS, so a refused name
/// never resolves.
fn decide(head: &str, shared: &Shared) -> Decision {
    let refuse = |status: &str, message: String| Decision::Refuse {
        status: status.to_string(),
        message,
    };
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let (method, target) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(method), Some(target), Some(_version), None) => (method, target),
        _ => {
            return refuse(
                "400 Bad Request",
                "CONNECT <host>:<port> HTTP/1.1".to_string(),
            );
        }
    };
    if method != "CONNECT" {
        return refuse(
            "405 Method Not Allowed",
            format!("{method} is not allowed; this proxy only CONNECTs"),
        );
    }
    let Some((host, port)) = split_authority(target) else {
        return refuse(
            "400 Bad Request",
            format!("{target} is not a <host>:<port> authority"),
        );
    };
    let message = || format!("CONNECT {host}:{port}");
    if !allowed_name(&host, &shared.allowed) {
        return refuse("403 Forbidden", format!("{} is not allowlisted", message()));
    }
    let resolved = resolve(&host, port);
    if let Some(reason) = address_refusal(&resolved, &shared.local) {
        return refuse("403 Forbidden", format!("{}: {reason}", message()));
    }
    Decision::Tunnel(Request {
        host,
        port,
        addresses: resolved,
    })
}

/// Split a CONNECT authority into its host and port: `example:443`,
/// `[::1]:443`, never credentials or a path.
fn split_authority(target: &str) -> Option<(String, u16)> {
    if target.contains('@') || target.contains('/') {
        return None;
    }
    let (host, port) = if let Some(rest) = target.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        (format!("[{host}]"), port)
    } else {
        let (host, port) = target.rsplit_once(':')?;
        (host.to_string(), port)
    };
    if host.is_empty() {
        return None;
    }
    Some((host, port.parse().ok()?))
}

/// Copy bytes both ways until either direction ends.
fn relay(client: std::os::unix::net::UnixStream, server: TcpStream) {
    let mut client_reader = match client.try_clone() {
        Ok(reader) => reader,
        Err(_) => return,
    };
    let mut server_reader = match server.try_clone() {
        Ok(reader) => reader,
        Err(_) => return,
    };
    let mut client_writer = client;
    let mut server_writer = server;
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ = std::io::copy(&mut client_reader, &mut server_writer);
        });
        let _ = std::io::copy(&mut server_reader, &mut client_writer);
    });
}

/// This machine's own addresses, from its interfaces.
fn local_addresses() -> Vec<IpAddr> {
    let mut addresses = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `getifaddrs` writes the list head on success; each entry is
    // read once and the list is freed below.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return addresses;
    }
    let mut current = list;
    while !current.is_null() {
        // SAFETY: `current` walks the list `getifaddrs` returned.
        let entry = unsafe { &*current };
        if !entry.ifa_addr.is_null() {
            // SAFETY: `ifa_addr` points at a `sockaddr` with its family set.
            let family = unsafe { (*entry.ifa_addr).sa_family as libc::c_int };
            if family == libc::AF_INET {
                // SAFETY: an `AF_INET` address is a `sockaddr_in`.
                let address = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
                addresses.push(IpAddr::from(std::net::Ipv4Addr::from(u32::from_be(
                    address.sin_addr.s_addr,
                ))));
            } else if family == libc::AF_INET6 {
                // SAFETY: an `AF_INET6` address is a `sockaddr_in6`.
                let address = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in6) };
                addresses.push(IpAddr::from(address.sin6_addr.s6_addr));
            }
        }
        current = entry.ifa_next;
    }
    // SAFETY: the list came from `getifaddrs` and is freed exactly once.
    unsafe { libc::freeifaddrs(list) };
    addresses
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    #[test]
    fn the_allowlist_matches_names_only() {
        let allowed = ["api.anthropic.com".to_string()];
        assert!(allowed_name("api.anthropic.com", &allowed));
        assert!(allowed_name("API.ANTHROPIC.COM", &allowed));
        assert!(allowed_name("api.anthropic.com.", &allowed));
        assert!(!allowed_name("github.com", &allowed));
        assert!(!allowed_name("api.anthropic.com.evil.com", &allowed));
        assert!(!allowed_name("evil-api.anthropic.com", &allowed));
        assert!(!allowed_name("", &allowed));
        assert!(!allowed_name("127.0.0.1", &allowed));
    }

    #[test]
    fn the_guard_refuses_loopback_and_this_machine() {
        let local = vec![
            "192.0.2.10".parse().unwrap(),
            "2001:db8::10".parse().unwrap(),
        ];
        let internet: Vec<IpAddr> = vec!["203.0.113.10".parse().unwrap()];
        assert_eq!(address_refusal(&internet, &local), None);
        for refused in [
            vec!["127.0.0.1".parse().unwrap()],
            vec!["::1".parse().unwrap()],
            vec!["0.0.0.0".parse().unwrap()],
            vec!["::".parse().unwrap()],
            vec!["192.0.2.10".parse().unwrap()],
            vec!["2001:db8::10".parse().unwrap()],
            // One bad address in the set refuses the name, against DNS
            // rebinding past the check.
            vec![
                "203.0.113.10".parse().unwrap(),
                "127.0.0.1".parse().unwrap(),
            ],
        ] {
            assert!(address_refusal(&refused, &local).is_some(), "{refused:?}");
        }
        assert!(address_refusal(&[], &local).is_some());
    }

    #[test]
    fn authorities_split_into_host_and_port() {
        assert_eq!(
            split_authority("example.com:443"),
            Some(("example.com".to_string(), 443))
        );
        assert_eq!(
            split_authority("[::1]:8080"),
            Some(("[::1]".to_string(), 8080))
        );
        for bad in [
            "example.com",
            ":443",
            "example.com:notaport",
            "user@example.com:443",
            "example.com:443/path",
            "[::1]8080",
        ] {
            assert_eq!(split_authority(bad), None, "{bad}");
        }
    }

    #[test]
    fn each_harness_allows_only_its_provider_hosts() {
        let cases = [
            (
                Harness::Claude,
                None,
                vec!["api.anthropic.com", "claude.ai", "platform.claude.com"],
            ),
            (
                Harness::Codex,
                None,
                vec!["api.openai.com", "chatgpt.com", "auth.openai.com"],
            ),
            (Harness::Grok, None, vec!["api.x.ai"]),
            (Harness::Muse, None, vec!["api.meta.ai"]),
            (
                Harness::Agy,
                None,
                vec!["antigravity-unleash.goog", "play.googleapis.com"],
            ),
            (
                Harness::OpenCode,
                Some("anthropic/claude"),
                vec!["opencode.ai", "models.opencode.ai", "api.anthropic.com"],
            ),
        ];
        for (harness, model, some) in cases {
            let allowed = allowed_hosts(harness, model).unwrap();
            for host in some {
                assert!(allowed.contains(&host.to_string()), "{harness:?}: {host}");
            }
        }
        // Codex's plugin-sync hosts stay refused, and GitHub is allowed
        // nowhere.
        for harness in [
            Harness::Claude,
            Harness::Codex,
            Harness::Grok,
            Harness::Muse,
            Harness::Agy,
        ] {
            let allowed = allowed_hosts(harness, None).unwrap();
            for host in ["github.com", "api.github.com", "files.openai.com"] {
                assert!(!allowed.contains(&host.to_string()), "{harness:?}: {host}");
            }
        }
        assert!(allowed_hosts(Harness::OpenCode, Some("acme/model")).is_err());
        assert!(allowed_hosts(Harness::OpenCode, Some("model-without-provider")).is_err());
        assert!(allowed_hosts(Harness::OpenCode, None).is_err());
    }

    /// Ask the proxy at `socket_path` for `head` and read its status line.
    fn connect_status(socket_path: &Path, head: &str) -> String {
        let mut stream = UnixStream::connect(socket_path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        stream.write_all(head.as_bytes()).unwrap();
        let mut reader = BufReader::new(&stream);
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        status.trim().to_string()
    }

    #[test]
    fn a_disallowed_host_is_refused_without_resolving() {
        let guard = start(&["api.anthropic.com".to_string()]).unwrap();
        let status = connect_status(
            &guard.socket_path(),
            "CONNECT github.com:443 HTTP/1.1\r\nHost: github.com:443\r\n\r\n",
        );
        assert_eq!(status, "HTTP/1.1 403 Forbidden");
    }

    #[test]
    fn a_name_that_resolves_to_loopback_is_refused() {
        let guard = start(&["localhost".to_string()]).unwrap();
        let status = connect_status(
            &guard.socket_path(),
            "CONNECT localhost:443 HTTP/1.1\r\nHost: localhost:443\r\n\r\n",
        );
        assert_eq!(status, "HTTP/1.1 403 Forbidden");
    }

    #[test]
    fn only_connect_tunnels() {
        let guard = start(&["api.anthropic.com".to_string()]).unwrap();
        let status = connect_status(
            &guard.socket_path(),
            "GET https://api.anthropic.com/ HTTP/1.1\r\nHost: api.anthropic.com\r\n\r\n",
        );
        assert_eq!(status, "HTTP/1.1 405 Method Not Allowed");
    }

    #[test]
    fn an_allowed_host_is_reached_through_the_proxy() {
        // Needs the internet: without it, this reports itself skipped
        // rather than failing on the dial.
        if TcpStream::connect_timeout(
            &std::net::SocketAddr::new("93.184.216.34".parse().unwrap(), 443),
            Duration::from_secs(8),
        )
        .is_err()
        {
            println!("SKIPPED (proxy dial): no route to the internet");
            return;
        }
        let guard = start(&["api.anthropic.com".to_string()]).unwrap();
        let status = connect_status(
            &guard.socket_path(),
            "CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: api.anthropic.com:443\r\n\r\n",
        );
        assert_eq!(status, "HTTP/1.1 200 Connection Established");
    }
}
