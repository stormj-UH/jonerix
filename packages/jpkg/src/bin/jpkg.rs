//! `jpkg` — the main jpkg binary, routing subcommands to `jpkg::cmd::*`.
//!
//! Mirrors `packages/jpkg/src/main.c` exit-code and dispatch semantics:
//! - `update`, `install`/`add`, `remove`/`del`, `upgrade`, `search`, `info`,
//!   `verify`, `license-audit`, `keygen`, `sign`, `build`, `build-world` are
//!   built-in.
//! - Unknown subcommands fall through to a PATH lookup of `jpkg-<verb>` and
//!   `execvp`, so external sub-commands (`jpkg-conform`, `jpkg-local`, etc.)
//!   continue to work transparently.
//! - Global options consumed BEFORE the verb: `-v/--verbose`, `-q/--quiet`,
//!   `-r/--root <path>`, `-V/--version`, `-h/--help`.

use std::process::ExitCode;

const USAGE: &str = "\
jpkg — jonerix package manager

USAGE:
    jpkg [GLOBAL_OPTS] <COMMAND> [ARGS...]

GLOBAL OPTIONS:
    -v, --verbose         Set log level to DEBUG
    -q, --quiet           Set log level to ERROR
    -r, --root <path>     Operate on an alternate rootfs
    -V, --version         Print version and exit
    -h, --help            Print this help and exit

BUILT-IN COMMANDS:
    update                Fetch the package index from configured mirrors
    install <pkg>...      (alias: add)    Install one or more packages
    remove  <pkg>...      (alias: del)    Remove one or more packages
    upgrade               Upgrade all installed packages
    search  <query>       Search the index by name and description
    info    <pkg>         Show package metadata
    verify  [<pkg>...]    Verify installed files against their manifests
    license-audit         Show installed-package licenses
    keygen  [<dir>]       Generate an Ed25519 keypair (default: /etc/jpkg/keys)
    sign    <key> <file>  Sign <file> with Ed25519 secret key <key>
    build   <recipe>      Build a recipe.toml into a .jpkg
    build-world           Rebuild every package in scripts/build-order.txt

EXTERNAL COMMANDS:
    Unknown commands are dispatched as `jpkg-<verb>` from $PATH (e.g. the
    `jpkg-local` and `jpkg-conform` binaries that ship with this package).
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mut idx = 1;
    let mut root: Option<String> = None;
    let mut log_filter: Option<&'static str> = None;

    // ── Global option parsing ───────────────────────────────────────────────
    while idx < argv.len() {
        match argv[idx].as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("jpkg {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-v" | "--verbose" => {
                log_filter = Some("debug");
                idx += 1;
            }
            "-q" | "--quiet" => {
                log_filter = Some("error");
                idx += 1;
            }
            "-r" | "--root" => {
                if idx + 1 >= argv.len() {
                    eprintln!("jpkg: --root requires a path argument");
                    return ExitCode::from(2);
                }
                root = Some(argv[idx + 1].clone());
                idx += 2;
            }
            // First non-option token is the verb.
            s if !s.starts_with('-') => break,
            other => {
                eprintln!("jpkg: unknown global option: {other}");
                return ExitCode::from(2);
            }
        }
    }

    // ── Initialise logging ──────────────────────────────────────────────────
    let mut env_builder = env_logger::Builder::new();
    if let Some(lvl) = log_filter {
        env_builder.parse_filters(lvl);
    } else if let Ok(env) = std::env::var("JPKG_LOG") {
        env_builder.parse_filters(&env);
    } else {
        env_builder.parse_filters("info");
    }
    let _ = env_builder.try_init();

    // ── Verb extraction ─────────────────────────────────────────────────────
    if idx >= argv.len() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let verb = &argv[idx];
    let verb_args: Vec<String> = argv[idx + 1..].to_vec();

    // The `--root` global is propagated to subcommand modules via env so each
    // module can `std::env::var("JPKG_ROOT")` without threading state.
    if let Some(r) = &root {
        std::env::set_var("JPKG_ROOT", r);
    }

    use jpkg::cmd;
    let rc: i32 = match verb.as_str() {
        "update" => cmd::update::run(&verb_args),
        "install" | "add" => cmd::install::run(&verb_args),
        "remove" | "del" => cmd::remove::run(&verb_args),
        "upgrade" => cmd::upgrade::run(&verb_args),
        "search" => cmd::search::run(&verb_args),
        "info" => cmd::info::run(&verb_args),
        "verify" => cmd::verify::run(&verb_args),
        "license-audit" => cmd::license_audit::run(&verb_args),
        "keygen" => cmd::keygen::run(&verb_args),
        "sign" => cmd::sign::run(&verb_args),
        "build" => cmd::build::run(&verb_args),
        "build-world" => cmd::build_world::run(&verb_args),
        other => {
            // External subcommand: search PATH for `jpkg-<verb>` and exec it.
            return external_subcommand(other, &verb_args);
        }
    };

    ExitCode::from(rc.clamp(0, 255) as u8)
}

/// Search PATH for `jpkg-<name>` and `execvp` it with the remaining args.
/// Mirrors `packages/jpkg/src/main.c:146-168`.
fn external_subcommand(name: &str, rest: &[String]) -> ExitCode {
    use std::os::unix::process::CommandExt;

    let target = format!("jpkg-{name}");

    // Prefer canonical paths first (matches the C lookup order).
    let canonical_dirs = ["/bin", "/usr/bin", "/usr/local/bin", "/sbin", "/usr/sbin"];
    let mut candidate: Option<std::path::PathBuf> = None;
    for d in canonical_dirs {
        let p = std::path::Path::new(d).join(&target);
        if p.is_file() {
            candidate = Some(p);
            break;
        }
    }
    // Fall back to $PATH if not found in canonical_dirs.
    if candidate.is_none() {
        if let Ok(path) = std::env::var("PATH") {
            for d in path.split(':') {
                if d.is_empty() {
                    continue;
                }
                let p = std::path::Path::new(d).join(&target);
                if p.is_file() {
                    candidate = Some(p);
                    break;
                }
            }
        }
    }

    let Some(bin) = candidate else {
        eprintln!("jpkg: '{name}' is not a built-in subcommand and no `{target}` was found on PATH");
        return ExitCode::from(127);
    };

    // execvp into the external binary. On success it never returns; on
    // failure we surface the error.
    let err = std::process::Command::new(&bin).args(rest).exec();
    eprintln!("jpkg: failed to exec {}: {err}", bin.display());
    ExitCode::from(127)
}
