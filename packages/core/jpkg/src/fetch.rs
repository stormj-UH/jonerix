// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! Synchronous HTTPS fetch layer — port of `jpkg/src/fetch.c` (libtls) to
//! `ureq` + `rustls`.
//!
//! # Invariants
//!
//! 1. **Atomicity**: [`download_to`] writes to `<dest>.partial` in the same
//!    directory as `dest` and renames it into place only on complete success.
//!    On any error the `.partial` file is removed and `dest` is absent.
//!    Callers must not treat a missing `dest` as a permanent error; the mirror
//!    fallback in [`download_via_mirrors_to`] retries the next mirror.
//!
//! 2. **TLS CA bundle**: the `webpki-roots` CA bundle is compiled in; no
//!    system CA store is read at runtime.  HTTP (plain) is also accepted for
//!    test harnesses that spin up a local `TcpListener`.  Callers must not
//!    pass plain-HTTP mirror URLs in production.
//!
//! 3. **Timeout bounds**: both connect and read timeouts are 30 seconds.  A
//!    [`FetchError::Timeout`] is returned when either limit is exceeded.
//!    [`download_via_mirrors`] and [`download_via_mirrors_to`] log a warning
//!    and move to the next mirror on any per-mirror error, including timeouts.
//!
//! 4. **Redirect limit**: up to 10 HTTP redirects are followed automatically.
//!    A chain longer than 10 redirects is treated as a transport error.
//!
//! 5. **URL joining**: [`join_mirror`] (used internally) ensures exactly one
//!    `/` separates the mirror base and the relative path regardless of whether
//!    either side has a trailing/leading slash.  Callers that construct mirror
//!    URLs must not double-slash paths; [`download_via_mirrors`] calls
//!    `join_mirror` for them.
//!
//! Design notes:
//! - No system CA bundle dependency.  webpki-roots is compiled in, matching the
//!   intent of the C version's `tls_config_set_ca_file` lookup.
//! - ureq follows redirects automatically (default 5; we raise it to 10 to match
//!   the spec comment).  The C code tracked 5 for HTTPS and was a soft limit; 10
//!   is more defensive and matches the task spec.
//! - Timeouts: 30 s connect + 30 s read, same as the C curl fallback's
//!   `--connect-timeout 30`.  ureq surfaces timeout IO errors as
//!   `io::ErrorKind::TimedOut` which we map to `FetchError::Timeout`.
//! - Atomic writes: `download_to` writes to `<dest>.partial` then `rename(2)`.
//!   On any error the partial file is unlinked, leaving `dest` absent —
//!   identical to the C `fetch_to_file` contract.
//! - HTTP (plain) is accepted in addition to HTTPS so that unit tests that spin
//!   up a local `TcpListener` server work without a TLS certificate.

use log::{debug, warn};
use rustls::ClientConfig;
use std::error::Error as StdError;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use webpki_roots;

// ─── Error type ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum FetchError {
    Io(io::Error),
    Http { status: u16, url: String },
    Transport(String),
    NoMirrors,
    BadUrl(String),
    Timeout,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Io(e) => write!(f, "I/O error: {}", e),
            FetchError::Http { status, url } => {
                write!(f, "HTTP {} fetching {}", status, url)
            }
            FetchError::Transport(msg) => write!(f, "transport error: {}", msg),
            FetchError::NoMirrors => write!(f, "all mirrors failed"),
            FetchError::BadUrl(u) => write!(f, "bad URL: {}", u),
            FetchError::Timeout => write!(f, "request timed out"),
        }
    }
}

impl std::error::Error for FetchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FetchError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for FetchError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::TimedOut {
            FetchError::Timeout
        } else {
            FetchError::Io(e)
        }
    }
}

// ─── TLS agent (webpki-roots, no system CA bundle) ──────────────────────────

fn build_agent() -> ureq::Agent {
    let root_store = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let tls_cfg = ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    // SAFETY: with_protocol_versions returns Err only when a protocol version
    // is unsupported by the selected crypto provider.  The ring provider
    // ships with TLS 1.2 and TLS 1.3 support compiled in, so this is
    // unreachable in all configurations we build.
    .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
    .expect("TLS protocol config: ring provider always supports TLS 1.2 and 1.3")
    .with_root_certificates(root_store)
    .with_no_client_auth();

    ureq::AgentBuilder::new()
        .tls_config(Arc::new(tls_cfg))
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(30))
        .redirects(10)
        .build()
}

// ─── Error mapping ───────────────────────────────────────────────────────────

fn map_ureq(err: ureq::Error, url: &str) -> FetchError {
    match err {
        ureq::Error::Status(status, _resp) => FetchError::Http {
            status,
            url: url.to_owned(),
        },
        ureq::Error::Transport(t) => {
            // Surface timeouts distinctly.
            if let Some(src) = StdError::source(&t) {
                if let Some(io_err) = src.downcast_ref::<io::Error>() {
                    if io_err.kind() == io::ErrorKind::TimedOut {
                        return FetchError::Timeout;
                    }
                }
            }
            FetchError::Transport(t.to_string())
        }
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Synchronous HTTPS (or HTTP) GET.  Returns the raw body bytes.
/// On any 4xx/5xx, returns `FetchError::Http` (does NOT retry).
/// 30-second connect + read timeouts.
pub fn download(url: &str) -> Result<Vec<u8>, FetchError> {
    debug!("fetch: GET {}", url);

    if url.is_empty() {
        return Err(FetchError::BadUrl(url.to_owned()));
    }

    let agent = build_agent();
    let resp = agent.get(url).call().map_err(|e| map_ureq(e, url))?;
    let mut body = Vec::new();
    resp.into_reader()
        .read_to_end(&mut body)
        .map_err(FetchError::from)?;
    Ok(body)
}

/// Synchronous HTTPS (or HTTP) GET, streaming directly to `dest`.
/// Atomic via a `.partial` tmp file in the same directory, renamed on success.
/// Creates parent directories as needed.
/// If the transfer fails mid-body the `.partial` file is removed and `dest` is absent.
pub fn download_to(url: &str, dest: &Path) -> Result<(), FetchError> {
    debug!("fetch: GET {} -> {}", url, dest.display());

    if url.is_empty() {
        return Err(FetchError::BadUrl(url.to_owned()));
    }

    // Ensure parent directory exists.
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(FetchError::from)?;
    }

    let partial = {
        let mut p = dest.as_os_str().to_os_string();
        p.push(".partial");
        std::path::PathBuf::from(p)
    };

    let agent = build_agent();
    let resp = agent.get(url).call().map_err(|e| map_ureq(e, url))?;
    let mut reader = resp.into_reader();

    let result = (|| -> Result<(), FetchError> {
        let mut file = fs::File::create(&partial).map_err(FetchError::from)?;
        let mut buf = [0u8; 65536];
        loop {
            let n = reader.read(&mut buf).map_err(FetchError::from)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(FetchError::from)?;
        }
        file.flush().map_err(FetchError::from)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            fs::rename(&partial, dest).map_err(FetchError::from)?;
            Ok(())
        }
        Err(e) => {
            // Best-effort cleanup; ignore unlink errors.
            let _ = fs::remove_file(&partial);
            Err(e)
        }
    }
}

/// HEAD request — used by repo to check INDEX freshness without re-downloading.
/// Returns the parsed Content-Length if present.
pub fn head_content_length(url: &str) -> Result<Option<u64>, FetchError> {
    debug!("fetch: HEAD {}", url);

    if url.is_empty() {
        return Err(FetchError::BadUrl(url.to_owned()));
    }

    let agent = build_agent();
    let resp = agent.head(url).call().map_err(|e| map_ureq(e, url))?;

    let length = resp
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok());
    Ok(length)
}

/// Try a list of mirror URLs in order, appending `path` to each.
/// Returns the body from the first mirror that succeeds.
/// Logs a warning for each failed mirror.
/// Returns `FetchError::NoMirrors` if every mirror fails.
pub fn download_via_mirrors(mirrors: &[String], path: &str) -> Result<Vec<u8>, FetchError> {
    if mirrors.is_empty() {
        return Err(FetchError::NoMirrors);
    }
    for mirror in mirrors {
        let url = join_mirror(mirror, path);
        match download(&url) {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                warn!("mirror {} failed: {}", url, e);
            }
        }
    }
    Err(FetchError::NoMirrors)
}

/// Like `download_via_mirrors` but writes directly to `dest` (atomic).
pub fn download_via_mirrors_to(
    mirrors: &[String],
    path: &str,
    dest: &Path,
) -> Result<(), FetchError> {
    if mirrors.is_empty() {
        return Err(FetchError::NoMirrors);
    }
    for mirror in mirrors {
        let url = join_mirror(mirror, path);
        match download_to(&url, dest) {
            Ok(()) => return Ok(()),
            Err(e) => {
                warn!("mirror {} failed: {}", url, e);
            }
        }
    }
    Err(FetchError::NoMirrors)
}

/// Join a mirror base URL and a relative path, inserting exactly one `/`
/// between them and tolerating a separator on either side (e.g.
/// `"https://m"` + `"INDEX.zst"`, `"https://m/"` + `"INDEX.zst"`,
/// `"https://m"` + `"/INDEX.zst"`, `"https://m/"` + `"/INDEX.zst"` all
/// produce `"https://m/INDEX.zst"`).
#[inline]
fn join_mirror(mirror: &str, path: &str) -> String {
    let m = mirror.trim_end_matches('/');
    let p = path.trim_start_matches('/');
    format!("{m}/{p}")
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;

    // ── Tiny one-shot HTTP server helpers ────────────────────────────────────

    /// Bind a listener on 127.0.0.1:0.  Returns (listener, port).
    fn bind_local() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        (listener, port)
    }

    /// Serve exactly one HTTP/1.0 response then close.
    /// `response_bytes` must be a fully-formed HTTP response (status line +
    /// headers + blank line + body).
    fn serve_once(listener: TcpListener, response_bytes: Vec<u8>) {
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Drain request (we don't inspect it).
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(&response_bytes);
                // Drop → close → EOF to client.
            }
        });
    }

    fn ok_response(body: &[u8]) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn not_found_response() -> Vec<u8> {
        b"HTTP/1.0 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found".to_vec()
    }

    fn head_response(content_length: u64) -> Vec<u8> {
        format!(
            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n",
            content_length
        )
        .into_bytes()
    }

    // ── download: 200 ────────────────────────────────────────────────────────

    #[test]
    fn test_download_200() {
        let (listener, port) = bind_local();
        let body = b"hello jpkg";
        serve_once(listener, ok_response(body));

        let url = format!("http://127.0.0.1:{}/", port);
        let result = download(&url).expect("download should succeed");
        assert_eq!(result, body);
    }

    // ── download: 404 → FetchError::Http ────────────────────────────────────

    #[test]
    fn test_download_404() {
        let (listener, port) = bind_local();
        serve_once(listener, not_found_response());

        let url = format!("http://127.0.0.1:{}/missing", port);
        match download(&url) {
            Err(FetchError::Http { status, .. }) => assert_eq!(status, 404),
            other => panic!("expected FetchError::Http(404), got {:?}", other),
        }
    }

    // ── download_to: atomic write + cleanup on mid-body drop ─────────────────

    #[test]
    fn test_download_to_atomic_success() {
        let (listener, port) = bind_local();
        let body = b"atomic content";
        serve_once(listener, ok_response(body));

        let dir = tempfile::tempdir().expect("tempdir");
        let dest: PathBuf = dir.path().join("out.bin");
        let url = format!("http://127.0.0.1:{}/file", port);

        download_to(&url, &dest).expect("download_to should succeed");

        let written = fs::read(&dest).expect("dest should exist after success");
        assert_eq!(written, body);

        // .partial must not remain.
        let mut partial = dest.as_os_str().to_os_string();
        partial.push(".partial");
        assert!(
            !Path::new(&partial).exists(),
            ".partial file should be cleaned up"
        );
    }

    #[test]
    fn test_download_to_cleans_partial_on_connection_drop() {
        // Send a response that is cut off mid-body (no content-length,
        // connection drops while body is still being written).
        // We advertise Content-Length: 10000 but only send 5 bytes before
        // closing the socket — ureq will surface an IO error.
        let (listener, port) = bind_local();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                // Lie about content-length, then drop before sending the full body.
                let partial_resp =
                    b"HTTP/1.0 200 OK\r\nContent-Length: 10000\r\n\r\nhello";
                let _ = stream.write_all(partial_resp);
                // Drop `stream` → connection closes prematurely.
            }
        });

        let dir = tempfile::tempdir().expect("tempdir");
        let dest: PathBuf = dir.path().join("truncated.bin");
        let url = format!("http://127.0.0.1:{}/truncated", port);

        // This SHOULD fail (connection drop mid-body).
        let result = download_to(&url, &dest);
        // dest must not exist.
        assert!(
            !dest.exists(),
            "dest must be absent when download_to fails: {:?}",
            result
        );
        // .partial must not remain.
        let mut partial_path = dest.as_os_str().to_os_string();
        partial_path.push(".partial");
        assert!(
            !Path::new(&partial_path).exists(),
            ".partial must be cleaned up on failure"
        );
    }

    // ── download_via_mirrors: first 404, second 200 ──────────────────────────

    #[test]
    fn test_download_via_mirrors_fallback() {
        // Mirror 1: 404
        let (listener1, port1) = bind_local();
        serve_once(listener1, not_found_response());

        // Mirror 2: 200
        let (listener2, port2) = bind_local();
        let body = b"from mirror 2";
        serve_once(listener2, ok_response(body));

        let mirrors = vec![
            format!("http://127.0.0.1:{}", port1),
            format!("http://127.0.0.1:{}", port2),
        ];
        let result = download_via_mirrors(&mirrors, "/pkg.tar.zst")
            .expect("should succeed via second mirror");
        assert_eq!(result, body);
    }

    #[test]
    fn test_download_via_mirrors_all_fail() {
        let (listener1, port1) = bind_local();
        serve_once(listener1, not_found_response());

        let (listener2, port2) = bind_local();
        serve_once(listener2, not_found_response());

        let mirrors = vec![
            format!("http://127.0.0.1:{}", port1),
            format!("http://127.0.0.1:{}", port2),
        ];
        match download_via_mirrors(&mirrors, "/pkg.tar.zst") {
            Err(FetchError::NoMirrors) => {}
            other => panic!("expected NoMirrors, got {:?}", other),
        }
    }

    // ── head_content_length ───────────────────────────────────────────────────

    #[test]
    fn test_head_content_length() {
        let (listener, port) = bind_local();
        serve_once(listener, head_response(42));

        let url = format!("http://127.0.0.1:{}/index.zst", port);
        let len = head_content_length(&url).expect("head should succeed");
        assert_eq!(len, Some(42));
    }

    // ── Network tests (skipped unless JPKG_FETCH_TESTS=1) ────────────────────

    #[test]
    fn test_download_network_real() {
        if std::env::var("JPKG_FETCH_TESTS").as_deref() != Ok("1") {
            eprintln!("skipped (set JPKG_FETCH_TESTS=1 to run)");
            return;
        }
        // Hit a reliable public endpoint; body must be non-empty.
        let bytes = download("https://httpbin.org/get").expect("network GET");
        assert!(!bytes.is_empty());
    }
}
