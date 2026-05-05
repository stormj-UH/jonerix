# jonerix UNIX V7 / SUSv4 Compliance Plan

Status: planning baseline.

Scope: Open Group UNIX V7, meaning Single UNIX Specification Version 4
conformance. This is not a Bell Labs Seventh Edition UNIX compatibility
project.

## Non-Negotiables

- Do not use source code under GPL, LGPL, AGPL, SSPL, EUPL, CC-BY-SA, or
  any copyleft license as implementation material. Treat GNU/GPL source as
  especially high-risk and out of bounds.
- New SUS utility clones are Rust crates. C is limited to existing
  permissively licensed upstream package builds, generated ABI shims, or tiny
  FFI/syscall adapters with a written exception.
- Build and test in jonerix environments:
  - `ghcr.io/stormj-uh/jonerix:builder` with `--entrypoint /bin/sh`
  - `jonerix-tormenta`, a Raspberry Pi 5 running jonerix
  - an x86 container on `castle`
- Do not commit API keys, private tokens, session cookies, or generated
  credentials. Forgejo/GitHub tokens must live in environment variables or
  local credential stores only.
- Every serious Rust crate must fail on warnings:
  - `#![deny(warnings)]`
  - `#![deny(unsafe_op_in_unsafe_fn)]`
  - `#![forbid(unsafe_code)]` unless the crate has an FFI/syscall boundary
- Every public boundary accepts messy input, validates it, and hands only
  domain types to the core.
- Every implementation must preserve jonerix's POSIX-first discipline for
  scripts, recipes, and shipped support files.

## Clean-Room Inputs

Allowed:

- The Single UNIX Specification, POSIX text, RFCs, man pages, standards, and
  public protocol or file format documentation.
- Black-box behavior captured from already-built tools, including byte output,
  exit status, errno behavior, timings, syscall traces, and debugger sessions.
- Upstream conformance tests and generated test corpora, subject to their
  licenses and without copying copyleft implementation code or data.
- Ghidra or binary analysis notes when source-license contamination would be a
  risk. Notes must record observations, not copy decompiled source structure as
  implementation. Any binary-analysis notes used for implementation must be
  checked in beside the source or included in the source-release provenance
  before packaging.
- Permissively licensed reference implementations only when their license and
  provenance are explicit, and only if using them does not violate the target
  package's clean-room story.

Forbidden:

- Reading GNU/GPL/LGPL/AGPL/SSPL/EUPL/CC-BY-SA or other copyleft source to
  implement behavior.
- Copying diagnostics, tables, algorithms, data files, or test fixtures from
  copyleft source unless the exact artifact is independently licensed for that
  use.
- Importing permissive code and replacing its copyright header unless it is
  0BSD/public-domain-equivalent and the provenance supports doing so.

## Product Shape

Formal certification needs a fixed product, not a flexible BYOK promise. The
first target should be:

- `jonerix-unixv7-x86_64`
- QEMU-bootable and container-testable where possible
- fixed Linux kernel config from `packages/extra/linux/jonerix-x86_64.config`,
  or an explicitly wired `uni7` fragment in the linux recipe
- fixed package set from a new `uni7` recipe or manifest
- documented compiler environment with `/bin/c99` backed by clang
- documented terminal, locale, IPC, filesystem, and device assumptions

The Raspberry Pi target can follow after the x86 profile is passing enough
tests to justify hardware time.

## Interface Matrix

Create a machine-readable matrix before writing large amounts of code:

- `docs/unixv7/interface-matrix.toml`
- one entry per SUSv4 system interface, utility, header, and X/Open Curses
  interface
- fields:
  - `name`
  - `class` (`system`, `utility`, `header`, `curses`)
  - `required_by` (`posix`, `xsi`, `xcurses`, option group)
  - `provider` (`musl`, `linux`, `toybox`, `mksh`, `ncurses`, package name)
  - `status` (`covered`, `partial`, `missing`, `blocked`, `unknown`)
  - `probe` (`path`, `header`, `compile`, `manual`)
  - `paths` (absolute, normalized paths to probe under a rootfs)
  - `verification`
  - `notes`

Then add a Rust audit tool:

- `tools/uni7-audit/`
- reads the matrix
- probes a jonerix rootfs/container
- checks command presence, header presence, feature-test macros, and basic
  compile/link smoke tests
- emits stable text and JSON output for CI

The matrix is the coordination point. No compatibility project should be
treated as done until its matrix entries name tests that prove it.

## Architecture Rule For New Tools

Each Rust clone follows the same shape:

```text
CLI / FFI / filesystem / network / env / time
        |
        v
parsing + validation + error conversion
        |
        v
pure core with strong domain types
        |
        v
small output adapter
```

The core must avoid:

- filesystem calls
- network calls
- environment reads
- global mutable state
- time calls
- randomness
- logging as control flow
- panic-based error handling

Those effects enter as values or traits. The CLI owns messy strings, paths,
environment variables, terminal state, and errno conversion. The core owns
validated types and deterministic behavior.

## Type Discipline

Introduce domain types when a function would otherwise accept:

- more than one primitive argument of the same type
- a `bool` that changes behavior
- an `Option` whose meaning is subtle
- a string/path/number that has not been validated

Examples:

- `UserName`, not `String`
- `LocaleName`, not `String`
- `ExitStatusCode`, not `i32`
- `ByteOffset`, not `usize`
- `LineNumber`, not `usize`
- `FollowSymlinks`, not `bool`
- `TestModeIdentity`, not `bool`

Make illegal states unrepresentable. Constructors validate once. Internal
functions accept the validated type.

## Invariant Ledger

Every serious module gets an invariant ledger before or alongside major
refactors:

- `INVARIANTS.md` at the crate root, or a top-level `//! Invariants` section
  for very small crates
- one section per module
- for each invariant:
  - identifier, for example `PATH-001`
  - statement
  - constructors that establish it
  - functions allowed to mutate it
  - tests or proofs that cover it
  - panic/drop/early-return considerations

Every function review asks:

- What invariant does this function require?
- What invariant does it restore before returning?
- Can panic, drop, or early return break it?

For state machines, each transition must name the state it accepts and the
state it returns.

## Panic Audit

Each crate must have a panic audit before release:

- `rg -n "panic!|unwrap\\(|expect\\(|todo!|unimplemented!|assert!" src tests`
- `unwrap` and `expect` are allowed in tests and build scripts when justified
- production paths must return typed errors unless the panic is proving an
  internal invariant that cannot be violated by external input
- every allowed production panic gets a comment naming the invariant it proves

Fuzzers should treat panics as failures unless the input generator explicitly
violated a private constructor's preconditions.

## Unsafe Policy

Default: zero unsafe.

Allowed unsafe zones:

- syscall and ioctl wrappers
- C ABI compatibility shims
- raw terminal, socket, or shared-memory boundaries
- narrow performance hotpaths only after measurement and safer alternatives
  are documented

Every unsafe block needs a nearby `SAFETY:` comment:

```rust
// SAFETY:
// - ptr is non-null.
// - ptr is aligned for T.
// - ptr points to initialized memory.
// - len elements are valid for reads.
// - no mutable alias exists for the duration.
```

Then turn each bullet into one of:

- a type invariant
- a constructor check
- an `assert!`
- a Kani assumption/assertion
- a Miri test
- a Prusti, Creusot, or Verus contract when practical

Unsafe must be quarantined behind safe APIs. Public safe APIs cannot require
callers to uphold undocumented pointer, lifetime, alignment, or aliasing rules.

Every Rust README must include:

- unsafe line/block count
- files containing unsafe
- why each unsafe boundary exists
- what tests or proofs cover it

## Test Mode Impersonation

Rust clones should identify normally by default, but impersonate their
compatibility target under a package-specific test-mode environment variable.

Examples:

- `JMAKE_TEST_MODE=1`
- `STORMWALL_TEST_MODE=1`
- future names should follow the same pattern, for example
  `PAXOXIDE_TEST_MODE=1`

Test mode may change:

- `--version` output
- error prefixes
- diagnostics where byte-for-byte upstream tests require it
- compatibility quirks needed only for oracle comparison

Test mode must not silently weaken security checks unless the test says so
explicitly and the behavior is isolated.

## Verification Ladder

Use the narrowest failing evidence first, then widen:

1. Unit tests for parser, formatter, domain types, and state machines.
2. Regression tests for every real bug fixed.
3. Oracle tests against black-box behavior.
4. Upstream conformance or application test suites.
5. Fuzzing for parsers and binary formats.
6. Miri for unsafe-free core logic where useful.
7. Kani/contract checks for compact invariants and arithmetic.
8. Build inside `jonerix:builder` for both x86_64 and aarch64.
9. Runtime smoke in a jonerix container for both architecture images when the
   package is architecture-independent at source level.
10. Runtime smoke on `jonerix-tormenta` when hardware, login, tty, or Pi
    behavior matters.
11. CI package build for both x86_64 and aarch64, followed by release artifact
    verification.

No package release is final until its source tarball has been downloaded back
and its SHA256 matches the recipe.

## Initial SUSv4 Gap Areas

These are likely missing or partial in the current package set. The matrix and
official tests decide final priority.

Utilities:

- `at`, `batch`
- `compress`, `uncompress`
- `crontab`
- `ed`
- `ex`, `vi`
- `gencat`
- `iconv`
- `ipcrm`, `ipcs`
- `join`
- `locale`, `localedef`
- `lp`
- `mailx`
- `mesg`
- `pathchk`
- `pax`
- `pr`
- `tabs`
- `tsort`
- `uudecode`, `uuencode`
- `write`
- `/bin/c99`

Library and header surfaces:

- `_XOPEN_SOURCE=700` exposure and `_XOPEN_*` reporting
- XSI message catalogs
- XSI `fmtmsg`
- XSI `ndbm`
- `utmpx` behavior and files
- locale generation and collation behavior
- SysV IPC end-to-end behavior
- AIO, message queues, and realtime option boundaries
- X/Open Curses validation over `ncurses`

## Candidate Rust Workstreams

Names are placeholders until repos are created.

### `uni7`

The jpkg package/profile name for the UNIX V7/SUSv4 compliance surface. It
should own the profile manifest, depend on the selected conformance providers,
and eventually install `uni7-audit` or a packaged audit companion.

### `uni7-audit`

Rust audit tool for the interface matrix. This should come first because it
turns the whole project into measurable work.

### `c99-shim`

Small Rust wrapper around clang. It validates SUSv4 `c99` options, maps them to
the jonerix clang/LLD/libc++/compiler-rt environment, and emits target-compatible
diagnostics in test mode.

### `localeoxide`

Rust `locale`, `localedef`, and locale database tooling. This is a large
compatibility surface and should start with `C`, `POSIX`, `C.UTF-8`, and
`en_US.UTF-8` behavior before expanding.

### `catmsgoxide`

Rust message catalog support: `gencat` plus tests for `catopen`, `catgets`,
and `catclose` behavior. If musl lacks required libc pieces, add a narrow
compatibility library or wrapper rather than polluting unrelated packages.

### `paxoxide`

Rust `pax` with ustar/cpio formats, strict path validation, hardlink/symlink
handling, sparse/large file decisions, and archive traversal hardening.

### `edoxide`

Rust `ed` first. `ex`/`vi` should not start until the `ed` command language,
regex behavior, file IO semantics, and diagnostics are under control.

### `joboxide`

Rust `at`, `batch`, and `crontab` built around a small scheduler core. It
should integrate with `snooze` or OpenRC only through adapters.

### `mailxoxide`

Rust `mailx` with local mailbox behavior first. Network delivery should be
explicitly out of core and behind traits.

### `lpoxide`

Rust `lp` compatibility. Start with queue submission and status semantics that
SUSv4 tests require; leave real printer backends behind adapters.

### `ipcutils`

Rust `ipcrm` and `ipcs` backed by Linux SysV IPC. The unsafe/syscall boundary
must be tiny and covered by container tests.

## Security Requirements

Each package gets a security review section in its README:

- threat model
- parser trust boundaries
- filesystem race policy
- symlink/hardlink policy
- temp-file policy
- privilege boundary
- unsafe audit
- fuzz targets
- known non-goals

Security fixes must not be allowed to create unacceptable performance
regressions. Performance fixes must not bypass validation or weaken invariants.
If they conflict, the implementation is not done.

## Performance Requirements

For every serious tool:

- identify hotpaths with benchmarks before refactoring
- document input sizes and complexity expectations
- prefer linear or near-linear algorithms
- reject hidden quadratic behavior in parsers, path normalization, archive
  tables, sort/merge paths, and regex-heavy flows
- compare against the original black-box target when legal and practical
- keep performance benchmarks separate from conformance tests

## Copyright Headers

New jonerix-owned source files should use:

```text
Copyright (c) 2026 Jon-Erik G. Storm, Inc., a California Corporation,
doing business as LAVA GOAT SOFTWARE
```

Do not replace imported copyright headers unless the imported code is 0BSD or
public-domain-equivalent and the provenance permits re-copyrighting. Prefer
preserving provenance even when re-copyrighting would be legally allowed.

## Repository Workflow

For each new Rust clone:

1. Create or open the Forgejo repo on `castle.great-morpho.ts.net:3000`.
2. Keep credentials outside the repo, for example `FORGEJO_TOKEN`.
3. Add license, README, invariant ledger, CI, fuzz/proof scaffolding, and
   minimal package source.
4. Build and test in `jonerix:builder`.
5. Add a jonerix recipe matching adjacent oxide conventions.
6. Vendor Cargo dependencies for offline jpkg builds.
7. Build package locally with `jpkg local build`.
8. Smoke in a jonerix container and on target hardware when relevant.
9. Create a source tarball release from the Forgejo repo.
10. Mirror the source tarball to a GitHub `stormj-UH/jonerix` source release
    because GitHub-hosted package CI cannot fetch Tailscale-only Forgejo URLs.
11. Download the mirrored source tarball back, verify SHA256, and update the
    recipe URL and hash.
12. Decide whether the package is rolling-only or part of a versioned jonerix
    release. For versioned releases, open/close `package-release-state.yml`
    according to `DESIGN.md` before calling the package release final.
13. Push the jonerix recipe change and let CI rebuild packages.
14. Verify both-arch `.jpkg` artifacts, `INDEX.zst`, and `INDEX.zst.sig`.

Commit frequently at clean points:

- after adding tests that reproduce a real issue
- after a focused fix passes its narrow test
- after widening validation passes
- after release metadata is verified

## First Milestone

Milestone 0 is documentation and measurement, not implementation heroics:

1. Add this plan.
2. Add the initial interface-matrix skeleton.
3. Add `tools/uni7-audit` skeleton.
4. Make it runnable in `jonerix:builder`.
5. Generate the first gap report against the current `jonerix:core` image.
6. Pick the first missing utility from test evidence, not aesthetics.

The likely first implementation target is `/bin/c99` or `pax`, because each has
a bounded command surface and clear tests. Locale work is more central but much
larger; start it only after the audit matrix can track partial coverage.
