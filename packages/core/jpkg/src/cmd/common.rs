/*
 * jpkg - jonerix package manager
 * cmd/common.rs - Shared helpers used by install, remove, upgrade, local_install
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 *
 * Port of the shared machinery scattered across cmd_install.c (lines 36-203)
 * and main_local.c (lines 100-186).
 */

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use walkdir::WalkDir;

use crate::archive::{ArchiveError, JpkgArchive};
use crate::db::{DbError, FileEntry, InstalledDb, InstalledPkg};
use crate::recipe::{Metadata, RecipeError};
use crate::util::sha256_file;

// ─── InstallError ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum InstallError {
    Io(io::Error),
    Archive(ArchiveError),
    Db(DbError),
    Recipe(RecipeError),
    HookFailed { hook: &'static str, status: i32 },
    Conflict { path: String, owned_by: String },
    /// Package has no signature but `signature_policy = require`.
    SignatureMissing { name: String, version: String },
    /// Package carries a signature but it did not verify.
    SignatureInvalid { name: String, version: String, reason: String },
    /// Package carries a signature referencing an unknown key.
    UnknownSigningKey { name: String, key_id: String },
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::Io(e) => write!(f, "I/O error: {e}"),
            InstallError::Archive(e) => write!(f, "archive error: {e}"),
            InstallError::Db(e) => write!(f, "database error: {e}"),
            InstallError::Recipe(e) => write!(f, "metadata error: {e}"),
            InstallError::HookFailed { hook, status } => {
                write!(f, "{hook} hook failed with exit status {status}")
            }
            InstallError::Conflict { path, owned_by } => {
                write!(f, "file conflict: {path} is owned by {owned_by}")
            }
            InstallError::SignatureMissing { name, version } => {
                write!(f, "signature missing for {name}-{version} (policy=require)")
            }
            InstallError::SignatureInvalid { name, version, reason } => {
                write!(f, "signature invalid for {name}-{version}: {reason}")
            }
            InstallError::UnknownSigningKey { name, key_id } => {
                write!(f, "unknown signing key {key_id} for package {name}")
            }
        }
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            InstallError::Io(e) => Some(e),
            InstallError::Archive(e) => Some(e),
            InstallError::Db(e) => Some(e),
            InstallError::Recipe(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(e: io::Error) -> Self {
        InstallError::Io(e)
    }
}

impl From<ArchiveError> for InstallError {
    fn from(e: ArchiveError) -> Self {
        InstallError::Archive(e)
    }
}

impl From<DbError> for InstallError {
    fn from(e: DbError) -> Self {
        InstallError::Db(e)
    }
}

impl From<RecipeError> for InstallError {
    fn from(e: RecipeError) -> Self {
        InstallError::Recipe(e)
    }
}

// ─── build_manifest ──────────────────────────────────────────────────────────

/// Walk `tree_root` recursively in sorted order and build a file manifest.
///
/// Paths are stored relative to `tree_root`, without a leading `/`.
/// Mirrors `build_file_manifest` in cmd_install.c:141-204 and
/// main_local.c:103-160.
///
/// Divergence from C: we use `walkdir` instead of raw opendir/readdir to get
/// deterministic sorted traversal for free, and we compute real SHA-256 for
/// regular files (the C code also does this via `sha256_file`).  For symlinks
/// the C code writes 64 zeros; we store an empty string in FileEntry.sha256
/// (matching the db.rs wire format — db.c:88-89 uses the all-zeros sentinel
/// in the manifest but that is re-added at serialisation time by db.rs).
pub fn build_manifest(tree_root: &Path) -> io::Result<Vec<FileEntry>> {
    let mut entries = Vec::new();

    for entry in WalkDir::new(tree_root).sort_by_file_name().into_iter() {
        let entry = entry.map_err(|e| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("walkdir error: {e}"),
            )
        })?;

        let abs = entry.path();

        // Skip the root itself.
        if abs == tree_root {
            continue;
        }

        let rel = abs
            .strip_prefix(tree_root)
            .expect("walkdir yields children of root")
            .to_string_lossy()
            .into_owned();

        let meta = abs.symlink_metadata()?;
        let mode = meta.mode();

        if meta.file_type().is_symlink() {
            let target = fs::read_link(abs)?
                .to_string_lossy()
                .into_owned();
            entries.push(FileEntry {
                path: rel,
                sha256: String::new(), // db.rs serialises the zeros sentinel for symlinks
                size: 0,
                mode,
                symlink_target: Some(target),
                is_dir: false,
            });
        } else if meta.is_dir() {
            entries.push(FileEntry {
                path: rel,
                sha256: "0".repeat(64),
                size: 0,
                mode,
                symlink_target: None,
                is_dir: true,
            });
        } else {
            // Regular file.
            let digest = sha256_file(abs)?;
            let size = meta.len();
            entries.push(FileEntry {
                path: rel,
                sha256: digest,
                size,
                mode,
                symlink_target: None,
                is_dir: false,
            });
        }
    }

    Ok(entries)
}

// ─── run_hook ─────────────────────────────────────────────────────────────────

/// Run a shell hook string in the context of `rootfs`.
///
/// # Strategy (mirrors cmd_install.c:36-131)
///
/// 1. If the hook body is empty, do nothing (return Ok).
/// 2. If uid == 0 AND rootfs != "/" AND rootfs/bin/sh exists:
///    - Bind-mount /dev, /proc, /sys into the rootfs via a POSIX shell wrapper
///      (same approach as the C code — we shell out to system() for the mounts
///      rather than calling nix::mount directly, because that keeps us free of
///      Linux-only mount(2) syscalls and matches the C's portability posture).
///    - Pass the hook body to the chrooted shell via a heredoc with the unique
///      delimiter `__JPKG_HOOK_EOF__` so shell metacharacters survive two
///      layers of parsing.
///    - Unmount in reverse on exit (via shell trap).
/// 3. Otherwise (non-root, or rootfs == "/", or no /bin/sh yet):
///    - Run `/bin/sh -c <body>` on the host with `JPKG_ROOT=<rootfs>` and
///      `DESTDIR=<rootfs>` in the environment.
///    - This is the fallback the C code uses (cmd_install.c:107-122) and is
///      also the path taken during tests (which run unprivileged).
///
/// Returns `Err(io::Error)` on execution failure; hook non-zero exit is mapped
/// to `InstallError::HookFailed` by the callers in install.rs.
pub fn run_hook(rootfs: &Path, hook_body: &str) -> io::Result<ExitStatus> {
    if hook_body.is_empty() {
        // Simulate a zero exit so callers don't have to special-case this.
        return Ok(std::process::Command::new("true").status()?);
    }

    let rootfs_str = rootfs.to_string_lossy();

    // Decide which execution path to take.
    let use_chroot = {
        // uid == 0 check (nix::unistd::getuid()).
        let is_root = nix::unistd::getuid().is_root();
        let not_slash = rootfs != Path::new("/");
        let sh_exists = rootfs.join("bin/sh").exists();
        is_root && not_slash && sh_exists
    };

    if use_chroot {
        // Build the outer shell script that mounts, chroots, and unmounts.
        // The heredoc delimiter is chosen to be unlikely in any real hook body.
        let script = format!(
            r#"ROOT='{root}'
mkdir -p "$ROOT/dev" "$ROOT/proc" "$ROOT/sys" 2>/dev/null
_unmount() {{
    umount "$ROOT/sys"  2>/dev/null || true
    umount "$ROOT/proc" 2>/dev/null || true
    umount "$ROOT/dev"  2>/dev/null || true
}}
mountpoint -q "$ROOT/dev" 2>/dev/null || mount --bind /dev "$ROOT/dev" 2>/dev/null || true
mountpoint -q "$ROOT/proc" 2>/dev/null || mount -t proc proc "$ROOT/proc" 2>/dev/null || mount --bind /proc "$ROOT/proc" 2>/dev/null || true
mountpoint -q "$ROOT/sys" 2>/dev/null || mount -t sysfs sysfs "$ROOT/sys" 2>/dev/null || mount --bind /sys "$ROOT/sys" 2>/dev/null || true
trap _unmount EXIT INT TERM
chroot "$ROOT" /bin/sh <<'__JPKG_HOOK_EOF__'
{body}
__JPKG_HOOK_EOF__
_rc=$?
_unmount
trap - EXIT INT TERM
exit $_rc
"#,
            root = rootfs_str,
            body = hook_body,
        );
        Command::new("/bin/sh").arg("-c").arg(&script).status()
    } else {
        // Non-root or no /bin/sh in rootfs yet — run on host with env vars.
        // Mirrors cmd_install.c:107-122.
        Command::new("/bin/sh")
            .arg("-c")
            .arg(hook_body)
            .env("JPKG_ROOT", rootfs_str.as_ref())
            .env("DESTDIR", rootfs_str.as_ref())
            .status()
    }
}

// ─── flatten_merged_usr ───────────────────────────────────────────────────────

/// Flatten `destdir/usr/` into `destdir/` and `destdir/lib64/` into `destdir/lib/`.
///
/// Mirrors the shell one-liners in cmd_build.c:517-535 and main_local.c:174-186:
/// ```sh
/// if [ -d '$destdir/usr' ] && [ ! -L '$destdir/usr' ]; then
///     cp -a '$destdir/usr/.' '$destdir/' && rm -rf '$destdir/usr'
/// fi
/// if [ -d '$destdir/lib64' ] && [ ! -L '$destdir/lib64' ]; then
///     cp -a '$destdir/lib64/.' '$destdir/lib/' && rm -rf '$destdir/lib64'
/// fi
/// ```
///
/// We do this in pure Rust rather than shelling out so we can handle the case
/// where bsdtar / toybox is absent (tests).  The algorithm:
///
/// 1. Enumerate `src/` recursively.
/// 2. For each file, compute `dest = destdir/ + path_relative_to_src`.
/// 3. If dest already exists and both are regular files, overwrite it.
/// 4. If dest is an existing symlink, recreate it.
/// 5. Remove `src/` tree after copying.
pub fn flatten_merged_usr(destdir: &Path) -> io::Result<()> {
    flatten_dir_into(destdir, "usr", destdir)?;
    let lib_dest = destdir.join("lib");
    fs::create_dir_all(&lib_dest)?;
    flatten_dir_into(destdir, "lib64", &lib_dest)?;
    Ok(())
}

/// Move all contents of `destdir/<src_name>/` into `dest_dir/`, then remove the src dir.
fn flatten_dir_into(destdir: &Path, src_name: &str, dest_dir: &Path) -> io::Result<()> {
    let src = destdir.join(src_name);

    // Only act on a real directory, not a symlink.
    match src.symlink_metadata() {
        Ok(m) if m.file_type().is_dir() => {}
        _ => return Ok(()), // not present or is a symlink — nothing to do
    }

    // Walk src, recreate tree under dest_dir.
    for entry in WalkDir::new(&src).sort_by_file_name().into_iter() {
        let entry = entry.map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("walkdir: {e}"))
        })?;
        let abs = entry.path();
        let rel = abs.strip_prefix(&src).expect("walkdir child of src");
        let dest = dest_dir.join(rel);

        let m = abs.symlink_metadata()?;

        if m.file_type().is_symlink() {
            let target = fs::read_link(abs)?;
            // Remove existing dest if it exists.
            if dest.symlink_metadata().is_ok() {
                if dest.symlink_metadata()?.is_dir() {
                    fs::remove_dir_all(&dest)?;
                } else {
                    fs::remove_file(&dest)?;
                }
            }
            std::os::unix::fs::symlink(target, &dest)?;
        } else if m.is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            // Regular file — overwrite.
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(abs, &dest)?;
        }
    }

    // Remove the now-duplicated source tree.
    fs::remove_dir_all(&src)?;

    Ok(())
}

// ─── install_files ────────────────────────────────────────────────────────────

/// Copy all files from `stage_dir` into `rootfs`.
///
/// Uses `WalkDir` + atomic per-file copy (regular files) / symlink recreation
/// rather than shelling out to tar.  This avoids the toybox-symlink-follow bug
/// documented in cmd_install.c:248-260 and the pipe deadlock with large packages
/// (cmd_install.c:260-262).
///
/// Directories are created first; symlinks are recreated verbatim; regular files
/// are overwritten.  This mirrors `tar -x` semantics: a destination symlink is
/// replaced by the new file, not followed.
fn install_files(stage_dir: &Path, rootfs: &Path) -> io::Result<()> {
    for entry in WalkDir::new(stage_dir).sort_by_file_name().into_iter() {
        let entry = entry.map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("walkdir: {e}"))
        })?;
        let abs = entry.path();

        if abs == stage_dir {
            continue; // skip root
        }

        let rel = abs
            .strip_prefix(stage_dir)
            .expect("walkdir child of stage_dir");
        let dest = rootfs.join(rel);

        let m = abs.symlink_metadata()?;

        if m.file_type().is_symlink() {
            let target = fs::read_link(abs)?;
            // Remove any existing dest (symlink, file, or empty dir) before
            // recreating — matches tar -x "replace symlinks, not follow" behaviour.
            if dest.symlink_metadata().is_ok() {
                let dm = dest.symlink_metadata()?;
                if dm.is_dir() && !dm.file_type().is_symlink() {
                    let _ = fs::remove_dir(&dest);
                } else {
                    let _ = fs::remove_file(&dest);
                }
            }
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p)?;
            }
            std::os::unix::fs::symlink(target, &dest)?;
        } else if m.is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            // Regular file — remove any existing symlink/file at dest first so
            // we do not inadvertently write through a symlink.
            if dest.symlink_metadata().is_ok() {
                let _ = fs::remove_file(&dest);
            }
            if let Some(p) = dest.parent() {
                fs::create_dir_all(p)?;
            }
            fs::copy(abs, &dest)?;
        }
    }
    Ok(())
}

// ─── extract_and_register ────────────────────────────────────────────────────

/// Extract `archive` into `rootfs`, build the file manifest, register in `db`,
/// and handle `replaces = [...]` ownership transfer.
///
/// Shape:
/// 1. Parse the embedded TOML metadata.
/// 2. Create a temp staging directory under `/tmp`.
/// 3. `archive.extract(stage)` — decompresses the zstd(tar) payload.
/// 4. `flatten_merged_usr(stage)` — jonerix merged-usr layout.
/// 5. `install_files(stage, rootfs)` — copy to final destination.
/// 6. `build_manifest(stage)` — walk the stage dir for the file list.
/// 7. `db.insert(InstalledPkg { metadata, files })`.
/// 8. For each name in `metadata.package.replaces`:
///    `db.transfer_ownership(replaced, pkg_name, &shared_paths)`.
/// 9. Clean up the staging directory.
///
/// Divergences from C:
/// - We build the manifest from the staging dir (step 6), not from the rootfs,
///   so paths are relative without the rootfs prefix — matching db.c's format.
/// - We do the flatten before copying so the staging dir is canonical; the C
///   code's `install_files` does a safety-re-flatten inline (cmd_install.c:268).
/// - No `audit_layout_tree` call; the archive crate already enforces this at
///   create time via `ArchiveError::UnflatLayout`.
pub fn extract_and_register(
    archive: &JpkgArchive,
    rootfs: &Path,
    db: &InstalledDb,
) -> Result<InstalledPkg, InstallError> {
    // ── 1. Parse metadata ─────────────────────────────────────────────────
    let metadata = Metadata::from_str(archive.metadata())?;
    let pkg_name = metadata
        .package
        .name
        .as_deref()
        .unwrap_or("(unnamed)")
        .to_string();

    // ── 2. Staging dir ────────────────────────────────────────────────────
    let stage_dir = std::env::temp_dir().join(format!(
        "jpkg-stage-{}-{}",
        pkg_name,
        std::process::id()
    ));
    fs::create_dir_all(&stage_dir)?;

    // Ensure cleanup on error.
    let stage_guard = StageDirGuard(&stage_dir);

    // ── 3. Extract ────────────────────────────────────────────────────────
    archive.extract(&stage_dir)?;

    // ── 4. Flatten usr/ and lib64/ ────────────────────────────────────────
    flatten_merged_usr(&stage_dir)?;

    // ── 5. Install files into rootfs ──────────────────────────────────────
    install_files(&stage_dir, rootfs)?;

    // ── 6. Build manifest from staging dir ───────────────────────────────
    let files = build_manifest(&stage_dir)?;

    // ── 7. Register in DB ─────────────────────────────────────────────────
    let pkg = InstalledPkg {
        metadata: metadata.clone(),
        files: files.clone(),
    };
    db.insert(&pkg)?;

    // ── 8. Transfer ownership for replaces = [...] ───────────────────────
    // Collect paths we now own.
    let our_paths: Vec<&str> = files.iter().map(|e| e.path.as_str()).collect();

    for replaced_name in &metadata.package.replaces {
        if replaced_name.is_empty() {
            continue;
        }
        // Only transfer if they exist in the db.
        if db.get(replaced_name)?.is_some() {
            db.transfer_ownership(replaced_name, &pkg_name, &our_paths)?;
        }
    }

    // ── 9. Cleanup ────────────────────────────────────────────────────────
    drop(stage_guard); // removes stage_dir

    Ok(pkg)
}

// ─── StageDirGuard ───────────────────────────────────────────────────────────

/// RAII guard: removes the staging directory on drop (even on error paths).
struct StageDirGuard<'a>(&'a Path);

impl Drop for StageDirGuard<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0);
    }
}

// ─── resolve_rootfs ──────────────────────────────────────────────────────────

/// Determine the effective rootfs path.
///
/// Precedence: `--root` CLI arg > `$JPKG_ROOT` env > `"/"`.
pub fn resolve_rootfs(root_arg: Option<&str>) -> PathBuf {
    if let Some(r) = root_arg {
        return PathBuf::from(r);
    }
    if let Ok(r) = std::env::var("JPKG_ROOT") {
        if !r.is_empty() {
            return PathBuf::from(r);
        }
    }
    PathBuf::from("/")
}

// ─── resolve_arch ────────────────────────────────────────────────────────────

/// Determine the target architecture string.
/// Falls back to uname(2) via the `nix` crate.
pub fn resolve_arch() -> String {
    if let Ok(a) = std::env::var("JPKG_ARCH") {
        if !a.is_empty() {
            return a;
        }
    }
    match nix::sys::utsname::uname() {
        Ok(u) => u.machine().to_string_lossy().into_owned(),
        Err(_) => "x86_64".to_string(),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::archive;
    use crate::db::InstalledDb;
    use crate::recipe::{DependsSection, HooksSection, Metadata, PackageSection, FilesSection};
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    // ── Helpers ───────────────────────────────────────────────────────────────

    pub fn make_metadata(name: &str, version: &str) -> Metadata {
        Metadata {
            package: PackageSection {
                name: Some(name.to_string()),
                version: Some(version.to_string()),
                license: Some("MIT".to_string()),
                description: Some(format!("{name} test package")),
                arch: Some("x86_64".to_string()),
                replaces: vec![],
                conflicts: vec![],
            },
            depends: DependsSection::default(),
            hooks: HooksSection::default(),
            files: FilesSection::default(),
            signature: None,
        }
    }

    pub fn make_metadata_with_replaces(name: &str, version: &str, replaces: Vec<String>) -> Metadata {
        let mut m = make_metadata(name, version);
        m.package.replaces = replaces;
        m
    }

    /// Build a minimal synthetic .jpkg for testing.
    /// Contents:
    ///   bin/foo  — regular file with "foo content\n"
    ///   lib/bar  — regular file with "bar content\n"
    pub fn build_test_jpkg(tmp: &Path, name: &str, version: &str) -> PathBuf {
        let destdir = tmp.join(format!("destdir-{name}"));
        fs::create_dir_all(destdir.join("bin")).unwrap();
        fs::create_dir_all(destdir.join("lib")).unwrap();
        fs::write(destdir.join("bin/foo"), b"foo content\n").unwrap();
        fs::write(destdir.join("lib/bar"), b"bar content\n").unwrap();

        let meta = Metadata {
            package: PackageSection {
                name: Some(name.to_string()),
                version: Some(version.to_string()),
                license: Some("MIT".to_string()),
                description: Some("test".to_string()),
                arch: Some("x86_64".to_string()),
                replaces: vec![],
                conflicts: vec![],
            },
            depends: DependsSection::default(),
            hooks: HooksSection::default(),
            files: FilesSection::default(),
            signature: None,
        };
        let meta_toml = meta.to_string().unwrap();

        let out = tmp.join(format!("{name}-{version}-x86_64.jpkg"));
        archive::create(&out, &meta_toml, &destdir).unwrap();
        out
    }

    /// Build a .jpkg with a custom post_install hook.
    pub fn build_test_jpkg_with_hook(tmp: &Path, name: &str, version: &str, post_install: &str) -> PathBuf {
        let destdir = tmp.join(format!("destdir-hook-{name}"));
        fs::create_dir_all(destdir.join("bin")).unwrap();
        fs::write(destdir.join("bin/foo"), b"hook content\n").unwrap();

        let meta = Metadata {
            package: PackageSection {
                name: Some(name.to_string()),
                version: Some(version.to_string()),
                license: Some("MIT".to_string()),
                description: Some("hook test".to_string()),
                arch: Some("x86_64".to_string()),
                replaces: vec![],
                conflicts: vec![],
            },
            depends: DependsSection::default(),
            hooks: HooksSection {
                post_install: Some(post_install.to_string()),
                ..Default::default()
            },
            files: FilesSection::default(),
            signature: None,
        };
        let meta_toml = meta.to_string().unwrap();
        let out = tmp.join(format!("{name}-{version}-x86_64.jpkg"));
        archive::create(&out, &meta_toml, &destdir).unwrap();
        out
    }

    /// Build a .jpkg with `replaces = [replaced_name]` that owns bin/sh.
    pub fn build_jpkg_with_replaces(
        tmp: &Path,
        name: &str,
        version: &str,
        replaces: Vec<String>,
    ) -> PathBuf {
        let destdir = tmp.join(format!("destdir-replaces-{name}"));
        fs::create_dir_all(destdir.join("bin")).unwrap();
        fs::write(destdir.join("bin/sh"), b"#!/bin/sh\n").unwrap();

        let meta = Metadata {
            package: PackageSection {
                name: Some(name.to_string()),
                version: Some(version.to_string()),
                license: Some("MIT".to_string()),
                description: Some("replaces test".to_string()),
                arch: Some("x86_64".to_string()),
                replaces,
                conflicts: vec![],
            },
            depends: DependsSection::default(),
            hooks: HooksSection::default(),
            files: FilesSection::default(),
            signature: None,
        };
        let meta_toml = meta.to_string().unwrap();
        let out = tmp.join(format!("{name}-{version}-x86_64.jpkg"));
        archive::create(&out, &meta_toml, &destdir).unwrap();
        out
    }

    // ── 1. build_manifest ─────────────────────────────────────────────────────

    #[test]
    fn test_build_manifest_walks_tree() {
        let tmp = TempDir::new().unwrap();
        let tree = tmp.path().join("tree");
        fs::create_dir_all(tree.join("bin")).unwrap();
        fs::create_dir_all(tree.join("lib")).unwrap();
        fs::write(tree.join("bin/foo"), b"hello").unwrap();
        fs::write(tree.join("lib/bar"), b"world").unwrap();
        symlink("bar", tree.join("lib/baz")).unwrap();

        let entries = build_manifest(&tree).unwrap();

        let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"bin"), "dir bin should appear");
        assert!(paths.contains(&"bin/foo"), "bin/foo should appear");
        assert!(paths.contains(&"lib/bar"), "lib/bar should appear");
        assert!(paths.contains(&"lib/baz"), "lib/baz symlink should appear");

        let baz = entries.iter().find(|e| e.path == "lib/baz").unwrap();
        assert_eq!(baz.symlink_target.as_deref(), Some("bar"));
        assert!(baz.sha256.is_empty(), "symlink sha256 should be empty in FileEntry");
    }

    // ── 2. flatten_merged_usr ─────────────────────────────────────────────────

    #[test]
    fn test_flatten_merged_usr_moves_usr() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("d");
        fs::create_dir_all(destdir.join("usr/bin")).unwrap();
        fs::write(destdir.join("usr/bin/hello"), b"hello").unwrap();

        flatten_merged_usr(&destdir).unwrap();

        assert!(destdir.join("bin/hello").exists(), "bin/hello should exist after flatten");
        assert!(!destdir.join("usr").exists(), "usr/ should be gone");
    }

    #[test]
    fn test_flatten_merged_usr_moves_lib64() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("d");
        fs::create_dir_all(destdir.join("lib64")).unwrap();
        fs::create_dir_all(destdir.join("lib")).unwrap();
        fs::write(destdir.join("lib64/ld.so"), b"elf stub").unwrap();

        flatten_merged_usr(&destdir).unwrap();

        assert!(destdir.join("lib/ld.so").exists(), "lib/ld.so should exist after flatten");
        assert!(!destdir.join("lib64").exists(), "lib64/ should be gone");
    }

    #[test]
    fn test_flatten_merged_usr_symlink_usr_left_alone() {
        // If usr/ is a symlink (already merged), do nothing.
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("d");
        fs::create_dir_all(&destdir).unwrap();
        symlink(".", destdir.join("usr")).unwrap();

        // Should not error, and the symlink should still be there.
        flatten_merged_usr(&destdir).unwrap();
        assert!(destdir.join("usr").symlink_metadata().unwrap().file_type().is_symlink());
    }

    // ── 3. extract_and_register ───────────────────────────────────────────────

    #[test]
    fn test_extract_and_register_basic() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let jpkg_path = build_test_jpkg(tmp.path(), "mypkg", "1.0.0");
        let archive = JpkgArchive::open(&jpkg_path).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        let _pkg = extract_and_register(&archive, &rootfs, &db).unwrap();

        // Verify on-disk.
        assert!(rootfs.join("bin/foo").exists(), "bin/foo should be installed");
        assert!(rootfs.join("lib/bar").exists(), "lib/bar should be installed");

        // Verify DB record.
        let got = db.get("mypkg").unwrap().expect("mypkg should be in db");
        assert_eq!(got.metadata.package.name.as_deref(), Some("mypkg"));
        assert!(!got.files.is_empty(), "files manifest should be non-empty");
    }

    // ── 4. extract_and_register + replaces ────────────────────────────────────

    #[test]
    fn test_extract_and_register_replaces_transfers_ownership() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // Install pkg A — it owns bin/sh.
        let a_jpkg = build_jpkg_with_replaces(tmp.path(), "pkgA", "1.0.0", vec![]);
        let a_arc = JpkgArchive::open(&a_jpkg).unwrap();
        extract_and_register(&a_arc, &rootfs, &db).unwrap();

        let a_before = db.get("pkgA").unwrap().unwrap();
        assert!(a_before.files.iter().any(|e| e.path == "bin/sh"), "A should own bin/sh");

        // Install pkg B — it replaces A and installs its own bin/sh.
        let b_jpkg = build_jpkg_with_replaces(tmp.path(), "pkgB", "1.0.0", vec!["pkgA".to_string()]);
        let b_arc = JpkgArchive::open(&b_jpkg).unwrap();
        extract_and_register(&b_arc, &rootfs, &db).unwrap();

        // A's manifest should no longer list bin/sh.
        let a_after = db.get("pkgA").unwrap().unwrap();
        assert!(
            !a_after.files.iter().any(|e| e.path == "bin/sh"),
            "A should not own bin/sh after B replaces it"
        );

        // B's manifest should list bin/sh.
        let b = db.get("pkgB").unwrap().unwrap();
        assert!(b.files.iter().any(|e| e.path == "bin/sh"), "B should own bin/sh");
    }

    // ── 5. run_hook (non-root / host path) ────────────────────────────────────

    #[test]
    fn test_run_hook_empty_succeeds() {
        let tmp = TempDir::new().unwrap();
        let status = run_hook(tmp.path(), "").unwrap();
        assert!(status.success());
    }

    #[test]
    fn test_run_hook_creates_marker_file() {
        let tmp = TempDir::new().unwrap();
        let marker = tmp.path().join("marker");
        // Hook uses JPKG_ROOT to place the marker inside our temp rootfs.
        let hook = format!(
            "touch \"$JPKG_ROOT/marker\"",
        );
        let status = run_hook(tmp.path(), &hook).unwrap();
        assert!(status.success(), "hook should exit 0");
        assert!(marker.exists(), "hook should have created the marker file");
    }

    #[test]
    fn test_run_hook_nonzero_exit_preserved() {
        let tmp = TempDir::new().unwrap();
        let status = run_hook(tmp.path(), "exit 42").unwrap();
        assert!(!status.success());
        // status.code() returns the exit code from the shell.
        // In the non-root host path we get it directly.
        assert_eq!(status.code().unwrap_or(-1), 42);
    }
}
