// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg list` — list installed packages.
//!
//! ```text
//! jpkg list [-q|--quiet] [<substring>...]
//! ```
//!
//! Prints `<name> <version>` per installed package, sorted by name.  With
//! `-q` only the names are printed.  Positional arguments filter by name
//! (a package is listed when its name contains any of them).  Reads only
//! the world-readable installed database, so it works without root.

use crate::cmd::common::resolve_rootfs;
use crate::db::{DbError, InstalledDb};

/// Run `jpkg list`.  Returns 0 on success, 1 on a database error, 2 on a
/// usage error.
pub fn run(args: &[String]) -> i32 {
    let mut names_only = false;
    let mut filters: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "-q" | "--quiet" => names_only = true,
            "-h" | "--help" => {
                println!("usage: jpkg list [-q|--quiet] [<substring>...]");
                return 0;
            }
            s if s.starts_with('-') => {
                eprintln!("jpkg list: unknown option: {s}");
                eprintln!("usage: jpkg list [-q|--quiet] [<substring>...]");
                return 2;
            }
            s => filters.push(s.to_string()),
        }
    }

    let rootfs = resolve_rootfs(None);
    let db = match InstalledDb::open(&rootfs) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("jpkg: failed to open database: {e}");
            return 1;
        }
    };
    match list_lines(&db, &filters, names_only) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            0
        }
        Err(e) => {
            eprintln!("jpkg: {e}");
            1
        }
    }
}

/// The lines `jpkg list` prints, in name order.
pub(crate) fn list_lines(
    db: &InstalledDb,
    filters: &[String],
    names_only: bool,
) -> Result<Vec<String>, DbError> {
    let mut out = Vec::new();
    for name in db.list()? {
        if !filters.is_empty() && !filters.iter().any(|f| name.contains(f.as_str())) {
            continue;
        }
        if names_only {
            out.push(name);
            continue;
        }
        let version = match db.get(&name) {
            Ok(Some(p)) => p.metadata.package.version.unwrap_or_else(|| "?".into()),
            Ok(None) => continue,
            Err(e) => {
                log::warn!("jpkg: cannot read record for {name}: {e}");
                "?".to_string()
            }
        };
        out.push(format!("{name} {version}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::tests::make_metadata;
    use crate::db::InstalledPkg;
    use tempfile::TempDir;

    fn db_with(names: &[(&str, &str)]) -> (TempDir, InstalledDb) {
        let tmp = TempDir::new().unwrap();
        let db = InstalledDb::open(tmp.path()).unwrap();
        for (n, v) in names {
            db.insert(&InstalledPkg {
                metadata: make_metadata(n, v),
                files: vec![],
            })
            .unwrap();
        }
        (tmp, db)
    }

    #[test]
    fn list_prints_name_and_version_sorted() {
        let (_t, db) = db_with(&[
            ("toybox", "0.8.11-r14"),
            ("jpkg", "2.2.10"),
            ("mksh", "R59c"),
        ]);
        assert_eq!(
            list_lines(&db, &[], false).unwrap(),
            vec!["jpkg 2.2.10", "mksh R59c", "toybox 0.8.11-r14"]
        );
    }

    #[test]
    fn list_quiet_and_filter() {
        let (_t, db) = db_with(&[("toybox", "1"), ("uutils", "2"), ("jpkg", "3")]);
        assert_eq!(
            list_lines(&db, &["box".to_string(), "utils".to_string()], true).unwrap(),
            vec!["toybox", "uutils"]
        );
    }

    #[test]
    fn list_empty_db() {
        let (_t, db) = db_with(&[]);
        assert!(list_lines(&db, &[], false).unwrap().is_empty());
    }
}
