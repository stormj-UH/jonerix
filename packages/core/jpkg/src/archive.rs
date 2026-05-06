// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `.jpkg` archive read/write — ported from `jpkg/src/pkg.c`.
//!
//! # Invariants
//!
//! 1. **Magic prefix**: every valid `.jpkg` begins with the 8-byte sequence
//!    `JPKG\x00\x01\x00\x00`.  Any file whose first 8 bytes differ is rejected
//!    with `ArchiveError::BadMagic` before any further parsing.
//!
//! 2. **Header-length bound**: the LE32 `hdr_len` field at bytes `[8..12]`
//!    must satisfy `12 + hdr_len ≤ file_length`.  A value that points past
//!    EOF is rejected with `ArchiveError::BadHeaderLen`.  Callers must not
//!    pass a `hdr_len` value that would overflow a `usize`.
//!
//! 3. **Metadata is UTF-8 TOML**: the metadata block (`bytes[12..12+hdr_len]`)
//!    contains valid UTF-8.  [`JpkgArchive::metadata`] panics if non-UTF-8
//!    bytes are present; use [`JpkgArchive::metadata_str`] for a fallible
//!    alternative.  The C writer never emits non-UTF-8, but callers that
//!    construct synthetic archives must ensure UTF-8 conformance.
//!
//! 4. **Flattened layout on write**: [`create`] and
//!    [`create_with_metadata_factory`] reject any `payload_root` that contains
//!    a top-level `usr/` or `lib64/` directory.  Callers (typically `cmd_build`)
//!    are responsible for running the `jpkg flatten` pass before calling these
//!    functions; violating this assumption produces an
//!    `ArchiveError::UnflatLayout` error and no archive is written.
//!
//! 5. **Atomic writes**: output archives are written to `{out_path}.tmp` and
//!    then renamed into place.  A concurrent reader therefore never observes a
//!    partially written archive.  If the rename fails the `.tmp` file is left
//!    on disk; callers that care about cleanup should remove it.
//!
//! 6. **Deterministic payload order**: [`build_compressed_tar`] sorts entries
//!    by path before adding them to the tar stream.  Given the same input tree,
//!    the compressed payload bytes are identical across runs (modulo zstd
//!    frame-level timestamp differences, which zstd level 0 does not embed).
//!    This is required by `canon::canonical_bytes` which signs the payload hash.
//!
//! # Wire format (verified against pkg.c:173-200 and pkg.h:17-20)
//!
//! ```text
//! [MAGIC      8 bytes]  "JPKG\x00\x01\x00\x00"   — JPKG_MAGIC
//! [HDR_LEN    4 bytes]  u32 little-endian          — byte count of the TOML block
//! [METADATA   variable] UTF-8 TOML, NOT NUL-terminated (pkg.c:141 adds '\0' only
//!                        after copying into a local C buffer, never writes it to disk)
//! [PAYLOAD    rest]     zstd-compressed tar archive
//! ```
//!
//! Minimum valid file: 12 bytes (magic + hdr_len with `hdr_len == 0` and no payload).
//! See `JPKG_HEADER_MIN` in pkg.h:20.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

pub use crate::JPKG_MAGIC;

/// Byte width of the LE32 header-length field.
pub const JPKG_HDR_LEN_BYTES: usize = 4;

// Derived constants matching pkg.h:20.
const MAGIC_LEN: usize = JPKG_MAGIC.len(); // 8
const HEADER_MIN: usize = MAGIC_LEN + JPKG_HDR_LEN_BYTES; // 12

// ──────────────────────────────────────────────────────────────────────────────
// Error type
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ArchiveError {
    /// Underlying I/O failure.
    Io(io::Error),
    /// The first 8 bytes do not match `JPKG_MAGIC`.
    BadMagic { got: [u8; 8] },
    /// The LE32 header-length field points past the end of the file.
    BadHeaderLen(u32),
    /// zstd decompression error (wrapped as an I/O error by the `zstd` crate).
    Zstd(io::Error),
    /// `tar` crate reported an error while reading or writing the tar stream.
    Tar(io::Error),
    /// `payload_root` contains a top-level `usr/` or `lib64/` directory that
    /// should have been flattened before calling `create`.
    UnflatLayout(PathBuf),
    /// The metadata block is not valid UTF-8.
    Utf8(std::str::Utf8Error),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::Io(e) => write!(f, "I/O error: {e}"),
            ArchiveError::BadMagic { got } => {
                write!(f, "invalid .jpkg magic: {:?}", got)
            }
            ArchiveError::BadHeaderLen(n) => {
                write!(f, "header_len {n} exceeds file size")
            }
            ArchiveError::Zstd(e) => write!(f, "zstd error: {e}"),
            ArchiveError::Tar(e) => write!(f, "tar error: {e}"),
            ArchiveError::UnflatLayout(p) => {
                write!(
                    f,
                    "unflattened layout: top-level '{}' found in payload_root; \
                     flatten before calling create()",
                    p.display()
                )
            }
            ArchiveError::Utf8(e) => write!(f, "metadata UTF-8 error: {e}"),
        }
    }
}

impl std::error::Error for ArchiveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ArchiveError::Io(e) | ArchiveError::Zstd(e) | ArchiveError::Tar(e) => Some(e),
            ArchiveError::Utf8(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for ArchiveError {
    fn from(e: io::Error) -> Self {
        ArchiveError::Io(e)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// JpkgArchive (read side)
// ──────────────────────────────────────────────────────────────────────────────

/// A parsed `.jpkg` archive held entirely in memory.
///
/// For large packages the caller may prefer `extract` directly from a path
/// rather than going through `open` + `extract`, but the API keeps both for
/// symmetry with the C `pkg_parse_file` / `pkg_extract` pair.
#[derive(Debug)]
pub struct JpkgArchive {
    // Raw bytes of the TOML metadata block (not NUL-terminated — see pkg.c:141).
    metadata_raw: Vec<u8>,
    // Raw zstd(tar) payload bytes.
    payload_raw: Vec<u8>,
}

impl JpkgArchive {
    // ── constructors ──────────────────────────────────────────────────────────

    /// Open and parse a `.jpkg` file from disk.
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

    /// Parse a `.jpkg` archive from an in-memory byte vector.
    ///
    /// Layout validated against `pkg_parse_buffer` (pkg.c:120-155):
    /// 1. Check length ≥ 12 (`JPKG_HEADER_MIN`).
    /// 2. Compare first 8 bytes to `JPKG_MAGIC` (memcmp).
    /// 3. Read `hdr_len` as LE32 from bytes[8..12].
    /// 4. Ensure `12 + hdr_len ≤ len` (header overflow guard, pkg.c:135).
    /// 5. Metadata = bytes[12 .. 12+hdr_len] — NOT NUL-terminated on disk.
    /// 6. Payload   = bytes[12+hdr_len ..]    — zstd(tar) to EOF.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ArchiveError> {
        if bytes.len() < HEADER_MIN {
            // Map to Io(UnexpectedEof) — matches pkg.c:122-124 ("package too small")
            return Err(ArchiveError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("file is only {} bytes (minimum {})", bytes.len(), HEADER_MIN),
            )));
        }

        // Magic check (pkg.c:22-25, pkg.h:17-20)
        let mut got = [0u8; 8];
        got.copy_from_slice(&bytes[..8]);
        if got != JPKG_MAGIC {
            return Err(ArchiveError::BadMagic { got });
        }

        // LE32 header length (pkg.c:132, uses util.c `read_le32`)
        // SAFETY: the `bytes.len() < HEADER_MIN` guard above ensures
        // bytes.len() >= 12, so bytes[8..12] is exactly 4 bytes.
        // The try_into() cannot fail for a fixed-size slice of the right length.
        let hdr_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let meta_start = HEADER_MIN; // 12
        let meta_end = meta_start
            .checked_add(hdr_len as usize)
            .filter(|&e| e <= bytes.len())
            .ok_or(ArchiveError::BadHeaderLen(hdr_len))?;

        let metadata_raw = bytes[meta_start..meta_end].to_vec();
        let payload_raw = bytes[meta_end..].to_vec();

        Ok(JpkgArchive {
            metadata_raw,
            payload_raw,
        })
    }

    // ── accessors ─────────────────────────────────────────────────────────────

    /// Raw TOML metadata block as a UTF-8 string slice.
    ///
    /// This is the verbatim on-disk bytes — not NUL-terminated.  The caller
    /// should parse with `crate::recipe::Metadata::from_str`.
    ///
    /// # Panics
    ///
    /// Panics if the metadata block is not valid UTF-8.  This is a
    /// programming error: every valid `.jpkg` archive written by this codebase
    /// or by C jpkg 1.1.5 contains only UTF-8 TOML.  Callers that handle
    /// untrusted on-disk data should call [`Self::metadata_str`] instead to
    /// get a `Result` rather than a potential panic.
    pub fn metadata(&self) -> &str {
        // The C writer (pkg.c) always emits valid UTF-8 TOML, so valid archives
        // never reach this branch.  The expect message is left for debugging.
        std::str::from_utf8(&self.metadata_raw)
            .expect("metadata block is not valid UTF-8; use metadata_str() for error handling")
    }

    /// Raw TOML metadata bytes (for callers that need to validate UTF-8
    /// themselves or handle legacy packages with non-UTF-8 metadata).
    pub fn metadata_bytes(&self) -> &[u8] {
        &self.metadata_raw
    }

    /// Validated UTF-8 metadata, returning an error instead of panicking.
    pub fn metadata_str(&self) -> Result<&str, ArchiveError> {
        std::str::from_utf8(&self.metadata_raw).map_err(ArchiveError::Utf8)
    }

    /// The raw zstd-compressed tar payload (everything after the header).
    pub fn payload(&self) -> &[u8] {
        &self.payload_raw
    }

    // ── extraction ────────────────────────────────────────────────────────────

    /// Decompress and extract the payload into `dest`, returning every path
    /// created (for the installed-file manifest).
    ///
    /// Unlike the C `pkg_extract` (pkg.c:213-369) we do NOT:
    /// - shell out to bsdtar / toybox tar / tar (no `Command::new`)
    /// - write a temp .tar file to `/tmp`
    /// - flatten `usr/` (that is the caller's job, matching the audit note)
    ///
    /// The `tar` crate handles symlinks natively (the reason the C code
    /// preferred bsdtar over toybox tar — audit §1 "Symlink handling").
    pub fn extract(&self, dest: &Path) -> Result<Vec<PathBuf>, ArchiveError> {
        fs::create_dir_all(dest)?;
        extract_zstd_tar(&self.payload_raw, dest)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Write side
// ──────────────────────────────────────────────────────────────────────────────

/// Build a `.jpkg` archive at `out_path`.
///
/// Wire format written (pkg.c:173-209):
/// ```text
/// MAGIC(8) | hdr_len_LE32(4) | toml_bytes(hdr_len) | zstd(tar(payload_root/**))
/// ```
///
/// The TOML string is written verbatim — without a trailing NUL — matching the
/// C `pkg_create` which copies `strlen(toml_metadata)` bytes (pkg.c:178, 193).
///
/// # Layout assertion
/// Returns `ArchiveError::UnflatLayout` if `payload_root` has a top-level
/// `usr/` or `lib64/` directory (mirroring `audit_layout_tree` from util.c).
/// Callers (cmd_build) are responsible for flattening before calling this fn.
///
/// # Atomicity
/// Written to `{out_path}.tmp` then renamed into place so a concurrent reader
/// never sees a partial archive.
pub fn create(
    out_path: &Path,
    metadata_toml: &str,
    payload_root: &Path,
) -> Result<(), ArchiveError> {
    // Thin shim over create_with_metadata_factory for callers that already
    // have a fixed metadata blob and don't need the payload-sha-and-size
    // chicken-and-egg fix.  Most production callers should use the factory
    // form so files.sha256 / files.size land filled-in instead of empty.
    create_with_metadata_factory(out_path, payload_root, |_sha, _size| {
        Ok(metadata_toml.to_string())
    })
}

/// Like [`create`], but lets the caller plug in the
/// `files.sha256` / `files.size` of the (zstd-compressed) payload AFTER it
/// has been built.  Resolves the chicken-and-egg in cmd_build.c (and Worker
/// M's previous FIXME): the metadata block is written BEFORE the payload in
/// the .jpkg file, but the payload's sha256/size are part of the metadata.
///
/// The closure receives the 64-char lowercase hex sha256 of the compressed
/// payload and its byte length, and must return the full metadata TOML text
/// to embed.  Typical usage:
///
/// ```ignore
/// archive::create_with_metadata_factory(&out_path, &dest_dir, |sha, size| {
///     let meta = Metadata::from_recipe(recipe, sha.to_string(), size);
///     meta.to_string().map_err(|e| ArchiveError::Io(io::Error::new(
///         io::ErrorKind::Other, e.to_string())))
/// })?;
/// ```
pub fn create_with_metadata_factory<F>(
    out_path: &Path,
    payload_root: &Path,
    metadata_factory: F,
) -> Result<(), ArchiveError>
where
    F: FnOnce(&str, u64) -> Result<String, ArchiveError>,
{
    // ── layout audit (mirrors audit_layout_tree in util.c) ──────────────────
    for banned in &["usr", "lib64"] {
        let candidate = payload_root.join(banned);
        if candidate.exists() {
            return Err(ArchiveError::UnflatLayout(candidate));
        }
    }

    // ── build tar in memory (streaming zstd encoder) ─────────────────────────
    let compressed_payload = build_compressed_tar(payload_root)?;

    // ── compute sha256 + size of the compressed payload ────────────────────
    let payload_sha256 = sha256_hex_of(&compressed_payload);
    let payload_size = compressed_payload.len() as u64;

    // ── caller plugs sha+size into the metadata TOML ───────────────────────
    let metadata_toml = metadata_factory(&payload_sha256, payload_size)?;
    let meta_bytes = metadata_toml.as_bytes();
    let hdr_len = meta_bytes.len() as u32; // pkg.c:178 uses strlen()

    // ── assemble .jpkg header + payload ─────────────────────────────────────
    // Atomic write: write to .tmp then rename (pkg.c uses file_write directly
    // but on POSIX that is not atomic; we improve on the C behaviour here).
    let tmp_path = out_path.with_extension("tmp");
    {
        let mut f = File::create(&tmp_path)?;
        // MAGIC (8 bytes) — pkg.c:185-186
        f.write_all(&JPKG_MAGIC)?;
        // HDR_LEN LE32 (4 bytes) — pkg.c:189-190
        f.write_all(&hdr_len.to_le_bytes())?;
        // TOML metadata (variable, NOT NUL-terminated) — pkg.c:193-194
        f.write_all(meta_bytes)?;
        // zstd payload — pkg.c:197-199
        f.write_all(&compressed_payload)?;
        f.flush()?;
    }
    fs::rename(&tmp_path, out_path)?;

    Ok(())
}

/// 64-char lowercase hex sha256 of `bytes`.  Local helper to avoid a
/// `crate::util` dependency from `archive` (keeps the module self-contained).
fn sha256_hex_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Walk `root` in sorted deterministic order, build a tar stream, zstd-compress
/// it, and return the compressed bytes.
fn build_compressed_tar(root: &Path) -> Result<Vec<u8>, ArchiveError> {
    let out_buf: Vec<u8> = Vec::new();

    // zstd::stream::Encoder<W> — default compression level (3).
    let zstd_enc = zstd::stream::Encoder::new(out_buf, 0)
        .map_err(ArchiveError::Zstd)?;

    // tar::Builder<zstd::Encoder<Vec<u8>>>
    let mut tar_builder = tar::Builder::new(zstd_enc);
    // Preserve mtime and permission bits (default in the tar crate).
    tar_builder.follow_symlinks(false);

    // Collect entries sorted for deterministic output.
    let mut entries: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|e| e.into_path())
        .filter(|p| p != root) // skip root itself
        .collect();
    entries.sort();

    for abs_path in &entries {
        // SAFETY: every entry came from WalkDir::new(root) with the root itself
        // filtered out, so strip_prefix always succeeds.  The unwrap is
        // unreachable in production because WalkDir only yields descendants.
        let rel = abs_path
            .strip_prefix(root)
            .expect("walkdir yields children of root");

        let meta = abs_path.symlink_metadata()?;
        if meta.is_symlink() {
            // tar crate appends a symlink entry preserving the target.
            tar_builder
                .append_path_with_name(abs_path, rel)
                .map_err(ArchiveError::Tar)?;
        } else if meta.is_dir() {
            tar_builder
                .append_path_with_name(abs_path, rel)
                .map_err(ArchiveError::Tar)?;
        } else {
            // Regular file (or other).
            tar_builder
                .append_path_with_name(abs_path, rel)
                .map_err(ArchiveError::Tar)?;
        }
    }

    // Finish the tar stream.
    let zstd_enc = tar_builder.into_inner().map_err(ArchiveError::Tar)?;
    // Finish the zstd stream.
    let compressed = zstd_enc.finish().map_err(ArchiveError::Zstd)?;

    Ok(compressed)
}

/// Decompress `zstd_tar_bytes` and extract the embedded tar into `dest`.
/// Returns the list of paths extracted.
fn extract_zstd_tar(zstd_tar_bytes: &[u8], dest: &Path) -> Result<Vec<PathBuf>, ArchiveError> {
    // zstd::stream::Decoder<&[u8]>
    let decoder =
        zstd::stream::Decoder::new(zstd_tar_bytes).map_err(ArchiveError::Zstd)?;

    let mut archive = tar::Archive::new(decoder);
    // preserve_mtime is true by default in the tar crate.
    // preserve_permissions is true by default.
    // unpack() handles symlinks, directories, and regular files.

    let mut created: Vec<PathBuf> = Vec::new();

    for entry in archive.entries().map_err(ArchiveError::Tar)? {
        let mut entry = entry.map_err(ArchiveError::Tar)?;
        let entry_path = entry.path().map_err(ArchiveError::Tar)?.into_owned();
        entry.unpack_in(dest).map_err(ArchiveError::Tar)?;
        created.push(dest.join(entry_path));
    }

    Ok(created)
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    // ── helpers ───────────────────────────────────────────────────────────────

    /// Build a synthetic DESTDIR tree:
    /// ```
    /// bin/hello        (regular file, "hello world\n")
    /// lib/libx.so.1    (regular file, "\x7fELF")
    /// lib/libx.so      (symlink → libx.so.1)
    /// share/doc/x.txt  (regular file, "docs\n")
    /// ```
    fn make_synthetic_destdir(dir: &Path) {
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::create_dir_all(dir.join("share/doc")).unwrap();

        fs::write(dir.join("bin/hello"), b"hello world\n").unwrap();
        fs::write(dir.join("lib/libx.so.1"), b"\x7fELF").unwrap();
        symlink("libx.so.1", dir.join("lib/libx.so")).unwrap();
        fs::write(dir.join("share/doc/x.txt"), b"docs\n").unwrap();
    }

    const SYNTHETIC_TOML: &str = r#"[package]
name = "synthetic"
version = "0.1.0"
"#;

    // ── 1. Round-trip ─────────────────────────────────────────────────────────

    #[test]
    fn test_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("destdir");
        make_synthetic_destdir(&destdir);

        let out = tmp.path().join("synthetic-0.1.0.jpkg");
        create(&out, SYNTHETIC_TOML, &destdir).expect("create() failed");

        // Open and inspect.
        let arch = JpkgArchive::open(&out).expect("open() failed");
        assert_eq!(arch.metadata().trim(), SYNTHETIC_TOML.trim());

        // Extract to a fresh dir.
        let extract_dir = tmp.path().join("extracted");
        let paths = arch.extract(&extract_dir).expect("extract() failed");
        assert!(!paths.is_empty(), "expected at least one extracted path");

        // Verify regular file contents.
        let hello = fs::read(extract_dir.join("bin/hello")).unwrap();
        assert_eq!(hello, b"hello world\n");

        let libx1 = fs::read(extract_dir.join("lib/libx.so.1")).unwrap();
        assert_eq!(libx1, b"\x7fELF");

        let doc = fs::read(extract_dir.join("share/doc/x.txt")).unwrap();
        assert_eq!(doc, b"docs\n");

        // Verify symlink is a symlink and has the right target.
        let symlink_path = extract_dir.join("lib/libx.so");
        let meta = symlink_path.symlink_metadata().unwrap();
        assert!(meta.file_type().is_symlink(), "lib/libx.so should be a symlink");
        let target = fs::read_link(&symlink_path).unwrap();
        assert_eq!(target.to_str().unwrap(), "libx.so.1");
    }

    // ── 2. Magic check ────────────────────────────────────────────────────────

    #[test]
    fn test_bad_magic() {
        // First 8 bytes are wrong ("XXXX\x00\x01\x00\x00"), rest is a valid
        // LE32 hdr_len=0 so the buffer is at least 12 bytes.
        let mut buf = vec![0u8; 12];
        buf[..4].copy_from_slice(b"XXXX");
        buf[4..8].copy_from_slice(b"\x00\x01\x00\x00");
        // hdr_len = 0 (already zero)

        let err = JpkgArchive::from_bytes(buf).unwrap_err();
        match err {
            ArchiveError::BadMagic { got } => {
                assert_eq!(&got[..4], b"XXXX");
            }
            other => panic!("expected BadMagic, got: {other}"),
        }
    }

    // ── 3. Truncated header ───────────────────────────────────────────────────

    #[test]
    fn test_truncated_header_too_short() {
        // Only 4 bytes — less than HEADER_MIN (12).
        let buf = vec![b'J', b'P', b'K', b'G'];
        let err = JpkgArchive::from_bytes(buf).unwrap_err();
        assert!(
            matches!(err, ArchiveError::Io(_)),
            "expected Io(UnexpectedEof) for 4-byte input, got: {err}"
        );
    }

    #[test]
    fn test_truncated_header_hdr_len_overflow() {
        // Valid magic + hdr_len=0xFFFF_FFFF, but no metadata follows.
        let mut buf = vec![0u8; 12];
        buf[..8].copy_from_slice(&JPKG_MAGIC);
        buf[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = JpkgArchive::from_bytes(buf).unwrap_err();
        assert!(
            matches!(err, ArchiveError::BadHeaderLen(u32::MAX)),
            "expected BadHeaderLen, got: {err}"
        );
    }

    // ── 4. Layout audit ───────────────────────────────────────────────────────

    #[test]
    fn test_unflat_layout_usr() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("destdir");
        // Create a usr/ subdirectory — this should be rejected.
        fs::create_dir_all(destdir.join("usr/bin")).unwrap();
        fs::write(destdir.join("usr/bin/foo"), b"foo").unwrap();

        let out = tmp.path().join("bad.jpkg");
        let err = create(&out, SYNTHETIC_TOML, &destdir).unwrap_err();
        match err {
            ArchiveError::UnflatLayout(p) => {
                assert!(
                    p.ends_with("usr"),
                    "expected path ending in 'usr', got: {p:?}"
                );
            }
            other => panic!("expected UnflatLayout, got: {other}"),
        }
    }

    #[test]
    fn test_unflat_layout_lib64() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("destdir");
        fs::create_dir_all(destdir.join("lib64")).unwrap();
        fs::write(destdir.join("lib64/ld.so"), b"stub").unwrap();

        let out = tmp.path().join("bad.jpkg");
        let err = create(&out, SYNTHETIC_TOML, &destdir).unwrap_err();
        assert!(
            matches!(err, ArchiveError::UnflatLayout(_)),
            "expected UnflatLayout for lib64, got: {err}"
        );
    }

    // ── 5. Real .jpkg sniff (skip if none present) ───────────────────────────

    #[test]
    fn test_real_jpkg_sniff() {
        use std::ffi::OsStr;

        // Search common output locations relative to the workspace root.
        let search_roots = [
            Path::new("/Users/jonerik/Desktop/jonerix/target/release"),
            Path::new("/Users/jonerik/Desktop/jonerix/out/jpkgs"),
            Path::new("/Users/jonerik/Desktop/jonerix/packages"),
        ];

        let jpkg_file: Option<PathBuf> = search_roots.iter().find_map(|root| {
            if !root.exists() {
                return None;
            }
            WalkDir::new(root)
                .max_depth(4)
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|e| e.into_path())
                .find(|p| p.extension() == Some(OsStr::new("jpkg")))
        });

        match jpkg_file {
            None => {
                eprintln!("test_real_jpkg_sniff: no .jpkg found in search roots — skipping");
            }
            Some(path) => {
                eprintln!("test_real_jpkg_sniff: found {}", path.display());
                let arch = JpkgArchive::open(&path)
                    .unwrap_or_else(|e| panic!("failed to open {}: {e}", path.display()));
                let meta = arch.metadata_str().expect("metadata not valid UTF-8");
                assert!(!meta.is_empty(), "metadata should be non-empty TOML");
                assert!(
                    meta.contains('[') || meta.contains('='),
                    "metadata doesn't look like TOML: {meta:?}"
                );
                // Payload may legitimately be empty for meta-packages; just
                // check that we can access the slice.
                let _ = arch.payload();
            }
        }
    }

    // ── 6. Symlink preservation ───────────────────────────────────────────────

    #[test]
    fn test_symlink_preserved_after_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("sym_destdir");
        fs::create_dir_all(destdir.join("lib")).unwrap();
        fs::write(destdir.join("lib/real.so.1"), b"\x7fELF stub").unwrap();
        symlink("real.so.1", destdir.join("lib/real.so")).unwrap();

        let out = tmp.path().join("sym-test.jpkg");
        create(&out, SYNTHETIC_TOML, &destdir).expect("create() failed");

        let arch = JpkgArchive::open(&out).expect("open() failed");
        let extract_dir = tmp.path().join("sym_extract");
        arch.extract(&extract_dir).expect("extract() failed");

        let sym_path = extract_dir.join("lib/real.so");
        let sym_meta = sym_path.symlink_metadata().expect("real.so not found after extract");
        assert!(
            sym_meta.file_type().is_symlink(),
            "lib/real.so should be a symlink after extraction"
        );
        let target = fs::read_link(&sym_path).expect("read_link failed");
        assert_eq!(target.to_str().unwrap(), "real.so.1");

        // The symlink target should be readable.
        let contents = fs::read(&sym_path).expect("reading through symlink failed");
        assert_eq!(contents, b"\x7fELF stub");
    }
}
