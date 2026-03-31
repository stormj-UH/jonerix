#!/bin/sh
# install/wsl/build-rootfs.sh
#
# Build a minimal jonerix WSL2 rootfs tarball inside the current Alpine
# container.  Produces:
#   jonerix-rootfs-aarch64.tar.gz
#
# Usage (run as root inside Alpine):
#   sh build-rootfs.sh
#
# Environment variables:
#   GITHUB_REPO — GitHub repository slug (default: stormj-UH/jonerix)
#   PKG_RELEASE — GitHub release tag for packages (default: packages)
#   STAGING     — staging directory (default: /tmp/jonerix-rootfs)
#   OUTPUT_DIR  — where to write the tarball (default: current directory)

set -e

ARCH="aarch64"

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
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
apk add --no-cache curl zstd tar

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

# FHS skeleton
for d in \
    bin lib etc boot dev proc sys run tmp \
    home root \
    var/log var/cache/jpkg var/db/jpkg/installed \
    etc/jpkg/keys \
    etc/jpkg \
    etc/ssl/certs \
    etc/dropbear \
    etc/init.d \
    etc/conf.d \
    etc/network
do
    mkdir -p "${STAGING}/${d}"
done

# Merged-usr: /usr -> / so /usr/bin, /usr/lib, etc. resolve correctly
ln -sf . "${STAGING}/usr"

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
# ---------------------------------------------------------------------------
echo "--- Installing packages via jpkg ---"
jpkg --root "${STAGING}" update
jpkg --root "${STAGING}" install \
    musl toybox bsdtar dropbear openrc openssl curl zstd

# Install jpkg itself into the rootfs
echo "  -> jpkg"
jpkg --root "${STAGING}" install jpkg || \
    install -Dm755 "$(command -v jpkg)" "${STAGING}/bin/jpkg"

# ---------------------------------------------------------------------------
# 6. Symlinks
# ---------------------------------------------------------------------------
ln -sf bsdtar "${STAGING}/bin/tar" 2>/dev/null || true
ln -sf dbclient "${STAGING}/bin/ssh" 2>/dev/null || true

# ---------------------------------------------------------------------------
# 7. Copy config files from repo defaults
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DEFAULTS="${REPO_ROOT}/config/defaults/etc"

echo "--- Copying default config files ---"
for f in hostname resolv.conf passwd group shadow shells profile os-release; do
    [ -f "${DEFAULTS}/${f}" ] && cp "${DEFAULTS}/${f}" "${STAGING}/etc/${f}"
done
[ -f "${DEFAULTS}/shadow" ] && chmod 0600 "${STAGING}/etc/shadow"

# Install signing key
PUBKEY="${DEFAULTS}/jpkg/keys/jonerix.pub"
[ -f "${PUBKEY}" ] && cp "${PUBKEY}" "${STAGING}/etc/jpkg/keys/jonerix.pub"

# CA certificates
echo "--- Fetching CA certificates ---"
curl -fsSL https://curl.se/ca/cacert.pem -o "${STAGING}/etc/ssl/certs/ca-certificates.crt"

# wsl.conf — WSL2-specific, not in repo defaults
cat > "${STAGING}/etc/wsl.conf" << 'EOF'
[user]
default = root

[interop]
enabled = true
appendWindowsPath = false

[automount]
enabled = true
mountFsTab = true
EOF

# ---------------------------------------------------------------------------
# 8. Pack the rootfs
# ---------------------------------------------------------------------------
OUTPUT_FILE="${OUTPUT_DIR}/jonerix-rootfs-${ARCH}.tar.gz"
echo "--- Creating tarball: ${OUTPUT_FILE} ---"
mkdir -p "${OUTPUT_DIR}"
tar -C "${STAGING}" -czf "${OUTPUT_FILE}" \
    --exclude='./proc/*' \
    --exclude='./sys/*' \
    --exclude='./dev/*' \
    .
echo "Done. Rootfs: $(ls -lh "${OUTPUT_FILE}" | awk '{print $5}') — ${OUTPUT_FILE}"
