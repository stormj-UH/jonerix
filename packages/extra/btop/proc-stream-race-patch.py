#!/usr/bin/env python3
"""
Mitigate aristocratos/btop#619 — SIGSEGV in libc++'s
std::basic_streambuf::uflow() when an std::istream::ignore() call is
mid-flight on a /proc file that gets invalidated under it.

The bug: btop reads /proc/<pid>/{stat,status,statm,...} via std::ifstream
and uses pread.ignore(SSmax, '\\n') to skip header bytes. SSmax is
std::numeric_limits<std::streamsize>::max(). When the file's backing
inode disappears mid-read (the PID exits between readdir and open, or
the kernel reshuffles the procfs entry), libstdc++ throws std::ios_base::failure
or sets failbit; libc++ instead chases a stale streambuf pointer in its
uflow() fast-path and crashes.

The fast-path is gated on the limit being exactly numeric_limits::max().
Passing max() - 1 forces the slower, defensive path that handles a
vanished underlying buffer correctly. This is the partial fix the
upstream maintainers (deckstose, aristocratos) verified in #619, never
merged because they were holding out for a complete solution.

On jonerix the issue is acute because:

  * jonerix's libcxx (Apache-2.0) is the only C++ runtime we ship —
    libstdc++ is GPL-3.0 and excluded from our runtime envelope.
  * The Pi 5 HDMI tty1 holds a busier baseload than SSH (DRM scanout,
    framebuffer compositor, kernel workqueue churn for vblank), so
    transient processes are constantly appearing and disappearing
    under btop's collector.

The patch is purely textual — `\\.ignore(SSmax, ` becomes
`\\.ignore(SSmax - 1, ` everywhere in src/. SSmax - 1 is still
~9.2 quintillion, well beyond any real /proc file, so behaviour for
the non-pathological case is unchanged. We intentionally apply it to
btop_theme.cpp's theme-file parser too — the change is harmless there
and keeps the substitution rule trivially simple.

Done as a pre-configure source rewrite (matching cpuname-patch.py)
because toybox patch 0.8.11 is fussy about hunk context and a 33-hit
diff would split into chunks it rejects.
"""

from __future__ import annotations

import pathlib
import re
import sys

# Pattern is anchored on `.ignore(SSmax,` (with the comma) so we can't
# accidentally rewrite a token like `SSmaxFoo` or `.ignore(SSmax)`. The
# replacement leaves the comma in place so the second argument (the
# delimiter character) is unaffected.
PATTERN = re.compile(r"\.ignore\(SSmax,")
REPLACEMENT = ".ignore(SSmax - 1,"

TARGETS = [
    pathlib.Path("src/linux/btop_collect.cpp"),  # /proc readers
    pathlib.Path("src/btop_theme.cpp"),          # theme-file readers (cosmetic, harmless)
]


def patch_file(path: pathlib.Path) -> int:
    """Return the number of substitutions made, or -1 on missing-file."""
    if not path.exists():
        print(f"proc-stream-race-patch: {path} not found (wrong cwd?)", file=sys.stderr)
        return -1
    text = path.read_text()
    new_text, n = PATTERN.subn(REPLACEMENT, text)
    if n == 0:
        # Either nothing to do, or the upstream code has already been
        # restructured. Not fatal — the bug may be gone in this version.
        print(f"proc-stream-race-patch: no SSmax,-ignore call sites in {path}")
        return 0
    path.write_text(new_text)
    print(f"proc-stream-race-patch: rewrote {n} call site(s) in {path}")
    return n


def main() -> int:
    total = 0
    missing = 0
    for target in TARGETS:
        n = patch_file(target)
        if n < 0:
            missing += 1
        else:
            total += n
    if missing == len(TARGETS):
        print(
            "proc-stream-race-patch: none of the expected source files exist; "
            "btop layout may have changed, review the patch",
            file=sys.stderr,
        )
        return 1
    if total == 0:
        print(
            "proc-stream-race-patch: no .ignore(SSmax,) call sites found; "
            "either the upstream fixed it or the pattern shifted",
            file=sys.stderr,
        )
        # Don't fail the build — the recipe is allowed to land on a
        # btop release that no longer needs this workaround.
        return 0
    print(f"proc-stream-race-patch: {total} total substitution(s) applied")
    return 0


if __name__ == "__main__":
    sys.exit(main())
