// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg owns` — which installed package owns a path.
//!
//! ```text
//! jpkg owns <path>...
//! jpkg owns --conflicts
//! ```
//!
//! `jpkg owns /bin/ping` prints `/bin/ping is owned by toybox 0.8.11-r14`.
//! Every owner is listed, so a path two packages both claim shows up as a
//! file conflict.  Exit status is 0 when every path has an owner and 1 when
//! any path is unowned.
//!
//! `jpkg owns --conflicts` lists every non-directory path that more than one
//! installed package claims, one per line (`/path: pkgA pkgB`), noting when
//! one owner declares `replaces = [...]` for another.  Exit status is 1 when
//! any conflict exists, 0 when there are none.
//!
//! Paths are matched against the installed manifests, which store them
//! relative to the root (`bin/ping`).  Relative arguments are resolved
//! against the current directory (inside `--root` they are taken relative to
//! that root), `.`/`..` are folded, `/usr/...` and `/lib64/...` map to their
//! merged-usr locations, and symlinked parent directories on disk are
//! followed as a second guess.  Only the world-readable installed database
//! is read, so this works without root.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::cmd::common::resolve_rootfs;
use crate::db::{DbError, InstalledDb, Ownership};

const USAGE: &str = "usage: jpkg owns <path>...\n       jpkg owns --conflicts";

/// Run `jpkg owns`.
pub fn run(args: &[String]) -> i32 {
    let mut conflicts = false;
    let mut paths: Vec<&str> = Vec::new();
    for a in args {
        match a.as_str() {
            "--conflicts" | "-c" => conflicts = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
            }
            s if s.starts_with('-') && s.len() > 1 => {
                eprintln!("jpkg owns: unknown option: {s}\n{USAGE}");
                return 2;
            }
            s => paths.push(s),
        }
    }
    if conflicts == !paths.is_empty() {
        eprintln!("{USAGE}");
        return 2;
    }

    let rootfs = resolve_rootfs(None);
    let db = match InstalledDb::open(&rootfs) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("jpkg: failed to open database: {e}");
            return 1;
        }
    };

    if conflicts {
        return match conflict_lines(&db) {
            Ok(lines) => {
                for l in &lines {
                    println!("{l}");
                }
                eprintln!(
                    "jpkg: {} path(s) owned by more than one package",
                    lines.len()
                );
                i32::from(!lines.is_empty())
            }
            Err(e) => {
                eprintln!("jpkg: {e}");
                1
            }
        };
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let mut rc = 0;
    match owner_lines(&db, &rootfs, &cwd, &paths) {
        Ok(results) => {
            for r in results {
                match r {
                    Ok(line) => println!("{line}"),
                    Err(line) => {
                        eprintln!("{line}");
                        rc = 1;
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("jpkg: {e}");
            rc = 1;
        }
    }
    rc
}

/// Candidate manifest keys for a user-supplied path, most likely first.
pub(crate) fn manifest_keys(rootfs: &Path, cwd: &Path, arg: &str) -> Vec<String> {
    let raw = Path::new(arg);
    // Relative arguments follow the usual CLI convention (relative to the
    // current directory) on the live system; inside an alternate root they
    // are taken relative to that root.
    let abs: PathBuf = if raw.is_absolute() || rootfs != Path::new("/") {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    };

    let mut keys: Vec<String> = Vec::new();
    let lexical = merged_usr(&lexical_rel(&abs));
    if !lexical.is_empty() {
        keys.push(lexical.clone());
    }

    // Second guess: follow symlinked parent directories on disk
    // (e.g. /sbin -> bin), staying inside the root.
    if let (Some(parent), Some(name)) = (
        Path::new(&lexical).parent(),
        Path::new(&lexical).file_name(),
    ) {
        if let (Ok(canon_root), Ok(canon_parent)) = (
            std::fs::canonicalize(rootfs),
            std::fs::canonicalize(rootfs.join(parent)),
        ) {
            if let Ok(rel) = canon_parent.strip_prefix(&canon_root) {
                let physical = merged_usr(&lexical_rel(&Path::new("/").join(rel).join(name)));
                if !physical.is_empty() && !keys.contains(&physical) {
                    keys.push(physical);
                }
            }
        }
    }
    keys
}

/// Root-relative, lexically normalised form of an absolute path.
fn lexical_rel(p: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop();
            }
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    parts.join("/")
}

/// Apply jonerix's merged-usr layout: `usr/x` → `x`, `lib64/x` → `lib/x`.
fn merged_usr(rel: &str) -> String {
    let mut s = rel;
    while let Some(rest) = s.strip_prefix("usr/") {
        s = rest;
    }
    if s == "usr" {
        return String::new();
    }
    if s == "lib64" {
        return "lib".to_string();
    }
    if let Some(rest) = s.strip_prefix("lib64/") {
        return format!("lib/{rest}");
    }
    s.to_string()
}

/// One result per argument: `Ok(line)` for an owned path, `Err(line)` for an
/// unowned one.
pub(crate) fn owner_lines(
    db: &InstalledDb,
    rootfs: &Path,
    cwd: &Path,
    args: &[&str],
) -> Result<Vec<Result<String, String>>, DbError> {
    let per_arg: Vec<Vec<String>> = args.iter().map(|a| manifest_keys(rootfs, cwd, a)).collect();
    let wanted: HashSet<&str> = per_arg.iter().flatten().map(String::as_str).collect();
    let own = db.path_owners(Some(&wanted), None)?;

    let mut out = Vec::new();
    for (arg, keys) in args.iter().zip(&per_arg) {
        match keys.iter().find(|k| own.is_claimed(k)) {
            Some(key) => out.push(Ok(describe(&own, key))),
            None => out.push(Err(format!("error: no package owns {arg}"))),
        }
    }
    Ok(out)
}

fn describe(own: &Ownership, key: &str) -> String {
    let claims = own.owners_of(key);
    let owners: Vec<String> = claims
        .iter()
        .map(|c| match own.versions.get(&c.owner) {
            Some(v) if !v.is_empty() => format!("{} {v}", c.owner),
            _ => c.owner.clone(),
        })
        .collect();
    let files = claims.iter().filter(|c| !c.is_dir).count();
    let mut line = format!("/{key} is owned by {}", owners.join(", "));
    if files > 1 {
        line.push_str(&format!(" (file conflict: {files} packages own this path)"));
    }
    line
}

/// `jpkg owns --conflicts`: every non-directory path with more than one
/// owner, as `/path: owner owner [note]`, in path order.
pub(crate) fn conflict_lines(db: &InstalledDb) -> Result<Vec<String>, DbError> {
    let own = db.path_owners(None, None)?;
    let mut out = Vec::new();
    for (path, claims) in &own.claims {
        let files: Vec<&str> = claims
            .iter()
            .filter(|c| !c.is_dir)
            .map(|c| c.owner.as_str())
            .collect();
        if files.len() < 2 {
            continue;
        }
        let mut line = format!("/{path}: {}", files.join(" "));
        let notes: Vec<String> = files
            .iter()
            .flat_map(|a| files.iter().map(move |b| (*a, *b)))
            .filter(|(a, b)| a != b && own.owner_replaces(a, b))
            .map(|(a, b)| format!("{a} replaces {b}"))
            .collect();
        if !notes.is_empty() {
            line.push_str(&format!(" [{}]", notes.join("; ")));
        }
        out.push(line);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::tests::{make_metadata, make_metadata_with_replaces};
    use crate::db::{FileEntry, InstalledPkg};
    use std::fs;
    use tempfile::TempDir;

    fn file(p: &str) -> FileEntry {
        FileEntry {
            path: p.to_string(),
            sha256: "a".repeat(64),
            size: 0,
            mode: 0o100755,
            symlink_target: None,
            is_dir: false,
        }
    }

    fn link(p: &str, t: &str) -> FileEntry {
        FileEntry {
            path: p.to_string(),
            sha256: String::new(),
            size: 0,
            mode: 0o120777,
            symlink_target: Some(t.to_string()),
            is_dir: false,
        }
    }

    fn dir(p: &str) -> FileEntry {
        FileEntry {
            path: p.to_string(),
            sha256: "0".repeat(64),
            size: 0,
            mode: 0o040755,
            symlink_target: None,
            is_dir: true,
        }
    }

    /// A small database shaped like tormenta's: toybox and jonerix-util both
    /// claim bin/hwclock (jonerix-util replaces toybox), openrc and the Pi
    /// fixups both claim etc/init.d/hwclock (no replaces relationship).
    fn fixture() -> (TempDir, InstalledDb) {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();
        let add = |m, files| db.insert(&InstalledPkg { metadata: m, files }).unwrap();
        add(
            make_metadata("toybox", "0.8.11-r14"),
            vec![
                dir("bin"),
                link("bin/ping", "toybox"),
                link("bin/hwclock", "toybox"),
            ],
        );
        add(
            make_metadata_with_replaces("jonerix-util", "0.1.1", vec!["toybox".into()]),
            vec![dir("bin"), file("bin/hwclock")],
        );
        add(
            make_metadata("openrc", "0.54-r7"),
            vec![dir("etc"), file("etc/init.d/hwclock")],
        );
        add(
            make_metadata("fixups", "1.6.32"),
            vec![dir("etc"), file("etc/init.d/hwclock")],
        );
        (tmp, db)
    }

    #[test]
    fn owns_finds_single_owner_with_version() {
        let (tmp, db) = fixture();
        let r = owner_lines(&db, tmp.path(), Path::new("/"), &["/bin/ping"]).unwrap();
        assert_eq!(
            r,
            vec![Ok("/bin/ping is owned by toybox 0.8.11-r14".to_string())]
        );
    }

    #[test]
    fn owns_reports_every_owner_of_a_conflicting_path() {
        let (tmp, db) = fixture();
        let r = owner_lines(&db, tmp.path(), Path::new("/"), &["/etc/init.d/hwclock"]).unwrap();
        assert_eq!(
            r,
            vec![Ok(
                "/etc/init.d/hwclock is owned by fixups 1.6.32, openrc 0.54-r7 \
                     (file conflict: 2 packages own this path)"
                    .to_string()
            )]
        );
    }

    #[test]
    fn owns_directory_shared_by_packages_is_not_a_conflict() {
        let (tmp, db) = fixture();
        let r = owner_lines(&db, tmp.path(), Path::new("/"), &["/etc/"]).unwrap();
        assert_eq!(
            r,
            vec![Ok(
                "/etc is owned by fixups 1.6.32, openrc 0.54-r7".to_string()
            )]
        );
    }

    #[test]
    fn owns_unowned_path_is_an_error_line() {
        let (tmp, db) = fixture();
        let r = owner_lines(&db, tmp.path(), Path::new("/"), &["/bin/nope", "/bin/ping"]).unwrap();
        assert_eq!(r[0], Err("error: no package owns /bin/nope".to_string()));
        assert!(r[1].is_ok());
    }

    #[test]
    fn owns_normalises_usr_dots_and_relative_paths() {
        let (tmp, db) = fixture();
        // Live-system semantics: relative to the current directory.
        let keys = manifest_keys(
            Path::new("/"),
            Path::new("/etc"),
            "init.d/../init.d/hwclock",
        );
        assert_eq!(keys[0], "etc/init.d/hwclock");
        assert_eq!(
            manifest_keys(Path::new("/"), Path::new("/"), "/usr/bin/ping")[0],
            "bin/ping"
        );
        assert_eq!(
            manifest_keys(Path::new("/"), Path::new("/"), "/lib64/ld.so")[0],
            "lib/ld.so"
        );
        assert!(manifest_keys(Path::new("/"), Path::new("/"), "/usr").is_empty());
        // Alternate root: relative arguments are root-relative.
        let r = owner_lines(&db, tmp.path(), Path::new("/somewhere"), &["usr/bin/ping"]).unwrap();
        assert!(r[0]
            .as_ref()
            .unwrap()
            .starts_with("/bin/ping is owned by toybox"));
    }

    #[test]
    fn owns_follows_symlinked_parent_directory() {
        let (tmp, db) = fixture();
        std::os::unix::fs::symlink("bin", tmp.path().join("sbin")).unwrap();
        fs::create_dir_all(tmp.path().join("bin")).unwrap();
        let r = owner_lines(&db, tmp.path(), Path::new("/"), &["/sbin/ping"]).unwrap();
        assert_eq!(
            r,
            vec![Ok("/bin/ping is owned by toybox 0.8.11-r14".to_string())]
        );
    }

    #[test]
    fn conflicts_lists_multi_owned_files_only() {
        let (_tmp, db) = fixture();
        assert_eq!(
            conflict_lines(&db).unwrap(),
            vec![
                "/bin/hwclock: jonerix-util toybox [jonerix-util replaces toybox]".to_string(),
                "/etc/init.d/hwclock: fixups openrc".to_string(),
            ]
        );
    }
}
