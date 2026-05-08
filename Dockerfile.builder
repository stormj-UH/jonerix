# jonerix:builder — Development and build image
#
# Starts from jonerix:core, adds compilers, build tools, and languages.
# Interactive shell: zsh (system /bin/sh remains mksh)
#
# Build: docker build -f Dockerfile.builder --tag jonerix:builder .
#
# Usage (build packages from source):
#   docker run --rm -v "$PWD:/workspace" -w /workspace jonerix:builder \
#     -lc 'sh scripts/build-all.sh --output /workspace/.build/pkgs'

ARG CORE_IMAGE=jonerix:core

FROM ${CORE_IMAGE}

COPY scripts/bootstrap-meson.sh /usr/local/bin/bootstrap-meson
COPY scripts/image-slim.sh /usr/local/sbin/image-slim

# Cache-bust: pass CACHEBUST=${{ github.run_id }} in CI to force re-download
ARG CACHEBUST=0

# Install compilers, build tools, and languages via jpkg.
#
# IMPORTANT: the slim step (scripts/image-slim.sh) runs IN THE SAME RUN
# as the install. Docker's overlay filesystem can't shrink a prior
# layer — if the slim ran in its own RUN, it would just ADD a layer
# containing stripped copies of every binary, leaving the originals
# in the install layer. Net effect: image GROWS by ~2 GB. Folding
# install + slim into one RUN collapses both into a single layer so
# strip-in-place actually reduces layer size.
RUN jpkg update && \
    failures=0 && \
    failed='' && \
    for pkg in \
      llvm rust go \
      cmake jmake samurai flex bc byacc \
      pkgconf \
      perl python3 nodejs \
      gitredoxide \
      strace; \
    do \
      echo "Installing: $pkg" && \
      if ! jpkg install --force "$pkg"; then \
        failures=$((failures + 1)); \
        failed="$failed $pkg"; \
      fi; \
    done && \
    if [ "$failures" -ne 0 ]; then \
      echo "builder package install failures:$failed"; \
      exit 1; \
    fi && \
    /usr/local/bin/bootstrap-meson && \
    sh /usr/local/sbin/image-slim && \
    rm /usr/local/sbin/image-slim

# Compiler wrappers and tool symlinks
#
# CLANG_CONFIG_FILE_SYSTEM_DIR is a compile-time CMake option, not a
# runtime env var. Alpine/jonerix clang doesn't have it set, so the
# config file at /etc/clang/<triple>.cfg is never auto-loaded.
# We create wrapper scripts that pass --config explicitly.
# Fixup: bsdtar sometimes extracts the uutils multicall binary into a
# GNUSparseFile.0/ subdirectory. Move it to the correct path so coreutils
# symlinks (rm, printf, chmod, ln, etc.) work.
RUN if [ -f /bin/GNUSparseFile.0/uutils ] && [ ! -f /bin/uutils ]; then \
      /bin/toybox cp /bin/GNUSparseFile.0/uutils /bin/uutils && \
      /bin/toybox chmod 755 /bin/uutils && \
      /bin/toybox rm -rf /bin/GNUSparseFile.0; \
    fi

# Compiler wrappers and tool symlinks
#
# CLANG_CONFIG_FILE_SYSTEM_DIR is a compile-time CMake option, not a
# runtime env var. Alpine/jonerix clang doesn't have it set, so the
# config file at /etc/clang/<triple>.cfg is never auto-loaded.
# We create wrapper scripts that pass --config explicitly.
RUN TRIPLE=$(/bin/clang-21 -dumpmachine 2>/dev/null || echo "unknown") && \
    mkdir -p /etc/clang && \
    printf -- '--rtlib=compiler-rt\n--unwindlib=libunwind\n-fuse-ld=lld\n' \
      > "/etc/clang/${TRIPLE}.cfg" && \
    rm -f /bin/clang /bin/clang++ && \
    printf '#!/bin/sh\nexec /bin/clang-21 --config="/etc/clang/%s.cfg" "$@"\n' "$TRIPLE" > /bin/clang && \
    printf '#!/bin/sh\nexec /bin/clang-21 --config="/etc/clang/%s.cfg" --unwindlib=libunwind -stdlib=libc++ -lc++ -lc++abi "$@"\n' "$TRIPLE" > /bin/clang++ && \
    chmod 755 /bin/clang /bin/clang++ && \
    ln -sf clang /bin/cc 2>/dev/null || true && \
    ln -sf clang++ /bin/c++ 2>/dev/null || true && \
    ln -sf ld.lld /bin/ld 2>/dev/null || true && \
    LLVM_BIN=; \
    for d in /lib/llvm*/bin; do [ -d "$d" ] && LLVM_BIN="$d" && break; done; \
    if [ -n "$LLVM_BIN" ]; then \
      for tool in ar nm ranlib strip objcopy objdump readelf; do \
        ln -sf "$LLVM_BIN/llvm-$tool" "/bin/$tool" 2>/dev/null || true; \
        ln -sf "$LLVM_BIN/llvm-$tool" "/bin/llvm-$tool" 2>/dev/null || true; \
      done; \
    fi && \
    ln -sf jmake /bin/make 2>/dev/null || true && \
    ln -sf samu /bin/ninja 2>/dev/null || true && \
    ln -sf python3 /bin/python 2>/dev/null || true && \
    ln -sf byacc /bin/yacc 2>/dev/null || true && \
    ln -sf flex /bin/lex 2>/dev/null || true && \
    # Linker fixups (provide GCC-compatible names via LLVM libs)
    ln -sf libunwind.so.1 /lib/libgcc_s.so.1 2>/dev/null || true && \
    ln -sf libgcc_s.so.1 /lib/libgcc_s.so 2>/dev/null || true && \
    ln -sf libc++.so.1 /lib/libstdc++.so.6 2>/dev/null || true && \
    ln -sf libstdc++.so.6 /lib/libstdc++.so 2>/dev/null || true && \
    printf '!<arch>\n' > /lib/libssp_nonshared.a 2>/dev/null || true

WORKDIR /root
ENTRYPOINT ["/bin/zsh"]
CMD ["-l"]
