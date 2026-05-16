// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg sign` — two modes:
//!
//! ## .jpkg archive signing (new in Phase 0 / Worker B)
//!
//! ```text
//! jpkg sign <pkg.jpkg> --key <keyfile.sec>
//! ```
//!
//! Opens the archive, computes canonical bytes via `canon::canonical_bytes`,
//! signs with the secret key, embeds the `[signature]` block in the metadata
//! header, and rewrites the .jpkg atomically.
//!
//! The exported helper `sign_jpkg_in_place` is called by `cmd::build` when
//! `--sign-key` is set, and is available to future workers (Worker E for
//! bulk re-signing).
//!
//! ## Arbitrary-file signing (original behaviour)
//!
//! ```text
//! jpkg sign <keyfile.sec> <input>
//! jpkg sign <input> --key <keyfile.sec>
//! ```
//!
//! Reads the 64-byte secret key, signs the bytes, and writes the raw 64-byte
//! signature to `<input>.sig` (mode 0644).  Wire-compatible with C `cmd_sign.c`.

use crate::archive::JpkgArchive;
use crate::canon::{canonical_bytes, compute_payload_sha256};
use crate::recipe::{Metadata, Signature};
use crate::sign;
use ed25519_dalek::SigningKey;
use std::fs;
use std::io::Write as IoWrite;
use std::path::Path;

pub use crate::sign::SIGNATURE_LEN;

const USAGE: &str = "\
usage: jpkg sign <pkg.jpkg> --key <keyfile.sec>
   or: jpkg sign <keyfile.sec> <input>
   or: jpkg sign <input> --key <keyfile.sec>";

// ── base64 helpers (no external crate — standard alphabet A-Za-z0-9+/ with = padding) ──

const B64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode `bytes` to standard base64 (with `=` padding).
pub(crate) fn b64_encode(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };

        let i0 = (b0 >> 2) as usize;
        let i1 = (((b0 & 0x03) << 4) | (b1 >> 4)) as usize;
        let i2 = (((b1 & 0x0f) << 2) | (b2 >> 6)) as usize;
        let i3 = (b2 & 0x3f) as usize;

        out.push(B64_CHARS[i0]);
        out.push(B64_CHARS[i1]);
        if chunk.len() > 1 {
            out.push(B64_CHARS[i2]);
        } else {
            out.push(b'=');
        }
        if chunk.len() > 2 {
            out.push(B64_CHARS[i3]);
        } else {
            out.push(b'=');
        }
    }
    String::from_utf8(out).expect("base64 chars are valid UTF-8")
}

/// Decode standard base64 (with or without `=` padding).
pub(crate) fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    // Build decode table.
    let mut table = [0xffu8; 256];
    for (i, &c) in B64_CHARS.iter().enumerate() {
        table[c as usize] = i as u8;
    }

    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 3);
    let mut i = 0;

    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && (bytes[i] == b'\n' || bytes[i] == b'\r' || bytes[i] == b' ') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let get = |j: usize| -> Result<u8, String> {
            if j >= bytes.len() || bytes[j] == b'=' {
                return Ok(0);
            }
            let v = table[bytes[j] as usize];
            if v == 0xff {
                return Err(format!(
                    "invalid base64 character at position {j}: 0x{:02x}",
                    bytes[j]
                ));
            }
            Ok(v)
        };

        let c0 = get(i)?;
        let c1 = get(i + 1)?;
        let c2 = get(i + 2)?;
        let c3 = get(i + 3)?;

        out.push((c0 << 2) | (c1 >> 4));
        if i + 2 < bytes.len() && bytes[i + 2] != b'=' {
            out.push(((c1 & 0x0f) << 4) | (c2 >> 2));
        }
        if i + 3 < bytes.len() && bytes[i + 3] != b'=' {
            out.push(((c2 & 0x03) << 6) | c3);
        }
        i += 4;
    }
    Ok(out)
}

// ── Core: sign a .jpkg archive in place ──────────────────────────────────────

/// Sign a .jpkg archive in place.
///
/// Opens the archive at `path`, reads metadata + payload, computes canonical
/// bytes via `canon::canonical_bytes`, signs with `secret`, and rewrites the
/// .jpkg with the signature embedded in `Metadata.signature`.
///
/// The payload bytes are byte-identical to before signing; only the metadata
/// header changes (slightly larger TOML + updated hdr_len).
///
/// Atomic write: written to `{path}.tmp` then renamed, so a concurrent reader
/// never sees a partial archive.
///
/// # Arguments
///
/// * `path`    — path to the .jpkg archive to sign
/// * `secret`  — Ed25519 signing key
/// * `key_id`  — human-readable key identifier stored in `[signature].key_id`
pub fn sign_jpkg_in_place(path: &Path, secret: &SigningKey, key_id: &str) -> Result<(), String> {
    // ── 1. Open and parse the existing .jpkg ──────────────────────────────────
    let archive = JpkgArchive::open(path)
        .map_err(|e| format!("sign_jpkg_in_place: open {}: {e}", path.display()))?;

    let meta_str = archive
        .metadata_str()
        .map_err(|e| format!("sign_jpkg_in_place: metadata UTF-8: {e}"))?;

    let mut metadata = Metadata::from_str(meta_str)
        .map_err(|e| format!("sign_jpkg_in_place: parse metadata: {e}"))?;

    let payload = archive.payload();

    // ── 2. Compute canonical bytes ────────────────────────────────────────────
    let payload_sha256 = compute_payload_sha256(payload);
    let canon = canonical_bytes(&metadata, &payload_sha256);

    // ── 3. Sign ───────────────────────────────────────────────────────────────
    let raw_sig = sign::sign_detached(secret, &canon);
    let sig_b64 = b64_encode(&raw_sig);

    // ── 4. Embed signature in metadata ───────────────────────────────────────
    metadata.signature = Some(Signature {
        algorithm: "ed25519".to_string(),
        key_id: key_id.to_string(),
        sig: sig_b64,
    });

    let new_meta_toml = metadata
        .to_string()
        .map_err(|e| format!("sign_jpkg_in_place: serialize metadata: {e}"))?;
    let new_meta_bytes = new_meta_toml.as_bytes();
    let new_hdr_len = new_meta_bytes.len() as u32;

    // ── 5. Rewrite the .jpkg atomically ──────────────────────────────────────
    // Format: MAGIC(8) | hdr_len_LE32(4) | metadata_toml(new_hdr_len) | payload(unchanged)
    let tmp_path = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp_path)
            .map_err(|e| format!("sign_jpkg_in_place: create tmp: {e}"))?;
        f.write_all(&crate::JPKG_MAGIC)
            .map_err(|e| format!("sign_jpkg_in_place: write magic: {e}"))?;
        f.write_all(&new_hdr_len.to_le_bytes())
            .map_err(|e| format!("sign_jpkg_in_place: write hdr_len: {e}"))?;
        f.write_all(new_meta_bytes)
            .map_err(|e| format!("sign_jpkg_in_place: write metadata: {e}"))?;
        f.write_all(payload)
            .map_err(|e| format!("sign_jpkg_in_place: write payload: {e}"))?;
        f.flush()
            .map_err(|e| format!("sign_jpkg_in_place: flush: {e}"))?;
    }
    fs::rename(&tmp_path, path)
        .map_err(|e| format!("sign_jpkg_in_place: rename into place: {e}"))?;

    Ok(())
}

// ── Subcommand dispatcher ─────────────────────────────────────────────────────

/// Run the `jpkg sign` subcommand.
///
/// Two modes: `.jpkg` archive signing (embeds `[signature]` in the metadata
/// header) and raw-file detached signing (writes a 64-byte `.sig` file).
/// Returns 0 on success, 1 on signing error, or 2 on usage error.
pub fn run(args: &[String]) -> i32 {
    // Detect if we're operating on a .jpkg archive (new mode) or a raw file
    // (original mode).  A .jpkg argument is indicated by either the --key flag
    // being present alongside a .jpkg path, or a positional .jpkg path.
    //
    // Mode detection: if any positional argument ends with ".jpkg", use the
    // archive-signing path.
    let has_jpkg = args
        .iter()
        .any(|a| !a.starts_with('-') && a.ends_with(".jpkg"));

    if has_jpkg {
        run_jpkg_sign(args)
    } else {
        run_raw_sign(args)
    }
}

// ── .jpkg archive signing ─────────────────────────────────────────────────────

fn run_jpkg_sign(args: &[String]) -> i32 {
    // Parse: jpkg sign <pkg.jpkg> --key <keyfile.sec>
    // The key_id defaults to the stem of the key file basename.
    let (key_str, jpkg_str) = match parse_jpkg_args(args) {
        Some(pair) => pair,
        None => {
            eprintln!("jpkg sign: bad arguments for .jpkg signing ({:?})", args);
            eprintln!("{USAGE}");
            return 2;
        }
    };

    let key_path = Path::new(&key_str);
    let jpkg_path = Path::new(&jpkg_str);

    // Derive key_id from the key file basename (without extension).
    let key_id = key_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());

    let sk = match sign::read_secret_key(key_path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("jpkg sign: failed to load key {}: {e}", key_path.display());
            return 1;
        }
    };

    match sign_jpkg_in_place(jpkg_path, &sk, &key_id) {
        Ok(()) => {
            println!(
                "Signed: {} (key_id={})",
                jpkg_path.file_name().unwrap_or_default().to_string_lossy(),
                key_id
            );
            0
        }
        Err(e) => {
            eprintln!("jpkg sign: {e}");
            1
        }
    }
}

/// Parse args for .jpkg signing:
///   form 1: <pkg.jpkg> --key <keyfile.sec>
///   form 2: --key <keyfile.sec> <pkg.jpkg>
fn parse_jpkg_args(args: &[String]) -> Option<(String, String)> {
    let mut key: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--key" {
            if i + 1 >= args.len() {
                return None;
            }
            key = Some(args[i + 1].clone());
            i += 2;
        } else if let Some(rest) = a.strip_prefix("--key=") {
            key = Some(rest.to_string());
            i += 1;
        } else {
            positional.push(a.clone());
            i += 1;
        }
    }
    match (positional.len(), key) {
        (1, Some(k)) => Some((k, positional.into_iter().next().unwrap())),
        _ => None,
    }
}

// ── Raw-file signing (original behaviour) ────────────────────────────────────

fn run_raw_sign(args: &[String]) -> i32 {
    let (key_str, input_str) = match parse_raw_args(args) {
        Some(pair) => pair,
        None => {
            eprintln!("jpkg sign: bad arguments ({:?})", args);
            eprintln!("{USAGE}");
            return 2;
        }
    };
    let key_path = Path::new(&key_str);
    let input_path = Path::new(&input_str);

    let sk = match sign::read_secret_key(key_path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("jpkg sign: failed to load key {}: {e}", key_path.display());
            return 1;
        }
    };

    let bytes = match std::fs::read(input_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("jpkg sign: failed to read {}: {e}", input_path.display());
            return 1;
        }
    };

    let sig = sign::sign_detached(&sk, &bytes);

    let sig_path_str = format!("{}.sig", input_path.display());
    let sig_path = Path::new(&sig_path_str);

    if let Err(e) = write_sig(sig_path, &sig) {
        eprintln!("jpkg sign: failed to write {}: {e}", sig_path.display());
        return 1;
    }

    println!("Signed: {} -> {}", input_path.display(), sig_path.display());
    0
}

/// Resolve the (key, input) pair for raw-file signing from either argv form:
///   form 1:  [<keyfile>, <input>]
///   form 2:  [<input>, --key, <keyfile>]   (or --key=<keyfile>, in any order)
fn parse_raw_args(args: &[String]) -> Option<(String, String)> {
    let mut key: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--key" {
            if i + 1 >= args.len() {
                return None;
            }
            key = Some(args[i + 1].clone());
            i += 2;
        } else if let Some(rest) = a.strip_prefix("--key=") {
            key = Some(rest.to_string());
            i += 1;
        } else {
            positional.push(a.clone());
            i += 1;
        }
    }
    match (positional.len(), key) {
        (1, Some(k)) => Some((k, positional.into_iter().next().unwrap())),
        (2, None) => {
            let mut it = positional.into_iter();
            let keyf = it.next().unwrap();
            let inp = it.next().unwrap();
            Some((keyf, inp))
        }
        _ => None,
    }
}

/// Write a raw 64-byte signature to `path` with mode 0644.
fn write_sig(path: &Path, sig: &[u8; SIGNATURE_LEN]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(path)?;
    f.write_all(sig)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::tests::build_test_jpkg;
    use crate::cmd::verify::verify_jpkg_signature;
    use crate::sign;
    use std::fs;
    use tempfile::tempdir;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── base64 round-trip ──────────────────────────────────────────────────────

    #[test]
    fn test_b64_roundtrip() {
        let data: Vec<u8> = (0u8..=63).collect();
        let encoded = b64_encode(&data);
        let decoded = b64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_b64_known_vectors() {
        // RFC 4648 test vectors.
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_encode(b"f"), "Zg==");
        assert_eq!(b64_encode(b"fo"), "Zm8=");
        assert_eq!(b64_encode(b"foo"), "Zm9v");
        assert_eq!(b64_encode(b"foob"), "Zm9vYg==");

        assert_eq!(b64_decode("").unwrap(), b"");
        assert_eq!(b64_decode("Zg==").unwrap(), b"f");
        assert_eq!(b64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(b64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(b64_decode("Zm9vYg==").unwrap(), b"foob");
    }

    #[test]
    fn test_b64_64_byte_sig_roundtrip() {
        // 64-byte signatures must round-trip through base64 cleanly.
        let raw = [0xABu8; 64];
        let enc = b64_encode(&raw);
        let dec = b64_decode(&enc).unwrap();
        assert_eq!(dec, raw);
    }

    // ── Original raw-file signing tests ───────────────────────────────────────

    #[test]
    fn test_sign_produces_64_byte_sig() {
        let dir = tempdir().unwrap();

        let sk = sign::keygen();
        let sec_path = dir.path().join("test.sec");
        sign::write_secret_key(&sec_path, &sk).unwrap();

        let input_path = dir.path().join("payload.bin");
        fs::write(&input_path, b"hello jonerix signed payload").unwrap();

        let rc = run(&args(&[
            sec_path.to_str().unwrap(),
            input_path.to_str().unwrap(),
        ]));
        assert_eq!(rc, 0, "expected exit 0");

        let sig_path = dir.path().join("payload.bin.sig");
        assert!(sig_path.exists(), "payload.bin.sig must exist");

        let sig_bytes = fs::read(&sig_path).unwrap();
        assert_eq!(sig_bytes.len(), 64, "signature must be exactly 64 bytes");
    }

    #[test]
    fn test_sign_verify_roundtrip() {
        let dir = tempdir().unwrap();

        let sk = sign::keygen();
        let vk = sk.verifying_key();

        let sec_path = dir.path().join("verify.sec");
        sign::write_secret_key(&sec_path, &sk).unwrap();

        let payload = b"package index bytes to verify";
        let input_path = dir.path().join("index.bin");
        fs::write(&input_path, payload).unwrap();

        let rc = run(&args(&[
            sec_path.to_str().unwrap(),
            input_path.to_str().unwrap(),
        ]));
        assert_eq!(rc, 0);

        let sig_path = dir.path().join("index.bin.sig");
        let sig_bytes = fs::read(&sig_path).unwrap();

        sign::verify_detached(&vk, payload, &sig_bytes)
            .expect("signature produced by cmd::sign must verify");
    }

    #[test]
    fn test_sign_missing_input_file() {
        let dir = tempdir().unwrap();

        let sk = sign::keygen();
        let sec_path = dir.path().join("k.sec");
        sign::write_secret_key(&sec_path, &sk).unwrap();

        let missing = dir.path().join("does_not_exist.bin");

        let rc = run(&args(&[
            sec_path.to_str().unwrap(),
            missing.to_str().unwrap(),
        ]));
        assert_eq!(rc, 1, "missing input file must return exit 1");
    }

    #[test]
    fn test_sign_wrong_arg_count() {
        assert_eq!(run(&args(&[])), 2, "zero args must return exit 2");
        assert_eq!(run(&args(&["only-one"])), 2, "one arg must return exit 2");
        assert_eq!(
            run(&args(&["a", "b", "c"])),
            2,
            "three positional args must return exit 2"
        );
    }

    #[test]
    fn test_sign_bad_key_file() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("data.bin");
        fs::write(&input_path, b"data").unwrap();

        let missing_key = dir.path().join("no.sec");
        let rc = run(&args(&[
            missing_key.to_str().unwrap(),
            input_path.to_str().unwrap(),
        ]));
        assert_eq!(rc, 1, "missing key file must return exit 1");
    }

    // ── .jpkg archive signing tests ────────────────────────────────────────────

    /// Build a test jpkg, sign it, then verify it — should succeed.
    #[test]
    fn test_jpkg_sign_then_verify_ok() {
        let dir = tempdir().unwrap();

        // Generate keypair.
        let sk = sign::keygen();
        let sec_path = dir.path().join("jonerix-2026.sec");
        let pub_path = dir.path().join("keys").join("jonerix-2026.pub");
        fs::create_dir_all(dir.path().join("keys")).unwrap();
        sign::write_secret_key(&sec_path, &sk).unwrap();
        sign::write_public_key(&pub_path, &sk.verifying_key()).unwrap();

        // Build a test .jpkg.
        let jpkg_path = build_test_jpkg(dir.path(), "sigpkg", "1.0.0");

        // Sign it.
        let key_id = "jonerix-2026";
        sign_jpkg_in_place(&jpkg_path, &sk, key_id).expect("sign_jpkg_in_place must succeed");

        // Verify it.
        let keys_dir = dir.path().join("keys");
        let result = verify_jpkg_signature(&jpkg_path, &keys_dir);
        assert!(result.is_ok(), "verify should succeed: {:?}", result);
        let msg = result.unwrap();
        assert!(
            msg.contains("sigpkg") || msg.contains("verified") || msg.contains(key_id),
            "verify message should mention package or key_id: {msg}"
        );
    }

    /// Build a test jpkg, sign it, tamper with payload bytes, verify should fail.
    #[test]
    fn test_jpkg_verify_fails_on_tampered_payload() {
        let dir = tempdir().unwrap();

        let sk = sign::keygen();
        let sec_path = dir.path().join("tamper-key.sec");
        let pub_path = dir.path().join("keys").join("tamper-key.pub");
        fs::create_dir_all(dir.path().join("keys")).unwrap();
        sign::write_secret_key(&sec_path, &sk).unwrap();
        sign::write_public_key(&pub_path, &sk.verifying_key()).unwrap();

        let jpkg_path = build_test_jpkg(dir.path(), "tamperpkg", "1.0.0");
        sign_jpkg_in_place(&jpkg_path, &sk, "tamper-key").expect("sign must succeed");

        // Read the .jpkg bytes, flip a byte in the payload area.
        let mut jpkg_bytes = fs::read(&jpkg_path).unwrap();
        // Payload starts at offset 12 + hdr_len.  Flip the last byte.
        let last = jpkg_bytes.len() - 1;
        jpkg_bytes[last] ^= 0xff;
        fs::write(&jpkg_path, &jpkg_bytes).unwrap();

        let keys_dir = dir.path().join("keys");
        let result = verify_jpkg_signature(&jpkg_path, &keys_dir);
        assert!(result.is_err(), "verify must fail on tampered payload");
    }

    /// Build unsigned .jpkg, verify should error with clear message.
    #[test]
    fn test_jpkg_verify_fails_on_missing_sig() {
        let dir = tempdir().unwrap();

        let sk = sign::keygen();
        let pub_path = dir.path().join("keys").join("nosig.pub");
        fs::create_dir_all(dir.path().join("keys")).unwrap();
        sign::write_public_key(&pub_path, &sk.verifying_key()).unwrap();

        let jpkg_path = build_test_jpkg(dir.path(), "unsignedpkg", "1.0.0");

        let keys_dir = dir.path().join("keys");
        let result = verify_jpkg_signature(&jpkg_path, &keys_dir);
        assert!(result.is_err(), "verify must fail on unsigned package");
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.to_lowercase().contains("no signature") || err_msg.contains("unsigned"),
            "error should mention missing signature: {err_msg}"
        );
    }

    /// Sign with key A, verify with key B's pubkey — must fail.
    #[test]
    fn test_jpkg_verify_fails_on_wrong_key() {
        let dir = tempdir().unwrap();

        let sk_a = sign::keygen();
        let sk_b = sign::keygen();

        let sec_a = dir.path().join("key-a.sec");
        let pub_b = dir.path().join("keys").join("key-b.pub");
        fs::create_dir_all(dir.path().join("keys")).unwrap();
        sign::write_secret_key(&sec_a, &sk_a).unwrap();
        sign::write_public_key(&pub_b, &sk_b.verifying_key()).unwrap();

        let jpkg_path = build_test_jpkg(dir.path(), "wrongkeypkg", "1.0.0");
        sign_jpkg_in_place(&jpkg_path, &sk_a, "key-a").expect("sign must succeed");

        let keys_dir = dir.path().join("keys");
        let result = verify_jpkg_signature(&jpkg_path, &keys_dir);
        assert!(
            result.is_err(),
            "verify must fail when signed with key-a but only key-b is trusted"
        );
    }

    /// Build with --sign-key produces a .jpkg that verifies.
    #[test]
    fn test_jpkg_build_with_sign_key_produces_signed_jpkg() {
        use crate::archive::JpkgArchive;
        use crate::recipe::Metadata;
        use std::fs;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let out_dir = dir.path().join("out");
        fs::create_dir_all(&out_dir).unwrap();

        // Generate a keypair.
        let sk = sign::keygen();
        let sec_path = dir.path().join("build-key.sec");
        let pub_path = dir.path().join("keys").join("build-key.pub");
        fs::create_dir_all(dir.path().join("keys")).unwrap();
        sign::write_secret_key(&sec_path, &sk).unwrap();
        sign::write_public_key(&pub_path, &sk.verifying_key()).unwrap();

        // Build a minimal recipe.
        let toml = r#"[package]
name = "signedpkg"
version = "0.1.0"
license = "MIT"
arch = "x86_64"

[build]
system = "custom"
install = "mkdir -p \"$DESTDIR/bin\" && touch \"$DESTDIR/bin/ok\""
"#;
        let recipe_dir = dir.path().join("recipe");
        fs::create_dir_all(&recipe_dir).unwrap();
        fs::write(recipe_dir.join("recipe.toml"), toml).unwrap();

        let rc = crate::cmd::build::run(&[
            recipe_dir.to_string_lossy().into_owned(),
            "--output".to_owned(),
            out_dir.to_string_lossy().into_owned(),
            "--sign-key".to_owned(),
            sec_path.to_string_lossy().into_owned(),
        ]);
        assert_eq!(rc, 0, "build with --sign-key should succeed");

        // Find the .jpkg artifact.
        let artifact = fs::read_dir(&out_dir)
            .unwrap()
            .flatten()
            .find(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x == "jpkg")
                    .unwrap_or(false)
            })
            .expect("artifact not found")
            .path();

        // Open and verify the [signature] section exists.
        let arch = JpkgArchive::open(&artifact).expect("open failed");
        let meta_str = arch.metadata_str().expect("metadata UTF-8");
        let meta = Metadata::from_str(meta_str).expect("parse metadata");
        assert!(
            meta.signature.is_some(),
            "metadata must contain [signature] after --sign-key build"
        );
        let sig = meta.signature.as_ref().unwrap();
        assert_eq!(sig.algorithm, "ed25519");
        assert!(!sig.key_id.is_empty());
        assert!(!sig.sig.is_empty());

        // Verify the signature with the public key.
        let keys_dir = dir.path().join("keys");
        let result = verify_jpkg_signature(&artifact, &keys_dir);
        assert!(result.is_ok(), "built .jpkg must verify: {:?}", result);
    }
}
