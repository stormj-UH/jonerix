// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg keygen [<name>] [--dir <path>]`
//!
//! Generates an Ed25519 keypair and writes two files:
//!   `<dir>/<name>.pub`  — 32 raw bytes, mode 0644
//!   `<dir>/<name>.sec`  — 64 raw bytes (seed||pubkey), mode 0600
//!
//! `<name>` defaults to `default`.
//! `<dir>`  defaults to `$JPKG_ROOT/etc/jpkg/keys`.

use crate::sign;
use std::path::PathBuf;

const USAGE: &str = "usage: jpkg keygen [<name>] [--dir <path>]";

/// Run the `jpkg keygen` subcommand.
///
/// Generates a fresh Ed25519 keypair and writes `<name>.pub` (mode 0644)
/// and `<name>.sec` (mode 0600) into the key directory.  The key directory
/// defaults to `$JPKG_ROOT/etc/jpkg/keys`; override with `--dir <path>`.
/// Returns 0 on success, 1 on I/O error, or 2 on usage error.
pub fn run(args: &[String]) -> i32 {
    // ── Argument parsing ─────────────────────────────────────────────────────
    let mut name: Option<String> = None;
    let mut dir_override: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                if i + 1 >= args.len() {
                    eprintln!("jpkg keygen: --dir requires a path argument");
                    eprintln!("{USAGE}");
                    return 2;
                }
                dir_override = Some(args[i + 1].clone());
                i += 2;
            }
            s if s.starts_with('-') => {
                eprintln!("jpkg keygen: unknown option: {s}");
                eprintln!("{USAGE}");
                return 2;
            }
            s => {
                if name.is_some() {
                    eprintln!("jpkg keygen: unexpected argument: {s}");
                    eprintln!("{USAGE}");
                    return 2;
                }
                name = Some(s.to_string());
                i += 1;
            }
        }
    }

    let name = name.unwrap_or_else(|| "default".to_string());

    // ── Resolve output directory ─────────────────────────────────────────────
    let dir: PathBuf = if let Some(d) = dir_override {
        PathBuf::from(d)
    } else {
        let root = std::env::var("JPKG_ROOT").unwrap_or_else(|_| String::new());
        let mut p = PathBuf::from(root);
        p.push("etc/jpkg/keys");
        p
    };

    // ── Create directory ─────────────────────────────────────────────────────
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "jpkg keygen: cannot create directory {}: {e}",
            dir.display()
        );
        return 1;
    }

    // Set mode 0755 on the directory.
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        // Ignore error — directory may be pre-existing with other perms.
        let _ = std::fs::set_permissions(&dir, perms);
    }

    let pub_path = dir.join(format!("{name}.pub"));
    let sec_path = dir.join(format!("{name}.sec"));

    // ── Collision guard ───────────────────────────────────────────────────────
    if pub_path.exists() {
        eprintln!("jpkg keygen: key already exists at {}", pub_path.display());
        return 1;
    }
    if sec_path.exists() {
        eprintln!("jpkg keygen: key already exists at {}", sec_path.display());
        return 1;
    }

    // ── Generate keypair ─────────────────────────────────────────────────────
    let sk = sign::keygen();
    let vk = sk.verifying_key();

    if let Err(e) = sign::write_public_key(&pub_path, &vk) {
        eprintln!("jpkg keygen: failed to write {}: {e}", pub_path.display());
        return 1;
    }
    if let Err(e) = sign::write_secret_key(&sec_path, &sk) {
        eprintln!("jpkg keygen: failed to write {}: {e}", sec_path.display());
        // Best-effort cleanup of the already-written public key.
        let _ = std::fs::remove_file(&pub_path);
        return 1;
    }

    println!(
        "Generated keypair: {}/{{{name}.pub,{name}.sec}}",
        dir.display()
    );
    0
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // 1. Generate to a tempdir; assert both files exist with correct sizes.
    #[test]
    fn test_keygen_creates_files() {
        let dir = tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap().to_string();

        let rc = run(&args(&["--dir", &dir_str]));
        assert_eq!(rc, 0, "expected exit 0");

        let pub_path = dir.path().join("default.pub");
        let sec_path = dir.path().join("default.sec");

        assert!(pub_path.exists(), "default.pub must exist");
        assert!(sec_path.exists(), "default.sec must exist");

        let pub_bytes = std::fs::read(&pub_path).unwrap();
        let sec_bytes = std::fs::read(&sec_path).unwrap();

        assert_eq!(pub_bytes.len(), 32, "public key must be 32 bytes");
        assert_eq!(sec_bytes.len(), 64, "secret key must be 64 bytes");
    }

    // 2. Re-run with the same name; expect exit 1 and no files modified.
    #[test]
    fn test_keygen_refuses_overwrite() {
        let dir = tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap().to_string();

        let rc1 = run(&args(&["--dir", &dir_str]));
        assert_eq!(rc1, 0, "first run must succeed");

        let pub_path = dir.path().join("default.pub");
        let meta_before = std::fs::metadata(&pub_path).unwrap();
        let mtime_before = meta_before.modified().unwrap();

        let rc2 = run(&args(&["--dir", &dir_str]));
        assert_eq!(rc2, 1, "second run must return 1 (collision)");

        // File must not have been touched.
        let meta_after = std::fs::metadata(&pub_path).unwrap();
        let mtime_after = meta_after.modified().unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "existing .pub must not be modified"
        );
    }

    // 3. Generate with explicit --dir; assert keys land in that dir.
    #[test]
    fn test_keygen_explicit_dir() {
        let dir = tempdir().unwrap();
        let subdir = dir.path().join("mykeys");
        let subdir_str = subdir.to_str().unwrap().to_string();

        let rc = run(&args(&["mykey", "--dir", &subdir_str]));
        assert_eq!(rc, 0, "expected exit 0");

        assert!(
            subdir.join("mykey.pub").exists(),
            "mykey.pub must be in explicit dir"
        );
        assert!(
            subdir.join("mykey.sec").exists(),
            "mykey.sec must be in explicit dir"
        );
    }

    // 4. Verify mode bits: secret 0600, public 0644.
    #[test]
    fn test_keygen_file_permissions() {
        let dir = tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap().to_string();

        let rc = run(&args(&["permtest", "--dir", &dir_str]));
        assert_eq!(rc, 0, "expected exit 0");

        let pub_mode = std::fs::metadata(dir.path().join("permtest.pub"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let sec_mode = std::fs::metadata(dir.path().join("permtest.sec"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(
            pub_mode, 0o644,
            "public key must have mode 0644, got {:04o}",
            pub_mode
        );
        assert_eq!(
            sec_mode, 0o600,
            "secret key must have mode 0600, got {:04o}",
            sec_mode
        );
    }

    // 5. Wrong number of positional args → exit 2.
    #[test]
    fn test_keygen_too_many_args() {
        let dir = tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap().to_string();
        let rc = run(&args(&["name1", "name2", "--dir", &dir_str]));
        assert_eq!(rc, 2, "extra positional arg must return exit 2");
    }

    // 6. --dir missing its argument → exit 2.
    #[test]
    fn test_keygen_dir_missing_value() {
        let rc = run(&args(&["--dir"]));
        assert_eq!(rc, 2);
    }
}
