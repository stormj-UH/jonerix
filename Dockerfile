# jonerix — Permissive Linux Distribution
# Multi-stage build: Alpine host → Build components → Minimal rootfs (FROM scratch)
#
# GPL tools (Alpine, gcc) used only at build time. Final image is pure permissive.
# All runtime components: MIT, BSD, ISC, 0BSD, PSF, Apache-2.0, zlib, or public domain.

# ============================================================
# Stage 0: Build all permissive components from source
# ============================================================
FROM alpine:latest AS builder

RUN apk add --no-cache \
    clang lld musl-dev musl-utils make cmake samurai curl patch tar xz \
    zstd-dev zstd-static linux-headers bash \
    zlib-dev zlib-static openssl-dev openssl-libs-static \
    libffi-dev ncurses-dev readline-dev m4 bison \
    python3 \
    && apk add --no-cache --repository=https://dl-cdn.alpinelinux.org/alpine/edge/community mksh

WORKDIR /build

# --- Build toybox (0BSD) from source ---
RUN curl -fsSL https://landley.net/toybox/downloads/toybox-0.8.11.tar.gz -o toybox.tar.gz && \
    tar xf toybox.tar.gz && cd toybox-0.8.11 && \
    make defconfig && \
    make CC=clang CFLAGS="-Os -static" -j$(nproc) && \
    cp toybox /build/toybox && strip /build/toybox

# --- Copy mksh (MirOS/ISC) ---
RUN cp $(find / -name mksh -type f 2>/dev/null | head -1) /build/mksh

# --- Get pico (Apache-2.0) from Alpine email client package ---
RUN apk add --no-cache alpine && \
    cp /usr/bin/pico /build/pico

# --- Build bmake (MIT) — BSD make, needed for kernel Makefiles ---
RUN curl -fsSL https://www.crufty.net/ftp/pub/sjg/bmake-20240808.tar.gz -o bmake.tar.gz && \
    tar xf bmake.tar.gz && cd bmake && \
    ./configure --prefix=/usr CC=clang && \
    sh make-bootstrap.sh && \
    find . -name bmake -type f -executable && \
    cp $(find . -name bmake -type f -executable | head -1) /build/bmake-bin && \
    strip /build/bmake-bin

# --- Build flex (BSD license) — lexer generator for kernel ---
RUN curl -fsSL https://github.com/westes/flex/releases/download/v2.6.4/flex-2.6.4.tar.gz -o flex.tar.gz && \
    tar xf flex.tar.gz && cd flex-2.6.4 && \
    ./configure --prefix=/usr CC=clang LDFLAGS="-static -fuse-ld=lld" CFLAGS="-Os" && \
    make -j$(nproc) && \
    cp src/flex /build/flex && strip /build/flex

# --- Get perl (Artistic License — permissive) for kernel scripts ---
RUN apk add --no-cache perl && \
    cp /usr/bin/perl /build/perl

# --- Build bc (BSD license) — calculator needed by kernel build ---
# Use gavin-bc, a BSD-licensed bc implementation
RUN curl -fsSL https://github.com/gavinhoward/bc/releases/download/7.0.3/bc-7.0.3.tar.xz -o bc.tar.xz && \
    tar xf bc.tar.xz && cd bc-7.0.3 && \
    CC=clang LDFLAGS="-static -fuse-ld=lld" CFLAGS="-Os" \
    ./configure --prefix=/usr --disable-nls && \
    make -j$(nproc) && \
    cp bin/bc /build/bc && strip /build/bc

# --- Build dropbear (MIT) SSH server from source ---
RUN curl -fsSL https://matt.ucc.asn.au/dropbear/releases/dropbear-2024.86.tar.bz2 -o dropbear.tar.bz2 && \
    tar xf dropbear.tar.bz2 && cd dropbear-2024.86 && \
    ./configure --prefix=/usr --disable-wtmp --disable-lastlog \
        CC=clang LDFLAGS="-fuse-ld=lld" CFLAGS="-Os" && \
    make PROGRAMS="dropbear dbclient dropbearkey scp" -j$(nproc) && \
    cp dropbear dbclient dropbearkey scp /build/ && \
    strip /build/dropbear /build/dbclient /build/dropbearkey /build/scp 2>/dev/null || true

# --- Build jpkg (MIT) from source ---
COPY packages/jpkg/ /build/jpkg-src/
RUN cd /build/jpkg-src && \
    find . -name '*.o' -o -name '*.d' | xargs rm -f && rm -f jpkg && \
    make CC=clang LD=ld.lld \
    CFLAGS="-Os -pipe -fstack-protector-strong -D_FORTIFY_SOURCE=2" \
    LDFLAGS="-static -fuse-ld=lld" jpkg && \
    cp jpkg /build/jpkg && strip /build/jpkg
RUN cd /build/jpkg-src && make test

# --- Build Python 3.12 (PSF license) from source ---
RUN curl -fsSL https://www.python.org/ftp/python/3.12.8/Python-3.12.8.tar.xz -o python.tar.xz && \
    tar xf python.tar.xz && cd Python-3.12.8 && \
    ./configure \
        CC=clang \
        LD=ld.lld \
        CFLAGS="-Os -pipe" \
        LDFLAGS="-fuse-ld=lld" \
        --prefix=/usr \
        --without-ensurepip \
        --with-system-ffi=no \
        --with-openssl=/usr \
        --disable-test-modules \
    && make -j$(nproc) && \
    make install DESTDIR=/build/python-install && \
    # Strip to reduce size — remove test suites, idle, tkinter, pycache
    rm -rf /build/python-install/usr/lib/python3.12/test \
           /build/python-install/usr/lib/python3.12/idlelib \
           /build/python-install/usr/lib/python3.12/tkinter \
           /build/python-install/usr/lib/python3.12/turtledemo \
           /build/python-install/usr/lib/python3.12/ensurepip && \
    find /build/python-install -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true && \
    find /build/python-install -name '*.pyc' -delete 2>/dev/null || true && \
    strip /build/python-install/usr/bin/python3.12

# --- Build Node.js 22 LTS (MIT license) from source ---
# Node.js uses its bundled OpenSSL (Apache-2.0) and V8. Needs Python for GYP build.
COPY node-v22.12.0.tar.gz /build/node.tar.gz
RUN tar xf node.tar.gz && cd node-v22.12.0 && \
    # Configure: use bundled deps (all permissive), no npm (saves ~30MB), no intl
    CC=clang CXX=clang++ \
    python3 configure.py \
        --prefix=/usr \
        --ninja \
        --partly-static \
        --without-npm \
        --without-corepack \
        --with-intl=none \
        --shared-zlib \
    && make -j$(nproc) && \
    make install DESTDIR=/build/node-install && \
    strip /build/node-install/usr/bin/node

# ============================================================
# Stage 1: Assemble clean jonerix rootfs
# ============================================================
FROM alpine:latest AS assembler

WORKDIR /jonerix

# Create merged /usr filesystem layout (DESIGN.md section 5)
RUN mkdir -p \
    bin lib etc dev proc sys run tmp \
    etc/init.d etc/conf.d etc/ssl etc/jpkg etc/network \
    var/log var/cache/jpkg var/db/jpkg/installed \
    home root boot \
    && chmod 1777 tmp \
    && chmod 700 root \
    && ln -s / usr

# --- Core binaries ---
COPY --from=builder /build/toybox bin/toybox
COPY --from=builder /build/mksh bin/mksh
COPY --from=builder /build/jpkg bin/jpkg

# Create toybox applet symlinks
RUN for path in $(bin/toybox --long); do \
        cmd=$(basename "$path"); \
        [ "$cmd" != "toybox" ] && [ ! -e "bin/$cmd" ] && \
        ln -s toybox "bin/$cmd" || true; \
    done && \
    ln -sf mksh bin/sh

# --- pico editor (Apache-2.0) ---
COPY --from=builder /build/pico bin/pico

# --- Kernel build tools ---
COPY --from=builder /build/bmake-bin bin/bmake
COPY --from=builder /build/flex bin/flex
COPY --from=builder /build/perl bin/perl
COPY --from=builder /build/bc bin/bc
RUN ln -s bmake bin/make && \
    ln -s flex bin/lex

# --- dropbear SSH (MIT) ---
COPY --from=builder /build/dropbear bin/dropbear
COPY --from=builder /build/dbclient bin/dbclient
COPY --from=builder /build/dropbearkey bin/dropbearkey
COPY --from=builder /build/scp bin/scp
RUN ln -s dbclient bin/ssh && \
    mkdir -p etc/dropbear

# --- Python 3.12 (PSF license) ---
COPY --from=builder /build/python-install/usr/bin/python3.12 bin/python3.12
COPY --from=builder /build/python-install/usr/lib/python3.12/ lib/python3.12/
RUN ln -s python3.12 bin/python3 && ln -s python3 bin/python

# --- Node.js 22 (MIT license) ---
COPY --from=builder /build/node-install/usr/bin/node bin/node

# --- LLVM/Clang toolchain (Apache-2.0 w/ LLVM exception) --- v2
# Install from Alpine, then copy actual binary locations (Alpine uses /usr/lib/llvm21/)
RUN apk add --no-cache clang lld llvm compiler-rt musl-dev linux-headers && \
    # Find and copy actual clang binary (follows Alpine's versioned layout)
    CLANG_BIN=$(readlink -f /usr/bin/clang) && \
    cp "$CLANG_BIN" bin/clang && \
    ln -s clang bin/clang++ && \
    ln -s clang bin/cc && \
    ln -s clang bin/c++ && \
    # LLD linker (follow symlinks to real binary)
    LLD_BIN=$(readlink -f /usr/bin/ld.lld) && \
    cp "$LLD_BIN" bin/ld.lld && \
    ln -s ld.lld bin/ld && \
    # LLVM tools (follow symlinks)
    for tool in llvm-ar llvm-nm llvm-objdump llvm-objcopy llvm-strip \
                llvm-ranlib llvm-readelf llvm-size llvm-strings \
                llvm-symbolizer llvm-profdata llvm-cov llvm-as llvm-dis; do \
        if [ -e "/usr/bin/$tool" ]; then \
            REAL=$(readlink -f "/usr/bin/$tool"); \
            cp "$REAL" "bin/$tool"; \
        fi; \
    done && \
    ln -sf llvm-ar bin/ar && \
    ln -sf llvm-nm bin/nm && \
    ln -sf llvm-objdump bin/objdump && \
    ln -sf llvm-objcopy bin/objcopy && \
    ln -sf llvm-strip bin/strip && \
    ln -sf llvm-ranlib bin/ranlib && \
    ln -sf llvm-readelf bin/readelf && \
    # LLVM shared libs (Alpine puts them under /usr/lib/llvm21/lib/)
    find /usr/lib -name 'libLLVM*.so*' -exec cp {} lib/ \; && \
    find /usr/lib -name 'libclang*.so*' -exec cp {} lib/ \; && \
    find /usr/lib -name 'libLTO*.so*' -exec cp {} lib/ \; && \
    find /usr/lib -name 'liblld*.so*' -exec cp {} lib/ \; && \
    # Clang resource dir (builtins, headers) — needed for compilation
    mkdir -p lib/clang && \
    CLANG_RES=$(find /usr/lib -path '*/clang/*/include' -type d 2>/dev/null | head -1) && \
    if [ -n "$CLANG_RES" ]; then cp -r "$(dirname "$CLANG_RES")" lib/clang/; fi && \
    # Also copy the clang config file if it exists
    mkdir -p etc/clang && \
    cp /etc/clang*/*.cfg etc/clang/ 2>/dev/null || true && \
    # musl-dev headers (MIT) + linux headers + CRT objects + static libs
    mkdir -p include && \
    cp -r /usr/include/* include/ && \
    # CRT startup objects (musl) — needed for linking executables
    cp /usr/lib/Scrt1.o lib/ 2>/dev/null || true && \
    cp /usr/lib/crt1.o lib/ 2>/dev/null || true && \
    cp /usr/lib/crti.o lib/ 2>/dev/null || true && \
    cp /usr/lib/crtn.o lib/ 2>/dev/null || true && \
    cp /usr/lib/rcrt1.o lib/ 2>/dev/null || true && \
    # Static C library and GCC runtime
    cp /usr/lib/libc.a lib/ 2>/dev/null || true && \
    cp /usr/lib/libc.so lib/ 2>/dev/null || true && \
    cp /usr/lib/libm.a lib/ 2>/dev/null || true && \
    cp /usr/lib/libpthread.a lib/ 2>/dev/null || true && \
    cp /usr/lib/libdl.a lib/ 2>/dev/null || true && \
    cp /usr/lib/librt.a lib/ 2>/dev/null || true && \
    cp /usr/lib/libcrypt.a lib/ 2>/dev/null || true && \
    cp /usr/lib/libutil.a lib/ 2>/dev/null || true && \
    cp /usr/lib/libresolv.a lib/ 2>/dev/null || true && \
    # GCC/compiler-rt runtime objects
    find /usr/lib/gcc -name 'crtbeginS.o' -exec cp {} lib/ \; 2>/dev/null || true && \
    find /usr/lib/gcc -name 'crtendS.o' -exec cp {} lib/ \; 2>/dev/null || true && \
    find /usr/lib/gcc -name 'crtbegin.o' -exec cp {} lib/ \; 2>/dev/null || true && \
    find /usr/lib/gcc -name 'crtend.o' -exec cp {} lib/ \; 2>/dev/null || true && \
    cp /usr/lib/libssp_nonshared.a lib/ 2>/dev/null || true && \
    cp /usr/lib/libssp.a lib/ 2>/dev/null || true && \
    find /usr/lib/gcc -name 'libgcc.a' -exec cp {} lib/ \; 2>/dev/null || true && \
    find /usr/lib/gcc -name 'libgcc_eh.a' -exec cp {} lib/ \; 2>/dev/null || true

# --- Shared libraries (all permissive) ---
# Collect ALL needed .so files in one pass using ldd on key binaries
RUN cp /lib/ld-musl-*.so.1 lib/ && \
    for f in /lib/libc.musl-*.so.1; do cp "$f" lib/; done && \
    # Systematically copy all needed shared libs
    for pattern in \
        '/lib/libz.so*' '/usr/lib/libz.so*' \
        '/lib/libgcc_s.so*' '/usr/lib/libgcc_s.so*' \
        '/usr/lib/libstdc++.so*' \
        '/lib/libcrypto.so*' '/usr/lib/libcrypto.so*' \
        '/lib/libssl.so*' '/usr/lib/libssl.so*' \
        '/usr/lib/libffi.so*' '/lib/libffi.so*' \
        '/lib/libncursesw.so*' '/usr/lib/libncursesw.so*' \
        '/lib/libreadline.so*' '/usr/lib/libreadline.so*' \
        '/usr/lib/libxml2.so*' \
        '/usr/lib/libxar.so*' \
        '/lib/liblzma.so*' '/usr/lib/liblzma.so*' \
        '/usr/lib/libzstd.so*' '/lib/libzstd.so*' \
        '/usr/lib/libncursesw.so*' '/lib/libncursesw.so*' \
        '/usr/lib/libtinfow.so*' '/lib/libtinfow.so*' \
        '/usr/lib/libncurses.so*' '/lib/libncurses.so*' \
    ; do cp $pattern lib/ 2>/dev/null || true; done && \
    # Perl modules
    cp -r /usr/lib/perl5 lib/perl5 2>/dev/null || true

# --- Default configs ---
COPY config/defaults/etc/hostname etc/hostname
COPY config/defaults/etc/resolv.conf etc/resolv.conf
COPY config/defaults/etc/passwd etc/passwd
COPY config/defaults/etc/group etc/group
COPY config/defaults/etc/shadow etc/shadow
COPY config/defaults/etc/shells etc/shells
COPY config/defaults/etc/profile etc/profile
COPY config/defaults/etc/doas.conf etc/doas.conf

# --- OpenRC service scripts ---
COPY config/openrc/init.d/ etc/init.d/
RUN chmod 755 etc/init.d/* 2>/dev/null || true

# --- Package database ---
RUN printf '[package]\nname = "toybox"\nversion = "0.8.11"\nlicense = "0BSD"\n' > var/db/jpkg/installed/toybox && \
    printf '[package]\nname = "mksh"\nversion = "R59c"\nlicense = "MirOS"\n' > var/db/jpkg/installed/mksh && \
    printf '[package]\nname = "jpkg"\nversion = "0.1.0"\nlicense = "MIT"\n' > var/db/jpkg/installed/jpkg && \
    printf '[package]\nname = "musl"\nversion = "1.2.5"\nlicense = "MIT"\n' > var/db/jpkg/installed/musl && \
    printf '[package]\nname = "python3"\nversion = "3.12.8"\nlicense = "PSF-2.0"\n' > var/db/jpkg/installed/python3 && \
    printf '[package]\nname = "nodejs"\nversion = "22.12.0"\nlicense = "MIT"\n' > var/db/jpkg/installed/nodejs && \
    printf '[package]\nname = "zlib"\nversion = "1.3.1"\nlicense = "Zlib"\n' > var/db/jpkg/installed/zlib && \
    printf '[package]\nname = "pico"\nversion = "2.26"\nlicense = "Apache-2.0"\n' > var/db/jpkg/installed/pico && \
    printf '[package]\nname = "dropbear"\nversion = "2024.86"\nlicense = "MIT"\n' > var/db/jpkg/installed/dropbear && \
    printf '[package]\nname = "llvm"\nversion = "21.1.2"\nlicense = "Apache-2.0"\n' > var/db/jpkg/installed/llvm && \
    printf '[package]\nname = "clang"\nversion = "21.1.2"\nlicense = "Apache-2.0"\n' > var/db/jpkg/installed/clang && \
    printf '[package]\nname = "lld"\nversion = "21.1.2"\nlicense = "Apache-2.0"\n' > var/db/jpkg/installed/lld && \
    printf '[package]\nname = "bmake"\nversion = "20240808"\nlicense = "MIT"\n' > var/db/jpkg/installed/bmake && \
    printf '[package]\nname = "flex"\nversion = "2.6.4"\nlicense = "BSD-2-Clause"\n' > var/db/jpkg/installed/flex && \
    printf '[package]\nname = "perl"\nversion = "5.40"\nlicense = "Artistic-2.0"\n' > var/db/jpkg/installed/perl && \
    printf '[package]\nname = "bc"\nversion = "7.0.3"\nlicense = "BSD-2-Clause"\n' > var/db/jpkg/installed/bc

COPY scripts/license-audit.sh bin/license-audit
RUN chmod 755 bin/license-audit

# ============================================================
# Stage 2: Final image — FROM scratch, zero GPL runtime
# ============================================================
FROM scratch

COPY --from=assembler /jonerix/ /

ENV PATH=/bin
ENV HOME=/root
ENV SHELL=/bin/mksh

WORKDIR /root

ENTRYPOINT ["/bin/mksh"]
CMD ["-l"]
