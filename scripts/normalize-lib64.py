#!/usr/bin/env python3

import sys
from pathlib import Path

TEXT_REPLACEMENTS = (
    (b"/usr/local/lib64/", b"/usr/local/lib/"),
    (b"/usr/local/lib64", b"/usr/local/lib"),
    (b"/usr/lib64/", b"/usr/lib/"),
    (b"/usr/lib64", b"/usr/lib"),
    (b"/lib64/", b"/lib/"),
    (b"/lib64", b"/lib"),
)

BINARY_REPLACEMENTS = (
    (b"/usr/local/lib64/", b"/usr/local/lib/\0\0"),
    (b"/usr/local/lib64", b"/usr/local/lib\0\0"),
    (b"/usr/lib64/", b"/usr/lib/\0\0"),
    (b"/usr/lib64", b"/usr/lib\0\0"),
    (b"/lib64/", b"/lib/\0\0"),
    (b"/lib64", b"/lib\0\0"),
)


def iter_files(path: Path):
    if path.is_symlink():
        return
    if path.is_file():
        yield path
        return
    if not path.is_dir():
        return
    for child in path.rglob("*"):
        if child.is_file() and not child.is_symlink():
            yield child


def normalize_file(path: Path) -> bool:
    data = path.read_bytes()
    replacements = TEXT_REPLACEMENTS if b"\0" not in data[:4096] else BINARY_REPLACEMENTS
    new_data = data
    for old, new in replacements:
        new_data = new_data.replace(old, new)
    if new_data == data:
        return False
    path.write_bytes(new_data)
    return True


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: normalize-lib64.py <path> [<path> ...]", file=sys.stderr)
        return 1

    changed = 0
    for arg in sys.argv[1:]:
        root = Path(arg)
        if not root.exists():
            continue
        for file_path in iter_files(root):
            if normalize_file(file_path):
                changed += 1

    leftovers = []
    for arg in sys.argv[1:]:
        root = Path(arg)
        if not root.exists():
            continue
        for file_path in iter_files(root):
            if b"/lib64" in file_path.read_bytes():
                leftovers.append(str(file_path))
                if len(leftovers) >= 20:
                    break
        if len(leftovers) >= 20:
            break

    if leftovers:
        print("normalize-lib64.py: unpatched /lib64 references remain:", file=sys.stderr)
        for file_path in leftovers:
            print(file_path, file=sys.stderr)
        return 1

    print(f"normalize-lib64.py: patched {changed} file(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
