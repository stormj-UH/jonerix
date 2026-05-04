#!/usr/bin/env python3
"""Repackage an existing .jpkg with a subset of its contents.

No source rebuild. Reads the existing jpkg archive, extracts its
zstd-compressed tar payload, filters files through a whitelist or
prune list, and writes a new .jpkg with updated metadata.

jpkg format:
    bytes 0-7    magic "JPKG\0\x01\0\0"
    bytes 8-11   meta_len (LE uint32)
    bytes 12..   TOML metadata (meta_len bytes)
    bytes N..    zstd-compressed tar payload

Metadata's [files] section holds sha256 + size of the payload. We
recompute both after repacking.

Usage:
    repack-jpkg.py --in <in.jpkg> --out <out.jpkg> \\
                   [--new-name NAME] [--new-version VER] \\
                   [--new-description TEXT] \\
                   (--keep-bins-file FILE | --drop-paths-file FILE)

--keep-bins-file: file with one binary name per line. Every file under
  ./bin/ that isn't listed is removed. Other files pass through unchanged.
--drop-paths-file: file with one tar-member glob per line (e.g.
  "./share/doc/*", "./bin/llvm-dwp"). Matching members are dropped.
"""

import argparse
import fnmatch
import hashlib
import io
import os
import shutil
import struct
import subprocess
import sys
import tarfile
from pathlib import Path

MAGIC = b"JPKG\x00\x01\x00\x00"


def read_jpkg(path):
    """Return (meta_bytes, payload_bytes)."""
    data = Path(path).read_bytes()
    if not data.startswith(MAGIC):
        raise ValueError(f"not a jpkg: {path}")
    (meta_len,) = struct.unpack("<I", data[8:12])
    meta = data[12 : 12 + meta_len]
    payload = data[12 + meta_len :]
    return meta, payload


def write_jpkg(path, meta_bytes, payload_bytes):
    with open(path, "wb") as f:
        f.write(MAGIC)
        f.write(struct.pack("<I", len(meta_bytes)))
        f.write(meta_bytes)
        f.write(payload_bytes)


def zstd_decompress(compressed):
    p = subprocess.run(
        ["zstd", "-d", "--stdout"], input=compressed, capture_output=True, check=True
    )
    return p.stdout


def zstd_compress(raw, level=19):
    p = subprocess.run(
        ["zstd", f"-{level}", "--stdout"],
        input=raw,
        capture_output=True,
        check=True,
    )
    return p.stdout


def load_whitelist(path):
    return [
        line.strip()
        for line in Path(path).read_text().splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]


def filter_tar(tar_bytes, keep_bins=None, drop_globs=None):
    """Re-emit a tar dropping filtered entries.

    keep_bins: list of basenames to preserve under ./bin/. Everything else
               under ./bin/ is dropped.
    drop_globs: list of fnmatch globs against member name. Matching members
                are dropped.
    """
    in_buf = io.BytesIO(tar_bytes)
    out_buf = io.BytesIO()
    kept = 0
    dropped = 0
    dropped_bytes = 0

    with tarfile.open(fileobj=in_buf, mode="r:") as tin, tarfile.open(
        fileobj=out_buf, mode="w:"
    ) as tout:
        for m in tin:
            name = m.name
            keep = True

            if drop_globs:
                for g in drop_globs:
                    if fnmatch.fnmatch(name, g):
                        keep = False
                        break

            if keep and keep_bins is not None:
                # Only apply keep-bins to files under ./bin/
                parts = name.split("/")
                # Common prefixes: './bin/...', 'bin/...'
                idx = None
                for i, p in enumerate(parts):
                    if p == "bin":
                        idx = i
                        break
                if idx is not None:
                    rest = parts[idx + 1 :]
                    if rest:  # there IS a file under bin/
                        if len(rest) == 1:
                            # Top-level entry under bin/
                            if rest[0] not in keep_bins:
                                keep = False

            if keep:
                if m.isfile():
                    data = tin.extractfile(m).read()
                    m.size = len(data)
                    tout.addfile(m, io.BytesIO(data))
                else:
                    tout.addfile(m)
                kept += 1
            else:
                if m.isfile():
                    dropped_bytes += m.size
                dropped += 1

    print(
        f"repack: kept {kept} entries, dropped {dropped} "
        f"({dropped_bytes / 1024 / 1024:.1f} MiB of file content)",
        file=sys.stderr,
    )
    return out_buf.getvalue()


def update_meta(meta_bytes, *, new_sha256, new_size, new_name=None, new_version=None, new_description=None):
    """Parse meta as text lines, rewrite specific fields. jpkg's TOML subset
    is simple — line-based key/value, no nested inline tables — so we can
    do surgical replacement.

    Drops any pre-existing [signature] table on the way out: a signature
    in the input file covers the input's payload bytes, and after we
    rewrite payload + metadata it's stale. Leaving it in place misleads
    `jpkg resign --keep-existing` into treating the repack as already
    signed, so the stale signature persists into the published artifact
    and `jpkg upgrade` rejects it with "Verification equation was not
    satisfied". Stripping forces a true resign on every repack — `jpkg
    resign` sees an unsigned file and produces a fresh signature over
    the new bytes."""
    text = meta_bytes.decode()
    out_lines = []
    in_signature_block = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("[signature]"):
            in_signature_block = True
            continue
        if in_signature_block:
            # Inside [signature]: drop until the next table header (or EOF).
            if s.startswith("[") and not s.startswith("[["):
                in_signature_block = False
                # Fall through to handle this header in the regular branch.
            else:
                continue
        if s.startswith("sha256 = "):
            out_lines.append(f'sha256 = "{new_sha256}"')
        elif s.startswith("size = "):
            out_lines.append(f"size = {new_size}")
        elif new_name and s.startswith("name = "):
            out_lines.append(f'name = "{new_name}"')
        elif new_version and s.startswith("version = "):
            out_lines.append(f'version = "{new_version}"')
        elif new_description and s.startswith("description = "):
            out_lines.append(f'description = "{new_description}"')
        else:
            out_lines.append(line)
    # Drop trailing blank lines the [signature] strip may have left behind.
    while out_lines and not out_lines[-1].strip():
        out_lines.pop()
    return ("\n".join(out_lines) + "\n").encode()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--in", dest="input", required=True)
    ap.add_argument("--out", dest="output", required=True)
    ap.add_argument("--keep-bins-file")
    ap.add_argument("--drop-paths-file")
    ap.add_argument("--new-name")
    ap.add_argument("--new-version")
    ap.add_argument("--new-description")
    ap.add_argument("--compress-level", type=int, default=19)
    args = ap.parse_args()

    keep_bins = load_whitelist(args.keep_bins_file) if args.keep_bins_file else None
    drop_globs = load_whitelist(args.drop_paths_file) if args.drop_paths_file else None

    meta, payload = read_jpkg(args.input)
    print(f"read {args.input}: meta={len(meta)} B, payload={len(payload)} B", file=sys.stderr)

    tar_data = zstd_decompress(payload)
    print(f"decompressed: {len(tar_data)} B", file=sys.stderr)

    new_tar = filter_tar(tar_data, keep_bins=keep_bins, drop_globs=drop_globs)
    print(f"filtered tar: {len(new_tar)} B", file=sys.stderr)

    new_payload = zstd_compress(new_tar, level=args.compress_level)
    new_sha = hashlib.sha256(new_payload).hexdigest()
    print(f"recompressed: {len(new_payload)} B  sha256={new_sha}", file=sys.stderr)

    new_meta = update_meta(
        meta,
        new_sha256=new_sha,
        new_size=len(new_payload),
        new_name=args.new_name,
        new_version=args.new_version,
        new_description=args.new_description,
    )

    write_jpkg(args.output, new_meta, new_payload)
    print(
        f"wrote {args.output}: total {os.path.getsize(args.output)} B",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
