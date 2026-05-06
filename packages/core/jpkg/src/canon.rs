// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! Canonical bytes for per-package signature.
//!
//! # Invariants
//!
//! 1. **Determinism**: [`canonical_bytes`] is a pure function.  Given the same
//!    `Metadata` struct content (ignoring the `signature` field) and the same
//!    `payload_sha256`, it always returns the same byte sequence.  Any change
//!    to the algorithm, field ordering, or prefix bytes constitutes a breaking
//!    change and must be accompanied by a [`CANON_VERSION`] increment.
//!
//! 2. **Signature-field stripping**: the `[signature]` section is removed from
//!    `Metadata` before serialisation.  This ensures that the canonical bytes
//!    are identical whether or not a signature is already embedded, solving the
//!    self-referential problem: you can verify a signed archive without first
//!    stripping the signature block manually.  Callers must not pre-strip the
//!    signature before calling this function; doing so is harmless but redundant.
//!
//! 3. **Payload binding**: the 32-byte raw SHA-256 of the zstd-compressed tar
//!    payload is embedded in the canonical bytes.  This cryptographically binds
//!    the metadata to a specific payload; a signature on canonical bytes
//!    `(M, H)` cannot be reused for `(M, H')` where `H ≠ H'`.  Callers must
//!    pass the SHA-256 of the *payload bytes* (everything after the 12-byte
//!    file header), not the SHA-256 of the whole `.jpkg` file.
//!
//! 4. **Version prefix**: every canonical-bytes buffer begins with the 11-byte
//!    tag `"jpkg-canon\0"` followed by [`CANON_VERSION`] (currently `1`).
//!    Verifiers must check the prefix and version byte before attempting
//!    signature verification.  A mismatch indicates the archive was signed with
//!    a different version of this algorithm.
//!
//! 5. **TOML serialisation contract**: the metadata body is produced by
//!    `toml::to_string` applied to the signature-stripped struct.  This is
//!    deterministic for a fixed struct layout with the `toml` crate; however
//!    any change to the field order in `Metadata` or its sub-structs will
//!    change the serialised output and invalidate all previously computed
//!    canonical bytes (and hence all signatures).
//!
//! The signed message is constructed deterministically from the
//! metadata-with-signature-section-stripped plus the sha256 of the .jpkg
//! payload (the zstd-tar bytes after the 12+hdr_len header).
//!
//! Rationale: signing the FULL metadata catches tampering with deps,
//! hooks, license, etc.  Stripping the signature section avoids the
//! self-referential problem.  Including payload sha256 binds the
//! metadata to a specific payload; you can't move a sig from one
//! package to another with the same metadata-and-different payload.

use sha2::{Digest, Sha256};

use crate::recipe::Metadata;

/// Version byte identifying this canonical-bytes format.
///
/// Increment if the format ever changes so verifiers can detect the mismatch.
pub const CANON_VERSION: u8 = 1;

/// Fixed header prefix embedded at the start of every canonical-bytes buffer.
const CANON_PREFIX: &[u8] = b"jpkg-canon\0";

// ─── Public API ───────────────────────────────────────────────────────────────

/// Produce the deterministic byte string to sign / verify against.
///
/// # Format (version-prefixed so the format can evolve)
///
/// ```text
/// "jpkg-canon\0"          — 11 bytes (NUL-terminated ASCII tag)
/// CANON_VERSION            — 1 byte   (currently 1)
/// payload_sha256           — 32 bytes (raw SHA-256 of the zstd-tar payload)
/// metadata_canonical_toml  — variable (UTF-8 TOML, signature field stripped)
/// ```
///
/// `metadata_canonical_toml` is produced by cloning `metadata`, setting
/// `metadata.signature = None`, then serialising with `toml::to_string`.
/// The `toml` crate's serialisation is deterministic for a fixed struct
/// layout; we lock to that contract.
///
/// # Arguments
///
/// * `metadata` — the full `Metadata` (may contain a `signature` field; it is
///   stripped before serialisation so the output is identical whether or not a
///   `[signature]` block was already present).
/// * `payload_sha256` — raw 32-byte SHA-256 of the `.jpkg` payload
///   (the zstd-compressed tar blob starting after the 12-byte file header).
pub fn canonical_bytes(metadata: &Metadata, payload_sha256: &[u8; 32]) -> Vec<u8> {
    // Strip the signature before serialising to avoid the self-referential problem.
    let mut stripped = metadata.clone();
    stripped.signature = None;

    // Serialise the stripped metadata.  `toml::to_string` is deterministic for
    // a fixed struct; any change to package/depends/hooks/files will produce
    // different bytes.
    // SAFETY: `toml::to_string` returns Err only for types that contain maps
    // with non-string keys or other TOML-incompatible constructs.  `Metadata`
    // and all its sub-structs use only `String`, `Vec<String>`, `Option<T>`,
    // and `u64` fields — all of which round-trip cleanly through TOML.  This
    // call is unreachable in practice.
    let meta_toml = toml::to_string(&stripped)
        .expect("Metadata serialisation must not fail — all field types are TOML-compatible");

    // Assemble: prefix || version || sha256 || toml_bytes
    let mut out = Vec::with_capacity(
        CANON_PREFIX.len() + 1 + 32 + meta_toml.len(),
    );
    out.extend_from_slice(CANON_PREFIX);            // "jpkg-canon\0"
    out.push(CANON_VERSION);                        // version byte
    out.extend_from_slice(payload_sha256);          // 32-byte hash
    out.extend_from_slice(meta_toml.as_bytes());    // TOML body
    out
}

/// Compute the raw 32-byte SHA-256 of `payload`.
///
/// Convenience wrapper so callers don't need to import `sha2` directly.
/// `payload` is the raw zstd-compressed tar bytes that follow the .jpkg
/// 12-byte file header (magic + LE32 hdr_len).
pub fn compute_payload_sha256(payload: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(payload);
    h.finalize().into()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{
        DependsSection, FilesSection, HooksSection, Metadata, PackageSection, Signature,
    };

    // ── helpers ───────────────────────────────────────────────────────────────

    fn make_metadata(version: &str, with_sig: bool) -> Metadata {
        let signature = if with_sig {
            Some(Signature {
                algorithm: "ed25519".to_owned(),
                key_id: "jonerix-2026".to_owned(),
                sig: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_owned(),
            })
        } else {
            None
        };
        Metadata {
            package: PackageSection {
                name: Some("testpkg".to_owned()),
                version: Some(version.to_owned()),
                license: Some("MIT".to_owned()),
                arch: Some("x86_64".to_owned()),
                ..Default::default()
            },
            depends: DependsSection {
                runtime: vec!["musl".to_owned()],
                ..Default::default()
            },
            hooks: HooksSection::default(),
            files: FilesSection {
                sha256: Some("deadbeef".to_owned()),
                size: Some(1024),
            },
            signature,
        }
    }

    fn zero_sha256() -> [u8; 32] {
        [0u8; 32]
    }

    fn ones_sha256() -> [u8; 32] {
        [1u8; 32]
    }

    // ── 1. Deterministic: same input → identical bytes ────────────────────────

    #[test]
    fn test_canonical_bytes_deterministic() {
        let meta = make_metadata("1.0.0", false);
        let sha = zero_sha256();
        let b1 = canonical_bytes(&meta, &sha);
        let b2 = canonical_bytes(&meta, &sha);
        assert_eq!(b1, b2, "canonical_bytes must be deterministic");
    }

    // ── 2. Strips signature field: with/without sig → same bytes ─────────────

    #[test]
    fn test_canonical_bytes_strips_signature_field() {
        let meta_no_sig = make_metadata("1.0.0", false);
        let meta_with_sig = make_metadata("1.0.0", true);
        let sha = zero_sha256();

        let b_no_sig = canonical_bytes(&meta_no_sig, &sha);
        let b_with_sig = canonical_bytes(&meta_with_sig, &sha);

        assert_eq!(
            b_no_sig, b_with_sig,
            "canonical_bytes must produce identical output regardless of whether \
             the signature field is populated"
        );
    }

    // ── 3. Different payload sha256 → different bytes ─────────────────────────

    #[test]
    fn test_canonical_bytes_changes_on_payload_change() {
        let meta = make_metadata("1.0.0", false);
        let b1 = canonical_bytes(&meta, &zero_sha256());
        let b2 = canonical_bytes(&meta, &ones_sha256());
        assert_ne!(b1, b2, "different payload sha256 must produce different canonical bytes");
    }

    // ── 4. Different metadata version → different bytes ───────────────────────

    #[test]
    fn test_canonical_bytes_changes_on_metadata_change() {
        let sha = zero_sha256();
        let b1 = canonical_bytes(&make_metadata("1.0.0", false), &sha);
        let b2 = canonical_bytes(&make_metadata("2.0.0", false), &sha);
        assert_ne!(b1, b2, "bumping version must produce different canonical bytes");
    }

    // ── 5. Format invariants ──────────────────────────────────────────────────

    #[test]
    fn test_canonical_bytes_format_invariants() {
        let sha: [u8; 32] = {
            let mut a = [0u8; 32];
            for (i, b) in a.iter_mut().enumerate() {
                *b = i as u8;
            }
            a
        };
        let meta = make_metadata("1.0.0", false);
        let bytes = canonical_bytes(&meta, &sha);

        // Must start with "jpkg-canon\0".
        let prefix_len = CANON_PREFIX.len(); // 11
        assert!(
            bytes.starts_with(CANON_PREFIX),
            "canonical bytes must start with {:?}, got {:?}",
            CANON_PREFIX,
            &bytes[..prefix_len.min(bytes.len())]
        );

        // Version byte immediately follows prefix.
        assert_eq!(
            bytes[prefix_len],
            CANON_VERSION,
            "version byte at offset {prefix_len} must be {CANON_VERSION}"
        );

        // 32-byte sha256 follows version byte.
        let hash_start = prefix_len + 1;
        let hash_end = hash_start + 32;
        assert_eq!(
            &bytes[hash_start..hash_end],
            &sha,
            "payload sha256 must appear at bytes [{hash_start}..{hash_end}]"
        );

        // Total length is at least prefix + version + hash.
        assert!(
            bytes.len() >= hash_end,
            "canonical bytes must be at least {} bytes long",
            hash_end
        );
    }

    // ── 6. compute_payload_sha256 ─────────────────────────────────────────────

    #[test]
    fn test_compute_payload_sha256_known_value() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let digest = compute_payload_sha256(b"");
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_compute_payload_sha256_deterministic() {
        let payload = b"some zstd-tar bytes for testing";
        let d1 = compute_payload_sha256(payload);
        let d2 = compute_payload_sha256(payload);
        assert_eq!(d1, d2, "SHA-256 must be deterministic");
    }

    #[test]
    fn test_compute_payload_sha256_different_for_different_input() {
        let d1 = compute_payload_sha256(b"payload A");
        let d2 = compute_payload_sha256(b"payload B");
        assert_ne!(d1, d2, "distinct payloads must have distinct SHA-256 digests");
    }
}
