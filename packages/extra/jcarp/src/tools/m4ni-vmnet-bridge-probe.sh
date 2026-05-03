#!/bin/sh
set -eu

HOST=${M4NI_HOST:-m4ni}
IFNAME=${M4NI_BRIDGE_IF:-en0}
SUDO_MODE=${M4NI_QEMU_USE_SUDO:-auto}
REMOTE_PATH='/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin'

printf '%s\n' "==> probing $HOST QEMU vmnet-bridged on $IFNAME"

ssh "$HOST" PATH="$REMOTE_PATH" IFNAME="$IFNAME" SUDO_MODE="$SUDO_MODE" sh -s <<'EOF'
set -eu

if ! command -v qemu-system-aarch64 >/dev/null 2>&1; then
    printf '%s\n' "error: qemu-system-aarch64 not found" >&2
    exit 1
fi

USE_SUDO=0
case "$SUDO_MODE" in
    1|yes|true)
        USE_SUDO=1
        ;;
    0|no|false)
        USE_SUDO=0
        ;;
    auto|"")
        if sudo -n true >/dev/null 2>&1; then
            USE_SUDO=1
        fi
        ;;
    *)
        printf '%s\n' "error: M4NI_QEMU_USE_SUDO must be auto, 1, or 0" >&2
        exit 1
        ;;
esac

run_qemu() {
    if [ "$USE_SUDO" -eq 1 ]; then
        sudo -n env PATH="$PATH" qemu-system-aarch64 "$@"
    else
        qemu-system-aarch64 "$@"
    fi
}

OUT=${TMPDIR:-/tmp}/jcarp-qemu-vmnet-probe.out
rm -f "$OUT"

if [ "$USE_SUDO" -eq 1 ]; then
    printf '%s\n' "using sudo -n for QEMU vmnet"
else
    printf '%s\n' "using unprivileged QEMU vmnet"
fi

run_qemu \
    -machine virt,accel=hvf \
    -nodefaults \
    -nographic \
    -serial none \
    -monitor none \
    -netdev vmnet-bridged,id=carp0,ifname="$IFNAME" \
    -device virtio-net-pci,netdev=carp0 \
    -S >"$OUT" 2>&1 &
pid=$!

sleep 2

if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" >/dev/null 2>&1 || true
    if [ "$USE_SUDO" -eq 1 ]; then
        sudo -n kill "$pid" >/dev/null 2>&1 || true
    fi
    wait "$pid" >/dev/null 2>&1 || true
    printf '%s\n' "ok: vmnet-bridged started; killed probe VM before boot"
    exit 0
fi

wait "$pid" || rc=$?
printf 'error: vmnet-bridged probe failed rc=%s\n' "${rc:-1}" >&2
sed -n '1,80p' "$OUT" >&2
exit 1
EOF
