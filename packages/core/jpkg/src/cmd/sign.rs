/*
 * jpkg - jonerix package manager
 * cmd/sign.rs - `jpkg sign`: produce a detached Ed25519 signature for a file
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

//! `jpkg sign <keyfile.sec> <input>`
//!
//! Reads the 64-byte secret key from `<keyfile.sec>`, signs the bytes of
//! `<input>`, and writes the raw 64-byte signature to `<input>.sig`
//! (mode 0644).  Wire-compatible with the C `cmd_sign.c`.

use crate::sign;
use std::path::Path;

const USAGE: &str = "usage: jpkg sign <keyfile.sec> <input>\n   or: jpkg sign <input> --key <keyfile.sec>";

pub fn run(args: &[String]) -> i32 {
    // Accept both forms — the C jpkg's CLI was lenient and the
    // publish-packages workflow uses the second form
    // (`jpkg sign INDEX.zst --key /tmp/jpkg-sign.sec`).
    //
    //   form 1:  <keyfile.sec> <input>
    //   form 2:  <input> --key <keyfile.sec>      (or --key=<keyfile.sec>)
    let (key_str, input_str) = match parse_args(args) {
        Some(pair) => pair,
        None => {
            eprintln!("jpkg sign: bad arguments ({:?})", args);
            eprintln!("{USAGE}");
            return 2;
        }
    };
    let key_path = Path::new(&key_str);
    let input_path = Path::new(&input_str);

    // ── Load secret key ───────────────────────────────────────────────────────
    let sk = match sign::read_secret_key(key_path) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("jpkg sign: failed to load key {}: {e}", key_path.display());
            return 1;
        }
    };

    // ── Read input ────────────────────────────────────────────────────────────
    let bytes = match std::fs::read(input_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("jpkg sign: failed to read {}: {e}", input_path.display());
            return 1;
        }
    };

    // ── Sign ──────────────────────────────────────────────────────────────────
    let sig = sign::sign_detached(&sk, &bytes);

    // ── Write .sig (mode 0644) ────────────────────────────────────────────────
    let sig_path_str = format!("{}.sig", input_path.display());
    let sig_path = Path::new(&sig_path_str);

    if let Err(e) = write_sig(sig_path, &sig) {
        eprintln!("jpkg sign: failed to write {}: {e}", sig_path.display());
        return 1;
    }

    println!("Signed: {} -> {}", input_path.display(), sig_path.display());
    0
}

/// Resolve the (key, input) pair from either argv form:
///   form 1:  [<keyfile>, <input>]
///   form 2:  [<input>, --key, <keyfile>]   (or --key=<keyfile>, in any order)
fn parse_args(args: &[String]) -> Option<(String, String)> {
    // Strip --key/--key=KEY from anywhere in the list and capture its value.
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
        // form 2: jpkg sign <input> --key <keyfile>
        (1, Some(k)) => Some((k, positional.into_iter().next().unwrap())),
        // form 1: jpkg sign <keyfile> <input>  (no --key)
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
fn write_sig(path: &Path, sig: &[u8; sign::SIGNATURE_LEN]) -> std::io::Result<()> {
    use std::io::Write as IoWrite;
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
    use crate::sign;
    use tempfile::tempdir;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // 1. Full sign round-trip: generate key, write input, run cmd::sign,
    //    assert <input>.sig is exactly 64 bytes.
    #[test]
    fn test_sign_produces_64_byte_sig() {
        let dir = tempdir().unwrap();

        let sk = sign::keygen();
        let sec_path = dir.path().join("test.sec");
        sign::write_secret_key(&sec_path, &sk).unwrap();

        let input_path = dir.path().join("payload.bin");
        std::fs::write(&input_path, b"hello jonerix signed payload").unwrap();

        let rc = run(&args(&[
            sec_path.to_str().unwrap(),
            input_path.to_str().unwrap(),
        ]));
        assert_eq!(rc, 0, "expected exit 0");

        let sig_path = dir.path().join("payload.bin.sig");
        assert!(sig_path.exists(), "payload.bin.sig must exist");

        let sig_bytes = std::fs::read(&sig_path).unwrap();
        assert_eq!(sig_bytes.len(), 64, "signature must be exactly 64 bytes");
    }

    // 2. Verify the produced sig with verify_detached — must succeed.
    #[test]
    fn test_sign_verify_roundtrip() {
        let dir = tempdir().unwrap();

        let sk = sign::keygen();
        let vk = sk.verifying_key();

        let sec_path = dir.path().join("verify.sec");
        sign::write_secret_key(&sec_path, &sk).unwrap();

        let payload = b"package index bytes to verify";
        let input_path = dir.path().join("index.bin");
        std::fs::write(&input_path, payload).unwrap();

        let rc = run(&args(&[
            sec_path.to_str().unwrap(),
            input_path.to_str().unwrap(),
        ]));
        assert_eq!(rc, 0);

        let sig_path = dir.path().join("index.bin.sig");
        let sig_bytes = std::fs::read(&sig_path).unwrap();

        sign::verify_detached(&vk, payload, &sig_bytes)
            .expect("signature produced by cmd::sign must verify with the matching public key");
    }

    // 3. Missing input file → exit 1.
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

    // 4. Wrong number of args → exit 2.
    #[test]
    fn test_sign_wrong_arg_count() {
        assert_eq!(run(&args(&[])), 2, "zero args must return exit 2");
        assert_eq!(run(&args(&["only-one"])), 2, "one arg must return exit 2");
        assert_eq!(
            run(&args(&["a", "b", "c"])),
            2,
            "three args must return exit 2"
        );
    }

    // 5. Bad / non-existent key file → exit 1.
    #[test]
    fn test_sign_bad_key_file() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("data.bin");
        std::fs::write(&input_path, b"data").unwrap();

        let missing_key = dir.path().join("no.sec");
        let rc = run(&args(&[
            missing_key.to_str().unwrap(),
            input_path.to_str().unwrap(),
        ]));
        assert_eq!(rc, 1, "missing key file must return exit 1");
    }
}
