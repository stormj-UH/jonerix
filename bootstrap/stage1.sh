#!/bin/sh
# jonerix Stage 1 — Cross-compile the Permissive World
#
# Using Alpine's build tools (GPL, throwaway), compile every jonerix
# component from source into a staging sysroot.  Build order follows
# dependency chains per DESIGN.md section 2.
#
# Build order:
#   1.  musl          (C library — everything links against this)
#   2.  zstd          (compression — needed by jpkg and kernel)
#   3.  lz4           (compression — needed by kernel)
#   4.  zlib          (compression — needed by Python, Node.js, pigz, many others)
#   5.  LibreSSL      (TLS — needed by curl, dropbear, Python, Node.js)
#   6.  toybox        (coreutils)
#   7.  mksh          (shell)
#   8.  samurai       (build tool)
#   9.  LLVM/Clang/lld/libc++/libc++abi (compiler + linker + C++ stdlib)
#   10. OpenRC        (init system)
#   11. dropbear      (SSH — needs LibreSSL)
#   12. curl          (HTTP client — needs LibreSSL)
#   13. dhcpcd        (DHCP)
#   14. unbound       (DNS resolver — needs LibreSSL)
#   15. doas          (privilege escalation)
#   16. socklog       (logging)
#   17. snooze        (cron)
#   18. mandoc        (man pages)
#   19. ifupdown-ng   (network config)
#   20. pigz          (parallel gzip — needs zlib)
#   21. nvi           (text editor)
#   22. Python 3      (needs: musl, zlib, LibreSSL, clang)
#   23. Node.js       (needs: musl, zlib, LibreSSL, clang, libc++, python3)
#
# jpkg and the Linux kernel are handled separately:
#   - jpkg is built from the jonerix repo itself (packages/jpkg/)
#   - Linux kernel is compiled but kept as a single blob under /boot
#
# SPDX-License-Identifier: MIT

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "${SCRIPT_DIR}/config.sh"

# =========================================================================
# Pre-flight checks
# =========================================================================

msg "Stage 1: Cross-compile the permissive world"
msg "Architecture: ${JONERIX_ARCH}"
msg "Sysroot:      ${SYSROOT}"
msg "Source dir:    ${SRCDIR}"
msg "Build dir:    ${BUILDDIR}"

if [ ! -d "${SYSROOT}" ]; then
    die "Sysroot not found at ${SYSROOT}. Run stage0.sh first."
fi

NPROC=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 1)
msg "Parallel jobs: ${NPROC}"

# Track build progress
PKG_NUM=0
progress() {
    PKG_NUM=$((PKG_NUM + 1))
    echo ""
    echo "================================================================"
    echo "  [${PKG_NUM}/${TOTAL_PACKAGES}] Building: $1"
    echo "================================================================"
    echo ""
}

# =========================================================================
# 1. musl — C library (everything links against this)
# =========================================================================
progress "musl ${MUSL_VERSION}"

fetch_source "${MUSL_SOURCE}" "${SRCDIR}/musl-${MUSL_VERSION}.tar.gz" "${MUSL_SHA256}"
extract_source "${SRCDIR}/musl-${MUSL_VERSION}.tar.gz" "musl-${MUSL_VERSION}"
apply_patches "${SRCDIR}/musl-${MUSL_VERSION}" "${PATCHDIR}/musl"

(
    cd "${SRCDIR}/musl-${MUSL_VERSION}"
    ./configure \
        --prefix="${SYSROOT}" \
        --syslibdir="${SYSROOT}/lib" \
        CC="${CC}" \
        CFLAGS="-Os -pipe -fPIC" \
        || die "musl configure failed"

    make -j"${NPROC}" || die "musl build failed"
    make install || die "musl install failed"
)

# Set up the sysroot so subsequent builds link against our musl
export CFLAGS="${CFLAGS} --sysroot=${SYSROOT}"
export LDFLAGS="${LDFLAGS} --sysroot=${SYSROOT} -L${SYSROOT}/lib"
export PKG_CONFIG_PATH="${SYSROOT}/lib/pkgconfig:${SYSROOT}/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="${SYSROOT}"

# =========================================================================
# 2. zstd — compression (needed by jpkg, kernel)
# =========================================================================
progress "zstd ${ZSTD_VERSION}"

fetch_source "${ZSTD_SOURCE}" "${SRCDIR}/zstd-${ZSTD_VERSION}.tar.gz" "${ZSTD_SHA256}"
extract_source "${SRCDIR}/zstd-${ZSTD_VERSION}.tar.gz" "zstd-${ZSTD_VERSION}"
apply_patches "${SRCDIR}/zstd-${ZSTD_VERSION}" "${PATCHDIR}/zstd"

(
    cd "${SRCDIR}/zstd-${ZSTD_VERSION}"
    make -j"${NPROC}" \
        CC="${CC}" \
        CFLAGS="${CFLAGS}" \
        LDFLAGS="${LDFLAGS}" \
        PREFIX="${SYSROOT}" \
        lib-mt || die "zstd lib build failed"

    make -j"${NPROC}" \
        CC="${CC}" \
        CFLAGS="${CFLAGS}" \
        LDFLAGS="${LDFLAGS}" \
        PREFIX="${SYSROOT}" \
        || die "zstd build failed"

    make PREFIX="${SYSROOT}" install || die "zstd install failed"
)

# =========================================================================
# 3. lz4 — compression (needed by kernel)
# =========================================================================
progress "lz4 ${LZ4_VERSION}"

fetch_source "${LZ4_SOURCE}" "${SRCDIR}/lz4-${LZ4_VERSION}.tar.gz" "${LZ4_SHA256}"
extract_source "${SRCDIR}/lz4-${LZ4_VERSION}.tar.gz" "lz4-${LZ4_VERSION}"
apply_patches "${SRCDIR}/lz4-${LZ4_VERSION}" "${PATCHDIR}/lz4"

(
    cd "${SRCDIR}/lz4-${LZ4_VERSION}"
    make -j"${NPROC}" \
        CC="${CC}" \
        CFLAGS="${CFLAGS}" \
        LDFLAGS="${LDFLAGS}" \
        PREFIX="${SYSROOT}" \
        || die "lz4 build failed"

    make PREFIX="${SYSROOT}" install || die "lz4 install failed"
)

# =========================================================================
# 4. zlib — compression library (needed by Python, Node.js, pigz, etc.)
# =========================================================================
progress "zlib ${ZLIB_VERSION}"

fetch_source "${ZLIB_SOURCE}" "${SRCDIR}/zlib-${ZLIB_VERSION}.tar.gz" "${ZLIB_SHA256}"
extract_source "${SRCDIR}/zlib-${ZLIB_VERSION}.tar.gz" "zlib-${ZLIB_VERSION}"
apply_patches "${SRCDIR}/zlib-${ZLIB_VERSION}" "${PATCHDIR}/zlib"

(
    cd "${SRCDIR}/zlib-${ZLIB_VERSION}"

    CC="${CC}" \
    CFLAGS="${CFLAGS}" \
    LDFLAGS="${LDFLAGS}" \
    ./configure \
        --prefix="${SYSROOT}" \
        --static \
        || die "zlib configure failed"

    make -j"${NPROC}" || die "zlib build failed"
    make install || die "zlib install failed"
)

# =========================================================================
# 5. LibreSSL — TLS (needed by curl, dropbear, unbound)
# =========================================================================
progress "LibreSSL ${LIBRESSL_VERSION}"

fetch_source "${LIBRESSL_SOURCE}" "${SRCDIR}/libressl-${LIBRESSL_VERSION}.tar.gz" "${LIBRESSL_SHA256}"
extract_source "${SRCDIR}/libressl-${LIBRESSL_VERSION}.tar.gz" "libressl-${LIBRESSL_VERSION}"
apply_patches "${SRCDIR}/libressl-${LIBRESSL_VERSION}" "${PATCHDIR}/libressl"

(
    _builddir="${BUILDDIR}/libressl"
    mkdir -p "$_builddir"
    cd "$_builddir"

    cmake "${SRCDIR}/libressl-${LIBRESSL_VERSION}" \
        -G Ninja \
        -DCMAKE_C_COMPILER="${CC}" \
        -DCMAKE_C_FLAGS="${CFLAGS}" \
        -DCMAKE_EXE_LINKER_FLAGS="${LDFLAGS}" \
        -DCMAKE_INSTALL_PREFIX="${SYSROOT}" \
        -DLIBRESSL_APPS=ON \
        -DLIBRESSL_TESTS=OFF \
        -DBUILD_SHARED_LIBS=OFF \
        || die "LibreSSL cmake failed"

    samu -j"${NPROC}" || die "LibreSSL build failed"
    samu install || die "LibreSSL install failed"
)

# =========================================================================
# 6. toybox — coreutils (ls, cp, cat, grep, sed, awk, tar, ...)
# =========================================================================
progress "toybox ${TOYBOX_VERSION}"

fetch_source "${TOYBOX_SOURCE}" "${SRCDIR}/toybox-${TOYBOX_VERSION}.tar.gz" "${TOYBOX_SHA256}"
extract_source "${SRCDIR}/toybox-${TOYBOX_VERSION}.tar.gz" "toybox-${TOYBOX_VERSION}"
apply_patches "${SRCDIR}/toybox-${TOYBOX_VERSION}" "${PATCHDIR}/toybox"

(
    cd "${SRCDIR}/toybox-${TOYBOX_VERSION}"

    # If a jonerix toybox config exists, use it; otherwise use defconfig
    if [ -f "${PATCHDIR}/toybox/toybox.config" ]; then
        cp "${PATCHDIR}/toybox/toybox.config" .config
    else
        make defconfig
    fi

    make -j"${NPROC}" \
        CC="${CC}" \
        CFLAGS="${CFLAGS}" \
        LDFLAGS="${LDFLAGS} -static" \
        || die "toybox build failed"

    # Install toybox and its symlinks
    make PREFIX="${SYSROOT}" install || die "toybox install failed"
)

# =========================================================================
# 7. mksh — shell
# =========================================================================
progress "mksh ${MKSH_VERSION}"

fetch_source "${MKSH_SOURCE}" "${SRCDIR}/mksh-${MKSH_VERSION}.tgz" "${MKSH_SHA256}"
extract_source "${SRCDIR}/mksh-${MKSH_VERSION}.tgz" "mksh"
apply_patches "${SRCDIR}/mksh" "${PATCHDIR}/mksh"

(
    cd "${SRCDIR}/mksh"

    # mksh uses its own Build.sh script
    CC="${CC}" \
    CFLAGS="${CFLAGS}" \
    LDFLAGS="${LDFLAGS} -static" \
    CPPFLAGS="-DMKSH_ASSUME_UTF8=1" \
    sh Build.sh -r || die "mksh build failed"

    install -Dm755 mksh "${SYSROOT}/bin/mksh"
    install -Dm644 mksh.1 "${SYSROOT}/share/man/man1/mksh.1"

    # Also install as /bin/sh
    ln -sf mksh "${SYSROOT}/bin/sh"
)

# =========================================================================
# 8. samurai — ninja-compatible build tool (Apache-2.0)
# =========================================================================
progress "samurai ${SAMURAI_VERSION}"

fetch_source "${SAMURAI_SOURCE}" "${SRCDIR}/samurai-${SAMURAI_VERSION}.tar.gz" "${SAMURAI_SHA256}"
extract_source "${SRCDIR}/samurai-${SAMURAI_VERSION}.tar.gz" "samurai-${SAMURAI_VERSION}"
apply_patches "${SRCDIR}/samurai-${SAMURAI_VERSION}" "${PATCHDIR}/samurai"

(
    cd "${SRCDIR}/samurai-${SAMURAI_VERSION}"

    ${CC} ${CFLAGS} ${LDFLAGS} -static -o samu *.c \
        || die "samurai build failed"

    install -Dm755 samu "${SYSROOT}/bin/samu"
    ln -sf samu "${SYSROOT}/bin/ninja"
)

# =========================================================================
# 9. LLVM/Clang/lld/libc++/libc++abi — compiler + linker + C++ stdlib
# =========================================================================
progress "LLVM/Clang/lld/libc++/libc++abi ${LLVM_VERSION}"

fetch_source "${LLVM_SOURCE}" "${SRCDIR}/llvm-project-${LLVM_VERSION}.src.tar.xz" "${LLVM_SHA256}"
extract_source "${SRCDIR}/llvm-project-${LLVM_VERSION}.src.tar.xz" "llvm-project-${LLVM_VERSION}.src"
apply_patches "${SRCDIR}/llvm-project-${LLVM_VERSION}.src" "${PATCHDIR}/llvm"

(
    _builddir="${BUILDDIR}/llvm"
    mkdir -p "$_builddir"
    cd "$_builddir"

    # Determine target triple
    case "${JONERIX_ARCH}" in
        x86_64)  _triple="x86_64-unknown-linux-musl"  ; _targets="X86"    ;;
        aarch64) _triple="aarch64-unknown-linux-musl"  ; _targets="AArch64" ;;
        *)       die "Unsupported arch for LLVM: ${JONERIX_ARCH}" ;;
    esac

    cmake "${SRCDIR}/llvm-project-${LLVM_VERSION}.src/llvm" \
        -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_C_COMPILER="${CC}" \
        -DCMAKE_CXX_COMPILER="clang++" \
        -DCMAKE_INSTALL_PREFIX="${SYSROOT}" \
        -DCMAKE_C_FLAGS="-Os -pipe" \
        -DCMAKE_CXX_FLAGS="-Os -pipe" \
        -DCMAKE_EXE_LINKER_FLAGS="-fuse-ld=lld" \
        -DLLVM_ENABLE_PROJECTS="clang;lld" \
        -DLLVM_ENABLE_RUNTIMES="compiler-rt;libcxx;libcxxabi" \
        -DLLVM_TARGETS_TO_BUILD="${_targets}" \
        -DLLVM_DEFAULT_TARGET_TRIPLE="${_triple}" \
        -DLLVM_HOST_TRIPLE="${_triple}" \
        -DLLVM_BUILD_LLVM_DYLIB=OFF \
        -DLLVM_LINK_LLVM_DYLIB=OFF \
        -DLLVM_BUILD_TOOLS=ON \
        -DLLVM_INCLUDE_TESTS=OFF \
        -DLLVM_INCLUDE_EXAMPLES=OFF \
        -DLLVM_INCLUDE_BENCHMARKS=OFF \
        -DLLVM_INCLUDE_DOCS=OFF \
        -DLLVM_ENABLE_BINDINGS=OFF \
        -DLLVM_ENABLE_OCAMLDOC=OFF \
        -DLLVM_ENABLE_Z3_SOLVER=OFF \
        -DLLVM_ENABLE_TERMINFO=OFF \
        -DLLVM_ENABLE_LIBXML2=OFF \
        -DLLVM_ENABLE_LIBEDIT=OFF \
        -DCLANG_DEFAULT_CXX_STDLIB="libc++" \
        -DCLANG_DEFAULT_LINKER="lld" \
        -DCLANG_DEFAULT_RTLIB="compiler-rt" \
        -DLIBCXX_HAS_MUSL_LIBC=ON \
        -DLIBCXX_USE_COMPILER_RT=ON \
        -DLIBCXX_ENABLE_STATIC=ON \
        -DLIBCXX_ENABLE_SHARED=OFF \
        -DLIBCXXABI_USE_COMPILER_RT=ON \
        -DLIBCXXABI_USE_LLVM_UNWINDER=OFF \
        -DLIBCXXABI_ENABLE_STATIC=ON \
        -DLIBCXXABI_ENABLE_SHARED=OFF \
        || die "LLVM cmake failed"

    samu -j"${NPROC}" || die "LLVM build failed"
    samu install || die "LLVM install failed"
)

# =========================================================================
# 10. OpenRC — init system
# =========================================================================
progress "OpenRC ${OPENRC_VERSION}"

fetch_source "${OPENRC_SOURCE}" "${SRCDIR}/openrc-${OPENRC_VERSION}.tar.gz" "${OPENRC_SHA256}"
extract_source "${SRCDIR}/openrc-${OPENRC_VERSION}.tar.gz" "openrc-${OPENRC_VERSION}"
apply_patches "${SRCDIR}/openrc-${OPENRC_VERSION}" "${PATCHDIR}/openrc"

(
    cd "${SRCDIR}/openrc-${OPENRC_VERSION}"

    # OpenRC uses meson; fall back to make if available
    if [ -f meson.build ]; then
        _builddir="${BUILDDIR}/openrc"
        mkdir -p "$_builddir"

        # Use meson + samu
        meson setup "$_builddir" \
            --prefix="${SYSROOT}" \
            --sysconfdir="${SYSROOT}/etc" \
            --localstatedir="${SYSROOT}/var" \
            -Dos=Linux \
            -Ddefault_library=static \
            -Dpam=false \
            -Daudit=disabled \
            -Dselinux=disabled \
            -Dtermcap=ncurses \
            || die "OpenRC meson setup failed"

        samu -C "$_builddir" -j"${NPROC}" || die "OpenRC build failed"
        samu -C "$_builddir" install || die "OpenRC install failed"
    else
        make -j"${NPROC}" \
            CC="${CC}" \
            CFLAGS="${CFLAGS}" \
            LDFLAGS="${LDFLAGS}" \
            DESTDIR="${SYSROOT}" \
            MKPAM=no \
            MKAUDIT=no \
            MKSELINUX=no \
            MKTERMCAP=ncurses \
            || die "OpenRC build failed"

        make DESTDIR="${SYSROOT}" install || die "OpenRC install failed"
    fi
)

# =========================================================================
# 11. dropbear — SSH server (needs LibreSSL)
# =========================================================================
progress "dropbear ${DROPBEAR_VERSION}"

fetch_source "${DROPBEAR_SOURCE}" "${SRCDIR}/dropbear-${DROPBEAR_VERSION}.tar.bz2" "${DROPBEAR_SHA256}"
extract_source "${SRCDIR}/dropbear-${DROPBEAR_VERSION}.tar.bz2" "dropbear-${DROPBEAR_VERSION}"
apply_patches "${SRCDIR}/dropbear-${DROPBEAR_VERSION}" "${PATCHDIR}/dropbear"

(
    cd "${SRCDIR}/dropbear-${DROPBEAR_VERSION}"

    ./configure \
        --prefix="${SYSROOT}" \
        --host="${JONERIX_ARCH}-linux-musl" \
        --disable-zlib \
        --disable-wtmp \
        --disable-lastlog \
        CC="${CC}" \
        CFLAGS="${CFLAGS} -I${SYSROOT}/include" \
        LDFLAGS="${LDFLAGS} -L${SYSROOT}/lib -static" \
        || die "dropbear configure failed"

    make -j"${NPROC}" PROGRAMS="dropbear dbclient dropbearkey scp" \
        || die "dropbear build failed"

    make PROGRAMS="dropbear dbclient dropbearkey scp" install \
        || die "dropbear install failed"
)

# =========================================================================
# 12. curl — HTTP client (needs LibreSSL)
# =========================================================================
progress "curl ${CURL_VERSION}"

fetch_source "${CURL_SOURCE}" "${SRCDIR}/curl-${CURL_VERSION}.tar.xz" "${CURL_SHA256}"
extract_source "${SRCDIR}/curl-${CURL_VERSION}.tar.xz" "curl-${CURL_VERSION}"
apply_patches "${SRCDIR}/curl-${CURL_VERSION}" "${PATCHDIR}/curl"

(
    cd "${SRCDIR}/curl-${CURL_VERSION}"

    ./configure \
        --prefix="${SYSROOT}" \
        --host="${JONERIX_ARCH}-linux-musl" \
        --with-openssl="${SYSROOT}" \
        --without-libpsl \
        --without-brotli \
        --without-nghttp2 \
        --without-libidn2 \
        --without-libssh2 \
        --without-zstd \
        --disable-shared \
        --enable-static \
        --disable-ldap \
        --disable-rtsp \
        --disable-dict \
        --disable-telnet \
        --disable-pop3 \
        --disable-imap \
        --disable-smb \
        --disable-smtp \
        --disable-gopher \
        --disable-mqtt \
        --disable-manual \
        CC="${CC}" \
        CFLAGS="${CFLAGS} -I${SYSROOT}/include" \
        LDFLAGS="${LDFLAGS} -L${SYSROOT}/lib -static" \
        LIBS="-lssl -lcrypto" \
        || die "curl configure failed"

    make -j"${NPROC}" || die "curl build failed"
    make install || die "curl install failed"
)

# =========================================================================
# 13. dhcpcd — DHCP client
# =========================================================================
progress "dhcpcd ${DHCPCD_VERSION}"

fetch_source "${DHCPCD_SOURCE}" "${SRCDIR}/dhcpcd-${DHCPCD_VERSION}.tar.xz" "${DHCPCD_SHA256}"
extract_source "${SRCDIR}/dhcpcd-${DHCPCD_VERSION}.tar.xz" "dhcpcd-${DHCPCD_VERSION}"
apply_patches "${SRCDIR}/dhcpcd-${DHCPCD_VERSION}" "${PATCHDIR}/dhcpcd"

(
    cd "${SRCDIR}/dhcpcd-${DHCPCD_VERSION}"

    ./configure \
        --prefix="${SYSROOT}" \
        --sysconfdir="${SYSROOT}/etc" \
        --libexecdir="${SYSROOT}/lib/dhcpcd" \
        --dbdir="${SYSROOT}/var/db/dhcpcd" \
        --rundir=/run/dhcpcd \
        --without-udev \
        CC="${CC}" \
        CFLAGS="${CFLAGS} -I${SYSROOT}/include" \
        LDFLAGS="${LDFLAGS} -L${SYSROOT}/lib -static" \
        || die "dhcpcd configure failed"

    make -j"${NPROC}" || die "dhcpcd build failed"
    make install || die "dhcpcd install failed"
)

# =========================================================================
# 14. unbound — DNS resolver (needs LibreSSL)
# =========================================================================
progress "unbound ${UNBOUND_VERSION}"

fetch_source "${UNBOUND_SOURCE}" "${SRCDIR}/unbound-${UNBOUND_VERSION}.tar.gz" "${UNBOUND_SHA256}"
extract_source "${SRCDIR}/unbound-${UNBOUND_VERSION}.tar.gz" "unbound-${UNBOUND_VERSION}"
apply_patches "${SRCDIR}/unbound-${UNBOUND_VERSION}" "${PATCHDIR}/unbound"

(
    cd "${SRCDIR}/unbound-${UNBOUND_VERSION}"

    ./configure \
        --prefix="${SYSROOT}" \
        --sysconfdir="${SYSROOT}/etc" \
        --with-ssl="${SYSROOT}" \
        --with-libevent=no \
        --with-libunbound-only \
        --enable-static-exe \
        --disable-shared \
        --disable-flto \
        CC="${CC}" \
        CFLAGS="${CFLAGS} -I${SYSROOT}/include" \
        LDFLAGS="${LDFLAGS} -L${SYSROOT}/lib -static" \
        || die "unbound configure failed"

    make -j"${NPROC}" || die "unbound build failed"
    make install || die "unbound install failed"
)

# =========================================================================
# 15. doas — privilege escalation
# =========================================================================
progress "doas ${DOAS_VERSION}"

fetch_source "${DOAS_SOURCE}" "${SRCDIR}/opendoas-${DOAS_VERSION}.tar.xz" "${DOAS_SHA256}"
extract_source "${SRCDIR}/opendoas-${DOAS_VERSION}.tar.xz" "opendoas-${DOAS_VERSION}"
apply_patches "${SRCDIR}/opendoas-${DOAS_VERSION}" "${PATCHDIR}/doas"

(
    cd "${SRCDIR}/opendoas-${DOAS_VERSION}"

    ./configure \
        --prefix="${SYSROOT}" \
        --with-timestamp \
        CC="${CC}" \
        CFLAGS="${CFLAGS}" \
        LDFLAGS="${LDFLAGS} -static" \
        || die "doas configure failed"

    make -j"${NPROC}" || die "doas build failed"
    make install || die "doas install failed"
)

# =========================================================================
# 16. socklog — logging
# =========================================================================
progress "socklog ${SOCKLOG_VERSION}"

fetch_source "${SOCKLOG_SOURCE}" "${SRCDIR}/socklog-${SOCKLOG_VERSION}.tar.gz" "${SOCKLOG_SHA256}"
extract_source "${SRCDIR}/socklog-${SOCKLOG_VERSION}.tar.gz" "admin/socklog-${SOCKLOG_VERSION}"
apply_patches "${SRCDIR}/admin/socklog-${SOCKLOG_VERSION}" "${PATCHDIR}/socklog"

(
    cd "${SRCDIR}/admin/socklog-${SOCKLOG_VERSION}"

    # socklog uses DJB-style build system
    echo "${CC} ${CFLAGS}" > src/conf-cc
    echo "${CC} ${LDFLAGS} -static" > src/conf-ld

    cd src
    make || die "socklog build failed"

    # Install binaries
    for _bin in socklog socklog-check; do
        if [ -f "$_bin" ]; then
            install -Dm755 "$_bin" "${SYSROOT}/bin/$_bin"
        fi
    done
)

# =========================================================================
# 17. snooze — cron (public domain)
# =========================================================================
progress "snooze ${SNOOZE_VERSION}"

fetch_source "${SNOOZE_SOURCE}" "${SRCDIR}/snooze-${SNOOZE_VERSION}.tar.gz" "${SNOOZE_SHA256}"
extract_source "${SRCDIR}/snooze-${SNOOZE_VERSION}.tar.gz" "snooze-${SNOOZE_VERSION}"
apply_patches "${SRCDIR}/snooze-${SNOOZE_VERSION}" "${PATCHDIR}/snooze"

(
    cd "${SRCDIR}/snooze-${SNOOZE_VERSION}"

    make -j"${NPROC}" \
        CC="${CC}" \
        CFLAGS="${CFLAGS}" \
        LDFLAGS="${LDFLAGS} -static" \
        || die "snooze build failed"

    install -Dm755 snooze "${SYSROOT}/bin/snooze"
    if [ -f snooze.1 ]; then
        install -Dm644 snooze.1 "${SYSROOT}/share/man/man1/snooze.1"
    fi
)

# =========================================================================
# 18. mandoc — man pages
# =========================================================================
progress "mandoc ${MANDOC_VERSION}"

fetch_source "${MANDOC_SOURCE}" "${SRCDIR}/mandoc-${MANDOC_VERSION}.tar.gz" "${MANDOC_SHA256}"
extract_source "${SRCDIR}/mandoc-${MANDOC_VERSION}.tar.gz" "mandoc-${MANDOC_VERSION}"
apply_patches "${SRCDIR}/mandoc-${MANDOC_VERSION}" "${PATCHDIR}/mandoc"

(
    cd "${SRCDIR}/mandoc-${MANDOC_VERSION}"

    # mandoc uses configure.local for build settings
    cat > configure.local <<CONFEOF
PREFIX=${SYSROOT}
MANDIR=${SYSROOT}/share/man
CC=${CC}
CFLAGS=${CFLAGS}
LDFLAGS=${LDFLAGS} -static
BUILD_CGI=0
INSTALL_LIBMANDOC=0
UTF8_LOCALE=C.UTF-8
CONFEOF

    ./configure || die "mandoc configure failed"
    make -j"${NPROC}" || die "mandoc build failed"
    make install || die "mandoc install failed"
)

# =========================================================================
# 19. ifupdown-ng — network config
# =========================================================================
progress "ifupdown-ng ${IFUPDOWN_NG_VERSION}"

fetch_source "${IFUPDOWN_NG_SOURCE}" "${SRCDIR}/ifupdown-ng-${IFUPDOWN_NG_VERSION}.tar.xz" "${IFUPDOWN_NG_SHA256}"
extract_source "${SRCDIR}/ifupdown-ng-${IFUPDOWN_NG_VERSION}.tar.xz" "ifupdown-ng-${IFUPDOWN_NG_VERSION}"
apply_patches "${SRCDIR}/ifupdown-ng-${IFUPDOWN_NG_VERSION}" "${PATCHDIR}/ifupdown-ng"

(
    cd "${SRCDIR}/ifupdown-ng-${IFUPDOWN_NG_VERSION}"

    make -j"${NPROC}" \
        CC="${CC}" \
        CFLAGS="${CFLAGS}" \
        LDFLAGS="${LDFLAGS} -static" \
        PREFIX="${SYSROOT}" \
        SYSCONFDIR="${SYSROOT}/etc" \
        || die "ifupdown-ng build failed"

    make PREFIX="${SYSROOT}" SYSCONFDIR="${SYSROOT}/etc" install \
        || die "ifupdown-ng install failed"
)

# =========================================================================
# 20. pigz — parallel gzip (zlib license, needs zlib)
# =========================================================================
progress "pigz ${PIGZ_VERSION}"

fetch_source "${PIGZ_SOURCE}" "${SRCDIR}/pigz-${PIGZ_VERSION}.tar.gz" "${PIGZ_SHA256}"
extract_source "${SRCDIR}/pigz-${PIGZ_VERSION}.tar.gz" "pigz-${PIGZ_VERSION}"
apply_patches "${SRCDIR}/pigz-${PIGZ_VERSION}" "${PATCHDIR}/pigz"

(
    cd "${SRCDIR}/pigz-${PIGZ_VERSION}"

    make -j"${NPROC}" \
        CC="${CC}" \
        CFLAGS="${CFLAGS}" \
        LDFLAGS="${LDFLAGS} -static -lz" \
        || die "pigz build failed"

    install -Dm755 pigz "${SYSROOT}/bin/pigz"
    ln -sf pigz "${SYSROOT}/bin/unpigz"
    ln -sf pigz "${SYSROOT}/bin/gzip"
    ln -sf pigz "${SYSROOT}/bin/gunzip"
)

# =========================================================================
# 21. nvi — text editor
# =========================================================================
progress "nvi ${NVI_VERSION}"

fetch_source "${NVI_SOURCE}" "${SRCDIR}/nvi2-${NVI_VERSION}.tar.gz" "${NVI_SHA256}"
extract_source "${SRCDIR}/nvi2-${NVI_VERSION}.tar.gz" "nvi2-${NVI_VERSION}"
apply_patches "${SRCDIR}/nvi2-${NVI_VERSION}" "${PATCHDIR}/nvi"

(
    _builddir="${BUILDDIR}/nvi"
    mkdir -p "$_builddir"
    cd "$_builddir"

    cmake "${SRCDIR}/nvi2-${NVI_VERSION}" \
        -G Ninja \
        -DCMAKE_C_COMPILER="${CC}" \
        -DCMAKE_C_FLAGS="${CFLAGS}" \
        -DCMAKE_EXE_LINKER_FLAGS="${LDFLAGS} -static" \
        -DCMAKE_INSTALL_PREFIX="${SYSROOT}" \
        || die "nvi cmake failed"

    samu -j"${NPROC}" || die "nvi build failed"
    samu install || die "nvi install failed"

    # Provide vi symlink
    ln -sf nvi "${SYSROOT}/bin/vi"
)

# =========================================================================
# 22. Python 3 — interpreter + build dependency for Node.js
# =========================================================================
progress "Python ${PYTHON_VERSION}"

fetch_source "${PYTHON_SOURCE}" "${SRCDIR}/Python-${PYTHON_VERSION}.tar.xz" "${PYTHON_SHA256}"
extract_source "${SRCDIR}/Python-${PYTHON_VERSION}.tar.xz" "Python-${PYTHON_VERSION}"
apply_patches "${SRCDIR}/Python-${PYTHON_VERSION}" "${PATCHDIR}/python3"

(
    cd "${SRCDIR}/Python-${PYTHON_VERSION}"

    # Python's configure needs to find zlib and LibreSSL
    CC="${CC}" \
    CXX="clang++" \
    CFLAGS="${CFLAGS} -I${SYSROOT}/include" \
    LDFLAGS="${LDFLAGS} -L${SYSROOT}/lib" \
    CPPFLAGS="-I${SYSROOT}/include" \
    ./configure \
        --prefix="${SYSROOT}" \
        --without-ensurepip \
        --with-openssl="${SYSROOT}" \
        --with-system-ffi=no \
        --with-lto \
        --disable-shared \
        --enable-ipv6 \
        --with-computed-gotos \
        --with-system-expat=no \
        --with-ssl-default-suites=openssl \
        --disable-test-modules \
        ac_cv_file__dev_ptmx=yes \
        ac_cv_file__dev_ptc=no \
        || die "Python configure failed"

    make -j"${NPROC}" || die "Python build failed"
    make install || die "Python install failed"

    # Provide python3 -> python symlink
    ln -sf python3 "${SYSROOT}/bin/python"

    # Remove test suite and unnecessary files to save space
    rm -rf "${SYSROOT}/lib/python3.12/test"
    rm -rf "${SYSROOT}/lib/python3.12/unittest/test"
    rm -rf "${SYSROOT}/lib/python3.12/lib2to3/tests"
    rm -rf "${SYSROOT}/lib/python3.12/idlelib"
    rm -rf "${SYSROOT}/lib/python3.12/tkinter"
    rm -rf "${SYSROOT}/lib/python3.12/turtledemo"
    find "${SYSROOT}/lib/python3.12" -name '__pycache__' -type d -exec rm -rf {} + 2>/dev/null || true
)

# =========================================================================
# 23. Node.js — JavaScript runtime (needs Python 3, libc++, zlib, LibreSSL)
# =========================================================================
progress "Node.js ${NODEJS_VERSION}"

fetch_source "${NODEJS_SOURCE}" "${SRCDIR}/node-v${NODEJS_VERSION}.tar.gz" "${NODEJS_SHA256}"
extract_source "${SRCDIR}/node-v${NODEJS_VERSION}.tar.gz" "node-v${NODEJS_VERSION}"
apply_patches "${SRCDIR}/node-v${NODEJS_VERSION}" "${PATCHDIR}/nodejs"

(
    cd "${SRCDIR}/node-v${NODEJS_VERSION}"

    # Node.js/V8 needs libc++ instead of libstdc++ to avoid GPL dependency.
    # musl compatibility notes:
    #   - V8 uses execinfo.h (backtrace) which musl does not provide.
    #     Node.js 22.x includes stubs for musl; patches may still be needed
    #     for older or less common code paths.
    #   - Set PYTHON to the just-built Python 3 for the GYP build system.

    CC="${CC}" \
    CXX="clang++ -stdlib=libc++" \
    CXXFLAGS="-stdlib=libc++ -I${SYSROOT}/include/c++/v1 -I${SYSROOT}/include" \
    LDFLAGS="${LDFLAGS} -L${SYSROOT}/lib -lc++ -lc++abi" \
    PYTHON="${SYSROOT}/bin/python3" \
    ./configure \
        --prefix="${SYSROOT}" \
        --ninja \
        --partly-static \
        --shared-openssl \
        --shared-openssl-libpath="${SYSROOT}/lib" \
        --shared-openssl-includes="${SYSROOT}/include" \
        --shared-zlib \
        --shared-zlib-libpath="${SYSROOT}/lib" \
        --shared-zlib-includes="${SYSROOT}/include" \
        --with-intl=none \
        --openssl-use-def-ca-store \
        --without-npm \
        --without-corepack \
        || die "Node.js configure failed"

    make -j"${NPROC}" || die "Node.js build failed"
    make install || die "Node.js install failed"

    # Remove unnecessary files to save space
    rm -rf "${SYSROOT}/include/node"
    rm -rf "${SYSROOT}/share/doc/node"
)

# =========================================================================
# jpkg — package manager (built from jonerix source tree)
# =========================================================================

msg "Note: jpkg will be built from packages/jpkg/ when that source is ready."
msg "Placeholder: creating jpkg stub for rootfs assembly."

if [ -d "${SCRIPT_DIR}/../packages/jpkg/src" ] && [ -f "${SCRIPT_DIR}/../packages/jpkg/Makefile" ]; then
    msg "Building jpkg from source tree..."
    (
        cd "${SCRIPT_DIR}/../packages/jpkg"
        make CC="${CC}" CFLAGS="${CFLAGS}" LDFLAGS="${LDFLAGS} -static" \
            PREFIX="${SYSROOT}" install \
            || die "jpkg build failed"
    )
else
    msg "jpkg source not yet available — skipping (will be built in a later task)"
fi

# =========================================================================
# Linux kernel (compiled but installed as blob under /boot)
# =========================================================================

msg "Building Linux kernel ${LINUX_VERSION}..."

fetch_source "${LINUX_SOURCE}" "${SRCDIR}/linux-${LINUX_VERSION}.tar.xz" "${LINUX_SHA256}"
extract_source "${SRCDIR}/linux-${LINUX_VERSION}.tar.xz" "linux-${LINUX_VERSION}"

(
    cd "${SRCDIR}/linux-${LINUX_VERSION}"

    # Use jonerix kernel config if available, otherwise defconfig
    _kconfig="${SCRIPT_DIR}/../config/kernel/${JONERIX_ARCH}.config"
    if [ -f "$_kconfig" ]; then
        cp "$_kconfig" .config
        make olddefconfig || die "kernel olddefconfig failed"
    else
        msg "No custom kernel config found, using defconfig"
        make defconfig || die "kernel defconfig failed"
    fi

    make -j"${NPROC}" \
        CC="${CC}" \
        LD="${LD}" \
        AR="${AR}" \
        NM="${NM}" \
        OBJCOPY="${OBJCOPY}" \
        LLVM=1 \
        || die "kernel build failed"

    # Install kernel image
    mkdir -p "${SYSROOT}/boot"
    case "${JONERIX_ARCH}" in
        x86_64)
            cp arch/x86/boot/bzImage "${SYSROOT}/boot/vmlinuz"
            ;;
        aarch64)
            cp arch/arm64/boot/Image "${SYSROOT}/boot/vmlinuz"
            ;;
    esac

    # Install kernel modules (minimal set)
    make modules_install \
        INSTALL_MOD_PATH="${SYSROOT}" \
        INSTALL_MOD_STRIP=1 \
        || msg "Warning: kernel module install skipped (may be built-in)"
)

# =========================================================================
# Stage 1 complete
# =========================================================================

echo ""
echo "================================================================"
echo "  Stage 1 COMPLETE"
echo "================================================================"
echo ""
msg "Sysroot populated at: ${SYSROOT}"
msg "Sysroot size: $(du -sh "${SYSROOT}" | cut -f1)"
msg ""
msg "Installed components:"
for _bin in \
    musl-gcc toybox mksh sh samu clang lld dropbear curl \
    dhcpcd unbound doas socklog snooze mandoc ifup pigz nvi vi \
    python3 node; do
    if [ -f "${SYSROOT}/bin/${_bin}" ] || [ -L "${SYSROOT}/bin/${_bin}" ]; then
        printf "  %-20s OK\n" "$_bin"
    else
        printf "  %-20s MISSING\n" "$_bin"
    fi
done

msg ""
msg "Next step: sh bootstrap/stage2.sh"
