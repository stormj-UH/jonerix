//! `jpkg build-world` — rebuild the entire jonerix package set from source.
//!
//! Ported from the loop in `packages/jpkg/src/cmd_build.c` (`cmd_build_world`)
//! and the CI driver in `scripts/build-all.sh`.
//!
//! # Behaviour
//!
//! Reads `build-order.txt` from one of several well-known locations (see
//! `find_build_order_file`), iterates every non-blank / non-comment line,
//! finds the matching `packages/<tier>/<name>/recipe.toml`, and calls
//! `cmd::build::run_build`.  Failures are collected; the summary is printed
//! at the end and the exit code is 1 if any package failed.
//!
//! # Divergences from C `cmd_build_world`
//!
//! * The C version uses a hard-coded `build_world_order[]` array.  We read
//!   `scripts/build-order.txt` instead (same as `build-all.sh`), which is the
//!   authoritative source of package order.
//! * The C version also accepts `--recipes <dir>`.  We do not; the tier-based
//!   discovery (packages/*/<name>) matches the real tree layout and is more
//!   robust than a flat recipes dir.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cmd::build::run_build;

// ── build-order.txt search ────────────────────────────────────────────────────

/// Search candidate paths for `build-order.txt` and return the first one found.
///
/// Resolution order (mirrors where the file lives in the jonerix repo and what
/// an installed jpkg would ship):
/// 1. `JPKG_ROOT` env var + `/usr/share/jpkg/build-order.txt`
/// 2. Relative to the Cargo manifest dir (for tests / dev runs).
/// 3. `/usr/share/jpkg/build-order.txt` (installed system path).
/// 4. `scripts/build-order.txt` relative to CWD (CI runner).
fn find_build_order_file() -> Option<PathBuf> {
    // 1. JPKG_ROOT env var.
    if let Ok(root) = std::env::var("JPKG_ROOT") {
        let p = PathBuf::from(&root)
            .join("usr/share/jpkg/build-order.txt");
        if p.exists() {
            return Some(p);
        }
        // Also try JPKG_ROOT/scripts/build-order.txt (dev checkout).
        let p2 = PathBuf::from(&root).join("scripts/build-order.txt");
        if p2.exists() {
            return Some(p2);
        }
    }

    // 2. Relative to CARGO_MANIFEST_DIR (dev / test runs inside packages/jpkg-rs).
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    if !manifest.is_empty() {
        // packages/jpkg-rs → ../../scripts/build-order.txt
        let p = PathBuf::from(&manifest)
            .join("../../scripts/build-order.txt");
        if p.exists() {
            return p.canonicalize().ok();
        }
    }

    // 3. System install.
    {
        let p = PathBuf::from("/usr/share/jpkg/build-order.txt");
        if p.exists() {
            return Some(p);
        }
    }

    // 4. CWD-relative (CI runner inside jonerix repo root).
    {
        let p = PathBuf::from("scripts/build-order.txt");
        if p.exists() {
            return p.canonicalize().ok();
        }
    }

    None
}

/// Parse `build-order.txt`: strip blank lines and `#`-comments, return
/// the remaining package names in order.
fn parse_build_order(path: &Path) -> std::io::Result<Vec<String>> {
    let text = fs::read_to_string(path)?;
    Ok(text
        .lines()
        .map(|l| {
            // Strip inline comments.
            let without_comment = if let Some(pos) = l.find('#') {
                &l[..pos]
            } else {
                l
            };
            without_comment.trim().to_owned()
        })
        .filter(|l| !l.is_empty())
        .collect())
}

// ── recipe discovery ──────────────────────────────────────────────────────────

/// Find `packages/<tier>/<name>/recipe.toml` by walking `packages/*/`.
///
/// The `packages_root` is typically the `packages/` directory next to the
/// repo root.  Returns `None` if no matching recipe is found.
fn find_recipe_for(name: &str, packages_root: &Path) -> Option<PathBuf> {
    let rd = fs::read_dir(packages_root).ok()?;
    for entry in rd.flatten() {
        let tier_path = entry.path();
        if !tier_path.is_dir() {
            continue;
        }
        let candidate = tier_path.join(name).join("recipe.toml");
        if candidate.exists() {
            return Some(tier_path.join(name));
        }
    }
    None
}

// ── packages/ root discovery ──────────────────────────────────────────────────

/// Infer the `packages/` directory from the build-order.txt path or env.
///
/// If `build_order_file` is at `<repo>/scripts/build-order.txt`, then
/// `packages/` is at `<repo>/packages/`.
fn find_packages_root(build_order_file: &Path) -> PathBuf {
    // Try going up from build-order.txt: scripts/../packages.
    if let Some(scripts_dir) = build_order_file.parent() {
        if let Some(repo_root) = scripts_dir.parent() {
            let p = repo_root.join("packages");
            if p.is_dir() {
                return p;
            }
        }
    }

    // JPKG_ROOT env var.
    if let Ok(root) = std::env::var("JPKG_ROOT") {
        let p = PathBuf::from(root).join("packages");
        if p.is_dir() {
            return p;
        }
    }

    // Fallback: relative to CARGO_MANIFEST_DIR.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    if !manifest.is_empty() {
        let p = PathBuf::from(&manifest).join("../../packages");
        if p.is_dir() {
            return p.canonicalize().unwrap_or(p);
        }
    }

    PathBuf::from("packages")
}

// ── argument parsing ──────────────────────────────────────────────────────────

struct BuildWorldOpts {
    output_dir: PathBuf,
    /// Override for build-order.txt path (used by tests).
    order_file: Option<PathBuf>,
    /// Override for packages root (used by tests).
    packages_root: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<BuildWorldOpts, String> {
    let mut output_dir = PathBuf::from("output/packages");
    let mut order_file: Option<PathBuf> = None;
    let mut packages_root: Option<PathBuf> = None;
    let mut i = 0usize;

    while i < args.len() {
        match args[i].as_str() {
            "--output" | "-o" => {
                i += 1;
                if i >= args.len() {
                    return Err("--output requires an argument".to_owned());
                }
                output_dir = PathBuf::from(&args[i]);
            }
            // Hidden flags used by tests / CI to inject paths.
            "--order-file" => {
                i += 1;
                if i >= args.len() {
                    return Err("--order-file requires an argument".to_owned());
                }
                order_file = Some(PathBuf::from(&args[i]));
            }
            "--packages-root" => {
                i += 1;
                if i >= args.len() {
                    return Err("--packages-root requires an argument".to_owned());
                }
                packages_root = Some(PathBuf::from(&args[i]));
            }
            other => {
                return Err(format!("unexpected argument: {other}"));
            }
        }
        i += 1;
    }

    Ok(BuildWorldOpts {
        output_dir,
        order_file,
        packages_root,
    })
}

// ── entry points ──────────────────────────────────────────────────────────────

/// `build_world` with explicit paths — called from tests and CI without going
/// through the arg parser.
pub fn run_with_paths(
    order_file: &Path,
    packages_root: &Path,
    output_dir: &Path,
) -> i32 {
    match build_world_inner(order_file, packages_root, output_dir) {
        Ok(exit_code) => exit_code,
        Err(e) => {
            eprintln!("jpkg build-world: {e}");
            1
        }
    }
}

/// Entry point wired by `cmd/mod.rs`.
pub fn run(args: &[String]) -> i32 {
    let opts = match parse_args(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("jpkg build-world: {e}");
            return 1;
        }
    };

    // Resolve build-order.txt.
    let order_file = match opts.order_file.as_deref().map(|p| p.to_path_buf()).or_else(find_build_order_file) {
        Some(f) => f,
        None => {
            eprintln!("jpkg build-world: cannot find build-order.txt (set JPKG_ROOT or run from repo root)");
            return 1;
        }
    };

    let packages_root = opts
        .packages_root
        .unwrap_or_else(|| find_packages_root(&order_file));

    match build_world_inner(&order_file, &packages_root, &opts.output_dir) {
        Ok(exit_code) => exit_code,
        Err(e) => {
            eprintln!("jpkg build-world: {e}");
            1
        }
    }
}

fn build_world_inner(
    order_file: &Path,
    packages_root: &Path,
    output_dir: &Path,
) -> Result<i32, String> {
    eprintln!(
        "build-world: order={} packages={} output={}",
        order_file.display(),
        packages_root.display(),
        output_dir.display()
    );

    let names = parse_build_order(order_file)
        .map_err(|e| format!("read build-order.txt: {e}"))?;

    if names.is_empty() {
        eprintln!("build-world: build-order.txt has no packages");
        return Ok(0);
    }

    std::fs::create_dir_all(output_dir)
        .map_err(|e| format!("create output dir: {e}"))?;

    let mut built = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for (idx, pkg_name) in names.iter().enumerate() {
        eprintln!(
            "\n=== Building {} ({}/{}) ===",
            pkg_name,
            idx + 1,
            names.len()
        );

        let recipe_dir = match find_recipe_for(pkg_name, packages_root) {
            Some(d) => d,
            None => {
                eprintln!("  WARNING: recipe not found for '{}' — skipping", pkg_name);
                continue;
            }
        };

        let rc = run_build(&recipe_dir, output_dir);
        if rc == 0 {
            built += 1;
        } else {
            eprintln!("  FAILED: {}", pkg_name);
            failures.push(pkg_name.clone());
        }
    }

    // Summary (mirrors C cmd_build_world printf block).
    println!("\n{} built, {} failed", built, failures.len());
    if !failures.is_empty() {
        for f in &failures {
            println!("  - {f}");
        }
    }

    Ok(if failures.is_empty() { 0 } else { 1 })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── helper: write a minimal recipe.toml into packages/<tier>/<name>/ ──────

    fn write_recipe(packages_root: &Path, tier: &str, name: &str) -> PathBuf {
        let dir = packages_root.join(tier).join(name);
        fs::create_dir_all(&dir).unwrap();
        let toml = format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
license = "MIT"
arch = "x86_64"

[build]
system = "custom"
install = "mkdir -p \"$DESTDIR/bin\" && touch \"$DESTDIR/bin/{name}\""
"#
        );
        fs::write(dir.join("recipe.toml"), toml).unwrap();
        dir
    }

    // ── 1. build-order.txt with one good recipe, one missing ─────────────────

    #[test]
    fn test_build_world_summary_good_and_missing() {
        let tmp = TempDir::new().unwrap();

        // Packages tree: one real recipe, one that is NOT in the tree.
        let pkgs = tmp.path().join("packages");
        write_recipe(&pkgs, "core", "alpha");
        // "missing" has no recipe.toml in the tree.

        // build-order.txt lists both.
        let order = tmp.path().join("build-order.txt");
        fs::write(&order, "# tier 0\nalpha\nmissing\n").unwrap();

        let out = tmp.path().join("out");

        let rc = run_with_paths(&order, &pkgs, &out);
        // "alpha" should build OK; "missing" is skipped (no recipe found).
        // "missing" is skipped (not counted as failure), so exit 0.
        assert_eq!(rc, 0, "missing recipe should be warned and skipped, not fail");

        // At least one artifact for "alpha" must exist.
        let artifacts: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x == "jpkg")
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(artifacts.len(), 1, "expected 1 artifact for 'alpha'");
    }

    // ── 2. build-order.txt with two buildable recipes ─────────────────────────

    #[test]
    fn test_build_world_two_good_recipes() {
        let tmp = TempDir::new().unwrap();

        let pkgs = tmp.path().join("packages");
        write_recipe(&pkgs, "core", "pkg-a");
        write_recipe(&pkgs, "core", "pkg-b");

        let order = tmp.path().join("build-order.txt");
        fs::write(&order, "pkg-a\npkg-b\n").unwrap();

        let out = tmp.path().join("out");
        let rc = run_with_paths(&order, &pkgs, &out);
        assert_eq!(rc, 0);

        let count = fs::read_dir(&out)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x == "jpkg")
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(count, 2, "expected 2 artifacts");
    }

    // ── 3. Comments and blank lines are stripped ──────────────────────────────

    #[test]
    fn test_parse_build_order_strips_comments_and_blanks() {
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("order.txt");
        fs::write(&f, "# header\n\npkg-x  # inline comment\npkg-y\n").unwrap();

        let names = parse_build_order(&f).unwrap();
        assert_eq!(names, vec!["pkg-x", "pkg-y"]);
    }

    // ── 4. parse_args honours --output ───────────────────────────────────────

    #[test]
    fn test_parse_args_output() {
        let args: Vec<String> = vec!["--output".to_owned(), "/tmp/myout".to_owned()];
        let opts = parse_args(&args).unwrap();
        assert_eq!(opts.output_dir, PathBuf::from("/tmp/myout"));
    }
}
