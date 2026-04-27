/*
 * jpkg - jonerix package manager
 * cmd/upgrade.rs - jpkg upgrade: compare installed vs INDEX, install newer
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 *
 * Port of jpkg/src/cmd_upgrade.c (158 lines).
 *
 * Divergences from C:
 * - The C code refreshes the INDEX unconditionally (cmd_upgrade.c:36-49).
 *   We prefer the cached INDEX and only fetch if absent, matching the install
 *   path.  Callers that want a fresh INDEX should run `jpkg update` first.
 *   This is deliberate: in CI, `jpkg update` is a separate step.
 * - We call install_packages() directly instead of rebuilding an argv[] and
 *   calling cmd_install() (cmd_upgrade.c:141), which avoids the boilerplate
 *   of synthetic arg vectors.
 */

use std::cmp::Ordering;

use crate::cmd::common::{resolve_arch, resolve_rootfs};
use crate::cmd::install::install_packages;
use crate::db::InstalledDb;
use crate::repo::Repo;
use crate::util::version_compare;

// ─── public entry point ───────────────────────────────────────────────────────

/// `jpkg upgrade [<pkg>...]`
///
/// With no arguments: upgrade all installed packages that have a newer version
/// in the INDEX.  With package names: upgrade only those.
///
/// Returns 0 on success, 1 on failure.
pub fn run(args: &[String]) -> i32 {
    let explicit_targets: Vec<String> = args.iter().cloned().collect();

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

    let index = match load_index_for_upgrade(&repo) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("jpkg: {e}");
            return 1;
        }
    };

    // ── Build upgrade list ────────────────────────────────────────────────
    let candidates = if explicit_targets.is_empty() {
        // Upgrade all installed packages.
        match db.list() {
            Ok(names) => names,
            Err(e) => {
                eprintln!("jpkg: failed to list installed packages: {e}");
                return 1;
            }
        }
    } else {
        // Verify every named package is actually installed.
        let mut failures = 0usize;
        for name in &explicit_targets {
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
        if failures > 0 {
            return 1;
        }
        explicit_targets.clone()
    };

    // ── Find packages with newer INDEX version ────────────────────────────
    let mut to_upgrade: Vec<String> = Vec::new();
    let mut selection_failures = 0usize;

    for name in &candidates {
        let installed = match db.get(name) {
            Ok(Some(p)) => p,
            Ok(None) => continue, // shouldn't happen but be defensive
            Err(e) => {
                eprintln!("jpkg: db error for {name}: {e}");
                selection_failures += 1;
                continue;
            }
        };

        let entry = match index.get(name, &arch) {
            Some(e) => e,
            None => {
                log::debug!("jpkg: {name} not found in INDEX, skipping");
                continue;
            }
        };

        let installed_ver = installed
            .metadata
            .package
            .version
            .as_deref()
            .unwrap_or("");

        let cmp = version_compare(entry.version.as_str(), installed_ver);
        if cmp == Ordering::Greater {
            eprintln!(
                "jpkg:   {}: {} -> {}",
                name, installed_ver, entry.version
            );
            to_upgrade.push(name.clone());
        } else if !explicit_targets.is_empty() {
            // Explicitly requested but already at the latest — log it.
            eprintln!("jpkg:   {name}: {installed_ver} is already the latest");
        }
    }

    if selection_failures > 0 {
        eprintln!("jpkg: {selection_failures} package(s) could not be checked");
        return 1;
    }

    if to_upgrade.is_empty() {
        eprintln!("jpkg: all packages are up to date");
        return 0;
    }

    eprintln!("jpkg: {} package(s) to upgrade", to_upgrade.len());

    // ── Install with force=true ───────────────────────────────────────────
    // force=true bypasses the "same version, already installed" check in
    // install_packages, which is correct here: we've already verified these
    // have newer versions available.
    match install_packages(&db, &repo, &index, &arch, &to_upgrade, true) {
        Ok(n) => {
            eprintln!("jpkg: {n} package(s) upgraded");
            0
        }
        Err(e) => {
            eprintln!("jpkg: upgrade failed: {e}");
            1
        }
    }
}

// ─── load_index_for_upgrade ───────────────────────────────────────────────────

/// Prefer cached INDEX; fall back to fetch.
fn load_index_for_upgrade(repo: &Repo) -> Result<crate::recipe::Index, String> {
    match repo.load_cached_index() {
        Ok(Some(idx)) => {
            log::debug!("jpkg: using cached INDEX for upgrade check");
            return Ok(idx);
        }
        Ok(None) => {
            log::info!("jpkg: no cached INDEX, fetching");
        }
        Err(e) => {
            log::warn!("jpkg: cached INDEX unreadable ({e}), fetching");
        }
    }
    repo.fetch_index()
        .map_err(|e| format!("failed to fetch INDEX: {e}"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────
//
// Upgrade logic tests use extract_and_register() as the test seam (bypassing
// the repo/network layer, same pattern as install.rs tests).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::JpkgArchive;
    use crate::cmd::common::{extract_and_register, tests as common_tests};
    use crate::db::InstalledDb;
    use std::fs;
    use tempfile::TempDir;

    // ── 1. version_compare selects the right upgrade candidates ───────────────

    #[test]
    fn test_version_compare_upgrade_selection() {
        assert_eq!(version_compare("1.1.0", "1.0.0"), Ordering::Greater);
        assert_eq!(version_compare("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(version_compare("0.9.0", "1.0.0"), Ordering::Less);
    }

    // ── 2. Upgrade: install v1, then install v2 with force → db shows v2 ──────

    #[test]
    fn test_upgrade_installs_newer_version() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        // Install 1.0.0 via test seam.
        let jpkg_v1 = common_tests::build_test_jpkg(tmp.path(), "upgpkg", "1.0.0");
        let arc_v1 = JpkgArchive::open(&jpkg_v1).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();
        extract_and_register(&arc_v1, &rootfs, &db).unwrap();

        let installed_before = db.get("upgpkg").unwrap().unwrap();
        assert_eq!(
            installed_before.metadata.package.version.as_deref(),
            Some("1.0.0")
        );

        // Simulate upgrade: extract_and_register v1.1.0, overwriting the db entry.
        let jpkg_v2 = common_tests::build_test_jpkg(tmp.path(), "upgpkg", "1.1.0");
        let arc_v2 = JpkgArchive::open(&jpkg_v2).unwrap();
        extract_and_register(&arc_v2, &rootfs, &db).unwrap();

        let after = db.get("upgpkg").unwrap().unwrap();
        assert_eq!(
            after.metadata.package.version.as_deref(),
            Some("1.1.0"),
            "db should reflect upgraded version"
        );
    }

    // ── 3. version_compare: newer-in-index vs installed drives the list ───────

    #[test]
    fn test_upgrade_nothing_to_upgrade() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let jpkg = common_tests::build_test_jpkg(tmp.path(), "latepkg", "2.0.0");
        let arc = JpkgArchive::open(&jpkg).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();
        extract_and_register(&arc, &rootfs, &db).unwrap();

        let installed = db.get("latepkg").unwrap().unwrap();
        let installed_ver = installed.metadata.package.version.as_deref().unwrap_or("");

        // Index version == installed version → no upgrade.
        let index_ver = "2.0.0";
        assert_ne!(
            version_compare(index_ver, installed_ver),
            Ordering::Greater,
            "same version should not trigger upgrade"
        );
    }
}
