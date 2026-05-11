// Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
// doing business as LAVA GOAT SOFTWARE. All rights reserved.
// SPDX-License-Identifier: MIT
//
// Integration smoke tests for the `-uu` CLI shortcut added in jpkg 2.2.2.
// Network-touching behaviour is covered by the unit tests in
// src/cmd/update_upgrade.rs; here we only check what is visible at the CLI
// surface and doesn't require a configured repo.

use assert_cmd::Command;
use predicates::boolean::PredicateBooleanExt;
use predicates::str::contains;
// `PredicateBooleanExt` is brought in for `.and(...)` in the help test.

/// `jpkg --help` must advertise the new `-uu` shortcut with the Arch-style
/// one-liner description.
#[test]
fn help_mentions_uu_shortcut() {
    let mut cmd = Command::cargo_bin("jpkg").expect("jpkg binary");
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(contains("-uu"))
        .stdout(contains("update").and(contains("upgrade")));
}

/// `jpkg -uu` must be a recognised verb (not the old
/// "unknown global option: -uu" error path).  We don't care here whether
/// the actual update+upgrade succeeds (that depends on whether the host
/// has a reachable mirror configured) — only that the dispatcher saw
/// `-uu` and routed it to the combined action, not the option-parser
/// "unknown option" branch.
#[test]
fn uu_is_a_recognised_verb_not_an_unknown_option() {
    let mut cmd = Command::cargo_bin("jpkg").expect("jpkg binary");
    let scratch = tempfile::tempdir().expect("tempdir");
    cmd.args(["--root", scratch.path().to_str().unwrap(), "-uu"]);
    // We don't assert success/failure of the combined op — only the
    // negative invariant that the failure path that existed in 2.2.1 is
    // gone.  Use `get_output()` so an exit-code != 0 doesn't fail the
    // assertion harness before we check stderr.
    let output = cmd.output().expect("spawn jpkg");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unknown global option"),
        "in 2.2.2 the `-uu` shortcut must NOT trip the unknown-option \
         branch; stderr was:\n{stderr}"
    );
}
