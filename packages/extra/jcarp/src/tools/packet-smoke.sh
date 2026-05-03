#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
REPO_DIR=$(CDPATH= cd "$SCRIPT_DIR/.." && pwd)
TMP_DIR=${TMPDIR:-/tmp}

cleanup() {
    if [ "${TCPDUMP_PID:-}" ]; then
        kill "$TCPDUMP_PID" >/dev/null 2>&1 || true
        wait "$TCPDUMP_PID" >/dev/null 2>&1 || true
    fi
    if [ "${WORK_DIR:-}" ]; then
        rm -rf "$WORK_DIR"
    fi
}
trap 'cleanup' 0 2 15

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf '%s\n' "error: required command not found: $1" >&2
        exit 1
    fi
}

need_cmd awk
need_cmd id
need_cmd mktemp
need_cmd sleep
need_cmd kill

if ! command -v tcpdump >/dev/null 2>&1; then
    printf '%s\n' "skip: tcpdump not found; cannot validate protocol 112 / ttl 255 / 224.0.0.18"
    exit 0
fi

if [ "$(id -u)" != "0" ]; then
    printf '%s\n' "error: packet smoke requires root (raw socket + tcpdump)." >&2
    printf '%s\n' "hint: run sudo sh $0" >&2
    exit 1
fi

if [ -n "${JCARP_BIN:-}" ]; then
    if [ ! -x "$JCARP_BIN" ]; then
        printf '%s\n' "error: JCARP_BIN is not executable: $JCARP_BIN" >&2
        exit 1
    fi
else
    need_cmd cargo
    printf '%s\n' "==> building jcarp"
    (
        cd "$REPO_DIR" || exit 1
        cargo build --locked
    ) || exit 1
    JCARP_BIN="$REPO_DIR/target/debug/jcarp"
fi

WORK_DIR=$(mktemp -d "$TMP_DIR/jcarp-smoke.XXXXXX")
CONFIG="$WORK_DIR/jcarp-smoke.conf"
TCPDUMP_LOG="$WORK_DIR/tcpdump.log"

cat >"$CONFIG" <<EOF
interface=lo
vhid=42
advbase=1
advskew=0
demote=0
preempt=true
vip=198.51.100.42
passphrase=packet-smoke
EOF

printf '%s\n' "==> starting tcpdump capture"
(
    cd "$REPO_DIR" || exit 1
    tcpdump -ni any -vv -c 1 'ip proto 112 and dst 224.0.0.18' >"$TCPDUMP_LOG" 2>&1
) &
TCPDUMP_PID=$!

sleep 1

printf '%s\n' "==> sending one CARP advertisement"
(
    cd "$REPO_DIR" || exit 1
    "$JCARP_BIN" --config "$CONFIG" send-once
) || exit 1

wait "$TCPDUMP_PID" || {
    printf '%s\n' "error: tcpdump did not capture a CARP advertisement" >&2
    cat "$TCPDUMP_LOG" >&2
    exit 1
}
TCPDUMP_PID=

if ! awk '/proto/ && /112/ && /ttl 255/ && /224\.0\.0\.18/ { found=1 } END { exit(found ? 0 : 1) }' "$TCPDUMP_LOG"; then
    printf '%s\n' "error: tcpdump did not confirm expected CARP packet fields" >&2
    printf '%s\n' "captured output:" >&2
    cat "$TCPDUMP_LOG" >&2
    exit 1
fi

printf '%s\n' "ok: tcpdump confirmed protocol 112 + ttl 255 + destination 224.0.0.18"
