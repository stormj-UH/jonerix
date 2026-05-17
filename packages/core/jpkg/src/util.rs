// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! Utility functions — port of `jpkg/src/util.c`: logging macros, string
//! helpers, path helpers, SHA-256, version comparison, license gate, and
//! layout audit.
//!
//! # Invariants
//!
//! 1. **Version comparison contract**: [`version_compare`] implements the same
//!    algorithm as the C `version_compare` in `util.c`.  The rules are: skip
//!    non-alphanumeric separators; compare digit segments numerically (longer
//!    digit string wins; strip leading zeros); compare alpha segments
//!    character-by-character; a digit segment beats an alpha segment.  Callers
//!    that sort package upgrade candidates MUST use this function, not
//!    `str::cmp`, because `"1.10" > "1.9"` under this ordering but not under
//!    lexicographic ordering.
//!
//! 2. **License gate completeness**: [`license_is_permissive`] checks against
//!    the `PERMISSIVE_LICENSES` whitelist compiled from `util.c`.  Any license
//!    identifier not in that list (or not decomposable via SPDX `OR`/`AND`
//!    operators into listed identifiers) returns `false`.  Adding a new license
//!    to the project requires updating `PERMISSIVE_LICENSES` here AND in
//!    `util.c`; a mismatch between the two lists will cause the Rust port to
//!    reject recipes that the C tool accepts (or vice versa).
//!
//! 3. **Layout audit scope**: [`audit_layout_tree`] checks for banned paths
//!    (`lib64/`, root-level `*.0` files) and banned string content (`/lib64`
//!    in ELF and text files, symlink targets).  Files under `share/man/`,
//!    `share/doc/`, and similar doc trees are exempt from the content scan.
//!    The `bin/jpkg` and `bin/jpkg-local` binaries are exempt because they
//!    contain `"/lib64"` as a literal in their own source.  Adding new exempt
//!    paths requires a code change here; it cannot be configured at runtime.
//!
//! 4. **SHA-256 correctness**: [`sha256_file`] streams the file in 8 KiB
//!    chunks and computes the digest incrementally; it does not read the whole
//!    file into memory.  The result is a 64-character lowercase hex string
//!    identical to `sha256sum` output.  Callers must not truncate or
//!    case-fold this string before comparing with INDEX/manifest entries.
//!
//! 5. **log_fatal panics**: the [`log_fatal`] macro logs at `error` level and
//!    then panics.  In production binaries a panic hook converts this to an
//!    exit(1); in tests the panic is caught by the test harness.  Callers must
//!    not use `log_fatal` for recoverable errors — use the normal `Result`
//!    error-propagation path instead.

use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ── Public constants ────────────────────────────────────────────────────────

pub const JPKG_VERSION: &str = "2.2.1";
pub const JPKG_DB_DIR: &str = "/var/db/jpkg";
pub const JPKG_CACHE_DIR: &str = "/var/cache/jpkg";
pub const JPKG_CONFIG_DIR: &str = "/etc/jpkg";
pub const JPKG_KEY_DIR: &str = "/etc/jpkg/keys";

// ── Logging ─────────────────────────────────────────────────────────────────
//
// The C code kept a mutable global `g_log_level`.  Rust logging is handled by
// the `log` façade; callers initialise `env_logger` in `main`.  We expose thin
// macro-style free functions so callers can migrate from C call-sites without
// changing every usage.  `log_fatal` panics instead of calling exit(1) so that
// tests don't kill the process; real binaries can set a panic hook or catch it.

#[macro_export]
macro_rules! log_debug { ($($arg:tt)*) => { log::debug!($($arg)*) } }
#[macro_export]
macro_rules! log_info  { ($($arg:tt)*) => { log::info!($($arg)*)  } }
#[macro_export]
macro_rules! log_warn  { ($($arg:tt)*) => { log::warn!($($arg)*)  } }
#[macro_export]
macro_rules! log_error { ($($arg:tt)*) => { log::error!($($arg)*) } }

/// Equivalent of `log_fatal`: log at error level then panic.
#[macro_export]
macro_rules! log_fatal {
    ($($arg:tt)*) => {{
        log::error!($($arg)*);
        panic!("jpkg: fatal: {}", format!($($arg)*));
    }};
}

// ── String utilities ─────────────────────────────────────────────────────────

/// Trim leading and trailing ASCII whitespace (mirrors `str_trim`).
pub fn str_trim(s: &str) -> &str {
    s.trim()
}

/// `str_starts_with` — thin wrapper kept for naming parity with the C API.
#[inline]
pub fn str_starts_with(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// `str_ends_with` — thin wrapper kept for naming parity.
#[inline]
pub fn str_ends_with(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// `str_contains` — thin wrapper kept for naming parity.
#[inline]
pub fn str_contains(haystack: &str, needle: &str) -> bool {
    haystack.contains(needle)
}

/// `str_replace` — replace all occurrences of `old` with `new_str`.
pub fn str_replace_all(s: &str, old: &str, new_str: &str) -> String {
    s.replace(old, new_str)
}

// ── Path utilities ───────────────────────────────────────────────────────────

/// Join two path components, mirroring `path_join(dir, name)`.
/// Uses `PathBuf::push` so it handles leading-slash cases correctly.
pub fn path_join(dir: &Path, name: &Path) -> PathBuf {
    let mut p = dir.to_path_buf();
    p.push(name);
    p
}

/// Ensure that the parent directory of `path` exists, creating it and all
/// missing ancestors with mode 0o755.  Mirrors `mkdirs(dirname(path), 0755)`.
pub fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// Create `path` and all missing ancestor directories with mode 0o755.
/// Direct equivalent of `mkdirs(path, 0755)`.
pub fn mkdirs(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

/// Return `true` if `path` exists and is a regular file (follows symlinks).
/// Mirrors `file_exists`.
pub fn file_exists(path: &Path) -> bool {
    path.metadata().map(|m| m.is_file()).unwrap_or(false)
}

/// Return `true` if `path` exists and is a directory (follows symlinks).
/// Mirrors `dir_exists`.
pub fn dir_exists(path: &Path) -> bool {
    path.metadata().map(|m| m.is_dir()).unwrap_or(false)
}

/// Read an entire file into a `Vec<u8>`.  Mirrors `file_read`.
pub fn file_read(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

/// Write `data` to `path`, creating or truncating.  Mirrors `file_write`.
pub fn file_write(path: &Path, data: &[u8]) -> io::Result<()> {
    fs::write(path, data)
}

/// Copy `src` to `dst` (read-then-write, no special-file handling).
/// Mirrors `file_copy`.
pub fn file_copy(src: &Path, dst: &Path) -> io::Result<()> {
    let data = fs::read(src)?;
    fs::write(dst, data)
}

/// Recursively remove `path` (file or directory).  Mirrors the recursive-rm
/// pattern used implicitly via `rm -rf` throughout the C code.
pub fn remove_recursive(path: &Path) -> io::Result<()> {
    if !path.exists() && !path.symlink_metadata().is_ok() {
        return Ok(());
    }
    if path.symlink_metadata()?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

// ── SHA-256 ──────────────────────────────────────────────────────────────────

/// Hash `data` and return 32 raw bytes.  Mirrors `sha256_hash`.
pub fn sha256_hash(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

/// Hex-encode a 32-byte hash to a 64-char lowercase string.
/// Mirrors `sha256_hex`.
pub fn sha256_hex(hash: &[u8; 32]) -> String {
    hex::encode(hash)
}

/// Hash the file at `path` and return a 64-char lowercase hex string.
/// Mirrors `sha256_file`.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

// ── Version comparison ───────────────────────────────────────────────────────

/// Compare two version strings with the same algorithm as the C `version_compare`.
///
/// Rules (straight port):
/// - Non-alphanumeric characters are treated as separators and skipped.
/// - Numeric segments: leading zeros are skipped; longer digit string wins;
///   equal-length strings compared lexicographically (digit chars only, so
///   this is equivalent to numeric comparison for equal lengths).
/// - Alpha segments: compared character-by-character; longer wins if prefixes
///   match.
/// - Digit segment vs alpha segment: digit wins.
pub fn version_compare(a: &str, b: &str) -> Ordering {
    let ab: Vec<char> = a.chars().collect();
    let bb: Vec<char> = b.chars().collect();
    let mut i = 0usize;
    let mut j = 0usize;

    loop {
        // Skip separators.
        while i < ab.len() && !ab[i].is_ascii_alphanumeric() {
            i += 1;
        }
        while j < bb.len() && !bb[j].is_ascii_alphanumeric() {
            j += 1;
        }

        let a_done = i >= ab.len();
        let b_done = j >= bb.len();
        if a_done && b_done {
            return Ordering::Equal;
        }
        if a_done {
            return Ordering::Less;
        }
        if b_done {
            return Ordering::Greater;
        }

        let a_digit = ab[i].is_ascii_digit();
        let b_digit = bb[j].is_ascii_digit();

        if a_digit && b_digit {
            // Skip leading zeros (but keep at least one digit).
            while i + 1 < ab.len() && ab[i] == '0' && ab[i + 1].is_ascii_digit() {
                i += 1;
            }
            while j + 1 < bb.len() && bb[j] == '0' && bb[j + 1].is_ascii_digit() {
                j += 1;
            }

            let si = i;
            let sj = j;
            while i < ab.len() && ab[i].is_ascii_digit() {
                i += 1;
            }
            while j < bb.len() && bb[j].is_ascii_digit() {
                j += 1;
            }
            let len_a = i - si;
            let len_b = j - sj;

            if len_a != len_b {
                return len_a.cmp(&len_b);
            }
            // Same length: lexicographic (equivalent to numeric for same-len strings).
            let cmp = ab[si..i].cmp(&bb[sj..j]);
            if cmp != Ordering::Equal {
                return cmp;
            }
        } else if !a_digit && !b_digit {
            // Both alpha: compare char-by-char.
            while i < ab.len()
                && j < bb.len()
                && ab[i].is_ascii_alphabetic()
                && bb[j].is_ascii_alphabetic()
            {
                if ab[i] != bb[j] {
                    return ab[i].cmp(&bb[j]);
                }
                i += 1;
                j += 1;
            }
            if i < ab.len() && ab[i].is_ascii_alphabetic() {
                return Ordering::Greater;
            }
            if j < bb.len() && bb[j].is_ascii_alphabetic() {
                return Ordering::Less;
            }
        } else {
            // Digit vs alpha: digit wins.
            return if a_digit {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
    }
}

// ── License gate ─────────────────────────────────────────────────────────────

/// The exact permissive-license whitelist from `util.c`.
static PERMISSIVE_LICENSES: &[&str] = &[
    "MIT",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Apache-2.0",
    "0BSD",
    "CC0",
    "CC0-1.0",
    "Unlicense",
    "curl",
    "MirOS",
    "OpenSSL",
    "SSLeay",
    "zlib",
    "Zlib",
    "public domain",
    "Public-Domain",
    "BSD-2-Clause-Patent",
    "PSF-2.0",
    "BSL-1.0",
    "Artistic-2.0",
    "Ruby",
    "MPL-2.0",
    "Info-ZIP",
    "bzip2-1.0.6",
    // FreeType Project License — BSD-style with attribution.  FSF Free; SPDX
    // categorises as permissive.  Used by FreeType (dual-licensed with
    // GPL-2.0-only; recipes select FTL).
    "FTL",
    // Historical Permission Notice and Disclaimer — pre-MIT permissive
    // template (X11-style).  OSI-approved.  Used by fontconfig and libtiff.
    "HPND",
    // Unicode Data Files and Software licenses — MIT-style with a
    // non-endorsement clause on the Unicode trademark.  ICU 60 through 75
    // ship Unicode-DFS-2016; ICU 76+ moved to Unicode-3.0 (textual cleanup,
    // not a substantive change).  Both are OSI-approved.
    "Unicode-DFS-2016",
    "Unicode-3.0",
    // libpng License v2 (2018-) — zlib-style permissive, OSI-approved.
    "libpng-2.0",
];

/// Return `true` if `license` is on the project's permissive whitelist.
///
/// Handles:
/// - Exact case-insensitive match against the list above.
/// - SPDX `OR` compound: permissive if **any** component is permissive.
/// - SPDX `AND` compound: permissive only if **all** components are permissive.
/// - Parenthesised sub-expressions, e.g. `"(MIT OR GPL-2.0) AND Apache-2.0"`.
pub fn license_is_permissive(license: &str) -> bool {
    let license = license.trim();
    if license.is_empty() {
        return false;
    }

    // Strip a single layer of matching outer parentheses: "(expr)" → "expr".
    let license = strip_outer_parens(license);

    // Exact case-insensitive match.
    for &known in PERMISSIVE_LICENSES {
        if license.eq_ignore_ascii_case(known) {
            return true;
        }
    }

    // Find a top-level " OR " or " AND " (i.e. not inside parentheses).
    // We scan left-to-right tracking paren depth; an operator at depth 0
    // is a genuine top-level split point.
    if let Some((op, left, right)) = find_top_level_op(license) {
        return match op {
            TopOp::Or => license_is_permissive(left) || license_is_permissive(right),
            TopOp::And => license_is_permissive(left) && license_is_permissive(right),
        };
    }

    false
}

/// Strip one layer of matching outer parentheses, returning the inner slice.
/// `"(MIT OR Apache-2.0)"` → `"MIT OR Apache-2.0"`.
/// Already-unparenthesised strings are returned unchanged.
fn strip_outer_parens(s: &str) -> &str {
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    let inner = &s[1..s.len() - 1];
    // Verify the opening paren really does close at the last char (i.e. the
    // whole expression is wrapped, not just a prefix like `(A) OR B`).
    let mut depth = 0i32;
    for ch in inner.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    // The first `(` closed before the end — the parens are not
                    // wrapping the whole expression; return the original.
                    return s;
                }
            }
            _ => {}
        }
    }
    // If we get here without early return, inner is a valid balanced expression.
    inner
}

#[derive(Copy, Clone)]
enum TopOp {
    Or,
    And,
}

/// Find the first top-level ` OR ` or ` AND ` (paren-depth == 0) in `s`.
/// Returns `Some((op, left, right))` or `None`.
fn find_top_level_op(s: &str) -> Option<(TopOp, &str, &str)> {
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut depth = 0i32;
    let mut i = 0usize;

    while i < n {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            b' ' if depth == 0 => {
                // Try " OR " (4 bytes including leading space already consumed)
                if i + 4 <= n && &bytes[i..i + 4] == b" OR " {
                    let left = &s[..i];
                    let right = &s[i + 4..];
                    return Some((TopOp::Or, left, right));
                }
                // Try " AND " (5 bytes)
                if i + 5 <= n && &bytes[i..i + 5] == b" AND " {
                    let left = &s[..i];
                    let right = &s[i + 5..];
                    return Some((TopOp::And, left, right));
                }
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

// ── Layout audit ─────────────────────────────────────────────────────────────

/// Result of an `audit_layout_tree` walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditResult {
    Ok,
    /// A root-level `*.0` file was found (e.g. `libfoo.0` at top level).
    RootDotZero(PathBuf),
    /// A `/lib64` path exists as a real entry (not a symlink).
    Lib64Path(PathBuf),
    /// A symlink or file contents reference `/lib64`.
    Lib64Reference(PathBuf),
    /// A staged `/sbin` path was found (use `/bin` instead).
    SbinPath(PathBuf),
}

impl AuditResult {
    /// Return `true` if the audit result is [`AuditResult::Ok`] (no violations found).
    pub fn is_ok(&self) -> bool {
        matches!(self, AuditResult::Ok)
    }

    /// Human-readable description, mirroring `audit_layout_result_string`.
    pub fn description(&self) -> &'static str {
        match self {
            AuditResult::Ok => "ok",
            AuditResult::RootDotZero(_) => "root-level *.0 payload",
            AuditResult::Lib64Path(_) => "staged /lib64 payload",
            AuditResult::Lib64Reference(_) => "embedded /lib64 reference",
            AuditResult::SbinPath(_) => "staged /sbin payload (use /bin)",
        }
    }
}

/// Return `Err` (carrying the `AuditResult`) if the tree under `root` is
/// non-conformant for jonerix's merged-usr flat layout.
///
/// Checks (same as the C version):
/// 1. Any path whose first component is `lib64` → `Lib64Path`.
/// 2. Any root-level entry ending in `.0` (no slash in the rel path) → `RootDotZero`.
/// 3. Any symlink whose target contains `/lib64` → `Lib64Reference`.
/// 4. Any regular file (ELF or text, not in doc dirs, not `bin/jpkg` /
///    `bin/jpkg-local`) whose contents contain the byte string `/lib64` →
///    `Lib64Reference`.
///
/// On success returns `Ok(())`.
pub fn audit_layout_tree(root: &Path) -> Result<(), AuditResult> {
    if !dir_exists(root) {
        return Ok(());
    }

    // Build the needle as a byte array so this binary does not itself trigger
    // the audit (mirrors the C `lib64_marker` trick).
    let lib64_marker: &[u8] = &[b'/', b'l', b'i', b'b', b'6', b'4'];

    for entry in WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let full = entry.path();
        let rel = match full.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let rel_str = rel.to_string_lossy();

        // Check lib64 path (first component == "lib64").
        {
            let mut comps = rel.components();
            if let Some(first) = comps.next() {
                let fname = first.as_os_str().to_string_lossy();
                if fname == "lib64" {
                    return Err(AuditResult::Lib64Path(rel.to_path_buf()));
                }
            }
        }

        // Root-level *.0: no parent component (depth == 1) and ends with ".0".
        if entry.depth() == 1 && rel_str.ends_with(".0") {
            return Err(AuditResult::RootDotZero(rel.to_path_buf()));
        }

        let ft = entry.file_type();

        if ft.is_symlink() {
            // Read the symlink target and check for /lib64.
            if let Ok(target) = fs::read_link(full) {
                let target_bytes = target.as_os_str().as_encoded_bytes();
                if contains_bytes(target_bytes, lib64_marker) {
                    return Err(AuditResult::Lib64Reference(rel.to_path_buf()));
                }
            }
        } else if ft.is_file() {
            // Skip exempt paths.
            if audit_path_is_doc_payload(&rel_str)
                || rel_str == "bin/jpkg"
                || rel_str == "bin/jpkg-local"
            {
                continue;
            }

            // Read the first 256 bytes to detect ELF or text.
            let head = read_head(full, 256);
            if head.is_empty() {
                continue;
            }
            if audit_buffer_is_elf(&head) || audit_buffer_is_text(&head) {
                // Full scan for /lib64.
                if file_contains_bytes(full, lib64_marker).unwrap_or(false) {
                    return Err(AuditResult::Lib64Reference(rel.to_path_buf()));
                }
            }
        }
    }

    Ok(())
}

// -- audit helpers (private) -------------------------------------------------

fn audit_path_is_doc_payload(rel: &str) -> bool {
    let doc_prefixes = [
        "share/man/",
        "share/doc/",
        "share/info/",
        "man/",
        "doc/",
        "info/",
    ];
    let doc_exacts = ["share/man", "share/doc", "share/info", "man", "doc", "info"];
    doc_prefixes.iter().any(|p| rel.starts_with(p)) || doc_exacts.iter().any(|e| rel == *e)
}

fn audit_buffer_is_elf(buf: &[u8]) -> bool {
    buf.len() >= 4 && buf[0] == 0x7f && buf[1] == b'E' && buf[2] == b'L' && buf[3] == b'F'
}

fn audit_buffer_is_text(buf: &[u8]) -> bool {
    !buf.contains(&0u8)
}

fn read_head(path: &Path, limit: usize) -> Vec<u8> {
    let Ok(mut f) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut buf = vec![0u8; limit];
    match f.read(&mut buf) {
        Ok(n) => {
            buf.truncate(n);
            buf
        }
        Err(_) => Vec::new(),
    }
}

/// Search `path` for `needle` using a sliding-window approach (mirrors
/// `audit_file_contains_string`).
fn file_contains_bytes(path: &Path, needle: &[u8]) -> io::Result<bool> {
    if needle.is_empty() {
        return Ok(false);
    }
    let mut file = fs::File::open(path)?;
    let buf_size = 8192 + needle.len();
    let mut buf = vec![0u8; buf_size];
    let mut carry = 0usize;

    loop {
        let n = file.read(&mut buf[carry..])?;
        if n == 0 {
            break;
        }
        let total = carry + n;
        for i in 0..total.saturating_sub(needle.len() - 1) {
            if buf[i..].starts_with(needle) {
                return Ok(true);
            }
        }
        carry = needle.len().saturating_sub(1).min(total);
        buf.copy_within(total - carry..total, 0);
    }
    Ok(false)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ── File-walking for manifest construction ───────────────────────────────────

/// A single entry produced by `walk_tree`.
#[derive(Debug)]
pub struct TreeEntry {
    /// Path relative to the walk root, sorted lexicographically.
    pub rel_path: PathBuf,
    /// Full absolute path.
    pub full_path: PathBuf,
    pub is_file: bool,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// `st_mode` bits (Unix permissions).
    pub mode: u32,
    /// File size in bytes (0 for symlinks/dirs).
    pub size: u64,
}

/// Walk `root` and return every entry (regular files, symlinks, dirs) sorted
/// by relative path.  Mirrors the recursive-walk pattern used in `cmd_build.c`
/// for manifest construction.
pub fn walk_tree(root: &Path) -> io::Result<Vec<TreeEntry>> {
    let mut entries: Vec<TreeEntry> = Vec::new();

    for entry in WalkDir::new(root)
        .min_depth(1)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
    {
        let entry = entry.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let full = entry.path().to_path_buf();
        let rel = full
            .strip_prefix(root)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            .to_path_buf();

        let meta = entry
            .metadata()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        use std::os::unix::fs::MetadataExt;
        let mode = meta.mode();
        let size = if meta.is_file() { meta.len() } else { 0 };
        let ft = entry.file_type();

        entries.push(TreeEntry {
            rel_path: rel,
            full_path: full,
            is_file: ft.is_file(),
            is_dir: ft.is_dir(),
            is_symlink: ft.is_symlink(),
            mode,
            size,
        });
    }

    Ok(entries)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs as unix_fs;
    use tempfile::TempDir;

    // ── sha256_file ───────────────────────────────────────────────────────

    #[test]
    fn test_sha256_file_known_value() {
        // echo -n "hello world" | sha256sum
        // b94d27b9934d3e08a52e52d7da7dabfac484efe04294e576a0a8a49f93083cff (NO)
        // Correct: b94d27b9934d3e08a52e52d7da7dabfac484efe04294e576a0a8a49f93083cff
        // Actually: echo -n "hello world" → b94d27b... let's use a verified value.
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("empty.txt");
        file_write(&p, b"").unwrap();
        let h = sha256_file(&p).unwrap();
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_file_hello() {
        // SHA-256("hello\n") = 5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("hello.txt");
        file_write(&p, b"hello\n").unwrap();
        let h = sha256_file(&p).unwrap();
        assert_eq!(
            h,
            "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
        );
    }

    // ── version_compare ───────────────────────────────────────────────────

    #[test]
    fn test_version_compare_basic() {
        assert_eq!(version_compare("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(version_compare("1.0.1", "1.0.0"), Ordering::Greater);
        assert_eq!(version_compare("1.0.0", "1.0.1"), Ordering::Less);
        assert_eq!(version_compare("2.0.0", "1.9.9"), Ordering::Greater);
        assert_eq!(version_compare("1.10.0", "1.9.0"), Ordering::Greater);
    }

    #[test]
    fn test_version_compare_leading_zeros() {
        // Leading zeros are stripped: "1.01" == "1.1"
        assert_eq!(version_compare("1.01", "1.1"), Ordering::Equal);
        assert_eq!(version_compare("1.001", "1.1"), Ordering::Equal);
    }

    #[test]
    fn test_version_compare_alpha_segments() {
        assert_eq!(version_compare("1.0a", "1.0b"), Ordering::Less);
        assert_eq!(version_compare("1.0b", "1.0a"), Ordering::Greater);
        assert_eq!(version_compare("1.0a", "1.0a"), Ordering::Equal);
    }

    #[test]
    fn test_version_compare_digit_beats_alpha() {
        // Digit segment > alpha segment.
        assert_eq!(version_compare("1.1", "1.a"), Ordering::Greater);
        assert_eq!(version_compare("1.a", "1.1"), Ordering::Less);
    }

    #[test]
    fn test_version_compare_empty() {
        assert_eq!(version_compare("", ""), Ordering::Equal);
        assert_eq!(version_compare("1.0", ""), Ordering::Greater);
        assert_eq!(version_compare("", "1.0"), Ordering::Less);
    }

    #[test]
    fn test_version_compare_rc_suffix() {
        // "1.0.0" vs "1.0.0rc1": digit segment "1.0.0" exhausted, "rc1" remaining → "1.0.0" < "1.0.0rc1"
        // In C: after matching 1, 0, 0, p2 has "rc1" left, so p1 is exhausted first → -1
        assert_eq!(version_compare("1.0.0", "1.0.0rc1"), Ordering::Less);
    }

    // ── license_is_permissive ────────────────────────────────────────────

    #[test]
    fn test_license_permissive_exact() {
        assert!(license_is_permissive("MIT"));
        assert!(license_is_permissive("Apache-2.0"));
        assert!(license_is_permissive("BSD-2-Clause"));
        assert!(license_is_permissive("BSD-3-Clause"));
        assert!(license_is_permissive("ISC"));
        assert!(license_is_permissive("0BSD"));
        assert!(license_is_permissive("MirOS"));
        assert!(license_is_permissive("Zlib"));
        assert!(license_is_permissive("zlib"));
        assert!(license_is_permissive("PSF-2.0"));
        assert!(license_is_permissive("Artistic-2.0"));
        assert!(license_is_permissive("MPL-2.0"));
        assert!(license_is_permissive("bzip2-1.0.6"));
        assert!(license_is_permissive("Public-Domain"));
        assert!(license_is_permissive("public domain"));
        // Typography stack additions (freetype/fontconfig/icu/libpng).
        assert!(license_is_permissive("FTL"));
        assert!(license_is_permissive("HPND"));
        assert!(license_is_permissive("Unicode-DFS-2016"));
        assert!(license_is_permissive("Unicode-3.0"));
        assert!(license_is_permissive("libpng-2.0"));
        // Common dual-licensed forms used in recipe metadata.
        assert!(license_is_permissive("FTL OR GPL-2.0-only"));
        assert!(license_is_permissive("GPL-2.0-only OR FTL"));
    }

    #[test]
    fn test_license_permissive_case_insensitive() {
        assert!(license_is_permissive("mit"));
        assert!(license_is_permissive("APACHE-2.0"));
        assert!(license_is_permissive("bsd-2-clause"));
    }

    #[test]
    fn test_license_forbidden() {
        assert!(!license_is_permissive("GPL-2.0-only"));
        assert!(!license_is_permissive("GPL-3.0-only"));
        assert!(!license_is_permissive("LGPL-2.1-only"));
        assert!(!license_is_permissive("AGPL-3.0-only"));
        assert!(!license_is_permissive("SSPL-1.0"));
        assert!(!license_is_permissive("EUPL-1.2"));
        assert!(!license_is_permissive("CC-BY-SA-4.0"));
        assert!(!license_is_permissive(""));
    }

    #[test]
    fn test_license_spdx_or() {
        // "MIT OR Apache-2.0" — either arm permissive → true.
        assert!(license_is_permissive("MIT OR Apache-2.0"));
        // "GPL-3.0-only OR MIT" — MIT arm saves it.
        assert!(license_is_permissive("GPL-3.0-only OR MIT"));
        // Both forbidden → false.
        assert!(!license_is_permissive("GPL-2.0-only OR GPL-3.0-only"));
    }

    #[test]
    fn test_license_spdx_and() {
        // Both permissive → true.
        assert!(license_is_permissive("MIT AND Apache-2.0"));
        // One forbidden → false.
        assert!(!license_is_permissive("MIT AND GPL-2.0-only"));
    }

    #[test]
    fn test_license_nested_parens() {
        // "(MIT OR GPL-2.0) AND Apache-2.0":
        //   outer parens strip → "MIT OR GPL-2.0 AND Apache-2.0"? No — the AND
        //   is at the top level AFTER the closing paren, so outer parens do NOT
        //   wrap the whole thing.  find_top_level_op scans at depth 0 and finds
        //   " AND " AFTER the closing paren → left = "(MIT OR GPL-2.0)",
        //   right = "Apache-2.0".  left strips parens → "MIT OR GPL-2.0" →
        //   MIT is permissive → true.  Apache-2.0 is permissive → true AND true.
        assert!(license_is_permissive("(MIT OR GPL-2.0) AND Apache-2.0"));

        // "(GPL-2.0 OR GPL-3.0) AND MIT":
        //   left = "(GPL-2.0 OR GPL-3.0)" → inner "GPL-2.0 OR GPL-3.0" →
        //   neither arm permissive → false.  AND short-circuits → false.
        assert!(!license_is_permissive("(GPL-2.0 OR GPL-3.0) AND MIT"));

        // "MIT AND (Apache-2.0 OR GPL-2.0)":
        //   top-level AND: left=MIT (permissive), right=(Apache-2.0 OR GPL-2.0).
        //   right strips parens → "Apache-2.0 OR GPL-2.0" → Apache-2.0 saves it.
        assert!(license_is_permissive("MIT AND (Apache-2.0 OR GPL-2.0)"));

        // Fully-wrapped: "(MIT AND Apache-2.0)" → strips outer parens →
        // "MIT AND Apache-2.0" → both permissive → true.
        assert!(license_is_permissive("(MIT AND Apache-2.0)"));

        // "(GPL-2.0)" → strips parens → "GPL-2.0" → not permissive.
        assert!(!license_is_permissive("(GPL-2.0)"));
    }

    // ── audit_layout_tree ────────────────────────────────────────────────

    #[test]
    fn test_audit_flat_tree_ok() {
        let tmp = TempDir::new().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        file_write(&bin.join("myapp"), b"#!/bin/sh\necho hi\n").unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        file_write(&lib.join("libfoo.so.1.0"), b"ELF").unwrap();

        let result = audit_layout_tree(tmp.path());
        assert!(result.is_ok(), "flat tree should pass audit: {:?}", result);
    }

    #[test]
    fn test_audit_rejects_lib64_dir() {
        let tmp = TempDir::new().unwrap();
        let lib64 = tmp.path().join("lib64");
        fs::create_dir_all(&lib64).unwrap();
        file_write(&lib64.join("libc.so.6"), b"ELF").unwrap();

        let result = audit_layout_tree(tmp.path());
        assert!(
            matches!(result, Err(AuditResult::Lib64Path(_))),
            "lib64 dir should be rejected, got: {:?}",
            result
        );
    }

    #[test]
    fn test_audit_rejects_lib64_symlink_target() {
        let tmp = TempDir::new().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        // symlink pointing at /lib64/ld-linux-x86-64.so.2
        unix_fs::symlink("/lib64/ld-linux-x86-64.so.2", lib.join("ld.so")).unwrap();

        let result = audit_layout_tree(tmp.path());
        assert!(
            matches!(result, Err(AuditResult::Lib64Reference(_))),
            "symlink to /lib64 should be rejected, got: {:?}",
            result
        );
    }

    #[test]
    fn test_audit_rejects_root_dot_zero() {
        let tmp = TempDir::new().unwrap();
        // A top-level file ending in ".0" — this is a bare .0 at root level.
        file_write(&tmp.path().join("libfoo.0"), b"data").unwrap();

        let result = audit_layout_tree(tmp.path());
        assert!(
            matches!(result, Err(AuditResult::RootDotZero(_))),
            "root-level *.0 should be rejected, got: {:?}",
            result
        );
    }

    #[test]
    fn test_audit_dot_zero_in_subdir_ok() {
        // lib/libfoo.so.0 is fine — only root-level *.0 is banned.
        let tmp = TempDir::new().unwrap();
        let lib = tmp.path().join("lib");
        fs::create_dir_all(&lib).unwrap();
        file_write(&lib.join("libfoo.so.0"), b"data").unwrap();

        let result = audit_layout_tree(tmp.path());
        assert!(result.is_ok(), "subdir *.0 should be fine: {:?}", result);
    }

    #[test]
    fn test_audit_rejects_lib64_in_file_contents() {
        let tmp = TempDir::new().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        // A text file that references /lib64.
        file_write(
            &bin.join("wrapper.sh"),
            b"#!/bin/sh\nexport LD_LIBRARY_PATH=/lib64\n",
        )
        .unwrap();

        let result = audit_layout_tree(tmp.path());
        assert!(
            matches!(result, Err(AuditResult::Lib64Reference(_))),
            "file referencing /lib64 should be rejected, got: {:?}",
            result
        );
    }

    #[test]
    fn test_audit_jpkg_binary_exempt() {
        // bin/jpkg is explicitly exempt from the content scan.
        let tmp = TempDir::new().unwrap();
        let bin = tmp.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        file_write(&bin.join("jpkg"), b"#!/bin/sh\necho /lib64\n").unwrap();

        let result = audit_layout_tree(tmp.path());
        assert!(result.is_ok(), "bin/jpkg should be exempt: {:?}", result);
    }

    // ── string helpers ───────────────────────────────────────────────────

    #[test]
    fn test_str_trim() {
        assert_eq!(str_trim("  hello  "), "hello");
        assert_eq!(str_trim(""), "");
        assert_eq!(str_trim("  "), "");
    }

    #[test]
    fn test_str_replace_all() {
        assert_eq!(str_replace_all("foo bar foo", "foo", "baz"), "baz bar baz");
        assert_eq!(str_replace_all("hello", "x", "y"), "hello");
    }

    // ── path helpers ─────────────────────────────────────────────────────

    #[test]
    fn test_ensure_parent_dir() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("a").join("b").join("file.txt");
        ensure_parent_dir(&target).unwrap();
        assert!(dir_exists(&tmp.path().join("a").join("b")));
    }

    #[test]
    fn test_file_exists_and_dir_exists() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("f.txt");
        assert!(!file_exists(&f));
        file_write(&f, b"x").unwrap();
        assert!(file_exists(&f));
        assert!(dir_exists(tmp.path()));
        assert!(!dir_exists(&f));
    }

    #[test]
    fn test_remove_recursive() {
        let tmp = TempDir::new().unwrap();
        let d = tmp.path().join("sub");
        fs::create_dir_all(&d).unwrap();
        file_write(&d.join("f"), b"x").unwrap();
        remove_recursive(&d).unwrap();
        assert!(!d.exists());
    }
}
