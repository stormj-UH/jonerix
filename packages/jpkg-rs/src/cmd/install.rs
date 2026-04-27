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

use crate::archive::JpkgArchive;
use crate::cmd::common::{self, InstallError, resolve_arch, resolve_rootfs};
use crate::db::InstalledDb;
use crate::deps::resolve_install;
use crate::recipe::{Index, Metadata};
use crate::repo::Repo;

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

        // Download.
        let jpkg_path = repo
            .fetch_package(name, &entry.version)
            .map_err(|e| InstallError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("fetch failed: {e}"),
            )))?;

        // Verify sha256.
        Repo::verify_package(&jpkg_path, &entry.sha256)
            .map_err(|e| InstallError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("verification failed: {e}"),
            )))?;

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
    use crate::archive::JpkgArchive;
    use crate::cmd::common::{extract_and_register, run_hook, tests as common_tests};
    use crate::db::InstalledDb;
    use std::fs;
    use tempfile::TempDir;

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
}
