// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg-local` — local-only operations.  Mirrors `packages/jpkg/src/main_local.c`.
//!
//! Verbs:
//!   install <file.jpkg|url|->  [--root <dir>]
//!   build   <recipe-dir|recipe.toml|url|->  [--output <dir>] [--arch <arch>] [--build-jpkg]

use std::process::ExitCode;

const USAGE: &str = "\
jpkg-local — install/build .jpkg files outside the live package database

USAGE:
    jpkg-local [GLOBAL_OPTS] <COMMAND> [ARGS...]

GLOBAL OPTIONS:
    -v, --verbose       Set log level to DEBUG
    -q, --quiet         Set log level to ERROR
    -h, --help          Print this help and exit

COMMANDS:
    install <src> [--root <dir>]
        Install a .jpkg from a local file path, HTTPS URL, or `-` (stdin).
        Default --root is `/`.

    build <recipe> [--output <dir>] [--arch <arch>] [--build-jpkg]
        Build a recipe.toml.  --output writes the .jpkg artifact to <dir>;
        --arch overrides the package target architecture for cross-builds;
        without --build-jpkg, the recipe is built and installed to the
        live rootfs (or --root) instead of being archived.
";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mut idx = 1;
    let mut log_filter: Option<&'static str> = None;

    while idx < argv.len() {
        match argv[idx].as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("jpkg-local {}", env!("CARGO_PKG_VERSION"));
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
            s if !s.starts_with('-') => break,
            other => {
                eprintln!("jpkg-local: unknown global option: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let mut env_builder = env_logger::Builder::new();
    if let Some(lvl) = log_filter {
        env_builder.parse_filters(lvl);
    } else if let Ok(env) = std::env::var("JPKG_LOG") {
        env_builder.parse_filters(&env);
    } else {
        env_builder.parse_filters("info");
    }
    let _ = env_builder.try_init();

    if idx >= argv.len() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let verb = &argv[idx];
    let verb_args: Vec<String> = argv[idx + 1..].to_vec();

    use jpkg::cmd;
    let rc: i32 = match verb.as_str() {
        "install" => cmd::local_install::run(&verb_args),
        "build" => cmd::build::run(&verb_args),
        other => {
            eprintln!("jpkg-local: unknown subcommand '{other}'");
            return ExitCode::from(2);
        }
    };

    ExitCode::from(rc.clamp(0, 255) as u8)
}
