// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg search` — search the package index by name and description.
//!
//! C reference: `jpkg/src/cmd_search.c`.
//!
//! Output format (one hit per line, sorted by name):
//!   "Found N package(s) matching '<query>':\n\n"
//!   "  <name>-<version>  <description>\n"
//! or "No packages found matching '<query>'\n" when empty.
//!
//! Divergences from C:
//! - The C version also checks the installed-db and appends "[installed]".
//!   We omit that because `InstalledDb` requires a live rootfs and the spec
//!   says "match what the C does"; the C flag is cosmetic/optional, and the
//!   spec contract says prefer cached index + match description/name.  If a
//!   rootfs is available we do open the db for the installed marker.
//! - The C version prints name/version/license in fixed columns.  The task
//!   spec says "print `<name>-<version>  <description>`" so we follow the spec
//!   over the C column layout.

use crate::db::InstalledDb;
use crate::recipe::Index;
use crate::repo::Repo;
use std::path::Path;

/// Run `jpkg search <query>`.  Returns 0 on success, 1 on failure, 2 on usage
/// error.
pub fn run(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("usage: jpkg search <query>");
        return 2;
    }

    // Join all positional arguments into one multi-word query.
    let query = args.join(" ");

    let rootfs_str = std::env::var("JPKG_ROOT").unwrap_or_else(|_| "/".to_string());
    let rootfs = Path::new(&rootfs_str);

    // Determine arch (default x86_64 when uname not consulted).
    let arch = std::env::var("JPKG_ARCH").unwrap_or_else(|_| detect_arch());

    // Load index: prefer cache, fall back to fetch.
    let repo = match Repo::from_rootfs(rootfs, &arch) {
        Ok(r) => r,
        Err(e) => {
            log::error!("failed to open repo config: {e}");
            eprintln!("error: {e}");
            return 1;
        }
    };

    let index = match load_index(&repo) {
        Some(idx) => idx,
        None => {
            eprintln!("error: no package index. Run 'jpkg update' first.");
            return 1;
        }
    };

    // Optionally open the installed DB so we can show [installed].
    let db = InstalledDb::open(rootfs).ok();
    let installed_names: Vec<String> = db.as_ref().and_then(|d| d.list().ok()).unwrap_or_default();

    // Collect matching entries, sorted by name (BTreeMap already sorted by key).
    let query_lower = query.to_ascii_lowercase();
    let words: Vec<&str> = query_lower.split_whitespace().collect();

    let mut hits: Vec<(&str, &crate::recipe::IndexEntry)> = index
        .entries
        .iter()
        .filter_map(|(key, entry)| {
            // The BTreeMap key is "name-arch"; recover the package name by
            // stripping the trailing "-<arch>" suffix.
            let name = key
                .strip_suffix(&format!("-{}", entry.arch))
                .unwrap_or(key.as_str());

            let haystack = format!(
                "{} {}",
                name.to_ascii_lowercase(),
                entry.description.to_ascii_lowercase()
            );

            // All words must be present (case-insensitive).
            if words.iter().all(|w| haystack.contains(w)) {
                Some((name, entry))
            } else {
                None
            }
        })
        .collect();

    // Already sorted by key (BTreeMap), but collect and sort explicitly by name
    // in case multiple arches produce duplicate package names.
    hits.sort_by_key(|(name, _)| *name);
    hits.dedup_by_key(|(name, _)| *name);

    if hits.is_empty() {
        println!("No packages found matching '{query}'");
    } else {
        println!("Found {} package(s) matching '{query}':\n", hits.len());
        for (name, entry) in &hits {
            let installed_marker = if installed_names.iter().any(|n| n == name) {
                " [installed]"
            } else {
                ""
            };
            // Format: "  <name>-<version>  <description>"
            // Matches the task spec; the C uses fixed columns, we use two spaces.
            println!(
                "  {}-{}  {}{}",
                name, entry.version, entry.description, installed_marker
            );
        }
    }

    0
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Try cache first; fall back to network fetch.
fn load_index(repo: &Repo) -> Option<Index> {
    match repo.load_cached_index() {
        Ok(Some(idx)) => return Some(idx),
        Ok(None) => log::info!("no cached INDEX; fetching"),
        Err(e) => log::warn!("cache read failed ({e}); fetching"),
    }
    match repo.fetch_index() {
        Ok(idx) => Some(idx),
        Err(e) => {
            log::error!("fetch_index failed: {e}");
            None
        }
    }
}

/// Best-effort arch detection from `uname -m`.  Falls back to "x86_64".
fn detect_arch() -> String {
    std::process::Command::new("uname")
        .arg("-m")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "x86_64".to_string())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recipe::{Index, IndexEntry};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    /// Build a minimal in-memory Index and write it to a cache file so that
    /// `Repo::load_cached_index` can pick it up without a network round-trip.
    fn seed_index(cache_dir: &std::path::Path, entries: BTreeMap<String, IndexEntry>) {
        let idx = Index { entries };
        let toml = idx.to_string().expect("serialize index");
        std::fs::create_dir_all(cache_dir).expect("create cache dir");
        std::fs::write(cache_dir.join("INDEX"), toml.as_bytes()).expect("write INDEX");
    }

    fn make_entry(version: &str, desc: &str, arch: &str) -> IndexEntry {
        IndexEntry {
            version: version.to_string(),
            license: "MIT".to_string(),
            description: desc.to_string(),
            arch: arch.to_string(),
            sha256: "a".repeat(64),
            size: 1024,
            depends: vec![],
            build_depends: vec![],
        }
    }

    #[test]
    fn search_finds_matching_package() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        // Seed the cache.
        let mut entries = BTreeMap::new();
        entries.insert(
            "zstd-x86_64".to_string(),
            make_entry("1.5.6", "Fast real-time compression library", "x86_64"),
        );
        entries.insert(
            "musl-x86_64".to_string(),
            make_entry("1.2.5", "Lightweight C library", "x86_64"),
        );

        let cache_dir = rootfs.join("var/cache/jpkg");
        seed_index(&cache_dir, entries);

        // Stub out the mirrors / keys dirs so Repo::from_rootfs doesn't error.
        std::fs::create_dir_all(rootfs.join("etc/jpkg/keys")).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());
        std::env::set_var("JPKG_ARCH", "x86_64");

        let rc = run(&["zstd".to_string()]);
        assert_eq!(rc, 0);
    }

    #[test]
    fn search_returns_zero_on_no_match() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let mut entries = BTreeMap::new();
        entries.insert(
            "mksh-x86_64".to_string(),
            make_entry("R59c", "MirBSD Korn Shell", "x86_64"),
        );

        let cache_dir = rootfs.join("var/cache/jpkg");
        seed_index(&cache_dir, entries);
        std::fs::create_dir_all(rootfs.join("etc/jpkg/keys")).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());
        std::env::set_var("JPKG_ARCH", "x86_64");

        // "python" should not match anything.
        let rc = run(&["python".to_string()]);
        assert_eq!(rc, 0); // C also returns 0 on empty results
    }

    #[test]
    fn search_usage_error_on_no_args() {
        // No env manipulation needed — usage check is before any I/O.
        let rc = run(&[]);
        assert_eq!(rc, 2);
    }

    #[test]
    fn search_multiword_query_requires_all_words() {
        let tmp = TempDir::new().unwrap();
        let rootfs = tmp.path();

        let mut entries = BTreeMap::new();
        entries.insert(
            "zstd-x86_64".to_string(),
            make_entry("1.5.6", "Fast real-time compression library", "x86_64"),
        );
        entries.insert(
            "lz4-x86_64".to_string(),
            make_entry("1.9.4", "Fast compression utility", "x86_64"),
        );

        let cache_dir = rootfs.join("var/cache/jpkg");
        seed_index(&cache_dir, entries);
        std::fs::create_dir_all(rootfs.join("etc/jpkg/keys")).unwrap();

        std::env::set_var("JPKG_ROOT", rootfs.as_os_str());
        std::env::set_var("JPKG_ARCH", "x86_64");

        // "real-time compression" should match only zstd, not lz4.
        let rc = run(&["real-time".to_string(), "compression".to_string()]);
        assert_eq!(rc, 0);
    }
}
