/*
 * jpkg - jonerix package manager
 * cmd/local_install.rs - jpkg-local install: force-install a single .jpkg
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 *
 * Port of main_local.c lines 498-596 (the jpkg-local install sub-command).
 *
 * Divergences from C:
 * - URL detection: the C code passes source to `fetch_to_tmp` which internally
 *   decides whether to download or just open the path (main_local.c:532).  We
 *   check the prefix explicitly (https?://), matching the documented interface.
 * - Stdin ("-"): we read all of stdin into a Vec<u8> and pass to
 *   JpkgArchive::from_bytes.  The C code also reads stdin to a temp file; this
 *   is equivalent without the temp-file indirection.
 * - We do NOT warn about unsatisfied runtime dependencies (the C code does,
 *   main_local.c:550-556).  The local-install command is intentionally
 *   dependency-free (it is used during bootstrap when the dep graph is not yet
 *   populated).  A FIXME is left below if this should be re-added.
 * - No `audit_layout_tree` call: the archive crate enforces the merged-/usr
 *   layout at create time via `ArchiveError::UnflatLayout`.
 */

use std::io::{self, Read};
use std::path::Path;

use crate::archive::JpkgArchive;
use crate::cmd::common::{extract_and_register, resolve_rootfs, run_hook};
use crate::db::InstalledDb;
use crate::fetch::download;
use crate::recipe::Metadata;

// ─── public entry point ───────────────────────────────────────────────────────

/// `jpkg-local install <file.jpkg|url|-> [--root <dir>]`
///
/// Installs a single .jpkg without dependency resolution.
/// Returns 0 on success, 1 on failure.
pub fn run(args: &[String]) -> i32 {
    // ── Parse args ────────────────────────────────────────────────────────
    let mut source: Option<String> = None;
    let mut root_arg: Option<String> = None;

    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--root" | "-r" => {
                root_arg = iter.next().map(|s| s.clone());
                if root_arg.is_none() {
                    eprintln!("jpkg-local install: --root requires an argument");
                    return 1;
                }
            }
            "-" => {
                source = Some("-".to_string());
            }
            other if other.starts_with('-') => {
                eprintln!("jpkg-local install: unknown option: {other}");
                eprintln!("usage: jpkg-local install <file.jpkg|url|-> [--root <dir>]");
                return 1;
            }
            other => {
                source = Some(other.to_string());
            }
        }
    }

    let source = match source {
        Some(s) => s,
        None => {
            eprintln!("usage: jpkg-local install <file.jpkg|url|-> [--root <dir>]");
            return 1;
        }
    };

    // ── Resolve rootfs ────────────────────────────────────────────────────
    let rootfs = resolve_rootfs(root_arg.as_deref());

    // ── Load archive ──────────────────────────────────────────────────────
    let archive = match load_archive(&source) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("jpkg-local install: failed to load {source}: {e}");
            return 1;
        }
    };

    // ── Parse metadata (for logging + hooks) ──────────────────────────────
    let metadata = match Metadata::from_str(archive.metadata()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("jpkg-local install: failed to parse metadata from {source}: {e}");
            return 1;
        }
    };

    let pkg_name = metadata
        .package
        .name
        .as_deref()
        .unwrap_or("(unnamed)")
        .to_string();
    let pkg_version = metadata
        .package
        .version
        .as_deref()
        .unwrap_or("?")
        .to_string();

    log::info!("jpkg-local: installing {pkg_name}-{pkg_version} from {source}...");

    // FIXME: warn about unsatisfied runtime dependencies (mirrors main_local.c:550-556).
    // Omitted because local-install is used during bootstrap when the db is empty.

    // ── Open DB + lock (create if missing — main_local.c:547-556) ────────
    let db = match InstalledDb::open(&rootfs) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("jpkg-local install: failed to open database: {e}");
            return 1;
        }
    };
    let _lock = match db.lock() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("jpkg-local install: {e}");
            return 1;
        }
    };

    // ── pre_install hook ──────────────────────────────────────────────────
    if let Some(ref body) = metadata.hooks.pre_install {
        match run_hook(&rootfs, body) {
            Ok(status) if !status.success() => {
                eprintln!(
                    "jpkg-local install: pre_install hook failed (exit {})",
                    status.code().unwrap_or(-1)
                );
                return 1;
            }
            Err(e) => {
                eprintln!("jpkg-local install: pre_install hook I/O error: {e}");
                return 1;
            }
            Ok(_) => {}
        }
    }

    // ── Extract, flatten, install, register ──────────────────────────────
    let pkg = match extract_and_register(&archive, &rootfs, &db) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("jpkg-local install: installation failed: {e}");
            return 1;
        }
    };

    // ── post_install hook ─────────────────────────────────────────────────
    if let Some(ref body) = pkg.metadata.hooks.post_install {
        match run_hook(&rootfs, body) {
            Ok(status) if !status.success() => {
                log::warn!(
                    "jpkg-local install: post_install hook for {pkg_name} exited {}",
                    status.code().unwrap_or(-1)
                );
                // Non-fatal (mirrors C behaviour: cmd_install.c:555 ignores return value).
            }
            Err(e) => {
                log::warn!("jpkg-local install: post_install hook I/O error: {e}");
            }
            Ok(_) => {}
        }
    }

    println!("installed {pkg_name}-{pkg_version}");
    0
}

// ─── load_archive ────────────────────────────────────────────────────────────

/// Resolve the source string to a loaded `JpkgArchive`.
///
/// Three cases (main_local.c:530-533):
/// - `"-"`: read stdin.
/// - `"https?://..."`: download via `crate::fetch::download`.
/// - anything else: treat as a filesystem path.
fn load_archive(source: &str) -> Result<JpkgArchive, String> {
    if source == "-" {
        let mut bytes = Vec::new();
        io::stdin()
            .read_to_end(&mut bytes)
            .map_err(|e| format!("reading stdin: {e}"))?;
        JpkgArchive::from_bytes(bytes).map_err(|e| format!("parsing stdin archive: {e}"))
    } else if source.starts_with("https://") || source.starts_with("http://") {
        let bytes = download(source).map_err(|e| format!("downloading {source}: {e}"))?;
        JpkgArchive::from_bytes(bytes).map_err(|e| format!("parsing downloaded archive: {e}"))
    } else {
        JpkgArchive::open(Path::new(source))
            .map_err(|e| format!("opening {source}: {e}"))
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::JpkgArchive;
    use crate::cmd::common::{extract_and_register, tests as common_tests};
    use crate::db::InstalledDb;
    use std::fs;
    use tempfile::TempDir;

    // ── 1. local_install from file path ───────────────────────────────────────

    #[test]
    fn test_local_install_from_file() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let jpkg_path = common_tests::build_test_jpkg(tmp.path(), "localpkg", "1.0.0");

        // Exercise load_archive with a file path.
        let archive = load_archive(jpkg_path.to_str().unwrap()).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        let _pkg = extract_and_register(&archive, &rootfs, &db).unwrap();

        // Verify on-disk files.
        assert!(rootfs.join("bin/foo").exists(), "bin/foo should be installed");
        assert!(rootfs.join("lib/bar").exists(), "lib/bar should be installed");

        // Verify DB state.
        let got = db.get("localpkg").unwrap().expect("localpkg should be in db");
        assert_eq!(got.metadata.package.name.as_deref(), Some("localpkg"));
        assert_eq!(got.metadata.package.version.as_deref(), Some("1.0.0"));
        assert!(!got.files.is_empty(), "files manifest should be non-empty");
    }

    // ── 2. local_install post_install hook ────────────────────────────────────

    #[test]
    fn test_local_install_post_hook_runs() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let hook = "touch \"$JPKG_ROOT/local_hook_marker\"";
        let jpkg_path = common_tests::build_test_jpkg_with_hook(tmp.path(), "hooklocal", "1.0.0", hook);

        let archive = load_archive(jpkg_path.to_str().unwrap()).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        let pkg = extract_and_register(&archive, &rootfs, &db).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.to_str().unwrap());
        if let Some(ref body) = pkg.metadata.hooks.post_install {
            let _ = run_hook(&rootfs, body).unwrap();
        }
        std::env::remove_var("JPKG_ROOT");

        assert!(
            rootfs.join("local_hook_marker").exists(),
            "post_install hook should have created local_hook_marker"
        );
    }

    // ── 3. local_install replaces transfer ────────────────────────────────────

    #[test]
    fn test_local_install_replaces() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // Install A which owns bin/sh.
        let a_jpkg = common_tests::build_jpkg_with_replaces(tmp.path(), "shpkgA", "1.0.0", vec![]);
        let a_arc = JpkgArchive::open(&a_jpkg).unwrap();
        extract_and_register(&a_arc, &rootfs, &db).unwrap();

        let a_before = db.get("shpkgA").unwrap().unwrap();
        assert!(a_before.files.iter().any(|e| e.path == "bin/sh"), "A should own bin/sh");

        // Install B via local_install path with replaces = [shpkgA].
        let b_jpkg = common_tests::build_jpkg_with_replaces(tmp.path(), "shpkgB", "1.0.0", vec!["shpkgA".to_string()]);
        let b_path = b_jpkg.to_str().unwrap();
        let b_arc = load_archive(b_path).unwrap();
        extract_and_register(&b_arc, &rootfs, &db).unwrap();

        // A should no longer own bin/sh.
        let a_after = db.get("shpkgA").unwrap().unwrap();
        assert!(
            !a_after.files.iter().any(|e| e.path == "bin/sh"),
            "A should not own bin/sh after B's local install replaces it"
        );
    }
}
