/*
 * jpkg - jonerix package manager
 * cmd/resign.rs - `jpkg resign`: bulk re-sign existing .jpkg archives
 *
 * MIT License
 * Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software
 */

//! `jpkg resign` — bulk re-sign one or more existing `.jpkg` archives.
//!
//! ```text
//! jpkg resign <pkg.jpkg>... [--key <secret-path>] [--key-id <id>] [--keep-existing] [--dry-run]
//! ```
//!
//! Used to retroactively sign historical packages that are already published
//! to the rolling `packages` release tag, completing Phase 1 of the signing
//! rollout without rebuilding anything from source.
//!
//! ## Options
//!
//! * `--key <path>` — Ed25519 secret key to sign with
//!   (default: `/etc/jpkg/keys/jonerix.sec`).
//! * `--key-id <id>` — identifier embedded into the `[signature]` section.
//!   Defaults to the stem of the key file basename, e.g.
//!   `/etc/jpkg/keys/jonerix-2026.sec` → `jonerix-2026`.
//! * `--keep-existing` — skip packages that already carry a signature.
//!   Default behaviour is to overwrite any existing signature.
//! * `--dry-run` — parse and validate everything but do not write back.
//!
//! ## Exit codes
//!
//! * `0` — all packages re-signed (or skipped) without error.
//! * `1` — one or more packages failed.
//! * `2` — bad arguments / usage error.

use crate::archive::JpkgArchive;
use crate::recipe::Metadata;
use crate::sign;
use std::path::Path;

const DEFAULT_KEY: &str = "/etc/jpkg/keys/jonerix.sec";

const USAGE: &str = "\
usage: jpkg resign <pkg.jpkg>... [--key <secret-path>] [--key-id <id>] [--keep-existing] [--dry-run]

  --key <path>       Ed25519 secret key (default: /etc/jpkg/keys/jonerix.sec)
  --key-id <id>      Key identifier in the [signature] section (default: key filename stem)
  --keep-existing    Skip packages that already have a signature
  --dry-run          Validate without writing";

// ── Argument parsing ──────────────────────────────────────────────────────────

struct ResignArgs {
    paths: Vec<String>,
    key: String,
    key_id: Option<String>,
    keep_existing: bool,
    dry_run: bool,
}

fn parse_args(args: &[String]) -> Result<ResignArgs, String> {
    let mut paths: Vec<String> = Vec::new();
    let mut key: Option<String> = None;
    let mut key_id: Option<String> = None;
    let mut keep_existing = false;
    let mut dry_run = false;

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "--keep-existing" => {
                keep_existing = true;
                i += 1;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--key" => {
                i += 1;
                if i >= args.len() {
                    return Err("--key requires an argument".to_string());
                }
                key = Some(args[i].clone());
                i += 1;
            }
            "--key-id" => {
                i += 1;
                if i >= args.len() {
                    return Err("--key-id requires an argument".to_string());
                }
                key_id = Some(args[i].clone());
                i += 1;
            }
            s if s.starts_with("--key=") => {
                key = Some(s["--key=".len()..].to_string());
                i += 1;
            }
            s if s.starts_with("--key-id=") => {
                key_id = Some(s["--key-id=".len()..].to_string());
                i += 1;
            }
            s if s.starts_with('-') => {
                return Err(format!("unknown option: {s}"));
            }
            _ => {
                paths.push(a.clone());
                i += 1;
            }
        }
    }

    if paths.is_empty() {
        return Err("no .jpkg files specified".to_string());
    }

    let key = key.unwrap_or_else(|| DEFAULT_KEY.to_string());
    Ok(ResignArgs { paths, key, key_id, keep_existing, dry_run })
}

// ── Core per-file logic ───────────────────────────────────────────────────────

/// Outcome for a single file processed by resign.
enum FileResult {
    Resigned,
    Skipped,
    Error(String),
}

fn process_one(path: &Path, key_path: &Path, key_id: &str, keep_existing: bool, dry_run: bool) -> FileResult {
    // 1. Open the archive and check for an existing signature.
    let archive = match JpkgArchive::open(path) {
        Ok(a) => a,
        Err(e) => return FileResult::Error(format!("open failed: {e}")),
    };

    let meta_str = match archive.metadata_str() {
        Ok(s) => s,
        Err(e) => return FileResult::Error(format!("metadata UTF-8: {e}")),
    };

    let metadata = match Metadata::from_str(meta_str) {
        Ok(m) => m,
        Err(e) => return FileResult::Error(format!("parse metadata: {e}")),
    };

    // 2. Honor --keep-existing.
    if keep_existing && metadata.signature.is_some() {
        log::info!("resign: skipping {} (already signed, --keep-existing)", path.display());
        return FileResult::Skipped;
    }

    // 3. Dry-run: stop here.
    if dry_run {
        let action = if metadata.signature.is_some() { "re-sign" } else { "sign" };
        log::info!("resign: dry-run — would {} {}", action, path.display());
        return FileResult::Resigned;
    }

    // 4. Load the secret key and sign in-place.
    let sk = match sign::read_secret_key(key_path) {
        Ok(k) => k,
        Err(e) => return FileResult::Error(format!("load key {}: {e}", key_path.display())),
    };

    match crate::cmd::sign::sign_jpkg_in_place(path, &sk, key_id) {
        Ok(()) => FileResult::Resigned,
        Err(e) => FileResult::Error(e),
    }
}

// ── Subcommand dispatcher ─────────────────────────────────────────────────────

pub fn run(args: &[String]) -> i32 {
    let parsed = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("jpkg resign: {e}");
            eprintln!("{USAGE}");
            return 2;
        }
    };

    let key_path = Path::new(&parsed.key);

    // Derive key_id from the key file basename (minus the .sec extension)
    // if the caller did not supply one explicitly.
    let key_id_owned: String;
    let key_id: &str = if let Some(ref id) = parsed.key_id {
        id.as_str()
    } else {
        key_id_owned = key_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());
        &key_id_owned
    };

    let total = parsed.paths.len();
    let mut succeeded: usize = 0;
    let mut skipped: usize = 0;
    let mut failed: usize = 0;

    for path_str in &parsed.paths {
        let path = Path::new(path_str);
        match process_one(path, key_path, key_id, parsed.keep_existing, parsed.dry_run) {
            FileResult::Resigned => {
                if parsed.dry_run {
                    println!("dry-run: {}", path.display());
                } else {
                    println!("resigned: {}", path.display());
                }
                succeeded += 1;
            }
            FileResult::Skipped => {
                println!("skipped: {} (already signed, --keep-existing)", path.display());
                skipped += 1;
            }
            FileResult::Error(reason) => {
                eprintln!("error: {}: {}", path.display(), reason);
                failed += 1;
            }
        }
    }

    println!(
        "jpkg resign: {} succeeded, {} skipped, {} failed (of {})",
        succeeded, skipped, failed, total
    );

    if failed > 0 { 1 } else { 0 }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::JpkgArchive;
    use crate::cmd::common::tests::build_test_jpkg;
    use crate::cmd::sign::sign_jpkg_in_place;
    use crate::cmd::verify::verify_jpkg_signature;
    use crate::recipe::Metadata;
    use crate::sign;
    use std::fs;
    use tempfile::tempdir;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn setup_key(dir: &std::path::Path, name: &str) -> (std::path::PathBuf, std::path::PathBuf, ed25519_dalek::SigningKey) {
        let sk = sign::keygen();
        let sec_path = dir.join(format!("{name}.sec"));
        let pub_path = dir.join(format!("{name}.pub"));
        sign::write_secret_key(&sec_path, &sk).unwrap();
        sign::write_public_key(&pub_path, &sk.verifying_key()).unwrap();
        (sec_path, pub_path, sk)
    }

    /// Build an unsigned test jpkg, run resign, verify it — should succeed.
    #[test]
    fn test_resign_unsigned_jpkg_signs_it() {
        let dir = tempdir().unwrap();
        let (sec_path, pub_path, _sk) = setup_key(dir.path(), "resign-key");
        let keys_dir = dir.path().to_path_buf();

        let jpkg_path = build_test_jpkg(dir.path(), "unsigned-pkg", "1.0.0");

        let rc = run(&args(&[
            jpkg_path.to_str().unwrap(),
            "--key", sec_path.to_str().unwrap(),
            "--key-id", "resign-key",
        ]));
        assert_eq!(rc, 0, "resign of unsigned package should succeed");

        // Verify the signature was written.
        let result = verify_jpkg_signature(&jpkg_path, &keys_dir);
        assert!(result.is_ok(), "verify after resign must succeed: {:?}", result);

        // The .pub file must be in keys_dir (same dir in this test).
        let _ = pub_path; // used above indirectly via keys_dir
    }

    /// Sign with key A, then resign with key B — verify with key B OK, key A fails.
    #[test]
    fn test_resign_signed_jpkg_overwrites_signature() {
        let dir = tempdir().unwrap();

        let sk_a = sign::keygen();
        let sec_a = dir.path().join("key-a.sec");
        let pub_a = dir.path().join("keys-a").join("key-a.pub");
        fs::create_dir_all(dir.path().join("keys-a")).unwrap();
        sign::write_secret_key(&sec_a, &sk_a).unwrap();
        sign::write_public_key(&pub_a, &sk_a.verifying_key()).unwrap();

        let sk_b = sign::keygen();
        let sec_b = dir.path().join("key-b.sec");
        let pub_b = dir.path().join("keys-b").join("key-b.pub");
        fs::create_dir_all(dir.path().join("keys-b")).unwrap();
        sign::write_secret_key(&sec_b, &sk_b).unwrap();
        sign::write_public_key(&pub_b, &sk_b.verifying_key()).unwrap();

        let jpkg_path = build_test_jpkg(dir.path(), "overwrite-pkg", "1.0.0");

        // Initial sign with key A.
        sign_jpkg_in_place(&jpkg_path, &sk_a, "key-a").expect("initial sign must succeed");

        // Resign with key B (overwrite).
        let rc = run(&args(&[
            jpkg_path.to_str().unwrap(),
            "--key", sec_b.to_str().unwrap(),
            "--key-id", "key-b",
        ]));
        assert_eq!(rc, 0, "resign with key B should succeed");

        // Verify with key B — must succeed.
        let result_b = verify_jpkg_signature(&jpkg_path, &dir.path().join("keys-b"));
        assert!(result_b.is_ok(), "verify with key B after resign must succeed: {:?}", result_b);

        // Verify with key A only — must fail (signature was overwritten).
        let result_a = verify_jpkg_signature(&jpkg_path, &dir.path().join("keys-a"));
        assert!(result_a.is_err(), "verify with key A should fail after resign with key B");
    }

    /// Sign with key A, resign with --keep-existing and key B — key A sig preserved.
    #[test]
    fn test_resign_keep_existing_skips() {
        let dir = tempdir().unwrap();

        let sk_a = sign::keygen();
        let sec_a = dir.path().join("ka.sec");
        let pub_a = dir.path().join("keys-a").join("ka.pub");
        fs::create_dir_all(dir.path().join("keys-a")).unwrap();
        sign::write_secret_key(&sec_a, &sk_a).unwrap();
        sign::write_public_key(&pub_a, &sk_a.verifying_key()).unwrap();

        let sk_b = sign::keygen();
        let sec_b = dir.path().join("kb.sec");
        sign::write_secret_key(&sec_b, &sk_b).unwrap();
        // No pub for key B — it must NOT have been used.

        let jpkg_path = build_test_jpkg(dir.path(), "keep-existing-pkg", "1.0.0");

        // Sign with key A.
        sign_jpkg_in_place(&jpkg_path, &sk_a, "ka").expect("initial sign must succeed");

        // Resign with key B + --keep-existing → should be skipped.
        let rc = run(&args(&[
            jpkg_path.to_str().unwrap(),
            "--key", sec_b.to_str().unwrap(),
            "--key-id", "kb",
            "--keep-existing",
        ]));
        assert_eq!(rc, 0, "resign --keep-existing should return 0");

        // Signature from key A must still verify.
        let result = verify_jpkg_signature(&jpkg_path, &dir.path().join("keys-a"));
        assert!(result.is_ok(), "key A signature must still verify after --keep-existing skip: {:?}", result);
    }

    /// Pass three test jpkgs — all three must be signed.
    #[test]
    fn test_resign_multiple_files() {
        let dir = tempdir().unwrap();
        let (sec_path, _pub_path, _sk) = setup_key(dir.path(), "multi-key");
        let keys_dir = dir.path().to_path_buf();

        let p1 = build_test_jpkg(dir.path(), "multi-a", "1.0.0");
        let p2 = build_test_jpkg(dir.path(), "multi-b", "1.0.0");
        let p3 = build_test_jpkg(dir.path(), "multi-c", "1.0.0");

        let rc = run(&args(&[
            p1.to_str().unwrap(),
            p2.to_str().unwrap(),
            p3.to_str().unwrap(),
            "--key", sec_path.to_str().unwrap(),
            "--key-id", "multi-key",
        ]));
        assert_eq!(rc, 0, "resign of three packages should succeed");

        for path in [&p1, &p2, &p3] {
            let result = verify_jpkg_signature(path, &keys_dir);
            assert!(result.is_ok(), "verify of {} must succeed: {:?}", path.display(), result);
        }
    }

    /// --dry-run must not write any signature to the package.
    #[test]
    fn test_resign_dry_run_no_writes() {
        let dir = tempdir().unwrap();
        let (sec_path, _pub_path, _sk) = setup_key(dir.path(), "dry-key");

        let jpkg_path = build_test_jpkg(dir.path(), "dry-run-pkg", "1.0.0");

        // Read original bytes before dry-run.
        let original_bytes = fs::read(&jpkg_path).unwrap();

        let rc = run(&args(&[
            jpkg_path.to_str().unwrap(),
            "--key", sec_path.to_str().unwrap(),
            "--key-id", "dry-key",
            "--dry-run",
        ]));
        assert_eq!(rc, 0, "dry-run should return 0");

        // File on disk must be byte-for-byte identical.
        let after_bytes = fs::read(&jpkg_path).unwrap();
        assert_eq!(original_bytes, after_bytes, "dry-run must not modify the .jpkg file");

        // Parse metadata — must still have no signature.
        let archive = JpkgArchive::open(&jpkg_path).unwrap();
        let meta_str = archive.metadata_str().unwrap();
        let metadata = Metadata::from_str(meta_str).unwrap();
        assert!(
            metadata.signature.is_none(),
            "dry-run must leave signature as None in the package metadata"
        );
    }
}
