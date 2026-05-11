// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

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
    /// I/O error tied to a specific filesystem path.
    ///
    /// Used wherever the bare `io::Error` would lose the path that triggered
    /// it — e.g. `symlinkat` returning `EEXIST` with no path attached.  See
    /// `install_files` for the canonical use site.
    FileOp { path: PathBuf, op: &'static str, source: io::Error },
    /// Upgrade-clean discovered a foreign file under a directory the new
    /// package wants to replace with a symlink.  The user must remove the
    /// foreign file by hand (or pass an as-yet-unwritten escape hatch); we
    /// refuse to nuke unowned data.
    UpgradeForeignFiles {
        pkg: String,
        new_version: String,
        dir: PathBuf,
        foreign: Vec<PathBuf>,
    },
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
            InstallError::FileOp { path, op, source } => {
                write!(f, "cannot {op} at {}: {source}", path.display())
            }
            InstallError::UpgradeForeignFiles { pkg, new_version, dir, foreign } => {
                // List up to 5 paths verbatim; truncate the rest.
                let shown: Vec<String> = foreign
                    .iter()
                    .take(5)
                    .map(|p| p.display().to_string())
                    .collect();
                let more = foreign.len().saturating_sub(shown.len());
                write!(
                    f,
                    "cannot install {pkg}-{new_version}: target path {} \
                     conflicts with existing directory (still populated \
                     after old-manifest cleanup); foreign files present: {}",
                    dir.display(),
                    if more > 0 {
                        format!("{} (+{} more)", shown.join(", "), more)
                    } else {
                        shown.join(", ")
                    }
                )
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
            InstallError::FileOp { source, .. } => Some(source),
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

        // SAFETY: every `abs` came from `WalkDir::new(tree_root)` with the root
        // itself skipped, so `strip_prefix(tree_root)` always succeeds.  The
        // expect is unreachable in production.
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
        // SAFETY: every `abs` came from `WalkDir::new(&src)`, so
        // `strip_prefix(&src)` always succeeds.  The expect is unreachable.
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

/// Wrap a path-bound `io::Result` into an `InstallError::FileOp` so the
/// final error message tells the user which path tripped over which syscall.
///
/// Without this wrapper, the bare `io::Error` from `symlinkat(2)` is something
/// like `File exists (os error 17)` with no file path — useless triage info.
/// Every callsite in `install_files` and `clean_old_files_for_upgrade` that
/// touches the filesystem goes through this helper.
fn wrap_io<T>(
    result: io::Result<T>,
    path: &Path,
    op: &'static str,
) -> Result<T, InstallError> {
    result.map_err(|e| InstallError::FileOp {
        path: path.to_path_buf(),
        op,
        source: e,
    })
}

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
///
/// # Errors
///
/// Returns [`InstallError::FileOp`] with the offending path on any filesystem
/// failure.  The 2.2.2-and-earlier behaviour of returning a bare `io::Error`
/// with no path attached made triage of upgrade-time collisions impossible
/// (see the 2.2.3 changelog: `symlinkat ... = -1 EEXIST` with no path).
fn install_files(stage_dir: &Path, rootfs: &Path) -> Result<(), InstallError> {
    for entry in WalkDir::new(stage_dir).sort_by_file_name().into_iter() {
        let entry = entry.map_err(|e| InstallError::Io(io::Error::new(
            io::ErrorKind::Other,
            format!("walkdir: {e}"),
        )))?;
        let abs = entry.path();

        if abs == stage_dir {
            continue; // skip root
        }

        let rel = abs
            .strip_prefix(stage_dir)
            .expect("walkdir child of stage_dir");
        let dest = rootfs.join(rel);

        let m = wrap_io(abs.symlink_metadata(), abs, "stat staged file")?;

        if m.file_type().is_symlink() {
            let target = wrap_io(fs::read_link(abs), abs, "read staged symlink")?;
            // Remove any existing dest (symlink, file, or empty dir) before
            // recreating — matches tar -x "replace symlinks, not follow"
            // behaviour.  If the existing dest is a populated directory the
            // upgrade-clean step in extract_and_register should have already
            // emptied it; if it didn't, fall through and let the symlink call
            // surface the EEXIST with its path so the user can see exactly
            // what got in the way.
            if let Ok(dm) = dest.symlink_metadata() {
                if dm.is_dir() && !dm.file_type().is_symlink() {
                    // Try a plain rmdir first (empty dir case).  If that
                    // fails (ENOTEMPTY) leave the directory in place — the
                    // subsequent symlink() will fail with EEXIST and surface
                    // dest as the conflicting path.  We deliberately do NOT
                    // remove_dir_all here unconditionally; the caller is
                    // responsible for proving the directory is owned by the
                    // old version (see clean_old_files_for_upgrade).
                    let _ = fs::remove_dir(&dest);
                } else {
                    wrap_io(fs::remove_file(&dest), &dest, "remove existing file")?;
                }
            }
            if let Some(p) = dest.parent() {
                wrap_io(fs::create_dir_all(p), p, "create parent directory")?;
            }
            wrap_io(
                std::os::unix::fs::symlink(&target, &dest),
                &dest,
                "create symlink",
            )?;
        } else if m.is_dir() {
            wrap_io(fs::create_dir_all(&dest), &dest, "create directory")?;
        } else {
            // Regular file — remove any existing symlink/file at dest first so
            // we do not inadvertently write through a symlink.  Tolerate
            // missing files (first install) and existing-as-symlink (upgrade
            // from symlink → regular file).
            if let Ok(dm) = dest.symlink_metadata() {
                if dm.is_dir() && !dm.file_type().is_symlink() {
                    // Populated dir where a file should go — same constraint
                    // as the symlink branch: caller must have cleaned it.
                    let _ = fs::remove_dir(&dest);
                } else {
                    wrap_io(fs::remove_file(&dest), &dest, "remove existing file")?;
                }
            }
            if let Some(p) = dest.parent() {
                wrap_io(fs::create_dir_all(p), p, "create parent directory")?;
            }
            wrap_io(fs::copy(abs, &dest), &dest, "copy file")?;
        }
    }
    Ok(())
}

// ─── upgrade-clean ────────────────────────────────────────────────────────────

/// Remove old-manifest files that are no longer in the new manifest, and
/// resolve dir→symlink (or symlink→dir) collisions for the same package
/// being upgraded.
///
/// This is what every real package manager does on upgrade (apt's dpkg
/// backend, rpm, pacman, …): the manifest of the previous version tells you
/// which files YOU put on disk, so you know which files YOU are allowed to
/// take back.  Anything you find under a path you used to own that isn't in
/// the old manifest is unowned data — the user's, or another package's — and
/// must not be silently destroyed.
///
/// # Algorithm
///
/// 1. Read the OLD installed pkg from the db (caller guarantees same name).
/// 2. Build a set of paths in the NEW manifest (`new_paths`).
/// 3. For each entry in `old_files - new_files`, delete it from rootfs:
///    - Regular file or symlink → `remove_file`
///    - Directory → defer (we collect them and rmdir at the end in reverse
///      sorted order so leaves come before parents).
/// 4. For each path that is a populated directory in old_files but a SYMLINK
///    in the new manifest (`dir_to_symlink`):
///    - Walk the on-disk directory.
///    - Every file underneath must be in the OLD manifest (i.e. owned by us).
///    - If any foreign file is found, return [`InstallError::UpgradeForeignFiles`].
///    - Otherwise `remove_dir_all` it.
///
/// The reverse for symlink→dir is handled by `install_files`' `remove_file`
/// branch (symlinks always rm cheaply, no contents to worry about).
///
/// # Why we tolerate failures at this stage
///
/// Old-manifest entries that are already gone (because a previous failed
/// upgrade half-cleaned, or an admin deleted them) must not abort the new
/// install.  ENOENT is silently tolerated; any other error is propagated.
fn clean_old_files_for_upgrade(
    rootfs: &Path,
    old_pkg: &InstalledPkg,
    new_files: &[FileEntry],
    new_pkg_name: &str,
    new_pkg_version: &str,
) -> Result<(), InstallError> {
    use std::collections::HashMap;

    // Build a lookup of new manifest entries keyed by path.
    let new_by_path: HashMap<&str, &FileEntry> =
        new_files.iter().map(|e| (e.path.as_str(), e)).collect();

    // Collect dir paths to rmdir at the very end, leaves first.
    let mut stale_dirs: Vec<PathBuf> = Vec::new();

    // First pass — handle dir→symlink transitions before bulk file removal.
    // We need to validate ownership of dir contents BEFORE we start removing
    // anything, so a failed validation leaves the system untouched (atomicity:
    // either we succeed and the dir is gone, or we error and on-disk state
    // matches the pre-upgrade db).
    let mut dirs_to_blast: Vec<PathBuf> = Vec::new();

    // Build a set of file paths (no-leading-slash, relative to rootfs)
    // that the OLD package owned, for the foreign-file check below.
    let old_owned: std::collections::HashSet<&str> =
        old_pkg.files.iter().map(|e| e.path.as_str()).collect();

    for old in &old_pkg.files {
        let new_entry = new_by_path.get(old.path.as_str()).copied();
        let is_dir_in_old = old.is_dir;
        let is_symlink_in_new = new_entry
            .map(|e| e.symlink_target.is_some())
            .unwrap_or(false);
        if is_dir_in_old && is_symlink_in_new {
            // Verify the on-disk dir is entirely owned by the old package.
            let on_disk = rootfs.join(&old.path);
            // Use symlink_metadata to avoid following a stray symlink at the
            // ownership-check target (defence in depth — the old manifest
            // says it's a dir, but if reality differs we just bail).
            let md = match on_disk.symlink_metadata() {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    // Old dir already gone (admin cleanup or prior failed
                    // upgrade) — nothing to do.
                    continue;
                }
                Err(e) => {
                    return Err(InstallError::FileOp {
                        path: on_disk,
                        op: "stat old directory",
                        source: e,
                    });
                }
            };
            if !md.is_dir() || md.file_type().is_symlink() {
                // Manifest says dir but reality says something else — leave
                // it alone and let install_files surface the collision.
                continue;
            }
            // Walk the on-disk dir, checking every contained path is in
            // old_owned (with the rootfs-relative form).
            let mut foreign: Vec<PathBuf> = Vec::new();
            for entry in WalkDir::new(&on_disk).into_iter() {
                let entry = entry.map_err(|e| InstallError::Io(io::Error::new(
                    io::ErrorKind::Other,
                    format!("walkdir while scanning {}: {e}", on_disk.display()),
                )))?;
                let abs = entry.path();
                if abs == on_disk {
                    continue;
                }
                // Compute the rootfs-relative key for old_owned lookup.
                let rel = match abs.strip_prefix(rootfs) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let rel_str = rel.to_string_lossy();
                // If the entry isn't in the old manifest, it's foreign.
                // is_dir entries get skipped: directories on disk that
                // happen to be unowned (e.g. an empty subdir made by the
                // user) shouldn't be flagged — only their CONTENTS matter
                // for the "is it safe to nuke?" question.  But a foreign
                // FILE inside a foreign DIR still surfaces correctly
                // because the file's path itself isn't in old_owned.
                let on_disk_md = match abs.symlink_metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !on_disk_md.is_dir() || on_disk_md.file_type().is_symlink() {
                    if !old_owned.contains(rel_str.as_ref()) {
                        foreign.push(abs.to_path_buf());
                    }
                }
            }
            if !foreign.is_empty() {
                foreign.sort();
                return Err(InstallError::UpgradeForeignFiles {
                    pkg: new_pkg_name.to_string(),
                    new_version: new_pkg_version.to_string(),
                    dir: on_disk,
                    foreign,
                });
            }
            dirs_to_blast.push(on_disk);
        }
    }

    // Second pass — bulk-delete old files that aren't in the new manifest.
    // (Same-path-different-kind transitions get handled in install_files,
    // EXCEPT for the populated-dir-to-symlink case, which we just validated
    // above and are about to blast.)
    for old in &old_pkg.files {
        if new_by_path.contains_key(old.path.as_str()) {
            // Path still owned in new manifest — install_files will overwrite
            // appropriately.  Don't remove it here or there'd be a window
            // where the rootfs is missing the file.
            continue;
        }
        let on_disk = rootfs.join(&old.path);
        if old.is_dir {
            // Defer; remove after all child files are unlinked.
            stale_dirs.push(on_disk);
            continue;
        }
        // Regular file or symlink.  Tolerate ENOENT (admin cleanup, prior
        // failed upgrade).  symlink_metadata to avoid following symlinks.
        match fs::symlink_metadata(&on_disk) {
            Ok(_) => {
                wrap_io(
                    fs::remove_file(&on_disk),
                    &on_disk,
                    "remove old-manifest file",
                )?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => { /* already gone */ }
            Err(e) => {
                return Err(InstallError::FileOp {
                    path: on_disk,
                    op: "stat old-manifest file",
                    source: e,
                });
            }
        }
    }

    // Blast the validated dir→symlink directories.
    for dir in &dirs_to_blast {
        wrap_io(
            fs::remove_dir_all(dir),
            dir,
            "remove old-manifest directory contents",
        )?;
    }

    // Sort stale dirs longest-first so we rmdir leaves before parents.
    stale_dirs.sort_by(|a, b| b.as_os_str().len().cmp(&a.as_os_str().len()));
    for d in &stale_dirs {
        // rmdir, not remove_dir_all — only succeed if empty, otherwise
        // leave it.  Matches `db.remove` semantics in db.rs (line 512).
        // ENOENT and ENOTEMPTY are both silently ignored (same as the C
        // jpkg-1.1.5 behaviour).
        match fs::remove_dir(d) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => { /* ENOTEMPTY etc. — leave it */ }
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
/// 5. Build the NEW file manifest from the staged tree (needed before step 6).
/// 6. Upgrade-clean: if `db.get(name)?` returns Some(old), remove old-manifest
///    files that aren't in the new manifest, and resolve dir→symlink layout
///    flips owned by the same package.
/// 7. `install_files(stage, rootfs)` — copy to final destination.
/// 8. `db.insert(InstalledPkg { metadata, files })`.
/// 9. For each name in `metadata.package.replaces`:
///    `db.transfer_ownership(replaced, pkg_name, &shared_paths)`.
/// 10. Clean up the staging directory.
///
/// # 2.2.3 — upgrade-clean
///
/// Steps 5 & 6 are new in 2.2.3.  Without them, replacing `/lib/terminfo`
/// (populated dir, owned by ncurses-r3) with a symlink (`lib/terminfo ->
/// ../share/terminfo` in ncurses-r4) failed at step 7 with `EEXIST` because
/// install_files' `remove_dir` only succeeds on empty dirs.  Now we read
/// old_pkg's manifest, verify every file under the doomed dir is owned by
/// the same package being upgraded, and only then `remove_dir_all` it.  If
/// foreign files are found, we surface them in the error message and refuse
/// to nuke user data.
///
/// Divergences from C:
/// - We build the manifest from the staging dir, not from the rootfs, so
///   paths are relative without the rootfs prefix — matching db.c's format.
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
    // Use metadata_str() rather than metadata() so a corrupt archive with
    // non-UTF-8 metadata bytes returns an error instead of panicking.
    let metadata = Metadata::from_str(archive.metadata_str()?)?;
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
        .unwrap_or("(unversioned)")
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

    // ── 5. Build the new manifest now (before install) so upgrade-clean can
    //       diff it against the old manifest.  Built from the staging dir so
    //       sha256s etc. are computed exactly once.
    let files = build_manifest(&stage_dir)?;

    // ── 6. Upgrade-clean — only when an older version of this package is
    //       already in the db.  We do this BEFORE install_files so the
    //       populated-dir-to-symlink case (the ncurses bug) doesn't trip the
    //       symlink call's `EEXIST`.
    if let Some(old_pkg) = db.get(&pkg_name)? {
        log::debug!(
            "jpkg: upgrade-clean: {} {} -> {}",
            pkg_name,
            old_pkg.metadata.package.version.as_deref().unwrap_or("?"),
            pkg_version,
        );
        clean_old_files_for_upgrade(
            rootfs,
            &old_pkg,
            &files,
            &pkg_name,
            &pkg_version,
        )?;
    }

    // ── 7. Install files into rootfs ──────────────────────────────────────
    install_files(&stage_dir, rootfs)?;

    // ── 8. Register in DB ─────────────────────────────────────────────────
    let pkg = InstalledPkg {
        metadata: metadata.clone(),
        files: files.clone(),
    };
    db.insert(&pkg)?;

    // ── 9. Transfer ownership for replaces = [...] ───────────────────────
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

    // ── 10. Cleanup ───────────────────────────────────────────────────────
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

    /// Build a .jpkg that ships `lib/terminfo/` as a populated DIRECTORY
    /// (mirrors ncurses-6.5-r3's layout pre-bug).  Two files live under it:
    /// `lib/terminfo/a` and `lib/terminfo/b`.  Used as the v1 in the
    /// dir→symlink upgrade test.
    pub fn build_jpkg_with_populated_terminfo_dir(
        tmp: &Path,
        name: &str,
        version: &str,
    ) -> PathBuf {
        let destdir = tmp.join(format!("destdir-popdir-{name}-{version}"));
        fs::create_dir_all(destdir.join("lib/terminfo")).unwrap();
        fs::write(destdir.join("lib/terminfo/a"), b"a entry\n").unwrap();
        fs::write(destdir.join("lib/terminfo/b"), b"b entry\n").unwrap();

        let meta = Metadata {
            package: PackageSection {
                name: Some(name.to_string()),
                version: Some(version.to_string()),
                license: Some("MIT".to_string()),
                description: Some("populated-terminfo test".to_string()),
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

    /// Build a .jpkg that ships `lib/terminfo` as a SYMLINK to
    /// `../share/terminfo`, with the real terminfo content under
    /// `share/terminfo/`.  Mirrors ncurses-6.5-r4's intended layout.
    /// This is the v2 in the dir→symlink upgrade test.
    pub fn build_jpkg_with_terminfo_symlink(
        tmp: &Path,
        name: &str,
        version: &str,
    ) -> PathBuf {
        let destdir = tmp.join(format!("destdir-sym-{name}-{version}"));
        fs::create_dir_all(destdir.join("share/terminfo")).unwrap();
        fs::write(destdir.join("share/terminfo/a"), b"a entry\n").unwrap();
        fs::write(destdir.join("share/terminfo/b"), b"b entry\n").unwrap();
        fs::create_dir_all(destdir.join("lib")).unwrap();
        symlink("../share/terminfo", destdir.join("lib/terminfo")).unwrap();

        let meta = Metadata {
            package: PackageSection {
                name: Some(name.to_string()),
                version: Some(version.to_string()),
                license: Some("MIT".to_string()),
                description: Some("terminfo-symlink test".to_string()),
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

    /// Build a .jpkg with a single file at a custom path.  Used for the
    /// "old file gone in new manifest" test (v1 has lib/extra, v2 doesn't).
    pub fn build_jpkg_with_extra_file(
        tmp: &Path,
        name: &str,
        version: &str,
        extra_rel: &str,
    ) -> PathBuf {
        let destdir = tmp.join(format!("destdir-extra-{name}-{version}"));
        fs::create_dir_all(destdir.join("bin")).unwrap();
        fs::write(destdir.join("bin/foo"), b"foo content\n").unwrap();
        if let Some(p) = std::path::Path::new(extra_rel).parent() {
            fs::create_dir_all(destdir.join(p)).unwrap();
        }
        fs::write(destdir.join(extra_rel), b"extra content\n").unwrap();

        let meta = Metadata {
            package: PackageSection {
                name: Some(name.to_string()),
                version: Some(version.to_string()),
                license: Some("MIT".to_string()),
                description: Some("extra-file test".to_string()),
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

    // ── 6. Upgrade-clean: the user's exact ncurses bug ────────────────────────
    //
    // ncurses-6.5-r3 ships /lib/terminfo as a populated DIRECTORY (with
    // terminfo entries inside it).  ncurses-6.5-r4 wants to make it a
    // SYMLINK to ../share/terminfo.  Before 2.2.3, install_files'
    // `fs::remove_dir` silently failed with ENOTEMPTY and the subsequent
    // symlink call returned EEXIST.  Now we read the old manifest, verify
    // every file under /lib/terminfo is owned by ncurses, and remove_dir_all
    // it before extracting v2.

    #[test]
    fn upgrade_replacing_populated_dir_with_symlink_succeeds() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // v1: lib/terminfo as a populated directory.
        let v1 = build_jpkg_with_populated_terminfo_dir(tmp.path(), "ncurses", "6.5-r3");
        let arc_v1 = JpkgArchive::open(&v1).unwrap();
        extract_and_register(&arc_v1, &rootfs, &db).unwrap();

        // Sanity: dir + entries are there.
        let lib_terminfo = rootfs.join("lib/terminfo");
        assert!(lib_terminfo.is_dir(), "v1 should install lib/terminfo as a directory");
        assert!(lib_terminfo.join("a").exists(), "v1 should install lib/terminfo/a");
        assert!(lib_terminfo.join("b").exists(), "v1 should install lib/terminfo/b");

        // v2: lib/terminfo as a symlink to ../share/terminfo.
        let v2 = build_jpkg_with_terminfo_symlink(tmp.path(), "ncurses", "6.5-r4");
        let arc_v2 = JpkgArchive::open(&v2).unwrap();
        extract_and_register(&arc_v2, &rootfs, &db).expect("v2 upgrade should succeed");

        // /lib/terminfo must now be a symlink.
        let md = lib_terminfo.symlink_metadata().unwrap();
        assert!(
            md.file_type().is_symlink(),
            "lib/terminfo should be a symlink after upgrade, got {:?}",
            md.file_type()
        );
        let target = fs::read_link(&lib_terminfo).unwrap();
        assert_eq!(
            target.to_string_lossy(),
            "../share/terminfo",
            "lib/terminfo should point to ../share/terminfo"
        );

        // DB should reflect v2.
        let after = db.get("ncurses").unwrap().unwrap();
        assert_eq!(after.metadata.package.version.as_deref(), Some("6.5-r4"));
    }

    // ── 7. Upgrade-clean: old files not in new manifest get removed ───────────

    #[test]
    fn upgrade_drops_old_files_not_in_new_manifest() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // v1: bin/foo + share/dropme.
        let v1 = build_jpkg_with_extra_file(tmp.path(), "droptest", "1.0.0", "share/dropme");
        let arc_v1 = JpkgArchive::open(&v1).unwrap();
        extract_and_register(&arc_v1, &rootfs, &db).unwrap();

        assert!(rootfs.join("share/dropme").exists(), "v1 should install share/dropme");
        assert!(rootfs.join("bin/foo").exists(), "v1 should install bin/foo");

        // v2: bin/foo + lib/bar (build_test_jpkg's layout, no share/dropme).
        let v2 = build_test_jpkg(tmp.path(), "droptest", "2.0.0");
        let arc_v2 = JpkgArchive::open(&v2).unwrap();
        extract_and_register(&arc_v2, &rootfs, &db).unwrap();

        // share/dropme should be gone.
        assert!(
            !rootfs.join("share/dropme").exists(),
            "share/dropme should be removed during upgrade (not in v2's manifest)"
        );
        // bin/foo should still be there (in both manifests).
        assert!(rootfs.join("bin/foo").exists(), "bin/foo should survive upgrade");
        // lib/bar (new in v2) should appear.
        assert!(rootfs.join("lib/bar").exists(), "lib/bar should be installed by v2");

        // DB reflects v2.
        let after = db.get("droptest").unwrap().unwrap();
        assert_eq!(after.metadata.package.version.as_deref(), Some("2.0.0"));
        assert!(
            !after.files.iter().any(|e| e.path == "share/dropme"),
            "share/dropme should not be in v2's manifest"
        );
    }

    // ── 8. Upgrade-clean: foreign file in dir to be replaced by symlink ───────

    #[test]
    fn upgrade_refuses_to_blast_foreign_files_in_dir_being_replaced_by_symlink() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // v1: populated lib/terminfo directory owned by ncurses.
        let v1 = build_jpkg_with_populated_terminfo_dir(tmp.path(), "ncurses", "6.5-r3");
        let arc_v1 = JpkgArchive::open(&v1).unwrap();
        extract_and_register(&arc_v1, &rootfs, &db).unwrap();

        // User drops a foreign file into lib/terminfo by hand.
        let foreign = rootfs.join("lib/terminfo/user-dropped-file");
        fs::write(&foreign, b"user data\n").unwrap();

        // v2 wants lib/terminfo as a symlink — should error out cleanly.
        let v2 = build_jpkg_with_terminfo_symlink(tmp.path(), "ncurses", "6.5-r4");
        let arc_v2 = JpkgArchive::open(&v2).unwrap();
        let err = extract_and_register(&arc_v2, &rootfs, &db).expect_err(
            "upgrade should fail when foreign files are present in dir being replaced",
        );

        // Error must name the foreign file.
        let msg = err.to_string();
        assert!(
            msg.contains("user-dropped-file"),
            "error should mention the foreign file path, got: {msg}"
        );
        assert!(
            msg.contains("ncurses-6.5-r4"),
            "error should mention the new package version, got: {msg}"
        );
        assert!(
            matches!(err, InstallError::UpgradeForeignFiles { .. }),
            "error variant should be UpgradeForeignFiles, got: {err:?}"
        );

        // DB unchanged: still pinned at v1.
        let still = db.get("ncurses").unwrap().unwrap();
        assert_eq!(still.metadata.package.version.as_deref(), Some("6.5-r3"));

        // The foreign file must still be on disk — we refused to blast it.
        assert!(foreign.exists(), "foreign file should not have been touched");
    }

    // ── 9. Path-aware error wrapping: io::Error now carries the path ──────────

    #[test]
    fn install_error_message_includes_path() {
        // Construct an install_files scenario that fails: pre-place an empty
        // directory at a path the staged tree wants to make a symlink to,
        // but ALSO drop a foreign file inside.  install_files only does a
        // plain remove_dir (not remove_dir_all), so the rmdir silently fails
        // on ENOTEMPTY and the subsequent symlink call returns EEXIST — the
        // ncurses bug reproduction at the install_files layer.  The wrapper
        // must convert that into an InstallError::FileOp carrying the path.
        let tmp = TempDir::new().unwrap();
        let stage = tmp.path().join("stage");
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(stage.join("lib")).unwrap();
        fs::create_dir_all(&rootfs).unwrap();
        // Staged: lib/symlink -> ../share/target  (the new package's intent).
        symlink("../share/target", stage.join("lib/symlink")).unwrap();

        // Pre-place a populated directory at the dest path so install_files'
        // bare remove_dir fails ENOTEMPTY and the symlink call hits EEXIST.
        let collision = rootfs.join("lib/symlink");
        fs::create_dir_all(&collision).unwrap();
        fs::write(collision.join("squatter"), b"x").unwrap();

        let err = install_files(&stage, &rootfs).expect_err(
            "install_files should fail when target dir is populated",
        );

        match &err {
            InstallError::FileOp { path, op, source } => {
                assert_eq!(
                    path, &collision,
                    "FileOp.path should be the colliding dest path"
                );
                assert!(
                    op == &"create symlink" || op == &"create directory",
                    "op should describe the failing operation, got {op:?}"
                );
                // The underlying error should be EEXIST-like.
                let _ = source; // any io::Error is fine
            }
            other => panic!("expected FileOp, got {other:?}"),
        }
        let msg = err.to_string();
        assert!(
            msg.contains(&*collision.to_string_lossy()),
            "error message should include the conflicting path, got: {msg}"
        );
    }

    // ── 10. Reinstall with identical manifest is a no-op for upgrade-clean ────

    #[test]
    fn upgrade_clean_reinstall_same_version_is_safe() {
        // Regression guard: `jpkg install --force ncurses` of the SAME
        // version should still work; upgrade-clean must not delete files
        // that are in both manifests.
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        let v1 = build_test_jpkg(tmp.path(), "samepkg", "1.0.0");
        let arc1 = JpkgArchive::open(&v1).unwrap();
        extract_and_register(&arc1, &rootfs, &db).unwrap();

        // Reinstall the EXACT same jpkg (simulates --force --reinstall).
        let arc2 = JpkgArchive::open(&v1).unwrap();
        extract_and_register(&arc2, &rootfs, &db).expect("reinstall should succeed");

        assert!(rootfs.join("bin/foo").exists(), "bin/foo should still exist");
        assert!(rootfs.join("lib/bar").exists(), "lib/bar should still exist");
    }
}
