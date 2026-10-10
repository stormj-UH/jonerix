// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use walkdir::WalkDir;

use crate::archive::{ArchiveError, JpkgArchive};
use crate::config::{OnDisk, Recorded};
use crate::db::{DbError, FileEntry, InstalledDb, InstalledPkg, Ownership};
use crate::recipe::{Metadata, RecipeError};
use crate::util::sha256_file;

// ─── InstallError ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum InstallError {
    Io(io::Error),
    Archive(ArchiveError),
    Db(DbError),
    Recipe(RecipeError),
    HookFailed {
        hook: &'static str,
        status: i32,
    },
    Conflict {
        path: String,
        owned_by: String,
    },
    /// Package has no signature but `signature_policy = require`.
    SignatureMissing {
        name: String,
        version: String,
    },
    /// Package carries a signature but it did not verify.
    SignatureInvalid {
        name: String,
        version: String,
        reason: String,
    },
    /// Package carries a signature referencing an unknown key.
    UnknownSigningKey {
        name: String,
        key_id: String,
    },
    /// I/O error tied to a specific filesystem path.
    ///
    /// Used wherever the bare `io::Error` would lose the path that triggered
    /// it — e.g. `symlinkat` returning `EEXIST` with no path attached.  See
    /// `install_files` for the canonical use site.
    FileOp {
        path: PathBuf,
        op: &'static str,
        source: io::Error,
    },
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
            InstallError::SignatureInvalid {
                name,
                version,
                reason,
            } => {
                write!(f, "signature invalid for {name}-{version}: {reason}")
            }
            InstallError::UnknownSigningKey { name, key_id } => {
                write!(f, "unknown signing key {key_id} for package {name}")
            }
            InstallError::FileOp { path, op, source } => {
                write!(f, "cannot {op} at {}: {source}", path.display())
            }
            InstallError::UpgradeForeignFiles {
                pkg,
                new_version,
                dir,
                foreign,
            } => {
                // List up to 5 paths verbatim; truncate the rest.
                let shown: Vec<String> = foreign
                    .iter()
                    .take(5)
                    .map(|p| p.display().to_string())
                    .collect();
                let more = foreign.len().saturating_sub(shown.len());
                write!(
                    f,
                    "cannot install {pkg}-{new_version}: it turns {} from a \
                     directory into a symlink or back, and that directory \
                     holds files that are not this package's unchanged ones; \
                     move them out of the way (or restore the packaged copies) \
                     and retry: {}",
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
        let entry = entry
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("walkdir error: {e}")))?;

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
            let target = fs::read_link(abs)?.to_string_lossy().into_owned();
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
        // JPKG_CONFFILES=1: this jpkg protects config files (2.2.11); a hook
        // can test it (see docs/packaging.md before relying on it to drop a
        // carry).  chroot inherits it.
        Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .env("JPKG_CONFFILES", "1")
            .status()
    } else {
        // Non-root or no /bin/sh in rootfs yet — run on host with env vars.
        // Mirrors cmd_install.c:107-122.
        Command::new("/bin/sh")
            .arg("-c")
            .arg(hook_body)
            .env("JPKG_ROOT", rootfs_str.as_ref())
            .env("DESTDIR", rootfs_str.as_ref())
            .env("JPKG_CONFFILES", "1")
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
        let entry =
            entry.map_err(|e| io::Error::new(io::ErrorKind::Other, format!("walkdir: {e}")))?;
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
fn wrap_io<T>(result: io::Result<T>, path: &Path, op: &'static str) -> Result<T, InstallError> {
    result.map_err(|e| InstallError::FileOp {
        path: path.to_path_buf(),
        op,
        source: e,
    })
}

/// Copy all files from `stage_dir` into `rootfs`.
///
/// Uses `WalkDir` + per-file copy (regular files) / symlink recreation
/// rather than shelling out to tar.  This avoids the toybox-symlink-follow bug
/// documented in cmd_install.c:248-260 and the pipe deadlock with large packages
/// (cmd_install.c:260-262).
///
/// Directories are created first; symlinks are recreated verbatim; regular files
/// are overwritten.  This mirrors `tar -x` semantics: a destination symlink is
/// replaced by the new file, not followed.
///
/// # Modes (2.2.10)
///
/// Regular files get the staged mode **including** setuid/setgid/sticky
/// (`mode & 0o7777`), applied after the data is written: `fs::copy` sets the
/// mode before writing, and the kernel clears S_ISUID/S_ISGID on that write
/// unless the writer holds CAP_FSETID.  Directories get the staged mode only
/// when this install creates them; an existing directory keeps its mode, so a
/// package that ships `tmp/` or `root/` at 0755 cannot re-mode `/tmp` (1777)
/// or `/root` (0700).
///
/// # Config paths (2.2.11)
///
/// Called twice.  [`Pass::Rest`] writes everything except regular files and
/// symlinks at config paths (see [`crate::config`]); [`Pass::Config`] then
/// writes those, offers included, each under a scratch name (`<path>.jpkg-tmp`)
/// that is fsynced and renamed into place.  The caller re-checks the config
/// decisions between the two.  A failure in the first pass, or a crash in
/// either, leaves no half-written config file, and a failure before the
/// second pass leaves the copies the database still records.
///
/// # Errors
///
/// Returns [`InstallError::FileOp`] with the offending path on any filesystem
/// failure.  The 2.2.2-and-earlier behaviour of returning a bare `io::Error`
/// with no path attached made triage of upgrade-time collisions impossible
/// (see the 2.2.3 changelog: `symlinkat ... = -1 EEXIST` with no path).
fn install_files(stage_dir: &Path, rootfs: &Path, pass: Pass) -> Result<(), InstallError> {
    for entry in WalkDir::new(stage_dir).sort_by_file_name().into_iter() {
        let entry = entry.map_err(|e| {
            InstallError::Io(io::Error::new(
                io::ErrorKind::Other,
                format!("walkdir: {e}"),
            ))
        })?;
        let abs = entry.path();

        if abs == stage_dir {
            continue; // skip root
        }

        let rel = abs
            .strip_prefix(stage_dir)
            .expect("walkdir child of stage_dir");
        let dest = rootfs.join(rel);

        let m = wrap_io(abs.symlink_metadata(), abs, "stat staged file")?;

        let at_config = !m.is_dir() && crate::config::is_config_path(&rel.to_string_lossy());
        if at_config != (pass == Pass::Config) {
            continue;
        }
        if pass == Pass::Config {
            install_config_object(abs, &dest, &m)?;
            continue;
        }

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
            let existed = dest.symlink_metadata().is_ok();
            wrap_io(fs::create_dir_all(&dest), &dest, "create directory")?;
            if !existed {
                wrap_io(
                    fs::set_permissions(&dest, fs::Permissions::from_mode(m.mode() & 0o7777)),
                    &dest,
                    "set directory mode",
                )?;
            }
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
            wrap_io(
                fs::set_permissions(&dest, fs::Permissions::from_mode(m.mode() & 0o7777)),
                &dest,
                "set file mode",
            )?;
        }
    }
    Ok(())
}

/// Which entries [`install_files`] writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    /// Everything except regular files and symlinks at config paths.
    Rest,
    /// Only regular files and symlinks at config paths.
    Config,
}

/// Write one regular file or symlink at a config path for
/// [`install_files`]: create it under a scratch name next to `dest`
/// (`<dest>.jpkg-tmp`; an interrupted install's leftover there is removed,
/// anything else is stepped around), apply the mode after the data (the
/// setuid reason above) and fsync, then rename it over `dest`.  `dest`
/// therefore holds either what was there or the whole new object.
fn install_config_object(src: &Path, dest: &Path, m: &fs::Metadata) -> Result<(), InstallError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(p) = dest.parent() {
        wrap_io(fs::create_dir_all(p), p, "create parent directory")?;
    }
    let tmp = scratch_name(dest)?;
    let written = if m.file_type().is_symlink() {
        fs::read_link(src).and_then(|t| std::os::unix::fs::symlink(t, &tmp))
    } else {
        fs::File::open(src)
            .and_then(|mut from| {
                let mut to = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp)?;
                io::copy(&mut from, &mut to)?;
                to.flush()?;
                to.sync_all()
            })
            .and_then(|()| fs::set_permissions(&tmp, fs::Permissions::from_mode(m.mode() & 0o7777)))
    };
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(InstallError::FileOp { path: tmp, op: "write file", source: e });
    }
    if let Ok(dm) = dest.symlink_metadata() {
        if dm.is_dir() && !dm.file_type().is_symlink() {
            // Same constraint as for other files: only an empty directory
            // goes; a populated one makes the rename fail with its path.
            let _ = fs::remove_dir(dest);
        }
    }
    if let Err(e) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(InstallError::FileOp { path: dest.to_path_buf(), op: "rename file into place", source: e });
    }
    Ok(())
}

/// A free scratch name next to `dest`: `<dest>.jpkg-tmp`, or `.jpkg-tmp.1`
/// to `.9` when something that is not jpkg's leftover (a directory, a FIFO)
/// sits there.  A leftover regular file or symlink is removed.
fn scratch_name(dest: &Path) -> Result<PathBuf, InstallError> {
    let base = crate::config::with_suffix(dest, crate::config::TMP_SUFFIX);
    for n in 0..10 {
        let tmp = if n == 0 { base.clone() } else { crate::config::with_suffix(&base, &format!(".{n}")) };
        match tmp.symlink_metadata() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(tmp),
            Err(e) => return Err(InstallError::FileOp { path: tmp, op: "check scratch name", source: e }),
            Ok(m) if m.file_type().is_file() || m.file_type().is_symlink() => {
                wrap_io(fs::remove_file(&tmp), &tmp, "remove stale scratch file")?;
                return Ok(tmp);
            }
            Ok(_) => {}
        }
    }
    Err(InstallError::FileOp {
        path: base,
        op: "find a free scratch name",
        source: io::Error::new(io::ErrorKind::AlreadyExists, "all taken"),
    })
}

// ─── cross-package ownership ─────────────────────────────────────────────────

/// How the package being installed relates to other packages' claims on the
/// non-directory paths it ships.
#[derive(Debug, Default, PartialEq, Eq)]
struct ClaimPlan {
    /// `(path, owner)`: `owner` declares `replaces = [<us>]` and still owns
    /// `path`, so we leave the path alone and do not claim it.
    yield_to: Vec<(String, String)>,
    /// `(path, owner)`: `owner` also owns `path` and neither package replaces
    /// the other.  We still install the path (last install wins, as before)
    /// but report the conflict.
    conflicts: Vec<(String, String)>,
}

/// Decide, for every non-directory path in `files`, whether another installed
/// package owns it and what to do about that.
///
/// * The other package replaces us (and we do not replace it) → yield.
/// * We replace the other package → no action here; `transfer_ownership`
///   moves the path to us after install.
/// * Anything else → conflict.
fn plan_claims(
    pkg_name: &str,
    our_replaces: &[String],
    files: &[FileEntry],
    others: &Ownership,
) -> ClaimPlan {
    let mut plan = ClaimPlan::default();
    for e in files.iter().filter(|e| !e.is_dir) {
        let claims = others.owners_of(&e.path);
        if claims.is_empty() {
            continue;
        }
        let we_replace = |owner: &str| our_replaces.iter().any(|r| r == owner);
        if let Some(c) = claims
            .iter()
            .find(|c| others.owner_replaces(&c.owner, pkg_name) && !we_replace(&c.owner))
        {
            plan.yield_to.push((e.path.clone(), c.owner.clone()));
            continue;
        }
        for c in claims.iter().filter(|c| !we_replace(&c.owner)) {
            plan.conflicts.push((e.path.clone(), c.owner.clone()));
        }
    }
    plan
}

/// Summarise `(path, owner)` pairs as `owner (n)` in owner-name order.
fn count_by_owner(pairs: &[(String, String)]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, owner) in pairs {
        *counts.entry(owner.as_str()).or_default() += 1;
    }
    counts
        .iter()
        .map(|(o, n)| format!("{o} ({n})"))
        .collect::<Vec<_>>()
        .join(", ")
}

// ─── config files (2.2.11) ───────────────────────────────────────────────────

/// What step 5c decided, for the steps that write to the root later.  Each
/// list carries what jpkg recorded for the path, so 6b and 7a can decide
/// again just before writing.
#[derive(Debug, Default)]
struct ConfigPlan<'a> {
    /// Config files whose package copy is staged to land.
    install: Vec<(&'a FileEntry, Recorded<'a>)>,
    /// Config files kept, with the package's copy staged as `<path>.jpkg-new`.
    offer: Vec<(&'a FileEntry, Recorded<'a>)>,
    /// Symlinks and directories the package places at config paths (see
    /// [`crate::config::displace`]), with the link target this package's
    /// previous version recorded there, if any.
    displace: Vec<(&'a FileEntry, Recorded<'a>, Option<&'a str>)>,
}

/// Everything jpkg recorded for `path` before this install: this package's
/// previous manifest entry plus every other installed owner's.
fn recorded_for<'a>(
    path: &str,
    old_by_path: &std::collections::HashMap<&str, &'a FileEntry>,
    others: &'a Ownership,
) -> Recorded<'a> {
    let mut rec = Recorded::default();
    for c in others.owners_of(path) {
        rec.add(&c.sha256, c.symlink_target.as_deref(), c.is_dir);
    }
    if let Some(o) = old_by_path.get(path) {
        rec.add(&o.sha256, o.symlink_target.as_deref(), o.is_dir);
    }
    rec
}

/// Every hash that counts as package content at `e.path`: what jpkg
/// recorded there plus the copy being installed.
fn package_shas<'a>(e: &'a FileEntry, rec: &Recorded<'a>) -> Vec<&'a str> {
    let mut shas = rec.shas.clone();
    shas.push(&e.sha256);
    shas
}

/// What [`stage_config_file`] did with the staged copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Staged {
    /// Left in place; install_files writes it.
    Install,
    /// Dropped; the object on disk stays.
    Kept,
    /// Moved to `<path>.jpkg-new`; the object on disk stays.
    Offered,
}

/// Apply the decision for one config file to the STAGING tree: leave the
/// package's copy, drop it, or move it to `<path>.jpkg-new` -- the last only
/// when that slot on disk holds nothing or package content and the package
/// does not ship that path itself, so an admin's merge in progress there is
/// never overwritten.
fn stage_config_file(
    rootfs: &Path,
    stage_dir: &Path,
    pkg_name: &str,
    e: &FileEntry,
    rec: &Recorded<'_>,
    disk: &OnDisk,
) -> Result<Staged, InstallError> {
    use crate::config::{self, Action};
    let staged = stage_dir.join(&e.path);
    let (p, s) = (&e.path, config::NEW_SUFFIX);
    // Say "changed locally" only when jpkg recorded something here.
    let why = if rec.shas.is_empty() && rec.links.is_empty() && !rec.dir {
        "which no package installed"
    } else {
        "changed locally"
    };
    match config::action(disk, &e.sha256, rec) {
        Action::Install => Ok(Staged::Install),
        Action::Keep => {
            wrap_io(fs::remove_file(&staged), &staged, "drop staged config file")?;
            log::info!("jpkg: {pkg_name}: kept /{p} ({why})");
            Ok(Staged::Kept)
        }
        Action::KeepAndNew => {
            let new = config::with_suffix(&staged, s);
            let free = new.symlink_metadata().is_err()
                && config::new_slot_free(rootfs, p, &package_shas(e, rec)).unwrap_or_else(|err| {
                    log::warn!("jpkg: cannot check /{p}{s} ({err}); leaving it alone");
                    false
                });
            if free {
                wrap_io(fs::rename(&staged, &new), &staged, "stage config .jpkg-new")?;
                log::warn!("jpkg: {pkg_name}: kept /{p} ({why}); the packaged version is /{p}{s}");
                Ok(Staged::Offered)
            } else {
                wrap_io(fs::remove_file(&staged), &staged, "drop staged config file")?;
                log::warn!(
                    "jpkg: {pkg_name}: kept /{p} ({why}); the packaged version was NOT written: \
                     /{p}{s} is not an earlier packaged copy, so it is left alone (the package \
                     offers its copy again the next time it changes /{p})"
                );
                Ok(Staged::Kept)
            }
        }
    }
}

/// Step 5c.  For every config file in `files` (see [`crate::config`]) decide
/// whether the package's copy may land; for every symlink or directory the
/// package places at a config path, check that what [`crate::config::displace`]
/// will need is possible.  Only the STAGING tree is touched, so an error here
/// (including a probe error) aborts before the root is written.  Runs after
/// the yield step, so yielded paths never get here.
fn plan_config_files<'a>(
    rootfs: &Path,
    stage_dir: &Path,
    pkg_name: &str,
    pkg_version: &str,
    replaces: &[String],
    old_pkg: Option<&'a InstalledPkg>,
    files: &'a [FileEntry],
    others: &'a Ownership,
) -> Result<ConfigPlan<'a>, InstallError> {
    use crate::config::{self, Displace};
    let mut plan = ConfigPlan::default();
    let old_by_path: std::collections::HashMap<&str, &'a FileEntry> = old_pkg
        .map(|o| o.files.iter().map(|e| (e.path.as_str(), e)).collect())
        .unwrap_or_default();

    // Symlinks and directories the package places at config paths first,
    // parents before children: where a link is removed or saved aside to
    // make room for a directory, everything below it is decided as if
    // nothing were there yet (6b clears the link before anything is written
    // through it), never by looking through the link.
    let shipped: HashSet<&str> = files.iter().map(|e| e.path.as_str()).collect();
    let mut gone_links: Vec<&str> = Vec::new();
    let below = |links: &[&str], p: &str| {
        links.iter().any(|l| p.strip_prefix(l).is_some_and(|r| r.starts_with('/')))
    };
    for n in files.iter().filter(|n| config::is_config_path(&n.path) && !config::is_config(n)) {
        let rec = recorded_for(&n.path, &old_by_path, others);
        let own = old_by_path.get(n.path.as_str()).and_then(|o| o.symlink_target.as_deref());
        let dest = rootfs.join(&n.path);
        let disk = if below(&gone_links, &n.path) {
            OnDisk::Missing
        } else {
            wrap_io(config::on_disk(rootfs, &n.path), &dest, "check config path")?
        };
        let own_link = own.is_some_and(|t| matches!(&disk, OnDisk::Link(d) if d == t));
        let decision = config::displace(&disk, n, &rec, leads_to_dir(&disk, &dest, &n.path, rootfs), own_link);
        if decision == Displace::Keep {
            keep_admin_object(stage_dir, pkg_name, n, &rec)?;
        }
        if decision == Displace::Remove && own_link {
            check_own_link_contents(rootfs, &n.path, &old_by_path, pkg_name, pkg_version)?;
        }
        if decision == Displace::Remove && !own_link && matches!(disk, OnDisk::Link(_)) {
            // Another package's link that cannot be followed here (an
            // absolute one under --root) but does lead to a directory in the
            // root: removing it would break that package, following it would
            // write outside the root.  Refuse before any write -- unless this
            // package replaces every owner of the link (a takeover).
            let owners = others.owners_of(&n.path);
            let taken_over = !owners.is_empty() && owners.iter().all(|c| replaces.iter().any(|r| *r == c.owner));
            if !taken_over && config::resolve_in_root(rootfs, Path::new(&n.path)).is_some_and(|p| p.is_dir()) {
                return Err(InstallError::FileOp {
                    path: dest,
                    op: "place a directory over another package's symlink to a directory, which cannot be \
                         followed inside this root (install without --root, or have that package use a \
                         relative link)",
                    source: io::Error::new(io::ErrorKind::AlreadyExists, "in use by another package"),
                });
            }
        }
        if matches!(decision, Displace::Remove | Displace::Save) && matches!(disk, OnDisk::Link(_)) {
            gone_links.push(&n.path);
        }
        if decision == Displace::Save {
            let save = config::with_suffix(&dest, config::SAVE_SUFFIX);
            let refuse = |why: &'static str, source: io::Error| InstallError::FileOp {
                path: save.clone(),
                op: why,
                source,
            };
            if shipped.contains(format!("{}{}", n.path, config::SAVE_SUFFIX).as_str()) {
                return Err(refuse(
                    "keep a locally changed config path (the package itself ships its .jpkg-save)",
                    io::Error::new(io::ErrorKind::AlreadyExists, "would overwrite"),
                ));
            }
            match save.symlink_metadata() {
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(InstallError::FileOp {
                        path: save,
                        op: "keep a locally changed config file (move the existing .jpkg-save away and retry)",
                        source: io::Error::new(io::ErrorKind::AlreadyExists, "would overwrite"),
                    });
                }
                Err(err) => return Err(refuse("check config .jpkg-save", err)),
            }
        }
        plan.displace.push((n, rec, own));
    }

    for e in files.iter().filter(|e| config::is_config(e)) {
        let rec = recorded_for(&e.path, &old_by_path, others);
        let dest = rootfs.join(&e.path);
        let disk = if below(&gone_links, &e.path) {
            OnDisk::Missing
        } else {
            wrap_io(config::on_disk(rootfs, &e.path), &dest, "check config file")?
        };
        // A directory jpkg recorded here (the package's own old layout, or
        // another owner's) is decided in 7a, once upgrade-clean has had the
        // chance to empty it.
        let staged = if disk == OnDisk::Dir && rec.dir {
            Staged::Install
        } else {
            stage_config_file(rootfs, stage_dir, pkg_name, e, &rec, &disk)?
        };
        match staged {
            Staged::Install => plan.install.push((e, rec)),
            Staged::Offered => plan.offer.push((e, rec)),
            Staged::Kept => {}
        }
    }
    Ok(plan)
}

/// True when `disk` is a symlink at `abs` (`rel` under `rootfs`) that
/// install_files may follow: create_dir_all resolves it on the host, so it
/// counts only when that host resolution is the same directory the link
/// leads to inside the root.  Under an alternate root that rules out an
/// absolute link, and a relative one that climbs out of the root.
fn leads_to_dir(disk: &OnDisk, abs: &Path, rel: &str, rootfs: &Path) -> bool {
    if !matches!(disk, OnDisk::Link(_)) {
        return false;
    }
    let (Ok(host), Some(inside)) = (fs::canonicalize(abs), crate::config::resolve_in_root(rootfs, Path::new(rel))) else {
        return false;
    };
    let inside = fs::canonicalize(&inside).unwrap_or(inside);
    host == inside && host.is_dir()
}

/// Before 6b removes this package's own previous link to make room for the
/// directory it now ships: everything below the directory the link leads to
/// (resolved inside the root, as a chroot would, and through any further
/// links that lead to directories) must be what that previous version
/// recorded there (by its real path) and still pristine, or an untouched
/// offer or scratch file of jpkg's.  Anything else -- the admin's files,
/// another package's -- would silently drop out of /etc, so the upgrade is
/// refused before any write, naming it, the same way a directory turning into
/// a link is refused.  A link that leads nowhere has nothing below it.
fn check_own_link_contents(
    rootfs: &Path,
    link_rel: &str,
    old_by_path: &std::collections::HashMap<&str, &FileEntry>,
    pkg_name: &str,
    pkg_version: &str,
) -> Result<(), InstallError> {
    let Some(target) = crate::config::resolve_in_root(rootfs, Path::new(link_rel)) else {
        return Ok(());
    };
    let top = target.clone();
    let mut dirs = vec![target];
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut foreign: Vec<PathBuf> = Vec::new();
    while let Some(dir) = dirs.pop() {
        if !dir.is_dir() || !seen.insert(dir.clone()) {
            continue;
        }
        for entry in WalkDir::new(&dir).min_depth(1) {
            let entry = entry.map_err(|e| {
                InstallError::Io(io::Error::new(io::ErrorKind::Other, format!("walkdir {}: {e}", dir.display())))
            })?;
            if entry.file_type().is_dir() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(rootfs) else {
                continue;
            };
            let key = rel.to_string_lossy();
            let ours = match old_by_path.get(key.as_ref()) {
                Some(o) => !crate::config::keep_probe(crate::config::on_disk(rootfs, &key), &Recorded::entry(o)),
                None => crate::config::is_package_leftover(rootfs, &key, old_by_path),
            };
            if !ours {
                foreign.push(entry.path().to_path_buf());
            }
            // A link below that leads to a directory: what is behind it
            // drops out of /etc too.
            // (Not one that points back up to the directory itself, an
            // ancestor or the root: nothing new is behind that.)
            if entry.path_is_symlink() {
                if let Some(next) = crate::config::resolve_in_root(rootfs, rel) {
                    if !top.starts_with(&next) {
                        dirs.push(next);
                    }
                }
            }
        }
    }
    if foreign.is_empty() {
        return Ok(());
    }
    foreign.sort();
    foreign.dedup();
    Err(InstallError::UpgradeForeignFiles {
        pkg: pkg_name.to_string(),
        new_version: pkg_version.to_string(),
        dir: rootfs.join(link_rel),
        foreign,
    })
}

/// [`crate::config::Displace::Keep`]: the admin's object stays, so the
/// package's symlink is dropped from staging (it may already be gone when
/// 6b repeats 5c's decision).
fn keep_admin_object(
    stage_dir: &Path,
    pkg_name: &str,
    n: &FileEntry,
    rec: &Recorded<'_>,
) -> Result<(), InstallError> {
    let staged = stage_dir.join(&n.path);
    if staged.symlink_metadata().is_ok() {
        wrap_io(fs::remove_file(&staged), &staged, "drop staged symlink")?;
        let why = if rec.shas.is_empty() && rec.links.is_empty() && !rec.dir {
            "which no package installed"
        } else {
            "changed locally"
        };
        log::warn!(
            "jpkg: {pkg_name}: kept /{} ({why}); the package's link to {} was not written",
            n.path,
            n.symlink_target.as_deref().unwrap_or("?")
        );
    }
    Ok(())
}

/// A config path this package is giving up (remove, or an upgrade that no
/// longer ships it) stays because another package also lists it.  If what
/// is there is this package's copy and no remaining owner recorded those
/// bytes, the remaining owner will treat it as changed locally from now
/// on: say so, and how to get the owner's own copy back.
pub(crate) fn warn_left_for_co_owner(rootfs: &Path, pkg_name: &str, e: &FileEntry, others: &Ownership) {
    if !crate::config::is_config(e) {
        return;
    }
    let claims = others.owners_of(&e.path);
    match crate::config::on_disk(rootfs, &e.path) {
        Ok(OnDisk::File(h)) if h == e.sha256 && !claims.iter().any(|c| c.sha256 == h) => {
            let owners: Vec<&str> = claims.iter().map(|c| c.owner.as_str()).collect();
            log::warn!(
                "jpkg: /{p} stays for {o}, but it holds {pkg_name}'s copy, which {o} will keep as \
                 changed locally; to get {first}'s own copy: rm /{p} && jpkg install --force {first}",
                p = e.path,
                o = owners.join(", "),
                first = owners.first().copied().unwrap_or("?"),
            );
        }
        _ => {}
    }
}

/// Step 6b.  Clear the way for the symlinks and directories the package
/// places at config paths, deciding again on what is there now: a pristine
/// file in the way of a directory is removed, anything not pristine is
/// moved to `<path>.jpkg-save` (never over an existing one).
fn displace_config_paths(
    rootfs: &Path,
    stage_dir: &Path,
    pkg_name: &str,
    pkg_version: &str,
    plan: &ConfigPlan<'_>,
) -> Result<(), InstallError> {
    use crate::config::{self, Displace};
    for (n, rec, own) in &plan.displace {
        let from = rootfs.join(&n.path);
        let disk = config::on_disk(rootfs, &n.path).unwrap_or_else(|err| {
            log::warn!("jpkg: cannot check /{} ({err}); treating it as changed", n.path);
            OnDisk::Other
        });
        let own_link = own.is_some_and(|t| matches!(&disk, OnDisk::Link(d) if d == t));
        match config::displace(&disk, n, rec, leads_to_dir(&disk, &from, &n.path, rootfs), own_link) {
            Displace::Leave => {}
            Displace::Keep => keep_admin_object(stage_dir, pkg_name, n, rec)?,
            Displace::Remove => wrap_io(fs::remove_file(&from), &from, "remove packaged file in the way of a directory")?,
            Displace::Save => {
                let to = config::with_suffix(&from, config::SAVE_SUFFIX);
                if to.symlink_metadata().is_ok() {
                    return Err(InstallError::FileOp {
                        path: to,
                        op: "keep a locally changed config file (move the existing .jpkg-save away and retry)",
                        source: io::Error::new(io::ErrorKind::AlreadyExists, "would overwrite"),
                    });
                }
                match fs::rename(&from, &to) {
                    Ok(()) => log::warn!(
                        "jpkg: {pkg_name}-{pkg_version} replaces /{p} with a link or a directory; \
                         what was there is now /{p}{s}",
                        p = n.path,
                        s = config::SAVE_SUFFIX
                    ),
                    Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                    Err(err) => {
                        return Err(InstallError::FileOp {
                            path: from,
                            op: "save locally changed config file",
                            source: err,
                        })
                    }
                }
            }
        }
    }
    Ok(())
}

/// Step 7a.  Upgrade-clean, 6b and the non-config copy have run; look at
/// every config path once more right before its file is written, so an
/// object that changed since 5c is kept the same way, an offer slot the
/// admin started using is left alone, and a directory jpkg recorded is
/// replaced only if it is now empty.  The writes themselves (7b) follow
/// immediately; an edit made during them can still be lost.  Probe errors
/// keep: nothing here aborts on one, because the root has already been
/// written.
fn recheck_config_files(
    rootfs: &Path,
    stage_dir: &Path,
    pkg_name: &str,
    plan: &mut ConfigPlan<'_>,
) -> Result<(), InstallError> {
    use crate::config;
    let (s, mut offer) = (config::NEW_SUFFIX, Vec::new());
    for (e, rec) in std::mem::take(&mut plan.offer) {
        let free = config::new_slot_free(rootfs, &e.path, &package_shas(e, &rec)).unwrap_or(false);
        if free {
            offer.push((e, rec));
        } else {
            let staged = config::with_suffix(&stage_dir.join(&e.path), s);
            wrap_io(fs::remove_file(&staged), &staged, "drop staged config .jpkg-new")?;
            log::warn!(
                "jpkg: {pkg_name}: /{p}{s} changed during the upgrade; the packaged version of /{p} was NOT written",
                p = e.path
            );
        }
    }
    let mut install = Vec::new();
    for (e, rec) in std::mem::take(&mut plan.install) {
        let disk = config::on_disk(rootfs, &e.path).unwrap_or_else(|err| {
            log::warn!("jpkg: cannot check /{} ({err}); keeping it", e.path);
            OnDisk::Other
        });
        let empty_recorded_dir =
            disk == OnDisk::Dir && rec.dir && config::is_empty_dir(rootfs, &e.path).unwrap_or(false);
        let staged = if empty_recorded_dir {
            Staged::Install
        } else {
            stage_config_file(rootfs, stage_dir, pkg_name, e, &rec, &disk)?
        };
        match staged {
            Staged::Install => install.push((e, rec)),
            Staged::Offered => offer.push((e, rec)),
            Staged::Kept => {}
        }
    }
    plan.install = install;
    plan.offer = offer;
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
/// 3. For each entry in `old_files - new_files` that no OTHER installed
///    package still lists (`others`), delete it from rootfs:
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
    others: &Ownership,
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
    let old_owned: std::collections::HashMap<&str, &FileEntry> =
        old_pkg.files.iter().map(|e| (e.path.as_str(), e)).collect();

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
                Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => {
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
                let entry = entry.map_err(|e| {
                    InstallError::Io(io::Error::new(
                        io::ErrorKind::Other,
                        format!("walkdir while scanning {}: {e}", on_disk.display()),
                    ))
                })?;
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
                let is_dir = on_disk_md.is_dir() && !on_disk_md.file_type().is_symlink();
                match old_owned.get(rel_str.as_ref()) {
                    // An untouched offer, or an interrupted install's
                    // scratch, next to an owned path is jpkg's, not the
                    // admin's.
                    None if !is_dir
                        && !crate::config::is_package_leftover(rootfs, &rel_str, &old_owned) =>
                    {
                        foreign.push(abs.to_path_buf())
                    }
                    None => {}
                    // 2.2.11: an owned file or link at a config path that the
                    // admin changed -- also into a directory -- is theirs now;
                    // blasting the directory would destroy it, so it stops
                    // the upgrade like a foreign file.
                    Some(owned) if crate::config::keep_on_disk(rootfs, owned) => {
                        foreign.push(abs.to_path_buf())
                    }
                    Some(_) => {}
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
        let still = new_by_path.get(old.path.as_str());
        // 2.2.11: jpkg's own leftovers next to a config file this version no
        // longer ships as one (dropped, turned into a link or a directory,
        // or left to a co-owner): an untouched offer is stale, and so is an
        // interrupted install's scratch.  Dropping them first also lets a
        // directory holding one empty out.
        if crate::config::is_config(old) && !still.is_some_and(|n| crate::config::is_config(n)) {
            if let Err(e) = crate::config::drop_stale_new(rootfs, &old.path, &[&old.sha256]) {
                log::warn!("jpkg: could not check /{}{}: {e}", old.path, crate::config::NEW_SUFFIX);
            }
        }
        if crate::config::is_config_path(&old.path) && still.map_or(true, |n| n.is_dir) {
            if let Err(e) = crate::config::drop_scratch(rootfs, &old.path) {
                log::warn!("jpkg: could not check /{}{}: {e}", old.path, crate::config::TMP_SUFFIX);
            }
        }
        if still.is_some() {
            // Path still owned in new manifest — install_files will overwrite
            // appropriately.  Don't remove it here or there'd be a window
            // where the rootfs is missing the file.
            continue;
        }
        if others.is_claimed(&old.path) {
            // Another installed package still lists this path (a shared
            // init script, a link a `replaces` package took over, …).  It
            // is theirs now; deleting it would break them.
            warn_left_for_co_owner(rootfs, new_pkg_name, old, others);
            log::debug!(
                "jpkg: upgrade-clean: keeping {} (still owned by {})",
                old.path,
                others
                    .owners_of(&old.path)
                    .iter()
                    .map(|c| c.owner.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            continue;
        }
        if crate::config::keep_on_disk(rootfs, old) {
            // 2.2.11: a config file the admin changed (or that cannot be
            // checked) stays where it is and is theirs from now on.
            log::warn!(
                "jpkg: {new_pkg_name}-{new_pkg_version} no longer ships /{}; kept your locally changed copy (no package owns it now)",
                old.path
            );
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
            // ENOTDIR: a parent is no longer a directory (the admin replaced
            // it with a file), so this path cannot exist.
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => {}
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
/// 2. Create a temp staging directory under the target root's `var/tmp`, unless
///    `JPKG_STAGE_DIR` or the standard temp environment explicitly overrides it.
/// 3. `archive.extract(stage)` — decompresses the zstd(tar) payload.
/// 4. `flatten_merged_usr(stage)` — jonerix merged-usr layout.
/// 5. Build the NEW file manifest from the staged tree (needed before step 6).
///    5b. Cross-package ownership (2.2.10): drop staged paths still owned by
///    a package that declares `replaces = [<this package>]`, and warn about
///    paths another package owns without any `replaces` relationship.
///    5c. Config files (2.2.11): decide, in the staging tree only, which
///    config files land, which are kept, and which get a `.jpkg-new`; refuse
///    here, before any write, what 6b could not do safely.
/// 6. Upgrade-clean: if `db.get(name)?` returns Some(old), remove old-manifest
///    files that aren't in the new manifest and that no other package owns,
///    and resolve dir→symlink layout flips owned by the same package.
///    6b. Clear the way for symlinks and directories at config paths
///    (`.jpkg-save`, or jpkg's own link removed).
/// 7. `install_files(stage, rootfs, Pass::Rest)` — copy everything but
///    files and links at config paths.  7a. Decide every config file again.
///    7b. `Pass::Config` — write those, each atomically.  7c. Remove offers
///    made stale by a config file that now matches.
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
    // Use a randomly named, newly-created directory.  The old
    // jpkg-stage-<pkg>-<pid> path was predictable and collided when parallel
    // installs extracted the same package name inside one process.
    let stage_guard = make_install_stage_dir(rootfs)?;
    let stage_dir = stage_guard.path().to_path_buf();

    // ── 3. Extract ────────────────────────────────────────────────────────
    archive.extract(&stage_dir)?;

    // ── 4. Flatten usr/ and lib64/ ────────────────────────────────────────
    flatten_merged_usr(&stage_dir)?;

    // ── 5. Build the new manifest now (before install) so upgrade-clean can
    //       diff it against the old manifest.  Built from the staging dir so
    //       sha256s etc. are computed exactly once.
    let mut files = build_manifest(&stage_dir)?;

    // ── 5b. Cross-package ownership.  Look up which OTHER installed packages
    //        list any path we ship (or used to ship).
    let old_pkg = db.get(&pkg_name)?;
    let others = {
        let mut wanted: HashSet<&str> = files.iter().map(|e| e.path.as_str()).collect();
        if let Some(old) = &old_pkg {
            wanted.extend(old.files.iter().map(|e| e.path.as_str()));
        }
        db.path_owners(Some(&wanted), Some(&pkg_name))?
    };
    let plan = plan_claims(&pkg_name, &metadata.package.replaces, &files, &others);

    // Yield: a package that declares `replaces = [<us>]` owns these paths.
    // Drop them from the staging tree and from our manifest so installing
    // (or reinstalling) us cannot clobber them — e.g. toybox must not lay
    // its /bin/<applet> links over uutils', bsdtar's or ncurses'.
    if !plan.yield_to.is_empty() {
        for (path, owner) in &plan.yield_to {
            log::debug!("jpkg: {pkg_name}: leaving {path} to {owner}");
            let staged = stage_dir.join(path);
            wrap_io(fs::remove_file(&staged), &staged, "drop yielded path")?;
        }
        let yielded: HashSet<&str> = plan.yield_to.iter().map(|(p, _)| p.as_str()).collect();
        files.retain(|e| !yielded.contains(e.path.as_str()));
        log::info!(
            "jpkg: {pkg_name}: left {} path(s) to packages that replace it: {}",
            plan.yield_to.len(),
            count_by_owner(&plan.yield_to)
        );
    }

    // Conflict: someone else owns a path we are about to overwrite, and
    // neither package replaces the other.  Keep the historical behaviour
    // (install it; last install wins) but say so, naming the paths.
    if !plan.conflicts.is_empty() {
        const SHOW: usize = 8;
        let shown: Vec<String> = plan
            .conflicts
            .iter()
            .take(SHOW)
            .map(|(p, o)| format!("/{p} ({o})"))
            .collect();
        let more = plan.conflicts.len().saturating_sub(SHOW);
        log::warn!(
            "jpkg: file conflict: {pkg_name}-{pkg_version} overwrites {} path(s) also owned by \
             other packages: {}{}; see `jpkg owns --conflicts`",
            plan.conflicts.len(),
            shown.join(", "),
            if more > 0 {
                format!(" (+{more} more)")
            } else {
                String::new()
            }
        );
    }

    // ── 5c. Config files (2.2.11): keep what the admin changed.  Touches
    //        only the staging tree.  The manifest keeps the package's hash
    //        either way.
    let mut config_plan = plan_config_files(
        rootfs,
        &stage_dir,
        &pkg_name,
        &pkg_version,
        &metadata.package.replaces,
        old_pkg.as_ref(),
        &files,
        &others,
    )?;

    // ── 6. Upgrade-clean — only when an older version of this package is
    //       already in the db.  We do this BEFORE install_files so the
    //       populated-dir-to-symlink case (the ncurses bug) doesn't trip the
    //       symlink call's `EEXIST`.  Paths another package still owns are
    //       never removed.
    if let Some(old_pkg) = &old_pkg {
        log::debug!(
            "jpkg: upgrade-clean: {} {} -> {}",
            pkg_name,
            old_pkg.metadata.package.version.as_deref().unwrap_or("?"),
            pkg_version,
        );
        clean_old_files_for_upgrade(rootfs, old_pkg, &files, &pkg_name, &pkg_version, &others)?;
    }

    // ── 6b. Config paths (2.2.11): clear the way for a symlink or a
    //        directory the package places at a config path; anything not
    //        pristine there goes to <path>.jpkg-save.  After upgrade-clean,
    //        whose validation can still refuse the upgrade, and which never
    //        touches a path the new version still lists.
    displace_config_paths(rootfs, &stage_dir, &pkg_name, &pkg_version, &config_plan)?;

    // ── 7. Install files into rootfs, all but config files and links ─────
    install_files(&stage_dir, rootfs, Pass::Rest)?;

    // ── 7a. Config files (2.2.11): decide again right before writing them.
    recheck_config_files(rootfs, &stage_dir, &pkg_name, &mut config_plan)?;

    // ── 7b. Config files and links at config paths, each atomically.
    install_files(&stage_dir, rootfs, Pass::Config)?;

    // ── 7c. Config files (2.2.11): once a path holds the package's copy, a
    //        leftover <path>.jpkg-new that is pure package content is stale.
    //        Anything else there is the admin's and stays.  Only warns.
    for (e, rec) in &config_plan.install {
        let p = &e.path;
        match crate::config::drop_stale_new(rootfs, p, &package_shas(e, rec)) {
            Ok(true) => log::debug!("jpkg: removed stale /{p}{}", crate::config::NEW_SUFFIX),
            Ok(false) => {}
            Err(e) => log::warn!("jpkg: could not check /{p}{}: {e}", crate::config::NEW_SUFFIX),
        }
    }

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

    Ok(pkg)
}

fn temp_env_is_set() -> bool {
    ["TMPDIR", "TEMP", "TMP"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty()))
}

fn choose_install_stage_parent(
    rootfs: &Path,
    explicit_stage_dir: Option<PathBuf>,
    temp_env_set: bool,
) -> Option<PathBuf> {
    if explicit_stage_dir
        .as_ref()
        .is_some_and(|p| !p.as_os_str().is_empty())
    {
        return explicit_stage_dir;
    }
    if temp_env_set {
        return None;
    }
    if rootfs == Path::new("/") {
        Some(PathBuf::from("/var/tmp"))
    } else {
        Some(rootfs.join("var/tmp"))
    }
}

fn make_install_stage_dir(rootfs: &Path) -> Result<tempfile::TempDir, InstallError> {
    let explicit_stage_dir = std::env::var_os("JPKG_STAGE_DIR").map(PathBuf::from);
    let stage_parent = choose_install_stage_parent(rootfs, explicit_stage_dir, temp_env_is_set());
    let mut builder = tempfile::Builder::new();
    builder.prefix("jpkg-stage-");

    if let Some(parent) = stage_parent {
        fs::create_dir_all(&parent).map_err(|source| InstallError::FileOp {
            path: parent.clone(),
            op: "create staging directory parent",
            source,
        })?;
        builder
            .tempdir_in(&parent)
            .map_err(|source| InstallError::FileOp {
                path: parent,
                op: "create staging directory",
                source,
            })
    } else {
        builder.tempdir().map_err(InstallError::Io)
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
    use crate::recipe::{DependsSection, FilesSection, HooksSection, Metadata, PackageSection};
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

    pub fn make_metadata_with_replaces(
        name: &str,
        version: &str,
        replaces: Vec<String>,
    ) -> Metadata {
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
    pub fn build_test_jpkg_with_hook(
        tmp: &Path,
        name: &str,
        version: &str,
        post_install: &str,
    ) -> PathBuf {
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
    pub fn build_jpkg_with_terminfo_symlink(tmp: &Path, name: &str, version: &str) -> PathBuf {
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
        assert!(
            baz.sha256.is_empty(),
            "symlink sha256 should be empty in FileEntry"
        );
    }

    // ── 2. flatten_merged_usr ─────────────────────────────────────────────────

    #[test]
    fn test_flatten_merged_usr_moves_usr() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("d");
        fs::create_dir_all(destdir.join("usr/bin")).unwrap();
        fs::write(destdir.join("usr/bin/hello"), b"hello").unwrap();

        flatten_merged_usr(&destdir).unwrap();

        assert!(
            destdir.join("bin/hello").exists(),
            "bin/hello should exist after flatten"
        );
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

        assert!(
            destdir.join("lib/ld.so").exists(),
            "lib/ld.so should exist after flatten"
        );
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
        assert!(destdir
            .join("usr")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
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
        assert!(
            rootfs.join("bin/foo").exists(),
            "bin/foo should be installed"
        );
        assert!(
            rootfs.join("lib/bar").exists(),
            "lib/bar should be installed"
        );

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
        assert!(
            a_before.files.iter().any(|e| e.path == "bin/sh"),
            "A should own bin/sh"
        );

        // Install pkg B — it replaces A and installs its own bin/sh.
        let b_jpkg =
            build_jpkg_with_replaces(tmp.path(), "pkgB", "1.0.0", vec!["pkgA".to_string()]);
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
        assert!(
            b.files.iter().any(|e| e.path == "bin/sh"),
            "B should own bin/sh"
        );
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
        let hook = format!("touch \"$JPKG_ROOT/marker\"",);
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
        assert!(
            lib_terminfo.is_dir(),
            "v1 should install lib/terminfo as a directory"
        );
        assert!(
            lib_terminfo.join("a").exists(),
            "v1 should install lib/terminfo/a"
        );
        assert!(
            lib_terminfo.join("b").exists(),
            "v1 should install lib/terminfo/b"
        );

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

        assert!(
            rootfs.join("share/dropme").exists(),
            "v1 should install share/dropme"
        );
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
        assert!(
            rootfs.join("bin/foo").exists(),
            "bin/foo should survive upgrade"
        );
        // lib/bar (new in v2) should appear.
        assert!(
            rootfs.join("lib/bar").exists(),
            "lib/bar should be installed by v2"
        );

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
        let err = extract_and_register(&arc_v2, &rootfs, &db)
            .expect_err("upgrade should fail when foreign files are present in dir being replaced");

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
        assert!(
            foreign.exists(),
            "foreign file should not have been touched"
        );
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

        let err = install_files(&stage, &rootfs, Pass::Rest)
            .expect_err("install_files should fail when target dir is populated");

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

    #[test]
    fn install_stage_parent_defaults_to_root_var_tmp() {
        let parent = choose_install_stage_parent(Path::new("/"), None, false);
        assert_eq!(parent.as_deref(), Some(Path::new("/var/tmp")));
    }

    #[test]
    fn install_stage_parent_tracks_alternate_root() {
        let parent = choose_install_stage_parent(Path::new("/alt-root"), None, false);
        assert_eq!(parent.as_deref(), Some(Path::new("/alt-root/var/tmp")));
    }

    #[test]
    fn install_stage_parent_respects_explicit_override() {
        let parent = choose_install_stage_parent(
            Path::new("/"),
            Some(PathBuf::from("/scratch/jpkg")),
            false,
        );
        assert_eq!(parent.as_deref(), Some(Path::new("/scratch/jpkg")));
    }

    #[test]
    fn install_stage_parent_defers_to_standard_temp_env() {
        let parent = choose_install_stage_parent(Path::new("/"), None, true);
        assert!(parent.is_none());
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

        assert!(
            rootfs.join("bin/foo").exists(),
            "bin/foo should still exist"
        );
        assert!(
            rootfs.join("lib/bar").exists(),
            "lib/bar should still exist"
        );
    }

    // ── 11. 2.2.10: modes, cross-package ownership, legacy manifests ─────────

    /// One entry of a synthetic package tree.
    #[derive(Clone, Copy)]
    pub(crate) enum Node<'a> {
        File(&'a [u8], u32),
        Link(&'a str),
        Dir(u32),
    }

    /// Build a .jpkg whose payload is exactly `nodes` (parents are created
    /// 0755 as needed; listed directories get the given mode).
    pub(crate) fn build_jpkg_tree(
        tmp: &Path,
        name: &str,
        version: &str,
        replaces: &[&str],
        nodes: &[(&str, Node<'_>)],
    ) -> PathBuf {
        let destdir = tmp.join(format!("tree-{name}-{version}"));
        fs::create_dir_all(&destdir).unwrap();
        for (path, node) in nodes {
            let p = destdir.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            match node {
                Node::File(data, mode) => {
                    fs::write(&p, data).unwrap();
                    fs::set_permissions(&p, fs::Permissions::from_mode(*mode)).unwrap();
                }
                Node::Link(target) => symlink(target, &p).unwrap(),
                Node::Dir(mode) => {
                    fs::create_dir_all(&p).unwrap();
                    fs::set_permissions(&p, fs::Permissions::from_mode(*mode)).unwrap();
                }
            }
        }
        let meta = make_metadata_with_replaces(
            name,
            version,
            replaces.iter().map(|r| r.to_string()).collect(),
        );
        let out = tmp.join(format!("{name}-{version}-x86_64.jpkg"));
        archive::create(&out, &meta.to_string().unwrap(), &destdir).unwrap();
        out
    }

    fn install(rootfs: &Path, db: &InstalledDb, jpkg: &Path) -> InstalledPkg {
        let arc = JpkgArchive::open(jpkg).unwrap();
        extract_and_register(&arc, rootfs, db).unwrap()
    }

    fn mode_of(p: &Path) -> u32 {
        p.symlink_metadata().unwrap().permissions().mode() & 0o7777
    }

    fn manifest_has(db: &InstalledDb, pkg: &str, path: &str) -> bool {
        db.get(pkg)
            .unwrap()
            .unwrap()
            .files
            .iter()
            .any(|e| e.path == path)
    }

    #[test]
    fn install_preserves_setuid_setgid_modes_on_disk_and_in_manifest() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        let pkg = build_jpkg_tree(
            tmp.path(),
            "suidpkg",
            "1.0.0",
            &[],
            &[
                ("bin/sudo", Node::File(b"sudo stub", 0o4755)),
                ("bin/wall", Node::File(b"wall stub", 0o2755)),
                ("bin/plain", Node::File(b"plain", 0o755)),
            ],
        );
        let got = install(&rootfs, &db, &pkg);

        assert_eq!(mode_of(&rootfs.join("bin/sudo")), 0o4755);
        assert_eq!(mode_of(&rootfs.join("bin/wall")), 0o2755);
        assert_eq!(mode_of(&rootfs.join("bin/plain")), 0o755);
        let m = |p: &str| got.files.iter().find(|e| e.path == p).unwrap().mode & 0o7777;
        assert_eq!(m("bin/sudo"), 0o4755, "manifest must record the setuid bit");
        assert_eq!(m("bin/wall"), 0o2755);

        // Reinstall over the existing file keeps the bits too.
        install(&rootfs, &db, &pkg);
        assert_eq!(mode_of(&rootfs.join("bin/sudo")), 0o4755);
    }

    #[test]
    fn install_never_remodes_existing_directories() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(rootfs.join("tmp")).unwrap();
        fs::create_dir_all(rootfs.join("root")).unwrap();
        fs::set_permissions(rootfs.join("tmp"), fs::Permissions::from_mode(0o1777)).unwrap();
        fs::set_permissions(rootfs.join("root"), fs::Permissions::from_mode(0o700)).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // Ships tmp/ and root/ at 0755, like zig and rust do.
        let pkg = build_jpkg_tree(
            tmp.path(),
            "dirpkg",
            "1.0.0",
            &[],
            &[
                ("tmp", Node::Dir(0o755)),
                ("tmp/junk", Node::Dir(0o755)),
                ("root", Node::Dir(0o755)),
                ("root/.cargo/config.toml", Node::File(b"x", 0o644)),
                ("var/spool/mail", Node::Dir(0o1777)),
                ("etc/secret", Node::Dir(0o700)),
            ],
        );
        install(&rootfs, &db, &pkg);

        assert_eq!(mode_of(&rootfs.join("tmp")), 0o1777, "/tmp must stay 1777");
        assert_eq!(mode_of(&rootfs.join("root")), 0o700, "/root must stay 0700");
        // Directories this install created get the packaged mode.
        assert_eq!(mode_of(&rootfs.join("tmp/junk")), 0o755);
        assert_eq!(mode_of(&rootfs.join("var/spool/mail")), 0o1777);
        assert_eq!(mode_of(&rootfs.join("etc/secret")), 0o700);
    }

    #[test]
    fn plan_claims_classifies_yield_conflict_and_takeover() {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();
        let entry = |p: &str| FileEntry {
            path: p.to_string(),
            sha256: "a".repeat(64),
            size: 0,
            mode: 0o100755,
            symlink_target: None,
            is_dir: false,
        };
        let add = |name: &str, replaces: &[&str], paths: &[&str]| {
            db.insert(&InstalledPkg {
                metadata: make_metadata_with_replaces(
                    name,
                    "1",
                    replaces.iter().map(|r| r.to_string()).collect(),
                ),
                files: paths.iter().map(|p| entry(p)).collect(),
            })
            .unwrap();
        };
        add("uutils", &["toybox"], &["bin/sort"]);
        add("openrc", &[], &["bin/rc"]);
        add("oldbox", &[], &["bin/old"]);

        let ours = vec![
            entry("bin/sort"),
            entry("bin/rc"),
            entry("bin/old"),
            entry("bin/mine"),
        ];
        let others = db.path_owners(None, Some("toybox")).unwrap();
        let plan = plan_claims("toybox", &["oldbox".to_string()], &ours, &others);
        assert_eq!(
            plan,
            ClaimPlan {
                yield_to: vec![("bin/sort".into(), "uutils".into())],
                conflicts: vec![("bin/rc".into(), "openrc".into())],
            }
        );
    }

    /// The tormenta bug: installing toybox after uutils/bsdtar/jonerix-util
    /// (all `replaces = ["toybox"]`) laid toybox's links over theirs.
    #[test]
    fn installing_replaced_package_does_not_clobber_replacers() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        let toybox_nodes = [
            ("bin/toybox", Node::File(b"toybox", 0o755)),
            ("bin/sort", Node::Link("toybox")),
            ("bin/tar", Node::Link("toybox")),
            ("bin/hwclock", Node::Link("toybox")),
            ("bin/ping", Node::Link("toybox")),
        ];
        let tb1 = build_jpkg_tree(tmp.path(), "toybox", "0.8.11-r14", &[], &toybox_nodes);
        install(&rootfs, &db, &tb1);

        let uu = build_jpkg_tree(
            tmp.path(),
            "uutils",
            "0.7.0-r2",
            &["toybox"],
            &[
                ("bin/uutils", Node::File(b"uutils", 0o755)),
                ("bin/sort", Node::Link("uutils")),
            ],
        );
        install(&rootfs, &db, &uu);
        let bt = build_jpkg_tree(
            tmp.path(),
            "bsdtar",
            "3.8",
            &["toybox"],
            &[
                ("bin/bsdtar", Node::File(b"bsdtar", 0o755)),
                ("bin/tar", Node::Link("bsdtar")),
            ],
        );
        install(&rootfs, &db, &bt);
        let ju = build_jpkg_tree(
            tmp.path(),
            "jonerix-util",
            "0.1.1",
            &["toybox"],
            &[("bin/hwclock", Node::File(b"real hwclock", 0o755))],
        );
        install(&rootfs, &db, &ju);
        assert!(
            !manifest_has(&db, "toybox", "bin/sort"),
            "replaces took it over"
        );

        // Reinstall and upgrade toybox: nothing of the replacers may move.
        install(&rootfs, &db, &tb1);
        let tb2 = build_jpkg_tree(tmp.path(), "toybox", "0.8.11-r15", &[], &toybox_nodes);
        install(&rootfs, &db, &tb2);

        assert_eq!(
            fs::read_link(rootfs.join("bin/sort")).unwrap(),
            Path::new("uutils")
        );
        assert_eq!(
            fs::read_link(rootfs.join("bin/tar")).unwrap(),
            Path::new("bsdtar")
        );
        assert_eq!(
            fs::read(rootfs.join("bin/hwclock")).unwrap(),
            b"real hwclock"
        );
        assert_eq!(
            fs::read_link(rootfs.join("bin/ping")).unwrap(),
            Path::new("toybox")
        );
        for p in ["bin/sort", "bin/tar", "bin/hwclock"] {
            assert!(
                !manifest_has(&db, "toybox", p),
                "toybox must not re-claim {p}"
            );
        }
        assert!(manifest_has(&db, "toybox", "bin/ping"));
        assert!(manifest_has(&db, "uutils", "bin/sort"));
        assert!(manifest_has(&db, "bsdtar", "bin/tar"));
        assert!(manifest_has(&db, "jonerix-util", "bin/hwclock"));
    }

    /// A host where the replaced package's manifest still claims the shared
    /// path (it was installed last under jpkg <= 2.2.9).  Upgrading it must
    /// neither overwrite nor delete the replacer's file.
    #[test]
    fn upgrade_clean_keeps_paths_owned_by_other_packages() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(rootfs.join("bin")).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();

        // jonerix-util owns bin/hwclock and replaces toybox …
        let ju = build_jpkg_tree(
            tmp.path(),
            "jonerix-util",
            "0.1.1",
            &["toybox"],
            &[("bin/hwclock", Node::File(b"real hwclock", 0o755))],
        );
        install(&rootfs, &db, &ju);
        // … but toybox's old record also lists it (both-claim state).
        let link = |p: &str| FileEntry {
            path: p.to_string(),
            sha256: String::new(),
            size: 0,
            mode: 0o120777,
            symlink_target: Some("toybox".to_string()),
            is_dir: false,
        };
        db.insert(&InstalledPkg {
            metadata: make_metadata("toybox", "0.8.11-r12"),
            files: vec![link("bin/hwclock"), link("bin/ping")],
        })
        .unwrap();
        // … and openrc/fixups share an init script with no replaces at all.
        let rc = build_jpkg_tree(
            tmp.path(),
            "openrc",
            "0.54-r7",
            &[],
            &[("etc/init.d/hwclock", Node::File(b"stock", 0o755))],
        );
        install(&rootfs, &db, &rc);
        let fx = build_jpkg_tree(
            tmp.path(),
            "fixups",
            "1.6.32",
            &[],
            &[
                ("etc/init.d/hwclock", Node::File(b"pi5", 0o755)),
                ("bin/pi5", Node::File(b"x", 0o755)),
            ],
        );
        install(&rootfs, &db, &fx);

        // toybox r15 no longer ships hwclock at all; fixups 1.6.33 drops its copy.
        let tb = build_jpkg_tree(
            tmp.path(),
            "toybox",
            "0.8.11-r15",
            &[],
            &[
                ("bin/toybox", Node::File(b"toybox", 0o755)),
                ("bin/ping", Node::Link("toybox")),
            ],
        );
        install(&rootfs, &db, &tb);
        let fx2 = build_jpkg_tree(
            tmp.path(),
            "fixups",
            "1.6.33",
            &[],
            &[("bin/pi5", Node::File(b"x", 0o755))],
        );
        install(&rootfs, &db, &fx2);

        assert_eq!(
            fs::read(rootfs.join("bin/hwclock")).unwrap(),
            b"real hwclock"
        );
        assert!(
            rootfs.join("etc/init.d/hwclock").exists(),
            "openrc still owns etc/init.d/hwclock"
        );
        assert!(!manifest_has(&db, "toybox", "bin/hwclock"));
    }

    /// C jpkg 1.1.5 manifests store absolute paths.  Under an alternate root,
    /// `rootfs.join("/share/dropme")` used to resolve to the HOST path, so
    /// upgrade-clean missed the file in the target root.
    #[test]
    fn upgrade_clean_of_legacy_manifest_stays_inside_alternate_root() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(rootfs.join("share")).unwrap();
        fs::create_dir_all(rootfs.join("bin")).unwrap();
        fs::write(rootfs.join("share/dropme"), b"old").unwrap();
        fs::write(rootfs.join("bin/foo"), b"old").unwrap();
        let pkg_dir = rootfs.join("var/db/jpkg/installed/droptest");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("metadata.toml"),
            "[package]\nname = \"droptest\"\nversion = \"1.0.0\"\nlicense = \"MIT\"\n",
        )
        .unwrap();
        let sha = "ab".repeat(32);
        fs::write(
            pkg_dir.join("files"),
            format!("{sha} 000644 /share/dropme\n{sha} 000755 /bin/foo\n"),
        )
        .unwrap();

        let db = InstalledDb::open(&rootfs).unwrap();
        let _lock = db.lock().unwrap();
        assert!(
            !fs::read_to_string(pkg_dir.join("files"))
                .unwrap()
                .contains(" /"),
            "taking the lock migrates the legacy manifest"
        );
        let v2 = build_test_jpkg(tmp.path(), "droptest", "2.0.0");
        install(&rootfs, &db, &v2);

        assert!(
            !rootfs.join("share/dropme").exists(),
            "stale legacy path must be removed inside the target root"
        );
        assert_eq!(fs::read(rootfs.join("bin/foo")).unwrap(), b"foo content\n");
    }

    // ── 2.2.11 config files ──────────────────────────────────────────────────

    fn conf_pkg(tmp: &Path, version: &str, nodes: &[(&str, Node<'_>)]) -> PathBuf {
        build_jpkg_tree(tmp, "confpkg", version, &[], nodes)
    }

    fn fresh_root(tmp: &TempDir) -> (PathBuf, InstalledDb) {
        let rootfs = tmp.path().join("rootfs");
        fs::create_dir_all(&rootfs).unwrap();
        let db = InstalledDb::open(&rootfs).unwrap();
        (rootfs, db)
    }

    fn manifest_sha(db: &InstalledDb, pkg: &str, path: &str) -> String {
        db.get(pkg)
            .unwrap()
            .unwrap()
            .files
            .into_iter()
            .find(|e| e.path == path)
            .unwrap()
            .sha256
    }

    #[test]
    fn upgrade_replaces_unchanged_config() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"v2\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists());
    }

    #[test]
    fn upgrade_keeps_changed_config_and_writes_jpkg_new() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v2\n", 0o600))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-new")).unwrap(), b"v2\n");
        assert_eq!(mode_of(&rootfs.join("etc/x.conf.jpkg-new")), 0o600);
        assert!(!manifest_has(&db, "confpkg", "etc/x.conf.jpkg-new"), ".jpkg-new is unowned");
        assert_eq!(
            manifest_sha(&db, "confpkg", "etc/x.conf"),
            crate::util::sha256_file(&rootfs.join("etc/x.conf.jpkg-new")).unwrap(),
            "manifest records the package's copy"
        );
    }

    #[test]
    fn reinstall_keeps_changed_config_without_jpkg_new() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let v1 = conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]);
        install(&rootfs, &db, &v1);
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &v1);
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists());
    }

    #[test]
    fn upgrade_restores_deleted_config_and_keeps_admin_symlink() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let n1 = [("etc/a.conf", Node::File(b"a1\n", 0o644)), ("etc/b.conf", Node::File(b"b1\n", 0o644))];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &n1));
        fs::remove_file(rootfs.join("etc/a.conf")).unwrap();
        fs::remove_file(rootfs.join("etc/b.conf")).unwrap();
        symlink("/data/b.conf", rootfs.join("etc/b.conf")).unwrap();
        let n2 = [("etc/a.conf", Node::File(b"a2\n", 0o644)), ("etc/b.conf", Node::File(b"b2\n", 0o644))];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &n2));
        assert_eq!(fs::read(rootfs.join("etc/a.conf")).unwrap(), b"a2\n");
        assert_eq!(fs::read_link(rootfs.join("etc/b.conf")).unwrap(), Path::new("/data/b.conf"));
        assert_eq!(fs::read(rootfs.join("etc/b.conf.jpkg-new")).unwrap(), b"b2\n");
    }

    #[test]
    fn upgrade_applies_package_mode_to_unchanged_config() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"same\n", 0o644))]));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"same\n", 0o600))]));
        assert_eq!(mode_of(&rootfs.join("etc/x.conf")), 0o600);
    }

    /// dhcpcd r10 -> r11: the new version stops shipping /etc/dhcpcd.conf.
    #[test]
    fn upgrade_clean_keeps_changed_config_and_drops_unchanged_one() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let n1 = [
            ("bin/d", Node::File(b"d\n", 0o755)),
            ("etc/edited.conf", Node::File(b"stock\n", 0o644)),
            ("etc/pristine.conf", Node::File(b"stock\n", 0o644)),
        ];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &n1));
        fs::write(rootfs.join("etc/edited.conf"), b"nohook resolv.conf\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("bin/d", Node::File(b"d\n", 0o755))]));
        assert_eq!(fs::read(rootfs.join("etc/edited.conf")).unwrap(), b"nohook resolv.conf\n");
        assert!(!manifest_has(&db, "confpkg", "etc/edited.conf"), "now the admin's");
        assert!(!rootfs.join("etc/pristine.conf").exists());
    }

    #[test]
    fn first_install_adopts_identical_and_keeps_different_preexisting_config() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        fs::create_dir_all(rootfs.join("etc")).unwrap();
        fs::write(rootfs.join("etc/same.conf"), b"pkg\n").unwrap();
        fs::write(rootfs.join("etc/mine.conf"), b"seeded and edited\n").unwrap();
        let n = [("etc/same.conf", Node::File(b"pkg\n", 0o644)), ("etc/mine.conf", Node::File(b"pkg\n", 0o644))];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &n));
        assert!(!rootfs.join("etc/same.conf.jpkg-new").exists());
        assert_eq!(fs::read(rootfs.join("etc/mine.conf")).unwrap(), b"seeded and edited\n");
        assert_eq!(fs::read(rootfs.join("etc/mine.conf.jpkg-new")).unwrap(), b"pkg\n");
    }

    #[test]
    fn takeover_replaces_previous_owners_unchanged_config() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let a = build_jpkg_tree(tmp.path(), "olda", "1", &[], &[("etc/x.conf", Node::File(b"a\n", 0o644))]);
        install(&rootfs, &db, &a);
        let b = build_jpkg_tree(tmp.path(), "newb", "1", &["olda"], &[("etc/x.conf", Node::File(b"b\n", 0o644))]);
        install(&rootfs, &db, &b);
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"b\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists());
        assert!(!manifest_has(&db, "olda", "etc/x.conf"));
    }

    #[test]
    fn init_d_and_cron_d_files_are_always_replaced() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let n1 = [("etc/init.d/svc", Node::File(b"v1\n", 0o755)), ("etc/cron.d/job", Node::File(b"v1\n", 0o644))];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &n1));
        fs::write(rootfs.join("etc/init.d/svc"), b"hacked\n").unwrap();
        fs::write(rootfs.join("etc/cron.d/job"), b"hacked\n").unwrap();
        let n2 = [("etc/init.d/svc", Node::File(b"v2\n", 0o755)), ("etc/cron.d/job", Node::File(b"v2\n", 0o644))];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &n2));
        assert_eq!(fs::read(rootfs.join("etc/init.d/svc")).unwrap(), b"v2\n");
        assert_eq!(fs::read(rootfs.join("etc/cron.d/job")).unwrap(), b"v2\n");
        assert!(!rootfs.join("etc/init.d/svc.jpkg-new").exists());
        assert!(!rootfs.join("etc/cron.d/job.jpkg-new").exists());
    }

    #[test]
    fn package_symlink_becoming_config_file_is_installed() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let n1 = [("etc/x.conf", Node::Link("x.conf.default")), ("etc/x.conf.default", Node::File(b"d\n", 0o644))];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &n1));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        let m = rootfs.join("etc/x.conf").symlink_metadata().unwrap();
        assert!(m.file_type().is_file(), "the package's own old link is pristine");
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"v2\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists());
    }

    #[test]
    fn stale_jpkg_new_is_removed_once_config_matches_package() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-new")).unwrap(), b"v2\n");
        // The admin adopts the packaged version but leaves the .jpkg-new.
        fs::copy(rootfs.join("etc/x.conf.jpkg-new"), rootfs.join("etc/x.conf")).unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "3", &[("etc/x.conf", Node::File(b"v3\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"v3\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists(), "package content: removed");
    }

    #[test]
    fn stale_jpkg_new_holding_admin_edits_is_kept() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf.jpkg-new"), b"the admin's draft\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-new")).unwrap(), b"the admin's draft\n");
    }

    fn dir_then_link(tmp: &Path, version: &str, as_link: bool) -> PathBuf {
        if as_link {
            conf_pkg(tmp, version, &[
                ("share/foo.d/a.conf", Node::File(b"a\n", 0o644)),
                ("etc/foo.d", Node::Link("../share/foo.d")),
            ])
        } else {
            conf_pkg(tmp, version, &[("etc/foo.d", Node::Dir(0o755)), ("etc/foo.d/a.conf", Node::File(b"a\n", 0o644))])
        }
    }

    #[test]
    fn dir_to_symlink_refuses_when_owned_config_inside_is_changed() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &dir_then_link(tmp.path(), "1", false));
        fs::write(rootfs.join("etc/foo.d/a.conf"), b"mine\n").unwrap();
        let err = extract_and_register(&JpkgArchive::open(&dir_then_link(tmp.path(), "2", true)).unwrap(), &rootfs, &db)
            .expect_err("a changed config file must not be blasted with its directory");
        assert!(matches!(err, InstallError::UpgradeForeignFiles { .. }), "{err:?}");
        assert!(err.to_string().contains("a.conf"), "{err}");
        assert_eq!(fs::read(rootfs.join("etc/foo.d/a.conf")).unwrap(), b"mine\n");
        assert_eq!(db.get("confpkg").unwrap().unwrap().metadata.package.version.as_deref(), Some("1"));
    }

    #[test]
    fn dir_to_symlink_proceeds_when_owned_config_inside_is_unchanged() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &dir_then_link(tmp.path(), "1", false));
        install(&rootfs, &db, &dir_then_link(tmp.path(), "2", true));
        assert!(rootfs.join("etc/foo.d").symlink_metadata().unwrap().file_type().is_symlink());
    }

    /// Step 5c only edits staging, so when upgrade-clean refuses, the root
    /// holds no .jpkg-new and the admin's file is as it was.
    #[test]
    fn config_step_failure_leaves_root_untouched() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let v1 = conf_pkg(tmp.path(), "1", &[
            ("etc/x.conf", Node::File(b"v1\n", 0o644)),
            ("etc/foo.d", Node::Dir(0o755)),
            ("etc/foo.d/a.conf", Node::File(b"a\n", 0o644)),
        ]);
        install(&rootfs, &db, &v1);
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        fs::write(rootfs.join("etc/foo.d/dropped-by-hand"), b"?\n").unwrap();
        let v2 = conf_pkg(tmp.path(), "2", &[
            ("etc/x.conf", Node::File(b"v2\n", 0o644)),
            ("share/foo.d/a.conf", Node::File(b"a\n", 0o644)),
            ("etc/foo.d", Node::Link("../share/foo.d")),
        ]);
        extract_and_register(&JpkgArchive::open(&v2).unwrap(), &rootfs, &db).expect_err("foreign file");
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists(), "nothing written to the root");
    }

    /// Opening the FIFO would block this test forever.
    #[test]
    fn fifo_at_config_path_is_kept_and_never_opened() {
        use std::os::unix::fs::FileTypeExt;
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::remove_file(rootfs.join("etc/x.conf")).unwrap();
        nix::unistd::mkfifo(&rootfs.join("etc/x.conf"), nix::sys::stat::Mode::from_bits_truncate(0o644)).unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        assert!(rootfs.join("etc/x.conf").symlink_metadata().unwrap().file_type().is_fifo());
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-new")).unwrap(), b"v2\n");
    }

    #[test]
    fn run_hook_exports_jpkg_conffiles() {
        let tmp = TempDir::new().unwrap();
        let out = tmp.path().join("seen");
        let body = format!("printf '%s' \"${{JPKG_CONFFILES:-unset}}\" > '{}'", out.display());
        let st = run_hook(tmp.path(), &body).unwrap();
        assert!(st.success());
        assert_eq!(fs::read_to_string(&out).unwrap(), "1");
    }

    #[test]
    fn conflict_keeps_locally_changed_copy_of_other_owners_config() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "pkga", "1", &[], &[("etc/x.conf", Node::File(b"a\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "pkgb", "1", &[], &[("etc/x.conf", Node::File(b"b\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-new")).unwrap(), b"b\n");
    }

    fn file_then_link(tmp: &Path, version: &str, as_link: bool) -> PathBuf {
        if as_link {
            conf_pkg(tmp, version, &[("etc/x.d/main", Node::File(b"v2\n", 0o644)), ("etc/x.conf", Node::Link("x.d/main"))])
        } else {
            conf_pkg(tmp, version, &[("etc/x.conf", Node::File(b"v1\n", 0o644))])
        }
    }

    #[test]
    fn changed_config_becoming_symlink_is_saved_as_jpkg_save() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &file_then_link(tmp.path(), "1", false));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &file_then_link(tmp.path(), "2", true));
        assert!(rootfs.join("etc/x.conf").symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-save")).unwrap(), b"mine\n");
    }

    #[test]
    fn unchanged_config_becoming_symlink_leaves_no_jpkg_save() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &file_then_link(tmp.path(), "1", false));
        install(&rootfs, &db, &file_then_link(tmp.path(), "2", true));
        assert!(rootfs.join("etc/x.conf").symlink_metadata().unwrap().file_type().is_symlink());
        assert!(!rootfs.join("etc/x.conf.jpkg-save").exists());
    }

    #[test]
    fn changed_config_becoming_symlink_never_overwrites_a_jpkg_save() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let v1 = conf_pkg(tmp.path(), "1", &[
            ("etc/x.conf", Node::File(b"v1\n", 0o644)),
            ("bin/dropped-in-v2", Node::File(b"x\n", 0o755)),
        ]);
        install(&rootfs, &db, &v1);
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        fs::write(rootfs.join("etc/x.conf.jpkg-save"), b"an older save\n").unwrap();
        extract_and_register(&JpkgArchive::open(&file_then_link(tmp.path(), "2", true)).unwrap(), &rootfs, &db)
            .expect_err("must not overwrite an existing .jpkg-save");
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-save")).unwrap(), b"an older save\n");
        assert!(rootfs.join("bin/dropped-in-v2").exists(), "refused in 5c, before upgrade-clean");
        assert_eq!(db.get("confpkg").unwrap().unwrap().metadata.package.version.as_deref(), Some("1"));
    }

    /// Review S1: the admin is merging inside the earlier offer.
    #[test]
    fn admin_work_in_jpkg_new_is_never_overwritten() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf.jpkg-new"), b"v2 half merged\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "3", &[("etc/x.conf", Node::File(b"v3\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-new")).unwrap(), b"v2 half merged\n");
        assert_eq!(db.get("confpkg").unwrap().unwrap().metadata.package.version.as_deref(), Some("3"));
    }

    /// An untouched earlier offer is package content: replaced by the newer one.
    #[test]
    fn earlier_offer_is_replaced_by_the_newer_one() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "3", &[("etc/x.conf", Node::File(b"v3\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-new")).unwrap(), b"v3\n");
    }

    /// A non-empty directory at the offer slot used to abort the upgrade
    /// half-way (EISDIR in install_files).
    #[test]
    fn directory_or_link_at_jpkg_new_is_left_alone_and_upgrade_completes() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let n1 = [("etc/a.conf", Node::File(b"a1\n", 0o644)), ("etc/b.conf", Node::File(b"b1\n", 0o644))];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &n1));
        fs::write(rootfs.join("etc/a.conf"), b"mine\n").unwrap();
        fs::write(rootfs.join("etc/b.conf"), b"mine\n").unwrap();
        fs::create_dir(rootfs.join("etc/a.conf.jpkg-new")).unwrap();
        fs::write(rootfs.join("etc/a.conf.jpkg-new/notes"), b"keep\n").unwrap();
        symlink("/data/b.draft", rootfs.join("etc/b.conf.jpkg-new")).unwrap();
        let n2 = [("etc/a.conf", Node::File(b"a2\n", 0o644)), ("etc/b.conf", Node::File(b"b2\n", 0o644))];
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &n2));
        assert_eq!(fs::read(rootfs.join("etc/a.conf.jpkg-new/notes")).unwrap(), b"keep\n");
        assert_eq!(fs::read_link(rootfs.join("etc/b.conf.jpkg-new")).unwrap(), Path::new("/data/b.draft"));
        assert_eq!(fs::read(rootfs.join("etc/a.conf")).unwrap(), b"mine\n");
        assert_eq!(db.get("confpkg").unwrap().unwrap().metadata.package.version.as_deref(), Some("2"));
    }

    fn dir_then_file(tmp: &Path, version: &str, as_file: bool) -> PathBuf {
        if as_file {
            conf_pkg(tmp, version, &[("etc/x.conf", Node::File(b"file\n", 0o644))])
        } else {
            conf_pkg(tmp, version, &[("etc/x.conf", Node::Dir(0o755)), ("etc/x.conf/a", Node::File(b"a\n", 0o644))])
        }
    }

    /// Review regression: 2.2.10 turned a package's own directory into a
    /// config file; 2.2.11 at first called the directory "changed".
    #[test]
    fn package_dir_becoming_config_file_is_installed() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &dir_then_file(tmp.path(), "1", false));
        install(&rootfs, &db, &dir_then_file(tmp.path(), "2", true));
        assert!(rootfs.join("etc/x.conf").symlink_metadata().unwrap().file_type().is_file());
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"file\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists());
    }

    #[test]
    fn package_dir_holding_admin_files_is_kept_when_it_becomes_a_config_file() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &dir_then_file(tmp.path(), "1", false));
        fs::write(rootfs.join("etc/x.conf/local"), b"mine\n").unwrap();
        install(&rootfs, &db, &dir_then_file(tmp.path(), "2", true));
        assert_eq!(fs::read(rootfs.join("etc/x.conf/local")).unwrap(), b"mine\n");
        assert!(!rootfs.join("etc/x.conf/a").exists(), "the package's own file went");
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-new")).unwrap(), b"file\n");
    }

    #[test]
    fn config_file_becoming_directory_removes_pristine_and_saves_changed() {
        for changed in [false, true] {
            let tmp = TempDir::new().unwrap();
            let (rootfs, db) = fresh_root(&tmp);
            let _lock = db.lock().unwrap();
            install(&rootfs, &db, &dir_then_file(tmp.path(), "1", true));
            if changed {
                fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
            }
            install(&rootfs, &db, &dir_then_file(tmp.path(), "2", false));
            assert_eq!(fs::read(rootfs.join("etc/x.conf/a")).unwrap(), b"a\n", "changed={changed}");
            assert_eq!(
                fs::read(rootfs.join("etc/x.conf.jpkg-save")).ok(),
                changed.then(|| b"mine\n".to_vec()),
                "changed={changed}"
            );
        }
    }

    /// A package placing a symlink where another ships a config file (as
    /// ca-certificates did over libressl's cert.pem before the trust store
    /// became package data).
    #[test]
    fn other_packages_changed_config_replaced_by_symlink_is_saved() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let ssl = build_jpkg_tree(tmp.path(), "ssl", "1", &[], &[("etc/pki/default.pem", Node::File(b"bundle\n", 0o644))]);
        install(&rootfs, &db, &ssl);
        fs::write(rootfs.join("etc/pki/default.pem"), b"bundle + private CA\n").unwrap();
        let ca = build_jpkg_tree(tmp.path(), "ca", "1", &["ssl"], &[
            ("etc/pki/certs/ca.crt", Node::File(b"ca\n", 0o644)),
            ("etc/pki/default.pem", Node::Link("certs/ca.crt")),
        ]);
        install(&rootfs, &db, &ca);
        assert_eq!(fs::read_link(rootfs.join("etc/pki/default.pem")).unwrap(), Path::new("certs/ca.crt"));
        assert_eq!(fs::read(rootfs.join("etc/pki/default.pem.jpkg-save")).unwrap(), b"bundle + private CA\n");
    }

    #[test]
    fn unchanged_other_owners_config_replaced_by_symlink_leaves_no_save() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let ssl = build_jpkg_tree(tmp.path(), "ssl", "1", &[], &[("etc/pki/default.pem", Node::File(b"bundle\n", 0o644))]);
        install(&rootfs, &db, &ssl);
        let ca = build_jpkg_tree(tmp.path(), "ca", "1", &["ssl"], &[
            ("etc/pki/certs/ca.crt", Node::File(b"ca\n", 0o644)),
            ("etc/pki/default.pem", Node::Link("certs/ca.crt")),
        ]);
        install(&rootfs, &db, &ca);
        install(&rootfs, &db, &ca);
        assert!(rootfs.join("etc/pki/default.pem").symlink_metadata().unwrap().file_type().is_symlink());
        assert!(!rootfs.join("etc/pki/default.pem.jpkg-save").exists());
    }

    /// Nothing recorded at the path: the admin's link is theirs, like a
    /// pre-existing different config file.
    #[test]
    fn admin_symlink_where_package_first_places_a_symlink_is_kept() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        fs::create_dir_all(rootfs.join("etc")).unwrap();
        symlink("/data/x.conf", rootfs.join("etc/x.conf")).unwrap();
        install(&rootfs, &db, &file_then_link(tmp.path(), "1", true));
        assert_eq!(fs::read_link(rootfs.join("etc/x.conf")).unwrap(), Path::new("/data/x.conf"));
        assert!(rootfs.join("etc/x.conf.jpkg-save").symlink_metadata().is_err());
        assert!(manifest_has(&db, "confpkg", "etc/x.conf"), "the manifest still records the package's link");
    }

    fn ca_pkg(tmp: &Path, version: &str) -> PathBuf {
        build_jpkg_tree(tmp, "ca", version, &[], &[
            ("etc/ssl/certs/ca-certificates.crt", Node::File(version.as_bytes(), 0o644)),
            ("etc/pki/default.pem", Node::Link("certs/ca-certificates.crt")),
        ])
    }

    /// Review d0: an admin pointing cert.pem at their own bundle used to be
    /// saved aside on every upgrade, and refused (aborting the whole run)
    /// on the second.  Same kind of object, so it is kept like a config file.
    #[test]
    fn admin_retargeted_package_link_is_kept_on_every_upgrade() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &ca_pkg(tmp.path(), "1"));
        fs::remove_file(rootfs.join("etc/pki/default.pem")).unwrap();
        symlink("local/corp.pem", rootfs.join("etc/pki/default.pem")).unwrap();
        let v3 = ca_pkg(tmp.path(), "3");
        for (v, pkg) in [("2", ca_pkg(tmp.path(), "2")), ("3", v3.clone()), ("3 again", v3)] {
            install(&rootfs, &db, &pkg);
            assert_eq!(fs::read_link(rootfs.join("etc/pki/default.pem")).unwrap(), Path::new("local/corp.pem"), "v{v}");
            assert!(rootfs.join("etc/pki/default.pem.jpkg-save").symlink_metadata().is_err(), "v{v}");
        }
        let r = crate::cmd::verify::verify_package_for_tests(&db.get("ca").unwrap().unwrap(), &rootfs);
        assert_eq!(r, (0, 1), "reported as changed, not a failure");
    }

    /// An unowned link identical to the one the package ships, over another
    /// package's recorded file (as old minimal images left cert.pem).
    /// Identical: nothing to save.
    #[test]
    fn unowned_link_identical_to_the_package_link_is_adopted() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "ssl", "1", &[], &[("etc/pki/default.pem", Node::File(b"bundle\n", 0o644))]));
        fs::remove_file(rootfs.join("etc/pki/default.pem")).unwrap();
        symlink("certs/ca-certificates.crt", rootfs.join("etc/pki/default.pem")).unwrap();
        install(&rootfs, &db, &ca_pkg(tmp.path(), "1"));
        assert_eq!(fs::read_link(rootfs.join("etc/pki/default.pem")).unwrap(), Path::new("certs/ca-certificates.crt"));
        assert!(rootfs.join("etc/pki/default.pem.jpkg-save").symlink_metadata().is_err());
    }

    /// Review c18/c20: the trust store is package data.  An edited or
    /// image-seeded bundle is replaced, and nothing lands in etc/ssl/certs.
    #[test]
    fn trust_store_bundle_is_always_replaced_and_gets_no_offer() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        fs::create_dir_all(rootfs.join("etc/ssl/certs")).unwrap();
        fs::write(rootfs.join("etc/ssl/certs/ca-certificates.crt"), b"curl.se extract from an old image\n").unwrap();
        install(&rootfs, &db, &ca_pkg(tmp.path(), "1"));
        assert_eq!(fs::read(rootfs.join("etc/ssl/certs/ca-certificates.crt")).unwrap(), b"1");
        fs::write(rootfs.join("etc/ssl/certs/ca-certificates.crt"), b"edited\n").unwrap();
        install(&rootfs, &db, &ca_pkg(tmp.path(), "2"));
        assert_eq!(fs::read(rootfs.join("etc/ssl/certs/ca-certificates.crt")).unwrap(), b"2");
        let names: Vec<_> = fs::read_dir(rootfs.join("etc/ssl/certs")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("ca-certificates.crt")]);
    }

    /// Review c1: a package directory over the admin's link that does not
    /// lead to a directory used to fail after upgrade-clean (EEXIST).
    #[test]
    fn package_directory_over_an_unfollowable_admin_link_saves_it() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &dir_then_file(tmp.path(), "1", true));
        fs::remove_file(rootfs.join("etc/x.conf")).unwrap();
        symlink("../data/x.conf", rootfs.join("etc/x.conf")).unwrap();
        install(&rootfs, &db, &dir_then_file(tmp.path(), "2", false));
        assert_eq!(fs::read(rootfs.join("etc/x.conf/a")).unwrap(), b"a\n");
        assert_eq!(fs::read_link(rootfs.join("etc/x.conf.jpkg-save")).unwrap(), Path::new("../data/x.conf"));
    }

    /// Review c2/c4: an untouched offer for a file the admin then adopted.
    #[test]
    fn stale_offer_goes_when_an_upgrade_drops_its_pristine_file() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let bin = ("bin/d", Node::File(b"d\n", 0o755));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[bin, ("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[bin, ("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        fs::copy(rootfs.join("etc/x.conf.jpkg-new"), rootfs.join("etc/x.conf")).unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "3", &[bin]));
        assert!(!rootfs.join("etc/x.conf").exists(), "pristine: removed");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists(), "and its offer with it");
    }

    #[test]
    fn stale_offer_does_not_block_dir_to_symlink_or_dir_to_file() {
        for to_link in [true, false] {
            let tmp = TempDir::new().unwrap();
            let (rootfs, db) = fresh_root(&tmp);
            let _lock = db.lock().unwrap();
            let v = |ver: &str, a: &'static [u8]| {
                conf_pkg(tmp.path(), ver, &[("etc/foo.d", Node::Dir(0o755)), ("etc/foo.d/a.conf", Node::File(a, 0o644))])
            };
            install(&rootfs, &db, &v("1", b"a1\n"));
            fs::write(rootfs.join("etc/foo.d/a.conf"), b"mine\n").unwrap();
            install(&rootfs, &db, &v("2", b"a2\n"));
            fs::copy(rootfs.join("etc/foo.d/a.conf.jpkg-new"), rootfs.join("etc/foo.d/a.conf")).unwrap();
            let v3 = if to_link {
                dir_then_link(tmp.path(), "3", true)
            } else {
                conf_pkg(tmp.path(), "3", &[("etc/foo.d", Node::File(b"now a file\n", 0o644))])
            };
            install(&rootfs, &db, &v3);
            let m = rootfs.join("etc/foo.d").symlink_metadata().unwrap();
            if to_link {
                assert!(m.file_type().is_symlink());
            } else {
                assert!(m.file_type().is_file(), "installed, not offered");
                assert!(!rootfs.join("etc/foo.d.jpkg-new").exists());
            }
        }
    }

    /// Review c0: config files are written last and atomically, so a step-7
    /// failure leaves the recorded copies and a fixed release replaces them.
    #[test]
    fn failed_install_leaves_recorded_config_and_the_retry_replaces_it() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "oth", "1", &[], &[("share/ku/t/o", Node::File(b"o\n", 0o644))]));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/ku.conf", Node::File(b"stock-1\n", 0o644))]));
        let broken = conf_pkg(tmp.path(), "2", &[
            ("etc/ku.conf", Node::File(b"stock-2\n", 0o644)),
            ("share/ku/t", Node::File(b"collides with oth's directory\n", 0o644)),
        ]);
        extract_and_register(&JpkgArchive::open(&broken).unwrap(), &rootfs, &db).expect_err("EISDIR at share/ku/t");
        assert_eq!(fs::read(rootfs.join("etc/ku.conf")).unwrap(), b"stock-1\n", "config not yet written");
        install(&rootfs, &db, &conf_pkg(tmp.path(), "3", &[("etc/ku.conf", Node::File(b"stock-3\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/ku.conf")).unwrap(), b"stock-3\n");
        assert!(!rootfs.join("etc/ku.conf.jpkg-new").exists());
        assert!(!rootfs.join("etc/ku.conf.jpkg-tmp").exists());
    }

    #[test]
    fn config_files_keep_mode_and_leave_no_temporary_file() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/s.conf", Node::File(b"s\n", 0o4750))]));
        assert_eq!(mode_of(&rootfs.join("etc/s.conf")), 0o4750);
        let names: Vec<_> = fs::read_dir(rootfs.join("etc")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("s.conf")]);
    }

    #[test]
    fn package_shipping_the_jpkg_save_path_is_refused_before_any_write() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &file_then_link(tmp.path(), "1", false));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        let v2 = conf_pkg(tmp.path(), "2", &[
            ("etc/x.d/main", Node::File(b"v2\n", 0o644)),
            ("etc/x.conf", Node::Link("x.d/main")),
            ("etc/x.conf.jpkg-save", Node::File(b"from the package\n", 0o644)),
        ]);
        let err = extract_and_register(&JpkgArchive::open(&v2).unwrap(), &rootfs, &db)
            .expect_err("its own .jpkg-save would overwrite the saved file");
        assert!(err.to_string().contains("jpkg-save"), "{err}");
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-save").exists());
        assert!(!rootfs.join("etc/x.d").exists(), "nothing written to the root");
    }

    /// dhcpcd r11 style drop of a kept file: an offer left by an earlier
    /// upgrade would otherwise stay forever.
    #[test]
    fn dropping_a_kept_config_also_drops_its_stale_offer() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let bin = ("bin/d", Node::File(b"d\n", 0o755));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[bin, ("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[bin, ("etc/x.conf", Node::File(b"v2\n", 0o644))]));
        assert!(rootfs.join("etc/x.conf.jpkg-new").exists());
        install(&rootfs, &db, &conf_pkg(tmp.path(), "3", &[bin]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists());
    }

    #[test]
    fn dir_to_symlink_refuses_when_owned_config_was_replaced_by_a_directory() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &dir_then_link(tmp.path(), "1", false));
        fs::remove_file(rootfs.join("etc/foo.d/a.conf")).unwrap();
        fs::create_dir(rootfs.join("etc/foo.d/a.conf")).unwrap();
        let err = extract_and_register(&JpkgArchive::open(&dir_then_link(tmp.path(), "2", true)).unwrap(), &rootfs, &db)
            .expect_err("the admin's directory must not be blasted");
        assert!(matches!(err, InstallError::UpgradeForeignFiles { .. }), "{err:?}");
        assert!(rootfs.join("etc/foo.d/a.conf").is_dir());
    }

    #[test]
    fn upgrade_clean_keeps_a_link_the_admin_replaced_with_a_file() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &file_then_link(tmp.path(), "1", true));
        fs::remove_file(rootfs.join("etc/x.conf")).unwrap();
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.d/main", Node::File(b"v2\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
    }

    /// Step 5c on its own (7a re-checks the slot too, which hides a 5c bug
    /// end to end): no offer is staged over the admin's work in the slot.
    #[test]
    fn plan_does_not_offer_over_admin_work_in_jpkg_new() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        fs::write(rootfs.join("etc/x.conf.jpkg-new"), b"merging\n").unwrap();
        let stage = tmp.path().join("stage");
        fs::create_dir_all(stage.join("etc")).unwrap();
        fs::write(stage.join("etc/x.conf"), b"v2\n").unwrap();
        let files = build_manifest(&stage).unwrap();
        let old = db.get("confpkg").unwrap().unwrap();
        let others = db.path_owners(None, Some("confpkg")).unwrap();
        let plan = plan_config_files(&rootfs, &stage, "confpkg", "2", &[], Some(&old), &files, &others).unwrap();
        assert!(plan.install.is_empty() && plan.offer.is_empty());
        assert!(!stage.join("etc/x.conf").exists() && !stage.join("etc/x.conf.jpkg-new").exists());
    }

    /// 5c and 6b each apply Displace::Keep on their own (6b repeats it for a
    /// link retargeted after planning), which hides either one end to end.
    fn retarget_plan(tmp: &TempDir) -> (PathBuf, InstalledDb, PathBuf, Vec<FileEntry>, InstalledPkg, Ownership) {
        let (rootfs, db) = fresh_root(tmp);
        install(&rootfs, &db, &ca_pkg(tmp.path(), "1"));
        let stage = tmp.path().join("stage");
        fs::create_dir_all(stage.join("etc/ssl/certs")).unwrap();
        fs::create_dir_all(stage.join("etc/pki")).unwrap();
        fs::write(stage.join("etc/ssl/certs/ca-certificates.crt"), b"2").unwrap();
        symlink("certs/ca-certificates.crt", stage.join("etc/pki/default.pem")).unwrap();
        let files = build_manifest(&stage).unwrap();
        let old = db.get("ca").unwrap().unwrap();
        let others = db.path_owners(None, Some("ca")).unwrap();
        (rootfs, db, stage, files, old, others)
    }

    #[test]
    fn plan_drops_the_package_link_where_the_admin_retargeted_it() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db, stage, files, old, others) = retarget_plan(&tmp);
        let _lock = db.lock().unwrap();
        fs::remove_file(rootfs.join("etc/pki/default.pem")).unwrap();
        symlink("local/corp.pem", rootfs.join("etc/pki/default.pem")).unwrap();
        plan_config_files(&rootfs, &stage, "ca", "2", &[], Some(&old), &files, &others).unwrap();
        assert!(stage.join("etc/pki/default.pem").symlink_metadata().is_err(), "5c dropped the staged link");
    }

    #[test]
    fn displace_step_keeps_a_link_retargeted_after_planning() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db, stage, files, old, others) = retarget_plan(&tmp);
        let _lock = db.lock().unwrap();
        let plan = plan_config_files(&rootfs, &stage, "ca", "2", &[], Some(&old), &files, &others).unwrap();
        assert!(stage.join("etc/pki/default.pem").symlink_metadata().is_ok(), "pristine at 5c");
        fs::remove_file(rootfs.join("etc/pki/default.pem")).unwrap();
        symlink("local/corp.pem", rootfs.join("etc/pki/default.pem")).unwrap();
        displace_config_paths(&rootfs, &stage, "ca", "2", &plan).unwrap();
        assert!(stage.join("etc/pki/default.pem").symlink_metadata().is_err(), "6b dropped it");
    }

    /// Step 7a: an edit made after 5c (while upgrade-clean runs) is kept.
    #[test]
    fn recheck_keeps_an_edit_made_after_planning() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[
            ("etc/a.conf", Node::File(b"a1\n", 0o644)),
            ("etc/b.conf", Node::File(b"b1\n", 0o644)),
        ]));
        fs::write(rootfs.join("etc/b.conf"), b"mine\n").unwrap();
        let stage = tmp.path().join("stage");
        fs::create_dir_all(stage.join("etc")).unwrap();
        fs::write(stage.join("etc/a.conf"), b"a2\n").unwrap();
        fs::write(stage.join("etc/b.conf"), b"b2\n").unwrap();
        let files = build_manifest(&stage).unwrap();
        let old = db.get("confpkg").unwrap().unwrap();
        let others = db.path_owners(None, Some("confpkg")).unwrap();
        let mut plan = plan_config_files(&rootfs, &stage, "confpkg", "2", &[], Some(&old), &files, &others).unwrap();
        assert_eq!(plan.install.len(), 1);
        assert_eq!(plan.offer.len(), 1);
        // Now, between planning and writing, the admin edits a.conf and
        // starts merging in the b.conf offer slot.
        fs::write(rootfs.join("etc/a.conf"), b"edited meanwhile\n").unwrap();
        fs::write(rootfs.join("etc/b.conf.jpkg-new"), b"merging\n").unwrap();
        recheck_config_files(&rootfs, &stage, "confpkg", &mut plan).unwrap();
        assert!(plan.install.is_empty());
        assert_eq!(plan.offer.len(), 1, "a.conf is offered instead");
        assert_eq!(fs::read(stage.join("etc/a.conf.jpkg-new")).unwrap(), b"a2\n");
        assert!(!stage.join("etc/a.conf").exists());
        assert!(!stage.join("etc/b.conf.jpkg-new").exists(), "the admin's slot is not written");
    }

    /// Re-review c21/c23: cert.pem is LibreSSL's default CA file, part of
    /// the trust store: an admin's (or an old image's) link there is
    /// replaced on every update, as in 2.2.10.
    #[test]
    fn trust_store_link_is_replaced_on_every_update() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let real = build_jpkg_tree(tmp.path(), "realca", "1", &[], &[
            ("etc/ssl/certs/ca-certificates.crt", Node::File(b"bundle\n", 0o644)),
            ("etc/ssl/cert.pem", Node::Link("certs/ca-certificates.crt")),
        ]);
        install(&rootfs, &db, &real);
        fs::remove_file(rootfs.join("etc/ssl/cert.pem")).unwrap();
        symlink("/frozen/curl-extract.pem", rootfs.join("etc/ssl/cert.pem")).unwrap();
        install(&rootfs, &db, &real);
        assert_eq!(fs::read_link(rootfs.join("etc/ssl/cert.pem")).unwrap(), Path::new("certs/ca-certificates.crt"));
        assert!(rootfs.join("etc/ssl/cert.pem.jpkg-save").symlink_metadata().is_err());
    }

    /// Re-review c17/c26: jpkg's own link to a directory becoming a real
    /// directory.  5c used to judge the files below through the old link.
    #[test]
    fn own_link_to_dir_becoming_a_real_dir_installs_its_config_files() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[
            ("share/foo/a.conf", Node::File(b"a1\n", 0o644)),
            ("share/foo/b.conf", Node::File(b"b\n", 0o644)),
            ("etc/foo", Node::Link("../share/foo")),
        ]));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[
            ("etc/foo", Node::Dir(0o755)),
            ("etc/foo/a.conf", Node::File(b"a2\n", 0o644)),
            ("etc/foo/b.conf", Node::File(b"b\n", 0o644)),
        ]));
        let m = rootfs.join("etc/foo").symlink_metadata().unwrap();
        assert!(m.is_dir() && !m.file_type().is_symlink(), "a real directory now");
        assert_eq!(fs::read(rootfs.join("etc/foo/a.conf")).unwrap(), b"a2\n");
        assert_eq!(fs::read(rootfs.join("etc/foo/b.conf")).unwrap(), b"b\n");
        assert!(!rootfs.join("etc/foo/a.conf.jpkg-new").exists());
        assert!(!rootfs.join("share/foo/a.conf").exists(), "the old layout is gone");
    }

    /// Re-review c18/c29: under an alternate root an absolute link would be
    /// followed on the host, out of the root.  It is saved instead.
    #[test]
    fn absolute_admin_link_under_alternate_root_is_saved_not_followed() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let host_dir = tmp.path().join("host-side");
        fs::create_dir_all(&host_dir).unwrap();
        fs::create_dir_all(rootfs.join("etc")).unwrap();
        symlink(&host_dir, rootfs.join("etc/x.d")).unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[
            ("etc/x.d", Node::Dir(0o755)),
            ("etc/x.d/a.conf", Node::File(b"a\n", 0o644)),
        ]));
        assert!(fs::read_dir(&host_dir).unwrap().next().is_none(), "nothing written on the host");
        assert!(rootfs.join("etc/x.d.jpkg-save").symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read(rootfs.join("etc/x.d/a.conf")).unwrap(), b"a\n");
    }

    /// Re-review c5/c8: an untouched offer when the package turns the file
    /// into a symlink.
    #[test]
    fn offer_goes_when_its_config_file_becomes_a_symlink() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &file_then_link(tmp.path(), "1", false));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/x.conf", Node::File(b"v1b\n", 0o644))]));
        fs::copy(rootfs.join("etc/x.conf.jpkg-new"), rootfs.join("etc/x.conf")).unwrap();
        install(&rootfs, &db, &file_then_link(tmp.path(), "3", true));
        assert!(rootfs.join("etc/x.conf").symlink_metadata().unwrap().file_type().is_symlink());
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists());
    }

    /// Re-review c33: the admin replaced a config directory with a file; an
    /// upgrade dropping the directory used to abort half-way on ENOTDIR.
    #[test]
    fn admin_file_in_place_of_a_dropped_config_dir_does_not_abort() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let bin = ("bin/d", Node::File(b"d\n", 0o755));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[
            bin,
            ("etc/df.d", Node::Dir(0o755)),
            ("etc/df.d/x.conf", Node::File(b"x\n", 0o644)),
        ]));
        fs::remove_dir_all(rootfs.join("etc/df.d")).unwrap();
        fs::write(rootfs.join("etc/df.d"), b"consolidated\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[bin]));
        assert_eq!(fs::read(rootfs.join("etc/df.d")).unwrap(), b"consolidated\n");
        assert_eq!(db.get("confpkg").unwrap().unwrap().metadata.package.version.as_deref(), Some("2"));
    }

    /// Re-review c30: something that is not jpkg's scratch at the scratch
    /// name is stepped around, not failed on or deleted.
    #[test]
    fn directory_at_the_scratch_name_is_left_alone() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        fs::create_dir_all(rootfs.join("etc/x.conf.jpkg-tmp")).unwrap();
        fs::write(rootfs.join("etc/x.conf.jpkg-tmp/keep"), b"k\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"v1\n");
        assert_eq!(fs::read(rootfs.join("etc/x.conf.jpkg-tmp/keep")).unwrap(), b"k\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-tmp.1").exists());
    }

    /// Re-review c2: links at config paths are written in the last step
    /// too, so a failure earlier leaves the recorded link.
    #[test]
    fn failed_install_leaves_the_recorded_config_link() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "oth", "1", &[], &[("share/ku/t/o", Node::File(b"o\n", 0o644))]));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/l.conf", Node::Link("a"))]));
        let broken = conf_pkg(tmp.path(), "2", &[
            ("etc/l.conf", Node::Link("b")),
            ("share/ku/t", Node::File(b"collides\n", 0o644)),
        ]);
        extract_and_register(&JpkgArchive::open(&broken).unwrap(), &rootfs, &db).expect_err("EISDIR at share/ku/t");
        assert_eq!(fs::read_link(rootfs.join("etc/l.conf")).unwrap(), Path::new("a"));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "3", &[("etc/l.conf", Node::Link("c"))]));
        assert_eq!(fs::read_link(rootfs.join("etc/l.conf")).unwrap(), Path::new("c"));
    }

    fn own_link_v1(tmp: &Path) -> PathBuf {
        conf_pkg(tmp, "1", &[
            ("share/foo/a.conf", Node::File(b"a1\n", 0o644)),
            ("etc/foo", Node::Link("../share/foo")),
        ])
    }

    fn own_dir_v2(tmp: &Path) -> PathBuf {
        conf_pkg(tmp, "2", &[("etc/foo", Node::Dir(0o755)), ("etc/foo/a.conf", Node::File(b"a2\n", 0o644))])
    }

    /// Ship gate c1: removing the package's own link would drop what else
    /// lives in the directory it leads to out of /etc.  Refused before any
    /// write, naming it.
    #[test]
    fn own_link_to_dir_holding_admin_files_is_refused() {
        for edit_own in [false, true] {
            let tmp = TempDir::new().unwrap();
            let (rootfs, db) = fresh_root(&tmp);
            let _lock = db.lock().unwrap();
            install(&rootfs, &db, &own_link_v1(tmp.path()));
            let what = if edit_own { "etc/foo/a.conf" } else { "etc/foo/local.conf" };
            fs::write(rootfs.join(what), b"mine\n").unwrap();
            let err = extract_and_register(&JpkgArchive::open(&own_dir_v2(tmp.path())).unwrap(), &rootfs, &db)
                .expect_err("must not drop the admin's file out of /etc");
            assert!(matches!(err, InstallError::UpgradeForeignFiles { .. }), "{err:?}");
            assert!(err.to_string().contains(if edit_own { "a.conf" } else { "local.conf" }), "{err}");
            assert!(rootfs.join("etc/foo").symlink_metadata().unwrap().file_type().is_symlink(), "untouched");
            assert_eq!(fs::read(rootfs.join(what)).unwrap(), b"mine\n");
        }
    }

    /// Ship gate c2/c6: another package's link to a directory is followed,
    /// as in 2.2.10, never removed; its owner keeps upgrading.
    #[test]
    fn another_packages_link_to_dir_is_followed_not_removed() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let pa = |v: &str| build_jpkg_tree(tmp.path(), "pa", v, &[], &[
            ("share/foo/a.conf", Node::File(b"a\n", 0o644)),
            ("etc/foo", Node::Link("../share/foo")),
            ("bin/pa", Node::File(v.as_bytes(), 0o755)),
        ]);
        install(&rootfs, &db, &pa("1"));
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "pb", "1", &[], &[
            ("etc/foo", Node::Dir(0o755)),
            ("etc/foo/b.conf", Node::File(b"b\n", 0o644)),
        ]));
        assert!(rootfs.join("etc/foo").symlink_metadata().unwrap().file_type().is_symlink(), "pa's link stays");
        assert_eq!(fs::read(rootfs.join("share/foo/b.conf")).unwrap(), b"b\n", "written through it");
        install(&rootfs, &db, &pa("2"));
        assert_eq!(fs::read(rootfs.join("bin/pa")).unwrap(), b"2");
    }

    /// Ship gate c3: a link the package ships below its own removed link is
    /// decided as new, not judged through the old link and dropped.
    #[test]
    fn link_below_a_removed_own_link_is_installed() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[
            ("share/a/l", Node::Link("t1")),
            ("etc/a", Node::Link("../share/a")),
        ]));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[("etc/a", Node::Dir(0o755)), ("etc/a/l", Node::Link("t2"))]));
        assert!(rootfs.join("etc/a").symlink_metadata().unwrap().is_dir());
        assert_eq!(fs::read_link(rootfs.join("etc/a/l")).unwrap(), Path::new("t2"));
    }

    /// Ship gate c4: an offer's own scratch file is jpkg's too.
    #[test]
    fn offer_scratch_is_cleaned_with_the_file() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let bin = ("bin/d", Node::File(b"d\n", 0o755));
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[bin, ("etc/x.conf", Node::File(b"v1\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf.jpkg-new.jpkg-tmp"), b"half an offer").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "2", &[bin]));
        assert!(!rootfs.join("etc/x.conf.jpkg-new.jpkg-tmp").exists());
    }

    /// 2.2.12: another package's absolute link to a directory under --root
    /// used to be removed (it cannot be followed there), breaking that
    /// package's later upgrades.  Refused before any write.
    #[test]
    fn another_packages_absolute_link_under_root_is_refused() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let pa = |v: &str| build_jpkg_tree(tmp.path(), "pa", v, &[], &[
            ("share/foo/a.conf", Node::File(b"a\n", 0o644)),
            ("etc/foo", Node::Link("/share/foo")),
            ("bin/pa", Node::File(v.as_bytes(), 0o755)),
        ]);
        install(&rootfs, &db, &pa("1"));
        let pb = build_jpkg_tree(tmp.path(), "pb", "1", &[], &[
            ("etc/foo", Node::Dir(0o755)),
            ("etc/foo/b.conf", Node::File(b"b\n", 0o644)),
            ("bin/pb", Node::File(b"pb\n", 0o755)),
        ]);
        let err = extract_and_register(&JpkgArchive::open(&pb).unwrap(), &rootfs, &db)
            .expect_err("must neither remove pa's link nor follow it out of the root");
        assert!(err.to_string().contains("cannot be followed inside this root"), "{err}");
        assert!(rootfs.join("etc/foo").symlink_metadata().unwrap().file_type().is_symlink());
        assert!(!rootfs.join("bin/pb").exists(), "refused before any write");
        install(&rootfs, &db, &pa("2"));
        assert_eq!(fs::read(rootfs.join("bin/pa")).unwrap(), b"2", "pa keeps upgrading");
    }

    fn own_abs_link_v1(tmp: &Path) -> PathBuf {
        conf_pkg(tmp, "1", &[
            ("share/foo/a.conf", Node::File(b"a1\n", 0o644)),
            ("etc/foo", Node::Link("/share/foo")),
        ])
    }

    /// 2.2.12: the package's own absolute link under --root is checked
    /// inside the root (it used to resolve on the host and check nothing).
    #[test]
    fn own_absolute_link_under_root_is_checked_inside_the_root() {
        for admin_file in [true, false] {
            let tmp = TempDir::new().unwrap();
            let (rootfs, db) = fresh_root(&tmp);
            let _lock = db.lock().unwrap();
            install(&rootfs, &db, &own_abs_link_v1(tmp.path()));
            if admin_file {
                fs::write(rootfs.join("share/foo/local.conf"), b"mine\n").unwrap();
            }
            let r = extract_and_register(&JpkgArchive::open(&own_dir_v2(tmp.path())).unwrap(), &rootfs, &db);
            if admin_file {
                let err = r.expect_err("the admin's file would drop out of /etc");
                assert!(err.to_string().contains("local.conf"), "{err}");
                assert!(rootfs.join("etc/foo").symlink_metadata().unwrap().file_type().is_symlink());
            } else {
                r.unwrap();
                assert!(rootfs.join("etc/foo").symlink_metadata().unwrap().is_dir());
                assert_eq!(fs::read(rootfs.join("etc/foo/a.conf")).unwrap(), b"a2\n");
            }
        }
    }

    /// 2.2.12: a link inside the old link's directory that leads to another
    /// directory is walked too: what is behind it would drop out as well.
    #[test]
    fn nested_links_below_an_own_link_are_checked() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[
            ("share/foo/a.conf", Node::File(b"a1\n", 0o644)),
            ("share/foo/sub", Node::Link("../other")),
            ("share/other/y", Node::File(b"y\n", 0o644)),
            ("etc/foo", Node::Link("../share/foo")),
        ]));
        fs::write(rootfs.join("share/other/x"), b"the admin's\n").unwrap();
        let err = extract_and_register(&JpkgArchive::open(&own_dir_v2(tmp.path())).unwrap(), &rootfs, &db)
            .expect_err("share/other/x is reachable through the nested link");
        assert!(err.to_string().contains("share/other/x"), "{err}");
    }

    /// 2.2.12: a relative admin link that climbs out of the root is not
    /// followed (install_files would write on the host); it is saved.
    #[test]
    fn relative_admin_link_climbing_out_of_the_root_is_saved() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        let host_dir = tmp.path().join("host-side");
        fs::create_dir_all(&host_dir).unwrap();
        fs::create_dir_all(rootfs.join("etc")).unwrap();
        symlink("../../host-side", rootfs.join("etc/x.d")).unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[
            ("etc/x.d", Node::Dir(0o755)),
            ("etc/x.d/a.conf", Node::File(b"a\n", 0o644)),
        ]));
        assert!(fs::read_dir(&host_dir).unwrap().next().is_none(), "nothing written outside the root");
        assert!(rootfs.join("etc/x.d.jpkg-save").symlink_metadata().unwrap().file_type().is_symlink());
    }

    /// Review 1.2.4 c1: a link below that points back up ('..', '/') is not
    /// walked, so it does not make the whole tree look foreign.
    #[test]
    fn nested_link_pointing_back_up_is_not_walked() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[
            ("share/foo/a.conf", Node::File(b"a1\n", 0o644)),
            ("share/foo/up", Node::Link("..")),
            ("share/foo/top", Node::Link("/")),
            ("etc/foo", Node::Link("../share/foo")),
        ]));
        install(&rootfs, &db, &own_dir_v2(tmp.path()));
        assert!(rootfs.join("etc/foo").symlink_metadata().unwrap().is_dir());
        assert_eq!(fs::read(rootfs.join("etc/foo/a.conf")).unwrap(), b"a2\n");
    }

    /// Review 1.2.4 c2: a package that replaces the link's owner may take
    /// its absolute link over under --root.
    #[test]
    fn replaces_takeover_of_an_absolute_link_under_root_is_allowed() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "pa", "1", &[], &[
            ("share/foo/a.conf", Node::File(b"a\n", 0o644)),
            ("etc/foo", Node::Link("/share/foo")),
        ]));
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "pb", "1", &["pa"], &[
            ("etc/foo", Node::Dir(0o755)),
            ("etc/foo/b.conf", Node::File(b"b\n", 0o644)),
        ]));
        assert!(rootfs.join("etc/foo").symlink_metadata().unwrap().is_dir());
        assert_eq!(fs::read(rootfs.join("etc/foo/b.conf")).unwrap(), b"b\n");
    }

    #[test]
    fn yielded_config_path_is_not_planned() {
        let tmp = TempDir::new().unwrap();
        let (rootfs, db) = fresh_root(&tmp);
        let _lock = db.lock().unwrap();
        install(&rootfs, &db, &build_jpkg_tree(tmp.path(), "rival", "1", &["confpkg"], &[("etc/x.conf", Node::File(b"rival\n", 0o644))]));
        fs::write(rootfs.join("etc/x.conf"), b"mine\n").unwrap();
        install(&rootfs, &db, &conf_pkg(tmp.path(), "1", &[("etc/x.conf", Node::File(b"pkg\n", 0o644))]));
        assert_eq!(fs::read(rootfs.join("etc/x.conf")).unwrap(), b"mine\n");
        assert!(!rootfs.join("etc/x.conf.jpkg-new").exists(), "a yielded path is never planned");
    }
}
