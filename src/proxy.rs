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

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::harness::Harness;
use crate::harness::kimi::Catalog;

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
/// Fireworks' OpenAI-compatible API, `https://api.fireworks.ai/inference/v1`.
const FIREWORKS_HOSTS: &[&str] = &["api.fireworks.ai"];
/// Baseten's OpenAI-compatible Model APIs, `https://inference.baseten.co/v1`.
const BASETEN_HOSTS: &[&str] = &["inference.baseten.co"];
/// The managed Kimi Code provider's API and sign-in hosts, from Kimi Code
/// 2.5.2's defaults, in its two regions.
const KIMI_HOSTS: &[&str] = &["api.kimi.com", "auth.kimi.com"];
const KIMI_GLOBAL_HOSTS: &[&str] = &["api.kimi.ai", "auth.kimi.ai"];

/// Direct-SDK providers accepted by a Custom runner manifest and Credentials.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Provider {
    Anthropic,
    Openai,
    Xai,
    Meta,
    Google,
    FireworksAi,
    Baseten,
    Kimi,
}

impl Provider {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Xai => "xai",
            Self::Meta => "meta",
            Self::Google => "google",
            Self::FireworksAi => "fireworks-ai",
            Self::Baseten => "baseten",
            Self::Kimi => "kimi",
        }
    }

    pub(crate) fn environment(self) -> &'static str {
        match self {
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::Openai => "OPENAI_API_KEY",
            Self::Xai => "XAI_API_KEY",
            Self::Meta => "META_API_KEY",
            Self::Google => "GOOGLE_API_KEY",
            Self::FireworksAi => "FIREWORKS_API_KEY",
            Self::Baseten => "BASETEN_API_KEY",
            Self::Kimi => "KIMI_API_KEY",
        }
    }

    pub(crate) fn hosts(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic => &["api.anthropic.com"],
            Self::Openai => &["api.openai.com"],
            Self::Xai => XAI_HOSTS,
            Self::Meta => META_HOSTS,
            Self::Google => &["generativelanguage.googleapis.com"],
            Self::FireworksAi => FIREWORKS_HOSTS,
            Self::Baseten => BASETEN_HOSTS,
            Self::Kimi => &["api.kimi.com", "api.kimi.ai"],
        }
    }

    pub(crate) fn family(self) -> &'static str {
        match self {
            Self::Kimi => "moonshot",
            Self::Anthropic
            | Self::Openai
            | Self::Xai
            | Self::Meta
            | Self::Google
            | Self::FireworksAi
            | Self::Baseten => self.name(),
        }
    }
}

/// A Harness Model's provider family, using its resolved hosts for Kimi aliases.
pub(crate) fn harness_family(harness: Harness, model: Option<&str>, hosts: &[String]) -> String {
    match harness {
        Harness::Claude => "anthropic",
        Harness::Codex => "openai",
        Harness::Grok => "xai",
        Harness::Muse => "meta",
        Harness::Agy => "google",
        Harness::OpenCode => opencode_provider(model).unwrap_or("opencode"),
        Harness::Kimi if hosts.iter().any(|host| host == "api.fireworks.ai") => "fireworks-ai",
        Harness::Kimi => "moonshot",
    }
    .to_owned()
}

/// One provider's hosts by the name an OpenCode Model uses for it.
fn provider_hosts(provider: &str) -> Option<&'static [&'static str]> {
    match provider {
        "anthropic" => Some(ANTHROPIC_HOSTS),
        "openai" => Some(OPENAI_HOSTS),
        "xai" => Some(XAI_HOSTS),
        "meta" => Some(META_HOSTS),
        "fireworks-ai" => Some(FIREWORKS_HOSTS),
        "baseten" => Some(BASETEN_HOSTS),
        _ => None,
    }
}

/// The provider an OpenCode Model names as `<provider>/<model>`, with an
/// optional `#effort` suffix on the model.
pub(crate) fn opencode_provider(model: Option<&str>) -> Option<&str> {
    model
        .and_then(|model| model.split('#').next())
        .and_then(|model| model.split_once('/'))
        .filter(|(provider, model)| !provider.is_empty() && !model.is_empty())
        .map(|(provider, _)| provider)
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
        Harness::Kimi => kimi_trial_hosts(model, Catalog::read)?,
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

/// A Kimi Trial's provider hosts: those of the provider its Model's alias
/// names in Kimi Code's catalog, read only once the Model is known. A Trial
/// must name its Model, since Kimi's default is not in the catalog's output.
fn kimi_trial_hosts(
    model: Option<&str>,
    catalog: impl FnOnce() -> Result<Catalog>,
) -> Result<Vec<&'static str>> {
    let Some(model) = model else {
        bail!(
            "Kimi Trial login failed: the Model must name an alias in Kimi Code's \
             config, so the Trial's proxy can allow its provider's hosts"
        );
    };
    kimi_hosts(&catalog()?.provider(model)?.base_url)
}

/// A Kimi provider's hosts, by the host of the base URL its Model's
/// provider sends requests to. A provider this lookup does not know fails
/// before the session starts rather than reaching further than its list.
pub(crate) fn kimi_hosts(base_url: &str) -> Result<Vec<&'static str>> {
    let host = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host = host
        .rsplit_once(':')
        .filter(|(_, port)| port.parse::<u16>().is_ok())
        .map_or(host, |(host, _)| host)
        .to_ascii_lowercase();
    let hosts = match host.as_str() {
        "api.fireworks.ai" => FIREWORKS_HOSTS,
        "api.kimi.com" => KIMI_HOSTS,
        "api.kimi.ai" => KIMI_GLOBAL_HOSTS,
        _ => bail!(
            "Kimi Trial login failed: unknown provider host {host}; the Trial's Model \
             must use a provider this proxy allows"
        ),
    };
    Ok(hosts.to_vec())
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
    // The kernel also dials IPv4 through mapped IPv6 addresses. Apply the
    // same policy to both spellings, including the host's own addresses.
    if resolved.iter().any(|address| {
        let address = address.to_canonical();
        address.is_loopback() || address.is_unspecified()
    }) {
        return Some("the name resolves to loopback");
    }
    if resolved.iter().any(|address| {
        local
            .iter()
            .any(|local| address.to_canonical() == local.to_canonical())
    }) {
        return Some("the name resolves to this machine");
    }
    None
}

/// The host network's I/O, separate from the proxy's allowlist and address
/// policy so transport tests can use a local provider without relaxing it.
trait Network: Send + Sync {
    fn local_addresses(&self) -> Vec<IpAddr>;
    fn resolve(&self, host: &str, port: u16) -> Vec<IpAddr>;
    fn connect(&self, address: SocketAddr) -> std::io::Result<TcpStream>;
}

struct SystemNetwork;

impl Network for SystemNetwork {
    fn local_addresses(&self) -> Vec<IpAddr> {
        local_addresses()
    }

    /// `getaddrinfo` reads `/etc/hosts` before DNS.
    fn resolve(&self, host: &str, port: u16) -> Vec<IpAddr> {
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

    fn connect(&self, address: SocketAddr) -> std::io::Result<TcpStream> {
        TcpStream::connect_timeout(&address, Duration::from_secs(10))
    }
}

/// The proxy outside one Trial's sandbox: `allowed` is that Trial Harness's
/// provider hosts. Serves its socket until the guard drops, which stops the
/// thread and removes the socket directory. Every refused CONNECT is logged
/// with its reason, the way #1009's checks read the log.
pub(crate) struct Guard {
    dir: tempfile::TempDir,
    shutdown: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    accounting: Arc<Mutex<Accounting>>,
}

/// Provider connections and Harness executions observed outside the Trial.
/// The runner can connect to the proxy, but cannot read or edit this state.
#[derive(Clone, Debug, Default)]
pub(crate) struct Accounting {
    pub(crate) provider_connections: u64,
    pub(crate) harness_invocations: Vec<HarnessInvocation>,
}

#[derive(Clone, Debug)]
pub(crate) struct HarnessInvocation {
    pub(crate) harness: Harness,
    pub(crate) provider_connections: u64,
}

/// A wrapper registers once, then prefixes its forwarded CONNECTs with the
/// returned invocation number. Its own loopback relay uses the same number.
pub(crate) const HARNESS_REGISTER_METHOD: &str = "THIRDSHIFT-HARNESS";
pub(crate) const HARNESS_CONNECTION_PREFIX: &str = "THIRDSHIFT-INVOCATION ";
pub(crate) const HARNESS_RELAY_BASE: u16 = 10000;

impl Guard {
    /// The socket path to bind into the sandbox: a short directory of its
    /// own under `/tmp`, since a Unix socket's path is limited to 108 bytes
    /// and never fits deep under an Experiment's folder.
    pub(crate) fn socket_path(&self) -> std::path::PathBuf {
        self.dir.path().join("proxy.sock")
    }

    pub(crate) fn accounting(&self) -> Accounting {
        self.accounting.lock().unwrap().clone()
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
    start_with(allowed, SystemNetwork)
}

fn start_with(allowed: &[String], network: impl Network + 'static) -> Result<Guard> {
    let dir = tempfile::Builder::new()
        .prefix("thirdshift-proxy-")
        .permissions(fs::Permissions::from_mode(0o700))
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
    let accounting = Arc::new(Mutex::new(Accounting::default()));
    let shared = Arc::new(Shared {
        allowed: allowed.to_vec(),
        local: network.local_addresses(),
        network: Box::new(network),
        accounting: Arc::clone(&accounting),
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
        accounting,
    })
}

/// What every proxy connection shares: the Harness's allowed hosts and this
/// machine's own addresses, read once when the proxy starts.
struct Shared {
    allowed: Vec<String>,
    local: Vec<IpAddr>,
    network: Box<dyn Network>,
    accounting: Arc<Mutex<Accounting>>,
}

/// Accept connections until `shutdown` is set.
fn serve(listener: UnixListener, shared: Arc<Shared>, shutdown: &AtomicBool) {
    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                // macOS hands out accepted sockets non-blocking like their
                // listener, which would end a tunnel at its first idle read.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
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
    if let Some(harness) = head
        .lines()
        .next()
        .and_then(|line| line.strip_prefix(&format!("{HARNESS_REGISTER_METHOD} ")))
        .and_then(|line| line.strip_suffix(" HTTP/1.1"))
        .and_then(Harness::named)
    {
        let mut accounting = shared.accounting.lock().unwrap();
        if accounting.harness_invocations.len() >= usize::from(u16::MAX - HARNESS_RELAY_BASE) {
            return;
        }
        accounting.harness_invocations.push(HarnessInvocation {
            harness,
            provider_connections: 0,
        });
        let _ = writeln!(stream, "{}", accounting.harness_invocations.len());
        return;
    }
    let (invocation, head) = if let Some(tagged) = head.strip_prefix(HARNESS_CONNECTION_PREFIX) {
        let Some((id, head)) = tagged.split_once('\n') else {
            return;
        };
        let Some(index) = id
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|id| id.checked_sub(1))
            .filter(|index| *index < shared.accounting.lock().unwrap().harness_invocations.len())
        else {
            return;
        };
        (Some(index), head)
    } else {
        (None, head.as_str())
    };
    let decision = decide(head, shared);
    if let Decision::Refuse { status, message } = &decision {
        eprintln!("thirdshift proxy: refused CONNECT {message} ({status})");
    }
    let tunnel = match &decision {
        Decision::Refuse { .. } => None,
        Decision::Tunnel(request) => dial(request, shared.network.as_ref()),
    };
    if tunnel.is_some() {
        let mut accounting = shared.accounting.lock().unwrap();
        accounting.provider_connections += 1;
        if let Some(index) = invocation {
            accounting.harness_invocations[index].provider_connections += 1;
        }
    }
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
fn dial(request: &Request, network: &dyn Network) -> Option<TcpStream> {
    request
        .addresses
        .iter()
        .filter_map(|address| {
            network
                .connect(SocketAddr::new(*address, request.port))
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
    let resolved = shared.network.resolve(&host, port);
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

/// Copy bytes both ways, forwarding each EOF so the peer can finish its
/// response and both forwarding threads can end.
fn relay(client: std::os::unix::net::UnixStream, server: TcpStream) {
    // The ten-second timeout only bounds the CONNECT header. Model calls
    // and local work between requests can leave a tunnel idle for longer.
    if client.set_read_timeout(None).is_err() {
        return;
    }
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
            let _ = server_writer.shutdown(std::net::Shutdown::Write);
        });
        let _ = std::io::copy(&mut server_reader, &mut client_writer);
        let _ = client_writer.shutdown(std::net::Shutdown::Write);
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
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::path::Path;

    /// Only the external network I/O is substituted: the proxy still
    /// resolves, checks its real policy, dials and forwards over sockets.
    struct LocalProvider(SocketAddr);

    impl Network for LocalProvider {
        fn local_addresses(&self) -> Vec<IpAddr> {
            vec!["192.0.2.10".parse().unwrap()]
        }

        fn resolve(&self, host: &str, port: u16) -> Vec<IpAddr> {
            assert_eq!((host, port), ("api.anthropic.com", 443));
            vec!["203.0.113.10".parse().unwrap()]
        }

        fn connect(&self, address: SocketAddr) -> std::io::Result<TcpStream> {
            assert_eq!(address, "203.0.113.10:443".parse().unwrap());
            TcpStream::connect(self.0)
        }
    }

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
    fn mapped_loopback_and_host_addresses_are_refused() {
        let local = vec!["192.0.2.10".parse().unwrap()];
        for host in ["::ffff:127.0.0.1", "::ffff:0.0.0.0", "::ffff:192.0.2.10"] {
            let address = host.parse().unwrap();
            assert!(
                address_refusal(&[address], &local).is_some(),
                "proxy permits {host}"
            );
        }
        assert_eq!(
            address_refusal(&["::ffff:203.0.113.10".parse().unwrap()], &local),
            None
        );
        assert!(
            address_refusal(
                &["192.0.2.10".parse().unwrap()],
                &["::ffff:192.0.2.10".parse().unwrap()]
            )
            .is_some()
        );
    }

    #[test]
    fn connect_tunnels_remain_writable_after_waiting_for_a_model_response() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut provider, _) = listener.accept().unwrap();
        provider
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let (mut client, mut proxy) = UnixStream::pair().unwrap();
        client
            .write_all(b"CONNECT api.anthropic.com:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        assert!(read_head(&mut proxy).is_some());
        let forwarding = std::thread::spawn(move || relay(proxy, upstream));
        client.write_all(b"first request").unwrap();
        let mut first = [0; 13];
        provider.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"first request");
        std::thread::sleep(Duration::from_secs(11));
        client.write_all(b"next request").unwrap();
        let mut next = [0; 12];
        let result = provider.read_exact(&mut next);
        drop(provider);
        drop(client);
        forwarding.join().unwrap();
        result.expect("CONNECT tunnel stopped forwarding after 10 idle seconds");
        assert_eq!(&next, b"next request");
    }

    #[test]
    fn connect_tunnels_forward_half_closes_and_the_final_response() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut provider, _) = listener.accept().unwrap();
        provider
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (mut client, proxy) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let forwarding = std::thread::spawn(move || relay(proxy, upstream));

        client.shutdown(std::net::Shutdown::Write).unwrap();
        let provider_eof = provider.read(&mut [0]);
        provider.write_all(b"final response").unwrap();
        provider.shutdown(std::net::Shutdown::Write).unwrap();
        let mut response = [0; 14];
        let response_result = client.read_exact(&mut response);
        let client_eof = client.read(&mut [0]);
        drop(provider);
        drop(client);
        forwarding.join().unwrap();

        assert!(matches!(provider_eof, Ok(0)), "{provider_eof:?}");
        response_result.unwrap();
        assert_eq!(&response, b"final response");
        assert!(matches!(client_eof, Ok(0)), "{client_eof:?}");
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
            (
                Harness::OpenCode,
                Some("fireworks-ai/accounts/fireworks/models/kimi-k3p1-pro"),
                vec!["api.fireworks.ai"],
            ),
            (
                Harness::Kimi,
                Some("kimi-k3.1-pro"),
                vec!["api.fireworks.ai"],
            ),
        ];
        let kimi = || Catalog::parse(include_str!("../tests/fixtures/kimi-providers.json"));
        for (harness, model, some) in cases {
            let allowed = if harness == Harness::Kimi {
                // The Kimi catalog comes from the fixture, not a machine's CLI.
                let allowed: Vec<String> = kimi_trial_hosts(model, kimi)
                    .unwrap()
                    .into_iter()
                    .map(String::from)
                    .collect();
                assert_eq!(allowed, some, "a Fireworks Model reaches Fireworks alone");
                allowed
            } else {
                allowed_hosts(harness, model).unwrap()
            };
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
        // A Kimi Trial must name its Model, whose provider the proxy allows.
        assert!(allowed_hosts(Harness::Kimi, None).is_err());
        assert!(kimi_trial_hosts(Some("kimi-k9"), kimi).is_err());
    }

    #[test]
    fn an_opencode_fireworks_or_baseten_model_reaches_its_provider_host_alone() {
        for (model, provider_host) in [
            (
                "fireworks-ai/accounts/fireworks/models/kimi-k3p1-pro",
                "api.fireworks.ai",
            ),
            (
                "baseten/deepseek-ai/DeepSeek-V4.1-Flash-1017",
                "inference.baseten.co",
            ),
        ] {
            assert_eq!(
                allowed_hosts(Harness::OpenCode, Some(model)).unwrap(),
                ["opencode.ai", "models.opencode.ai", provider_host],
                "{model}"
            );
        }
    }

    #[test]
    fn a_kimi_trial_allows_only_its_models_provider_hosts() {
        assert_eq!(
            kimi_hosts("https://api.fireworks.ai/inference/v1").unwrap(),
            ["api.fireworks.ai"]
        );
        assert_eq!(
            kimi_hosts("https://api.kimi.com/coding/v1").unwrap(),
            ["api.kimi.com", "auth.kimi.com"]
        );
        assert_eq!(
            kimi_hosts("https://API.Kimi.ai:443/coding/v1").unwrap(),
            ["api.kimi.ai", "auth.kimi.ai"]
        );
        for unknown in [
            "https://api.moonshot.ai/v1",
            "https://api.fireworks.ai.attacker.example/v1",
            "http://127.0.0.1:8080/v1",
        ] {
            assert!(kimi_hosts(unknown).is_err(), "{unknown}");
        }
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
    fn the_trial_proxy_directory_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let guard = start(&["api.anthropic.com".to_string()]).unwrap();
        let directory = guard.socket_path().parent().unwrap().to_path_buf();
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
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
        assert_eq!(guard.accounting().provider_connections, 0);
    }

    #[test]
    fn accounting_distinguishes_harness_invocations_from_direct_provider_calls() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let guard = start_with(
            &["api.anthropic.com".to_string()],
            LocalProvider(listener.local_addr().unwrap()),
        )
        .unwrap();
        for (harness, expected) in [("claude", "1"), ("codex", "2"), ("claude", "3")] {
            assert_eq!(
                connect_status(
                    &guard.socket_path(),
                    &format!("{HARNESS_REGISTER_METHOD} {harness} HTTP/1.1\r\n\r\n"),
                ),
                expected
            );
        }
        // More than one provider connection may belong to a single session.
        for invocation in [Some(1), Some(1), Some(2), None] {
            let prefix = invocation.map_or_else(String::new, |id| {
                format!("{HARNESS_CONNECTION_PREFIX}{id}\n")
            });
            assert_eq!(
                connect_status(
                    &guard.socket_path(),
                    &format!("{prefix}CONNECT api.anthropic.com:443 HTTP/1.1\r\n\r\n"),
                ),
                "HTTP/1.1 200 Connection Established"
            );
            drop(listener.accept().unwrap());
        }
        assert_eq!(
            connect_status(
                &guard.socket_path(),
                &format!("{HARNESS_CONNECTION_PREFIX}3\nCONNECT github.com:443 HTTP/1.1\r\n\r\n"),
            ),
            "HTTP/1.1 403 Forbidden"
        );
        let accounting = guard.accounting();
        assert_eq!(accounting.provider_connections, 4);
        assert_eq!(
            accounting
                .harness_invocations
                .iter()
                .map(|invocation| (invocation.harness, invocation.provider_connections))
                .collect::<Vec<_>>(),
            [
                (Harness::Claude, 2),
                (Harness::Codex, 1),
                (Harness::Claude, 0)
            ]
        );
    }

    #[test]
    fn failed_provider_dials_do_not_create_accounting_gaps() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let guard = start_with(&["api.anthropic.com".to_string()], LocalProvider(address)).unwrap();
        assert_eq!(
            connect_status(
                &guard.socket_path(),
                "CONNECT api.anthropic.com:443 HTTP/1.1\r\n\r\n",
            ),
            "HTTP/1.1 502 Bad Gateway"
        );
        assert_eq!(guard.accounting().provider_connections, 0);
    }

    #[test]
    fn an_allowed_host_is_reached_through_the_proxy() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let guard = start_with(
            &["api.anthropic.com".to_string()],
            LocalProvider(listener.local_addr().unwrap()),
        )
        .unwrap();
        let mut client = UnixStream::connect(guard.socket_path()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client
            .write_all(b"CONNECT api.anthropic.com:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut client = BufReader::new(client);
        let mut status = String::new();
        client.read_line(&mut status).unwrap();
        assert_eq!(status, "HTTP/1.1 200 Connection Established\r\n");
        let mut blank = String::new();
        client.read_line(&mut blank).unwrap();
        assert_eq!(blank, "\r\n");

        let (mut provider, _) = listener.accept().unwrap();
        provider
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        // A tunnel sits idle between requests; the relay must wait for it.
        std::thread::sleep(Duration::from_millis(200));
        client.get_mut().write_all(b"request").unwrap();
        let mut request = [0; 7];
        provider.read_exact(&mut request).unwrap();
        assert_eq!(&request, b"request");
        provider.write_all(b"response").unwrap();
        let mut response = [0; 8];
        client.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"response");
    }
}
