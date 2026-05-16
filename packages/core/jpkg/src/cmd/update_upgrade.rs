// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT

//! `jpkg -uu` — Arch-style "refresh repos then upgrade everything".
//!
//! Equivalent to running `jpkg update && jpkg upgrade` back-to-back.  The
//! shortcut exists for the same reason `pacman -Syu` does on Arch: one short
//! invocation refreshes the index and then upgrades every installed package
//! that's out of date.
//!
//! Semantics:
//!
//! - `update` runs first.  If it fails, `upgrade` is **not** invoked and the
//!   combined operation exits with the same code `update` returned.
//! - On `update` success, `upgrade` runs with whatever positional / flag
//!   arguments the user supplied after `-uu` (today that's just an optional
//!   package list; if `upgrade` grows flags later, they pass through with no
//!   changes to this shim).
//! - The shim never duplicates logic — it composes the existing `update::run`
//!   and `upgrade::run` entry points.

use crate::cmd::{update, upgrade};

/// Public entry point.  Forwards `args` to `upgrade` only; `update` is run
/// with no arguments (it rejects positional input).
pub fn run(args: &[String]) -> i32 {
    run_with(args, update::run, upgrade::run)
}

/// Inner, generic over the two sub-run callables, for unit testing.
///
/// Returns the exit code of `update` if it was non-zero, otherwise the exit
/// code of `upgrade`.  This is the single invariant that distinguishes the
/// `-uu` shortcut from a naïve `update; upgrade` chain.
pub(crate) fn run_with<U, P>(args: &[String], update_run: U, upgrade_run: P) -> i32
where
    U: FnOnce(&[String]) -> i32,
    P: FnOnce(&[String]) -> i32,
{
    // `update` doesn't accept positional args — pass an empty slice always.
    let rc = update_run(&[]);
    if rc != 0 {
        return rc;
    }
    upgrade_run(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn update_runs_before_upgrade() {
        // Record the order of calls.
        let log: RefCell<Vec<&'static str>> = RefCell::new(Vec::new());

        let rc = run_with(
            &[],
            |_args| {
                log.borrow_mut().push("update");
                0
            },
            |_args| {
                log.borrow_mut().push("upgrade");
                0
            },
        );

        assert_eq!(rc, 0);
        assert_eq!(*log.borrow(), vec!["update", "upgrade"]);
    }

    #[test]
    fn update_failure_short_circuits_upgrade() {
        let upgrade_called = RefCell::new(false);

        let rc = run_with(
            &[],
            |_args| 7, // simulate update failure with a distinctive code
            |_args| {
                *upgrade_called.borrow_mut() = true;
                0
            },
        );

        assert_eq!(rc, 7, "the combined op must surface update's exit code");
        assert!(
            !*upgrade_called.borrow(),
            "upgrade must not run after update failure"
        );
    }

    #[test]
    fn upgrade_failure_propagates() {
        let rc = run_with(&[], |_args| 0, |_args| 1);
        assert_eq!(rc, 1);
    }

    #[test]
    fn upgrade_receives_caller_args_unchanged() {
        let captured: RefCell<Vec<String>> = RefCell::new(Vec::new());

        let args = vec!["pkg-a".to_string(), "pkg-b".to_string()];
        let rc = run_with(
            &args,
            |update_args| {
                // `update` must always get an empty slice — it rejects
                // positional arguments.
                assert!(
                    update_args.is_empty(),
                    "update must never receive positional args from the -uu shim"
                );
                0
            },
            |upgrade_args| {
                captured.borrow_mut().extend_from_slice(upgrade_args);
                0
            },
        );

        assert_eq!(rc, 0);
        assert_eq!(*captured.borrow(), vec!["pkg-a", "pkg-b"]);
    }
}
