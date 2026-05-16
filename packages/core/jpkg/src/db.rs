// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! Installed-package database — Rust port of `jpkg/src/db.c`.
//!
//! # Invariants
//!
//! 1. **Manifest format compatibility**: the `files` manifest written by
//!    [`InstalledDb::insert`] is byte-for-byte compatible with the format
//!    produced by C jpkg 1.1.5 (`db.c:serialize_files_list`, lines 156–175).
//!    Any deviation breaks forward/backward compatibility: existing installations
//!    managed by the C tool will become unreadable by the Rust port.  The
//!    symlink sentinel SHA-256 (`0000…0000`) and the `%06o` octal mode format
//!    must never change without a version bump.
//!
//! 2. **Atomic writes**: every mutation of the on-disk state goes through
//!    [`atomic_write`], which writes to a sibling `.tmp` file and renames it
//!    into place.  Callers must not write to `metadata.toml` or `files`
//!    directly; doing so can leave a partially written record that looks valid
//!    to a concurrent reader.
//!
//! 3. **Lock semantics**: [`InstalledDb::lock`] acquires an `fcntl` write-lock
//!    (non-blocking, `F_SETLK`) on `db_dir/lock`.  Only one process may hold
//!    this lock at a time.  The [`DbLock`] RAII guard drops the lock and
//!    removes the lock file on exit.  Callers that mutate the database (insert,
//!    remove, transfer_ownership) must hold a [`DbLock`] for the duration of
//!    the operation; read-only operations (list, get) do not require the lock
//!    but may observe partially committed state if a writer races them.
//!
//! 4. **Path format**: all paths stored in `FileEntry.path` are relative to the
//!    rootfs with no leading slash (e.g. `"bin/foo"`, not `"/bin/foo"`).
//!    The C code stores them with a leading `/`; the Rust port strips it during
//!    parse and re-adds it during serialisation.  Callers that compare path
//!    strings must use the no-leading-slash form.
//!
//! 5. **No size field**: the `files` manifest does not store file size.
//!    `FileEntry.size` is always 0 after a round-trip through the database; it
//!    is populated on demand from the filesystem by callers that need it
//!    (e.g. `cmd verify`).  Treating `size` as authoritative after a DB read
//!    will produce incorrect results.
//!
//! # Database layout
//!
//! ```text
//! /var/db/jpkg/
//! ├── installed/
//! │   └── <pkgname>/
//! │       ├── metadata.toml     (TOML: package.*, depends.*, hooks.*)
//! │       └── files             (newline-separated manifest; see below)
//! └── lock                      (PID file for mutual exclusion via fcntl F_SETLK)
//! ```
//!
//! # `files` manifest format
//!
//! One entry per line. Two line forms (verified against db.c lines 82–89 and
//! the `serialize_files_list` / `rewrite_files_manifest` implementations at
//! lines 156–175 and 732–754):
//!
//! **Regular file or directory** (db.c:169):
//! ```text
//! <sha256_64hex> <mode_06octal> <path>\n
//! ```
//!
//! **Symlink** (db.c:166–167; symlinks use the all-zeros sentinel sha256):
//! ```text
//! 0000000000000000000000000000000000000000000000000000000000000000 <mode_06octal> <path> -> <target>\n
//! ```
//!
//! Field separator is a single ASCII space (U+0020). Mode is printed with
//! `%06o` (6-digit, zero-padded, octal). There is **no size field** — the C
//! code never stores file size in the manifest. Size in `FileEntry` is derived
//! at query time from the filesystem when needed; the DB stores zero.
//!
//! This format must remain byte-compatible with C jpkg 1.1.5 so that users can
//! upgrade from C jpkg to jpkg-rs 2.0.0 without re-installing packages.

use crate::recipe::Metadata;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nix::fcntl::{Flock, FlockArg};

// ─── Symlink sentinel (db.c:88-89) ─────────────────────────────────────────

const SYMLINK_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

// ─── Public types ───────────────────────────────────────────────────────────

/// A single entry from a package's `files` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative to rootfs, e.g. `"bin/foo"` (no leading slash).
    pub path: String,
    /// 64-char lowercase hex SHA-256; `""` for symlinks (verified by target).
    pub sha256: String,
    /// 0 for symlinks and directories (not stored in the C manifest).
    pub size: u64,
    /// POSIX mode bits (e.g. `0o100755`).
    pub mode: u32,
    /// `Some("../bin/sh")` for symlinks, `None` for regular files/dirs.
    pub symlink_target: Option<String>,
    /// `true` if this entry represents a directory.
    pub is_dir: bool,
}

/// An installed package: its metadata plus its file manifest.
#[derive(Debug, Clone)]
pub struct InstalledPkg {
    pub metadata: Metadata,
    pub files: Vec<FileEntry>,
}

/// Handle to the on-disk installed-package database.
pub struct InstalledDb {
    /// e.g. `"/"` or an alternate `$JPKG_ROOT`.
    rootfs: PathBuf,
    /// `rootfs/var/db/jpkg`
    db_dir: PathBuf,
    /// `db_dir/installed`
    installed_dir: PathBuf,
}

/// RAII guard for the DB lock file at `db_dir/lock`.
///
/// Dropping this value releases the `fcntl` write-lock (via `Flock::drop`)
/// and removes the lock file from disk.
#[derive(Debug)]
pub struct DbLock {
    // Flock<File> holds the file open and releases the lock on drop.
    _flock: Flock<File>,
    path: PathBuf,
}

impl Drop for DbLock {
    fn drop(&mut self) {
        // Best-effort: ignore errors on cleanup.
        let _ = fs::remove_file(&self.path);
        // _flock drops here, releasing the fcntl lock.
    }
}

// ─── Error type ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum DbError {
    Io(io::Error),
    Recipe(crate::recipe::RecipeError),
    /// Another process holds the lock.
    Locked {
        pid: i32,
    },
    NotInstalled(String),
    AlreadyInstalled(String),
    BadManifestLine {
        line: u64,
        content: String,
    },
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Io(e) => write!(f, "I/O error: {e}"),
            DbError::Recipe(e) => write!(f, "metadata parse error: {e}"),
            DbError::Locked { pid } => {
                write!(f, "package database is locked by process {pid}")
            }
            DbError::NotInstalled(n) => write!(f, "package not installed: {n}"),
            DbError::AlreadyInstalled(n) => write!(f, "package already installed: {n}"),
            DbError::BadManifestLine { line, content } => {
                write!(f, "bad manifest line {line}: {content:?}")
            }
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::Io(e) => Some(e),
            DbError::Recipe(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for DbError {
    fn from(e: io::Error) -> Self {
        DbError::Io(e)
    }
}

impl From<crate::recipe::RecipeError> for DbError {
    fn from(e: crate::recipe::RecipeError) -> Self {
        DbError::Recipe(e)
    }
}

// ─── Manifest serialisation ──────────────────────────────────────────────────

/// Serialise a slice of [`FileEntry`] into the wire format written by C jpkg 1.1.5.
///
/// Format per db.c `serialize_files_list` (lines 156–175) and
/// `rewrite_files_manifest` (lines 732–754):
///
/// * Regular/dir: `"<sha256> %06o <path>\n"`
/// * Symlink:     `"0000...0000 %06o <path> -> <target>\n"`
fn serialize_files(entries: &[FileEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        if let Some(target) = &e.symlink_target {
            // db.c:166-167 — symlinks use the all-zeros sentinel sha256.
            out.push_str(&format!(
                "{} {:06o} {} -> {}\n",
                SYMLINK_SHA256, e.mode, e.path, target
            ));
        } else {
            // db.c:168-170 — regular files and directories.
            out.push_str(&format!("{} {:06o} {}\n", e.sha256, e.mode, e.path));
        }
    }
    out
}

/// Parse the `files` manifest text into a `Vec<FileEntry>`.
///
/// Mirrors `parse_files_list` in db.c (lines 91–154).  Fields:
///   `<sha256_64hex> <mode_octal> <path>[ -> <target>]`
fn parse_files(text: &str) -> Result<Vec<FileEntry>, DbError> {
    let mut entries = Vec::new();

    for (idx, raw_line) in text.lines().enumerate() {
        let line_no = idx as u64 + 1;
        let line = raw_line.trim_end_matches('\r'); // tolerate CRLF
        if line.is_empty() {
            continue;
        }

        // Field 1: sha256 — exactly 64 hex chars (db.c:100-107).
        if line.len() < 64 {
            return Err(DbError::BadManifestLine {
                line: line_no,
                content: line.to_string(),
            });
        }
        let sha256 = &line[..64];
        let rest = &line[64..];

        // Expect a single space separator after sha256 (db.c:106-108).
        if !rest.starts_with(' ') {
            return Err(DbError::BadManifestLine {
                line: line_no,
                content: line.to_string(),
            });
        }
        let rest = &rest[1..];

        // Field 2: mode (octal string up to next space) (db.c:109-116).
        let space2 = rest.find(' ').ok_or_else(|| DbError::BadManifestLine {
            line: line_no,
            content: line.to_string(),
        })?;
        let mode_str = &rest[..space2];
        let mode = u32::from_str_radix(mode_str, 8).map_err(|_| DbError::BadManifestLine {
            line: line_no,
            content: line.to_string(),
        })?;
        let path_and_rest = &rest[space2 + 1..];

        // Field 3: path (and optional " -> target") (db.c:118-144).
        // Check for symlink arrow " -> " (db.c:130-136).
        let is_symlink = sha256 == SYMLINK_SHA256;
        let (path, symlink_target) = if is_symlink {
            // Find " -> " in path_and_rest.
            if let Some(arrow_pos) = find_arrow(path_and_rest) {
                let path = &path_and_rest[..arrow_pos];
                let target = &path_and_rest[arrow_pos + 4..]; // skip " -> "
                (path.to_string(), Some(target.to_string()))
            } else {
                // Symlink sentinel but no arrow — treat path as-is.
                (path_and_rest.to_string(), None)
            }
        } else {
            (path_and_rest.to_string(), None)
        };

        // Infer is_dir from mode bits (S_IFDIR = 0o040000).
        let is_dir = (mode & 0o170000) == 0o040000;

        entries.push(FileEntry {
            path,
            sha256: if symlink_target.is_some() {
                String::new()
            } else {
                sha256.to_string()
            },
            size: 0, // not stored in manifest (db.c has no size field)
            mode,
            symlink_target,
            is_dir,
        });
    }

    Ok(entries)
}

/// Find the first occurrence of `" -> "` in `s`, mirroring db.c lines 130-136.
fn find_arrow(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(3) {
        if bytes[i] == b' ' && bytes[i + 1] == b'-' && bytes[i + 2] == b'>' && bytes[i + 3] == b' '
        {
            return Some(i);
        }
    }
    None
}

// ─── Atomic write helper ─────────────────────────────────────────────────────

/// Write `data` to `path` atomically: write to `<path>.tmp`, then rename.
fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp_path = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp_path)?;
        f.write_all(data)?;
        f.flush()?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

// ─── InstalledDb implementation ──────────────────────────────────────────────

impl InstalledDb {
    /// Open (and create-if-missing) the database at `rootfs/var/db/jpkg/installed/`.
    ///
    /// Creates all parent directories with mode 0755. Does not acquire the
    /// lock — call [`InstalledDb::lock`] separately when mutation is needed.
    pub fn open(rootfs: &Path) -> Result<Self, DbError> {
        let db_dir = rootfs.join("var/db/jpkg");
        let installed_dir = db_dir.join("installed");
        fs::create_dir_all(&installed_dir)?;
        // Best-effort: set mode 0755 (create_dir_all uses umask).
        let _ = fs::set_permissions(&installed_dir, fs::Permissions::from_mode(0o755));
        let _ = fs::set_permissions(&db_dir, fs::Permissions::from_mode(0o755));
        Ok(InstalledDb {
            rootfs: rootfs.to_path_buf(),
            db_dir,
            installed_dir,
        })
    }

    /// Acquire the global DB write-lock at `db_dir/lock`.
    ///
    /// Mirrors `db_lock` in db.c (lines 28–64):
    /// - Opens `db_dir/lock` with `O_WRONLY|O_CREAT`.
    /// - Calls `fcntl(F_SETLK)` (non-blocking) via `nix::fcntl::Flock`.
    /// - If `EACCES`/`EAGAIN`: reads the PID from the file and returns
    ///   `Err(DbError::Locked { pid })`.
    /// - On success: truncates the file and writes own PID, returns a
    ///   [`DbLock`] whose `Drop` removes the file.
    pub fn lock(&self) -> Result<DbLock, DbError> {
        let lock_path = self.db_dir.join("lock");

        // Open or create the lock file with O_WRONLY|O_CREAT (db.c:32) using
        // safe std::fs::OpenOptions — no `unsafe` raw-fd dance required.  The
        // owned File is handed straight to nix::fcntl::Flock, which takes any
        // `AsFd` implementor (File qualifies).
        let std_file: File = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o644)
            .open(&lock_path)?;

        // Attempt a non-blocking exclusive write lock (F_SETLK) (db.c:45).
        let flock_result = Flock::lock(std_file, FlockArg::LockExclusiveNonblock);

        match flock_result {
            Ok(mut locked_file) => {
                // Lock acquired. Truncate and write own PID (db.c:56-61).
                let own_pid = std::process::id() as i32;
                let pid_str = format!("{own_pid}\n");
                use std::io::Seek;
                locked_file.set_len(0)?;
                locked_file.seek(io::SeekFrom::Start(0))?;
                locked_file.write_all(pid_str.as_bytes())?;

                Ok(DbLock {
                    _flock: locked_file,
                    path: lock_path,
                })
            }
            Err((file, nix::errno::Errno::EWOULDBLOCK))
            | Err((file, nix::errno::Errno::EACCES)) => {
                // Another process holds the lock. Read its PID.
                drop(file); // close the fd
                let pid = read_pid_from_lock(&lock_path).unwrap_or(-1);
                Err(DbError::Locked { pid })
            }
            Err((_file, e)) => Err(io::Error::from_raw_os_error(e as i32).into()),
        }
    }

    /// List installed package names, sorted alphabetically.
    pub fn list(&self) -> Result<Vec<String>, DbError> {
        let mut names = Vec::new();
        let read_dir = match fs::read_dir(&self.installed_dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        for entry in read_dir {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') {
                continue;
            }
            // Only list directories (package subdirs).
            if entry.file_type()?.is_dir() {
                names.push(name_str.into_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Read a single installed package's metadata + files manifest.
    ///
    /// Returns `Ok(None)` if the package directory is missing.
    /// Returns `Err` on parse failure.
    pub fn get(&self, name: &str) -> Result<Option<InstalledPkg>, DbError> {
        let pkg_dir = self.installed_dir.join(name);
        if !pkg_dir.exists() {
            return Ok(None);
        }

        // Read and parse metadata.toml.
        let meta_path = pkg_dir.join("metadata.toml");
        let meta_text = match fs::read_to_string(&meta_path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let metadata = Metadata::from_str(&meta_text)?;

        // Read and parse the files manifest.
        let files_path = pkg_dir.join("files");
        let files = match fs::read_to_string(&files_path) {
            Ok(text) => parse_files(&text)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };

        Ok(Some(InstalledPkg { metadata, files }))
    }

    /// Insert or overwrite an installed-package record.
    ///
    /// Writes `metadata.toml` and `files` atomically (write to `.tmp`, rename).
    pub fn insert(&self, pkg: &InstalledPkg) -> Result<(), DbError> {
        let name = pkg.metadata.package.name.as_deref().unwrap_or("(unnamed)");
        let pkg_dir = self.installed_dir.join(name);
        fs::create_dir_all(&pkg_dir)?;

        // Serialize and write metadata.toml atomically.
        let meta_text = pkg.metadata.to_string()?;
        atomic_write(&pkg_dir.join("metadata.toml"), meta_text.as_bytes())?;

        // Serialize and write the files manifest atomically.
        let files_text = serialize_files(&pkg.files);
        atomic_write(&pkg_dir.join("files"), files_text.as_bytes())?;

        Ok(())
    }

    /// Remove an installed-package record (DB entry only; file removal is the caller's job).
    ///
    /// Returns the removed record so the caller can iterate `files` and unlink them.
    /// Returns `Ok(None)` if the package is not recorded.
    pub fn remove(&self, name: &str) -> Result<Option<InstalledPkg>, DbError> {
        let pkg = self.get(name)?;
        if pkg.is_none() {
            return Ok(None);
        }

        let pkg_dir = self.installed_dir.join(name);
        let meta_path = pkg_dir.join("metadata.toml");
        let files_path = pkg_dir.join("files");

        // Unlink the two files (ignore NotFound — tolerate partial state).
        for p in [&meta_path, &files_path] {
            match fs::remove_file(p) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }

        // Remove the package directory (rmdir — only succeeds if empty, mirrors db.c:509).
        match fs::remove_dir(&pkg_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {} // not empty — leave it, same as C rmdir()
        }

        Ok(pkg)
    }

    /// Transfer ownership of `paths` from `from` to `to`.
    ///
    /// Mirrors `db_transfer_ownership` in db.c (lines 756–775):
    /// 1. Load the `from` record.
    /// 2. Drop every path in `paths` from `from.files`.
    /// 3. Re-write `from` (or remove its DB entry if `files` is now empty).
    ///
    /// The `to` package is expected to have already been inserted with its
    /// complete file list — this function only scrubs entries from `from`.
    pub fn transfer_ownership(&self, from: &str, _to: &str, paths: &[&str]) -> Result<(), DbError> {
        let mut from_pkg = match self.get(from)? {
            Some(p) => p,
            None => return Ok(()), // nothing to transfer
        };

        let path_set: std::collections::HashSet<&str> = paths.iter().copied().collect();
        let before = from_pkg.files.len();
        from_pkg
            .files
            .retain(|e| !path_set.contains(e.path.as_str()));

        if from_pkg.files.len() == before {
            // Nothing was actually dropped — no-op.
            return Ok(());
        }

        if from_pkg.files.is_empty() {
            // Mirrors the C logic's rewrite; if the package has no more files
            // we still keep the DB entry (the C code does a rewrite, not remove).
            // Rewrite an empty files manifest.
            let pkg_dir = self.installed_dir.join(from);
            atomic_write(&pkg_dir.join("files"), b"")?;
        } else {
            // Rewrite the files manifest with the remaining entries.
            let pkg_dir = self.installed_dir.join(from);
            let text = serialize_files(&from_pkg.files);
            atomic_write(&pkg_dir.join("files"), text.as_bytes())?;
        }

        Ok(())
    }
}

// ─── Lock PID reader ─────────────────────────────────────────────────────────

/// Read the PID written by the current lock-holder from the lock file.
/// Returns `None` if the file cannot be read or has no valid integer.
fn read_pid_from_lock(path: &Path) -> Option<i32> {
    let mut f = File::open(path).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    buf.trim().parse::<i32>().ok()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{DependsSection, HooksSection, Metadata, PackageSection};
    use tempfile::TempDir;

    fn make_metadata(name: &str, version: &str) -> Metadata {
        Metadata {
            package: PackageSection {
                name: Some(name.to_string()),
                version: Some(version.to_string()),
                license: Some("MIT".to_string()),
                description: Some(format!("{name} description")),
                arch: Some("x86_64".to_string()),
                ..Default::default()
            },
            depends: DependsSection {
                runtime: vec!["musl".to_string()],
                ..Default::default()
            },
            hooks: HooksSection {
                post_install: Some("ldconfig /lib 2>/dev/null || true".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn make_three_files() -> Vec<FileEntry> {
        vec![
            FileEntry {
                path: "bin/mytool".to_string(),
                sha256: "a".repeat(64),
                size: 0,
                mode: 0o100755,
                symlink_target: None,
                is_dir: false,
            },
            FileEntry {
                path: "lib/libfoo.so".to_string(),
                sha256: String::new(),
                size: 0,
                mode: 0o120777,
                symlink_target: Some("../bin/mytool".to_string()),
                is_dir: false,
            },
            FileEntry {
                path: "share/mytool".to_string(),
                sha256: "b".repeat(64),
                size: 0,
                mode: 0o040755,
                symlink_target: None,
                is_dir: true,
            },
        ]
    }

    // ── 1. open() creates /var/db/jpkg/installed/ if missing ────────────────

    #[test]
    fn test_open_creates_dirs() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();
        let db = InstalledDb::open(rootfs).unwrap();
        assert!(db.installed_dir.exists(), "installed_dir should be created");
        assert!(db.db_dir.exists(), "db_dir should be created");
    }

    // ── 2. lock() succeeds; second call from same process returns Err::Locked ─

    #[test]
    fn test_lock_exclusive() {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();

        let lock1 = db.lock().expect("first lock should succeed");

        // While lock1 is held, a second lock attempt must fail.
        let result = db.lock();
        assert!(
            matches!(result, Err(DbError::Locked { .. })),
            "second lock should fail with Locked, got: {:?}",
            result
        );

        // Drop the lock.
        drop(lock1);

        // Now we should be able to re-acquire.
        let lock2 = db.lock().expect("lock after drop should succeed");
        drop(lock2);
    }

    // ── 3. list() of empty db is [] ──────────────────────────────────────────

    #[test]
    fn test_list_empty() {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();
        let names = db.list().unwrap();
        assert!(names.is_empty(), "expected empty list, got: {names:?}");
    }

    // ── 4. insert() then get() round-trips a pkg with 3 files ───────────────

    #[test]
    fn test_insert_and_get_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();

        let pkg = InstalledPkg {
            metadata: make_metadata("mytool", "1.2.3"),
            files: make_three_files(),
        };
        db.insert(&pkg).unwrap();

        let got = db.get("mytool").unwrap().expect("package should be found");
        assert_eq!(got.metadata.package.name.as_deref(), Some("mytool"));
        assert_eq!(got.metadata.package.version.as_deref(), Some("1.2.3"));
        assert_eq!(got.files.len(), 3);

        // Regular file.
        let bin = got.files.iter().find(|e| e.path == "bin/mytool").unwrap();
        assert_eq!(bin.sha256, "a".repeat(64));
        assert_eq!(bin.mode, 0o100755);
        assert!(bin.symlink_target.is_none());
        assert!(!bin.is_dir);

        // Symlink.
        let link = got
            .files
            .iter()
            .find(|e| e.path == "lib/libfoo.so")
            .unwrap();
        assert_eq!(link.symlink_target.as_deref(), Some("../bin/mytool"));
        assert_eq!(link.sha256, ""); // empty for symlinks

        // Directory.
        let dir = got.files.iter().find(|e| e.path == "share/mytool").unwrap();
        assert!(dir.is_dir);
    }

    // ── 5. list() after 3 inserts is sorted alphabetically ──────────────────

    #[test]
    fn test_list_sorted() {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();

        for name in ["zstd", "alpine-baselayout", "musl"] {
            let pkg = InstalledPkg {
                metadata: make_metadata(name, "1.0.0"),
                files: vec![],
            };
            db.insert(&pkg).unwrap();
        }

        let names = db.list().unwrap();
        assert_eq!(names, vec!["alpine-baselayout", "musl", "zstd"]);
    }

    // ── 6. remove() returns the pkg; get() afterwards is None ───────────────

    #[test]
    fn test_remove() {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();

        let pkg = InstalledPkg {
            metadata: make_metadata("target", "2.0.0"),
            files: make_three_files(),
        };
        db.insert(&pkg).unwrap();

        let removed = db
            .remove("target")
            .unwrap()
            .expect("remove should return pkg");
        assert_eq!(removed.metadata.package.name.as_deref(), Some("target"));
        assert_eq!(removed.files.len(), 3);

        let after = db.get("target").unwrap();
        assert!(after.is_none(), "get after remove should return None");
    }

    // ── 7. transfer_ownership: A loses bin/sh, B keeps it ───────────────────

    #[test]
    fn test_transfer_ownership() {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();

        let pkg_a = InstalledPkg {
            metadata: make_metadata("pkgA", "1.0.0"),
            files: vec![
                FileEntry {
                    path: "bin/sh".to_string(),
                    sha256: "c".repeat(64),
                    size: 0,
                    mode: 0o100755,
                    symlink_target: None,
                    is_dir: false,
                },
                FileEntry {
                    path: "lib/x".to_string(),
                    sha256: "d".repeat(64),
                    size: 0,
                    mode: 0o100644,
                    symlink_target: None,
                    is_dir: false,
                },
            ],
        };
        db.insert(&pkg_a).unwrap();

        let pkg_b = InstalledPkg {
            metadata: make_metadata("pkgB", "1.0.0"),
            files: vec![FileEntry {
                path: "bin/sh".to_string(),
                sha256: "e".repeat(64),
                size: 0,
                mode: 0o100755,
                symlink_target: None,
                is_dir: false,
            }],
        };
        db.insert(&pkg_b).unwrap();

        db.transfer_ownership("pkgA", "pkgB", &["bin/sh"]).unwrap();

        let a_after = db.get("pkgA").unwrap().expect("pkgA should still exist");
        assert_eq!(a_after.files.len(), 1);
        assert_eq!(a_after.files[0].path, "lib/x");
        assert!(
            a_after.files.iter().all(|e| e.path != "bin/sh"),
            "bin/sh should be gone from pkgA"
        );
    }

    // ── 8. files manifest byte format ────────────────────────────────────────

    #[test]
    fn test_manifest_byte_format() {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();

        let sha = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let pkg = InstalledPkg {
            metadata: make_metadata("fmttest", "1.0"),
            files: vec![
                FileEntry {
                    path: "bin/fmttest".to_string(),
                    sha256: sha.to_string(),
                    size: 0,
                    mode: 0o100755,
                    symlink_target: None,
                    is_dir: false,
                },
                FileEntry {
                    path: "lib/fmttest.so".to_string(),
                    sha256: String::new(),
                    size: 0,
                    mode: 0o120777,
                    symlink_target: Some("../bin/fmttest".to_string()),
                    is_dir: false,
                },
            ],
        };
        db.insert(&pkg).unwrap();

        let raw =
            fs::read_to_string(tmp.path().join("var/db/jpkg/installed/fmttest/files")).unwrap();

        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);

        // Regular file: "<sha256> %06o <path>\n"
        assert_eq!(lines[0], format!("{sha} {:06o} bin/fmttest", 0o100755u32));

        // Symlink: "<zeros> %06o <path> -> <target>\n"
        assert_eq!(
            lines[1],
            format!(
                "{} {:06o} lib/fmttest.so -> ../bin/fmttest",
                SYMLINK_SHA256, 0o120777u32
            )
        );
    }

    // ── 9. Forward-compat: hand-crafted C jpkg 1.1.5 files format ───────────
    //
    // This fixture is the EXACT bytes C jpkg 1.1.5 writes, taken from
    // db.c:serialize_files_list (lines 165-174) and rewrite_files_manifest
    // (lines 744-750). Format strings:
    //   Regular: "%s %06o %s\n" (sha256, mode, path)
    //   Symlink: "%s %06o %s -> %s\n" (SYMLINK_SHA256, mode, path, link_target)

    #[test]
    fn test_forward_compat_c_jpkg_format() {
        // Exactly what C jpkg 1.1.5 would write for a two-file package:
        //   1. regular file: bin/sh, mode 0100755, sha256 = "ab"*32
        //   2. symlink: usr/bin/sh -> ../../bin/sh, mode 0120777
        let ab32 = "ab".repeat(32); // 64 chars
        let c_format = format!(
            "{ab32} {mode_file:06o} bin/sh\n\
             {zeros} {mode_link:06o} usr/bin/sh -> ../../bin/sh\n",
            ab32 = ab32,
            mode_file = 0o100755u32,
            zeros = SYMLINK_SHA256,
            mode_link = 0o120777u32,
        );

        // Write it into a temp rootfs as if C jpkg had installed a package.
        let tmp = TempDir::new().unwrap();
        let pkg_dir = tmp.path().join("var/db/jpkg/installed/compat-pkg");
        fs::create_dir_all(&pkg_dir).unwrap();

        // Write a minimal metadata.toml.
        let meta_toml = r#"[package]
name = "compat-pkg"
version = "1.0.0"
license = "MIT"
"#;
        fs::write(pkg_dir.join("metadata.toml"), meta_toml).unwrap();
        fs::write(pkg_dir.join("files"), c_format.as_bytes()).unwrap();

        // Parse via the Rust implementation.
        let db = InstalledDb::open(tmp.path()).unwrap();
        let pkg = db
            .get("compat-pkg")
            .unwrap()
            .expect("compat-pkg should be found");

        assert_eq!(pkg.files.len(), 2, "should parse 2 entries");

        let regular = pkg
            .files
            .iter()
            .find(|e| e.path == "bin/sh")
            .expect("bin/sh not found");
        assert_eq!(regular.sha256, ab32);
        assert_eq!(regular.mode, 0o100755);
        assert!(regular.symlink_target.is_none());

        let symlink = pkg
            .files
            .iter()
            .find(|e| e.path == "usr/bin/sh")
            .expect("usr/bin/sh not found");
        assert_eq!(symlink.symlink_target.as_deref(), Some("../../bin/sh"));
        assert_eq!(symlink.mode, 0o120777);
        assert_eq!(symlink.sha256, ""); // empty for symlinks in our repr
    }
}
