//! `jpkg verify` — re-hash installed files against the manifest SHA-256.
//!
//! C reference: `jpkg/src/cmd_verify.c` (120 lines).
//!
//! Usage: `jpkg verify [--quiet|-q] [<pkg>...]`
//!   - No package names → verify ALL installed packages.
//!   - With package names → verify only those packages.
//!
//! For each package:
//!   - Walk `InstalledPkg.files`.
//!   - Regular files (`!is_dir && symlink_target.is_none()`):
//!       call `sha256_file(rootfs/path)` and compare to manifest sha256.
//!   - Symlinks (`symlink_target.is_some()`):
//!       call `std::fs::read_link(rootfs/path)` and compare to manifest target.
//!   - Directories: skipped.
//!
//! Mismatch output (one line each):
//!   `<pkg>:<path>  expected=<sha>  got=<sha>`   (regular file)
//!   `<pkg>:<path>  expected=<target>  got=<target>` (symlink)
//!
//! Summary line:
//!   `<N> packages verified, <M> mismatches`
//!
//! Exit codes:
//!   0 — no mismatches
//!   1 — at least one mismatch
//!   2 — db read failure or unknown package (matches C's db-error path)
//!
//! Divergences from C:
//! - The C `cmd_verify.c` uses `db_verify_files` (a callback-based helper in
//!   db.c); the Rust DB does not expose that callback.  We inline the same
//!   logic: open file → sha256 → compare.
//! - The C counts "missing", "modified", "errors" separately and prints
//!   verbose per-file lines for single-package mode.  We replicate this
//!   distinction: missing files (ENOENT) are counted separately and printed
//!   as `got=(missing)`.  I/O errors are counted as errors and printed as
//!   `got=(error)`.
//! - The C default is `verbose = true`; we follow that (verbose unless -q).
//! - For multi-package ("verify all") mode: the C suppresses per-file detail
//!   and only prints one "Checking <name>-<ver>... OK/FAIL" line per package.
//!   We replicate that behaviour.

use crate::db::InstalledDb;
use crate::util::sha256_file;
use std::path::Path;

/// Run `jpkg verify [--quiet|-q] [<pkg>...]`.
pub fn run(args: &[String]) -> i32 {
    let mut quiet = false;
    let mut pkg_names: Vec<&str> = Vec::new();

    for arg in args {
        if arg == "--quiet" || arg == "-q" {
            quiet = true;
        } else {
            pkg_names.push(arg.as_str());
        }
    }

    let rootfs_str = std::env::var("JPKG_ROOT").unwrap_or_else(|_| "/".to_string());
    let rootfs = Path::new(&rootfs_str);

    let db = match InstalledDb::open(rootfs) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: failed to open installed db: {e}");
            return 2;
        }
    };

    // Determine the set of packages to verify.
    let all_names: Vec<String>;
    let names_to_verify: &[String];

    if pkg_names.is_empty() {
        all_names = match db.list() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("error: db.list() failed: {e}");
                return 2;
            }
        };
        if all_names.is_empty() {
            log::info!("no packages installed");
            return 0;
        }
        names_to_verify = &all_names;
    } else {
        // Validate that every requested package is installed.
        for name in &pkg_names {
            match db.get(name) {
                Ok(None) => {
                    eprintln!("error: package {name} is not installed");
                    return 2;
                }
                Err(e) => {
                    eprintln!("error: cannot read package {name}: {e}");
                    return 2;
                }
                Ok(Some(_)) => {}
            }
        }
        all_names = pkg_names.iter().map(|s| s.to_string()).collect();
        names_to_verify = &all_names;
    }

    let single = names_to_verify.len() == 1;
    let verbose = !quiet; // C default is verbose=true

    if !single {
        println!("Verifying all installed packages...\n");
    }

    let mut total_mismatches: usize = 0;
    let mut packages_ok: usize = 0;
    let mut packages_bad: usize = 0;

    for name in names_to_verify {
        let pkg = match db.get(name) {
            Ok(Some(p)) => p,
            Ok(None) => {
                eprintln!("error: package {name} disappeared from db");
                return 2;
            }
            Err(e) => {
                eprintln!("error: cannot read package {name}: {e}");
                return 2;
            }
        };

        if single {
            println!("Verifying {}...", name);
        } else if verbose {
            let version = pkg.metadata.package.version.as_deref().unwrap_or("?");
            print!("Checking {}-{}...", name, version);
        }

        let result = verify_package(&pkg, rootfs, name, single && verbose);

        total_mismatches += result.mismatches;

        if result.mismatches == 0 {
            packages_ok += 1;
            if single {
                println!("  OK: all files verified");
            } else if verbose {
                println!(" OK");
            }
        } else {
            packages_bad += 1;
            if single {
                println!(
                    "  FAIL: {} missing, {} modified, {} errors",
                    result.missing, result.modified, result.errors
                );
            } else if verbose {
                println!(
                    " FAIL ({} missing, {} modified, {} errors)",
                    result.missing, result.modified, result.errors
                );
            }
        }
    }

    if !single {
        println!("\nVerification summary:");
        println!("  Packages OK:     {}", packages_ok);
        println!("  Packages failed: {}", packages_bad);
        println!("  Total issues:    {}", total_mismatches);
    }

    println!("\n{} packages verified, {} mismatches", names_to_verify.len(), total_mismatches);

    if total_mismatches > 0 { 1 } else { 0 }
}

// ── Per-package verification ──────────────────────────────────────────────────

struct VerifyResult {
    mismatches: usize,
    missing: usize,
    modified: usize,
    errors: usize,
}

/// Verify all files in `pkg` against `rootfs`.
/// When `print_detail` is true, print mismatch lines to stdout.
fn verify_package(
    pkg: &crate::db::InstalledPkg,
    rootfs: &Path,
    pkg_name: &str,
    print_detail: bool,
) -> VerifyResult {
    let mut result = VerifyResult {
        mismatches: 0,
        missing: 0,
        modified: 0,
        errors: 0,
    };

    for fe in &pkg.files {
        if fe.is_dir {
            // Directories are not checked by the C version.
            continue;
        }

        // Build the absolute path (strip leading '/' from the manifest path
        // since the DB stores it without a leading slash, e.g. "bin/foo").
        let rel = fe.path.trim_start_matches('/');
        let abs = rootfs.join(rel);

        if let Some(ref expected_target) = fe.symlink_target {
            // Symlink check: read_link and compare.
            match std::fs::read_link(&abs) {
                Ok(actual_path) => {
                    let actual = actual_path.to_string_lossy();
                    if actual.as_ref() != expected_target.as_str() {
                        result.mismatches += 1;
                        result.modified += 1;
                        if print_detail {
                            println!(
                                "{}:{}  expected={}  got={}",
                                pkg_name, fe.path, expected_target, actual
                            );
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    result.mismatches += 1;
                    result.missing += 1;
                    if print_detail {
                        println!(
                            "{}:{}  expected={}  got=(missing)",
                            pkg_name, fe.path, expected_target
                        );
                    }
                }
                Err(_) => {
                    result.mismatches += 1;
                    result.errors += 1;
                    if print_detail {
                        println!(
                            "{}:{}  expected={}  got=(error)",
                            pkg_name, fe.path, expected_target
                        );
                    }
                }
            }
        } else {
            // Regular file: SHA-256 check.
            match sha256_file(&abs) {
                Ok(actual_sha) => {
                    if actual_sha != fe.sha256 {
                        result.mismatches += 1;
                        result.modified += 1;
                        if print_detail {
                            println!(
                                "{}:{}  expected={}  got={}",
                                pkg_name, fe.path, fe.sha256, actual_sha
                            );
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    result.mismatches += 1;
                    result.missing += 1;
                    if print_detail {
                        println!(
                            "{}:{}  expected={}  got=(missing)",
                            pkg_name, fe.path, fe.sha256
                        );
                    }
                }
                Err(_) => {
                    result.mismatches += 1;
                    result.errors += 1;
                    if print_detail {
                        println!(
                            "{}:{}  expected={}  got=(error)",
                            pkg_name, fe.path, fe.sha256
                        );
                    }
                }
            }
        }
    }

    result
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{FileEntry, InstalledDb, InstalledPkg};
    use crate::recipe::{DependsSection, Metadata, PackageSection};
    use crate::util::sha256_file;
    use tempfile::TempDir;

    fn make_pkg_with_file(name: &str, file_rel: &str, sha256: &str) -> InstalledPkg {
        InstalledPkg {
            metadata: Metadata {
                package: PackageSection {
                    name: Some(name.to_string()),
                    version: Some("1.0.0".to_string()),
                    license: Some("MIT".to_string()),
                    description: Some("test".to_string()),
                    arch: Some("x86_64".to_string()),
                    ..Default::default()
                },
                depends: DependsSection::default(),
                ..Default::default()
            },
            files: vec![FileEntry {
                path: file_rel.to_string(),
                sha256: sha256.to_string(),
                size: 0,
                mode: 0o100644,
                symlink_target: None,
                is_dir: false,
            }],
        }
    }

    #[test]
    fn verify_correct_file_exits_zero() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        // Write the real file under rootfs/bin/testbin.
        let bin_dir = rootfs.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let file_path = bin_dir.join("testbin");
        std::fs::write(&file_path, b"hello verify").unwrap();

        // Compute the correct sha256.
        let sha = sha256_file(&file_path).unwrap();

        // Insert the package with the correct sha.
        let db = InstalledDb::open(rootfs).unwrap();
        db.insert(&make_pkg_with_file("mypkg", "bin/testbin", &sha))
            .unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&["mypkg".to_string()]);
        assert_eq!(rc, 0, "correct file should give exit 0");
    }

    #[test]
    fn verify_corrupted_file_exits_one() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let bin_dir = rootfs.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let file_path = bin_dir.join("testbin2");
        std::fs::write(&file_path, b"original content").unwrap();

        let good_sha = sha256_file(&file_path).unwrap();

        let db = InstalledDb::open(rootfs).unwrap();
        db.insert(&make_pkg_with_file("mypkg2", "bin/testbin2", &good_sha))
            .unwrap();

        // Now corrupt the file.
        std::fs::write(&file_path, b"CORRUPTED").unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&["mypkg2".to_string()]);
        assert_eq!(rc, 1, "corrupted file should give exit 1");
    }

    #[test]
    fn verify_missing_file_exits_one() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let db = InstalledDb::open(rootfs).unwrap();
        db.insert(&make_pkg_with_file(
            "mypkg3",
            "bin/nonexistent",
            &"a".repeat(64),
        ))
        .unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&["mypkg3".to_string()]);
        assert_eq!(rc, 1, "missing file should give exit 1");
    }

    #[test]
    fn verify_unknown_package_exits_two() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();
        InstalledDb::open(rootfs).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&["nosuchpkg".to_string()]);
        assert_eq!(rc, 2, "unknown package should give exit 2");
    }

    #[test]
    fn verify_all_empty_db_exits_zero() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();
        InstalledDb::open(rootfs).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&[]);
        assert_eq!(rc, 0, "empty db verify-all should exit 0");
    }

    #[test]
    fn verify_symlink_correct_exits_zero() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        // Create the symlink in rootfs.
        let bin_dir = rootfs.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::os::unix::fs::symlink("sh", bin_dir.join("ash")).unwrap();

        let pkg = InstalledPkg {
            metadata: Metadata {
                package: PackageSection {
                    name: Some("busybox".to_string()),
                    version: Some("1.0.0".to_string()),
                    license: Some("GPL-2.0-only".to_string()),
                    description: Some("test".to_string()),
                    arch: Some("x86_64".to_string()),
                    ..Default::default()
                },
                depends: DependsSection::default(),
                ..Default::default()
            },
            files: vec![FileEntry {
                path: "bin/ash".to_string(),
                sha256: String::new(),
                size: 0,
                mode: 0o120777,
                symlink_target: Some("sh".to_string()),
                is_dir: false,
            }],
        };

        let db = InstalledDb::open(rootfs).unwrap();
        db.insert(&pkg).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&["busybox".to_string()]);
        assert_eq!(rc, 0, "correct symlink should give exit 0");
    }

    #[test]
    fn verify_symlink_wrong_target_exits_one() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let bin_dir = rootfs.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        // Symlink points to "sh" but manifest says "bash".
        std::os::unix::fs::symlink("sh", bin_dir.join("ash2")).unwrap();

        let pkg = InstalledPkg {
            metadata: Metadata {
                package: PackageSection {
                    name: Some("symlinkpkg".to_string()),
                    version: Some("1.0.0".to_string()),
                    license: Some("MIT".to_string()),
                    description: Some("test".to_string()),
                    arch: Some("x86_64".to_string()),
                    ..Default::default()
                },
                depends: DependsSection::default(),
                ..Default::default()
            },
            files: vec![FileEntry {
                path: "bin/ash2".to_string(),
                sha256: String::new(),
                size: 0,
                mode: 0o120777,
                symlink_target: Some("bash".to_string()), // wrong
                is_dir: false,
            }],
        };

        let db = InstalledDb::open(rootfs).unwrap();
        db.insert(&pkg).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&["symlinkpkg".to_string()]);
        assert_eq!(rc, 1, "wrong symlink target should give exit 1");
    }
}
