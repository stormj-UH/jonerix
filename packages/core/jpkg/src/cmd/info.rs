// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg info` — show package metadata.
//!
//! C reference: `jpkg/src/cmd_info.c`.
//!
//! Lookup order:
//!   1. Repository INDEX (if reachable/cached) — prints index entry + install
//!      status from DB.
//!   2. Installed DB only — if the package is installed but not in the index.
//!   3. Neither found → error, exit 1.
//!
//! Output format (key: value pairs, matching C byte-for-byte where possible):
//!   Name:         <name>
//!   Version:      <version>
//!   License:      <license|"unknown">
//!   Architecture: <arch|"unknown">
//!   Description:  <description>
//!   Package size: N MiB  (or KiB or bytes, matching C thresholds)
//!   SHA256:       <sha256>   (omitted if empty)
//!   Dependencies: a, b, c    (omitted if empty)
//!   Build deps:   a, b, c    (omitted if empty)
//!   Status:       installed (version, YYYY-MM-DD HH:MM:SS) | not installed
//!   Installed files: N        (when installed)
//!   License OK:   yes (permissive) | WARNING - not recognized as permissive
//!
//! Divergences from C:
//! - The C `db_pkg_t` carries `install_time`; `InstalledPkg` does not expose a
//!   timestamp (it is not stored in metadata.toml in the Rust DB).  We omit
//!   the timestamp from "Status: installed (...)" rather than fabricating one.
//! - The C supports `--files` / `-f` flag.  We replicate it: when passed,
//!   print the full file list after the key-value block.

use crate::db::InstalledDb;
use crate::repo::Repo;
use crate::util::license_is_permissive;
use std::path::Path;

/// Run `jpkg info [--files|-f] <package>`.
pub fn run(args: &[String]) -> i32 {
    let mut show_files = false;
    let mut pkg_name: Option<&str> = None;

    for arg in args {
        if arg == "--files" || arg == "-f" {
            show_files = true;
        } else {
            pkg_name = Some(arg.as_str());
        }
    }

    let pkg_name = match pkg_name {
        Some(n) => n,
        None => {
            eprintln!("usage: jpkg info [--files] <package>");
            return 2;
        }
    };

    let rootfs_str = std::env::var("JPKG_ROOT").unwrap_or_else(|_| "/".to_string());
    let rootfs = Path::new(&rootfs_str);
    let arch = std::env::var("JPKG_ARCH").unwrap_or_else(|_| detect_arch());

    // Try the installed DB.
    let db = InstalledDb::open(rootfs).ok();
    let installed = db
        .as_ref()
        .and_then(|d| d.get(pkg_name).ok())
        .flatten();

    // Try the repository INDEX (cached preferred).
    let index = Repo::from_rootfs(rootfs, &arch)
        .ok()
        .and_then(|repo| {
            repo.load_cached_index()
                .ok()
                .flatten()
                .or_else(|| repo.fetch_index().ok())
        });

    let index_entry = index.as_ref().and_then(|idx| idx.get(pkg_name, &arch));

    if index_entry.is_none() && installed.is_none() {
        eprintln!("error: package '{pkg_name}' not found");
        return 1;
    }

    if let Some(entry) = index_entry {
        // C: print_entry_info(entry, installed)
        println!("Name:         {}", pkg_name);
        println!("Version:      {}", entry.version);
        println!(
            "License:      {}",
            if entry.license.is_empty() {
                "unknown"
            } else {
                &entry.license
            }
        );
        println!(
            "Architecture: {}",
            if entry.arch.is_empty() {
                "unknown"
            } else {
                &entry.arch
            }
        );
        println!("Description:  {}", entry.description);

        if entry.size > 0 {
            if entry.size >= 1_048_576 {
                println!(
                    "Package size: {:.1} MiB",
                    entry.size as f64 / 1_048_576.0
                );
            } else if entry.size >= 1024 {
                println!(
                    "Package size: {:.1} KiB",
                    entry.size as f64 / 1024.0
                );
            } else {
                println!("Package size: {} bytes", entry.size);
            }
        }

        if !entry.sha256.is_empty() {
            println!("SHA256:       {}", entry.sha256);
        }

        if !entry.depends.is_empty() {
            println!("Dependencies: {}", entry.depends.join(", "));
        }

        if !entry.build_depends.is_empty() {
            println!("Build deps:   {}", entry.build_depends.join(", "));
        }

        if let Some(ref inst) = installed {
            let inst_version = inst
                .metadata
                .package
                .version
                .as_deref()
                .unwrap_or("unknown");
            println!("Status:       installed ({inst_version})");
            println!("Installed files: {}", inst.files.len());
        } else {
            println!("Status:       not installed");
        }

        // License OK line — mirrors C's license_is_permissive check.
        if !entry.license.is_empty() {
            if license_is_permissive(&entry.license) {
                println!("License OK:   yes (permissive)");
            } else {
                println!("License OK:   WARNING - not recognized as permissive");
            }
        }
    } else if let Some(ref inst) = installed {
        // C: print_installed_info(installed)
        let meta = &inst.metadata.package;
        println!("Name:         {}", meta.name.as_deref().unwrap_or(pkg_name));
        println!(
            "Version:      {}",
            meta.version.as_deref().unwrap_or("unknown")
        );
        println!(
            "License:      {}",
            meta.license.as_deref().unwrap_or("unknown")
        );
        println!(
            "Architecture: {}",
            meta.arch.as_deref().unwrap_or("unknown")
        );
        println!("Description:  {}", meta.description.as_deref().unwrap_or(""));

        let runtime = &inst.metadata.depends.runtime;
        if !runtime.is_empty() {
            println!("Dependencies: {}", runtime.join(", "));
        }

        println!("Files:        {}", inst.files.len());
        println!("Status:       installed");
    }

    // Optional --files output.
    if show_files {
        if let Some(ref inst) = installed {
            println!("\nInstalled files:");
            for f in &inst.files {
                println!("  {}", f.path);
            }
        }
    }

    0
}

// ── helpers ──────────────────────────────────────────────────────────────────

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
    use crate::db::{FileEntry, InstalledDb, InstalledPkg};
    use crate::recipe::{DependsSection, Index, IndexEntry, Metadata, PackageSection};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn make_installed_pkg(name: &str, version: &str) -> InstalledPkg {
        InstalledPkg {
            metadata: Metadata {
                package: PackageSection {
                    name: Some(name.to_string()),
                    version: Some(version.to_string()),
                    license: Some("MIT".to_string()),
                    description: Some("A test package".to_string()),
                    arch: Some("x86_64".to_string()),
                    ..Default::default()
                },
                depends: DependsSection {
                    runtime: vec!["musl".to_string()],
                    ..Default::default()
                },
                ..Default::default()
            },
            files: vec![
                FileEntry {
                    path: "bin/test".to_string(),
                    sha256: "a".repeat(64),
                    size: 0,
                    mode: 0o100755,
                    symlink_target: None,
                    is_dir: false,
                },
            ],
        }
    }

    #[test]
    fn info_installed_package_exits_zero() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        // Insert a synthetic package.
        let db = InstalledDb::open(rootfs).unwrap();
        db.insert(&make_installed_pkg("testpkg", "2.0.0")).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());
        std::env::set_var("JPKG_ARCH", "x86_64");

        let rc = run(&["testpkg".to_string()]);
        assert_eq!(rc, 0);
    }

    #[test]
    fn info_index_entry_exits_zero() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        // Write a cached INDEX with one entry.
        let mut entries = BTreeMap::new();
        entries.insert(
            "zstd-x86_64".to_string(),
            IndexEntry {
                version: "1.5.6".to_string(),
                license: "BSD-3-Clause".to_string(),
                description: "Fast compression".to_string(),
                arch: "x86_64".to_string(),
                sha256: "b".repeat(64),
                size: 2_000_000,
                depends: vec!["musl".to_string()],
                build_depends: vec!["cmake".to_string()],
            },
        );
        let cache_dir = rootfs.join("var/cache/jpkg");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let index = Index { entries };
        std::fs::write(
            cache_dir.join("INDEX"),
            index.to_string().unwrap().as_bytes(),
        )
        .unwrap();
        std::fs::create_dir_all(rootfs.join("etc/jpkg/keys")).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());
        std::env::set_var("JPKG_ARCH", "x86_64");

        let rc = run(&["zstd".to_string()]);
        assert_eq!(rc, 0);
    }

    #[test]
    fn info_unknown_package_exits_one() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        // Empty DB, no index.
        InstalledDb::open(rootfs).unwrap();
        std::fs::create_dir_all(rootfs.join("etc/jpkg/keys")).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());
        std::env::set_var("JPKG_ARCH", "x86_64");

        let rc = run(&["nosuchpkg".to_string()]);
        assert_eq!(rc, 1);
    }

    #[test]
    fn info_no_args_exits_two() {
        let rc = run(&[]);
        assert_eq!(rc, 2);
    }
}
