# jonerix:builder — Development and build image
#
# Starts from jonerix:core, adds compilers, build tools, and languages.
# Shell: mksh (MirOS, POSIX-compliant — zsh deadlocks on musl)
#
# Build: docker build -f Dockerfile.builder --tag jonerix:builder .
#
# Usage (build packages from source):
#   docker run --rm -v "$PWD:/workspace" -w /workspace jonerix:builder \
#     sh bootstrap/build-all.sh --output /workspace/.build/pkgs

ARG CORE_IMAGE=jonerix:core

FROM ${CORE_IMAGE}

# Install compilers, build tools, and languages via jpkg
# Order: compilers -> build tools -> languages -> extras
RUN jpkg update && \
    for pkg in \
      llvm libcxx rust go \
      cmake bmake samurai flex bc byacc \
      perl python3 pip nodejs; \
    do \
      echo "Installing: $pkg" && jpkg install "$pkg" || echo "WARN: $pkg failed"; \
    done

# Compiler and build tool symlinks
RUN ln -sf clang /bin/cc 2>/dev/null || true && \
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
    ln -sf bmake /bin/make 2>/dev/null || true && \
    ln -sf samu /bin/ninja 2>/dev/null || true && \
    ln -sf python3 /bin/python 2>/dev/null || true && \
    ln -sf byacc /bin/yacc 2>/dev/null || true && \
    ln -sf flex /bin/lex 2>/dev/null || true && \
    # Linker fixups (GCC runtime compat)
    ln -sf libgcc_s.so.1 /lib/libgcc_s.so 2>/dev/null || true && \
    ln -sf libstdc++.so.6 /lib/libstdc++.so 2>/dev/null || true && \
    printf '!<arch>\n' > /lib/libssp_nonshared.a 2>/dev/null || true && \
    # Ensure clang config exists for the actual triple (may be *-alpine-* or *-jonerix-*)
    TRIPLE=$(clang -dumpmachine 2>/dev/null || echo "") && \
    if [ -n "$TRIPLE" ]; then \
      mkdir -p /etc/clang && \
      CFG="/etc/clang/${TRIPLE}.cfg" && \
      printf -- '--rtlib=compiler-rt\n--unwindlib=libunwind\n-fuse-ld=lld\n--stdlib=libc++\n' > "$CFG"; \
    fi

# Tell clang to load /etc/clang/<triple>.cfg automatically.
# Alpine (and jonerix) clang doesn't set CLANG_CONFIG_FILE_SYSTEM_DIR,
# so without this, --rtlib=compiler-rt/--unwindlib=libunwind/-fuse-ld=lld
# are NOT applied and clang falls back to GCC CRT (crtbeginS.o/libgcc).
ENV CLANG_CONFIG_FILE_SYSTEM_DIR=/etc/clang

WORKDIR /root
ENTRYPOINT ["/bin/mksh"]
CMD ["-l"]
