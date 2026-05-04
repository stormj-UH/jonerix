//! `jpkg build` — build a package from a recipe.
//!
//! Ported from `packages/jpkg/src/cmd_build.c` (`cmd_build`).
//!
//! # Divergences from C semantics
//!
//! * **--build-jpkg is the default.**  The C binary distinguishes "produce an
//!   artifact" (--build-jpkg) from "install directly to the running system"
//!   (bare).  Phase-3 only ports the artifact path; direct-install is the
//!   integrator's job.  The flag is accepted for parity but is a no-op.
//!
//! * **Source-tarball extraction.**  We shell out to `tar xf` (matching the C
//!   cmd_build.c approach) rather than pulling in extra Rust crates for every
//!   compression format.  The same toybox→bsdtar→tar fallback order is used.
//!
//! * **Metadata sha256/size chicken-and-egg.**  `archive::create` builds and
//!   compresses the payload internally; we cannot know the zstd payload bytes
//!   before the archive is written.  We therefore compute the sha256 of the
//!   *final .jpkg file* as a proxy and set size to the .jpkg file size.  This
//!   differs from the C code, which computes the sha256 of the raw zstd payload
//!   blob before embedding it.  A future Worker-C integration that exposes the
//!   compressed payload bytes separately can replace this with the exact value.
//!   FIXME(integrator): once archive::create can return compressed-payload bytes,
//!   replace jpkg-file sha256/size with payload sha256/payload size.
//!
//! * **flatten_merged_usr.**  Worker L has not yet delivered `cmd::common::
//!   flatten_merged_usr`; an inline equivalent is provided below.
//!   FIXME(integrator): replace `inline_flatten_merged_usr` with
//!   `crate::cmd::common::flatten_merged_usr` once Worker L delivers it.
//!
//! * **JPKG_SOURCE_CACHE** is supported for offline package builds.  The cache
//!   lookup accepts exact URL basenames plus package-version basenames with or
//!   without a packaging release suffix (`-rN`).

use std::env;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use crate::archive;
use crate::recipe::{Metadata, Recipe};
use crate::util::{audit_layout_tree, sha256_file, AuditResult};

// ── inline flatten (Worker L placeholder) ────────────────────────────────────

/// Flatten `destdir/usr/* → destdir/*` and `destdir/lib64/* → destdir/lib/*`.
///
/// Mirrors the C `main_local.c` flatten shells that most `create_package`
/// implementations run before audit.
///
/// FIXME(integrator): replace with `crate::cmd::common::flatten_merged_usr`
/// once Worker L delivers it.
pub fn inline_flatten_merged_usr(destdir: &Path) -> io::Result<()> {
    // usr/ → ./
    let usr = destdir.join("usr");
    if usr.exists() && !usr.symlink_metadata()?.file_type().is_symlink() {
        copy_dir_contents(&usr, destdir)?;
        fs::remove_dir_all(&usr)?;
    }

    // lib64/ → lib/
    let lib64 = destdir.join("lib64");
    if lib64.exists() && !lib64.symlink_metadata()?.file_type().is_symlink() {
        let lib = destdir.join("lib");
        fs::create_dir_all(&lib)?;
        copy_dir_contents(&lib64, &lib)?;
        fs::remove_dir_all(&lib64)?;
    }

    Ok(())
}

/// Recursively copy contents of `src` into `dst` (cp -a src/. dst/).
fn copy_dir_contents(src: &Path, dst: &Path) -> io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let src_path = entry.path();
        let dst_path = dst.join(&name);

        let ft = entry.file_type()?;
        if ft.is_symlink() {
            let target = fs::read_link(&src_path)?;
            // Remove existing destination if present.
            if dst_path.symlink_metadata().is_ok() {
                let _ = fs::remove_file(&dst_path);
            }
            std::os::unix::fs::symlink(&target, &dst_path)?;
        } else if ft.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_dir_contents(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

// ── argument parsing ──────────────────────────────────────────────────────────

struct BuildOpts {
    /// Path to the recipe directory or recipe.toml file (or "-" for stdin).
    recipe_arg: String,
    /// Directory into which to write the .jpkg artifact.
    output_dir: PathBuf,
    /// Path to the Ed25519 secret key for signing the built package.
    ///
    /// Phase 0 stub: stored but not used for signing.  Worker B will implement
    /// the actual signing logic.  When set, a notice is printed and the build
    /// proceeds unsigned.
    sign_key: Option<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<BuildOpts, String> {
    if args.is_empty() {
        return Err(
            "usage: jpkg build <recipe-dir|recipe.toml|-> [--output <dir>] [--build-jpkg] [--sign-key <path>]"
                .to_owned(),
        );
    }

    let mut recipe_arg: Option<String> = None;
    let mut output_dir = PathBuf::from(".");
    let mut sign_key: Option<PathBuf> = None;
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
            "--build-jpkg" => {
                // accepted, no-op (see divergence note above)
            }
            "--sign-key" => {
                // Phase 0 stub: accept the flag and store the path.
                // Worker B will implement the actual signing; for now we
                // store the path so Worker D can wire CI plumbing in parallel.
                i += 1;
                if i >= args.len() {
                    return Err("--sign-key requires an argument".to_owned());
                }
                sign_key = Some(PathBuf::from(&args[i]));
            }
            other => {
                if recipe_arg.is_some() {
                    return Err(format!("unexpected argument: {other}"));
                }
                recipe_arg = Some(other.to_owned());
            }
        }
        i += 1;
    }

    Ok(BuildOpts {
        recipe_arg: recipe_arg.ok_or_else(|| "missing <recipe> argument".to_owned())?,
        output_dir,
        sign_key,
    })
}

// ── recipe loading ────────────────────────────────────────────────────────────

/// Load a `Recipe` from a path (dir or direct .toml), a URL, or "-" (stdin).
/// Returns (recipe, recipe_dir) where recipe_dir is used for RECIPE_DIR env.
fn load_recipe_from_arg(
    arg: &str,
) -> Result<(Recipe, PathBuf), String> {
    if arg == "-" {
        let mut s = String::new();
        io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| format!("reading stdin: {e}"))?;
        let recipe = Recipe::from_str(&s).map_err(|e| format!("parse recipe: {e}"))?;
        return Ok((recipe, PathBuf::from(".")));
    }

    if arg.starts_with("https://") || arg.starts_with("http://") {
        let bytes = crate::fetch::download(arg)
            .map_err(|e| format!("download recipe URL: {e}"))?;
        let s = std::str::from_utf8(&bytes)
            .map_err(|e| format!("recipe UTF-8: {e}"))?;
        let recipe = Recipe::from_str(s).map_err(|e| format!("parse recipe: {e}"))?;
        return Ok((recipe, PathBuf::from(".")));
    }

    let arg_path = Path::new(arg);

    // Resolve: directory containing recipe.toml, or a direct .toml file.
    let (toml_path, recipe_dir) = if arg_path.is_dir() {
        let t = arg_path.join("recipe.toml");
        (
            t,
            arg_path
                .canonicalize()
                .unwrap_or_else(|_| arg_path.to_path_buf()),
        )
    } else {
        let dir = arg_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let dir = dir.canonicalize().unwrap_or(dir);
        (arg_path.to_path_buf(), dir)
    };

    if !toml_path.exists() {
        return Err(format!("recipe not found: {}", toml_path.display()));
    }

    let recipe = Recipe::from_file(&toml_path).map_err(|e| format!("parse recipe: {e}"))?;
    Ok((recipe, recipe_dir))
}

// ── source fetching ───────────────────────────────────────────────────────────

/// Download (or skip) the source tarball; verify sha256 if provided.
/// Returns the path to the downloaded tarball, or None if no source.
fn fetch_source(
    recipe: &Recipe,
    src_dir: &Path,
    name: &str,
    version: &str,
) -> Result<Option<PathBuf>, String> {
    let url = match recipe.source.url.as_deref() {
        None | Some("") | Some("local") => return Ok(None),
        Some(u) => u,
    };

    let basename = url.rsplit('/').next().unwrap_or(url);
    let tarball = src_dir.join(basename);

    // Try JPKG_SOURCE_CACHE first (colon-separated dirs).
    let cached = try_source_cache(url, name, version, &tarball);
    if !cached {
        eprintln!("  downloading source: {url}");
        // Shell out to curl with same flags as C cmd_build.c.
        let status = Command::new("curl")
            .args([
                "-fsSL",
                "--retry", "3",
                "--retry-delay", "2",
                "--retry-connrefused",
                "--retry-all-errors",
                "-o",
            ])
            .arg(&tarball)
            .arg(url)
            .status()
            .map_err(|e| format!("curl exec: {e}"))?;
        if !status.success() {
            return Err("failed to download source".to_owned());
        }
    }

    // Verify sha256 if provided.
    if let Some(expected) = recipe.source.sha256.as_deref() {
        if !expected.is_empty() {
            let actual = sha256_file(&tarball)
                .map_err(|e| format!("hash tarball: {e}"))?;
            if actual != expected {
                return Err(format!(
                    "source hash mismatch:\n  expected: {expected}\n  actual:   {actual}"
                ));
            }
            eprintln!("  source hash verified");
        }
    }

    Ok(Some(tarball))
}

/// Attempt to copy a cached tarball instead of downloading.
/// Returns true if a cached copy was successfully placed at `dest`.
fn try_source_cache(url: &str, name: &str, version: &str, dest: &Path) -> bool {
    let cache = match env::var("JPKG_SOURCE_CACHE") {
        Ok(v) if !v.is_empty() => v,
        _ => return false,
    };

    let url_base = url.rsplit('/').next().unwrap_or(url);
    let prefix = format!("{name}-{version}.");
    let base_version = strip_release_suffix(version);
    let base_prefix = if base_version == version {
        None
    } else {
        Some(format!("{name}-{base_version}."))
    };

    for dir in cache.split(':') {
        // Exact basename match.
        let exact = Path::new(dir).join(url_base);
        if exact.exists() {
            if fs::copy(&exact, dest).is_ok() {
                eprintln!("  source from cache: {}", exact.display());
                return true;
            }
        }

        // Fuzzy: name-version.* or name-upstream_version.* in dir.
        if let Ok(rd) = fs::read_dir(dir) {
            for entry in rd.flatten() {
                let fname = entry.file_name();
                let fname = fname.to_string_lossy();
                if fname.starts_with(&prefix)
                    || base_prefix
                        .as_deref()
                        .is_some_and(|p| fname.starts_with(p))
                {
                    let src = entry.path();
                    if fs::copy(&src, dest).is_ok() {
                        eprintln!("  source from cache: {}", src.display());
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn strip_release_suffix(version: &str) -> &str {
    let Some((base, suffix)) = version.rsplit_once("-r") else {
        return version;
    };
    if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
        base
    } else {
        version
    }
}

/// Extract `tarball` into `src_dir`.  Uses toybox→bsdtar→tar fallback
/// (same as C cmd_build.c:421-438), NOT --strip-components (C doesn't either).
fn extract_source(tarball: &Path, src_dir: &Path) -> Result<(), String> {
    let tar_bin: &str = if Path::new("/bin/toybox").exists() {
        "/bin/toybox tar"
    } else if Path::new("/root/jonerix/tools/bsdtar-static-aarch64").exists() {
        "/root/jonerix/tools/bsdtar-static-aarch64"
    } else {
        "tar"
    };

    // Split the tar_bin string for Command.
    let mut parts = tar_bin.split_whitespace();
    let prog = parts.next().unwrap();
    let mut cmd = Command::new(prog);
    for part in parts {
        cmd.arg(part);
    }
    cmd.args(["xf"]).arg(tarball).current_dir(src_dir);

    let status = cmd.status().map_err(|e| format!("extract exec: {e}"))?;
    if !status.success() {
        return Err("failed to extract source tarball".to_owned());
    }
    Ok(())
}

/// After extraction, if `src_dir` has exactly one sub-directory (the typical
/// tarball layout), return that sub-directory as the actual source dir.
/// Mirrors C cmd_build.c lines 864-890.
fn find_source_dir(src_dir: &Path) -> PathBuf {
    let Ok(rd) = fs::read_dir(src_dir) else {
        return src_dir.to_path_buf();
    };
    let mut only: Option<PathBuf> = None;
    let mut count = 0usize;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            count += 1;
            only = Some(p);
        }
    }
    if count == 1 {
        only.unwrap()
    } else {
        src_dir.to_path_buf()
    }
}

// ── patches ───────────────────────────────────────────────────────────────────

fn apply_patches(recipe_dir: &Path, src_dir: &Path) -> Result<(), String> {
    let patches_dir = recipe_dir.join("patches");
    if !patches_dir.is_dir() {
        return Ok(());
    }

    let mut patches: Vec<PathBuf> = fs::read_dir(&patches_dir)
        .map_err(|e| format!("read patches dir: {e}"))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            matches!(ext, "patch" | "diff")
        })
        .collect();
    patches.sort();

    for patch in patches {
        eprintln!("  applying patch: {}", patch.file_name().unwrap_or_default().to_string_lossy());
        let status = Command::new("patch")
            .args(["-p1"])
            .stdin(fs::File::open(&patch).map_err(|e| format!("open patch: {e}"))?)
            .current_dir(src_dir)
            .status()
            .map_err(|e| format!("patch exec: {e}"))?;
        if !status.success() {
            return Err(format!(
                "failed to apply patch: {}",
                patch.display()
            ));
        }
    }
    Ok(())
}

// ── build step runner ─────────────────────────────────────────────────────────

/// Run one shell snippet (configure / build / install) via a temp script,
/// matching the approach in `run_build_step` from cmd_build.c (lines 209-305).
///
/// Key behaviours preserved from C:
/// - Writes a `/tmp/jpkg-build-<pid>.sh` script to avoid quoting issues.
/// - Pre-expands `$(nproc)` and `\`nproc\`` to avoid toybox sh deadlocks.
/// - Exports CC/LD/AR/NM/RANLIB, CFLAGS, LDFLAGS, DESTDIR, C_INCLUDE_PATH,
///   LIBRARY_PATH, RECIPE_DIR, NPROC.
/// - POSIX `/bin/sh` (no bashisms), no `set -e` (each step checked separately).
fn run_build_step(
    step_name: &str,
    cmd: &str,
    work_dir: &Path,
    dest_dir: &Path,
    recipe_dir: &Path,
) -> Result<(), String> {
    if cmd.trim().is_empty() {
        return Ok(());
    }

    eprintln!("  {step_name}: {cmd}");

    let ncpu = num_cpus();
    let ncpu_str = ncpu.to_string();

    // Pre-expand $(nproc) / `nproc` to avoid toybox sh command-substitution deadlock.
    let expanded = cmd
        .replace("$(nproc)", &ncpu_str)
        .replace("`nproc`", &ncpu_str);

    let script = format!(
        "#!/bin/sh\n\
         export NPROC={ncpu}\n\
         nproc() {{ printf '%s\\n' {ncpu}; }}\n\
         cd '{work}'\n\
         export CC=clang\n\
         export LD=ld.lld\n\
         export AR=llvm-ar\n\
         export NM=llvm-nm\n\
         export RANLIB=llvm-ranlib\n\
         export CFLAGS='-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2 --rtlib=compiler-rt --unwindlib=libunwind'\n\
         export LDFLAGS='-Wl,-z,relro,-z,now -pie --rtlib=compiler-rt --unwindlib=libunwind -fuse-ld=lld'\n\
         export DESTDIR='{dest}'\n\
         export C_INCLUDE_PATH=/include\n\
         export LIBRARY_PATH=/lib\n\
         export RECIPE_DIR='{recipe}'\n\
         {body}\n",
        ncpu = ncpu,
        work = work_dir.display(),
        dest = dest_dir.display(),
        recipe = recipe_dir.display(),
        body = expanded,
    );

    let script_path = std::env::temp_dir().join(format!("jpkg-build-{}.sh", std::process::id()));
    fs::write(&script_path, script.as_bytes())
        .map_err(|e| format!("write build script: {e}"))?;

    // chmod +x
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path)
            .map_err(|e| format!("stat script: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)
            .map_err(|e| format!("chmod script: {e}"))?;
    }

    let status = Command::new("/bin/sh")
        .arg(&script_path)
        .status()
        .map_err(|e| format!("{step_name} exec: {e}"))?;

    let _ = fs::remove_file(&script_path);

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(format!("{step_name} step failed (exit {code})"));
    }
    Ok(())
}

/// Return the online CPU count (or 1 on error).
///
/// Uses `std::thread::available_parallelism` (stable, cross-platform) which
/// wraps `sysconf(_SC_NPROCESSORS_ONLN)` on Linux and the equivalent on macOS.
fn num_cpus() -> u64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(1)
}

// ── archive creation ──────────────────────────────────────────────────────────

fn create_archive(
    recipe: &Recipe,
    dest_dir: &Path,
    output_dir: &Path,
) -> Result<PathBuf, String> {
    let name = recipe
        .package
        .name
        .as_deref()
        .unwrap_or("unknown");
    let version = recipe
        .package
        .version
        .as_deref()
        .unwrap_or("0.0.0");
    // Resolve arch: use recipe field, fall back to uname().machine.
    let arch_owned: String = recipe.package.arch.clone().unwrap_or_else(|| {
        nix::sys::utsname::uname()
            .map(|u| u.machine().to_string_lossy().into_owned())
            .unwrap_or_else(|_| "x86_64".to_owned())
    });
    let arch = arch_owned.as_str();

    fs::create_dir_all(output_dir)
        .map_err(|e| format!("create output dir: {e}"))?;

    let out_path = output_dir.join(format!("{name}-{version}-{arch}.jpkg"));

    // archive::create_with_metadata_factory handles the chicken-and-egg
    // (metadata.files.sha256/size depend on the compressed payload, which
    // is built inside the archive call).  The factory closure runs AFTER
    // the payload is compressed, so the sha256 + size we pass to
    // Metadata::from_recipe are real values, not placeholders.
    //
    // We also inject the RESOLVED arch into the recipe-clone before
    // building Metadata.  Recipes commonly omit `arch` (it's a build-time
    // resolution from `uname -m`), but the EMITTED metadata.toml MUST
    // include arch so downstream tooling like scripts/gen-index.sh can
    // distinguish jpkg-2.0.0-x86_64.jpkg from jpkg-2.0.0-aarch64.jpkg
    // without falling back to the default "x86_64" — which would group
    // both arches into the same (name, arch) bucket and let dedup discard
    // the wrong file.  Reproduced 2026-04-27 publish-packages run
    // 24977249193: gen-index.sh ran `discarding: jpkg-2.0.0-aarch64.jpkg`
    // because that file's metadata had no arch field.
    let mut recipe_for_factory = recipe.clone();
    if recipe_for_factory.package.arch.is_none() {
        recipe_for_factory.package.arch = Some(arch_owned.clone());
    }
    archive::create_with_metadata_factory(&out_path, dest_dir, move |sha, size| {
        let meta = Metadata::from_recipe(&recipe_for_factory, sha.to_owned(), size);
        meta.to_string().map_err(|e| {
            crate::archive::ArchiveError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("serialize metadata: {e}"),
            ))
        })
    })
    .map_err(|e| format!("archive::create: {e}"))?;

    Ok(out_path)
}

// ── public API ────────────────────────────────────────────────────────────────

/// Callable from `build_world` without going through arg parsing.
///
/// `recipe_path` is a path to the recipe directory or recipe.toml.
/// `output_dir` is where the .jpkg artifact is written.
pub fn run_build(recipe_path: &Path, output_dir: &Path) -> i32 {
    // Convert to arg-style string for the shared loader.
    let arg = recipe_path.to_string_lossy().into_owned();
    let args = vec![
        arg,
        "--output".to_owned(),
        output_dir.to_string_lossy().into_owned(),
    ];
    run(&args)
}

/// Entry point wired by `cmd/mod.rs`.
pub fn run(args: &[String]) -> i32 {
    let opts = match parse_args(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("jpkg build: {e}");
            return 1;
        }
    };

    match build_inner(&opts) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("jpkg build: error: {e}");
            1
        }
    }
}

fn build_inner(opts: &BuildOpts) -> Result<(), String> {
    // ── 1. Resolve recipe ────────────────────────────────────────────────────
    let (recipe, recipe_dir) = load_recipe_from_arg(&opts.recipe_arg)?;

    // ── 2. Validate ─────────────────────────────────────────────────────────
    recipe.validate().map_err(|e| format!("recipe validation: {e}"))?;

    let name = recipe.package.name.as_deref().unwrap_or("unknown");
    let version = recipe.package.version.as_deref().unwrap_or("0.0.0");
    eprintln!("building {name}-{version}...");

    // ── 3. Set up temp build dirs ────────────────────────────────────────────
    let tmp = TempDir::new().map_err(|e| format!("tempdir: {e}"))?;
    let src_dir = tmp.path().join("src");
    let dest_dir = tmp.path().join("dest");
    fs::create_dir_all(&src_dir).map_err(|e| format!("mkdir src: {e}"))?;
    fs::create_dir_all(&dest_dir).map_err(|e| format!("mkdir dest: {e}"))?;

    // ── 4. Fetch source ──────────────────────────────────────────────────────
    let tarball_opt = fetch_source(&recipe, &src_dir, name, version)?;

    // ── 5. Extract source (if we have a tarball) ─────────────────────────────
    let build_dir = if let Some(ref tarball) = tarball_opt {
        extract_source(tarball, &src_dir)?;
        find_source_dir(&src_dir)
    } else {
        src_dir.clone()
    };

    // ── 5b. Apply patches ────────────────────────────────────────────────────
    apply_patches(&recipe_dir, &build_dir)?;

    // ── 6. Run shell snippets ────────────────────────────────────────────────
    if let Some(ref configure) = recipe.build.configure {
        run_build_step("configure", configure, &build_dir, &dest_dir, &recipe_dir)?;
    }
    if let Some(ref build_cmd) = recipe.build.build {
        run_build_step("build", build_cmd, &build_dir, &dest_dir, &recipe_dir)?;
    }
    if let Some(ref install_cmd) = recipe.build.install {
        run_build_step("install", install_cmd, &build_dir, &dest_dir, &recipe_dir)?;
    }

    // ── 7. Audit the DESTDIR; flatten if needed ──────────────────────────────
    // The C cmd_build.c (post-2026 hardened version) makes usr/lib64/sbin hard
    // errors.  We mirror main_local.c which still auto-flattens, then re-audits.
    match audit_layout_tree(&dest_dir) {
        Ok(()) => {}
        Err(AuditResult::Lib64Path(_)) => {
            // FIXME(integrator): use crate::cmd::common::flatten_merged_usr
            inline_flatten_merged_usr(&dest_dir)
                .map_err(|e| format!("flatten: {e}"))?;
            audit_layout_tree(&dest_dir).map_err(|r| {
                format!("layout audit still failing after flatten: {}", r.description())
            })?;
        }
        Err(other) => {
            return Err(format!(
                "refusing to package {name}: {}",
                other.description()
            ));
        }
    }

    // ── 8. Build the archive ─────────────────────────────────────────────────
    let out_path = create_archive(&recipe, &dest_dir, &opts.output_dir)?;

    // ── 9. Sign the archive (optional) ──────────────────────────────────────
    if let Some(ref key_path) = opts.sign_key {
        let sk = crate::sign::read_secret_key(key_path)
            .map_err(|e| format!("--sign-key: failed to load {}: {e}", key_path.display()))?;
        // Derive key_id from the key file's stem (e.g. "jonerix-2026" from "jonerix-2026.sec").
        let key_id = key_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());
        crate::cmd::sign::sign_jpkg_in_place(&out_path, &sk, &key_id)
            .map_err(|e| format!("signing: {e}"))?;
        eprintln!("  signed with key_id={key_id}");
    }

    // ── 10. Report ───────────────────────────────────────────────────────────
    let file_size = fs::metadata(&out_path)
        .map(|m| m.len())
        .unwrap_or(0);

    println!(
        "Built {} ({} bytes)",
        out_path.file_name().unwrap_or_default().to_string_lossy(),
        file_size
    );

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use tempfile::TempDir;

    // ── helpers ───────────────────────────────────────────────────────────────

    /// Write a minimal recipe.toml into `dir` and return its path.
    fn write_recipe(dir: &Path, extra: &str) -> PathBuf {
        let toml = format!(
            r#"[package]
name = "testpkg"
version = "0.1.0"
license = "MIT"
description = "test package"
arch = "x86_64"

[build]
{extra}
"#
        );
        let p = dir.join("recipe.toml");
        fs::write(&p, toml).unwrap();
        p
    }

    #[test]
    fn test_strip_release_suffix() {
        assert_eq!(strip_release_suffix("1.2.3-r4"), "1.2.3");
        assert_eq!(strip_release_suffix("1.2.3"), "1.2.3");
        assert_eq!(strip_release_suffix("1.2.3-rx"), "1.2.3-rx");
        assert_eq!(strip_release_suffix("1.2.3-r"), "1.2.3-r");
    }

    #[test]
    fn test_source_cache_matches_release_stripped_name() {
        let tmp = TempDir::new().unwrap();
        let cache = tmp.path().join("cache");
        let dest = tmp.path().join("source").join("upstream.tar.gz");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::write(cache.join("pkg-1.2.3.tar.gz"), b"vendored").unwrap();

        std::env::set_var("JPKG_SOURCE_CACHE", cache.as_os_str());
        assert!(try_source_cache(
            "https://example.invalid/upstream.tar.gz",
            "pkg",
            "1.2.3-r4",
            &dest
        ));
        std::env::remove_var("JPKG_SOURCE_CACHE");

        assert_eq!(fs::read(&dest).unwrap(), b"vendored");
    }

    // ── 1. Recipe loading + validation ───────────────────────────────────────

    #[test]
    fn test_recipe_load_and_validate() {
        let tmp = TempDir::new().unwrap();
        write_recipe(tmp.path(), "");
        let args = vec![
            tmp.path().to_string_lossy().into_owned(),
            "--output".to_owned(),
            tmp.path().join("out").to_string_lossy().into_owned(),
        ];
        // We only exercise loading+validation; the build will fail (no source,
        // no install step) — that is expected and tested via exit code.
        let (recipe, _) = load_recipe_from_arg(&args[0]).expect("load recipe");
        recipe.validate().expect("validate");
        assert_eq!(recipe.package.name.as_deref(), Some("testpkg"));
    }

    // ── 2. Layout audit + flatten ─────────────────────────────────────────────

    #[test]
    fn test_flatten_and_audit() {
        let tmp = TempDir::new().unwrap();
        let destdir = tmp.path();

        // Create usr/bin/foo (the bad layout).
        fs::create_dir_all(destdir.join("usr/bin")).unwrap();
        fs::write(destdir.join("usr/bin/foo"), b"#!/bin/sh\necho hi\n").unwrap();

        // Flatten.
        inline_flatten_merged_usr(destdir).expect("flatten");

        // usr/ should be gone; bin/foo should be present.
        assert!(!destdir.join("usr").exists(), "usr/ should have been removed");
        assert!(destdir.join("bin/foo").exists(), "bin/foo should exist after flatten");

        // Audit should now pass.
        audit_layout_tree(destdir).expect("audit should pass after flatten");
    }

    // ── 3. Custom build + inline install ─────────────────────────────────────

    #[test]
    fn test_build_with_inline_install() {
        let tmp = TempDir::new().unwrap();
        let out_dir = tmp.path().join("out");

        // Recipe with system = "custom", no configure/build, install touches a file.
        let toml = r#"[package]
name = "custompkg"
version = "0.1.0"
license = "MIT"
arch = "x86_64"

[build]
system = "custom"
build = "true"
install = "mkdir -p \"$DESTDIR/bin\" && touch \"$DESTDIR/bin/x\""
"#;
        let recipe_dir = tmp.path().join("recipe");
        fs::create_dir_all(&recipe_dir).unwrap();
        fs::write(recipe_dir.join("recipe.toml"), toml).unwrap();

        let args = vec![
            recipe_dir.to_string_lossy().into_owned(),
            "--output".to_owned(),
            out_dir.to_string_lossy().into_owned(),
        ];
        let rc = run(&args);
        assert_eq!(rc, 0, "build with inline install should succeed");

        // A .jpkg artifact should have been created.
        let artifacts: Vec<_> = fs::read_dir(&out_dir)
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
        assert_eq!(artifacts.len(), 1, "expected exactly one .jpkg artifact");
    }

    // ── 4. Source fetch sha256 mismatch → exit 1 ─────────────────────────────

    #[test]
    fn test_sha256_mismatch_returns_1() {
        // Spin up a one-shot HTTP server that serves a tiny "tarball".
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();

        // The "tarball" is just some bytes (not a real tar; the sha256 check
        // happens before extraction, so that's fine for this test).
        let body = b"fake tarball content";
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let resp = format!(
                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(body);
            }
        });

        let tmp = TempDir::new().unwrap();
        let out_dir = tmp.path().join("out");
        let url = format!("http://127.0.0.1:{port}/pkg.tar.gz");

        let toml = format!(
            r#"[package]
name = "hashtest"
version = "1.0.0"
license = "MIT"
arch = "x86_64"

[source]
url = "{url}"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"

[build]
install = "true"
"#
        );
        let recipe_dir = tmp.path().join("recipe");
        fs::create_dir_all(&recipe_dir).unwrap();
        fs::write(recipe_dir.join("recipe.toml"), &toml).unwrap();

        let args = vec![
            recipe_dir.to_string_lossy().into_owned(),
            "--output".to_owned(),
            out_dir.to_string_lossy().into_owned(),
        ];
        let rc = run(&args);
        assert_eq!(rc, 1, "sha256 mismatch must return exit 1");
    }

    // ── 5. Archive emission + magic byte verification ────────────────────────

    #[test]
    fn test_archive_emission_magic_and_metadata() {
        use crate::archive::JpkgArchive;
        use crate::recipe::Metadata;

        let tmp = TempDir::new().unwrap();
        let out_dir = tmp.path().join("out");

        let toml = r#"[package]
name = "magictest"
version = "0.2.0"
license = "ISC"
arch = "x86_64"

[build]
system = "custom"
install = "mkdir -p \"$DESTDIR/bin\" && printf '#!/bin/sh\\necho ok\\n' > \"$DESTDIR/bin/magictest\""
"#;
        let recipe_dir = tmp.path().join("recipe");
        fs::create_dir_all(&recipe_dir).unwrap();
        fs::write(recipe_dir.join("recipe.toml"), toml).unwrap();

        let args = vec![
            recipe_dir.to_string_lossy().into_owned(),
            "--output".to_owned(),
            out_dir.to_string_lossy().into_owned(),
        ];
        assert_eq!(run(&args), 0, "build should succeed");

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

        // Open and verify magic + metadata round-trip.
        let arch = JpkgArchive::open(&artifact).expect("JpkgArchive::open failed");
        let meta_str = arch.metadata_str().expect("metadata not valid UTF-8");
        let meta = Metadata::from_str(meta_str).expect("parse embedded metadata");
        assert_eq!(meta.package.name.as_deref(), Some("magictest"));
        assert_eq!(meta.package.version.as_deref(), Some("0.2.0"));
        assert_eq!(meta.package.license.as_deref(), Some("ISC"));
    }
}
