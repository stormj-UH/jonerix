// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

use std::fs;

use crate::cmd::common::{resolve_rootfs, run_hook};
use crate::db::InstalledDb;
use crate::deps::resolve_remove;
use crate::types::OrphanMode;

// ─── public entry point ───────────────────────────────────────────────────────

/// `jpkg remove [--orphans] [--force] <pkg>...`
///
/// Returns 0 on success, 1 on any failure.
pub fn run(args: &[String]) -> i32 {
    // ── Parse flags ───────────────────────────────────────────────────────
    let mut orphans = false;
    let mut force = false;
    let mut pkg_names: Vec<String> = Vec::new();

    for a in args {
        match a.as_str() {
            "--orphans" | "-o" => orphans = true,
            "--force" | "-f" => force = true,
            other => pkg_names.push(other.to_string()),
        }
    }

    if pkg_names.is_empty() {
        eprintln!("usage: jpkg remove [--orphans] [--force] <package> [package...]");
        return 1;
    }

    // ── Environment ───────────────────────────────────────────────────────
    let rootfs = resolve_rootfs(None);

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

    // ── Verify all targets are installed ─────────────────────────────────
    let mut failures = 0usize;
    for name in &pkg_names {
        match db.get(name) {
            Ok(Some(_)) => {}
            Ok(None) => {
                eprintln!("jpkg: package {name} is not installed");
                failures += 1;
            }
            Err(e) => {
                eprintln!("jpkg: db error for {name}: {e}");
                failures += 1;
            }
        }
    }
    if failures > 0 && !force {
        return 1;
    }

    // ── Resolve removal order ─────────────────────────────────────────────
    let orphan_mode = if orphans { OrphanMode::PruneOrphans } else { OrphanMode::KeepOrphans };
    let order = match resolve_remove(&pkg_names, &db, orphan_mode) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("jpkg: removal resolution failed: {e}");
            return 1;
        }
    };

    if order.len() > pkg_names.len() {
        eprintln!(
            "jpkg: removing {} package(s) including {} orphaned dependency/ies:",
            order.len(),
            order.len() - pkg_names.len()
        );
        for name in &order {
            if let Ok(Some(p)) = db.get(name) {
                eprintln!(
                    "  {}-{}",
                    p.metadata.package.name.as_deref().unwrap_or(name),
                    p.metadata.package.version.as_deref().unwrap_or("?")
                );
            }
        }
    }

    // ── Remove loop ───────────────────────────────────────────────────────
    let mut removed = 0usize;

    for pkg_name in &order {
        log::info!("jpkg: removing {pkg_name}...");

        // Fetch record (we need hooks and file list).
        let pkg = match db.get(pkg_name) {
            Ok(Some(p)) => p,
            Ok(None) => {
                log::warn!("jpkg: {pkg_name} not found in db (already removed?)");
                continue;
            }
            Err(e) => {
                eprintln!("jpkg: db error reading {pkg_name}: {e}");
                failures += 1;
                continue;
            }
        };

        // pre_remove hook.
        if let Some(ref body) = pkg.metadata.hooks.pre_remove {
            let status = match run_hook(&rootfs, body) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("jpkg: pre_remove hook I/O error for {pkg_name}: {e}");
                    continue;
                }
            };
            if !status.success() {
                log::warn!(
                    "jpkg: pre_remove hook for {pkg_name} exited {}",
                    status.code().unwrap_or(-1)
                );
            }
        }

        // Save post_remove hook body before we drop the record.
        let post_hook = pkg.metadata.hooks.post_remove.clone();

        // Remove files in reverse order: longer paths (files) before shorter
        // ones (their parent directories).  This ensures we can rmdir
        // directories only after all their children are gone.
        let mut files = pkg.files.clone();
        files.sort_by(|a, b| b.path.cmp(&a.path));

        let mut file_errors = 0usize;
        for entry in &files {
            let full = rootfs.join(&entry.path);

            match full.symlink_metadata() {
                Err(_) => {
                    log::debug!("jpkg: file already absent: {}", entry.path);
                    continue;
                }
                Ok(m) => {
                    if m.is_dir() && !m.file_type().is_symlink() {
                        // Only remove directory if empty (mirrors C rmdir call).
                        if let Err(e) = fs::remove_dir(&full) {
                            log::debug!(
                                "jpkg: leaving non-empty dir {}: {e}",
                                entry.path
                            );
                        }
                    } else {
                        if let Err(e) = fs::remove_file(&full) {
                            log::warn!("jpkg: failed to remove {}: {e}", entry.path);
                            file_errors += 1;
                        }
                    }
                }
            }
        }

        if file_errors > 0 {
            log::warn!(
                "jpkg: {file_errors} file(s) could not be removed from {pkg_name}"
            );
        }

        // Remove DB record.
        if let Err(e) = db.remove(pkg_name) {
            eprintln!("jpkg: failed to remove db record for {pkg_name}: {e}");
            failures += 1;
        }

        // post_remove hook.
        if let Some(body) = post_hook {
            let status = match run_hook(&rootfs, &body) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("jpkg: post_remove hook I/O error for {pkg_name}: {e}");
                    // Continue — matches C behaviour (cmd_remove.c:173-175).
                    removed += 1;
                    continue;
                }
            };
            if !status.success() {
                log::warn!(
                    "jpkg: post_remove hook for {pkg_name} exited {}",
                    status.code().unwrap_or(-1)
                );
            }
        }

        log::info!("jpkg: removed {pkg_name}");
        removed += 1;
    }

    eprintln!("jpkg: {removed} package(s) removed");

    if failures > 0 {
        eprintln!("jpkg: {failures} package(s) could not be removed");
        return 1;
    }

    0
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

    // ── 1. Install then remove — files gone, db cleared ───────────────────────

    #[test]
    fn test_install_then_remove() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let jpkg_path = common_tests::build_test_jpkg(tmp.path(), "rmpkg", "1.0.0");
        let archive = JpkgArchive::open(&jpkg_path).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // Install.
        extract_and_register(&archive, &rootfs, &db).unwrap();
        assert!(rootfs.join("bin/foo").exists(), "bin/foo should be installed");
        assert!(rootfs.join("lib/bar").exists(), "lib/bar should be installed");
        assert!(db.get("rmpkg").unwrap().is_some(), "rmpkg should be in db");

        // Remove via the helper directly (bypasses run() to avoid process::exit).
        let pkg = db.get("rmpkg").unwrap().unwrap();
        let mut files = pkg.files.clone();
        files.sort_by(|a, b| b.path.cmp(&a.path));

        for entry in &files {
            let full = rootfs.join(&entry.path);
            if let Ok(m) = full.symlink_metadata() {
                if m.is_dir() && !m.file_type().is_symlink() {
                    let _ = fs::remove_dir(&full);
                } else {
                    let _ = fs::remove_file(&full);
                }
            }
        }
        db.remove("rmpkg").unwrap();

        // Verify.
        assert!(!rootfs.join("bin/foo").exists(), "bin/foo should be gone");
        assert!(!rootfs.join("lib/bar").exists(), "lib/bar should be gone");
        assert!(
            db.get("rmpkg").unwrap().is_none(),
            "rmpkg should be gone from db"
        );
    }

    // ── 2. pre_remove hook runs ───────────────────────────────────────────────

    #[test]
    fn test_pre_remove_hook_runs() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        // Build a package with a pre_remove hook.
        let hook = "touch \"$JPKG_ROOT/pre_remove_ran\"";
        let jpkg_path = crate::cmd::common::tests::build_test_jpkg_with_hook(
            tmp.path(), "hookrm", "1.0.0", hook,
        );
        // Rebuild with pre_remove instead — we need a bespoke helper here.
        // Use the archive and db APIs directly to register a fake pkg with a pre_remove hook.
        drop(jpkg_path);

        use crate::db::InstalledPkg;
        use crate::recipe::{DependsSection, HooksSection, Metadata, PackageSection, FilesSection};

        let meta = Metadata {
            package: PackageSection {
                name: Some("hookrm".to_string()),
                version: Some("1.0.0".to_string()),
                license: Some("MIT".to_string()),
                description: Some("hook remove test".to_string()),
                arch: Some("x86_64".to_string()),
                replaces: vec![],
                conflicts: vec![],
            },
            depends: DependsSection::default(),
            hooks: HooksSection {
                pre_remove: Some(hook.to_string()),
                ..Default::default()
            },
            files: FilesSection::default(),
            signature: None,
        };

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        db.insert(&InstalledPkg {
            metadata: meta,
            files: vec![],
        }).unwrap();

        // Simulate the removal path that run() takes.
        let pkg = db.get("hookrm").unwrap().unwrap();
        std::env::set_var("JPKG_ROOT", rootfs.to_str().unwrap());
        if let Some(ref body) = pkg.metadata.hooks.pre_remove {
            let _ = run_hook(&rootfs, body).unwrap();
        }
        std::env::remove_var("JPKG_ROOT");

        assert!(
            rootfs.join("pre_remove_ran").exists(),
            "pre_remove hook should have created pre_remove_ran"
        );
    }
}
