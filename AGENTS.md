# jonerix - Codex Instructions

## Git Commits

- Do NOT sign commits (no --gpg-sign, no commit.gpgsign). Company policy: commits are signed by the company or a human.
- Author all commits as `@Lava-Goat` — do not use the Co-Authored-By trailer.

## Coding

- **POSIX-first.** No bashisms, no GNUisms in anything that ships. `/bin/sh` is mksh, coreutils is toybox, `patch(1)` is toybox. Use `#!/bin/sh`, `[ ]` not `[[ ]]`, `printf` not `echo -e`, `$()` not backticks, no `sed -i` / `readlink -f` / `grep -P`. Prefer POSIX APIs over `_GNU_SOURCE` in C, and avoid crates that gate on `target_env = "gnu"` in Rust. Full rule list: DESIGN.md §3 "POSIX-First Code Discipline".
