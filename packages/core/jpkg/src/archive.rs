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
//!    must satisfy `12 + hdr_len <= file_length`.  A value that points past
//!    EOF is rejected with `ArchiveError::BadHeaderLen`.
//!
//! 3. **Metadata is UTF-8 TOML**: the metadata block contains valid UTF-8.
//!    [`JpkgArchive::metadata`] panics if non-UTF-8 bytes are present; use
//!    [`JpkgArchive::metadata_str`] for a fallible alternative.
//!
//! 4. **Flattened layout on write**: [`create`] and
//!    [`create_with_metadata_factory`] reject any `payload_root` that contains
//!    a top-level `usr/` or `lib64/` directory.
//!
//! 5. **Atomic writes**: output archives are written to a randomly-named temp
//!    file in the same directory as `out_path`, then renamed into place.  This
//!    prevents both torn-write exposure and symlink-based temp-file hijacking
//!    by local attackers (the old `{out_path}.tmp` was predictable).
//!
//! 6. **Deterministic payload order**: [`build_compressed_tar`] sorts entries
//!    by path before adding them to the tar stream.
//!
//! # Wire format
//!
//! ```text
//! [MAGIC      8 bytes]  "JPKG\x00\x01\x00\x00"
//! [HDR_LEN    4 bytes]  u32 little-endian
//! [METADATA   variable] UTF-8 TOML, NOT NUL-terminated
//! [PAYLOAD    rest]     zstd-compressed tar archive
//! ```

use std::fs::{self};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

pub use crate::JPKG_MAGIC;

/// Byte width of the LE32 header-length field.
pub const JPKG_HDR_LEN_BYTES: usize = 4;

const MAGIC_LEN: usize = JPKG_MAGIC.len(); // 8
const HEADER_MIN: usize = MAGIC_LEN + JPKG_HDR_LEN_BYTES; // 12

// ──────────────────────────────────────────────────────────────────────────────
// Error type
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ArchiveError {
    Io(io::Error),
    BadMagic { got: [u8; 8] },
    BadHeaderLen(u32),
    Zstd(io::Error),
    Tar(io::Error),
    UnflatLayout(PathBuf),
    Utf8(std::str::Utf8Error),
}

impl std::fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveError::Io(e) => write!(f, "I/O error: {e}"),
            ArchiveError::BadMagic { got } => write!(f, "invalid .jpkg magic: {:?}", got),
            ArchiveError::BadHeaderLen(n) => write!(f, "header_len {n} exceeds file size"),
            ArchiveError::Zstd(e) => write!(f, "zstd error: {e}"),
            ArchiveError::Tar(e) => write!(f, "tar error: {e}"),
            ArchiveError::UnflatLayout(p) => write!(
                f,
                "unflattened layout: top-level '{}' found in payload_root; \
                 flatten before calling create()",
                p.display()
            ),
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

#[derive(Debug)]
pub struct JpkgArchive {
    metadata_raw: Vec<u8>,
    payload_raw: Vec<u8>,
}

impl JpkgArchive {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let bytes = fs::read(path)?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, ArchiveError> {
        if bytes.len() < HEADER_MIN {
            return Err(ArchiveError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("file is only {} bytes (minimum {})", bytes.len(), HEADER_MIN),
            )));
        }

        let mut got = [0u8; 8];
        got.copy_from_slice(&bytes[..8]);
        if got != JPKG_MAGIC {
            return Err(ArchiveError::BadMagic { got });
        }

        let hdr_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let meta_start = HEADER_MIN;
        let meta_end = meta_start
            .checked_add(hdr_len as usize)
            .filter(|&e| e <= bytes.len())
            .ok_or(ArchiveError::BadHeaderLen(hdr_len))?;

        let metadata_raw = bytes[meta_start..meta_end].to_vec();
        let payload_raw = bytes[meta_end..].to_vec();

        Ok(JpkgArchive { metadata_raw, payload_raw })
    }

    pub fn metadata(&self) -> &str {
        std::str::from_utf8(&self.metadata_raw)
            .expect("metadata block is not valid UTF-8; use metadata_bytes() for raw access")
    }

    pub fn metadata_bytes(&self) -> &[u8] {
        &self.metadata_raw
    }

    pub fn metadata_str(&self) -> Result<&str, ArchiveError> {
        std::str::from_utf8(&self.metadata_raw).map_err(ArchiveError::Utf8)
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload_raw
    }

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
/// Uses a randomly-named tempfile for the write so that a local attacker
/// cannot pre-place a symlink at a predictable path to redirect the output.
pub fn create(
    out_path: &Path,
    metadata_toml: &str,
    payload_root: &Path,
) -> Result<(), ArchiveError> {
    create_with_metadata_factory(out_path, payload_root, |_sha, _size| {
        Ok(metadata_toml.to_string())
    })
}

pub fn create_with_metadata_factory<F>(
    out_path: &Path,
    payload_root: &Path,
    metadata_factory: F,
) -> Result<(), ArchiveError>
where
    F: FnOnce(&str, u64) -> Result<String, ArchiveError>,
{
    // Layout audit
    for banned in &["usr", "lib64"] {
        let candidate = payload_root.join(banned);
        if candidate.exists() {
            return Err(ArchiveError::UnflatLayout(candidate));
        }
    }

    let compressed_payload = build_compressed_tar(payload_root)?;
    let payload_sha256 = sha256_hex_of(&compressed_payload);
    let payload_size = compressed_payload.len() as u64;

    let metadata_toml = metadata_factory(&payload_sha256, payload_size)?;
    let meta_bytes = metadata_toml.as_bytes();
    let hdr_len = meta_bytes.len() as u32;

    // Security fix: use a randomly-named tempfile instead of the predictable
    // `{out_path}.tmp`.  A predictable name lets a local attacker pre-place a
    // symlink at that path to redirect the write to an arbitrary destination.
    // tempfile::Builder creates the file with O_EXCL, guaranteeing uniqueness.
    let out_dir = out_path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_file = tempfile::Builder::new()
        .prefix(".jpkg-write-")
        .suffix(".tmp")
        .tempfile_in(out_dir)
        .map_err(ArchiveError::Io)?;

    {
        let mut f = tmp_file.as_file();
        f.write_all(&JPKG_MAGIC)?;
        f.write_all(&hdr_len.to_le_bytes())?;
        f.write_all(meta_bytes)?;
        f.write_all(&compressed_payload)?;
        f.flush()?;
    }

    tmp_file
        .persist(out_path)
        .map_err(|e| ArchiveError::Io(e.error))?;

    Ok(())
}

fn sha256_hex_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

// ──────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────────

fn build_compressed_tar(root: &Path) -> Result<Vec<u8>, ArchiveError> {
    let out_buf: Vec<u8> = Vec::new();
    let zstd_enc = zstd::stream::Encoder::new(out_buf, 0).map_err(ArchiveError::Zstd)?;
    let mut tar_builder = tar::Builder::new(zstd_enc);
    tar_builder.follow_symlinks(false);

    let mut entries: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .map(|e| e.into_path())
        .filter(|p| p != root)
        .collect();
    entries.sort();

    for abs_path in &entries {
        let rel = abs_path.strip_prefix(root).expect("walkdir yields children of root");
        tar_builder
            .append_path_with_name(abs_path, rel)
            .map_err(ArchiveError::Tar)?;
    }

    let zstd_enc = tar_builder.into_inner().map_err(ArchiveError::Tar)?;
    let compressed = zstd_enc.finish().map_err(ArchiveError::Zstd)?;
    Ok(compressed)
}

/// Decompress `zstd_tar_bytes` and extract the embedded tar into `dest`.
///
/// # Security
///
/// Before any filesystem I/O, two attacks are blocked:
///
/// 1. **Path traversal**: entry paths containing `..` that escape `dest`, or
///    absolute paths (e.g. `../../etc/shadow`, `/etc/shadow`).
///
/// 2. **Symlink-through-extraction-root**: a symlink entry whose target, when
///    resolved relative to its parent directory inside `dest`, escapes `dest`.
///    Absolute symlink targets are always rejected.
fn extract_zstd_tar(zstd_tar_bytes: &[u8], dest: &Path) -> Result<Vec<PathBuf>, ArchiveError> {
    let decoder = zstd::stream::Decoder::new(zstd_tar_bytes).map_err(ArchiveError::Zstd)?;
    let mut archive = tar::Archive::new(decoder);

    let mut created: Vec<PathBuf> = Vec::new();

    for entry in archive.entries().map_err(ArchiveError::Tar)? {
        let mut entry = entry.map_err(ArchiveError::Tar)?;
        let entry_path = entry.path().map_err(ArchiveError::Tar)?.into_owned();

        // Security check 1: path traversal
        validate_entry_path(dest, &entry_path).map_err(|msg| {
            ArchiveError::Io(io::Error::new(io::ErrorKind::InvalidData, msg))
        })?;

        // Security check 2: symlink target must stay within dest
        if entry.header().entry_type().is_symlink() {
            let link_target = entry
                .link_name()
                .map_err(ArchiveError::Tar)?
                .ok_or_else(|| {
                    ArchiveError::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("symlink entry {:?} has no link name", entry_path),
                    ))
                })?
                .into_owned();

            let symlink_parent = dest.join(
                entry_path.parent().unwrap_or_else(|| std::path::Path::new("")),
            );

            validate_symlink_target(&symlink_parent, &link_target, dest).map_err(|msg| {
                ArchiveError::Io(io::Error::new(io::ErrorKind::InvalidData, msg))
            })?;
        }

        entry.unpack_in(dest).map_err(ArchiveError::Tar)?;
        created.push(dest.join(&entry_path));
    }

    Ok(created)
}

// ── Path-arithmetic security helpers ─────────────────────────────────────────

/// Validate that `entry_path`, when resolved inside `dest`, stays within `dest`.
///
/// Pure path arithmetic — no filesystem calls.
pub(crate) fn validate_entry_path(dest: &Path, entry_path: &Path) -> Result<(), String> {
    if entry_path.is_absolute() {
        return Err(format!(
            "archive entry has absolute path: {:?}",
            entry_path
        ));
    }

    let mut depth: i64 = 0;
    for component in entry_path.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!(
                        "archive entry path escapes extraction root: {:?}",
                        entry_path
                    ));
                }
            }
            Component::CurDir => {}
            Component::Normal(part) => {
                if part.as_encoded_bytes().contains(&0u8) {
                    return Err(format!(
                        "archive entry path contains NUL byte: {:?}",
                        entry_path
                    ));
                }
                depth += 1;
            }
            _ => {
                return Err(format!(
                    "archive entry path has unexpected component: {:?}",
                    entry_path
                ));
            }
        }
    }

    // Belt-and-suspenders: compare the lexically-normalised final path against
    // the canonical dest.  We use lexical normalisation of the entry path (not
    // canonicalize of the final resolved path) to avoid following symlinks
    // already placed by earlier entries in this extraction.
    //
    // We must root `lexical` at `canon_dest` (not raw `dest`) because on some
    // systems (e.g. macOS where /tmp → /private/tmp) `dest` may contain
    // symlink components; starts_with would spuriously fail if both sides are
    // not in the same canonical namespace.
    if dest.exists() {
        if let Ok(canon_dest) = std::fs::canonicalize(dest) {
            let mut lexical = canon_dest.clone();
            for component in entry_path.components() {
                use std::path::Component;
                match component {
                    Component::Normal(p) => lexical.push(p),
                    Component::ParentDir => { lexical.pop(); }
                    Component::CurDir => {}
                    _ => {}
                }
            }
            if !lexical.starts_with(&canon_dest) {
                return Err(format!(
                    "archive entry {:?} resolves outside extraction root {:?}",
                    entry_path, dest
                ));
            }
        }
    }

    Ok(())
}

/// Validate that `link_target`, resolved relative to `symlink_parent` inside
/// `dest`, stays within `dest`.  Absolute targets are always rejected.
///
/// Pure depth arithmetic — no filesystem calls.
pub(crate) fn validate_symlink_target(
    symlink_parent: &Path,
    link_target: &Path,
    dest: &Path,
) -> Result<(), String> {
    if link_target.is_absolute() {
        return Err(format!(
            "symlink target is absolute and would escape extraction root: {:?}",
            link_target
        ));
    }

    let dest_depth = dest.components().count();
    let parent_depth = symlink_parent.components().count();
    let mut depth = parent_depth.saturating_sub(dest_depth) as i64;

    for component in link_target.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(format!(
                        "symlink target {:?} escapes extraction root {:?}",
                        link_target, dest
                    ));
                }
            }
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            _ => {
                return Err(format!(
                    "symlink target has unexpected component: {:?}",
                    link_target
                ));
            }
        }
    }

    Ok(())
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

    fn make_synthetic_destdir(dir: &Path) {
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::create_dir_all(dir.join("share/doc")).unwrap();

        fs::write(dir.join("bin/hello"), b"hello world\n").unwrap();
        fs::write(dir.join("lib/libx.so.1"), b"\x7fELF").unwrap();
        symlink("libx.so.1", dir.join("lib/libx.so")).unwrap();
        fs::write(dir.join("share/doc/x.txt"), b"docs\n").unwrap();
    }

    const SYNTHETIC_TOML: &str = "[package]\nname = \"synthetic\"\nversion = \"0.1.0\"\n";

    // ── 1. Round-trip ─────────────────────────────────────────────────────────

    #[test]
    fn test_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("destdir");
        make_synthetic_destdir(&destdir);

        let out = tmp.path().join("synthetic-0.1.0.jpkg");
        create(&out, SYNTHETIC_TOML, &destdir).expect("create() failed");

        let arch = JpkgArchive::open(&out).expect("open() failed");
        assert_eq!(arch.metadata().trim(), SYNTHETIC_TOML.trim());

        let extract_dir = tmp.path().join("extracted");
        let paths = arch.extract(&extract_dir).expect("extract() failed");
        assert!(!paths.is_empty(), "expected at least one extracted path");

        assert_eq!(fs::read(extract_dir.join("bin/hello")).unwrap(), b"hello world\n");
        assert_eq!(fs::read(extract_dir.join("lib/libx.so.1")).unwrap(), b"\x7fELF");
        assert_eq!(fs::read(extract_dir.join("share/doc/x.txt")).unwrap(), b"docs\n");

        let symlink_path = extract_dir.join("lib/libx.so");
        let meta = symlink_path.symlink_metadata().unwrap();
        assert!(meta.file_type().is_symlink(), "lib/libx.so should be a symlink");
        assert_eq!(fs::read_link(&symlink_path).unwrap().to_str().unwrap(), "libx.so.1");
    }

    // ── 2. Magic check ────────────────────────────────────────────────────────

    #[test]
    fn test_bad_magic() {
        let mut buf = vec![0u8; 12];
        buf[..4].copy_from_slice(b"XXXX");
        buf[4..8].copy_from_slice(b"\x00\x01\x00\x00");
        let err = JpkgArchive::from_bytes(buf).unwrap_err();
        match err {
            ArchiveError::BadMagic { got } => assert_eq!(&got[..4], b"XXXX"),
            other => panic!("expected BadMagic, got: {other}"),
        }
    }

    // ── 3. Truncated header ───────────────────────────────────────────────────

    #[test]
    fn test_truncated_header_too_short() {
        let buf = vec![b'J', b'P', b'K', b'G'];
        let err = JpkgArchive::from_bytes(buf).unwrap_err();
        assert!(matches!(err, ArchiveError::Io(_)), "expected Io(UnexpectedEof): {err}");
    }

    #[test]
    fn test_truncated_header_hdr_len_overflow() {
        let mut buf = vec![0u8; 12];
        buf[..8].copy_from_slice(&JPKG_MAGIC);
        buf[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = JpkgArchive::from_bytes(buf).unwrap_err();
        assert!(
            matches!(err, ArchiveError::BadHeaderLen(u32::MAX)),
            "expected BadHeaderLen: {err}"
        );
    }

    // ── 4. Layout audit ───────────────────────────────────────────────────────

    #[test]
    fn test_unflat_layout_usr() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("destdir");
        fs::create_dir_all(destdir.join("usr/bin")).unwrap();
        fs::write(destdir.join("usr/bin/foo"), b"foo").unwrap();

        let out = tmp.path().join("bad.jpkg");
        let err = create(&out, SYNTHETIC_TOML, &destdir).unwrap_err();
        match err {
            ArchiveError::UnflatLayout(p) => assert!(p.ends_with("usr")),
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
        assert!(matches!(err, ArchiveError::UnflatLayout(_)), "expected UnflatLayout: {err}");
    }

    // ── 5. Real .jpkg sniff (skip if none present) ───────────────────────────

    #[test]
    fn test_real_jpkg_sniff() {
        use std::ffi::OsStr;
        let search_roots = [
            Path::new("/Users/jonerik/Desktop/jonerix/target/release"),
            Path::new("/Users/jonerik/Desktop/jonerix/out/jpkgs"),
            Path::new("/Users/jonerik/Desktop/jonerix/packages"),
        ];
        let jpkg_file: Option<PathBuf> = search_roots.iter().find_map(|root| {
            if !root.exists() { return None; }
            WalkDir::new(root).max_depth(4).into_iter()
                .filter_map(|e| e.ok()).map(|e| e.into_path())
                .find(|p| p.extension() == Some(OsStr::new("jpkg")))
        });
        match jpkg_file {
            None => eprintln!("test_real_jpkg_sniff: no .jpkg found — skipping"),
            Some(path) => {
                let arch = JpkgArchive::open(&path)
                    .unwrap_or_else(|e| panic!("failed to open {}: {e}", path.display()));
                let meta = arch.metadata_str().expect("metadata not valid UTF-8");
                assert!(!meta.is_empty());
                assert!(meta.contains('[') || meta.contains('='));
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
        assert!(sym_meta.file_type().is_symlink());
        assert_eq!(fs::read_link(&sym_path).unwrap().to_str().unwrap(), "real.so.1");
        assert_eq!(fs::read(&sym_path).unwrap(), b"\x7fELF stub");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Security regression tests
    // ─────────────────────────────────────────────────────────────────────────

    // Helper: craft a raw ustar tar entry (regular file).
    fn append_tar_file_entry(buf: &mut Vec<u8>, path: &str, content: &[u8]) {
        let mut hdr = [0u8; 512];
        let name_bytes = path.as_bytes();
        hdr[..name_bytes.len().min(100)].copy_from_slice(&name_bytes[..name_bytes.len().min(100)]);
        hdr[100..108].copy_from_slice(b"0000644\0");
        hdr[108..116].copy_from_slice(b"0000000\0");
        hdr[116..124].copy_from_slice(b"0000000\0");
        let size_octal = format!("{:011o}\0", content.len());
        hdr[124..136].copy_from_slice(size_octal.as_bytes());
        hdr[136..148].copy_from_slice(b"00000000000\0");
        hdr[148..156].copy_from_slice(b"        ");
        hdr[156] = b'0'; // regular file
        hdr[257..263].copy_from_slice(b"ustar ");
        hdr[263] = b' ';
        let cksum: u32 = hdr.iter().map(|&b| b as u32).sum();
        let cksum_str = format!("{:06o}\0 ", cksum);
        hdr[148..156].copy_from_slice(cksum_str.as_bytes());
        buf.extend_from_slice(&hdr);
        buf.extend_from_slice(content);
        let pad = (512 - (content.len() % 512)) % 512;
        buf.extend(std::iter::repeat(0u8).take(pad));
    }

    // Helper: craft a raw ustar tar entry (symlink).
    fn append_tar_symlink_entry(buf: &mut Vec<u8>, path: &str, link_target: &str) {
        let mut hdr = [0u8; 512];
        let name_bytes = path.as_bytes();
        hdr[..name_bytes.len().min(100)].copy_from_slice(&name_bytes[..name_bytes.len().min(100)]);
        hdr[100..108].copy_from_slice(b"0000777\0");
        hdr[108..116].copy_from_slice(b"0000000\0");
        hdr[116..124].copy_from_slice(b"0000000\0");
        hdr[124..136].copy_from_slice(b"00000000000\0");
        hdr[136..148].copy_from_slice(b"00000000000\0");
        hdr[148..156].copy_from_slice(b"        ");
        hdr[156] = b'2'; // symlink
        let tgt = link_target.as_bytes();
        hdr[157..157 + tgt.len().min(100)].copy_from_slice(&tgt[..tgt.len().min(100)]);
        hdr[257..263].copy_from_slice(b"ustar ");
        hdr[263] = b' ';
        let cksum: u32 = hdr.iter().map(|&b| b as u32).sum();
        let cksum_str = format!("{:06o}\0 ", cksum);
        hdr[148..156].copy_from_slice(cksum_str.as_bytes());
        buf.extend_from_slice(&hdr);
    }

    /// Wrap a raw tar byte stream into a minimal .jpkg envelope (zstd payload).
    fn make_malicious_jpkg(tar_bytes: &[u8]) -> Vec<u8> {
        let mut full_tar = tar_bytes.to_vec();
        full_tar.extend(std::iter::repeat(0u8).take(1024)); // end-of-archive blocks
        let compressed = zstd::encode_all(full_tar.as_slice(), 1).unwrap();
        let meta = b"[package]\nname = \"evil\"\nversion = \"0.0.0\"\n";
        let hdr_len = meta.len() as u32;
        let mut jpkg = Vec::new();
        jpkg.extend_from_slice(&JPKG_MAGIC);
        jpkg.extend_from_slice(&hdr_len.to_le_bytes());
        jpkg.extend_from_slice(meta);
        jpkg.extend_from_slice(&compressed);
        jpkg
    }

    // SEC-1: path traversal via `../../` in entry name must be rejected.
    //
    // A malicious .jpkg containing `../../etc/shadow` must be rejected before
    // any bytes reach the filesystem.
    #[test]
    fn test_sec_path_traversal_dotdot_rejected() {
        let mut tar_buf = Vec::new();
        append_tar_file_entry(&mut tar_buf, "../../etc/shadow", b"root:x:0:0\n");
        let arch = JpkgArchive::from_bytes(make_malicious_jpkg(&tar_buf)).unwrap();

        let tmp = TempDir::new().unwrap();
        let extract_dir = tmp.path().join("extract");
        fs::create_dir_all(&extract_dir).unwrap();

        let result = arch.extract(&extract_dir);
        assert!(result.is_err(), "path-traversal entry must be rejected");
        match result.unwrap_err() {
            ArchiveError::Io(e) => {
                assert_eq!(e.kind(), io::ErrorKind::InvalidData);
                assert!(e.to_string().contains("escapes extraction root"), "{e}");
            }
            other => panic!("expected Io(InvalidData), got: {other}"),
        }
        // The file must not have been created anywhere under tmp.
        assert!(!tmp.path().join("etc/shadow").exists());
    }

    // SEC-2: absolute path entry must be rejected.
    #[test]
    fn test_sec_absolute_path_rejected() {
        let mut tar_buf = Vec::new();
        append_tar_file_entry(&mut tar_buf, "/etc/motd", b"pwned\n");
        let arch = JpkgArchive::from_bytes(make_malicious_jpkg(&tar_buf)).unwrap();

        let tmp = TempDir::new().unwrap();
        let extract_dir = tmp.path().join("extract");
        fs::create_dir_all(&extract_dir).unwrap();

        let result = arch.extract(&extract_dir);
        assert!(result.is_err(), "absolute path entry must be rejected");
        match result.unwrap_err() {
            ArchiveError::Io(e) => assert_eq!(e.kind(), io::ErrorKind::InvalidData),
            other => panic!("expected Io(InvalidData), got: {other}"),
        }
    }

    // SEC-3: symlink with absolute target must be rejected.
    #[test]
    fn test_sec_absolute_symlink_target_rejected() {
        let mut tar_buf = Vec::new();
        append_tar_symlink_entry(&mut tar_buf, "lib/evil", "/etc");
        let arch = JpkgArchive::from_bytes(make_malicious_jpkg(&tar_buf)).unwrap();

        let tmp = TempDir::new().unwrap();
        let extract_dir = tmp.path().join("extract");
        fs::create_dir_all(extract_dir.join("lib")).unwrap();

        let result = arch.extract(&extract_dir);
        assert!(result.is_err(), "absolute symlink target must be rejected");
        match result.unwrap_err() {
            ArchiveError::Io(e) => {
                assert_eq!(e.kind(), io::ErrorKind::InvalidData);
                assert!(e.to_string().contains("absolute"), "{e}");
            }
            other => panic!("expected Io(InvalidData), got: {other}"),
        }
        assert!(!extract_dir.join("lib/evil").symlink_metadata().is_ok());
    }

    // SEC-4: two-step symlink-through-extraction-root attack must be rejected.
    //
    // Entry 1: `escape -> ../..`  (symlink, target escapes dest two levels up)
    // Entry 2: `escape/shadow`    (file written through the symlink)
    //
    // The validation must block entry 1 before it is written to disk.
    #[test]
    fn test_sec_symlink_through_extraction_root_rejected() {
        let mut tar_buf = Vec::new();
        append_tar_symlink_entry(&mut tar_buf, "escape", "../..");
        append_tar_file_entry(&mut tar_buf, "escape/shadow", b"written-through-symlink\n");
        let arch = JpkgArchive::from_bytes(make_malicious_jpkg(&tar_buf)).unwrap();

        let tmp = TempDir::new().unwrap();
        let extract_dir = tmp.path().join("extract");
        fs::create_dir_all(&extract_dir).unwrap();

        let result = arch.extract(&extract_dir);
        assert!(result.is_err(), "escape symlink must be rejected");
        assert!(!extract_dir.join("escape").symlink_metadata().is_ok());
    }

    // SEC-5: valid relative symlink within dest must still work after the fix.
    #[test]
    fn test_sec_valid_relative_symlink_allowed() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("destdir");
        fs::create_dir_all(destdir.join("lib")).unwrap();
        fs::write(destdir.join("lib/libfoo.so.1"), b"\x7fELF").unwrap();
        symlink("libfoo.so.1", destdir.join("lib/libfoo.so")).unwrap();

        let out = tmp.path().join("pkg.jpkg");
        create(&out, SYNTHETIC_TOML, &destdir).unwrap();

        let arch = JpkgArchive::open(&out).unwrap();
        let extract_dir = tmp.path().join("extract");
        arch.extract(&extract_dir).expect("valid relative symlink must extract");

        let sym = extract_dir.join("lib/libfoo.so");
        assert!(sym.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&sym).unwrap().to_str().unwrap(), "libfoo.so.1");
    }

    // SEC-6: atomic write uses unpredictable temp name, not `{out_path}.tmp`.
    //
    // The old code used `out_path.with_extension("tmp")` which is predictable.
    // A local attacker could pre-place a symlink there to redirect the write.
    // After the fix, we use a randomly-named tempfile via tempfile::Builder.
    #[test]
    fn test_sec_create_no_predictable_tmp_file() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path().join("destdir");
        fs::create_dir_all(destdir.join("bin")).unwrap();
        fs::write(destdir.join("bin/hello"), b"hello").unwrap();

        let out = tmp.path().join("test.jpkg");
        create(&out, SYNTHETIC_TOML, &destdir).expect("create() must succeed");

        // Output file must exist.
        assert!(out.exists(), "output .jpkg must exist");

        // The old predictable .tmp path must NOT exist after create().
        let old_tmp = out.with_extension("tmp");
        assert!(
            !old_tmp.exists(),
            "predictable .tmp file must not remain: {:?}",
            old_tmp
        );

        // No .jpkg-write-*.tmp files must remain (tempfile cleans up on persist).
        let leftover: Vec<_> = fs::read_dir(tmp.path()).unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".jpkg-write-") && n.ends_with(".tmp"))
            .collect();
        assert!(
            leftover.is_empty(),
            "no leftover .jpkg-write-*.tmp files: {:?}",
            leftover
        );
    }

    // Unit tests for the path-validation helpers.

    #[test]
    fn test_validate_entry_path_rejects_dotdot() {
        let dest = Path::new("/dest");
        assert!(validate_entry_path(dest, Path::new("../../etc/shadow")).is_err());
        assert!(validate_entry_path(dest, Path::new("../sibling")).is_err());
    }

    #[test]
    fn test_validate_entry_path_rejects_absolute() {
        let dest = Path::new("/dest");
        assert!(validate_entry_path(dest, Path::new("/etc/shadow")).is_err());
    }

    #[test]
    fn test_validate_entry_path_allows_normal_paths() {
        let dest = Path::new("/dest");
        assert!(validate_entry_path(dest, Path::new("bin/sh")).is_ok());
        assert!(validate_entry_path(dest, Path::new("./bin/sh")).is_ok());
        // a/b/../c stays within dest
        assert!(validate_entry_path(dest, Path::new("a/b/../c")).is_ok());
    }

    #[test]
    fn test_validate_symlink_target_rejects_absolute() {
        let dest = Path::new("/dest");
        let parent = Path::new("/dest/lib");
        assert!(validate_symlink_target(parent, Path::new("/etc"), dest).is_err());
    }

    #[test]
    fn test_validate_symlink_target_rejects_escape() {
        let dest = Path::new("/dest");
        let parent = Path::new("/dest/lib");
        // ../../ from /dest/lib reaches /, above /dest
        assert!(validate_symlink_target(parent, Path::new("../../"), dest).is_err());
        assert!(validate_symlink_target(parent, Path::new("../.."), dest).is_err());
    }

    #[test]
    fn test_validate_symlink_target_allows_normal_relative() {
        let dest = Path::new("/dest");
        let parent = Path::new("/dest/lib");
        assert!(validate_symlink_target(parent, Path::new("libfoo.so.1"), dest).is_ok());
        // ../bin/sh goes dest/lib -> dest/bin/sh, still within dest
        assert!(validate_symlink_target(parent, Path::new("../bin/sh"), dest).is_ok());
        // ../ alone reaches dest itself, which is within dest (depth >= 0)
        assert!(validate_symlink_target(parent, Path::new("../"), dest).is_ok());
    }
}
