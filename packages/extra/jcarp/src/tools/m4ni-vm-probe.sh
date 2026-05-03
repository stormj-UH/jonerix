#!/bin/sh
set -eu

HOST=${M4NI_HOST:-m4ni}
REMOTE_PATH='/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin'

printf '%s\n' "==> probing $HOST for CARP VM prerequisites"

ssh "$HOST" PATH="$REMOTE_PATH" sh -s <<'EOF'
set -eu

show_cmd() {
    name=$1
    if command -v "$name" >/dev/null 2>&1; then
        path=$(command -v "$name")
        printf '%s %s\n' "$name:" "$path"
        case "$name" in
            qemu-system-aarch64)
                "$name" --version | sed -n '1p'
                ;;
            qemu-img|limactl|colima|docker|brew)
                "$name" --version 2>&1 | sed -n '1p'
                ;;
        esac
    else
        printf '%s not found\n' "$name:"
    fi
}

printf '%s\n' "host: $(hostname)"
printf '%s\n' "kernel: $(uname -a)"
printf '%s\n' "path: $PATH"

if [ -x /usr/bin/sw_vers ]; then
    sw_vers
fi

for tool in qemu-system-aarch64 qemu-img limactl colima docker brew tart vfkit utmctl prlctl VBoxManage multipass podman; do
    show_cmd "$tool"
done

if command -v qemu-system-aarch64 >/dev/null 2>&1; then
    printf '%s\n' "qemu netdev help:"
    qemu-system-aarch64 -machine virt -netdev help 2>&1 | sed -n '/vmnet/p'
fi

printf '%s\n' "existing OpenBSD/QEMU-looking artifacts:"
for dir in "$HOME/VMs" "$HOME/Virtual Machines.localized" "$HOME/.qemu" "$HOME/.lima" "$HOME/.colima" "$HOME/Downloads"; do
    [ -d "$dir" ] || continue
    find "$dir" -maxdepth 3 \( -iname '*openbsd*' -o -iname '*.qcow2' -o -iname '*.img' \) -print 2>/dev/null | sed -n '1,40p'
done
EOF
