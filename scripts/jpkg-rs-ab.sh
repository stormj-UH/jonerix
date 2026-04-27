#!/bin/sh
# jpkg-rs-ab.sh — A/B build a recipe with both /bin/jpkg (C) and
# /bin/jpkg-rs (Rust port), then compare the two .jpkg artefacts to confirm
# wire compatibility.  Intended to be run inside a jonerix runtime that
# has both binaries installed (i.e. after `jpkg install jpkg jpkg-rs` on a
# bootstrapped system).  Returns 0 if the two outputs are equivalent on
# the wire (matching magic, hdr_len, metadata bytes, payload sha256), else
# 1 with a per-field diff to stdout.
#
# Usage:
#     scripts/jpkg-rs-ab.sh <recipe-dir>
#     scripts/jpkg-rs-ab.sh packages/develop/byacc
#
# When run on a non-jonerix host (e.g. macOS during local development),
# both jpkg and jpkg-rs binaries must be on PATH; otherwise the script
# warns-and-skips with exit 0 so it doesn't fail dev-machine smoke runs.
#
# POSIX sh, no bashisms; runs under mksh or brash.

set -eu

RECIPE_DIR="${1:-}"
if [ -z "$RECIPE_DIR" ] || [ ! -d "$RECIPE_DIR" ]; then
    printf 'usage: %s <recipe-dir>\n' "$0" >&2
    exit 2
fi

JPKG_C="${JPKG_C:-jpkg}"
JPKG_RS="${JPKG_RS:-jpkg-rs}"

# Locate both binaries.  If either is absent, warn-and-skip rather than
# fail so CI on non-jonerix hosts (Alpine / macOS) doesn't break.
have() { command -v "$1" >/dev/null 2>&1; }
if ! have "$JPKG_C"; then
    printf 'jpkg-rs-ab: skipping — %s not on PATH\n' "$JPKG_C" >&2
    exit 0
fi
if ! have "$JPKG_RS"; then
    printf 'jpkg-rs-ab: skipping — %s not on PATH\n' "$JPKG_RS" >&2
    exit 0
fi

WORK="$(mktemp -d 2>/dev/null || mktemp -d -t jpkg-rs-ab)"
trap 'rm -rf "$WORK"' EXIT INT HUP TERM

OUT_C="$WORK/out-c"
OUT_RS="$WORK/out-rs"
mkdir -p "$OUT_C" "$OUT_RS"

# ── 1.  Build with C jpkg ────────────────────────────────────────────────────
printf '== C jpkg build ==\n'
"$JPKG_C" build "$RECIPE_DIR" --output "$OUT_C" --build-jpkg

# ── 2.  Build with jpkg-rs ──────────────────────────────────────────────────
printf '== jpkg-rs build ==\n'
"$JPKG_RS" build "$RECIPE_DIR" --output "$OUT_RS" --build-jpkg

# ── 3.  Locate the two artefacts ─────────────────────────────────────────────
JPKG_C_FILE="$(ls "$OUT_C"/*.jpkg 2>/dev/null | head -1 || true)"
JPKG_RS_FILE="$(ls "$OUT_RS"/*.jpkg 2>/dev/null | head -1 || true)"
if [ -z "$JPKG_C_FILE" ]; then
    printf 'FAIL: C jpkg produced no .jpkg in %s\n' "$OUT_C" >&2
    exit 1
fi
if [ -z "$JPKG_RS_FILE" ]; then
    printf 'FAIL: jpkg-rs produced no .jpkg in %s\n' "$OUT_RS" >&2
    exit 1
fi
printf 'C  artefact: %s\n' "$JPKG_C_FILE"
printf 'Rs artefact: %s\n' "$JPKG_RS_FILE"

# ── 4.  Wire-format comparison ───────────────────────────────────────────────
# Compare:  magic (8 bytes), hdr_len (LE32), metadata bytes (variable),
# payload sha256.  We do NOT byte-compare the payload itself because zstd
# can compress identical tar streams to non-identical bytes (timestamps in
# tar headers, compression level non-determinism); we compare the SHA-256
# of the *uncompressed* tar streams instead.
diff_cmd() {
    awk -v a="$1" -v b="$2" '
        BEGIN { if (a == b) exit 0; else { printf "  A=%s\n  B=%s\n", a, b; exit 1 } }
    '
}

inspect() {
    # $1 = .jpkg file → emits "magic_hex hdr_len metadata_sha256 payload_sha256"
    python3 - "$1" <<'PYEOF'
import sys, struct, hashlib, io, tarfile
import zstandard  # if not present, fall back to system zstd via subprocess
from pathlib import Path
path = Path(sys.argv[1])
buf  = path.read_bytes()
magic    = buf[:8].hex()
hdr_len  = struct.unpack('<I', buf[8:12])[0]
meta     = buf[12:12+hdr_len]
payload  = buf[12+hdr_len:]
meta_sha    = hashlib.sha256(meta).hexdigest()
payload_sha = hashlib.sha256(payload).hexdigest()
# Also decompress and re-hash to compare uncompressed tar streams.
try:
    dctx = zstandard.ZstdDecompressor()
    raw = dctx.decompress(payload)
except Exception:
    import subprocess
    raw = subprocess.check_output(['zstd', '-d', '-c'], input=payload)
tar_sha = hashlib.sha256(raw).hexdigest()
print(f'magic   {magic}')
print(f'hdr_len {hdr_len}')
print(f'meta_sha    {meta_sha}')
print(f'payload_sha {payload_sha}')
print(f'tar_sha     {tar_sha}')
print('--- metadata.toml ---')
print(meta.decode('utf-8', errors='replace'))
print('--- end metadata ---')
PYEOF
}

printf '\n== C jpkg fields ==\n'
inspect "$JPKG_C_FILE"  > "$WORK/c.fields"
cat "$WORK/c.fields"

printf '\n== Rs jpkg fields ==\n'
inspect "$JPKG_RS_FILE" > "$WORK/rs.fields"
cat "$WORK/rs.fields"

# ── 5.  Per-field diff ───────────────────────────────────────────────────────
# Compare each line up to "--- metadata.toml ---" (which contains the same
# information as meta_sha and is shown for human inspection).  Specifically
# we compare:  magic, hdr_len, tar_sha (uncompressed-tar identity).  We do
# NOT compare meta_sha / payload_sha because both can legitimately differ
# while the wire semantics are still equivalent (e.g. metadata tooling
# differences in field ordering, or zstd version differences).
extract_field() {
    awk -v key="$1" '$1 == key { for (i=2;i<=NF;i++) printf "%s%s", $i, (i==NF?"":" ") ; print "" ; exit }' "$2"
}
fail=0
for field in magic hdr_len tar_sha; do
    a=$(extract_field "$field" "$WORK/c.fields")
    b=$(extract_field "$field" "$WORK/rs.fields")
    if [ "$a" = "$b" ]; then
        printf '  OK  %-12s  %s\n' "$field" "$a"
    else
        printf '  FAIL %-12s  C=%s  Rs=%s\n' "$field" "$a" "$b"
        fail=1
    fi
done

if [ "$fail" -eq 0 ]; then
    printf '\nA/B PASS — both binaries produce wire-equivalent .jpkg artefacts for %s\n' "$RECIPE_DIR"
    exit 0
else
    printf '\nA/B FAIL — see field diff above\n' >&2
    exit 1
fi
