#!/bin/sh
# stormwall iptables soak — hundreds of `iptables` invocations against the
# real netlink backend, then verify the resulting `nft list ruleset` matches
# expectations. Intended to run as root on a live jonerix box.
#
# Each test:
#   1. Flushes all nft tables
#   2. Runs the iptables command(s) under test
#   3. Captures `nft list ruleset` output
#   4. Greps for required tokens AND for forbidden tokens
#   5. Reports PASS/FAIL with the ruleset on failure
#
# Exit status: 0 if all pass, 1 if any fail.
#
# Run: sudo IPTABLES=/usr/sbin/iptables ./iptables-soak.sh

set -u

: "${IPTABLES:=/usr/sbin/iptables}"
: "${IP6TABLES:=/usr/sbin/ip6tables}"
: "${NFT:=/bin/nft}"
: "${SAVE:=/usr/sbin/iptables-save}"
: "${RESTORE:=/usr/sbin/iptables-restore}"

PASS=0
FAIL=0
FAIL_NAMES=""

# ── helpers ───────────────────────────────────────────────────────────

flush() {
    "$NFT" flush ruleset 2>/dev/null
}

# run_test "name" "want_token" "forbidden_token_or_empty" -- iptables-args...
run_test() {
    name=$1; want=$2; forbid=$3
    shift 3
    [ "$1" = "--" ] && shift
    flush
    if ! "$IPTABLES" "$@" 2>&1; then
        rc=$?
        printf '  FAIL %s [exit %d]\n' "$name" "$rc"
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES $name"
        return
    fi
    state=$("$NFT" list ruleset 2>/dev/null)
    if [ -n "$want" ]; then
        if ! printf '%s' "$state" | grep -qF -- "$want"; then
            printf '  FAIL %s [missing: %s]\n%s\n' "$name" "$want" "$state"
            FAIL=$((FAIL + 1))
            FAIL_NAMES="$FAIL_NAMES $name"
            return
        fi
    fi
    if [ -n "$forbid" ]; then
        if printf '%s' "$state" | grep -qF -- "$forbid"; then
            printf '  FAIL %s [unexpected: %s]\n%s\n' "$name" "$forbid" "$state"
            FAIL=$((FAIL + 1))
            FAIL_NAMES="$FAIL_NAMES $name"
            return
        fi
    fi
    PASS=$((PASS + 1))
}

# run_seq "name" "want_token" "forbid_or_empty" -- "cmd1" "cmd2" ...
# Each cmd is a single-string with iptables args; runs sequentially.
run_seq() {
    name=$1; want=$2; forbid=$3
    shift 3
    [ "$1" = "--" ] && shift
    flush
    for cmd; do
        # shellcheck disable=SC2086
        if ! "$IPTABLES" $cmd 2>&1; then
            rc=$?
            printf '  FAIL %s [step "%s" exit %d]\n' "$name" "$cmd" "$rc"
            FAIL=$((FAIL + 1))
            FAIL_NAMES="$FAIL_NAMES $name"
            return
        fi
    done
    state=$("$NFT" list ruleset 2>/dev/null)
    if [ -n "$want" ] && ! printf '%s' "$state" | grep -qF -- "$want"; then
        printf '  FAIL %s [missing: %s]\n%s\n' "$name" "$want" "$state"
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES $name"
        return
    fi
    if [ -n "$forbid" ] && printf '%s' "$state" | grep -qF -- "$forbid"; then
        printf '  FAIL %s [unexpected: %s]\n%s\n' "$name" "$forbid" "$state"
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES $name"
        return
    fi
    PASS=$((PASS + 1))
}

# expect_fail "name" -- iptables-args...
# The command MUST fail with non-zero exit.
expect_fail() {
    name=$1; shift
    [ "$1" = "--" ] && shift
    flush
    if "$IPTABLES" "$@" 2>/dev/null; then
        printf '  FAIL %s [expected failure but exit was 0]\n' "$name"
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES $name"
        return
    fi
    PASS=$((PASS + 1))
}

# ── Section 1: built-in chain operations ──────────────────────────────

printf '=== 1. Built-in chains, simple targets ===\n'
run_test  "1.01-input-accept-all"          "accept" "" -- -A INPUT -j ACCEPT
run_test  "1.02-input-drop-all"            "drop"   "" -- -A INPUT -j DROP
run_test  "1.03-output-return"             "return" "" -- -A OUTPUT -j RETURN
run_test  "1.04-forward-reject"            "reject" "" -- -A FORWARD -j REJECT
run_test  "1.05-forward-reject-port-unreachable" "reject with icmp" "" -- -A FORWARD -j REJECT --reject-with icmp-port-unreachable
run_test  "1.06-input-accept-tcp-22"       "tcp dport 22" "" -- -A INPUT -p tcp --dport 22 -j ACCEPT
run_test  "1.07-input-accept-udp-53"       "udp dport 53" "" -- -A INPUT -p udp --dport 53 -j ACCEPT
run_test  "1.08-input-accept-from-net"     "ip saddr 10.0.0.0/8" "" -- -A INPUT -s 10.0.0.0/8 -j ACCEPT
run_test  "1.09-output-to-host"            "ip daddr 192.168.1.1" "" -- -A OUTPUT -d 192.168.1.1 -j ACCEPT
run_test  "1.10-input-iface-eth0"          "iifname \"eth0\"" "" -- -A INPUT -i eth0 -j ACCEPT
run_test  "1.11-output-iface-eth1"         "oifname \"eth1\"" "" -- -A OUTPUT -o eth1 -j ACCEPT

# ── Section 2: negation ──────────────────────────────────────────────

printf '=== 2. Negation (modern + legacy) ===\n'
run_test  "2.01-modern-not-source"         "ip saddr != 10.0.0.0/8" "" -- -A FORWARD ! -s 10.0.0.0/8 -j DROP
run_test  "2.02-legacy-not-source"         "ip saddr != 10.0.0.0/8" "" -- -A FORWARD -s ! 10.0.0.0/8 -j DROP
run_test  "2.03-modern-not-dest-iface"     "oifname != \"docker0\"" "" -- -A FORWARD ! -o docker0 -j DROP
run_test  "2.04-legacy-not-dest-iface"     "oifname != \"docker0\"" "" -- -A FORWARD -o ! docker0 -j DROP
run_test  "2.05-modern-not-dport"          "tcp dport != 22" "" -- -A INPUT -p tcp ! --dport 22 -j DROP
run_test  "2.06-legacy-not-dport"          "tcp dport != 22" "" -- -A INPUT -p tcp --dport ! 22 -j DROP
run_test  "2.07-not-protocol"              "meta l4proto != tcp" "" -- -A INPUT ! -p tcp -j ACCEPT

# ── Section 3: nat table ──────────────────────────────────────────────

printf '=== 3. NAT table ===\n'
run_test  "3.01-masquerade-out"            "masquerade" "" -- -t nat -A POSTROUTING -o eth0 -j MASQUERADE
run_test  "3.02-masquerade-from-net"       "ip saddr 172.17.0.0/16" "" -- -t nat -A POSTROUTING -s 172.17.0.0/16 ! -o docker0 -j MASQUERADE
run_test  "3.03-snat-to-source"            "snat to" "" -- -t nat -A POSTROUTING -s 10.0.0.0/24 -j SNAT --to-source 1.2.3.4
run_test  "3.04-dnat-to-dest"              "dnat to" "" -- -t nat -A PREROUTING -p tcp --dport 80 -j DNAT --to-destination 10.0.0.5:8080
run_test  "3.05-redirect-to-port"          "redirect to" "" -- -t nat -A PREROUTING -p tcp --dport 80 -j REDIRECT --to-ports 8080

# ── Section 4: user chains and jumps ──────────────────────────────────

printf '=== 4. User chains ===\n'
run_test  "4.01-create-user-chain"         "chain DOCKER {" "" -- -N DOCKER
run_test  "4.02-append-to-user-chain"      "chain DOCKER {" "" -- -A DOCKER -j RETURN
run_test  "4.03-insert-jump-to-user"       "jump DOCKER" "" -- -I FORWARD -o docker0 -j DOCKER
run_test  "4.04-append-jump-to-user"       "jump DOCKER-USER" "" -- -A FORWARD -j DOCKER-USER
run_test  "4.05-jump-target-auto-creates"  "chain DOCKER-ISOLATION-STAGE-1 {" "" -- -A FORWARD -j DOCKER-ISOLATION-STAGE-1
run_test  "4.06-nat-table-user-chain"      "chain DOCKER {" "" -- -t nat -A DOCKER -i docker0 -j RETURN
run_seq   "4.07-create-then-append"        "jump MYCHAIN" "" -- "-N MYCHAIN" "-A MYCHAIN -j RETURN" "-I INPUT -j MYCHAIN"
run_seq   "4.08-create-flush-readd"        "ct state established,related accept" "" -- \
    "-N FILTER-CHAIN" "-F FILTER-CHAIN" "-A FILTER-CHAIN -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT"

# ── Section 5: conntrack / set literals ───────────────────────────────

printf '=== 5. Conntrack states (set literal coverage) ===\n'
run_test  "5.01-ctstate-single"            "ct state established" "" -- -A INPUT -m conntrack --ctstate ESTABLISHED -j ACCEPT
run_test  "5.02-ctstate-pair"              "ct state" "" -- -A INPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
run_test  "5.03-ctstate-three"             "ct state" "" -- -A INPUT -m conntrack --ctstate NEW,ESTABLISHED,RELATED -j ACCEPT
run_test  "5.04-state-module-alias"        "ct state" "" -- -A INPUT -m state --state ESTABLISHED,RELATED -j ACCEPT
run_test  "5.05-ctstate-not"               "ct state !=" "" -- -A INPUT -m conntrack ! --ctstate INVALID -j ACCEPT

# ── Section 6: match modules ──────────────────────────────────────────

printf '=== 6. Match modules ===\n'
run_test  "6.01-multiport-dports"          "tcp dport { 80, 443 }" "" -- -A INPUT -p tcp -m multiport --dports 80,443 -j ACCEPT
run_test  "6.02-multiport-sports"          "udp sport" "" -- -A INPUT -p udp -m multiport --sports 53,5353 -j ACCEPT
run_test  "6.03-mac-source"                "ether saddr" "" -- -A INPUT -m mac --mac-source 00:11:22:33:44:55 -j ACCEPT
run_test  "6.04-mark-match"                "meta mark" "" -- -A INPUT -m mark --mark 0x10 -j ACCEPT
run_test  "6.05-tcp-syn"                   "tcp flags" "" -- -A INPUT -p tcp --syn -j ACCEPT
run_test  "6.06-port-range"                "tcp dport 1000-2000" "" -- -A INPUT -p tcp --dport 1000:2000 -j ACCEPT
run_test  "6.07-icmp-type-echo"            "icmp type echo-request" "" -- -A INPUT -p icmp --icmp-type echo-request -j ACCEPT
run_test  "6.08-limit-rate"                "limit rate" "" -- -A INPUT -m limit --limit 5/sec -j ACCEPT

# ── Section 7: targets ────────────────────────────────────────────────

printf '=== 7. Target variants ===\n'
run_test  "7.01-mark-set"                  "meta mark set 0x10" "" -- -t mangle -A PREROUTING -j MARK --set-mark 0x10
run_test  "7.02-log-prefix"                "log prefix" "" -- -A INPUT -j LOG --log-prefix "DROP: "
run_test  "7.03-reject-tcp-reset"          "reject with tcp" "" -- -A INPUT -p tcp -j REJECT --reject-with tcp-reset

# ── Section 8: -P policy ──────────────────────────────────────────────

printf '=== 8. Default policy ===\n'
run_seq   "8.01-policy-forward-drop"       "policy drop" "" -- "-P FORWARD DROP"
run_seq   "8.02-policy-input-accept"       "policy accept" "" -- "-P INPUT ACCEPT"
expect_fail "8.03-policy-on-user-chain" -- -P MYCHAIN DROP
expect_fail "8.04-policy-invalid-target" -- -P INPUT QUEUE

# ── Section 9: chain lifecycle ────────────────────────────────────────

printf '=== 9. Chain lifecycle ===\n'
run_seq   "9.01-create-then-delete"        ""        "chain DOCKER" -- "-N DOCKER" "-X DOCKER"
run_seq   "9.02-create-flush-delete"       ""        "chain TMP"    -- "-N TMP" "-A TMP -j RETURN" "-F TMP" "-X TMP"
expect_fail "9.03-delete-builtin-chain"  -- -X INPUT
expect_fail "9.04-create-existing-builtin" -- -N INPUT

# ── Section 10: -D delete by spec ─────────────────────────────────────

printf '=== 10. -D rule deletion ===\n'
run_seq   "10.01-add-then-delete-by-spec" ""         "tcp dport 22" -- \
    "-A INPUT -p tcp --dport 22 -j ACCEPT" "-D INPUT -p tcp --dport 22 -j ACCEPT"
run_seq   "10.02-add-then-delete-by-num"  ""         "tcp dport 22" -- \
    "-A INPUT -p tcp --dport 22 -j ACCEPT" "-D INPUT 1"

# ── Section 11: -C check rule existence ───────────────────────────────

printf '=== 11. -C check ===\n'
run_seq   "11.01-check-existing-rule"      ""        "" -- \
    "-A INPUT -p tcp --dport 22 -j ACCEPT" "-C INPUT -p tcp --dport 22 -j ACCEPT"
# 11.02: rule doesn't exist → -C should exit non-zero
flush; "$IPTABLES" -A INPUT -p tcp --dport 22 -j ACCEPT >/dev/null 2>&1
if "$IPTABLES" -C INPUT -p tcp --dport 80 -j ACCEPT 2>/dev/null; then
    printf '  FAIL 11.02-check-missing-rule [-C returned 0 for absent rule]\n'
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 11.02-check-missing-rule"
else
    PASS=$((PASS + 1))
fi

# ── Section 12: ip6tables family ──────────────────────────────────────

printf '=== 12. ip6tables ===\n'
flush
if "$IP6TABLES" -A FORWARD -j ACCEPT 2>&1; then
    state=$("$NFT" list ruleset 2>/dev/null)
    if printf '%s' "$state" | grep -qF "table ip6 filter" && \
       printf '%s' "$state" | grep -qF "accept"; then
        PASS=$((PASS + 1))
    else
        printf '  FAIL 12.01-ip6tables-basic\n%s\n' "$state"
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES 12.01-ip6tables-basic"
    fi
else
    printf '  FAIL 12.01-ip6tables-basic [exit nonzero]\n'
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 12.01-ip6tables-basic"
fi
flush
if "$IP6TABLES" -A FORWARD -s fe80::/64 -j ACCEPT 2>&1; then
    state=$("$NFT" list ruleset 2>/dev/null)
    if printf '%s' "$state" | grep -qF "ip6 saddr fe80::/64"; then
        PASS=$((PASS + 1))
    else
        printf '  FAIL 12.02-ip6tables-link-local\n%s\n' "$state"
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES 12.02-ip6tables-link-local"
    fi
fi

# ── Section 13: docker first-start corpus (smoke) ─────────────────────

printf '=== 13. Docker first-start corpus ===\n'
flush
ok=1
{
    "$IPTABLES" -t filter -N DOCKER &&
    "$IPTABLES" -t filter -N DOCKER-USER &&
    "$IPTABLES" -t filter -N DOCKER-ISOLATION-STAGE-1 &&
    "$IPTABLES" -t filter -N DOCKER-ISOLATION-STAGE-2 &&
    "$IPTABLES" -t filter -A FORWARD -j DOCKER-USER &&
    "$IPTABLES" -t filter -A FORWARD -j DOCKER-ISOLATION-STAGE-1 &&
    "$IPTABLES" -t filter -A FORWARD -o docker0 -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT &&
    "$IPTABLES" -t filter -A FORWARD -o docker0 -j DOCKER &&
    "$IPTABLES" -t filter -A FORWARD -i docker0 ! -o docker0 -j ACCEPT &&
    "$IPTABLES" -t filter -A FORWARD -i docker0 -o docker0 -j ACCEPT &&
    "$IPTABLES" -t filter -A DOCKER-ISOLATION-STAGE-1 -j RETURN &&
    "$IPTABLES" -t filter -A DOCKER-ISOLATION-STAGE-2 -j RETURN &&
    "$IPTABLES" -t filter -A DOCKER-USER -j RETURN &&
    "$IPTABLES" -t nat -N DOCKER &&
    "$IPTABLES" -t nat -A POSTROUTING -s 172.17.0.0/16 ! -o docker0 -j MASQUERADE &&
    "$IPTABLES" -t nat -A PREROUTING -m addrtype --dst-type LOCAL -j DOCKER &&
    "$IPTABLES" -t nat -A OUTPUT -m addrtype --dst-type LOCAL -j DOCKER ! --dst 127.0.0.0/8 &&
    "$IPTABLES" -t nat -A DOCKER -i docker0 -j RETURN
} >/dev/null 2>&1 || ok=0

if [ "$ok" = 1 ]; then
    PASS=$((PASS + 1))
    printf '  ok 13.01-docker-first-start-full\n'
else
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 13.01-docker-first-start-full"
    printf '  FAIL 13.01-docker-first-start-full\n'
    "$NFT" list ruleset 2>&1 | head -30
fi

# ── Section 14: iptables-save / restore round-trip ────────────────────

printf '=== 14. iptables-save / -restore round-trip ===\n'
flush
"$IPTABLES" -t filter -A INPUT -p tcp --dport 22 -j ACCEPT >/dev/null 2>&1
"$IPTABLES" -t filter -A FORWARD -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT >/dev/null 2>&1
"$IPTABLES" -t nat -A POSTROUTING -s 172.17.0.0/16 -j MASQUERADE >/dev/null 2>&1
saved=$("$SAVE" 2>&1)
flush
if printf '%s\n' "$saved" | "$RESTORE" 2>&1; then
    state=$("$NFT" list ruleset 2>/dev/null)
    if printf '%s' "$state" | grep -qF "tcp dport 22" && \
       printf '%s' "$state" | grep -qF "ct state" && \
       printf '%s' "$state" | grep -qF "masquerade"; then
        PASS=$((PASS + 1))
        printf '  ok 14.01-save-restore-roundtrip\n'
    else
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES 14.01-save-restore-roundtrip"
        printf '  FAIL 14.01-save-restore-roundtrip [content drift]\n%s\n' "$state"
    fi
else
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 14.01-save-restore-roundtrip"
    printf '  FAIL 14.01-save-restore-roundtrip [restore failed]\n'
fi

# ── final tally ───────────────────────────────────────────────────────

flush
TOTAL=$((PASS + FAIL))
printf '\n========================================\n'
printf 'soak result: %d/%d passed\n' "$PASS" "$TOTAL"
if [ "$FAIL" -gt 0 ]; then
    printf 'failed: %s\n' "$FAIL_NAMES"
    exit 1
fi
exit 0
