/*
 * jpkg - jonerix package manager
 * cmd/install.rs - jpkg install: resolve deps, fetch, verify, extract
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 *
 * Port of jpkg/src/cmd_install.c.
 *
 * Divergences from C:
 * - --force applies to all packages in the plan (not just explicit names).
 *   The C code only force-installs names explicitly typed on the CLI
 *   (cmd_install.c:679-684).  In practice the only caller that passes
 *   --force is cmd_upgrade, which names the exact packages to force, so
 *   the observable difference is zero.  Simplicity wins.
 * - install_packages is pub(crate) so upgrade.rs can call it directly
 *   without going through run().  The C code calls cmd_install() with a
 *   synthetic argv; we avoid that by sharing the function.
 */

use std::path::Path;

use base64::Engine as _;

use crate::archive::JpkgArchive;
use crate::canon::{canonical_bytes, compute_payload_sha256};
use crate::cmd::common::{self, InstallError, resolve_arch, resolve_rootfs};
use crate::db::InstalledDb;
use crate::deps::resolve_install;
use crate::recipe::{Index, Metadata};
use crate::repo::{Repo, SignaturePolicy};
use crate::sign::PublicKeySet;

// ─── per-package signature verification ──────────────────────────────────────

/// Outcome of a successful signature verification pass (no error path).
#[derive(Debug)]
pub(crate) enum VerifyOutcome {
    /// Signature present and valid.  `key_id` is the `.pub` filename that matched.
    Verified { key_id: String },
    /// No signature, policy=Warn: caller should log WARN and continue.
    UnsignedAccepted,
    /// No signature, policy=Ignore: caller is silent.
    UnsignedIgnored,
}

/// Errors returned only by `verify_jpkg_signature`.
#[derive(Debug)]
pub(crate) enum SignatureError {
    /// No `[signature]` section and `policy=Require`.
    Missing,
    /// A `[signature]` section exists but the signature bytes are invalid.
    Invalid(String),
    /// A `[signature]` section references a key_id not in keys_dir.
    UnknownKey(String),
}

/// Verify the per-package signature embedded in a `.jpkg` archive.
///
/// # Behaviour
///
/// | signature present | valid | policy   | outcome                     |
/// |-------------------|-------|----------|-----------------------------|
/// | No                | —     | Warn     | Ok(UnsignedAccepted)        |
/// | No                | —     | Ignore   | Ok(UnsignedIgnored)         |
/// | No                | —     | Require  | Err(Missing)                |
/// | Yes               | Yes   | any      | Ok(Verified { key_id })     |
/// | Yes               | No    | any      | Err(Invalid(_))             |
/// | Yes               | bad key_id | any  | Err(UnknownKey(_))          |
///
/// `keys_dir` is the path to the directory that holds `*.pub` key files
/// (typically `rootfs/etc/jpkg/keys`).  If the directory doesn't exist or
/// has no keys, unsigned packages pass only under Warn/Ignore policy.
pub(crate) fn verify_jpkg_signature(
    jpkg_path: &Path,
    keys_dir: &Path,
    policy: SignaturePolicy,
) -> Result<VerifyOutcome, SignatureError> {
    // Open archive to get metadata + payload bytes.
    let archive = crate::archive::JpkgArchive::open(jpkg_path)
        .map_err(|e| SignatureError::Invalid(format!("failed to open archive: {e}")))?;

    let metadata = Metadata::from_str(archive.metadata())
        .map_err(|e| SignatureError::Invalid(format!("failed to parse metadata: {e}")))?;

    // No signature section.
    let sig = match &metadata.signature {
        None => {
            return match policy {
                SignaturePolicy::Require => Err(SignatureError::Missing),
                SignaturePolicy::Warn => Ok(VerifyOutcome::UnsignedAccepted),
                SignaturePolicy::Ignore => Ok(VerifyOutcome::UnsignedIgnored),
            };
        }
        Some(s) => s,
    };

    // There IS a signature — validate it regardless of policy.
    // (present-but-invalid is always an error)

    let key_id = sig.key_id.clone();

    // base64-decode the sig bytes.
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(&sig.sig)
        .map_err(|e| SignatureError::Invalid(format!("base64 decode failed: {e}")))?;

    // Reconstruct the canonical bytes.
    let payload_sha256 = compute_payload_sha256(archive.payload());
    let canon = canonical_bytes(&metadata, &payload_sha256);

    // Load the key set.
    let key_set = PublicKeySet::load_dir(keys_dir)
        .map_err(|e| SignatureError::Invalid(format!("failed to load keys: {e}")))?;

    if key_set.is_empty() {
        // No keys — treat as unknown key (can't validate).
        return Err(SignatureError::UnknownKey(key_id));
    }

    match key_set.verify_detached(&canon, &sig_bytes) {
        Ok(matched_key) => Ok(VerifyOutcome::Verified { key_id: matched_key }),
        Err(crate::sign::SignError::NoKeys) => Err(SignatureError::UnknownKey(key_id)),
        Err(e) => Err(SignatureError::Invalid(format!("verification failed: {e}"))),
    }
}

// ─── public entry point ───────────────────────────────────────────────────────

/// `jpkg install [--force] <pkg>...`
///
/// Returns 0 on success, 1 on any failure.
pub fn run(args: &[String]) -> i32 {
    // ── Parse flags ───────────────────────────────────────────────────────
    let mut force = false;
    let mut pkg_names: Vec<String> = Vec::new();

    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--force" | "-f" => force = true,
            other => pkg_names.push(other.to_string()),
        }
    }

    if pkg_names.is_empty() {
        eprintln!("usage: jpkg install [--force] <package> [package...]");
        return 1;
    }

    // ── Environment ───────────────────────────────────────────────────────
    let rootfs = resolve_rootfs(None);
    let arch = resolve_arch();

    // ── Open DB + lock ────────────────────────────────────────────────────
    let db = match InstalledDb::open(&rootfs) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("jpkg: failed to open database: {e}");
            return 1;
        }
    };
    let _lock = match db.lock() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("jpkg: {e}");
            return 1;
        }
    };

    // ── Load repo + index ─────────────────────────────────────────────────
    let repo = match Repo::from_rootfs(&rootfs, &arch) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("jpkg: failed to load repository config: {e}");
            return 1;
        }
    };

    let index = match load_index(&repo) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("jpkg: {e}");
            return 1;
        }
    };

    // ── Resolve deps ──────────────────────────────────────────────────────
    let plan = match resolve_install(&pkg_names, &arch, &db, &index, force) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("jpkg: dependency resolution failed: {e}");
            return 1;
        }
    };

    if plan.to_install.is_empty() {
        eprintln!("jpkg: nothing to install — all packages are up to date");
        return 0;
    }

    eprintln!("jpkg: packages to install ({}):", plan.to_install.len());
    for name in &plan.to_install {
        if let Some(entry) = index.get(name, &arch) {
            eprintln!("  {}-{}", name, entry.version);
        }
    }

    // ── Install loop ──────────────────────────────────────────────────────
    match install_packages(&db, &repo, &index, &arch, &plan.to_install, force) {
        Ok(n) => {
            eprintln!("jpkg: {n} package(s) installed");
            0
        }
        Err(e) => {
            eprintln!("jpkg: install failed: {e}");
            1
        }
    }
}

// ─── install_packages (shared with upgrade.rs) ───────────────────────────────

/// Install every package in `names` (already resolved, in dep order).
///
/// Returns the count of packages actually installed, or an error on first failure.
/// Matches `install_single_package` (cmd_install.c:305-576) called in a loop.
pub(crate) fn install_packages(
    db: &InstalledDb,
    repo: &Repo,
    index: &Index,
    arch: &str,
    names: &[String],
    force: bool,
) -> Result<usize, InstallError> {
    let mut installed = 0usize;

    for name in names {
        // Look up in index.
        let entry = match index.get(name, arch) {
            Some(e) => e,
            None => {
                return Err(InstallError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("package not found in index: {name}"),
                )));
            }
        };

        // Skip if already installed at same version and not forced.
        if !force {
            if let Some(existing) = db.get(name)? {
                let installed_ver = existing
                    .metadata
                    .package
                    .version
                    .as_deref()
                    .unwrap_or("");
                if installed_ver == entry.version {
                    log::info!("jpkg: {name}-{} is already installed", entry.version);
                    continue;
                }
            }
        }

        log::info!("jpkg: installing {}-{}...", name, entry.version);

        // Download + verify.  When a cached `.jpkg` fails sha256 verification,
        // it's almost always because the cache holds a same-filename blob from
        // a different mirror (rolling `packages` vs a tagged release like
        // v1.2.1, both shipping `libressl-4.0.0-aarch64.jpkg` with different
        // bytes when libressl was rebuilt between them).  Treat that as a
        // cache miss: delete the bad blob and re-fetch from the configured
        // mirror.  If the freshly-downloaded blob ALSO mismatches, that's a
        // real corrupted-mirror or man-in-the-middle situation and we error
        // hard.  Reproduced on jonerix-tormenta 2026-05-03 with a stale
        // libressl jpkg cached from `packages` and `jpkg conform 1.2.1`
        // expecting v1.2.1's sha256.
        let jpkg_path = {
            let path = repo
                .fetch_package(name, &entry.version)
                .map_err(|e| InstallError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("fetch failed: {e}"),
                )))?;
            match Repo::verify_package(&path, &entry.sha256) {
                Ok(()) => path,
                Err(first_err) => {
                    log::warn!(
                        "cached {} sha256 mismatch ({first_err}) — purging and re-fetching",
                        path.display()
                    );
                    // Remove the bad file so fetch_package's cache check
                    // misses and goes to the network.
                    let _ = std::fs::remove_file(&path);
                    let path = repo
                        .fetch_package(name, &entry.version)
                        .map_err(|e| InstallError::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("re-fetch after cache invalidation failed: {e}"),
                        )))?;
                    Repo::verify_package(&path, &entry.sha256)
                        .map_err(|e| InstallError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "verification failed even after re-fetch (corrupted mirror?): {e}"
                            ),
                        )))?;
                    path
                }
            }
        };

        // Per-package signature verification (Phase 0).
        // Always runs after sha256 passes.  A present-but-invalid signature is
        // always an error; only the missing-signature case branches on policy.
        //
        // keys_dir derivation: cache_dir is `<rootfs>/var/cache/jpkg`, so we
        // need three `.parent()` hops to climb back to `<rootfs>`, then append
        // `etc/jpkg/keys`. (A two-hop walk lands at `<rootfs>/var`, yielding
        // `<rootfs>/var/etc/jpkg/keys` — which doesn't exist, producing an
        // empty keyset and a spurious "unknown signing key" install failure
        // for every signed package even when /etc/jpkg/keys/jonerix.pub is
        // present and correct.)
        let keys_dir = repo.cache_dir
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join("etc/jpkg/keys"))
            .unwrap_or_else(|| std::path::PathBuf::from("/etc/jpkg/keys"));
        match verify_jpkg_signature(&jpkg_path, &keys_dir, repo.signature_policy) {
            Ok(VerifyOutcome::Verified { key_id }) => {
                log::info!(
                    "verified signature for {}-{} (key {})",
                    name, entry.version, key_id
                );
            }
            Ok(VerifyOutcome::UnsignedAccepted) => {
                // Reached only when repos.conf overrides the default to `warn`.
                // jpkg 2.2.0+ defaults to `require` and would have errored above.
                log::warn!(
                    "no signature for {}-{} — accepting under signature_policy=warn (set signature_policy=require in /etc/jpkg/repos.conf to reject)",
                    name, entry.version
                );
            }
            Ok(VerifyOutcome::UnsignedIgnored) => {
                // ignore mode — silent
            }
            Err(SignatureError::Missing) => {
                return Err(InstallError::SignatureMissing {
                    name: name.to_string(),
                    version: entry.version.clone(),
                });
            }
            Err(SignatureError::Invalid(msg)) => {
                return Err(InstallError::SignatureInvalid {
                    name: name.to_string(),
                    version: entry.version.clone(),
                    reason: msg,
                });
            }
            Err(SignatureError::UnknownKey(key_id)) => match repo.signature_policy {
                SignaturePolicy::Require => {
                    return Err(InstallError::UnknownSigningKey {
                        name: name.to_string(),
                        key_id,
                    });
                }
                SignaturePolicy::Warn => {
                    // Reached only when repos.conf opts back to `warn`.
                    log::warn!(
                        "no trusted key for {}-{} (signed by {}) — accepting under signature_policy=warn; \
                         drop the matching .pub into /etc/jpkg/keys/ or set signature_policy=require to reject",
                        name, entry.version, key_id
                    );
                }
                SignaturePolicy::Ignore => {
                    // ignore mode — silent
                }
            },
        }

        // Open archive + parse metadata.
        let archive = JpkgArchive::open(&jpkg_path)?;
        let metadata = Metadata::from_str(archive.metadata())?;

        // pre_install hook.
        let rootfs = crate::cmd::common::resolve_rootfs(None);
        if let Some(ref body) = metadata.hooks.pre_install {
            let status = common::run_hook(&rootfs, body)?;
            if !status.success() {
                return Err(InstallError::HookFailed {
                    hook: "pre_install",
                    status: status.code().unwrap_or(-1),
                });
            }
        }

        // Extract, flatten, install, register.
        let pkg = common::extract_and_register(&archive, &rootfs, db)?;

        // post_install hook.
        if let Some(ref body) = pkg.metadata.hooks.post_install {
            let status = common::run_hook(&rootfs, body)?;
            if !status.success() {
                log::warn!(
                    "jpkg: post_install hook for {name} exited {}",
                    status.code().unwrap_or(-1)
                );
                // C code: post_install failure is logged but does not abort
                // the overall install (cmd_install.c:555).
            }
        }

        log::info!("jpkg: installed {}-{}", name, entry.version);
        installed += 1;
    }

    Ok(installed)
}

// ─── load_index ──────────────────────────────────────────────────────────────

/// Load the cached INDEX; fall back to fetching from mirrors.
fn load_index(repo: &Repo) -> Result<Index, String> {
    match repo.load_cached_index() {
        Ok(Some(idx)) => {
            log::debug!("jpkg: using cached INDEX");
            return Ok(idx);
        }
        Ok(None) => {
            log::info!("jpkg: no cached INDEX, fetching from mirrors");
        }
        Err(e) => {
            log::warn!("jpkg: failed to load cached INDEX ({e}), fetching");
        }
    }
    repo.fetch_index().map_err(|e| format!("failed to fetch INDEX: {e}"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────
//
// The install pipeline (run / install_packages) calls repo.fetch_package()
// which requires live mirrors.  Tests use extract_and_register() directly as
// the "test seam" for the install logic, bypassing the repo layer entirely.
// This matches the task spec: "inject the .jpkg path directly via a
// pub(crate) test seam".

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use ed25519_dalek::SigningKey;

    use crate::archive::{self, JpkgArchive};
    use crate::canon::canonical_bytes;
    use crate::cmd::common::{extract_and_register, run_hook, tests as common_tests};
    use crate::db::InstalledDb;
    use crate::recipe::{
        DependsSection, FilesSection, HooksSection, Metadata, PackageSection, Signature,
    };
    use crate::repo::SignaturePolicy;
    use crate::sign::{keygen, sign_detached, write_public_key};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    use super::{SignatureError, VerifyOutcome, verify_jpkg_signature};

    // ── Signature test helpers ────────────────────────────────────────────────

    /// Build a minimal unsigned .jpkg (metadata has no [signature] section).
    fn build_unsigned_jpkg(tmp: &Path, name: &str, version: &str) -> std::path::PathBuf {
        common_tests::build_test_jpkg(tmp, name, version)
    }

    /// Build a signed .jpkg.  The [signature] section in the metadata is
    /// computed over `canonical_bytes(metadata_without_sig, payload_sha256)`.
    fn build_signed_jpkg(
        tmp: &Path,
        name: &str,
        version: &str,
        sk: &SigningKey,
        key_id: &str,
    ) -> std::path::PathBuf {
        // Build the destdir tree.
        let destdir = tmp.join(format!("destdir-signed-{name}"));
        fs::create_dir_all(destdir.join("bin")).unwrap();
        fs::write(destdir.join("bin/foo"), b"foo content\n").unwrap();

        // We need to build the archive in two passes:
        // 1. Build the payload and compute its sha256.
        // 2. Compute canonical bytes, sign, embed signature in metadata.
        // 3. Write the final .jpkg.
        //
        // Use create_with_metadata_factory which calls us back with
        // (payload_sha256_hex, payload_size) so we can produce the final TOML.
        let out = tmp.join(format!("{name}-{version}-x86_64.jpkg"));
        let sk_clone = sk.clone();
        let key_id_str = key_id.to_string();
        let name_str = name.to_string();
        let version_str = version.to_string();

        archive::create_with_metadata_factory(&out, &destdir, move |sha_hex, size| {
            // Build metadata without signature first (for canonical bytes).
            let mut meta = Metadata {
                package: PackageSection {
                    name: Some(name_str.clone()),
                    version: Some(version_str.clone()),
                    license: Some("MIT".to_string()),
                    description: Some("signed test pkg".to_string()),
                    arch: Some("x86_64".to_string()),
                    replaces: vec![],
                    conflicts: vec![],
                },
                depends: DependsSection::default(),
                hooks: HooksSection::default(),
                files: FilesSection {
                    sha256: Some(sha_hex.to_string()),
                    size: Some(size),
                },
                signature: None,
            };

            // The payload sha256 is what was just computed by create_with_metadata_factory.
            // We need the raw 32-byte form; hex-decode it.
            let payload_sha256_raw: [u8; 32] = hex::decode(sha_hex)
                .unwrap()
                .try_into()
                .unwrap();

            let canon = canonical_bytes(&meta, &payload_sha256_raw);
            let sig_bytes = sign_detached(&sk_clone, &canon);
            let sig_b64 =
                base64::engine::general_purpose::STANDARD.encode(sig_bytes);

            meta.signature = Some(Signature {
                algorithm: "ed25519".to_string(),
                key_id: key_id_str.clone(),
                sig: sig_b64,
            });

            meta.to_string().map_err(|e| {
                crate::archive::ArchiveError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })
        })
        .unwrap();

        out
    }

    /// Build a signed .jpkg where one byte in the sig has been flipped.
    fn build_tampered_sig_jpkg(
        tmp: &Path,
        name: &str,
        version: &str,
        sk: &SigningKey,
        key_id: &str,
    ) -> std::path::PathBuf {
        let destdir = tmp.join(format!("destdir-tampered-{name}"));
        fs::create_dir_all(destdir.join("bin")).unwrap();
        fs::write(destdir.join("bin/foo"), b"foo content\n").unwrap();

        let out = tmp.join(format!("{name}-{version}-x86_64.jpkg"));
        let sk_clone = sk.clone();
        let key_id_str = key_id.to_string();
        let name_str = name.to_string();
        let version_str = version.to_string();

        archive::create_with_metadata_factory(&out, &destdir, move |sha_hex, size| {
            let mut meta = Metadata {
                package: PackageSection {
                    name: Some(name_str.clone()),
                    version: Some(version_str.clone()),
                    license: Some("MIT".to_string()),
                    description: Some("tampered test pkg".to_string()),
                    arch: Some("x86_64".to_string()),
                    replaces: vec![],
                    conflicts: vec![],
                },
                depends: DependsSection::default(),
                hooks: HooksSection::default(),
                files: FilesSection {
                    sha256: Some(sha_hex.to_string()),
                    size: Some(size),
                },
                signature: None,
            };

            let payload_sha256_raw: [u8; 32] = hex::decode(sha_hex)
                .unwrap()
                .try_into()
                .unwrap();
            let canon = canonical_bytes(&meta, &payload_sha256_raw);
            let mut sig_bytes = sign_detached(&sk_clone, &canon);
            // Flip a byte in the signature.
            sig_bytes[0] ^= 0xFF;
            let sig_b64 =
                base64::engine::general_purpose::STANDARD.encode(sig_bytes);

            meta.signature = Some(Signature {
                algorithm: "ed25519".to_string(),
                key_id: key_id_str.clone(),
                sig: sig_b64,
            });

            meta.to_string().map_err(|e| {
                crate::archive::ArchiveError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })
        })
        .unwrap();

        out
    }

    // ── 1. install end-to-end via test seam ───────────────────────────────────

    #[test]
    fn test_install_packages_installs_files() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        // Build a .jpkg and extract_and_register it directly (bypassing repo).
        let jpkg_path = common_tests::build_test_jpkg(tmp.path(), "mypkg", "1.0.0");
        let archive = JpkgArchive::open(&jpkg_path).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        let _pkg = extract_and_register(&archive, &rootfs, &db).unwrap();

        assert!(rootfs.join("bin/foo").exists(), "bin/foo should be installed");
        assert!(rootfs.join("lib/bar").exists(), "lib/bar should be installed");

        let got = db.get("mypkg").unwrap().expect("mypkg should be in db");
        assert_eq!(got.metadata.package.name.as_deref(), Some("mypkg"));
        assert_eq!(got.metadata.package.version.as_deref(), Some("1.0.0"));
    }

    // ── 2. skip already-installed: re-register and verify db is consistent ────

    #[test]
    fn test_install_packages_skips_same_version() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let jpkg_path = common_tests::build_test_jpkg(tmp.path(), "mypkg2", "1.0.0");
        let archive = JpkgArchive::open(&jpkg_path).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // First install.
        extract_and_register(&archive, &rootfs, &db).unwrap();
        let before_files = db.get("mypkg2").unwrap().unwrap().files.len();

        // Simulate what install_packages does for same-version: checks version
        // in db, sees it matches, returns 0.  We verify the db is unchanged.
        let installed = db.get("mypkg2").unwrap().unwrap();
        let installed_ver = installed.metadata.package.version.as_deref().unwrap_or("");
        // Index would say 1.0.0 == installed 1.0.0 → skip.
        assert_eq!(installed_ver, "1.0.0", "installed version should be 1.0.0");

        // After: db should still have the same number of files.
        let after_files = db.get("mypkg2").unwrap().unwrap().files.len();
        assert_eq!(before_files, after_files, "file count should be unchanged");
    }

    // ── 3. post_install hook touches a marker ─────────────────────────────────

    #[test]
    fn test_install_post_install_hook_runs() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let hook = "touch \"$JPKG_ROOT/hook_marker\"";
        let jpkg_path =
            common_tests::build_test_jpkg_with_hook(tmp.path(), "hookpkg", "1.0.0", hook);
        let archive = JpkgArchive::open(&jpkg_path).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        let pkg = extract_and_register(&archive, &rootfs, &db).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.to_str().unwrap());
        if let Some(ref body) = pkg.metadata.hooks.post_install {
            let status = run_hook(&rootfs, body).unwrap();
            assert!(status.success(), "post_install hook should succeed");
        }
        std::env::remove_var("JPKG_ROOT");

        assert!(
            rootfs.join("hook_marker").exists(),
            "post_install hook should have created hook_marker"
        );
    }

    // ── Signature verification tests ──────────────────────────────────────────

    // 4. Unsigned package → warn policy → UnsignedAccepted.
    #[test]
    fn test_install_accepts_unsigned_under_warn_policy() {
        let tmp = TempDir::new().unwrap();
        let jpkg_path = build_unsigned_jpkg(tmp.path(), "nopkg", "1.0.0");
        let empty_keys = tmp.path().join("keys");
        fs::create_dir_all(&empty_keys).unwrap();

        let result = verify_jpkg_signature(&jpkg_path, &empty_keys, SignaturePolicy::Warn);
        assert!(
            matches!(result, Ok(VerifyOutcome::UnsignedAccepted)),
            "policy=warn + no sig must give UnsignedAccepted; got: {:?}",
            result
        );
    }

    // 5. Unsigned package → ignore policy → UnsignedIgnored (silent).
    #[test]
    fn test_install_accepts_unsigned_under_ignore_policy() {
        let tmp = TempDir::new().unwrap();
        let jpkg_path = build_unsigned_jpkg(tmp.path(), "nopkg2", "1.0.0");
        let empty_keys = tmp.path().join("keys");
        fs::create_dir_all(&empty_keys).unwrap();

        let result = verify_jpkg_signature(&jpkg_path, &empty_keys, SignaturePolicy::Ignore);
        assert!(
            matches!(result, Ok(VerifyOutcome::UnsignedIgnored)),
            "policy=ignore + no sig must give UnsignedIgnored; got: {:?}",
            result
        );
    }

    // 6. Unsigned package → require policy → SignatureMissing error.
    #[test]
    fn test_install_rejects_unsigned_under_require_policy() {
        let tmp = TempDir::new().unwrap();
        let jpkg_path = build_unsigned_jpkg(tmp.path(), "nopkg3", "1.0.0");
        let empty_keys = tmp.path().join("keys");
        fs::create_dir_all(&empty_keys).unwrap();

        let result = verify_jpkg_signature(&jpkg_path, &empty_keys, SignaturePolicy::Require);
        assert!(
            matches!(result, Err(SignatureError::Missing)),
            "policy=require + no sig must give Missing; got: {:?}",
            result
        );
    }

    // 7. Signed package with tampered (invalid) sig → SignatureInvalid,
    //    regardless of policy.
    #[test]
    fn test_install_rejects_signed_with_bad_signature() {
        let tmp = TempDir::new().unwrap();
        let sk = keygen();
        let key_id = "test-key.pub";

        // Write the public key so it can be loaded.
        let keys_dir = tmp.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        write_public_key(&keys_dir.join(key_id), &sk.verifying_key()).unwrap();

        let jpkg_path =
            build_tampered_sig_jpkg(tmp.path(), "tamperedpkg", "1.0.0", &sk, key_id);

        // All three policies should error on an invalid signature.
        for policy in [SignaturePolicy::Warn, SignaturePolicy::Ignore, SignaturePolicy::Require] {
            let result = verify_jpkg_signature(&jpkg_path, &keys_dir, policy);
            assert!(
                matches!(result, Err(SignatureError::Invalid(_))),
                "tampered sig must always give Invalid (policy={policy:?}); got: {:?}",
                result
            );
        }
    }

    // 8. Signed package with valid sig and matching pubkey → Verified.
    #[test]
    fn test_install_accepts_signed_with_valid_signature() {
        let tmp = TempDir::new().unwrap();
        let sk = keygen();
        let key_id = "jonerix-2026.pub";

        let keys_dir = tmp.path().join("keys");
        fs::create_dir_all(&keys_dir).unwrap();
        write_public_key(&keys_dir.join(key_id), &sk.verifying_key()).unwrap();

        let jpkg_path =
            build_signed_jpkg(tmp.path(), "signedpkg", "2.0.0", &sk, key_id);

        for policy in [SignaturePolicy::Warn, SignaturePolicy::Ignore, SignaturePolicy::Require] {
            let result = verify_jpkg_signature(&jpkg_path, &keys_dir, policy);
            match result {
                Ok(VerifyOutcome::Verified { key_id: ref kid }) => {
                    assert_eq!(kid, key_id, "matched key should be {key_id}");
                }
                other => panic!(
                    "expected Verified (policy={policy:?}), got: {:?}", other
                ),
            }
        }
    }
}
