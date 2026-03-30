# jonerix — Permissive Linux Distribution (full image)
# Multi-stage build: Alpine builds jpkg → jpkg installs everything → FROM scratch
#
# All components are permissively licensed — zero GPL runtime.
# Alpine is used ONLY at build time to compile jpkg (the bootstrap step).
# Everything else is installed from the jpkg package repository.
#
# Images:
#   Dockerfile.minimal  — runtime only (shell, init, network, SSH)
#   Dockerfile          — full image (runtime + compilers + languages + tools)
#   Dockerfile.develop  — develop on top of minimal (jpkg install layer)
#
# Build: docker build --tag jonerix:latest .

# ============================================================
# Stage 0: Build jpkg from source (the only Alpine build step)
# ============================================================
FROM alpine:latest AS jpkg-builder

RUN apk add --no-cache clang lld musl-dev make zstd-dev zstd-static linux-headers

COPY packages/jpkg/ /src/
RUN cd /src && \
    find . -name '*.o' -o -name '*.d' | xargs rm -f && rm -f jpkg && \
    make CC=clang LDFLAGS="-static -fuse-ld=lld" jpkg -j$(nproc) && \
    strip jpkg
RUN cd /src && make test

# ============================================================
# Stage 1: Assemble jonerix rootfs using jpkg packages
# ============================================================
FROM alpine:latest AS rootfs

# Alpine tools needed only to run jpkg during assembly
RUN apk add --no-cache curl ca-certificates zstd tar libarchive-tools

COPY --from=jpkg-builder /src/jpkg /usr/local/bin/jpkg

# Create rootfs skeleton
RUN mkdir -p \
    /jonerix/etc/jpkg/keys \
    /jonerix/etc/ssl/certs \
    /jonerix/etc/dropbear \
    /jonerix/etc/init.d \
    /jonerix/etc/conf.d \
    /jonerix/etc/network \
    /jonerix/bin \
    /jonerix/lib \
    /jonerix/include \
    /jonerix/share \
    /jonerix/var/log \
    /jonerix/var/cache/jpkg \
    /jonerix/var/db/jpkg/installed \
    /jonerix/home \
    /jonerix/root \
    /jonerix/dev \
    /jonerix/proc \
    /jonerix/sys \
    /jonerix/run \
    /jonerix/tmp && \
    chmod 1777 /jonerix/tmp && chmod 700 /jonerix/root

# Configure jpkg repo (no /usr symlink yet — would redirect to host /)
COPY config/defaults/etc/jpkg/keys/ /jonerix/etc/jpkg/keys/
RUN printf '[repo]\nurl = "https://github.com/stormj-UH/jonerix/releases/download/packages"\n' \
        > /jonerix/etc/jpkg/repos.conf

# Install bsdtar on Alpine HOST so jpkg can extract symlinks correctly
RUN apk add --no-cache libarchive-tools && ln -sf bsdtar /usr/bin/tar

# Install all packages via jpkg
# Order: base libs → compilers → build tools → languages → utilities
RUN jpkg --root /jonerix update && \
    for pkg in \
      musl ncurses openssl zlib xz lz4 zstd ca-certificates \
      toybox bsdtar zsh \
      llvm \
      cmake bmake samurai flex bc byacc \
      perl python3 nodejs \
      npm pip \
      curl dropbear openrc doas \
      snooze socklog dhcpcd ifupdown-ng unbound \
      mandoc pigz fastfetch \
      micro; \
    do \
      echo "=== Installing: $pkg ===" && \
      jpkg --root /jonerix install "$pkg" || echo "WARN: $pkg failed"; \
    done

# Flatten usr/ into / (merged-usr layout), then add the symlink
RUN if [ -d /jonerix/usr ]; then \
        cp -a /jonerix/usr/. /jonerix/ && rm -rf /jonerix/usr; \
    fi && \
    ln -s / /jonerix/usr

# Copy jpkg itself into the rootfs
RUN cp /usr/local/bin/jpkg /jonerix/bin/jpkg

# Post-install symlinks
RUN \
    # sh symlink (required by system() calls)
    # zsh as bash (bash-compatible mode)
    ln -sf zsh /jonerix/bin/bash 2>/dev/null || true && \
    # tar → bsdtar (toybox tar can't handle symlinks in jpkg archives)
    ln -sf bsdtar /jonerix/bin/tar && \
    # ssh → dbclient (dropbear SSH client)
    ln -sf dbclient /jonerix/bin/ssh 2>/dev/null || true && \
    # Standard tool names for build systems
    ln -sf clang /jonerix/bin/cc 2>/dev/null || true && \
    ln -sf clang++ /jonerix/bin/c++ 2>/dev/null || true && \
    ln -sf ld.lld /jonerix/bin/ld 2>/dev/null || true && \
    ln -sf llvm-ar /jonerix/bin/ar 2>/dev/null || true && \
    ln -sf llvm-nm /jonerix/bin/nm 2>/dev/null || true && \
    ln -sf llvm-ranlib /jonerix/bin/ranlib 2>/dev/null || true && \
    ln -sf llvm-strip /jonerix/bin/strip 2>/dev/null || true && \
    ln -sf llvm-objcopy /jonerix/bin/objcopy 2>/dev/null || true && \
    ln -sf llvm-objdump /jonerix/bin/objdump 2>/dev/null || true && \
    ln -sf llvm-readelf /jonerix/bin/readelf 2>/dev/null || true && \
    ln -sf bmake /jonerix/bin/make 2>/dev/null || true && \
    ln -sf samu /jonerix/bin/ninja 2>/dev/null || true && \
    ln -sf python3 /jonerix/bin/python 2>/dev/null || true && \
    ln -sf flex /jonerix/bin/lex 2>/dev/null || true && \
    ln -sf micro /jonerix/bin/editor 2>/dev/null || true && \
    ln -sf micro /jonerix/bin/vi 2>/dev/null || true

# --- Default config files ---
COPY config/defaults/etc/hostname     /jonerix/etc/hostname
COPY config/defaults/etc/resolv.conf  /jonerix/etc/resolv.conf
COPY config/defaults/etc/passwd       /jonerix/etc/passwd
COPY config/defaults/etc/group        /jonerix/etc/group
COPY config/defaults/etc/shadow       /jonerix/etc/shadow
COPY config/defaults/etc/shells       /jonerix/etc/shells
COPY config/defaults/etc/profile      /jonerix/etc/profile
COPY config/defaults/etc/doas.conf    /jonerix/etc/doas.conf
COPY config/defaults/etc/os-release   /jonerix/etc/os-release
COPY config/defaults/etc/fastfetch/   /jonerix/etc/fastfetch/
COPY config/openrc/init.d/            /jonerix/etc/init.d/
RUN chmod 755 /jonerix/etc/init.d/* 2>/dev/null || true

COPY scripts/license-audit.sh /jonerix/bin/license-audit
RUN chmod 755 /jonerix/bin/license-audit

# ============================================================
# Stage 2: Final image — FROM scratch, zero GPL runtime
# ============================================================
FROM scratch

COPY --from=rootfs /jonerix/ /

ENV PATH=/bin
ENV HOME=/root
ENV SHELL=/bin/zsh
ENV TERM=xterm
ENV EDITOR=micro
ENV VISUAL=micro
ENV LANG=C.UTF-8

WORKDIR /root

ENTRYPOINT ["/bin/zsh"]
CMD ["-l"]
