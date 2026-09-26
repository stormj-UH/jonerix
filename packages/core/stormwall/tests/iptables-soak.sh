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
#
# Preconditions for a full run. Without them the affected tests fail on
# the environment, not on stormwall:
#   - root, or a passwordless sudo (uid 0 is used directly when it is
#     already uid 0, so a root shell needs no sudo at all)
#   - CAP_NET_ADMIN in the netns under test, and a writable nft ruleset
#   - nf_conntrack_ftp loaded, for the `-j CT --helper ftp` test
#   - nft_log / nf_log_syslog, nft_reject, nft_limit, nft_fib

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

# priv CMD ... — run CMD with root privilege. Already uid 0 (the usual
# case: the soak needs CAP_NET_ADMIN anyway) runs it directly, so the
# script does not need sudo installed to be reproducible.
priv() {
    if [ "$(id -u)" = "0" ]; then
        "$@"
    else
        sudo "$@"
    fi
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

# expect_refused BINARY "name" -- args...
# The command MUST fail AND leave no rule behind: 1.1.13 refuses what
# its nft parser cannot encode instead of installing a different rule.
expect_refused() {
    bin=$1; name=$2; shift 2
    [ "$1" = "--" ] && shift
    flush
    if "$bin" "$@" 2>/dev/null; then
        printf '  FAIL %s [expected refusal but exit was 0]\n%s\n' "$name" "$("$NFT" list ruleset 2>/dev/null)"
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES $name"
        return
    fi
    left=$("$NFT" list ruleset 2>/dev/null | grep -v -E '^[[:space:]]*(table |chain |type |}|$)')
    if [ -n "$left" ]; then
        printf '  FAIL %s [refused but left rules behind]\n%s\n' "$name" "$left"
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
expect_refused "$IPTABLES" "3.05-redirect-to-port" -- -t nat -A PREROUTING -p tcp --dport 80 -j REDIRECT --to-ports 8080   # 1.1.12: the match, no redirect

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
run_test  "6.03-mac-source"                "ether saddr 00:11:22:33:44:55 accept" "" -- -A INPUT -m mac --mac-source 00:11:22:33:44:55 -j ACCEPT
run_test  "6.04-mark-match"                "meta mark" "" -- -A INPUT -m mark --mark 0x10 -j ACCEPT
# Up to 1.1.12 the flags mask was 0 (`tcp flags & (0) == syn`).
run_test  "6.05-tcp-syn"                   "tcp flags & (fin | syn | rst | ack) == syn accept" "(0)" -- -A INPUT -p tcp --syn -j ACCEPT
run_test  "6.06-port-range"                "tcp dport 1000-2000" "" -- -A INPUT -p tcp --dport 1000:2000 -j ACCEPT
run_test  "6.07-icmp-type-echo"            "icmp type echo-request" "" -- -A INPUT -p icmp --icmp-type echo-request -j ACCEPT
run_test  "6.08-limit-rate"                "limit rate" "" -- -A INPUT -m limit --limit 5/sec -j ACCEPT
# -p with no port match. Up to 1.1.12 these installed a rule with no
# protocol test and no verdict, and `gre` became protocol 103 (pim).
run_test  "6.09-proto-only-drop"           "meta l4proto tcp drop" "" -- -A INPUT -p tcp -j DROP
run_test  "6.10-proto-name-gre"            "meta l4proto gre accept" "pim" -- -A INPUT -p gre -j ACCEPT
run_test  "6.11-tcp-flags-all-none"        "tcp flags & (fin | syn | rst | psh | ack | urg) == 0x0 drop" "" -- -A INPUT -p tcp --tcp-flags ALL NONE -j DROP
# `! --syn` lost its `!` up to 1.1.12, or the `!` moved onto --state;
# with the flags mask fixed the first rule dropped every new connection.
run_test  "6.12-not-syn-after-ctstate"     "tcp flags & (fin | syn | rst | ack) != syn ct state new drop" "ct state != new" -- -A INPUT -p tcp -m conntrack --ctstate NEW ! --syn -j DROP
run_test  "6.13-not-syn-before-state"      "tcp flags & (fin | syn | rst | ack) != syn ct state new drop" "ct state != new" -- -A INPUT -p tcp ! --syn -m state --state NEW -j DROP
run_test  "6.14-addrtype-not-local"        "fib saddr type != local drop" "" -- -A INPUT -m addrtype ! --src-type LOCAL -j DROP
expect_fail "6.15-bang-on-limit"           -- -A INPUT -m limit ! --limit 5/sec -j ACCEPT
expect_fail "6.16-bang-on-uid-owner"       -- -A OUTPUT -m owner ! --uid-owner 0 -j ACCEPT
expect_fail "6.17-uid-owner-name"          -- -A OUTPUT -m owner --uid-owner nobody -j ACCEPT

# ── Section 7: targets ────────────────────────────────────────────────

printf '=== 7. Target variants ===\n'
run_test  "7.01-mark-set"                  "meta mark set 0x00000010" "" -- -t mangle -A PREROUTING -j MARK --set-mark 0x10
run_test  "7.02-log-prefix"                "log prefix" "" -- -A INPUT -j LOG --log-prefix "DROP: "
run_test  "7.03-reject-tcp-reset"          "meta l4proto tcp reject with tcp reset" "" -- -A INPUT -p tcp -j REJECT --reject-with tcp-reset
# Masked marks as tailscale (ipt-default) and CNI portmap install them.
# Up to 1.1.12 the match became `meta mark 0x00000026` (the `&` byte)
# and the setter took its value from whatever register 1 held.
run_test  "7.04-mark-set-masked"           "iifname \"tailscale0\" meta mark set meta mark & 0xff04ffff | 0x00040000" "" -- -A FORWARD -i tailscale0 -j MARK --set-mark 0x40000/0xff0000
run_test  "7.05-mark-match-masked"         "meta mark & 0x00ff0000 == 0x00040000 accept" "0x00000026" -- -A FORWARD -m mark --mark 0x40000/0xff0000 -j ACCEPT
run_test  "7.06-mark-set-xmark"            "meta mark set meta mark | 0x00002000" "" -- -t nat -A POSTROUTING -j MARK --set-xmark 0x2000/0x2000
run_test  "7.07-connmark-match-masked"     "ct mark & 0x00ff0000 == 0x00040000 accept" "0x00000026" -- -A FORWARD -m connmark --mark 0x40000/0xff0000 -j ACCEPT
# LOG levels and NFLOG groups. Up to 1.1.12 the level was dropped (and
# a named level read as 0), --log-uid became a rule comment, and NFLOG
# without --nflog-group logged to syslog instead of nfnetlink_log.
run_test  "7.08-log-level-named"           "log prefix \"IN \" level info" "" -- -A INPUT -j LOG --log-prefix "IN " --log-level info
run_test  "7.09-log-level-numeric"         "limit rate 5/minute burst 5 packets log prefix \"FW: \" level debug" "" -- -A INPUT -m limit --limit 5/min -j LOG --log-prefix "FW: " --log-level 7
run_test  "7.10-log-level-default"         "log prefix \"W \"" "level" -- -A INPUT -j LOG --log-prefix "W " --log-level warning
run_test  "7.11-log-uid"                   "log flags skuid" "comment" -- -A INPUT -j LOG --log-uid
run_test  "7.12-nflog-group-prefix"        "log prefix \"x\" group 5" "" -- -A INPUT -j NFLOG --nflog-group 5 --nflog-prefix x
run_test  "7.13-nflog-default-group"       "log group 0" "" -- -A INPUT -j NFLOG
run_test  "7.14-nflog-size-threshold"      "log group 2 snaplen 128 queue-threshold 10" "" -- -A INPUT -j NFLOG --nflog-group 2 --nflog-size 128 --nflog-threshold 10
expect_fail "7.15-log-level-out-of-range"  -- -A INPUT -j LOG --log-level 8

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

# ── Section 15: 1.1.5 feature additions ──────────────────────────────

printf '=== 15. 1.1.5 features ===\n'

# 15.01: -E rename (requires the chain to exist first).
flush
priv "$IPTABLES" -N OLDNAME 2>/dev/null
if priv "$IPTABLES" -E OLDNAME NEWNAME 2>&1; then
    state=$("$NFT" list ruleset 2>/dev/null)
    if printf '%s' "$state" | grep -qF "chain NEWNAME" && \
       ! printf '%s' "$state" | grep -qF "chain OLDNAME"; then
        PASS=$((PASS + 1))
        printf '  ok 15.01-rename-chain\n'
    else
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES 15.01-rename-chain"
        printf '  FAIL 15.01-rename-chain\n%s\n' "$state"
    fi
else
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 15.01-rename-chain"
    printf '  FAIL 15.01-rename-chain [exit nonzero]\n'
fi

# Compat parsing + new targets/matches.
run_test  "15.02-set-counters-compat"      "tcp dport 22"                    "" -- -A INPUT -c 0 0 -p tcp --dport 22 -j ACCEPT
run_test  "15.03-masquerade-random-fully"  "fully-random"                    "" -- -t nat -A POSTROUTING -o eth0 -j MASQUERADE --random-fully
run_test  "15.04-iprange-src"              "ip saddr 10.0.0.1-10.0.0.100"    "" -- -A FORWARD -m iprange --src-range 10.0.0.1-10.0.0.100 -j ACCEPT
run_test  "15.05-length-single"            "meta length 100"                 "" -- -A INPUT -m length --length 100 -j ACCEPT
run_test  "15.06-length-range"             "meta length 200-500"             "" -- -A INPUT -m length --length 200:500 -j ACCEPT
run_test  "15.07-pkttype-broadcast"        "meta pkttype broadcast"          "" -- -A INPUT -m pkttype --pkt-type broadcast -j DROP
run_test  "15.08-connmark-match"           "ct mark"                         "" -- -A INPUT -m connmark --mark 0x10 -j ACCEPT
run_test  "15.09-notrack-target"           "notrack"                         "" -- -t raw -A PREROUTING -p udp -j NOTRACK
run_test  "15.10-nfqueue-num"              "queue num 5"                     "" -- -A INPUT -j NFQUEUE --queue-num 5
expect_refused "$IPTABLES" "15.11-nfqueue-balance" -- -A INPUT -j NFQUEUE --queue-balance 0:3 --queue-bypass             # 1.1.12: queue to 0
run_test  "15.12-ct-helper"                "ct helper set"                   "" -- -t raw -A PREROUTING -p tcp --dport 21 -j CT --helper ftp
run_test  "15.13-ct-notrack"               "notrack"                         "" -- -t raw -A PREROUTING -p udp --dport 53 -j CT --notrack
expect_refused "$IPTABLES" "15.14-tcpmss-clamp" -- -t mangle -A FORWARD -p tcp --tcp-flags SYN,RST SYN -j TCPMSS --clamp-mss-to-pmtu  # 1.1.12: the match, no clamp
expect_refused "$IPTABLES" "15.15-tcpmss-set" -- -t mangle -A FORWARD -j TCPMSS --set-mss 1400                            # 1.1.12: no MSS change
expect_refused "$IPTABLES" "15.16-tcpmss-match" -- -A FORWARD -p tcp -m tcpmss --mss 1400 -j DROP                         # 1.1.12: bare drop
run_test  "15.17-log-tcp-options"          "flags tcp"                       "" -- -A INPUT -j LOG --log-prefix "DROP: " --log-tcp-sequence --log-tcp-options

# 15.18: -X no-arg
flush
priv "$IPTABLES" -N TMP1 2>/dev/null
priv "$IPTABLES" -N TMP2 2>/dev/null
if priv "$IPTABLES" -X 2>&1; then
    state=$("$NFT" list ruleset 2>/dev/null)
    if ! printf '%s' "$state" | grep -qE "chain (TMP1|TMP2)"; then
        PASS=$((PASS + 1))
        printf '  ok 15.18-delete-all-empty-user-chains\n'
    else
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES 15.18-delete-all-empty-user-chains"
        printf '  FAIL 15.18-delete-all-empty-user-chains\n%s\n' "$state"
    fi
else
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 15.18-delete-all-empty-user-chains"
    printf '  FAIL 15.18-delete-all-empty-user-chains [exit nonzero]\n'
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

# ── Section 16: refused by 1.1.13 ─────────────────────────────────────
#
# Each of these installed a different rule on 1.1.12, without an error
# (the comment says what); several matched every packet.

printf '=== 16. Refused, not installed wrong ===\n'
expect_refused "$IPTABLES"  "16.01-redirect-bare"         -- -t nat -A PREROUTING -p tcp --dport 80 -j REDIRECT            # match, no redirect
expect_refused "$IPTABLES"  "16.02-masquerade-to-ports"   -- -t nat -A POSTROUTING -p tcp -j MASQUERADE --to-ports 1024-65535 # no masquerade
expect_refused "$IPTABLES"  "16.03-dnat-port-range"       -- -t nat -A PREROUTING -p tcp --dport 8000:8010 -j DNAT --to-destination 10.0.0.2:9000-9010 # first port only
expect_refused "$IPTABLES"  "16.04-snat-port-range"       -- -t nat -A POSTROUTING -o eth0 -j SNAT --to-source 1.2.3.4:1024-2048 # first port only
expect_refused "$IP6TABLES" "16.05-ip6-dnat-address"      -- -t nat -A PREROUTING -p tcp --dport 80 -j DNAT --to-destination fd00::2 # dnat to :0
expect_refused "$IP6TABLES" "16.06-ip6-dnat-bracketed"    -- -t nat -A PREROUTING -p tcp --dport 80 -j DNAT --to-destination "[fd00::2]:8080" # dnat to :0
expect_refused "$IPTABLES"  "16.07-nfqueue-balance"       -- -A INPUT -j NFQUEUE --queue-balance 0:3                       # queue to 0
expect_refused "$IPTABLES"  "16.08-ttl-inc"               -- -t mangle -A POSTROUTING -j TTL --ttl-inc 1                   # ip ttl set 6909952
expect_refused "$IP6TABLES" "16.09-hl-set"                -- -t mangle -A POSTROUTING -j HL --hl-set 64                    # hop-limit match
expect_refused "$IPTABLES"  "16.10-ttl-lt"                -- -A INPUT -m ttl --ttl-lt 5 -j DROP                            # every IPv4 packet
expect_refused "$IP6TABLES" "16.11-hl-gt"                 -- -A INPUT -m hl --hl-gt 5 -j DROP                              # every IPv6 packet
expect_refused "$IPTABLES"  "16.12-connlimit"             -- -A INPUT -p tcp -m connlimit --connlimit-above 10 --connlimit-mask 24 -j DROP # bare drop
expect_refused "$IPTABLES"  "16.13-sctp-dport"            -- -A INPUT -p sctp --dport 9 -j ACCEPT                          # bare accept
expect_refused "$IPTABLES"  "16.14-esp-spi"               -- -A INPUT -p esp -m esp --espspi 100 -j ACCEPT                 # no SPI test
expect_refused "$IP6TABLES" "16.15-frag-more"             -- -A INPUT -m frag --fragmore -j DROP                           # bare drop
expect_refused "$IPTABLES"  "16.16-fragment"              -- -A INPUT -f -j DROP                                           # bare drop
# iptables-restore is one batch: one refused line and nothing installs.
flush
if printf '*filter\n:INPUT ACCEPT [0:0]\n:FORWARD ACCEPT [0:0]\n:OUTPUT ACCEPT [0:0]\n-A INPUT -p tcp --dport 22 -j ACCEPT\n-A INPUT -m limit --limit 5/min -j LOG --log-prefix "IN " --log-level 4\n-A INPUT -m ttl --ttl-lt 5 -j DROP\nCOMMIT\n' | "$RESTORE" 2>/dev/null; then
    printf '  FAIL 16.17-restore-all-or-nothing [restore exit was 0]\n'
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 16.17-restore-all-or-nothing"
elif "$NFT" list ruleset 2>/dev/null | grep -qF "dport 22"; then
    printf '  FAIL 16.17-restore-all-or-nothing [part of the file installed]\n'
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 16.17-restore-all-or-nothing"
else
    PASS=$((PASS + 1))
fi

# ── Section 17: 1.1.14 — a match with no clause is refused ────────────
#
# Each of these exited 0 on 1.1.13 and installed its verdict with no
# match at all, i.e. applied to every packet.

printf '=== 17. A match that lowers to no clause ===\n'
expect_refused "$IP6TABLES" "17.01-hbh-bare"          -- -A INPUT -m hbh -j DROP
expect_refused "$IP6TABLES" "17.02-dst-bare"          -- -A INPUT -m dst -j DROP
expect_refused "$IP6TABLES" "17.03-mh-bare"           -- -A INPUT -m mh -j DROP
expect_refused "$IP6TABLES" "17.04-frag-bare"         -- -A INPUT -m frag -j DROP
expect_refused "$IP6TABLES" "17.05-rt-bare"           -- -A INPUT -m rt -j DROP
expect_refused "$IPTABLES"  "17.06-esp-bare"          -- -A INPUT -m esp -j DROP
expect_refused "$IPTABLES"  "17.07-ah-bare"           -- -A INPUT -m ah -j DROP
expect_refused "$IP6TABLES" "17.08-hbh-opts-comment"  -- -A INPUT -m hbh --hbh-opts 5 -j DROP
expect_refused "$IP6TABLES" "17.09-dst-opts-comment"  -- -A INPUT -m dst --dst-opts 5 -j DROP
expect_refused "$IP6TABLES" "17.10-frag-last-comment" -- -A INPUT -m frag --fraglast -j DROP
expect_refused "$IPTABLES"  "17.11-physdev-in"        -- -A FORWARD -m physdev --physdev-in eth0 -j DROP
expect_refused "$IPTABLES"  "17.12-physdev-is-bridged" -- -A FORWARD -m physdev --physdev-is-bridged -j DROP
expect_refused "$IPTABLES"  "17.13-socket"            -- -A INPUT -m socket -j ACCEPT
expect_refused "$IPTABLES"  "17.14-time-start-only"   -- -A INPUT -m time --timestart 10:00 -j ACCEPT
expect_refused "$IPTABLES"  "17.15-pkttype-bare"      -- -A INPUT -m pkttype -j DROP
expect_refused "$IPTABLES"  "17.16-policy-ipsec"      -- -A INPUT -m policy --dir in --pol ipsec -j ACCEPT
expect_refused "$IPTABLES"  "17.17-string"            -- -A INPUT -m string --string abc --algo bm -j DROP
expect_refused "$IPTABLES"  "17.18-u32-bare"          -- -A INPUT -m u32 -j DROP
expect_refused "$IPTABLES"  "17.19-quota-bare"        -- -A INPUT -m quota -j DROP
expect_refused "$IPTABLES"  "17.20-hashlimit-bare"    -- -A INPUT -m hashlimit -j DROP
expect_refused "$IPTABLES"  "17.21-connbytes"         -- -A FORWARD -m connbytes --connbytes 100 --connbytes-mode packets -j DROP
expect_refused "$IP6TABLES" "17.22-rt-type"           -- -A INPUT -m rt --rt-type 0 -j DROP
# Accepted more loosely than iptables until 1.1.14.
expect_refused "$IPTABLES"  "17.23-dnat-port-no-proto" -- -t nat -A PREROUTING -j DNAT --to-destination 10.0.0.5:8080
expect_refused "$IPTABLES"  "17.24-snat-port-no-proto" -- -t nat -A POSTROUTING -j SNAT --to-source 10.0.0.5:8080
expect_refused "$IPTABLES"  "17.25-notrack-filter"     -- -t filter -A INPUT -p udp -j NOTRACK
# The native nft front-end had the same hole: an unknown meta key fell
# through to NFT_META_LEN, so the rule installed a bare verdict. The
# table and chain are created first so a refusal is the only reason
# this can fail.
flush
"$NFT" add table ip t 2>/dev/null
"$NFT" add chain ip t c 2>/dev/null
if "$NFT" add rule ip t c meta ibrname "eth0" drop 2>/dev/null; then
    printf '  FAIL 17.26-nft-unknown-meta-key [expected refusal but exit was 0]\n%s\n' \
        "$("$NFT" list ruleset 2>/dev/null)"
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 17.26-nft-unknown-meta-key"
elif "$NFT" list ruleset 2>/dev/null | grep -qE '^[[:space:]]*drop$'; then
    printf '  FAIL 17.26-nft-unknown-meta-key [refused but left a bare drop]\n'
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 17.26-nft-unknown-meta-key"
else
    PASS=$((PASS + 1))
fi
# A meta key it does know still installs through the same path.
flush
"$NFT" add table ip t 2>/dev/null
"$NFT" add chain ip t c 2>/dev/null
if "$NFT" add rule ip t c meta mark 0x40 drop 2>/dev/null &&
   "$NFT" list ruleset 2>/dev/null | grep -qE "meta mark 0x0*40 drop"; then
    PASS=$((PASS + 1))
else
    printf '  FAIL 17.31-nft-known-meta-key\n'
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 17.31-nft-known-meta-key"
fi
# And the matches that must keep installing.
run_test "17.27-pkttype-broadcast" "meta pkttype broadcast" "" -- -A INPUT -m pkttype --pkt-type broadcast -j DROP
run_test "17.28-time-window"       "meta hour"              "" -- -A INPUT -m time --timestart 10:00 --timestop 12:00 -j ACCEPT
run_test "17.29-dnat-port-tcp"     "dnat to 10.0.0.5:8080"  "" -- -t nat -A PREROUTING -p tcp -j DNAT --to-destination 10.0.0.5:8080
run_test "17.30-notrack-raw"       "notrack"                "" -- -t raw -A PREROUTING -p udp -j NOTRACK

# ── Section 18: a name stormwall cannot encode ────────────────────────
#
# 1.1.14 read an unknown `--ctstate` / `--icmp-type` / `--reject-with`
# name as raw bytes or fell back to a default, so each of these exited
# 0 and installed a rule nobody asked for.

printf '=== 18. Unknown names are refused, known ones encode ===\n'
expect_refused "$IPTABLES"  "18.01-ctstate-bogus"      -- -A INPUT -m conntrack --ctstate BOGUS -j ACCEPT
expect_refused "$IPTABLES"  "18.02-state-bogus"        -- -A INPUT -m state --state BOGUS -j ACCEPT
expect_refused "$IPTABLES"  "18.03-ctstate-new-bogus"  -- -A INPUT -m conntrack --ctstate NEW,BOGUS -j ACCEPT
expect_refused "$IPTABLES"  "18.04-ctstate-dnat"       -- -A INPUT -m conntrack --ctstate DNAT -j ACCEPT
expect_refused "$IPTABLES"  "18.05-ctstate-dnat-snat"  -- -A INPUT -m conntrack --ctstate DNAT,SNAT -j ACCEPT
expect_refused "$IPTABLES"  "18.06-ctstate-list-dnat"  -- -A INPUT -m conntrack --ctstate RELATED,ESTABLISHED,DNAT -j ACCEPT
expect_refused "$IPTABLES"  "18.07-icmp-type-bogus"    -- -A INPUT -p icmp --icmp-type bogus -j ACCEPT
expect_refused "$IPTABLES"  "18.08-icmp-code-range"    -- -A INPUT -p icmp --icmp-type 3/999 -j ACCEPT
expect_refused "$IPTABLES"  "18.09-icmp-type-v6-name"  -- -A INPUT -p icmp --icmp-type no-route -j ACCEPT
expect_refused "$IP6TABLES" "18.10-icmpv6-type-bogus"  -- -A INPUT -p icmpv6 --icmpv6-type bogus -j ACCEPT
expect_refused "$IPTABLES"  "18.11-reject-with-bogus"  -- -A INPUT -j REJECT --reject-with bogus-thing
expect_refused "$IPTABLES"  "18.12-reject-with-v6-kind" -- -A INPUT -j REJECT --reject-with icmp6-no-route
expect_refused "$IPTABLES"  "18.13-icmp-neg-type-code" -- -A INPUT -p icmp ! --icmp-type port-unreachable -j ACCEPT
# The names that do resolve reach the kernel as the numbers iptables
# matches on, not as the text of the name.
run_test "18.14-ctstate-list"      "ct state"                 "ct state 0x0"  -- -A FORWARD -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT
run_test "18.15-icmp-echo-request" "icmp type echo-request"   ""              -- -A INPUT -p icmp --icmp-type echo-request -j ACCEPT
run_test "18.16-icmp-type-code"    "icmp code 3"              ""              -- -A INPUT -p icmp --icmp-type port-unreachable -j ACCEPT
run_test "18.17-icmp-numeric-pair" "icmp code 4"              ""              -- -A INPUT -p icmp --icmp-type 3/4 -j ACCEPT
run_test "18.18-icmp-type-any"     "meta l4proto icmp"        "icmp type"     -- -A INPUT -p icmp --icmp-type any -j ACCEPT
run_test "18.19-icmp-neg-type"     "icmp type != echo-request" ""             -- -A INPUT -p icmp ! --icmp-type echo-request -j ACCEPT
run_test "18.20-reject-host-unreach" "reject with icmp host-unreachable" ""    -- -A INPUT -j REJECT --reject-with icmp-host-unreachable
# ip6tables names resolve through the ICMPv6 table and list as ICMPv6,
# so `nft list ruleset` can be fed back to `nft -f` unchanged.
flush
if "$IP6TABLES" -A INPUT -p icmpv6 --icmpv6-type neighbour-solicitation -j ACCEPT 2>/dev/null &&
   "$NFT" list ruleset 2>/dev/null | grep -qF "icmpv6 type nd-neighbor-solicit"; then
    PASS=$((PASS + 1))
else
    printf '  FAIL 18.21-icmpv6-name\n%s\n' "$("$NFT" list ruleset 2>/dev/null)"
    FAIL=$((FAIL + 1))
    FAIL_NAMES="$FAIL_NAMES 18.21-icmpv6-name"
fi
expect_refused "$IPTABLES"  "18.23-dscp-class-bogus" -- -A INPUT -m dscp --dscp-class bogus -j ACCEPT
expect_refused "$IP6TABLES" "18.24-icmpv6-any"       -- -A INPUT -p icmpv6 --icmpv6-type any -j ACCEPT
run_test "18.25-icmp-any-no-p"  "meta l4proto icmp" "icmp type" -- -A INPUT -m icmp --icmp-type any -j ACCEPT
run_test "18.26-dscp-class-ef"  "ip dscp"           ""          -- -A INPUT -m dscp --dscp-class EF -j ACCEPT
# The native nft front-end has the same contract: an unknown `ct state`
# name used to OR in 0 (a rule matching nothing) or install the ASCII of
# the name, and an unknown `reject with` kind became a bare reject.
for t in "ct state bogus accept" "ct state { established, dnat } accept" "reject with bogus-thing"; do
    flush
    "$NFT" add table ip t 2>/dev/null
    "$NFT" add chain ip t c 2>/dev/null
    if "$NFT" add rule ip t c $t 2>/dev/null; then
        printf '  FAIL 18.22-nft-unknown-name [accepted: %s]\n' "$t"
        FAIL=$((FAIL + 1))
        FAIL_NAMES="$FAIL_NAMES 18.22-nft-unknown-name"
    else
        PASS=$((PASS + 1))
    fi
done

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
