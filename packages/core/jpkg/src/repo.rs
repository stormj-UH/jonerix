/*
 * jpkg - jonerix package manager
 * repo.rs - Repository handling (INDEX fetch, verify, cache, package download)
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 *
 * Rust port of jpkg/src/repo.c.
 *
 * Cross-module deps:
 *   crate::recipe::Index / IndexEntry  — DONE
 *   crate::sign::PublicKeySet          — DONE
 *   crate::fetch::*                    — Worker F (running in parallel)
 *
 * On-mirror URL layout (matches publish-packages.yml + gen-index.sh):
 *   INDEX:   <mirror_base>/INDEX.zst
 *   SIG:     <mirror_base>/INDEX.zst.sig
 *   PACKAGE: <mirror_base>/<name>-<version>-<arch>.jpkg
 *   (legacy) <mirror_base>/<name>-<version>.jpkg
 *
 * The section key in INDEX is "name-arch" (see gen-index.sh line 151).
 * Package filenames on the release: name-version-arch.jpkg (pkg.c:pkg_filename).
 * No arch sub-directory prefix — everything flat under the release asset URL.
 */

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::recipe::Index;
use crate::sign::PublicKeySet;

// Real fetch layer (Worker F):
use crate::fetch::{download_via_mirrors, download_via_mirrors_to, FetchError};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum RepoError {
    Fetch(FetchError),
    Sign(crate::sign::SignError),
    Recipe(crate::recipe::RecipeError),
    Zstd(std::io::Error),
    Io(std::io::Error),
    NoMirrors,
    /// sig verify failed AND at least one key was loaded
    SignatureRejected,
    PackageNotFound { name: String, arch: String },
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::Fetch(e) => write!(f, "fetch error: {e}"),
            RepoError::Sign(e) => write!(f, "signature error: {e}"),
            RepoError::Recipe(e) => write!(f, "index parse error: {e}"),
            RepoError::Zstd(e) => write!(f, "zstd decompression error: {e}"),
            RepoError::Io(e) => write!(f, "I/O error: {e}"),
            RepoError::NoMirrors => write!(f, "no repository mirrors configured"),
            RepoError::SignatureRejected => {
                write!(f, "INDEX signature verification failed")
            }
            RepoError::PackageNotFound { name, arch } => {
                write!(f, "package not found: {name} ({arch})")
            }
        }
    }
}

impl std::error::Error for RepoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RepoError::Fetch(e) => Some(e),
            RepoError::Sign(e) => Some(e),
            RepoError::Recipe(e) => Some(e),
            RepoError::Zstd(e) => Some(e),
            RepoError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<FetchError> for RepoError {
    fn from(e: FetchError) -> Self {
        RepoError::Fetch(e)
    }
}

impl From<crate::sign::SignError> for RepoError {
    fn from(e: crate::sign::SignError) -> Self {
        RepoError::Sign(e)
    }
}

impl From<crate::recipe::RecipeError> for RepoError {
    fn from(e: crate::recipe::RecipeError) -> Self {
        RepoError::Recipe(e)
    }
}

impl From<std::io::Error> for RepoError {
    fn from(e: std::io::Error) -> Self {
        RepoError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// parse_mirrors_conf
// ---------------------------------------------------------------------------

/// Parse a `mirrors.conf`-style file: one URL per line, blank lines and
/// `#`-prefixed comments ignored.
pub fn parse_mirrors_conf(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_owned())
        .collect()
}

/// Parse the legacy `repos.conf` format used by older jonerix image builders:
///
/// ```text
/// [repo]
/// url = "https://example.invalid/packages"
/// ```
///
/// Rust jpkg's native config is `mirrors.conf`, but existing installers still
/// emit this file. Treat it as a fallback so pinned image builds do not
/// silently fall back to the rolling package release.
pub fn parse_repos_conf(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            if key.trim() != "url" {
                return None;
            }
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if value.is_empty() {
                None
            } else {
                Some(value.to_owned())
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Repo struct
// ---------------------------------------------------------------------------

/// Repository state: mirrors, signing keys, cache path, and target arch.
pub struct Repo {
    /// Mirror base URLs in priority order.
    pub mirrors: Vec<String>,
    /// Public-key set used for INDEX.zst.sig verification.
    pub keys: PublicKeySet,
    /// `/var/cache/jpkg` — where decompressed INDEX is cached and where
    /// fetched .jpkg files land before install.
    pub cache_dir: PathBuf,
    /// Architecture string (e.g. "x86_64", "aarch64").
    pub arch: String,
}

impl Repo {
    // ── Constructors ───────────────────────────────────────────────────────

    /// Manually construct a Repo from in-memory config (used by tests and by
    /// callers that want to inject a non-default arch).
    pub fn new(
        mirrors: Vec<String>,
        keys: PublicKeySet,
        cache_dir: PathBuf,
        arch: String,
    ) -> Self {
        Self {
            mirrors,
            keys,
            cache_dir,
            arch,
        }
    }

    /// Construct a Repo from on-disk config.  `rootfs` is the path prefix
    /// applied to all jpkg system paths (e.g. "/" in production, a tempdir in
    /// tests).
    ///
    /// Reads mirrors from `rootfs/etc/jpkg/mirrors.conf`, falling back to the
    /// legacy `rootfs/etc/jpkg/repos.conf` when needed.
    /// Loads keys from `rootfs/etc/jpkg/keys/`.
    /// Cache dir is `rootfs/var/cache/jpkg/`.
    pub fn from_rootfs(rootfs: &Path, arch: &str) -> Result<Self, RepoError> {
        // --- mirrors -------------------------------------------------------
        let mirrors_path = rootfs.join("etc/jpkg/mirrors.conf");
        let legacy_repos_path = rootfs.join("etc/jpkg/repos.conf");
        let mirrors = if mirrors_path.exists() {
            let text = std::fs::read_to_string(&mirrors_path)?;
            parse_mirrors_conf(&text)
        } else if legacy_repos_path.exists() {
            let text = std::fs::read_to_string(&legacy_repos_path)?;
            parse_repos_conf(&text)
        } else {
            // Default mirror matches the C fallback in repo_config_load.
            vec![
                "https://github.com/stormj-UH/jonerix/releases/download/packages"
                    .to_owned(),
            ]
        };

        // --- keys ----------------------------------------------------------
        let keys_dir = rootfs.join("etc/jpkg/keys");
        let keys = PublicKeySet::load_dir(&keys_dir)?;

        // --- cache dir -----------------------------------------------------
        let cache_dir = rootfs.join("var/cache/jpkg");

        Ok(Self {
            mirrors,
            keys,
            cache_dir,
            arch: arch.to_owned(),
        })
    }

    // ── Index operations ───────────────────────────────────────────────────

    /// Fetch INDEX.zst + INDEX.zst.sig from mirrors, verify sig (warn-and-proceed
    /// when `keys.is_empty()` per audit § 3), decompress with `zstd::decode_all`,
    /// parse TOML.  Caches the decompressed plaintext to `cache_dir/INDEX`
    /// atomically (write to a tempfile, then rename).
    pub fn fetch_index(&self) -> Result<Index, RepoError> {
        if self.mirrors.is_empty() {
            return Err(RepoError::NoMirrors);
        }

        std::fs::create_dir_all(&self.cache_dir)?;

        // --- fetch INDEX.zst -----------------------------------------------
        log::info!("fetching INDEX.zst from mirrors");
        let compressed = download_via_mirrors(&self.mirrors, "INDEX.zst")?;

        // --- fetch INDEX.zst.sig (best-effort) -----------------------------
        let sig_bytes: Option<Vec<u8>> = match download_via_mirrors(&self.mirrors, "INDEX.zst.sig")
        {
            Ok(b) => Some(b),
            Err(e) => {
                log::warn!("failed to fetch INDEX.zst.sig: {e}");
                None
            }
        };

        // --- verify signature ----------------------------------------------
        if self.keys.is_empty() {
            // Audit § 3: no keys configured → warn-and-proceed.
            log::warn!(
                "no public keys loaded; skipping INDEX signature verification"
            );
        } else if let Some(ref sig) = sig_bytes {
            // We verify the compressed bytes (that is what the C code does:
            // it passes cdata/clen — the raw INDEX.zst — to sign_verify_detached).
            match self.keys.verify_detached(&compressed, sig) {
                Ok(key_name) => {
                    log::info!("INDEX.zst signature verified (key: {key_name})");
                }
                Err(e) => {
                    log::error!("INDEX.zst signature verification failed: {e}");
                    return Err(RepoError::SignatureRejected);
                }
            }
        } else {
            // Keys are loaded but no sig file — treat as rejected.
            log::error!("keys are configured but INDEX.zst.sig is absent");
            return Err(RepoError::SignatureRejected);
        }

        // --- decompress ----------------------------------------------------
        let plain = zstd_decompress(&compressed)?;

        // --- atomic cache write -------------------------------------------
        let index_path = self.cache_dir.join("INDEX");
        atomic_write(&index_path, &plain)?;
        log::info!("INDEX cached to {}", index_path.display());

        // --- parse TOML ----------------------------------------------------
        let text = std::str::from_utf8(&plain)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let index = Index::parse(text)?;
        log::debug!("parsed {} INDEX entries", index.entries.len());
        Ok(index)
    }

    /// Read the cached decompressed INDEX from `cache_dir/INDEX` without
    /// touching the network.  Returns `None` if the cache file is absent.
    pub fn load_cached_index(&self) -> Result<Option<Index>, RepoError> {
        let index_path = self.cache_dir.join("INDEX");
        match std::fs::read_to_string(&index_path) {
            Ok(text) => {
                let index = Index::parse(&text)?;
                Ok(Some(index))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(RepoError::Io(e)),
        }
    }

    // ── Package download ───────────────────────────────────────────────────

    /// Fetch a single .jpkg file by package name into
    /// `cache_dir/<name>-<version>-<arch>.jpkg`.
    ///
    /// On-mirror path: `<name>-<version>-<arch>.jpkg` (flat under the
    /// release asset URL — no arch sub-directory).  Falls back to the legacy
    /// `<name>-<version>.jpkg` name if the arch-qualified fetch fails.
    ///
    /// Returns the path of the downloaded file.
    pub fn fetch_package(&self, name: &str, version: &str) -> Result<PathBuf, RepoError> {
        std::fs::create_dir_all(&self.cache_dir)?;

        // Current filename: name-version-arch.jpkg
        let filename = format!("{}-{}-{}.jpkg", name, version, self.arch);
        let dest = self.cache_dir.join(&filename);
        // Legacy fallback name (pre-arch era): name-version.jpkg
        let legacy_name = format!("{}-{}.jpkg", name, version);
        let legacy_dest = self.cache_dir.join(&legacy_name);

        // Cache short-circuit: if either filename is already present in
        // cache_dir, return that path without hitting the network.  Mirrors
        // the C `repo_fetch_package` cache-lookup at repo.c:498-516 — and
        // makes the install pipeline test-friendly when a synthetic .jpkg
        // is dropped into the cache directly.
        if dest.is_file() {
            log::info!("cache hit: {filename}");
            return Ok(dest);
        }
        if legacy_dest.is_file() {
            log::info!("cache hit: {legacy_name}");
            return Ok(legacy_dest);
        }

        if self.mirrors.is_empty() {
            return Err(RepoError::NoMirrors);
        }

        // Try arch-qualified name.
        match download_via_mirrors_to(&self.mirrors, &filename, &dest) {
            Ok(()) => {
                log::info!("downloaded {filename}");
                return Ok(dest);
            }
            Err(e) => {
                log::warn!("arch-qualified download failed ({e}), trying legacy name");
            }
        }

        // Legacy fallback: name-version.jpkg (pre-arch era).
        download_via_mirrors_to(&self.mirrors, &legacy_name, &legacy_dest)?;
        log::info!("downloaded legacy {legacy_name}");
        Ok(legacy_dest)
    }

    // ── Verification ───────────────────────────────────────────────────────

    /// Verify the sha256 of a downloaded .jpkg file against the INDEX entry.
    /// `expected_sha256` comes from `IndexEntry.sha256`.
    pub fn verify_package(jpkg_path: &Path, expected_sha256: &str) -> Result<(), RepoError> {
        use sha2::{Digest, Sha256};

        let data = std::fs::read(jpkg_path)?;
        let hash = Sha256::digest(&data);
        let got = hex::encode(hash);

        if got != expected_sha256 {
            return Err(RepoError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "sha256 mismatch for {}: expected {expected_sha256}, got {got}",
                    jpkg_path.display()
                ),
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Decompress zstd-compressed bytes.  Accepts plain-text input that lacks the
/// zstd magic (matches the C fallback at repo.c:381-453).
fn zstd_decompress(data: &[u8]) -> Result<Vec<u8>, RepoError> {
    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
    if data.len() >= 4 && data[..4] == ZSTD_MAGIC {
        let mut out = Vec::new();
        zstd::stream::read::Decoder::new(Cursor::new(data))
            .map_err(RepoError::Zstd)?
            .read_to_end(&mut out)
            .map_err(RepoError::Zstd)?;
        Ok(out)
    } else {
        // Plain-text INDEX (no zstd magic) — pass through as-is.
        log::debug!("INDEX is not zstd-compressed; using as plain text");
        Ok(data.to_vec())
    }
}

/// Write `data` to `path` atomically: write to a sibling tempfile then rename.
fn atomic_write(path: &Path, data: &[u8]) -> Result<(), std::io::Error> {
    let parent = path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let tmp = tempfile::Builder::new()
        .prefix(".INDEX.tmp")
        .tempfile_in(parent)?;
    std::fs::write(tmp.path(), data)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::IndexEntry;
    use crate::sign::{keygen, sign_detached, write_public_key, write_secret_key};
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::thread;
    use tempfile::TempDir;

    // ── HTTP test harness ──────────────────────────────────────────────────
    //
    // One-shot minimal HTTP/1.1 server.  Accepts a single GET, returns `body`,
    // then closes the connection.

    struct FakeServer {
        addr: std::net::SocketAddr,
        _handle: thread::JoinHandle<()>,
    }

    /// Serve a map of path → (status_code, body_bytes).
    fn fake_http_server(routes: Arc<std::collections::HashMap<String, (u16, Vec<u8>)>>) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");

        let handle = thread::spawn(move || {
            // Accept connections until all routes have been served once each.
            // For simplicity: accept up to routes.len()*2 + 4 connections.
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
                thread::spawn(move || {
                    serve_one(stream, &routes);
                });
            }
        });

        FakeServer { addr, _handle: handle }
    }

    fn serve_one(
        mut stream: std::net::TcpStream,
        routes: &std::collections::HashMap<String, (u16, Vec<u8>)>,
    ) {
        let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));

        // Read request line.
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        let request_line = request_line.trim();
        // "GET /path HTTP/1.1"
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .to_owned();

        // Drain headers.
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if line.trim().is_empty() {
                        break;
                    }
                }
            }
        }

        // Respond.
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

    // ── Helper: build a minimal Index ──────────────────────────────────────

    fn minimal_index(name: &str, arch: &str, version: &str) -> Index {
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

    // ── Test 1: parse_mirrors_conf ──────────────────────────────────────────

    #[test]
    fn test_parse_mirrors_conf_strips_comments_and_blanks() {
        let conf = r#"
# This is a comment
https://mirror1.example.com/packages

https://mirror2.example.com/packages
  # indented comment
  https://mirror3.example.com/packages
"#;
        let mirrors = parse_mirrors_conf(conf);
        assert_eq!(mirrors, vec![
            "https://mirror1.example.com/packages",
            "https://mirror2.example.com/packages",
            "https://mirror3.example.com/packages",
        ]);
    }

    #[test]
    fn test_parse_mirrors_conf_empty_input() {
        assert!(parse_mirrors_conf("").is_empty());
        assert!(parse_mirrors_conf("# only a comment\n").is_empty());
    }

    #[test]
    fn test_parse_repos_conf_legacy_url() {
        let conf = r#"
# legacy jpkg config
[repo]
url = "https://github.com/stormj-UH/jonerix/releases/download/v1.2.1"
"#;
        assert_eq!(
            parse_repos_conf(conf),
            vec!["https://github.com/stormj-UH/jonerix/releases/download/v1.2.1"],
        );
    }

    #[test]
    fn test_from_rootfs_uses_legacy_repos_conf() {
        let dir = TempDir::new().expect("tempdir");
        let etc_jpkg = dir.path().join("etc/jpkg");
        std::fs::create_dir_all(etc_jpkg.join("keys")).expect("mkdir keys");
        std::fs::write(
            etc_jpkg.join("repos.conf"),
            "[repo]\nurl = \"https://example.invalid/v1.2.1\"\n",
        )
        .expect("write repos.conf");

        let repo = Repo::from_rootfs(dir.path(), "aarch64").expect("repo");
        assert_eq!(repo.mirrors, vec!["https://example.invalid/v1.2.1"]);
    }

    #[test]
    fn test_from_rootfs_prefers_mirrors_conf() {
        let dir = TempDir::new().expect("tempdir");
        let etc_jpkg = dir.path().join("etc/jpkg");
        std::fs::create_dir_all(etc_jpkg.join("keys")).expect("mkdir keys");
        std::fs::write(
            etc_jpkg.join("repos.conf"),
            "[repo]\nurl = \"https://example.invalid/legacy\"\n",
        )
        .expect("write repos.conf");
        std::fs::write(
            etc_jpkg.join("mirrors.conf"),
            "https://example.invalid/native\n",
        )
        .expect("write mirrors.conf");

        let repo = Repo::from_rootfs(dir.path(), "aarch64").expect("repo");
        assert_eq!(repo.mirrors, vec!["https://example.invalid/native"]);
    }

    // ── Test 2: fetch_index — no sig file, keys empty → warn-and-accept ───

    #[test]
    fn test_fetch_index_no_sig_no_keys_warn_accept() {
        let index = minimal_index("toybox", "x86_64", "0.8.11");
        let index_toml = index.to_string().expect("serialise index");
        let compressed = zstd::encode_all(index_toml.as_bytes(), 3).expect("zstd encode");

        let mut routes: std::collections::HashMap<String, (u16, Vec<u8>)> =
            std::collections::HashMap::new();
        routes.insert("/INDEX.zst".to_owned(), (200, compressed));
        // /INDEX.zst.sig deliberately absent → 404
        let srv = fake_http_server(Arc::new(routes));

        let cache_dir = TempDir::new().expect("tempdir");
        let keys_dir = TempDir::new().expect("tempdir");
        let keys = PublicKeySet::load_dir(keys_dir.path()).expect("load_dir empty");
        assert!(keys.is_empty());

        let repo = Repo::new(
            vec![format!("http://127.0.0.1:{}", srv.addr.port())],
            keys,
            cache_dir.path().to_path_buf(),
            "x86_64".to_owned(),
        );

        let result = repo.fetch_index();
        assert!(
            result.is_ok(),
            "warn-and-accept when no keys: {:?}",
            result.err()
        );
        let fetched = result.unwrap();
        assert!(fetched.get("toybox", "x86_64").is_some());
    }

    // fetch_index — no sig file, keys loaded → SignatureRejected

    #[test]
    fn test_fetch_index_no_sig_keys_loaded_rejected() {
        let index = minimal_index("toybox", "x86_64", "0.8.11");
        let index_toml = index.to_string().expect("serialise index");
        let compressed = zstd::encode_all(index_toml.as_bytes(), 3).expect("zstd encode");

        let mut routes: std::collections::HashMap<String, (u16, Vec<u8>)> =
            std::collections::HashMap::new();
        routes.insert("/INDEX.zst".to_owned(), (200, compressed));

        let srv = fake_http_server(Arc::new(routes));

        let cache_dir = TempDir::new().expect("tempdir");
        let keys_dir = TempDir::new().expect("tempdir");

        // Generate and install a real key so the set is non-empty.
        let sk = keygen();
        write_public_key(&keys_dir.path().join("test.pub"), &sk.verifying_key())
            .expect("write pub");
        let keys = PublicKeySet::load_dir(keys_dir.path()).expect("load_dir");
        assert!(!keys.is_empty());

        let repo = Repo::new(
            vec![format!("http://127.0.0.1:{}", srv.addr.port())],
            keys,
            cache_dir.path().to_path_buf(),
            "x86_64".to_owned(),
        );

        let result = repo.fetch_index();
        assert!(
            matches!(result, Err(RepoError::SignatureRejected)),
            "expected SignatureRejected when sig absent and keys loaded"
        );
    }

    // ── Test 3: fetch_index round-trip (sign, serve, verify, parse) ────────

    #[test]
    fn test_fetch_index_roundtrip_with_signature() {
        // Build a rich Index.
        let index = minimal_index("musl", "x86_64", "1.2.5");
        let index_toml = index.to_string().expect("serialise index");
        let compressed = zstd::encode_all(index_toml.as_bytes(), 3).expect("zstd encode");

        // Sign the compressed bytes.
        let sk = keygen();
        let sig_bytes = sign_detached(&sk, &compressed);

        // Set up keys in a tempdir.
        let keys_dir = TempDir::new().expect("tempdir");
        write_public_key(&keys_dir.path().join("jonerix.pub"), &sk.verifying_key())
            .expect("write pub");
        write_secret_key(&keys_dir.path().join("jonerix.sec"), &sk).expect("write sec");

        // Serve INDEX.zst + INDEX.zst.sig.
        let mut routes: std::collections::HashMap<String, (u16, Vec<u8>)> =
            std::collections::HashMap::new();
        routes.insert("/INDEX.zst".to_owned(), (200, compressed.clone()));
        routes.insert("/INDEX.zst.sig".to_owned(), (200, sig_bytes.to_vec()));

        let srv = fake_http_server(Arc::new(routes));

        let cache_dir = TempDir::new().expect("tempdir");
        let keys = PublicKeySet::load_dir(keys_dir.path()).expect("load_dir");

        let repo = Repo::new(
            vec![format!("http://127.0.0.1:{}", srv.addr.port())],
            keys,
            cache_dir.path().to_path_buf(),
            "x86_64".to_owned(),
        );

        let fetched = repo.fetch_index().expect("fetch_index must succeed");

        // Assert round-trip equality.
        let orig_entry = index.get("musl", "x86_64").expect("musl-x86_64 in original");
        let got_entry = fetched.get("musl", "x86_64").expect("musl-x86_64 in fetched");
        assert_eq!(orig_entry.version, got_entry.version);
        assert_eq!(orig_entry.license, got_entry.license);

        // Cache file must exist.
        assert!(
            cache_dir.path().join("INDEX").exists(),
            "INDEX must be cached on disk"
        );
    }

    // ── Test 4: load_cached_index ───────────────────────────────────────────

    #[test]
    fn test_load_cached_index_present() {
        let cache_dir = TempDir::new().expect("tempdir");
        let index = minimal_index("zstd", "aarch64", "1.5.6");
        let toml_text = index.to_string().expect("serialise");

        std::fs::write(cache_dir.path().join("INDEX"), &toml_text)
            .expect("write cached INDEX");

        let repo = Repo::new(
            vec![],
            PublicKeySet::load_dir(&cache_dir.path().join("nokeys")).unwrap_or_else(|_| {
                PublicKeySet::load_dir(cache_dir.path()).expect("fallback load_dir")
            }),
            cache_dir.path().to_path_buf(),
            "aarch64".to_owned(),
        );

        let cached = repo.load_cached_index().expect("load_cached_index");
        assert!(cached.is_some(), "must return Some when cache file exists");
        let idx = cached.unwrap();
        assert!(idx.get("zstd", "aarch64").is_some());
    }

    #[test]
    fn test_load_cached_index_absent() {
        let cache_dir = TempDir::new().expect("tempdir");
        let repo = Repo::new(
            vec![],
            PublicKeySet::load_dir(&cache_dir.path().join("nokeys")).unwrap_or_else(|_| {
                PublicKeySet::load_dir(cache_dir.path()).expect("fallback load_dir")
            }),
            cache_dir.path().to_path_buf(),
            "x86_64".to_owned(),
        );

        let cached = repo.load_cached_index().expect("load_cached_index should not error");
        assert!(cached.is_none(), "must return None when cache file absent");
    }

    // ── Test 5: verify_package ─────────────────────────────────────────────

    #[test]
    fn test_verify_package_ok_on_match() {
        let dir = TempDir::new().expect("tempdir");
        let data = b"fake jpkg payload";
        let path = dir.path().join("test.jpkg");
        std::fs::write(&path, data).expect("write");

        let expected = hex::encode(Sha256::digest(data));
        Repo::verify_package(&path, &expected).expect("must be Ok on hash match");
    }

    #[test]
    fn test_verify_package_err_on_mismatch() {
        let dir = TempDir::new().expect("tempdir");
        let data = b"fake jpkg payload";
        let path = dir.path().join("test.jpkg");
        std::fs::write(&path, data).expect("write");

        let result = Repo::verify_package(&path, "deadbeefdeadbeef");
        assert!(result.is_err(), "must return Err on hash mismatch");
    }

    // ── Test 6: mirror failover ─────────────────────────────────────────────

    #[test]
    fn test_fetch_index_mirror_failover() {
        // Build a valid INDEX.
        let index = minimal_index("mksh", "x86_64", "R59c");
        let index_toml = index.to_string().expect("serialise");
        let compressed = zstd::encode_all(index_toml.as_bytes(), 3).expect("zstd encode");

        // Good server: serves INDEX.zst.
        let mut good_routes: std::collections::HashMap<String, (u16, Vec<u8>)> =
            std::collections::HashMap::new();
        good_routes.insert("/INDEX.zst".to_owned(), (200, compressed));
        // No .sig served → warn-and-accept (keys will be empty).
        let good_srv = fake_http_server(Arc::new(good_routes));

        // Bad server: returns 404 for everything.
        let bad_routes: std::collections::HashMap<String, (u16, Vec<u8>)> =
            std::collections::HashMap::new();
        let bad_srv = fake_http_server(Arc::new(bad_routes));

        let cache_dir = TempDir::new().expect("tempdir");
        let keys = PublicKeySet::load_dir(&cache_dir.path().join("nokeys"))
            .unwrap_or_else(|_| PublicKeySet::load_dir(cache_dir.path()).expect("load_dir"));

        // Put the bad mirror first.
        let repo = Repo::new(
            vec![
                format!("http://127.0.0.1:{}", bad_srv.addr.port()),
                format!("http://127.0.0.1:{}", good_srv.addr.port()),
            ],
            keys,
            cache_dir.path().to_path_buf(),
            "x86_64".to_owned(),
        );

        let result = repo.fetch_index();
        assert!(
            result.is_ok(),
            "second mirror must succeed after first returns 404: {}",
            result.as_ref().err().map(|e| e.to_string()).unwrap_or_default()
        );
        let idx = result.unwrap();
        assert!(idx.get("mksh", "x86_64").is_some());
    }
}
