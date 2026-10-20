//! Classification of git's failures to reach origin, for the short retry of
//! read-only network commands. Unknown failures stay permanent.

/// Whether git's `stderr` says the connection to origin failed in a way a
/// moment's wait can mend: a dropped stream or connection, a name lookup, a
/// TLS read, or an HTTP 429 or 5xx. A permanent cause, such as a failed
/// login or a missing ref or remote, and then an HTTP status git reports,
/// decide over any other wording, so each stays permanent even when git
/// adds that the remote end hung up.
pub(super) fn is_transient(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if [
        "authentication failed",
        "could not read username",
        "could not read password",
        "repository not found",
        "permission denied",
        "couldn't find remote ref",
        "does not appear to be a git repository",
    ]
    .iter()
    .any(|permanent| stderr.contains(permanent))
    {
        return false;
    }
    if let Some(status) = http_status(&stderr) {
        return status == 429 || (500..600).contains(&status);
    }
    [
        "rpc failed; curl ",
        "error in the http2 framing layer",
        "expected flush after ref listing",
        "early eof",
        "the remote end hung up unexpectedly",
        "unexpected disconnect while reading sideband packet",
        "connection reset",
        "connection timed out",
        "operation timed out",
        "could not resolve host",
        "ssl_read",
        "gnutls_handshake",
        "gnutls_recv",
    ]
    .iter()
    .any(|transient| stderr.contains(transient))
}

/// The HTTP status of curl's `The requested URL returned error: <n>` or of
/// git's `RPC failed; HTTP <n>`, in lowercased `stderr`.
fn http_status(stderr: &str) -> Option<u16> {
    ["returned error: ", "rpc failed; http "]
        .iter()
        .flat_map(|marker| stderr.split(marker).skip(1))
        .find_map(|tail| tail.get(..3).and_then(|code| code.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_failures_are_transient() {
        for stderr in [
            "error: RPC failed; curl 16 Error in the HTTP2 framing layer\n\
             fatal: expected flush after ref listing",
            "error: RPC failed; curl 18 transfer closed with outstanding read data remaining\n\
             fatal: early EOF",
            "error: RPC failed; curl 56 OpenSSL SSL_read: Connection reset by peer, errno 104",
            "fatal: the remote end hung up unexpectedly",
            "fatal: unable to access 'https://github.com/a/b/': Failed to connect to github.com port 443: Connection timed out",
            "fatal: unable to access 'https://github.com/a/b/': Could not resolve host: github.com",
            "fatal: unable to access 'https://github.com/a/b/': The requested URL returned error: 502",
            "fatal: unable to access 'https://github.com/a/b/': The requested URL returned error: 429",
            "error: RPC failed; HTTP 503 curl 22 The requested URL returned error: 503",
            "fatal: unable to access 'https://github.com/a/b/': gnutls_handshake() failed: Error in the pull function.",
        ] {
            assert!(is_transient(stderr.as_bytes()), "{stderr}");
        }
    }

    #[test]
    fn a_missing_ref_with_a_hang_up_is_permanent() {
        assert!(!is_transient(
            b"fatal: couldn't find remote ref absent\nfatal: the remote end hung up unexpectedly"
        ));
    }

    #[test]
    fn other_failures_are_permanent() {
        for stderr in [
            "remote: Invalid username or password.\n\
             fatal: Authentication failed for 'https://github.com/a/b/'",
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            "remote: Repository not found.\n\
             fatal: repository 'https://github.com/a/b/' not found",
            "error: RPC failed; HTTP 403 curl 22 The requested URL returned error: 403\n\
             fatal: the remote end hung up unexpectedly",
            "fatal: unable to access 'https://github.com/a/b/': The requested URL returned error: 401",
            "git@github.com: Permission denied (publickey).\n\
             fatal: Could not read from remote repository.",
            "fatal: couldn't find remote ref absent",
            "fatal: 'origin' does not appear to be a git repository",
            "",
        ] {
            assert!(!is_transient(stderr.as_bytes()), "{stderr}");
        }
    }
}
