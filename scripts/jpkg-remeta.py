#!/usr/bin/env python3
"""jpkg-remeta — repackage a .jpkg with new metadata (no rebuild).

jpkg file format (from packages/jpkg/src/pkg.c):
  [0..7]      magic: 'JPKG' + 0x00 0x01 0x00 0x00
  [8..11]     uint32 LE: meta TOML length (N)
  [12..12+N]  meta TOML bytes
  [12+N..]    zstd-compressed tar payload

Usage:
  jpkg-remeta INPUT.jpkg OUTPUT.jpkg NEW_VERSION [KEY=VAL ...]

NEW_VERSION replaces `version = "..."` in the [package] block.

Any extra KEY=VAL pairs are INJECTED into the [package] block (before
the next section, or at the end if [package] is the only section).
Use TOML syntax for the value:

  jpkg-remeta old.jpkg new.jpkg R59c-r1 'replaces = ["toybox"]'

The zstd payload is carried over byte-for-byte. This is the fast path
for adjusting metadata on an already-built jpkg — e.g. adding a
`package.replaces` field to existing jpkgs so they silently take over
paths from toybox without a full recompile.

Used 2026-04-15 to re-meta mksh-R59c → R59c-r1, bsdtar-3.8.6-r5 →
3.8.6-r6, ncurses-6.5 → 6.5-r1 with `replaces = ["toybox"]` so the
new jpkg 1.0.4 file-ownership-transfer logic could clean up the
long-standing /bin/{sh,tar,reset,clear} manifest conflicts.
"""

import sys
import struct
import re

MAGIC = b"JPKG\x00\x01\x00\x00"


def main():
    if len(sys.argv) < 4:
        print(__doc__)
        sys.exit(1)

    inp, outp, new_ver, *extras = sys.argv[1:]

    data = open(inp, "rb").read()
    if data[:8] != MAGIC:
        raise SystemExit(f"not a jpkg (bad magic): {data[:8]!r}")

    meta_len = struct.unpack("<I", data[8:12])[0]
    meta = data[12:12 + meta_len].decode()
    payload = data[12 + meta_len:]

    # Walk lines; rewrite `version =` inside [package]; inject extras
    # before the next [section] (or append if no later section).
    lines = meta.split("\n")
    in_package = False
    version_updated = False
    extras_injected = False
    out_lines = []

    for ln in lines:
        stripped = ln.strip()

        if stripped.startswith("[package]"):
            in_package = True
            out_lines.append(ln)
            continue

        # Leaving [package]: inject extras before the next section header.
        if in_package and stripped.startswith("[") and not stripped.startswith("[package]"):
            for kv in extras:
                if "=" in kv and kv not in out_lines:
                    out_lines.append(kv)
            extras_injected = True
            in_package = False
            out_lines.append(ln)
            continue

        if in_package and re.match(r"\s*version\s*=", ln):
            out_lines.append(f'version = "{new_ver}"')
            version_updated = True
            continue

        out_lines.append(ln)

    # No other section after [package]: append extras at the end.
    if in_package and not extras_injected:
        for kv in extras:
            if "=" in kv and kv not in out_lines:
                out_lines.append(kv)

    if not version_updated:
        print("WARN: version not found in [package] — leaving meta's version as-is",
              file=sys.stderr)

    new_meta = "\n".join(out_lines).encode()
    out = MAGIC + struct.pack("<I", len(new_meta)) + new_meta + payload

    open(outp, "wb").write(out)
    print(f"wrote {outp} ({len(out)} bytes; "
          f"meta={len(new_meta)}B, payload={len(payload)}B unchanged)")
    print(f"--- new meta ---\n{new_meta.decode()}")


if __name__ == "__main__":
    main()
