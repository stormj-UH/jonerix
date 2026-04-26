#!/bin/sh
# jonerix-pi5-netboot.sh — boot a Raspberry Pi 5 over the network from
# this Mac or Linux host. Downloads the latest netboot payload + rootfs
# from the jonerix release matching the source tree's VERSION_ID
# (default: v1.1.7 → "CONFORMable" set), starts a TFTP server on :69 and
# an HTTP server on :8080 pointing at them, and prints the exact Pi 5
# bootloader settings the user needs.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5-netboot.sh \
#     | sudo sh
#
# Or with a fixed release pin and a specific network interface:
#   curl -fsSL .../jonerix-pi5-netboot.sh | sudo sh -s -- \
#     --release-tag v1.1.7 --bind 192.168.1.42
#
# Why root: TFTP listens on port 69 (privileged). HTTP defaults to 8080
# (unprivileged) so the same script works on hosts where you'd rather
# not bind low ports for HTTP.
#
# Cross-platform: tested on macOS 14 (Sonoma) and Ubuntu 24.04. Uses
# only python3 from the OS — no Homebrew or apt dependencies.
#
# POSIX shell only (dash/ash/sh tested). Part of jonerix — MIT License.

set -eu

BRANCH="${BRANCH:-main}"
GH_RAW="https://raw.githubusercontent.com/stormj-UH/jonerix/${BRANCH}"
RELEASE_TAG="${RELEASE_TAG:-}"
BIND_IP="${BIND_IP:-}"
TFTP_PORT="${TFTP_PORT:-69}"
HTTP_PORT="${HTTP_PORT:-8080}"
CACHE_DIR="${CACHE_DIR:-${HOME}/.cache/jonerix-pi5-netboot}"
FIRMWARE_TAG="${FIRMWARE_TAG:-stable}"
FIRMWARE_URL="https://github.com/raspberrypi/firmware/archive/refs/heads/${FIRMWARE_TAG}.tar.gz"
ACCEPT_LICENSES="${ACCEPT_LICENSES:-0}"
# Mode B scratch tmpfs size (passed to the Pi via kernel cmdline as
# jonerix.state_size=). Anything Linux's tmpfs `size=` accepts: 256M,
# 1G, 2048k, 50% (percent of RAM), etc.
STATE_SIZE="${STATE_SIZE:-512M}"

banner() {
    cat <<'EOF'
================================================================================
    _                       _
   (_) ___  _ __   ___ _ __(_)_  __
   | |/ _ \| '_ \ / _ \ '__| \ \/ /
   | | (_) | | | |  __/ |  | |>  <
  _/ |\___/|_| |_|\___|_|  |_/_/\_\   raspberry pi 5 netboot server
 |__/
================================================================================
EOF
}

msg()  { printf '==> %s\n' "$*"; }
warn() { printf '!!  %s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
    banner
    cat <<EOF

usage: jonerix-pi5-netboot.sh [options]

Starts a TFTP server (port ${TFTP_PORT}) and an HTTP rootfs server (port ${HTTP_PORT})
on this host. Point a Raspberry Pi 5's TFTP_PREFIX (or your DHCP
server's option 66/67) at this machine and the Pi will netboot a
permissive-licensed jonerix userland.

Options:
  --release-tag TAG     Pin to a specific jonerix release (e.g. v1.1.7).
                        Default: VERSION_ID from the BRANCH's os-release.
                        Pass 'packages' to use the rolling mirror.
  --bind IP             IP to bind both servers to (default: 0.0.0.0,
                        printed addresses use the first non-loopback
                        interface).
  --tftp-port N         TFTP port (default 69; needs root).
  --http-port N         HTTP rootfs port (default 8080).
  --firmware-tag TAG    raspberrypi/firmware branch/tag to fetch
                        (default ${FIRMWARE_TAG}).
  --accept-licenses     Skip the interactive GPL-2.0 + Broadcom
                        Redistributable license prompt and accept
                        non-interactively. Required for unattended use.
  --state-size SIZE     Mode-B scratch tmpfs at /var/state on the Pi
                        (default ${STATE_SIZE}). Accepts anything Linux's
                        tmpfs size= takes: 256M, 1G, 2048k, 50% (% of
                        RAM). Written to cmdline.txt as jonerix.state_size=.
  --cache DIR           Cache dir for downloaded payloads
                        (default ${CACHE_DIR}).
  -h, --help            Show this help.

Pi 5 bootloader configuration (one-time, on the Pi side):

  # via raspi-config nonint:
  raspi-config nonint do_boot_order 0xf124   # USB → SD → NETWORK
  # or via rpi-eeprom-config edit:
  TFTP_PREFIX=2
  TFTP_IP=<this-host-ip>
  BOOT_ORDER=0xf124

EOF
}

# ── Argument parsing ─────────────────────────────────────────────────
while [ $# -gt 0 ]; do
    case "$1" in
        --release-tag) RELEASE_TAG="${2:-}"; shift 2 ;;
        --bind)        BIND_IP="${2:-}"; shift 2 ;;
        --tftp-port)   TFTP_PORT="${2:-}"; shift 2 ;;
        --http-port)   HTTP_PORT="${2:-}"; shift 2 ;;
        --cache)       CACHE_DIR="${2:-}"; shift 2 ;;
        --branch)      BRANCH="${2:-}"; GH_RAW="https://raw.githubusercontent.com/stormj-UH/jonerix/${BRANCH}"; shift 2 ;;
        --firmware-tag)
            FIRMWARE_TAG="${2:-stable}"
            FIRMWARE_URL="https://github.com/raspberrypi/firmware/archive/refs/heads/${FIRMWARE_TAG}.tar.gz"
            shift 2 ;;
        --accept-licenses) ACCEPT_LICENSES=1; shift ;;
        --state-size) STATE_SIZE="${2:-512M}"; shift 2 ;;
        -h|--help)     usage; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
done

# ── Sanity ───────────────────────────────────────────────────────────
banner

case "$(uname -s)" in
    Linux|Darwin) ;;
    *) die "unsupported OS: $(uname -s) (Linux or macOS only)" ;;
esac

if [ "$TFTP_PORT" -lt 1024 ] && [ "$(id -u)" -ne 0 ]; then
    die "TFTP port ${TFTP_PORT} is privileged — re-run with sudo or pass --tftp-port 6969"
fi

for t in curl python3; do
    command -v "$t" >/dev/null 2>&1 || die "missing required tool: $t"
done

# ── Resolve --release-tag from os-release if unset ───────────────────
if [ -z "$RELEASE_TAG" ]; then
    _osr=$(curl -fsSL "${GH_RAW}/config/defaults/etc/os-release" 2>/dev/null \
        | awk -F= '/^VERSION_ID=/ { gsub(/"/,"",$2); print $2 }')
    if [ -n "$_osr" ]; then
        RELEASE_TAG="v${_osr}"
        msg "Resolved release tag: ${RELEASE_TAG} (from BRANCH=${BRANCH} os-release)"
    else
        RELEASE_TAG="packages"
        warn "could not fetch ${GH_RAW}/config/defaults/etc/os-release; using rolling 'packages' mirror"
    fi
fi
RELEASE_BASE="https://github.com/stormj-UH/jonerix/releases/download/${RELEASE_TAG}"

# ── Detect bind IP ───────────────────────────────────────────────────
if [ -z "$BIND_IP" ]; then
    case "$(uname -s)" in
        Darwin)
            BIND_IP=$(ifconfig 2>/dev/null \
                | awk '/^en[0-9]+:/{ifc=$1; next} ifc && /inet [0-9]/{print $2; exit}' \
                || true)
            ;;
        Linux)
            BIND_IP=$(ip -o -4 addr 2>/dev/null \
                | awk '$2!="lo"{split($4,a,"/"); print a[1]; exit}' \
                || true)
            ;;
    esac
    BIND_IP="${BIND_IP:-0.0.0.0}"
fi

# ── Fetch payload(s) ─────────────────────────────────────────────────
mkdir -p "$CACHE_DIR"
TFTP_TGZ="${CACHE_DIR}/jonerix-pi5-netboot.tar.gz"

# The CI workflow Publish Pi 5 netboot payload publishes to the rolling
# `pi5-netboot` release tag. Per-version pinning is via the matching
# v* tag where available; fall back to pi5-netboot for older trees
# that haven't shipped the per-version artifact yet.
fetch_one() {
    _name="$1"; _dest="$2"
    for _src in \
        "${RELEASE_BASE}/${_name}" \
        "https://github.com/stormj-UH/jonerix/releases/download/pi5-netboot/${_name}"; do
        msg "GET ${_src}"
        if curl -fsSL --retry 3 -o "$_dest" "$_src"; then
            return 0
        fi
    done
    return 1
}

if [ ! -s "$TFTP_TGZ" ]; then
    fetch_one "jonerix-pi5-netboot.tar.gz" "$TFTP_TGZ" \
        || die "could not fetch netboot tarball from ${RELEASE_BASE} or pi5-netboot"
else
    msg "Cached netboot tarball at ${TFTP_TGZ}"
fi

# (Live-installer rootfs is fetched later, after firmware staging.)

# ── Stage TFTP tree ──────────────────────────────────────────────────
TFTP_DIR="${CACHE_DIR}/tftp"
rm -rf "$TFTP_DIR"
mkdir -p "$TFTP_DIR"
msg "Extracting jonerix TFTP tree to ${TFTP_DIR}"
( cd "$TFTP_DIR" && tar xzf "$TFTP_TGZ" --strip-components=1 ) \
    || die "failed to extract ${TFTP_TGZ}"

# ── Firmware: not in jonerix CI artifacts; fetch from raspberrypi/firmware ──
# jonerix's permissive-userland policy keeps the Linux kernel (GPL-2.0)
# and the Broadcom GPU/CPU firmware blobs (Broadcom Redistributable)
# OUT of CI artifacts. The bootstrap downloads them directly from the
# upstream raspberrypi/firmware repo on the user's machine, with
# explicit license acceptance, and stages them into the same TFTP tree
# the Pi will boot from.
needs_firmware=1
[ -f "$TFTP_DIR/kernel_2712.img" ] && needs_firmware=0
if [ "$needs_firmware" = 1 ]; then
    cat <<'LICENSE_NOTICE'

------------------------------------------------------------------------
The Pi 5 cannot boot without these two non-permissive components, both
fetched from raspberrypi/firmware (NOT from jonerix):

  1. Linux kernel (kernel_2712.img, device-tree blobs)
     License: GNU General Public License v2.0
     Source:  https://github.com/raspberrypi/linux

  2. VideoCore / Broadcom firmware blobs (start4.elf, fixup4.dat, etc.)
     License: proprietary Broadcom binary — see LICENCE.broadcom in
              the tarball. Free to redistribute with Raspberry Pi
              hardware; may NOT be modified or used outside Pi boards.
     Source:  closed-source (Broadcom, distributed by Raspberry Pi Ltd)

Continuing means you have reviewed and accept BOTH licenses.
LICENSE_NOTICE
    if [ "$ACCEPT_LICENSES" != 1 ]; then
        printf 'Accept these licenses and download? [y/N] '
        read _ans </dev/tty || _ans=""
        case "${_ans:-n}" in
            y|Y|yes|YES) : ;;
            *) die "declined firmware/kernel license — aborting (re-run with --accept-licenses to skip this prompt)" ;;
        esac
    fi

    FW_TGZ="${CACHE_DIR}/raspberrypi-firmware-${FIRMWARE_TAG}.tar.gz"
    if [ ! -s "$FW_TGZ" ]; then
        msg "Fetching ${FIRMWARE_URL} (~500 MiB — first run only, then cached)"
        curl -fL --retry 3 -o "$FW_TGZ" "$FIRMWARE_URL" \
            || die "failed to download ${FIRMWARE_URL}"
    else
        msg "Cached firmware tarball at ${FW_TGZ}"
    fi

    # Extract just boot/* into the TFTP dir (drop the
    # firmware-${TAG}/ prefix the github archive includes).
    FW_STAGE="${CACHE_DIR}/firmware-extract"
    rm -rf "$FW_STAGE"; mkdir -p "$FW_STAGE"
    msg "Extracting firmware boot/ tree"
    tar xzf "$FW_TGZ" -C "$FW_STAGE" \
        --strip-components=1 \
        '*/boot' \
        || die "failed to extract firmware tarball"
    if [ ! -d "$FW_STAGE/boot" ]; then
        die "firmware tarball did not contain a boot/ tree"
    fi
    # Copy the Pi 5-relevant subset into the TFTP root.
    for f in kernel_2712.img \
             bcm2712-rpi-5-b.dtb bcm2712-d-rpi-5-b.dtb \
             bcm2712-rpi-cm5-cm4io.dtb bcm2712-rpi-cm5-cm5io.dtb \
             bcm2712-rpi-cm5l-cm4io.dtb bcm2712-rpi-cm5l-cm5io.dtb \
             bcm2712-rpi-500.dtb \
             start4.elf start4cd.elf start4db.elf start4x.elf \
             fixup4.dat fixup4cd.dat fixup4db.dat fixup4x.dat \
             LICENCE.broadcom; do
        if [ -f "$FW_STAGE/boot/$f" ]; then
            cp -f "$FW_STAGE/boot/$f" "$TFTP_DIR/$f"
        fi
    done
    # Overlays directory (Pi 5 needs at least disable-bt, miniuart-bt
    # for some setups; bring the lot — they're tiny).
    if [ -d "$FW_STAGE/boot/overlays" ]; then
        mkdir -p "$TFTP_DIR/overlays"
        cp -f "$FW_STAGE/boot/overlays/"*.dtbo "$TFTP_DIR/overlays/" 2>/dev/null || true
    fi
    rm -f "$TFTP_DIR/FIRMWARE_MISSING"
    msg "Firmware staged into TFTP tree."
fi

# ── Inject state-size into the kernel cmdline ────────────────────────
# The rootfs's pi5-state OpenRC service reads jonerix.state_size from
# /proc/cmdline at boot and mounts /var/state with that size. We
# patch the cmdline.txt the Pi will receive over TFTP to carry the
# user's chosen value (default 512M).
if [ -f "$TFTP_DIR/cmdline.txt" ]; then
    # Strip any prior jonerix.state_size= and append the new one.
    _tmp=$(mktemp)
    awk -v sz="$STATE_SIZE" '
        {
            gsub(/[[:space:]]+jonerix.state_size=[^[:space:]]*/, "", $0)
            sub(/[[:space:]]*$/, "", $0)
            printf "%s jonerix.state_size=%s\n", $0, sz
        }
    ' "$TFTP_DIR/cmdline.txt" > "$_tmp"
    mv "$_tmp" "$TFTP_DIR/cmdline.txt"
    msg "cmdline.txt now carries jonerix.state_size=${STATE_SIZE}"
fi

# ── Stage live-installer rootfs (mode A/B target) ────────────────────
ROOTFS_TZST_LOCAL="${CACHE_DIR}/jonerix-pi5-netboot-rootfs.tar.zst"
if [ ! -s "$ROOTFS_TZST_LOCAL" ]; then
    if fetch_one "jonerix-pi5-netboot-rootfs.tar.zst" "$ROOTFS_TZST_LOCAL" 2>/dev/null; then
        msg "Rootfs cached at ${ROOTFS_TZST_LOCAL}"
    else
        warn "no jonerix-pi5-netboot-rootfs.tar.zst at ${RELEASE_BASE} or pi5-netboot — Pi will boot a kernel but no rootfs to mount"
    fi
fi

# ── Stage rootfs (best-effort) ───────────────────────────────────────
HTTP_ROOT="${CACHE_DIR}/http-root"
rm -rf "$HTTP_ROOT"
mkdir -p "$HTTP_ROOT"
# Always copy the TFTP tree into the HTTP root too — initramfs scripts
# often want to fetch kernel + dtb over HTTP rather than TFTP.
cp -R "$TFTP_DIR/." "$HTTP_ROOT/"
if [ -s "$ROOTFS_TZST_LOCAL" ]; then
    cp "$ROOTFS_TZST_LOCAL" "$HTTP_ROOT/jonerix-pi5-netboot-rootfs.tar.zst"
fi

# ── Run servers ──────────────────────────────────────────────────────
msg "Starting servers"
msg "  TFTP: tftp://${BIND_IP}:${TFTP_PORT}/  (root: ${TFTP_DIR})"
msg "  HTTP: http://${BIND_IP}:${HTTP_PORT}/  (root: ${HTTP_ROOT})"
echo
cat <<EOF
On the Pi 5, set the bootloader to netboot pointed at this host:

  sudo rpi-eeprom-config --edit
  # ...add or change:
  BOOT_ORDER=0xf124
  TFTP_IP=${BIND_IP}
  TFTP_PREFIX=2

Or, pre-image an SD card with raspi-config nonint:

  sudo raspi-config nonint do_boot_order 0xf124

Once the Pi reboots it will TFTP the kernel + DTBs from this server,
fetch the rootfs over HTTP, and come up at jonerix ${RELEASE_TAG}.

Press Ctrl-C to stop both servers.

EOF

# Use python3 for both because it's universally available on macOS +
# Linux. The TFTP server is a ~80-line read-only RFC-1350 implementation
# embedded below; the HTTP server is stdlib http.server.

# ── HTTP server (background) ─────────────────────────────────────────
python3 -m http.server --bind "$BIND_IP" --directory "$HTTP_ROOT" "$HTTP_PORT" &
HTTP_PID=$!
trap 'kill "$HTTP_PID" 2>/dev/null || true' EXIT INT TERM

# ── TFTP server (foreground) ─────────────────────────────────────────
export TFTPD_DIR="$TFTP_DIR"
export TFTPD_HOST="$BIND_IP"
export TFTPD_PORT="$TFTP_PORT"
exec python3 -c '
import os, sys, socket, struct, pathlib
TFTPD_DIR = pathlib.Path(os.environ["TFTPD_DIR"]).resolve()
HOST = os.environ["TFTPD_HOST"]
PORT = int(os.environ["TFTPD_PORT"])

OP_RRQ, OP_DATA, OP_ACK, OP_ERR = 1, 3, 4, 5
BLOCK_SIZE_DEFAULT = 512

def send_err(sock, peer, code, msg):
    sock.sendto(struct.pack("!HH", OP_ERR, code) + msg.encode() + b"\0", peer)

def serve_one(sock, peer, data):
    if struct.unpack("!H", data[:2])[0] != OP_RRQ:
        send_err(sock, peer, 4, "only read requests supported")
        return
    parts = data[2:].split(b"\0")
    if len(parts) < 2:
        send_err(sock, peer, 0, "malformed RRQ")
        return
    fname = parts[0].decode("ascii", errors="replace").lstrip("/")
    mode  = parts[1].decode("ascii", errors="replace").lower()
    options = {}
    it = iter(parts[2:-1])
    for k in it:
        try:
            v = next(it)
        except StopIteration:
            break
        options[k.decode().lower()] = v.decode()
    block_size = int(options.get("blksize", BLOCK_SIZE_DEFAULT))
    block_size = max(8, min(65464, block_size))

    target = (TFTPD_DIR / fname).resolve()
    try:
        target.relative_to(TFTPD_DIR)
    except ValueError:
        send_err(sock, peer, 2, "path traversal blocked")
        return
    if not target.is_file():
        sys.stderr.write(f"[tftp] miss: {fname}\n")
        send_err(sock, peer, 1, "file not found")
        return
    sys.stderr.write(f"[tftp] serve: {fname} ({target.stat().st_size} bytes) -> {peer[0]}\n")

    # OACK if the client requested options.
    if options:
        oack_pairs = []
        for k, v in options.items():
            if k == "blksize":
                oack_pairs.append((b"blksize", str(block_size).encode()))
            elif k == "tsize":
                oack_pairs.append((b"tsize", str(target.stat().st_size).encode()))
        if oack_pairs:
            payload = b""
            for k, v in oack_pairs:
                payload += k + b"\0" + v + b"\0"
            sock.sendto(struct.pack("!H", 6) + payload, peer)  # OACK = 6
            sock.settimeout(5)
            try:
                ack, _ = sock.recvfrom(4)
            except socket.timeout:
                return

    with open(target, "rb") as f:
        block = 0
        while True:
            chunk = f.read(block_size)
            block = (block + 1) & 0xFFFF
            sock.sendto(struct.pack("!HH", OP_DATA, block) + chunk, peer)
            sock.settimeout(5)
            for _retry in range(3):
                try:
                    ack, _ = sock.recvfrom(4)
                    if struct.unpack("!HH", ack[:4]) == (OP_ACK, block):
                        break
                except socket.timeout:
                    sock.sendto(struct.pack("!HH", OP_DATA, block) + chunk, peer)
            if len(chunk) < block_size:
                return

main_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
main_sock.bind((HOST, PORT))
sys.stderr.write(f"[tftp] listening on {HOST}:{PORT} root={TFTPD_DIR}\n")
while True:
    data, peer = main_sock.recvfrom(65535)
    # Spawn a per-transfer ephemeral socket so the main one stays
    # ready for the next RRQ while the previous transfer runs.
    pid = os.fork()
    if pid == 0:
        client_sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        client_sock.bind((HOST, 0))
        try:
            serve_one(client_sock, peer, data)
        finally:
            client_sock.close()
            os._exit(0)
    else:
        try:
            os.waitpid(-1, os.WNOHANG)  # reap finished children
        except ChildProcessError:
            pass
'
# (unreachable; exec replaces the shell)
