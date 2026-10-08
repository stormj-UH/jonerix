// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::cmd::common::{resolve_rootfs, run_hook};
use crate::db::{InstalledDb, InstalledPkg, Ownership};
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
    let orphan_mode = if orphans {
        OrphanMode::PruneOrphans
    } else {
        OrphanMode::KeepOrphans
    };
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

        // Paths another installed package still lists stay on disk: they are
        // shared (e.g. an init script two packages ship) or were taken over
        // by a `replaces` package.  Deleting them would break that package.
        let others = {
            let wanted: HashSet<&str> = pkg.files.iter().map(|e| e.path.as_str()).collect();
            match db.path_owners(Some(&wanted), Some(pkg_name)) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("jpkg: cannot check shared ownership for {pkg_name}: {e}");
                    failures += 1;
                    continue;
                }
            }
        };

        let stats = remove_package_files(&rootfs, &pkg, &others);
        if stats.errors > 0 {
            log::warn!(
                "jpkg: {} file(s) could not be removed from {pkg_name}",
                stats.errors
            );
        }
        if stats.kept > 0 {
            log::info!(
                "jpkg: kept {} path(s) of {pkg_name} that other packages still own",
                stats.kept
            );
        }
        if stats.config_kept > 0 {
            log::warn!(
                "jpkg: kept {} locally changed config file(s) of {pkg_name}; no package owns them now",
                stats.config_kept
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

// ─── remove_package_files ────────────────────────────────────────────────────

/// Outcome of [`remove_package_files`].
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RemoveStats {
    /// Files/symlinks unlinked.
    pub removed: usize,
    /// Non-directory paths left in place because another package owns them.
    pub kept: usize,
    /// Unlink failures other than "already absent".
    pub errors: usize,
    /// Config files left in place, unowned, because they were changed
    /// locally or could not be checked (2.2.11).
    pub config_kept: usize,
}

/// Delete `pkg`'s files from `rootfs`, skipping every path that `others`
/// (the ownership index of all OTHER installed packages) still claims.
///
/// Files go in reverse path order (children before parents) so a directory
/// is only `rmdir`ed once everything under it is gone; populated directories
/// are left in place, mirroring the C `rmdir` call.
pub(crate) fn remove_package_files(
    rootfs: &Path,
    pkg: &InstalledPkg,
    others: &Ownership,
) -> RemoveStats {
    let mut stats = RemoveStats::default();
    let mut files = pkg.files.clone();
    files.sort_by(|a, b| b.path.cmp(&a.path));

    for entry in &files {
        if others.is_claimed(&entry.path) {
            if !entry.is_dir {
                log::debug!(
                    "jpkg: keeping /{} (still owned by {})",
                    entry.path,
                    others
                        .owners_of(&entry.path)
                        .iter()
                        .map(|c| c.owner.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                stats.kept += 1;
            }
            continue;
        }
        let full = rootfs.join(&entry.path);

        match full.symlink_metadata() {
            Err(_) => {
                log::debug!("jpkg: file already absent: {}", entry.path);
            }
            Ok(m) => {
                if crate::config::keep_on_disk(rootfs, entry) {
                    // 2.2.11: checked before anything else, so not even an
                    // (empty) directory the admin put at a config path is
                    // removed.  The file is now owned by no package.
                    log::warn!(
                        "jpkg: kept /{} (changed locally; no package owns it now)",
                        entry.path
                    );
                    stats.config_kept += 1;
                } else if m.is_dir() && !m.file_type().is_symlink() {
                    // Only remove directory if empty (mirrors C rmdir call).
                    if let Err(e) = fs::remove_dir(&full) {
                        log::debug!("jpkg: leaving non-empty dir {}: {e}", entry.path);
                    }
                } else if let Err(e) = fs::remove_file(&full) {
                    log::warn!("jpkg: failed to remove {}: {e}", entry.path);
                    stats.errors += 1;
                } else {
                    stats.removed += 1;
                }
            }
        }
        if crate::config::is_config(entry) {
            // A <path>.jpkg-new holding exactly this package's copy goes
            // with it; the admin's own .jpkg-new edits stay.
            if let Err(e) =
                crate::config::drop_stale_new(rootfs, &entry.path, &[entry.sha256.as_str()])
            {
                log::warn!("jpkg: could not check /{}{}: {e}", entry.path, crate::config::NEW_SUFFIX);
            }
        }
    }
    stats
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
        assert!(
            rootfs.join("bin/foo").exists(),
            "bin/foo should be installed"
        );
        assert!(
            rootfs.join("lib/bar").exists(),
            "lib/bar should be installed"
        );
        assert!(db.get("rmpkg").unwrap().is_some(), "rmpkg should be in db");

        // Remove via the helper directly (bypasses run() to avoid process::exit).
        let pkg = db.get("rmpkg").unwrap().unwrap();
        let others = db.path_owners(None, Some("rmpkg")).unwrap();
        let stats = remove_package_files(&rootfs, &pkg, &others);
        assert_eq!(stats.kept, 0);
        assert_eq!(stats.errors, 0);
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
            tmp.path(),
            "hookrm",
            "1.0.0",
            hook,
        );
        // Rebuild with pre_remove instead — we need a bespoke helper here.
        // Use the archive and db APIs directly to register a fake pkg with a pre_remove hook.
        drop(jpkg_path);

        use crate::db::InstalledPkg;
        use crate::recipe::{DependsSection, FilesSection, HooksSection, Metadata, PackageSection};

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
        })
        .unwrap();

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

    // ── 3. Shared paths survive removal of one owner ──────────────────────────
    //
    // jonerix-raspi5-fixups and openrc both ship etc/init.d/hwclock.  Removing
    // fixups must not delete the file openrc still owns.

    #[test]
    fn remove_keeps_paths_another_package_owns() {
        use crate::db::FileEntry;
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(rootfs.join("etc/init.d")).unwrap();
        fs::create_dir_all(rootfs.join("bin")).unwrap();
        fs::write(rootfs.join("etc/init.d/hwclock"), b"#!/bin/openrc-run\n").unwrap();
        fs::write(rootfs.join("bin/pi5-only"), b"x").unwrap();

        let file = |p: &str| FileEntry {
            path: p.to_string(),
            sha256: "a".repeat(64),
            size: 0,
            mode: 0o100755,
            symlink_target: None,
            is_dir: false,
        };
        let dir = |p: &str| FileEntry {
            path: p.to_string(),
            sha256: "0".repeat(64),
            size: 0,
            mode: 0o040755,
            symlink_target: None,
            is_dir: true,
        };
        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();
        db.insert(&InstalledPkg {
            metadata: common_tests::make_metadata("openrc", "0.54"),
            files: vec![dir("etc"), dir("etc/init.d"), file("etc/init.d/hwclock")],
        })
        .unwrap();
        db.insert(&InstalledPkg {
            metadata: common_tests::make_metadata("fixups", "1.6"),
            files: vec![
                dir("bin"),
                file("bin/pi5-only"),
                dir("etc"),
                dir("etc/init.d"),
                file("etc/init.d/hwclock"),
            ],
        })
        .unwrap();

        let pkg = db.get("fixups").unwrap().unwrap();
        let others = db.path_owners(None, Some("fixups")).unwrap();
        let stats = remove_package_files(&rootfs, &pkg, &others);

        assert_eq!(stats.kept, 1, "hwclock is shared with openrc");
        assert_eq!(stats.removed, 1, "only bin/pi5-only is fixups-only");
        assert!(rootfs.join("etc/init.d/hwclock").exists());
        assert!(rootfs.join("etc/init.d").is_dir());
        assert!(!rootfs.join("bin/pi5-only").exists());
        assert!(
            !rootfs.join("bin").exists(),
            "empty unshared dir is removed"
        );
    }

    fn conf_root(tmp: &TempDir) -> (std::path::PathBuf, InstalledDb) {
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        (rootfs, db)
    }

    #[test]
    fn remove_keeps_changed_config_and_deletes_unchanged_one() {
        use crate::cmd::common::tests::{build_jpkg_tree, Node};
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = conf_root(&tmp);
        let _lock = db.lock().unwrap();
        let j = build_jpkg_tree(
            tmp.path(),
            "confpkg",
            "1",
            &[],
            &[
                ("etc/edited.conf", Node::File(b"stock\n", 0o644)),
                ("etc/pristine.conf", Node::File(b"stock\n", 0o644)),
            ],
        );
        extract_and_register(&JpkgArchive::open(&j).unwrap(), &rootfs, &db).unwrap();
        fs::write(rootfs.join("etc/edited.conf"), b"mine\n").unwrap();
        let pkg = db.get("confpkg").unwrap().unwrap();
        let others = db.path_owners(None, Some("confpkg")).unwrap();
        let stats = remove_package_files(&rootfs, &pkg, &others);
        assert_eq!(stats.config_kept, 1);
        assert_eq!(stats.errors, 0);
        assert_eq!(
            fs::read(rootfs.join("etc/edited.conf")).unwrap(),
            b"mine\n",
            "a changed config file stays in place"
        );
        assert!(!rootfs.join("etc/edited.conf.jpkg-save").exists(), "no rename on remove");
        assert!(!rootfs.join("etc/pristine.conf").exists());
    }

    #[test]
    fn remove_deletes_jpkg_new_equal_to_package_content() {
        use crate::cmd::common::tests::{build_jpkg_tree, Node};
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = conf_root(&tmp);
        let _lock = db.lock().unwrap();
        let j = build_jpkg_tree(
            tmp.path(),
            "confpkg",
            "1",
            &[],
            &[
                ("etc/a.conf", Node::File(b"pkg\n", 0o644)),
                ("etc/b.conf", Node::File(b"pkg\n", 0o644)),
            ],
        );
        extract_and_register(&JpkgArchive::open(&j).unwrap(), &rootfs, &db).unwrap();
        fs::write(rootfs.join("etc/a.conf"), b"mine\n").unwrap();
        fs::write(rootfs.join("etc/a.conf.jpkg-new"), b"pkg\n").unwrap(); // package content
        fs::write(rootfs.join("etc/b.conf"), b"mine\n").unwrap();
        fs::write(rootfs.join("etc/b.conf.jpkg-new"), b"admin tweaked the new one\n").unwrap();
        let pkg = db.get("confpkg").unwrap().unwrap();
        let others = db.path_owners(None, Some("confpkg")).unwrap();
        remove_package_files(&rootfs, &pkg, &others);
        assert!(!rootfs.join("etc/a.conf.jpkg-new").exists(), "package content goes");
        assert_eq!(
            fs::read(rootfs.join("etc/b.conf.jpkg-new")).unwrap(),
            b"admin tweaked the new one\n",
            "anything else stays"
        );
    }

    #[test]
    fn remove_keeps_a_file_the_admin_put_in_place_of_a_packaged_link() {
        use crate::cmd::common::tests::{build_jpkg_tree, Node};
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = conf_root(&tmp);
        let _lock = db.lock().unwrap();
        let j = build_jpkg_tree(
            tmp.path(),
            "ca",
            "1",
            &[],
            &[
                ("etc/ssl/certs/ca.crt", Node::File(b"ca\n", 0o644)),
                ("etc/ssl/cert.pem", Node::Link("certs/ca.crt")),
                ("etc/ssl/ca.pem", Node::Link("certs/ca.crt")),
            ],
        );
        extract_and_register(&JpkgArchive::open(&j).unwrap(), &rootfs, &db).unwrap();
        fs::remove_file(rootfs.join("etc/ssl/cert.pem")).unwrap();
        fs::write(rootfs.join("etc/ssl/cert.pem"), b"private CA\n").unwrap();
        let pkg = db.get("ca").unwrap().unwrap();
        let others = db.path_owners(None, Some("ca")).unwrap();
        let stats = remove_package_files(&rootfs, &pkg, &others);
        assert_eq!(stats.config_kept, 1);
        assert_eq!(fs::read(rootfs.join("etc/ssl/cert.pem")).unwrap(), b"private CA\n");
        assert!(rootfs.join("etc/ssl/ca.pem").symlink_metadata().is_err(), "jpkg's own link goes");
    }
}
