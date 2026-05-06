// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! Ed25519 keypair generation, signing, and verification — Rust port of
//! `jpkg/src/sign.c`.
//!
//! # Invariants
//!
//! 1. **Key format**: public keys are 32-byte raw Ed25519 verifying keys; secret
//!    keys are 64-byte sequences `seed[32] || pubkey[32]` matching the C `.sec`
//!    file layout.  Files with any other length are rejected with
//!    `SignError::BadKeyLen`.  Callers must not pass a key file whose length is
//!    correct but whose bytes are not a valid Ed25519 scalar; `ed25519-dalek`
//!    will reject such keys during signing or verification with a
//!    `SignError::Verify` variant.
//!
//! 2. **Signature format**: detached signatures are exactly 64 bytes of raw
//!    Ed25519 signature material (`R || S`), with no length prefix, no framing,
//!    and no algorithm tag.  This is wire-compatible with the C jpkg
//!    `sign_create` / `sign_verify_detached` pair.  A slice of any other length
//!    passed to `verify_detached` returns `SignError::BadSigLen` before any
//!    cryptographic work is done.
//!
//! 3. **Verification guarantee**: `verify_detached` returning `Ok(())` means
//!    the signature was produced by the private half of the provided public key
//!    over the exact bytes in `msg`.  Any single-byte change to `msg` or `sig`
//!    after signing will cause verification to return `Err(SignError::Verify)`.
//!
//! 4. **Key-set semantics**: [`PublicKeySet::verify_detached`] tries all loaded
//!    keys and returns `Ok(key_name)` for the first match.  An empty key set
//!    returns `Err(SignError::NoKeys)` without attempting any verification.
//!    A non-empty key set where no key matches returns `Err(SignError::Verify)`
//!    from the last attempted key.  Callers must treat `NoKeys` and `Verify`
//!    differently: `NoKeys` means verification was not possible (unconfigured
//!    host), while `Verify` means the signature is positively wrong.
//!
//! 5. **Secret key file mode**: [`write_secret_key`] creates files with mode
//!    0o600 (owner-read/write only).  Callers must not relax this mode; leaking
//!    a secret key allows an attacker to forge package signatures.
//!
//! Wire-compatible with the C jpkg:
//!   - Public key:  32 bytes raw binary  (.pub)
//!   - Secret key:  64 bytes raw binary  (.sec) — seed[32] || pubkey[32]
//!   - Signature:   64 bytes raw binary  (.sig) — no length prefix

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write as IoWrite;
use std::path::Path;

pub const PUBLIC_KEY_LEN: usize = 32;
pub const SECRET_KEY_LEN: usize = 64; // seed (32) + public (32)
pub const SIGNATURE_LEN: usize = 64;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum SignError {
    Io(std::io::Error),
    BadKeyLen { expected: usize, got: usize },
    BadSigLen { expected: usize, got: usize },
    Verify(ed25519_dalek::SignatureError),
    NoKeys,
}

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignError::Io(e) => write!(f, "I/O error: {}", e),
            SignError::BadKeyLen { expected, got } => {
                write!(f, "bad key length: expected {} bytes, got {}", expected, got)
            }
            SignError::BadSigLen { expected, got } => {
                write!(f, "bad signature length: expected {} bytes, got {}", expected, got)
            }
            SignError::Verify(e) => write!(f, "signature verification failed: {}", e),
            SignError::NoKeys => write!(f, "no public keys loaded; cannot verify signature"),
        }
    }
}

impl std::error::Error for SignError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SignError::Io(e) => Some(e),
            SignError::Verify(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SignError {
    fn from(e: std::io::Error) -> Self {
        SignError::Io(e)
    }
}

impl From<ed25519_dalek::SignatureError> for SignError {
    fn from(e: ed25519_dalek::SignatureError) -> Self {
        SignError::Verify(e)
    }
}

// ---------------------------------------------------------------------------
// File I/O helpers
// ---------------------------------------------------------------------------

/// Read a public key (32-byte raw binary) from `path`.
pub fn read_public_key(path: &Path) -> Result<VerifyingKey, SignError> {
    let bytes = fs::read(path)?;
    if bytes.len() != PUBLIC_KEY_LEN {
        return Err(SignError::BadKeyLen {
            expected: PUBLIC_KEY_LEN,
            got: bytes.len(),
        });
    }
    // SAFETY: the `bytes.len() != PUBLIC_KEY_LEN` guard above ensures exactly
    // PUBLIC_KEY_LEN (32) bytes are present, so try_into() into [u8; 32] cannot
    // fail — the slice length matches the array length exactly.
    let arr: [u8; PUBLIC_KEY_LEN] = bytes.try_into().unwrap();
    Ok(VerifyingKey::from_bytes(&arr)?)
}

/// Read a secret key (64-byte raw binary: seed[32] || pubkey[32]) from `path`.
pub fn read_secret_key(path: &Path) -> Result<SigningKey, SignError> {
    let bytes = fs::read(path)?;
    if bytes.len() != SECRET_KEY_LEN {
        return Err(SignError::BadKeyLen {
            expected: SECRET_KEY_LEN,
            got: bytes.len(),
        });
    }
    // SAFETY: the `bytes.len() != SECRET_KEY_LEN` guard above ensures exactly
    // SECRET_KEY_LEN (64) bytes are present, so try_into() into [u8; 64] cannot
    // fail — the slice length matches the array length exactly.
    let arr: [u8; SECRET_KEY_LEN] = bytes.try_into().unwrap();
    // from_keypair_bytes expects seed[32] || pubkey[32] — exactly our .sec format.
    Ok(SigningKey::from_keypair_bytes(&arr)?)
}

/// Write a public key to `path` with mode 0644.
pub fn write_public_key(path: &Path, key: &VerifyingKey) -> Result<(), SignError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(path)?;
    f.write_all(key.as_bytes())?;
    Ok(())
}

/// Write a secret key to `path` with mode 0600.
pub fn write_secret_key(path: &Path, key: &SigningKey) -> Result<(), SignError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // Serialise as seed[32] || pubkey[32] to match the C .sec format.
    f.write_all(&key.to_keypair_bytes())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Keypair generation
// ---------------------------------------------------------------------------

/// Generate a fresh Ed25519 keypair using the OS CSPRNG.
pub fn keygen() -> SigningKey {
    SigningKey::generate(&mut OsRng)
}

// ---------------------------------------------------------------------------
// Detached-signature primitives
// ---------------------------------------------------------------------------

/// Produce a raw 64-byte detached signature.  Wire-compatible with C jpkg's
/// `sign_create`: the signature is the first 64 bytes of the tweetnacl
/// signed-message, which is the Ed25519 signature bytes (R || S).
pub fn sign_detached(secret: &SigningKey, msg: &[u8]) -> [u8; SIGNATURE_LEN] {
    let sig: Signature = secret.sign(msg);
    sig.to_bytes()
}

/// Verify a raw 64-byte detached signature.
pub fn verify_detached(
    public: &VerifyingKey,
    msg: &[u8],
    sig: &[u8],
) -> Result<(), SignError> {
    if sig.len() != SIGNATURE_LEN {
        return Err(SignError::BadSigLen {
            expected: SIGNATURE_LEN,
            got: sig.len(),
        });
    }
    // SAFETY: the `sig.len() != SIGNATURE_LEN` guard above ensures exactly
    // SIGNATURE_LEN (64) bytes are present, so try_into() into [u8; 64] cannot
    // fail — the slice length matches the array length exactly.
    let arr: [u8; SIGNATURE_LEN] = sig.try_into().unwrap();
    let signature = Signature::from_bytes(&arr);
    public.verify(msg, &signature)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// PublicKeySet — mirrors C sign_load_keys / sign_verify_detached
// ---------------------------------------------------------------------------

/// A set of named public keys, loaded from a directory of `*.pub` files.
/// Mirrors the global `g_keys` array in the C implementation.
pub struct PublicKeySet {
    keys: Vec<(String, VerifyingKey)>,
}

impl PublicKeySet {
    /// Construct an empty `PublicKeySet` with no keys loaded.
    ///
    /// Useful in tests and in contexts where signature verification is
    /// intentionally skipped (e.g. local recipe builds where no key dir exists).
    pub fn empty() -> Self {
        Self { keys: Vec::new() }
    }

    /// Load every `*.pub` file from `dir`.
    ///
    /// If `dir` does not exist, or contains no `*.pub` files, returns an
    /// empty set rather than an error — matching the C behaviour where
    /// `sign_load_keys` succeeds silently and `sign_verify_detached` warns
    /// when `g_keys.count == 0`.
    pub fn load_dir(dir: &Path) -> Result<Self, SignError> {
        let mut keys = Vec::new();

        let rd = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self { keys });
            }
            Err(e) => return Err(SignError::Io(e)),
        };

        for entry in rd {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.ends_with(".pub") {
                continue;
            }
            match read_public_key(&entry.path()) {
                Ok(vk) => {
                    // Strip the `.pub` suffix for the display name, matching
                    // what C does when it logs `ent->d_name`.
                    let display = name_str.into_owned();
                    keys.push((display, vk));
                }
                Err(_) => {
                    // Skip unreadable / malformed key files, matching C which
                    // logs an error and continues.
                    continue;
                }
            }
        }

        Ok(Self { keys })
    }

    /// Return `true` if no public keys are loaded.
    ///
    /// An empty set cannot verify any signature; [`verify_detached`](Self::verify_detached)
    /// will return `Err(SignError::NoKeys)` immediately.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Return the file-names (including `.pub`) of all loaded keys.
    pub fn names(&self) -> Vec<&str> {
        self.keys.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Verify `sig` over `msg` against every loaded key.
    ///
    /// Returns `Ok(name)` for the first matching key, `Err(NoKeys)` if the
    /// set is empty, or `Err(Verify(_))` if all keys fail.
    pub fn verify_detached(&self, msg: &[u8], sig: &[u8]) -> Result<String, SignError> {
        if self.keys.is_empty() {
            return Err(SignError::NoKeys);
        }
        let mut last_err: Option<SignError> = None;
        for (name, vk) in &self.keys {
            match verify_detached(vk, msg, sig) {
                Ok(()) => return Ok(name.clone()),
                Err(e) => last_err = Some(e),
            }
        }
        // SAFETY: the `self.keys.is_empty()` early-return above guarantees at
        // least one iteration of the loop, so `last_err` is always `Some` by
        // the time we reach this point.  The unwrap is unreachable.
        Err(last_err.unwrap())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // 1. Round-trip: keygen → sign → verify
    #[test]
    fn test_roundtrip_sign_verify() {
        let sk = keygen();
        let vk = sk.verifying_key();
        let msg = b"hello jonerix";
        let sig = sign_detached(&sk, msg);
        verify_detached(&vk, msg, &sig).expect("valid signature must verify");
    }

    // 2. Tampered message fails
    #[test]
    fn test_tampered_message_fails() {
        let sk = keygen();
        let vk = sk.verifying_key();
        let mut msg = *b"hello jonerix!!";
        let sig = sign_detached(&sk, &msg);
        msg[0] ^= 0xff;
        assert!(
            verify_detached(&vk, &msg, &sig).is_err(),
            "tampered message must not verify"
        );
    }

    // 3. Tampered signature fails
    #[test]
    fn test_tampered_sig_fails() {
        let sk = keygen();
        let vk = sk.verifying_key();
        let msg = b"hello jonerix";
        let mut sig = sign_detached(&sk, msg);
        sig[0] ^= 0x01;
        assert!(
            verify_detached(&vk, msg, &sig).is_err(),
            "tampered signature must not verify"
        );
    }

    // 4. Wrong key fails
    #[test]
    fn test_wrong_key_fails() {
        let sk_a = keygen();
        let sk_b = keygen();
        let vk_b = sk_b.verifying_key();
        let msg = b"signed by A";
        let sig = sign_detached(&sk_a, msg);
        assert!(
            verify_detached(&vk_b, msg, &sig).is_err(),
            "signature from key A must not verify under key B"
        );
    }

    // 5. PublicKeySet round-trip with 2 keys
    #[test]
    fn test_public_key_set_roundtrip() {
        let dir = tempdir().unwrap();

        let sk_a = keygen();
        let sk_b = keygen();

        let path_a = dir.path().join("alice.pub");
        let path_b = dir.path().join("bob.pub");
        write_public_key(&path_a, &sk_a.verifying_key()).unwrap();
        write_public_key(&path_b, &sk_b.verifying_key()).unwrap();

        let set = PublicKeySet::load_dir(dir.path()).unwrap();
        assert_eq!(set.keys.len(), 2);

        let mut names = set.names();
        names.sort();
        assert!(names.contains(&"alice.pub"), "alice.pub should be in names()");
        assert!(names.contains(&"bob.pub"), "bob.pub should be in names()");

        // Verify a sig made by sk_a
        let msg = b"package index bytes";
        let sig_a = sign_detached(&sk_a, msg);
        let matched = set.verify_detached(msg, &sig_a).expect("sk_a sig must verify");
        assert_eq!(matched, "alice.pub");

        // Verify a sig made by sk_b
        let sig_b = sign_detached(&sk_b, msg);
        let matched_b = set.verify_detached(msg, &sig_b).expect("sk_b sig must verify");
        assert_eq!(matched_b, "bob.pub");
    }

    // 6. PublicKeySet: missing dir returns empty set, not an error
    #[test]
    fn test_public_key_set_missing_dir() {
        let dir = tempdir().unwrap();
        let nonexistent = dir.path().join("does_not_exist");
        let set = PublicKeySet::load_dir(&nonexistent).expect("missing dir must not error");
        assert!(set.is_empty());
        assert!(matches!(
            set.verify_detached(b"x", &[0u8; 64]),
            Err(SignError::NoKeys)
        ));
    }

    // 7. Bad key length: 31-byte file → BadKeyLen
    #[test]
    fn test_bad_key_length() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("short.pub");
        std::fs::write(&path, &[0u8; 31]).unwrap();
        match read_public_key(&path) {
            Err(SignError::BadKeyLen { expected: 32, got: 31 }) => {}
            other => panic!("expected BadKeyLen(32, 31), got {:?}", other),
        }
    }

    // 8. write_secret_key creates file with mode 0600
    #[test]
    fn test_secret_key_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.sec");
        let sk = keygen();
        write_secret_key(&path, &sk).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "secret key must have mode 0600, got {:04o}", mode);
    }

    // Bonus: secret key file round-trip (write then read back)
    #[test]
    fn test_secret_key_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.sec");
        let sk = keygen();
        write_secret_key(&path, &sk).unwrap();
        let sk2 = read_secret_key(&path).unwrap();
        assert_eq!(sk.to_bytes(), sk2.to_bytes());
    }
}
