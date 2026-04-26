#!/bin/mksh
# /etc/init.d/pi5-netboot-menu (when shipped as an OpenRC service) or
# directly as the tty1 entry point of a netboot live rootfs.
#
# Text-mode menu shown at first boot of a jonerix Pi 5 netboot session.
# Three paths:
#
#   1) Run jonerix from this netboot session  (mode B — diskless)
#      Quietly drops the menu and lets the system finish booting; the
#      user gets a normal /bin/login on tty1, served from the rootfs
#      that the host's HTTP/NFS server is exposing. Persistence lives
#      on the server, not the Pi.
#
#   2) Install jonerix to a local disk         (mode A — disk image)
#      Walks the user through picking a target SD/USB/NVMe, accepts
#      the GPL-2.0 + Broadcom Redistributable licenses for the kernel
#      + firmware, runs `pi5-install.sh -y -d <target> --release-tag
#      v$VERSION_ID`. End state: target disk has the same package set
#      a CI-built jonerix-pi5.img would, pinned to the conformable
#      release. Pi reboots from local storage; the netboot rootfs is
#      no longer needed.
#
#   3) Drop to a shell                          (escape hatch)
#      For the rare moment you need to poke at the live rootfs
#      manually — diagnose hardware, install one extra package, etc.
#      Re-running this script (`/etc/init.d/pi5-netboot-menu`) brings
#      the menu back.
#
# Pure mksh: no whiptail (GPL), no dialog (LGPL), no curses runtime.
# ANSI escapes only — works on the Pi's HDMI VC + serial console + ssh.
# Part of jonerix — MIT License.

set -u

# ── Cosmetic ─────────────────────────────────────────────────────────
ESC=$(printf '\033')
RESET="${ESC}[0m"
BOLD="${ESC}[1m"
DIM="${ESC}[2m"
BLUE="${ESC}[34m"
CYAN="${ESC}[36m"
GREEN="${ESC}[32m"
YELLOW="${ESC}[33m"
RED="${ESC}[31m"
WHITE="${ESC}[37m"
CLR="${ESC}[2J${ESC}[H"     # clear screen + home

# Try to detect terminal width; fall back to 80.
COLS=80
if command -v stty >/dev/null 2>&1; then
    _sz=$(stty size 2>/dev/null || true)
    if [ -n "${_sz:-}" ]; then
        COLS=${_sz#* }
    fi
fi
[ "$COLS" -ge 40 ] || COLS=80

# Centered single-line text.
center() {
    _line="$1"
    _len=${#_line}
    _pad=$(( (COLS - _len) / 2 ))
    [ "$_pad" -lt 0 ] && _pad=0
    printf '%*s%s\n' "$_pad" '' "$_line"
}

hr() {
    printf '%s%*s%s\n' "$BLUE" "$COLS" '' "$RESET" | tr ' ' '='
}

# ── Banner ───────────────────────────────────────────────────────────
banner() {
    printf '%s' "$CLR"
    hr
    printf '%s%s\n' "$BLUE" '
       _                       _
      (_) ___  _ __   ___ _ __(_)_  __
      | |/ _ \| '"'"'_ \ / _ \ '"'"'__| \ \/ /
      | | (_) | | | |  __/ |  | |>  <
     _/ |\___/|_| |_|\___|_|  |_/_/\_\
    |__/                                        '"$BOLD"'pi 5 netboot'"$RESET"'
'
    printf '%s' "$RESET"
    hr

    # Identify the release we netbooted from.
    _rel="(unknown)"
    if [ -r /etc/jonerix-netboot/build-info.json ]; then
        _rel=$(awk -F'"' '/release_tag/ { print $4; exit }' \
            /etc/jonerix-netboot/build-info.json 2>/dev/null || echo unknown)
    elif [ -r /etc/os-release ]; then
        _rel="v$(awk -F= '/^VERSION_ID=/ { gsub(/"/,"",$2); print $2 }' /etc/os-release)"
    fi

    center "${WHITE}You are running a netboot live session pinned to ${GREEN}${_rel}${RESET}"
    center "${DIM}rootfs is served from your host machine; nothing has touched local disks yet${RESET}"
    echo
}

# ── Detect candidate target disks (mode A) ───────────────────────────
list_targets() {
    # Look at every block device and emit "name|size|model|removable"
    # for the candidates that look like a real attached disk.
    for _bd in /sys/block/*; do
        [ -d "$_bd" ] || continue
        _n=$(basename "$_bd")
        case "$_n" in
            loop*|ram*|zram*) continue ;;
        esac
        _rem=$(cat "$_bd/removable" 2>/dev/null || echo 0)
        _sz=$(cat "$_bd/size" 2>/dev/null || echo 0)
        _gb=$(( _sz * 512 / 1024 / 1024 / 1024 ))
        # Hide things smaller than 1 GiB (loops, zram, etc).
        [ "$_gb" -ge 1 ] || continue
        _model=$(cat "$_bd/device/model" 2>/dev/null | tr -s ' ' || true)
        printf '%s|%sGiB|%s|%s\n' "/dev/$_n" "$_gb" "${_model:-?}" "$_rem"
    done
}

# ── Mode A: install to local disk ────────────────────────────────────
mode_install() {
    banner
    printf '%s%s%s\n\n' "$BOLD" "Install jonerix to a local disk" "$RESET"

    _cands=$(list_targets)
    if [ -z "$_cands" ]; then
        printf '%sNo candidate target disks detected.%s\n' "$RED" "$RESET"
        printf 'Plug an SD card, USB drive, or NVMe into the Pi and try again.\n\n'
        printf 'Press Enter to return to the main menu... '
        read _ </dev/tty1 || true
        return
    fi

    printf '%sCandidate target devices:%s\n\n' "$BOLD" "$RESET"
    _i=0
    echo "$_cands" | while IFS='|' read -r dev sz model rem; do
        _i=$((_i + 1))
        _flag=""
        [ "$rem" = 1 ] && _flag=" ${DIM}(removable)${RESET}"
        printf '  %s[%d]%s  %-12s %-8s  %s%s\n' "$CYAN" "$_i" "$RESET" "$dev" "$sz" "$model" "$_flag"
    done
    echo

    printf '%sChoose target (number, or 0 to cancel): %s' "$BOLD" "$RESET"
    read _choice </dev/tty1 || _choice=0
    [ "$_choice" = "0" ] && return
    _target=$(echo "$_cands" | sed -n "${_choice}p" | cut -d'|' -f1)
    if [ -z "$_target" ] || [ ! -b "$_target" ]; then
        printf '%sInvalid selection.%s\n' "$RED" "$RESET"
        sleep 2
        return
    fi

    echo
    printf '%sWARNING:%s ' "${BOLD}${RED}" "$RESET"
    printf 'EVERYTHING on %s%s%s will be ERASED.\n' "$BOLD" "$_target" "$RESET"
    printf 'The install lays down a v%s-pinned jonerix matching the SD/USB image.\n' \
        "$(awk -F= '/^VERSION_ID=/ { gsub(/"/,"",$2); print $2 }' /etc/os-release 2>/dev/null || echo current)"
    printf 'Type %sYES%s (uppercase) to proceed: ' "$BOLD" "$RESET"
    read _confirm </dev/tty1 || _confirm=""
    if [ "$_confirm" != "YES" ]; then
        printf '%sCancelled.%s\n' "$YELLOW" "$RESET"
        sleep 2
        return
    fi

    echo
    printf '%sRunning pi5-install.sh -y -d %s --branch main%s\n' "$DIM" "$_target" "$RESET"
    echo

    # The live rootfs ships pi5-install.sh at /usr/local/bin/. Run it
    # with the same release tag we ourselves netbooted from so the
    # local disk ends up identical to a build-image.py output.
    _ver=$(awk -F= '/^VERSION_ID=/ { gsub(/"/,"",$2); print $2 }' /etc/os-release 2>/dev/null)
    _tag=""
    [ -n "${_ver:-}" ] && _tag="--release-tag v$_ver"

    # shellcheck disable=SC2086  # _tag is intentionally word-split
    if /usr/local/bin/pi5-install.sh -y -d "$_target" $_tag; then
        echo
        printf '%sInstall complete.%s\n' "${BOLD}${GREEN}" "$RESET"
        printf 'Reboot now to start jonerix from %s? [Y/n] ' "$_target"
        read _r </dev/tty1 || _r=y
        case "${_r:-y}" in
            n|N) printf 'Skipping reboot. Run %ssudo reboot%s when ready.\n' "$BOLD" "$RESET" ;;
            *) sync; reboot ;;
        esac
    else
        echo
        printf '%spi5-install.sh failed.%s See output above.\n' "${BOLD}${RED}" "$RESET"
        printf 'Press Enter to return to the menu... '
        read _ </dev/tty1 || true
    fi
}

# ── Mode B: just run from netboot ────────────────────────────────────
mode_diskless() {
    banner
    printf '%s%s%s\n\n' "$BOLD" "Running diskless from netboot rootfs" "$RESET"
    cat <<EOF
This Pi will continue running from the rootfs your host is serving over
HTTP/NFS. Nothing on local storage has been touched. /home/, /etc/,
and /var/ live on the server — when this Pi reboots, all changes are
lost unless you saved them server-side.

Useful for:
  - kicking the tires before committing to a local install
  - running the Pi as a stateless cluster node
  - rescuing a Pi whose local disk is unhappy

Hand-off: dropping to a regular login on tty1 in 3 seconds...
EOF
    sleep 3
    # Hand off to whatever the system normally does at this point.
    # Under shadow-getty / OpenRC, rc-service should put login back.
    if command -v rc-service >/dev/null 2>&1 \
        && [ -f /etc/init.d/shadow-login ]; then
        rc-service shadow-login start 2>/dev/null || true
    fi
    exit 0
}

# ── Mode 3: shell ────────────────────────────────────────────────────
mode_shell() {
    banner
    printf '%sDropping to a root shell on tty1.%s\n' "$BOLD" "$RESET"
    printf 'Re-run %s/etc/init.d/pi5-netboot-menu start%s to bring the menu back.\n\n' \
        "$BOLD" "$RESET"
    exec /bin/mksh -l
}

# ── Main loop ────────────────────────────────────────────────────────
main_menu() {
    while :; do
        banner
        printf '%sWhat do you want to do?%s\n\n' "$BOLD" "$RESET"
        printf '  %s[1]%s  Run jonerix from this netboot session ${DIM}(mode B — diskless)${RESET}\n' "$CYAN" "$RESET"
        printf '  %s[2]%s  Install jonerix to a local disk        ${DIM}(mode A — pi5-install.sh)${RESET}\n' "$CYAN" "$RESET"
        printf '  %s[3]%s  Drop to a shell                        ${DIM}(escape hatch)${RESET}\n' "$CYAN" "$RESET"
        echo
        printf '%sChoice [1/2/3]: %s' "$BOLD" "$RESET"
        read _c </dev/tty1 || _c=""
        case "${_c:-}" in
            1) mode_diskless; return ;;
            2) mode_install ;;
            3) mode_shell ;;
            *) ;;  # bad input → loop, redraw
        esac
    done
}

# When the script is invoked directly (vs sourced), run the menu.
case "${0##*/}" in
    pi5-netboot-menu*|*-netboot-menu*) main_menu ;;
    *) : ;;
esac
