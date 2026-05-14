// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::repo::{Repo, RepoError};

// ---------------------------------------------------------------------------
// Public entry point (wired by main dispatcher)
// ---------------------------------------------------------------------------

/// Run the `jpkg update` subcommand.
///
/// Fetches `INDEX.zst` and `INDEX.zst.sig` from the configured mirrors,
/// verifies the signature, decompresses the index, and writes it to the
/// cache at `$JPKG_ROOT/var/cache/jpkg/INDEX`.  Returns 0 on success,
/// 1 on error, or 2 on usage error.
pub fn run(args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("jpkg update: unexpected argument(s): {:?}", args);
        eprintln!("usage: jpkg update");
        return 2;
    }

    let rootfs = std::env::var("JPKG_ROOT").unwrap_or_else(|_| "/".to_string());

    // Require root when writing to the real system cache.
    if rootfs == "/" {
        if !nix::unistd::Uid::effective().is_root() {
            eprintln!("jpkg update: must run as root (or set --root)");
            return 1;
        }
    }

    let arch = std::env::consts::ARCH;

    let repo = match Repo::from_rootfs(Path::new(&rootfs), arch) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };

    run_with_repo(repo)
}

// ---------------------------------------------------------------------------
// Testable inner function — accepts an already-constructed Repo
// ---------------------------------------------------------------------------

pub(crate) fn run_with_repo(repo: Repo) -> i32 {
    let mirror_hint = repo.mirrors.first().cloned().unwrap_or_default();
    let old_timestamp = cached_index_timestamp(&repo);

    match repo.fetch_index() {
        Ok(index) => {
            print_index_timestamps(
                old_timestamp.as_deref(),
                cached_index_timestamp(&repo).as_deref(),
            );
            if mirror_hint.is_empty() {
                println!("Updated package index");
            } else {
                println!("Updated package index from {mirror_hint}");
            }
            println!("{} packages indexed", index.entries.len());
            0
        }
        Err(RepoError::NoMirrors) => {
            eprintln!("no mirrors configured (write /etc/jpkg/mirrors.conf)");
            1
        }
        Err(RepoError::SignatureRejected) => {
            eprintln!("INDEX signature verification FAILED");
            1
        }
        Err(RepoError::Fetch(ref e)) => {
            eprintln!("failed to fetch index from any mirror: {e}");
            1
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn cached_index_timestamp(repo: &Repo) -> Option<String> {
    let index_path = repo.cache_dir.join("INDEX");
    let text = std::fs::read_to_string(index_path).ok()?;
    index_timestamp_from_text(&text)
}

fn index_timestamp_from_text(text: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(text).ok()?;
    value
        .get("meta")?
        .get("timestamp")?
        .as_str()
        .map(str::to_owned)
}

fn print_index_timestamps(old: Option<&str>, new: Option<&str>) {
    match old {
        Some(ts) => println!("Existing INDEX timestamp: {ts}"),
        None => println!("Existing INDEX timestamp: none"),
    }

    match (old, new) {
        (_, Some(ts)) if old == Some(ts) => {
            println!("Fetched INDEX timestamp: {ts} (unchanged)");
        }
        (Some(old_ts), Some(new_ts)) => {
            println!("Fetched INDEX timestamp: {new_ts} (was {old_ts})");
        }
        (None, Some(new_ts)) => {
            println!("Fetched INDEX timestamp: {new_ts}");
        }
        (_, None) => {
            println!("Fetched INDEX timestamp: unknown");
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{Index, IndexEntry};
    use crate::sign::{keygen, sign_detached, write_public_key, PublicKeySet};
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;
    use tempfile::TempDir;

    // ── Minimal HTTP test harness (mirrors repo.rs tests pattern) ──────────

    struct FakeServer {
        addr: std::net::SocketAddr,
        _handle: thread::JoinHandle<()>,
    }

    fn fake_http_server(
        routes: Arc<std::collections::HashMap<String, (u16, Vec<u8>)>>,
    ) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");

        let handle = thread::spawn(move || {
            let max_conns = routes.len() * 2 + 4;
            let mut count = 0;
            for stream in listener.incoming() {
                count += 1;
                if count > max_conns {
                    break;
                }
                let stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let routes = Arc::clone(&routes);
                thread::spawn(move || serve_one(stream, &routes));
            }
        });

        FakeServer {
            addr,
            _handle: handle,
        }
    }

    fn serve_one(
        mut stream: std::net::TcpStream,
        routes: &std::collections::HashMap<String, (u16, Vec<u8>)>,
    ) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone"));

        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        let path = request_line
            .trim()
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .to_owned();

        // Drain headers.
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if line.trim().is_empty() => break,
                Ok(_) => {}
            }
        }

        let (status, body) = routes
            .get(&path)
            .cloned()
            .unwrap_or_else(|| (404, b"not found".to_vec()));

        let status_text = if status == 200 { "OK" } else { "Not Found" };
        let header = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
    }

    // ── Helper: build a minimal valid INDEX ────────────────────────────────

    fn make_index(name: &str, arch: &str, version: &str) -> Index {
        let key = format!("{name}-{arch}");
        let entry = IndexEntry {
            version: version.to_owned(),
            license: "MIT".to_owned(),
            description: "test package".to_owned(),
            arch: arch.to_owned(),
            sha256: "abc123".to_owned(),
            size: 42,
            depends: vec![],
            build_depends: vec![],
        };
        let mut entries = BTreeMap::new();
        entries.insert(key, entry);
        Index { entries }
    }

    fn compressed_index(name: &str, arch: &str, version: &str) -> Vec<u8> {
        let index = make_index(name, arch, version);
        let toml = index.to_string().expect("serialise index");
        zstd::encode_all(toml.as_bytes(), 3).expect("zstd encode")
    }

    fn compressed_index_with_timestamp(
        name: &str,
        arch: &str,
        version: &str,
        timestamp: &str,
    ) -> Vec<u8> {
        let index = make_index(name, arch, version);
        let mut toml = String::new();
        toml.push_str("[meta]\n");
        toml.push_str(&format!("timestamp = \"{timestamp}\"\n\n"));
        toml.push_str(&index.to_string().expect("serialise index"));
        zstd::encode_all(toml.as_bytes(), 3).expect("zstd encode")
    }

    #[test]
    fn test_index_timestamp_from_text_reads_meta_timestamp() {
        let text = r#"
[meta]
timestamp = "2026-05-14T16:30:00Z"

[musl-x86_64]
version = "1.2.6-r2"
license = "MIT"
description = "test"
arch = "x86_64"
sha256 = "abc123"
size = 42
"#;

        assert_eq!(
            index_timestamp_from_text(text).as_deref(),
            Some("2026-05-14T16:30:00Z")
        );
    }

    // ── Test 1: no mirrors.conf → NoMirrors → exit 1 ──────────────────────
    //
    // from_rootfs falls back to the GitHub default mirror when mirrors.conf is
    // absent.  To force NoMirrors we construct the Repo directly with an empty
    // mirror list, which matches what a real system would produce if mirrors.conf
    // existed but was empty.

    #[test]
    fn test_update_no_mirrors_exit1() {
        let cache_dir = TempDir::new().expect("tempdir");

        // Construct Repo with zero mirrors to force RepoError::NoMirrors.
        let repo = Repo::new(
            vec![],
            PublicKeySet::load_dir(&cache_dir.path().join("nokeys"))
                .unwrap_or_else(|_| PublicKeySet::load_dir(cache_dir.path()).expect("fallback")),
            cache_dir.path().to_path_buf(),
            "x86_64".to_owned(),
        );

        let exit = run_with_repo(repo);
        assert_eq!(exit, 1, "empty mirror list must yield exit 1");
    }

    // ── Test 2: real fetch via local server → exit 0, cache file exists ───

    #[test]
    fn test_update_fetch_success_exit0() {
        let compressed =
            compressed_index_with_timestamp("musl", "x86_64", "1.2.5", "2026-05-14T16:30:00Z");

        // Sign the index with a freshly generated key.
        let sk = keygen();
        let sig_bytes = sign_detached(&sk, &compressed);

        // Set up the fake server.
        let mut routes: std::collections::HashMap<String, (u16, Vec<u8>)> =
            std::collections::HashMap::new();
        routes.insert("/INDEX.zst".to_owned(), (200, compressed));
        routes.insert("/INDEX.zst.sig".to_owned(), (200, sig_bytes.to_vec()));
        let srv = fake_http_server(Arc::new(routes));

        // Write the public key into a tempdir.
        let keys_dir = TempDir::new().expect("tempdir");
        write_public_key(&keys_dir.path().join("test.pub"), &sk.verifying_key())
            .expect("write pub key");
        let keys = PublicKeySet::load_dir(keys_dir.path()).expect("load_dir");

        let cache_dir = TempDir::new().expect("tempdir");
        let repo = Repo::new(
            vec![format!("http://127.0.0.1:{}", srv.addr.port())],
            keys,
            cache_dir.path().to_path_buf(),
            "x86_64".to_owned(),
        );

        let exit = run_with_repo(repo);
        assert_eq!(exit, 0, "successful fetch must yield exit 0");

        // Cache file must have been written.
        assert!(
            cache_dir.path().join("INDEX").exists(),
            "INDEX cache file must exist after update"
        );
    }

    // ── Test 3: extra args → exit 2 ───────────────────────────────────────

    #[test]
    fn test_update_extra_args_exit2() {
        // We can call run() directly here; the arg-rejection check happens
        // before any root or repo logic.
        let exit = run(&["foo".to_string()]);
        assert_eq!(exit, 2, "extra args must yield exit 2");
    }
}
