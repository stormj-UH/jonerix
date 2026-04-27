//! `cmd::*` — one sub-module per jpkg subcommand.
//!
//! Each module exposes a `pub fn run(args: ...) -> i32` (or returns a typed
//! `Result`) and is wired up by the main dispatcher in `src/bin/jpkg.rs`
//! (or `src/bin/jpkg-local.rs` for the local-only verbs).

pub mod search;
pub mod info;
pub mod license_audit;
pub mod keygen;
pub mod sign;
pub mod update;
pub mod verify;
pub mod install;
pub mod remove;
pub mod upgrade;
pub mod build;
pub mod build_world;
pub mod local_install;

/// Shared helpers: chroot+bind-mount for hooks, payload extraction, db.insert
/// wrappers, replaces-ownership transfer.  Owned by Worker L (install side);
/// Worker M (build side) imports the merged-/usr flattening helpers.
pub mod common;
