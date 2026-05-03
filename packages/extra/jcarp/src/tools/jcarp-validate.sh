#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
REPO_DIR=$(CDPATH= cd "$SCRIPT_DIR/.." && pwd)

usage() {
    printf '%s\n' "usage: $0 <local|smoke|daemon|all>"
}

run_local() {
    printf '%s\n' "==> local: cargo test --locked --lib --bins --tests"
    (
        cd "$REPO_DIR"
        cargo test --locked --lib --bins --tests
    )
}

run_smoke() {
    printf '%s\n' "==> smoke: privileged packet validation"
    "$SCRIPT_DIR/packet-smoke.sh"
}

run_daemon() {
    printf '%s\n' "==> daemon: privileged runtime validation"
    "$SCRIPT_DIR/daemon-smoke.sh"
}

mode=${1:-all}

case "$mode" in
    local)
        run_local
        ;;
    smoke)
        run_smoke
        ;;
    daemon)
        run_daemon
        ;;
    all)
        run_local
        run_smoke
        run_daemon
        ;;
    -h|--help|help)
        usage
        ;;
    *)
        usage
        exit 1
        ;;
esac
