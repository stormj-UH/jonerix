#!/bin/sh
# scripts/image-slim.sh — final-layer cleanup for jonerix:builder and :router.
#
# Run AFTER all `jpkg install` steps as the last RUN in the Dockerfile. Trims
# ~400–600 MB off the builder image (a mix of debug symbols, man pages, info
# docs, locales, and cached .jpkg downloads).
#
# Safe to run unconditionally; every operation is wrapped in `|| true` so
# missing tools or paths don't break the image build.

set -u

# 1. Strip binaries and shared libraries.
#    --strip-unneeded on .so files: keeps the dynamic symbol table so dlopen
#    and linking still work, drops local symbols + debug info.
#    --strip-all on executables: removes everything non-essential.
#    Non-ELF files (shell scripts etc.) just produce a benign stderr error.
for d in /bin /lib /usr/bin /usr/lib /usr/local/bin /usr/local/lib; do
    [ -d "$d" ] || continue
    find "$d" -type f -name '*.so*' -exec llvm-strip --strip-unneeded {} + 2>/dev/null || true
    find "$d" -type f -name '*.a'   -exec llvm-strip --strip-debug     {} + 2>/dev/null || true
    # Executable binaries — only strip files that llvm-strip recognises as ELF.
    # Non-ELF failures are silent thanks to `|| true`.
    find "$d" -maxdepth 1 -type f -perm -u+x -not -name '*.so*' -not -name '*.a' \
        -exec llvm-strip --strip-all {} + 2>/dev/null || true
done

# 2. Remove documentation, man pages, info pages, locales.
#    jonerix images target server/build use — no one reads `man clang` in a
#    container. Install the matching package on a dev machine if you want docs.
for d in \
    /share/doc /share/info /share/man /share/locale /share/gtk-doc \
    /usr/share/doc /usr/share/info /usr/share/man /usr/share/locale /usr/share/gtk-doc \
    /usr/local/share/doc /usr/local/share/info /usr/local/share/man
do
    rm -rf "$d" 2>/dev/null || true
done

# 3. Purge the jpkg download cache — installs are done; these are just
#    compressed copies of files we've already extracted.
rm -rf /var/cache/jpkg/*.jpkg /var/cache/jpkg/INDEX* 2>/dev/null || true

# 4. Rust-specific cleanup: drop documentation and unused targets.
#    Rust ships rustlib/ per-target including tests. On jonerix's
#    aarch64/x86_64 musl targets we only need the one that matches host arch;
#    cross-targets (wasm, etc.) add hundreds of MB and are rarely used.
rm -rf /share/doc/rust 2>/dev/null || true
# rust-lldb is a Python wrapper; we have no LLDB on jonerix anyway.
rm -f /bin/rust-lldb /usr/bin/rust-lldb 2>/dev/null || true

# 5. Go-specific: trim the per-target test corpora. `go build` only needs
#    pkg/, not src/<stdlib>/testdata/ (large test fixtures).
find /lib/go/src /usr/lib/go/src /usr/local/go/src 2>/dev/null \
    -type d -name testdata -prune -exec rm -rf {} + 2>/dev/null || true

# 6. Python __pycache__/*.pyc — Python will regenerate these on first use.
find /lib/python* /usr/lib/python* 2>/dev/null \
    -type d -name __pycache__ -prune -exec rm -rf {} + 2>/dev/null || true

echo "image-slim: done"
