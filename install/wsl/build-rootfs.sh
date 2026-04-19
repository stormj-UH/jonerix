#!/bin/sh
# install/wsl/build-rootfs.sh
#
# Build a jonerix WSL2 rootfs tarball inside the current Alpine container.
# Produces one of:
#   jonerix-rootfs-aarch64.tar.gz
#   jonerix-rootfs-x86_64.tar.gz
#
# The resulting rootfs mirrors what a fresh Pi install would see: mksh as
# /bin/sh, toybox coreutils, dropbear, micro, fastfetch, libressl, curl,
# ripgrep, mandoc — everything a user expects on first login. WSL-specific
# bits (wsl.conf, no-unbound resolv.conf) are layered on top.
#
# Usage (run as root inside Alpine):
#   sh build-rootfs.sh
#
# Environment variables:
#   ARCH        — target architecture (default: host's uname -m)
#   GITHUB_REPO — GitHub repository slug (default: stormj-UH/jonerix)
#   PKG_RELEASE — GitHub release tag for packages (default: packages)
#   STAGING     — staging directory (default: /tmp/jonerix-rootfs)
#   OUTPUT_DIR  — where to write the tarball (default: current directory)

set -e

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
ARCH="${ARCH:-$(uname -m)}"
GITHUB_REPO="${GITHUB_REPO:-stormj-UH/jonerix}"
PKG_RELEASE="${PKG_RELEASE:-packages}"
STAGING="${STAGING:-/tmp/jonerix-rootfs}"
OUTPUT_DIR="${OUTPUT_DIR:-$(pwd)}"
PKG_BASE_URL="https://github.com/${GITHUB_REPO}/releases/download/${PKG_RELEASE}"

echo "=== Building jonerix WSL rootfs for ${ARCH} ==="
echo "    Package repo : ${PKG_BASE_URL}"
echo "    Staging dir  : ${STAGING}"
echo "    Output dir   : ${OUTPUT_DIR}"

# ---------------------------------------------------------------------------
# 1. Install host-side build tools
# ---------------------------------------------------------------------------
echo "--- Installing host build tools ---"
apk add --no-cache curl zstd tar libarchive-tools ca-certificates
# jpkg archives embed symlinks that toybox/BusyBox tar mishandle.
# Point /usr/bin/tar at bsdtar for the duration of the build.
ln -sf bsdtar /usr/bin/tar

# ---------------------------------------------------------------------------
# 2. Build the jpkg binary if not already present
# ---------------------------------------------------------------------------
if ! command -v jpkg > /dev/null 2>&1; then
    echo "--- Building jpkg ---"
    apk add --no-cache clang lld musl-dev make zstd-dev zstd-static
    # WORKSPACE is set by CI; fall back to the repo root relative to this script
    SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
    REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
    cd "${REPO_ROOT}/packages/jpkg"
    make CC=clang LDFLAGS="-static -fuse-ld=lld" jpkg
    install -m 755 jpkg /usr/local/bin/jpkg
    cd -
fi

# ---------------------------------------------------------------------------
# 3. Prepare staging directory (clean slate)
# ---------------------------------------------------------------------------
echo "--- Preparing staging directory ---"
rm -rf "${STAGING}"
mkdir -p "${STAGING}"

# FHS skeleton (pre-flatten; jpkg will land files under usr/ which we merge
# into / after the install pass, matching the Dockerfile/Pi layout).
for d in \
    bin lib etc boot dev proc sys run tmp \
    home root \
    var/log var/cache/jpkg var/db/jpkg/installed \
    etc/jpkg/keys \
    etc/ssl/certs \
    etc/dropbear \
    etc/init.d \
    etc/conf.d \
    etc/network
do
    mkdir -p "${STAGING}/${d}"
done

# Fix permissions
chmod 0700 "${STAGING}/root"
chmod 1777 "${STAGING}/tmp"

# ---------------------------------------------------------------------------
# 4. Configure jpkg repository
# ---------------------------------------------------------------------------
echo "--- Writing /etc/jpkg/repos.conf ---"
printf '[repo]\nurl = "%s"\n' "${PKG_BASE_URL}" > "${STAGING}/etc/jpkg/repos.conf"

# ---------------------------------------------------------------------------
# 5. Install packages via jpkg
#
# Core runtime mirrors jonerix:core (the "fresh install" reference image):
#   libs/crypto  : musl, zlib, ncurses, libressl, zstd, lz4, xz
#   shell/coreutils: mksh (as /bin/sh), toybox, libarchive, bsdtar
#   network      : curl, dropbear
#   dev essentials (expected by `jpkg install` users): tzdata
#   userland     : doas, snooze, pigz, mandoc
#   editor/shell : micro, fastfetch
#   search       : ripgrep
#   pkg mgmt     : jpkg (installed last)
#
# Not shipped in WSL rootfs:
#   openrc, dhcpcd, ifupdown-ng, unbound, openntpd — WSL provides init,
#   networking, and DNS through the hypervisor; shipping these just adds
#   bloat and services that never start. Users can `jpkg install` them.
#   LLVM/Go/Rust/nodejs/python3 — huge; install on demand.
# ---------------------------------------------------------------------------
echo "--- Installing packages via jpkg ---"
jpkg --root "${STAGING}" update
jpkg --root "${STAGING}" install \
    musl ncurses libressl zlib xz lz4 zstd \
    mksh toybox libarchive bsdtar \
    curl dropbear \
    tzdata doas snooze pigz mandoc \
    micro fastfetch ripgrep

# Install jpkg itself into the rootfs
echo "  -> jpkg"
jpkg --root "${STAGING}" install jpkg || \
    install -Dm755 "$(command -v jpkg)" "${STAGING}/bin/jpkg"

# ---------------------------------------------------------------------------
# 6. Flatten merged-usr layout
#
# jpkg archives land files under usr/ by convention; jonerix is a merged-usr
# system where /usr is a symlink to /. Flatten now, then add the symlink so
# /usr/bin, /usr/lib, /usr/share all resolve to /bin, /lib, /share.
# ---------------------------------------------------------------------------
echo "--- Flattening merged-usr layout ---"
if [ -d "${STAGING}/usr" ] && [ ! -L "${STAGING}/usr" ]; then
    cp -a "${STAGING}/usr/." "${STAGING}/"
    rm -rf "${STAGING}/usr"
fi
ln -s / "${STAGING}/usr"

# ---------------------------------------------------------------------------
# 7. Symlinks — match what a fresh Pi install ships
# ---------------------------------------------------------------------------
echo "--- Creating default symlinks ---"
# mksh is /bin/sh (POSIX, MirOS-licensed)
ln -sf mksh    "${STAGING}/bin/sh"   2>/dev/null || true
# bsdtar replaces toybox tar (symlink handling)
ln -sf bsdtar  "${STAGING}/bin/tar"  2>/dev/null || true
# dropbear ssh client
ln -sf dbclient "${STAGING}/bin/ssh" 2>/dev/null || true
# editor aliases
ln -sf micro   "${STAGING}/bin/editor" 2>/dev/null || true
ln -sf micro   "${STAGING}/bin/vi"     2>/dev/null || true

# SUID bits on login utilities (if present)
chmod 4755 "${STAGING}/bin/su" "${STAGING}/bin/passwd" "${STAGING}/bin/login" "${STAGING}/bin/doas" 2>/dev/null || true

# ---------------------------------------------------------------------------
# 8. Copy config files from repo defaults
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DEFAULTS="${REPO_ROOT}/config/defaults/etc"

echo "--- Copying default config files ---"
for f in hostname passwd group shadow shells profile zshrc doas.conf os-release securetty; do
    if [ -f "${DEFAULTS}/${f}" ]; then
        cp "${DEFAULTS}/${f}" "${STAGING}/etc/${f}"
    fi
done
[ -f "${DEFAULTS}/shadow" ] && chmod 0600 "${STAGING}/etc/shadow"

# fastfetch config tree
if [ -d "${DEFAULTS}/fastfetch" ]; then
    mkdir -p "${STAGING}/etc/fastfetch"
    cp -a "${DEFAULTS}/fastfetch/." "${STAGING}/etc/fastfetch/"
fi

# sysctl.d (safe defaults; kernel may ignore some under WSL, harmless)
if [ -d "${DEFAULTS}/sysctl.d" ]; then
    mkdir -p "${STAGING}/etc/sysctl.d"
    cp -a "${DEFAULTS}/sysctl.d/." "${STAGING}/etc/sysctl.d/"
fi

# Install signing key
PUBKEY="${DEFAULTS}/jpkg/keys/jonerix.pub"
[ -f "${PUBKEY}" ] && cp "${PUBKEY}" "${STAGING}/etc/jpkg/keys/jonerix.pub"

# CA certificates — jonerix does not ship a ca-certificates package yet,
# so curl the Mozilla bundle from curl.se (matches Dockerfile.minimal).
echo "--- Fetching CA certificates ---"
curl -fsSL https://curl.se/ca/cacert.pem -o "${STAGING}/etc/ssl/certs/ca-certificates.crt"

# ---------------------------------------------------------------------------
# 9. WSL-specific config (not in repo defaults)
# ---------------------------------------------------------------------------

# /etc/resolv.conf: WSL generates this itself from the Windows resolver.
# Leave the stock jonerix copy out — WSL overwrites it on boot when
# generateResolvConf=true (the default). Drop a placeholder only so
# lookups work on the very first boot before WSL writes the real one.
cat > "${STAGING}/etc/resolv.conf" << 'EOF'
# /etc/resolv.conf — replaced at boot by WSL (generateResolvConf=true).
# If you disable WSL's generator in /etc/wsl.conf, add your own nameservers.
nameserver 1.1.1.1
nameserver 1.0.0.1
EOF

# /etc/wsl.conf — WSL2 per-distro configuration.
# - [boot] systemd=false: jonerix uses openrc/no-init; no systemd in the tree.
# - [network] generateResolvConf=true: let WSL maintain resolv.conf from the
#   Windows host (users with corporate DNS get it automatically).
# - [user] default=root: match the Pi install default (root-by-default shell).
# - [interop] appendWindowsPath=false: keep $PATH clean; Windows tools are
#   still launchable via explicit /mnt/c/... paths.
# - [automount] options: metadata lets chmod/chown work on /mnt/c.
cat > "${STAGING}/etc/wsl.conf" << 'EOF'
[boot]
systemd = false

[user]
default = root

[network]
generateHosts = true
generateResolvConf = true

[interop]
enabled = true
appendWindowsPath = false

[automount]
enabled = true
mountFsTab = false
options = "metadata,umask=22,fmask=11"
EOF

# /etc/fstab — WSL runs `mount -a` at boot when mountFsTab=true. We
# disable that in wsl.conf (mountFsTab=false) because the Pi-inherited
# fstab lines in any package we install (raspi5-fixups' base rescue,
# etc.) reference /dev/mmcblk0p2 and blow up. Still ship an empty
# header file so tools that read /etc/fstab (e.g. `findmnt --source`)
# don't trip on ENOENT.
cat > "${STAGING}/etc/fstab" << 'EOF'
# /etc/fstab — intentionally empty on WSL.
# WSL mounts rootfs itself (via --import) and /mnt/<drive> via drvfs
# from [automount] in /etc/wsl.conf. Add extra mounts here only if
# you also set `mountFsTab = true` in /etc/wsl.conf.
EOF

# ---------------------------------------------------------------------------
# 10. Pack the rootfs
# ---------------------------------------------------------------------------
OUTPUT_FILE="${OUTPUT_DIR}/jonerix-rootfs-${ARCH}.tar.gz"
echo "--- Creating tarball: ${OUTPUT_FILE} ---"
mkdir -p "${OUTPUT_DIR}"
# bsdtar (libarchive) preserves symlinks and handles the merged-usr layout
# cleanly; GNU/BSD --exclude syntax is identical here.
tar -C "${STAGING}" -czf "${OUTPUT_FILE}" \
    --exclude='./proc/*' \
    --exclude='./sys/*' \
    --exclude='./dev/*' \
    --exclude='./run/*' \
    .
echo "Done. Rootfs: $(ls -lh "${OUTPUT_FILE}" | awk '{print $5}') — ${OUTPUT_FILE}"
