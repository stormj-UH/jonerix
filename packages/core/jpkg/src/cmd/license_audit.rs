// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg license-audit` — verify installed packages have permissive licenses.
//!
//! C reference: `jpkg/src/cmd_audit.c`.
//!
//! Output (installed-packages path, no `--repo` flag):
//!
//! ```text
//!   License audit of installed packages:
//!
//!     PACKAGE                  LICENSE          STATUS
//!     -------                  -------          ------
//!     <name>                   <license>        OK | VIOLATION | UNKNOWN
//!   ...
//! ```
//!
//! ```text
//!   Audit summary:
//!     Total packages:     N
//!     Permissive (OK):    N
//!     Unknown license:    N
//!     License violations: N
//!
//!   PASSED: All packages have permissive licenses.
//!   --- or ---
//!   FAILED: N package(s) have non-permissive licenses.
//!   ...
//! ```
//!
//! Exit codes:
//!   0 — all permissive (or all unknown, same as C)
//!   1 — at least one VIOLATION (non-permissive), matches C behaviour
//!   2 — usage error (should not occur: no positional args)
//!
//! Divergences from C:
//! - C supports `--verbose` / `-v` (show OK rows) and `--repo` / `-r`
//!   (audit the index instead of installed packages).  We implement both for
//!   fidelity, but the task spec mandates installed-package auditing as the
//!   default path.  `--repo` requires a live/cached index; if absent, exit 1.
//! - The C `<name>` column is 24 chars, license 16 chars. We match those
//!   widths byte-for-byte.

use crate::db::InstalledDb;
use crate::repo::Repo;
use crate::util::license_is_permissive;
use std::path::Path;

/// Run `jpkg license-audit [--verbose|-v] [--repo|-r]`.
pub fn run(args: &[String]) -> i32 {
    let mut verbose = false;
    let mut check_repo = false;

    for arg in args {
        match arg.as_str() {
            "--verbose" | "-v" => verbose = true,
            "--repo" | "-r" => check_repo = true,
            _ => {} // unknown flags ignored (mirrors C)
        }
    }

    let rootfs_str = std::env::var("JPKG_ROOT").unwrap_or_else(|_| "/".to_string());
    let rootfs = Path::new(&rootfs_str);

    if check_repo {
        return audit_repo(rootfs, verbose);
    }

    audit_installed(rootfs, verbose)
}

// ── Installed packages audit ──────────────────────────────────────────────────

fn audit_installed(rootfs: &Path, verbose: bool) -> i32 {
    let db = match InstalledDb::open(rootfs) {
        Ok(d) => d,
        Err(e) => {
            log::error!("failed to open installed db: {e}");
            eprintln!("error: {e}");
            return 1;
        }
    };

    let names = match db.list() {
        Ok(n) => n,
        Err(e) => {
            log::error!("db.list() failed: {e}");
            eprintln!("error: {e}");
            return 1;
        }
    };

    if names.is_empty() {
        log::info!("no packages installed");
        return 0;
    }

    println!("License audit of installed packages:\n");
    println!("  {:<24} {:<16} {}", "PACKAGE", "LICENSE", "STATUS");
    println!("  {:<24} {:<16} {}", "-------", "-------", "------");

    let mut total = 0i32;
    let mut violations = 0i32;
    let mut unknown = 0i32;

    for name in &names {
        let pkg = match db.get(name) {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(e) => {
                log::warn!("cannot read {name}: {e}");
                continue;
            }
        };

        total += 1;

        let license = pkg.metadata.package.license.as_deref().unwrap_or("unknown");

        let status = if license.is_empty() || license == "unknown" {
            unknown += 1;
            "UNKNOWN"
        } else if license_is_permissive(license) {
            "OK"
        } else {
            violations += 1;
            "VIOLATION"
        };

        if verbose || status != "OK" {
            println!("  {:<24} {:<16} {}", name, license, status);
        }
    }

    print_summary(total, violations, unknown);

    if violations > 0 {
        1
    } else {
        0
    }
}

// ── Repository index audit ────────────────────────────────────────────────────

fn audit_repo(rootfs: &Path, verbose: bool) -> i32 {
    let arch = std::env::var("JPKG_ARCH").unwrap_or_else(|_| detect_arch());

    let repo = match Repo::from_rootfs(rootfs, &arch) {
        Ok(r) => r,
        Err(e) => {
            log::error!("failed to open repo config: {e}");
            eprintln!("error: {e}");
            return 1;
        }
    };

    let index = match repo
        .load_cached_index()
        .ok()
        .flatten()
        .or_else(|| repo.fetch_index().ok())
    {
        Some(idx) => idx,
        None => {
            eprintln!("error: no package index. Run 'jpkg update' first.");
            return 1;
        }
    };

    println!("License audit of repository packages:\n");
    println!(
        "  {:<24} {:<12} {:<16} {}",
        "PACKAGE", "VERSION", "LICENSE", "STATUS"
    );
    println!(
        "  {:<24} {:<12} {:<16} {}",
        "-------", "-------", "-------", "------"
    );

    let mut total = 0i32;
    let mut violations = 0i32;
    let mut unknown = 0i32;

    for (key, entry) in &index.entries {
        let name = key
            .strip_suffix(&format!("-{}", entry.arch))
            .unwrap_or(key.as_str());

        total += 1;

        let license = if entry.license.is_empty() {
            "unknown"
        } else {
            &entry.license
        };

        let status = if license == "unknown" {
            unknown += 1;
            "UNKNOWN"
        } else if license_is_permissive(license) {
            "OK"
        } else {
            violations += 1;
            "VIOLATION"
        };

        if verbose || status != "OK" {
            println!(
                "  {:<24} {:<12} {:<16} {}",
                name, entry.version, license, status
            );
        }
    }

    print_summary(total, violations, unknown);

    if violations > 0 {
        1
    } else {
        0
    }
}

// ── Shared summary printer ─────────────────────────────────────────────────────

fn print_summary(total: i32, violations: i32, unknown: i32) {
    let permissive = total - violations - unknown;
    println!();
    println!("Audit summary:");
    println!("  Total packages:     {}", total);
    println!("  Permissive (OK):    {}", permissive);
    println!("  Unknown license:    {}", unknown);
    println!("  License violations: {}", violations);

    if violations > 0 {
        println!(
            "\nFAILED: {} package(s) have non-permissive licenses.",
            violations
        );
        println!("jonerix requires all packages to use permissive licenses");
        println!("(MIT, BSD, ISC, Apache-2.0, public domain, etc.)");
    } else if unknown > 0 {
        println!("\nWARNING: {} package(s) have unknown licenses.", unknown);
        println!("Verify these manually before deployment.");
    } else {
        println!("\nPASSED: All packages have permissive licenses.");
    }
}

fn detect_arch() -> String {
    std::process::Command::new("uname")
        .arg("-m")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "x86_64".to_string())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{InstalledDb, InstalledPkg};
    use crate::recipe::{DependsSection, Metadata, PackageSection};
    use tempfile::TempDir;

    fn insert_pkg(db: &InstalledDb, name: &str, license: &str) {
        let pkg = InstalledPkg {
            metadata: Metadata {
                package: PackageSection {
                    name: Some(name.to_string()),
                    version: Some("1.0.0".to_string()),
                    license: Some(license.to_string()),
                    description: Some("test".to_string()),
                    arch: Some("x86_64".to_string()),
                    ..Default::default()
                },
                depends: DependsSection::default(),
                ..Default::default()
            },
            files: vec![],
        };
        db.insert(&pkg).expect("insert failed");
    }

    #[test]
    fn audit_all_permissive_exits_zero() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let db = InstalledDb::open(rootfs).unwrap();
        insert_pkg(&db, "musl", "MIT");
        insert_pkg(&db, "zstd", "BSD-3-Clause");

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&[]);
        assert_eq!(rc, 0, "all-permissive should return 0");
    }

    #[test]
    fn audit_non_permissive_exits_one() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let db = InstalledDb::open(rootfs).unwrap();
        insert_pkg(&db, "good", "MIT");
        insert_pkg(&db, "bad", "GPL-3.0-only");

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&[]);
        assert_eq!(rc, 1, "non-permissive pkg should cause exit 1");
    }

    #[test]
    fn audit_empty_db_exits_zero() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();
        InstalledDb::open(rootfs).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&[]);
        assert_eq!(rc, 0, "empty db should return 0");
    }

    #[test]
    fn audit_unknown_license_exits_zero_with_warning() {
        // C returns 0 for unknown (not a hard failure).
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let db = InstalledDb::open(rootfs).unwrap();
        insert_pkg(&db, "mystery", "unknown");

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&[]);
        assert_eq!(rc, 0, "unknown license is warning, not failure");
    }

    #[test]
    fn audit_verbose_flag_accepted() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let db = InstalledDb::open(rootfs).unwrap();
        insert_pkg(&db, "musl", "MIT");

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());

        let rc = run(&["--verbose".to_string()]);
        assert_eq!(rc, 0);
    }
}
