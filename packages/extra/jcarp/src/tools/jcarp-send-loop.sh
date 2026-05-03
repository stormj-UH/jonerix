#!/bin/sh
set -eu

JCARP_BIN=${JCARP_BIN:-jcarp}
CONFIG=${CONFIG:-${TMPDIR:-/tmp}/jcarp-interop.conf}
INTERFACE=${INTERFACE:-eth0}
VHID=${VHID:-42}
ADVBASE=${ADVBASE:-1}
ADVSKEW=${ADVSKEW:-50}
DEMOTE=${DEMOTE:-0}
PREEMPT=${PREEMPT:-true}
PEER=${PEER:-224.0.0.18}
VIP=${VIP:-10.0.253.42}
PASSPHRASE=${PASSPHRASE:-interop-pass}
COUNT=${COUNT:-30}
INTERVAL=${INTERVAL:-1}
USE_SUDO=${JCARP_USE_SUDO:-1}

cat >"$CONFIG" <<EOF
interface=$INTERFACE
vhid=$VHID
advbase=$ADVBASE
advskew=$ADVSKEW
demote=$DEMOTE
preempt=$PREEMPT
peer=$PEER
vip=$VIP
passphrase=$PASSPHRASE
EOF

i=0
while [ "$i" -lt "$COUNT" ]; do
    if [ "$USE_SUDO" = "1" ]; then
        sudo -n "$JCARP_BIN" --config "$CONFIG" send-once
    else
        "$JCARP_BIN" --config "$CONFIG" send-once
    fi
    i=$((i + 1))
    if [ "$i" -lt "$COUNT" ]; then
        sleep "$INTERVAL"
    fi
done
