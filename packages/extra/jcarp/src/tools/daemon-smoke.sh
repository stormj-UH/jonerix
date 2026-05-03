#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
REPO_DIR=$(CDPATH= cd "$SCRIPT_DIR/.." && pwd)
TMP_DIR=${TMPDIR:-/tmp}

child_pid() {
    ps -o pid= -o ppid= | awk -v parent="$1" '$2 == parent { print $1; exit }'
}

daemon_alive() {
    if /bin/kill -0 "$JCARP_PID" >/dev/null 2>&1; then
        return 0
    fi
    CHILD_PID=$(child_pid "$JCARP_PID")
    if [ -n "$CHILD_PID" ] && /bin/kill -0 "$CHILD_PID" >/dev/null 2>&1; then
        return 0
    fi
    return 1
}

terminate_daemon() {
    CHILD_PID=$(child_pid "$JCARP_PID")
    if [ -n "$CHILD_PID" ]; then
        /bin/kill "$CHILD_PID" >/dev/null 2>&1 || true
    fi
    /bin/kill "$JCARP_PID" >/dev/null 2>&1 || true
}

cleanup() {
    if [ "${JCARP_PID:-}" ]; then
        terminate_daemon >/dev/null 2>&1 || true
        wait "$JCARP_PID" >/dev/null 2>&1 || true
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

need_cmd id
need_cmd /bin/kill
need_cmd mktemp
need_cmd awk
need_cmd ps
need_cmd sleep

if [ "$(id -u)" != "0" ]; then
    printf '%s\n' "error: daemon smoke requires root (raw CARP sockets)." >&2
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

INTERFACE=${INTERFACE:-eth0}
DURATION=${DURATION:-4}
WORK_DIR=$(mktemp -d "$TMP_DIR/jcarp-daemon.XXXXXX")
CONFIG="$WORK_DIR/jcarp-daemon.conf"
LOG="$WORK_DIR/jcarp-daemon.log"

cat >"$CONFIG" <<EOF
interface=$INTERFACE
vhid=42
advbase=1
advskew=50
demote=0
preempt=true
peer=224.0.0.18
vip=10.0.253.42/32
passphrase=daemon-smoke
manage_vip=false
announce=false
mac=interface
load_filter=off
EOF

printf '%s\n' "==> running jcarp daemon smoke on $INTERFACE for ${DURATION}s"
(
    exec "$JCARP_BIN" --config "$CONFIG" run
) >"$LOG" 2>&1 &
JCARP_PID=$!

/bin/sleep "$DURATION"

if ! daemon_alive; then
    printf '%s\n' "error: jcarp daemon exited early" >&2
    cat "$LOG" >&2
    exit 1
fi

terminate_daemon
wait "$JCARP_PID" >/dev/null 2>&1 || true
JCARP_PID=

if [ -s "$LOG" ]; then
    printf '%s\n' "error: jcarp daemon wrote unexpected output" >&2
    cat "$LOG" >&2
    exit 1
fi

printf '%s\n' "ok: daemon stayed up and stopped cleanly"
