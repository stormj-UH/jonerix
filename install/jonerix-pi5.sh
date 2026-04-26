#!/bin/sh
# jonerix-pi5.sh — bootstrap installer for the jonerix Raspberry Pi 5 image.
#
# Designed to be run with curl + sh from any POSIX-compliant Linux host:
#
#   curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5.sh | sh
#
# or with arguments:
#
#   curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5.sh \
#     | sh -s -- -d /dev/sda
#
# What this does, in order:
#   1. Sanity-check the environment (Linux, root or sudo, curl, target device exists).
#   2. Download the latest pi5-install.sh from the source tree (default: main).
#   3. Hand off to it with the same arguments.
#
# Why this exists: the full installer (install/pi5-install.sh) is ~500 lines
# and depends on pulling more files at runtime; trying to one-liner it inside
# a curl-pipe-sh is awkward. This wrapper is small enough to read, short
# enough to host on raw.githubusercontent.com, and forgiving enough that a
# user with a single Pi 5 USB drive can get from "no jonerix" to "Pi 5
# booting jonerix" in two commands.
#
# POSIX shell only — runs on dash, ash, bash, mksh, zsh. No bashisms.
# Tested on Ubuntu 24.04, Debian 12, Alpine 3.21, Arch, jonerix.
#
# Part of jonerix — MIT License.

set -eu

# ── Defaults ────────────────────────────────────────────────────────
BRANCH="${BRANCH:-main}"
GH_RAW="https://raw.githubusercontent.com/stormj-UH/jonerix/${BRANCH}"
INSTALLER_URL="${GH_RAW}/install/pi5-install.sh"
TARGET_SCRIPT="/tmp/jonerix-pi5-install.$$.sh"

# ── Helpers ─────────────────────────────────────────────────────────
banner() {
    cat <<'EOF'
================================================================================
    _                       _
   (_) ___  _ __   ___ _ __(_)_  __
   | |/ _ \| '_ \ / _ \ '__| \ \/ /
   | | (_) | | | |  __/ |  | |>  <
  _/ |\___/|_| |_|\___|_|  |_/_/\_\   raspberry pi 5 installer
 |__/
================================================================================
EOF
}

msg()  { printf '==> %s\n' "$*"; }
warn() { printf '!!  %s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

# ── Help ────────────────────────────────────────────────────────────
case "${1:-}" in
    -h|--help)
        banner
        cat <<EOF

usage: jonerix-pi5.sh [options] [-- args-passed-to-pi5-install.sh]

This script downloads the latest install/pi5-install.sh from the jonerix
repo (branch: ${BRANCH}) and runs it. All arguments after \`--\` are
passed straight through. Common flows:

  # Fresh install onto an attached USB / SD / NVMe device:
  curl -fsSL ${GH_RAW}/install/jonerix-pi5.sh \\
    | sudo sh -s -- -d /dev/sdX

  # Complete an install after dd'ing a jonerix-pi5.img CI artifact
  # (which deliberately ships without firmware):
  curl -fsSL ${GH_RAW}/install/jonerix-pi5.sh \\
    | sudo sh -s -- -d /dev/sdX --firmware-only

  # Pin to a specific jonerix release:
  curl -fsSL ${GH_RAW}/install/jonerix-pi5.sh \\
    | sudo sh -s -- -d /dev/sdX --release-tag v1.1.6

  # Use a feature branch's installer (developer flow):
  BRANCH=my-branch curl -fsSL ${GH_RAW}/install/jonerix-pi5.sh | sudo sh

Bootstrap-script options (consumed here, not forwarded):
  -h, --help       Show this help.
  --branch NAME    Override jonerix git branch (default: ${BRANCH}).

For the full pi5-install.sh option set (target device, firmware-only
mode, release tag pinning, etc.) see:
  ${GH_RAW}/install/pi5-install.sh
EOF
        exit 0
        ;;
esac

banner

# ── Strip our own --branch flag so we don't double-pass it ──────────
# Everything else is forwarded verbatim to pi5-install.sh.
ARGS=""
while [ $# -gt 0 ]; do
    case "$1" in
        --branch)
            [ $# -ge 2 ] || die "--branch requires an argument"
            BRANCH="$2"
            GH_RAW="https://raw.githubusercontent.com/stormj-UH/jonerix/${BRANCH}"
            INSTALLER_URL="${GH_RAW}/install/pi5-install.sh"
            shift 2
            ;;
        --) shift; break ;;
        *)
            # Quote the arg so shell metacharacters survive the
            # eventual eval. Pure POSIX equivalent of "$@" forwarding
            # in a string-accumulator.
            ARGS="$ARGS \"$(printf '%s' "$1" | sed 's/"/\\"/g')\""
            shift
            ;;
    esac
done
# Anything after `--` also forwards verbatim.
while [ $# -gt 0 ]; do
    ARGS="$ARGS \"$(printf '%s' "$1" | sed 's/"/\\"/g')\""
    shift
done

# ── Sanity checks ───────────────────────────────────────────────────

case "$(uname -s)" in
    Linux) ;;
    Darwin)
        die "macOS detected. dd-ing to /dev/disk* on macOS works but the
   install script needs Linux block-device tooling (sfdisk, mkfs.ext4,
   blkid, mount). Run this from a Linux host (a VM works fine), or
   download the CI image artifact and dd it directly:
     gh run download --repo stormj-UH/jonerix -n jonerix-pi5-image
     sudo dd if=jonerix-pi5.img of=/dev/diskN bs=4m status=progress
   then complete the install on a Linux host with --firmware-only."
        ;;
    *)
        warn "untested OS '$(uname -s)' — proceeding optimistically"
        ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    die "must be run as root. Re-run with sudo:
   curl -fsSL ${INSTALLER_URL%/install/pi5-install.sh}/install/jonerix-pi5.sh | sudo sh"
fi

if ! command -v curl >/dev/null 2>&1; then
    die "curl is required (we used it to get here, so check \$PATH)"
fi

# ── Fetch the real installer ────────────────────────────────────────
msg "Downloading installer from ${INSTALLER_URL}"
if ! curl -fsSL --retry 3 -o "$TARGET_SCRIPT" "$INSTALLER_URL"; then
    rm -f "$TARGET_SCRIPT"
    die "failed to download $INSTALLER_URL — check the BRANCH name + your network"
fi
chmod +x "$TARGET_SCRIPT"

# Trap-cleanup the downloaded copy regardless of exit path.
trap 'rm -f "$TARGET_SCRIPT"' EXIT

# Pass BRANCH through so pi5-install.sh's GH_RAW resolves to the same
# branch we picked up from. This matters for --release-tag default
# resolution (which pulls config/defaults/etc/os-release from BRANCH)
# and for any other GH_RAW-relative fetches inside the installer.
msg "Running pi5-install.sh ${ARGS:-(no args)}"
# shellcheck disable=SC2086  # ARGS is a quoted-token string we want to eval
BRANCH="$BRANCH" eval "$TARGET_SCRIPT $ARGS"
