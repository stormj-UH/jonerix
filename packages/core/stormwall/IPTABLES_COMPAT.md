# stormwall iptables compatibility surface

Last audited: 1.1.15 (2026-09-25) — every `--ctstate`, `--icmp-type`,
`--reject-with` and `--dscp-class` name re-run against a live kernel,
side by side
with real iptables-nft 1.8.13 and read back with real nft 1.1.6. See
Known limitation 10 for the class of "unknown name installed as
something else" bugs that audit found and 1.1.15 closed.

Previously audited: 1.1.14 (2026-09-25) — every `-m <name>` re-run in
its optionless form as well as with options, against a live kernel,
plus the 298-command corpus. See Known limitation 8 for the class of
"installed the verdict alone" bugs that audit found and 1.1.14 closed.

Previously audited: 1.1.8 (2026-05-09). `MARK`, `CONNMARK`, `-m mark`,
`-m connmark`, `TRACE`, `CT --zone`, `CLASSIFY`, `-p`, `-f`, `--syn`,
`! --syn`, `--tcp-flags`, `!` placement, `-m addrtype`, `-m owner`,
`-m mac`, `REJECT`, `LOG`, `NFLOG`, the NAT targets, `NFQUEUE`,
`TTL`/`HL` and every refused form in Known limitation 7 re-audited for
1.1.13 (2026-09-25) against a live kernel, by running 298 common
iptables/ip6tables commands through 1.1.11 and 1.1.13 and comparing
exit codes and nft 1.0.9's listing.

This document maps every iptables operation, parameter, target, and match
module that stormwall recognises against the upstream `iptables(8)` and
`iptables-extensions(8)` man-page surface (consulted as descriptive
documentation on manpages.debian.org — no GPL source code referenced
during implementation).

## Coverage levels

- **`covered`** — installs the same rule shape upstream iptables would.
- **`partial`** — installs but with a degraded rendering on `iptables -L` /
  `iptables-save` (rule is functional in the kernel; the round-trip text
  is just not yet decodable by stormwall's listing path).
- **`stubbed`** — accepted on the command line but produces a no-op or a
  comment-only rule.
- **`missing`** — argv parser rejects with `unknown flag`.
- **`refused`** (1.1.13) — parsed and lowered, but stormwall's nft text
  parser cannot encode the lowered rule, so the command fails with
  "internal nft synthesis failed". 1.1.12 and earlier installed a
  different rule instead (see Known limitation 7).
- **`refused, no clause`** (1.1.14) — the match lowers to nothing at
  all, or to a `comment` alone, so installing the rule would apply its
  target to every packet. 1.1.13 and earlier did exactly that, with
  exit 0; 1.1.14 fails the command with
  "-m NAME: stormwall has no nft equivalent for this match" and
  installs nothing (see Known limitation 8). **The optionless form of
  every `-m <name>` is in this class**, including modules whose
  option-bearing forms are covered.

## Operations

| Flag | Long form | Status | Notes |
|---|---|---|---|
| `-A` | `--append` | covered | |
| `-I` | `--insert` | covered | optional position |
| `-D` (by index) | `--delete N` | covered | walks chain to resolve handle |
| `-D` (by spec) | `--delete <spec>` | stubbed | resolve_handle_by_match unimplemented (no nft → iptables inverse renderer yet); reports "rule does not exist" |
| `-R` | `--replace` | covered | |
| `-L` | `--list` | partial | chain headers + COMMIT emit; rule bodies don't (no inverse renderer) |
| `-S` | `--list-rules` | partial | same |
| `-F` | `--flush` | covered | |
| `-Z` | `--zero` | covered | |
| `-N` | `--new-chain` | covered | |
| `-X` (with chain) | `--delete-chain X` | covered | |
| `-X` (no arg) | `--delete-chain` | **covered (1.1.5)** | enumerates user chains via NFT_MSG_GETCHAIN, emits delete chain for each non-builtin |
| `-P` | `--policy` | covered | ACCEPT / DROP only (QUEUE/RETURN policies rejected like real iptables) |
| `-E` | `--rename-chain` | **covered (1.1.5)** | nft `rename chain` |
| `-C` | `--check` | stubbed | same root cause as `-D <spec>` |
| `-h` | `--help` | covered | |
| `-V` | `--version` | covered | |

## Parameters

| Flag | Status | Notes |
|---|---|---|
| `!` (invert) | **covered (1.1.13)** | modern (`! -s X`) and legacy (`-s ! X`) forms. A `!` that the option after it cannot take is an error, as in iptables (`option "--limit" cannot be inverted`); so are `! !` and a trailing `!`. Refused this way: `! --limit`, `-m owner ! --uid-owner`/`--gid-owner`, `-m physdev ! --physdev-*`, `-m string ! --string`, `! --probability`, `! --every`, `! --weekdays`, `! -f`, `! -m`, `! -j`. **Broken before 1.1.13:** such a `!` was dropped, or moved onto the next option that takes one: `! --syn` lost its `!`, and `-p tcp ! --syn -m state --state NEW -j DROP` installed `ct state != new drop`. |
| `-4` / `--ipv4` | accepted/ignored | family inferred from argv0 |
| `-6` / `--ipv6` | accepted/ignored | same |
| `-p` / `--protocol` | **covered (1.1.13)** | with negation. **Broken before 1.1.13 without a port match:** `-p tcp` lowers to `meta l4proto tcp`, and the nft parser took that `tcp` for the start of a `tcp FIELD` match that also swallowed the verdict, so `-p tcp -j DROP` (or `-p udp -j NOTRACK`) installed a rule with no protocol test and no verdict. Names other than tcp/udp/icmp/icmpv6/sctp/esp (`gre`, `ah`, `udplite`, `igmp`, ...) were stored as the name's first byte (`gre` became protocol 103, pim). |
| `--icmp-type` / `--icmpv6-type` | **covered (1.1.15)** | resolved through iptables' own name table — which is **not** nft's — and lowered as numbers: `echo-request` → `icmp type 8`, `port-unreachable` → `icmp type 3 icmp code 3`, `3/4` → `icmp type 3 icmp code 4`, `any` → the `-p icmp` test alone. An unambiguous prefix works (`echo-req`), as in iptables. Up to 1.1.14 the name went through verbatim, so anything outside the handful of names nft shares installed the ASCII of the name (`--icmp-type bogus` → `icmp type 626f67757300`, `3/4` → `icmp type 858731520`, `any` → `icmp type 1634629888`) or failed with a register-width error (`port-unreachable`, `neighbour-solicitation`). An unknown name is now refused with iptables' wording (``Unknown ICMP type `bogus'``, exit 2). `!` on a plain type gives `icmp type != N`; `!` on a name that pins a code is refused, because `not (type 3 and code 3)` is not an AND of two nft clauses. |
| `-s` / `--source` | covered | modern + legacy negation |
| `-d` / `--destination` | covered | same |
| `-m` / `--match` | covered | dispatches to the module table below |
| `-j` / `--jump` | covered | dispatches to the target table below |
| `-g` / `--goto` | covered | nft `goto` |
| `-i` / `--in-interface` | covered | with negation |
| `-o` / `--out-interface` | covered | with negation |
| `-f` / `--fragment` | **refused (1.1.13)** | lowers to nft `ip frag-off & 0x1fff != 0`, which the nft parser does not take; 1.1.12 installed the verdict alone (`-f -j DROP` dropped every packet). |
| `-c` / `--set-counters` | **covered (1.1.5)** | parsed and ignored (no nft equivalent on the forward path) |
| `-w` / `-W` | accepted/ignored | xtables-lock compat |
| `-n` / `-v` / `-x` / `--line-numbers` | parsed | display flags consumed but unused (renderer not wired up) |

## Targets (`-j`)

| Target | Status | Notes |
|---|---|---|
| `ACCEPT` / `DROP` / `RETURN` | covered | |
| `REJECT` | covered | bare + every documented `--reject-with` kind. nft `reject` / `reject with icmp type X` / `reject with tcp reset`. **Broken before 1.1.13 for `-p tcp -j REJECT --reject-with tcp-reset`** (no port match): see `-p`; the rule rejected nothing. **Requires** `nft_reject` + `nft_reject_ipv4` (or `_ipv6`) kernel modules. |
| `LOG` | **covered (1.1.13)** | `--log-prefix`; `--log-level` 0-7 or a name (`emerg` ... `debug`, `error`, `warning`, `panic`, or an unambiguous prefix, any case; 4/`warning` is the default and lists as plain `log`); `--log-tcp-sequence`, `--log-tcp-options`, `--log-ip-options`, `--log-uid`, `--log-macdecode` → `log flags tcp sequence,options` / `flags ip options` / `flags skuid` / `flags ether`, or `flags all`. Kernel expression and `nft list`/`nft -j list` output checked against nft 1.0.9. **Broken before 1.1.13:** the level was dropped (named levels read as 0), the flags were dropped, `--log-uid` became a rule comment; the first 1.1.13 cut refused every `--log-level`. Requires `nft_log`, `nf_log_syslog`. |
| `MASQUERADE` | covered; `--to-ports` **refused (1.1.13)** | `--random`, plus 1.1.5: `--random-fully`. `masquerade to :PORTS` does not parse; 1.1.12 installed the match with no masquerade (libvirt's default network masquerades TCP/UDP with `--to-ports 1024-65535`). |
| `SNAT` | covered; port ranges and IPv6 **refused (1.1.13)** | `--to-source ADDR[-ADDR][:PORT]` (IPv4, one port), `--random`, `--persistent`, plus 1.1.5: `--random-fully`. A port range installed only its first port on 1.1.12; `ip6tables` with an IPv6 address installed `snat to :0`. Since 1.1.14 a `:PORT` without `-p tcp\|udp\|sctp\|dccp` is refused with iptables' own "Need TCP, UDP, SCTP or DCCP with port specification". |
| `DNAT` | covered; port ranges and IPv6 **refused (1.1.13)** | same, including the 1.1.14 transport-protocol requirement for a `:PORT`. `ip6tables ... -j DNAT --to-destination [fd00::2]:8080` (dockerd's IPv6 port publishing) and `fd00::2` installed `dnat to :0` on 1.1.12. |
| `REDIRECT` | **refused (1.1.13)** | any form (`--to-ports`, `--random`, bare). `redirect [to :PORT]` does not parse; 1.1.12 installed the match with no redirect. Requires `nft_redir`. |
| `MARK` | **covered (1.1.13)** | `--set-mark V` → `meta mark set V`; `--set-mark V/M` → `meta mark set meta mark and ~(M\|V) xor V`; `--set-xmark V/M` → `... and ~M xor V`; `--and-mark` / `--or-mark` / `--xor-mark` → `meta mark set meta mark and\|or\|xor V`. Lists back as nft does, e.g. `meta mark set meta mark & 0xff04ffff \| 0x00040000`. **Broken before 1.1.13:** the mask was dropped and the nft parser emitted `meta mark set` with no value source, so the kernel copied whatever register 1 held into the mark (for tailscale's `-i tailscale0 -j MARK --set-mark 0x40000/0xff0000` that was the interface name, mark `0x6c696174`); `--or-mark` / `--xor-mark` were no-ops. |
| `CONNMARK` | covered | SetMark/SaveMark/RestoreMark/And/Or/Xor. **1.1.13:** `--set-mark V[/M]` and `--set-xmark V/M` encode like `MARK` on `ct mark` (before 1.1.13 `ct mark set V` had no value source, like `MARK`), `--or-mark` / `--xor-mark` take effect, and the masked save/restore forms list back as `ct mark set meta mark & 0x00ff0000` / `meta mark set ct mark & 0x00ff0000`. Save/restore with masks still clear the destination bits outside the mask (iptables keeps them); see Known limitations. **1.1.7:** `--nfmask`/`--ctmask` parsed and lowered to `meta mark set ct mark and <mask>` / `ct mark set meta mark and <mask>` (single-mask emit; ctmask wins on save, nfmask on restore — strictly-correct multi-statement form left for follow-up). Required a paired fix in the nft text parser (`meta KEY set ct KEY [and MASK]` / symmetric form). **1.1.8:** completed the round-trip — Expr::CtReg / MetaReg / CtSet anchor the source-side load to NFT_REG_1 (matching MetaSet's hard-coded SREG), and the listing renderer's meta/ct arms decode the SREG path back into the combined `set` statement so `nft list` matches what was installed. Tailscale's healthcheck rule depends on these and is now fully bidirectional. |
| `TPROXY` | partial | `--on-port`/`--on-ip` covered; `--tproxy-mark` parsed-and-dropped |
| `NFLOG` | **covered (1.1.13)** | `--nflog-group` (0-65535, default 0 as in iptables), `--nflog-prefix`, `--nflog-size` → `snaplen`, `--nflog-threshold` → `queue-threshold` (1, the default, is left out); `--nflog-range` is accepted with a warning and ignored, as the kernel does. Checked by receiving the packet on the nfnetlink_log group. **Broken before 1.1.13:** the group was dropped, so every NFLOG rule logged to the kernel log instead of nfnetlink_log; the first 1.1.13 cut refused `--nflog-group`. Requires `nfnetlink_log`. |
| `TRACE` | covered | nft `meta nftrace set 1` (the value is encoded since 1.1.13; before, the setter had no value source) |
| `NOTRACK` | **covered (1.1.5)** | nft `notrack`. Refused outside the `raw` table since 1.1.14, as in iptables; up to 1.1.13 `-t filter -p udp -j NOTRACK` installed `meta l4proto udp notrack` where it does nothing. |
| `NFQUEUE` | **covered (1.1.5)**; `--queue-balance` **refused (1.1.13)** | `--queue-num`, `--queue-bypass`, `--queue-cpu-fanout`. `--queue-balance 0:3`, with or without flags, lowers to `queue num 0-3`, which does not parse (1.1.12 installed `queue to 0`). Requires `nft_queue`. |
| `TCPMSS` | **refused (1.1.13)** | `--set-mss N` and `--clamp-mss-to-pmtu` lower to nft `tcp option maxseg size set N` / `... set rt mtu`, which the nft parser does not take; 1.1.12 installed the rule without the MSS change. |
| `TTL` | covered; `--ttl-inc`/`--ttl-dec` **refused (1.1.13)** | `--ttl-set N` → `ip ttl set N`. `--ttl-inc`/`--ttl-dec` lower to arithmetic the nft parser does not take; 1.1.12 installed `ip ttl set 6909952`. |
| `HL` | **refused (1.1.13)** | `--hl-set`, `--hl-inc`, `--hl-dec`; 1.1.12 installed a hop-limit match (`ip6 hoplimit 115`) instead of a set. |
| `CT` | **covered (1.1.5)** | `--notrack`, `--helper X`, `--zone N`. nft `notrack` / `ct helper set` / `ct zone set`. `ct zone set N` carries its value since 1.1.13. |
| `CLASSIFY` | covered (1.1.9) | `--set-class M:N` → `meta priority set M:N`; the value is encoded since 1.1.13. |
| `<user-chain>` | covered | nft `jump <name>`; user chain auto-created if missing (1.1.2/1.1.3) |
| `--goto <name>` | covered | nft `goto <name>` |
| `DSCP` | installs a TOS-byte set | `--set-dscp N` / `--set-dscp-class` → `meta nfproto ipv4 @nh,8,8 set <N&lt;&lt;2>`, the same encoding as `-j TOS`. The DSCP bits land, but the write covers the whole TOS byte, so any ECN marking on the packet is cleared; iptables' `DSCP` target leaves ECN alone. |
| `CHECKSUM` | missing | `--checksum-fill` exits 0 and installs `comment "checksum-fill"` and nothing else — no checksum action reaches the kernel. Wrong but installable. |
| `AUDIT`, `ECN`, `SECMARK`, `SYNPROXY` | refused | Their mandatory options are not recognised (`unknown flag: --type`, `--ecn-tcp-remove`, `--selctx`, `--sack-perm`), so any real use exits 2. |
| `NETMAP`, `SET` | refused | `-j NETMAP --to 10.0.0.0/24` is refused by the synthesiser (`snat to 10.0.0.0/24 ... IPv4 only`); `-j SET --add-set` needs an nft set that does not exist and fails with a bare `No such file or directory (os error 2)` — safe, but an unhelpful message. |
| `CLUSTERIP`, `MIRROR`, `ULOG`, and any unrecognised name | missing | parsed as `Target::Jump(<NAME>)` and ensure_jump_target_chain creates an empty user chain — exit 0 and the rule does nothing, where iptables fails with "Couldn't load target". `CONNSECMARK`, `DNPT`, `HMARK`, `IDLETIMER`, `LED`, `RATEEST` and `SAME` are in the same code path but untested with their own options. See limitation 5. |

## Match modules (`-m`)

| Module | Status | Notes |
|---|---|---|
| `addrtype` | **covered (1.1.13)** | `--src-type`, `--dst-type`, with negation (`fib saddr type != local`). Up to 1.1.12 `! --src-type`/`! --dst-type` dropped the `!` or moved it onto the next option. |
| `comment` | covered | `--comment` (also stored on cmd.spec.comment) |
| `conntrack` / `state` | **partial (1.1.15)** | `--ctstate` / `--state` NEW, ESTABLISHED, RELATED, INVALID, UNTRACKED → `ct state`, with negation. **`--ctstatus`, `--ctdir`, `--ctexpire`, `--ctproto` and the `--ctorig*`/`--ctrepl*` address and port options are not recognised at all** — the command fails with "unknown flag", exit 2 (an earlier revision of this table claimed `--ctstatus`/`--ctdir`/`--ctexpire` were covered; they never were). **`DNAT` and `SNAT` are refused:** iptables lowers them to `ct status dnat`/`snat`, a different conntrack field stormwall has no matcher for, and OR-ing one against a `ct state` list is not a single nft expression. Up to 1.1.14 `--ctstate DNAT,SNAT` installed `ct state 0x0` (matched 0 of 2 packets in a counter test) and `RELATED,ESTABLISHED,DNAT` silently dropped the `DNAT`. **An unknown name is refused** with iptables' wording (`Bad ctstate "BOGUS"` / `Bad state "BOGUS"`, exit 2); up to 1.1.14 it installed the ASCII bytes of the name as the state bitmask, so `--ctstate BOGUS -j ACCEPT` became `ct state established,untracked accept`. A lowercase spelling is normalised to uppercase for `iptables-save`. |
| `multiport` | covered | `--sports`, `--dports`, comma-bearing `--sport`/`--dport` |
| `mark` | **covered (1.1.13)** | `--mark V[/M]`, with negation: `meta mark & M == V` (`!=` when negated). **Distinct from `connmark`** since 1.1.5. **Broken before 1.1.13 for the masked form:** the nft parser read the `&` as the compare value and installed `meta mark == 0x26`, so tailscale's exit-node/subnet-router masquerade and forward accept, and CNI portmap's hairpin masquerade, never matched. The unmasked form was fine. |
| `connmark` | **covered (1.1.13)** | `--mark V[/M]`, ct mark match (vs meta mark): `ct mark & M == V`. Disambiguated from `-m mark` via `in_module` parser state. The masked form had the same `== 0x26` fault as `mark` before 1.1.13. |
| `physdev` | **refused (1.1.14)** | every form. nft's `meta ibrname`/`obrname` name the bridge, not the member port iptables matches, and stormwall's nft parser has no key for either, so the clause was dropped on the way to the kernel: `--physdev-in eth0 -j DROP` installed a bare `drop` and `--physdev-is-bridged -j DROP` a `meta length != 0 drop` (always true) up to 1.1.13. `!` is refused (1.1.13). |
| `owner` | partial | `--uid-owner`, `--gid-owner`, numeric ids only. A name or a range is refused (1.1.13); up to 1.1.12 it parsed as 0, so `--uid-owner tor -j ACCEPT` matched root's packets. `!` is refused. `--socket-exists`, `--suppl-groups` missing. |
| `set` | partial | `--match-set NAME dirs` — only first dir consumed |
| `mac` | **covered (1.1.13)** | `--mac-source` → `ether saddr X`. Before 1.1.13 an address whose first octet is all digits (`00:11:22:...`) did not tokenize (1.1.12 installed a rule listed `0 accept`). No `meta iiftype ether` dependency is added, so real nft lists the rule as a raw `@ll,48,48` match; packets without a link-layer header do not match, as with xt_mac. |
| `string` | **refused (1.1.14)** | lowered to `comment "string-match-stub"` and nothing else, so `-m string --string abc -j DROP` dropped every packet up to 1.1.13. `!` is refused (1.1.13). |
| `limit` | covered | `--limit RATE[/UNIT]`, `--limit-burst`. Requires `nft_limit`. |
| `tcp` / `udp` | covered (implicit) | enables `--sport`/`--dport`/`--syn`/`--tcp-flags`. **Broken before 1.1.13:** `--syn` and `--tcp-flags` lowered to `tcp flags & (fin\|syn\|rst\|ack) == syn`, and the nft parser read `fin\|syn\|rst\|ack` as one unknown word, so the mask was 0 and the rule never matched. 1.1.13 also expands `ALL` and encodes `! --tcp-flags` and `! --syn` as `!=` (the `--syn` arm used to drop its `!`, which the zero mask hid; with the mask fixed, `-m conntrack --ctstate NEW ! --syn -j DROP` would have dropped every new connection). Listed as `tcp flags & (fin \| syn \| rst \| ack) == syn` (nft 1.0+ prints `tcp flags syn / fin,syn,rst,ack` for the same rule). |
| `iprange` | **covered (1.1.5)** | `--src-range`, `--dst-range`. Both fold into one MatchModule::Iprange. |
| `length` | **covered (1.1.5)** | `--length N` or `N:M`. nft `meta length`. |
| `pkttype` | **covered (1.1.14)** | `--pkt-type unicast\|broadcast\|multicast\|other`, with negation. The kernel field is one byte of `PACKET_*`; up to 1.1.13 the name was compared as ASCII against it, so the match never fired and real nft read the rule back as a bare verdict. Bare `-m pkttype` is refused. |
| `tcpmss` | **refused (1.1.13)** | `--mss N` or `N:M` lower to nft `tcp option maxseg size`, which the nft parser does not take; 1.1.12 dropped the match and kept the verdict (`-p tcp -m tcpmss --mss 1400 -j DROP` installed as `drop`). |
| `connlimit` | **refused (1.1.13)** | every form; 1.1.12 installed the verdict without the count (`-p tcp -m connlimit --connlimit-above 10 --connlimit-mask 24 -j DROP` installed `drop`). |
| `ttl` / `hl` | covered; `--ttl-lt`/`--ttl-gt`, `--hl-lt`/`--hl-gt` **refused (1.1.13)** | `--ttl-eq`/`--hl-eq`. The less/greater forms installed `meta nfproto ipv4 drop` (every IPv4 packet) on 1.1.12. |
| `sctp` / `dccp` ports | **refused (1.1.13)** | `-p sctp --dport N` (also `-m sctp`) and `-p dccp --dport N` installed the bare verdict on 1.1.12 (`accept` of everything). |
| `esp` / `ah` | **refused** | `--espspi`/`--ahspi` refused since 1.1.13 (1.1.12 installed the protocol match without the SPI). The optionless `-m esp`/`-m ah`, and `-m ah --ahlen`/`--ahres` (a comment alone), are refused since 1.1.14 — up to 1.1.13 they installed the verdict alone. |
| `frag` / `hbh` / `mh` / `dst` / `rt` (ip6tables) | **refused** | The option-bearing forms (`--fragid`, `--fragmore`, `--hbh-len`, `--mh-type`, `--dst-len`, `--rt-type`, `--rt-segsleft`) lower to nft exthdr text the parser does not take, refused since 1.1.13 for frag/hbh/mh/dst and since 1.1.14 for `rt` (which used to fold onto `rt classid`, the routing-metadata key). The optionless forms and the comment-only options (`--hbh-opts`, `--dst-opts`, `--fraglast`, `--fragres`, `--rt-0-res`) are refused since 1.1.14; up to 1.1.13 every one of them installed the bare verdict, i.e. `ip6tables -m hbh -j DROP` dropped all IPv6. |
| `socket`, `u32`, `quota`, `bpf`, `nfacct`, `cluster`, `devgroup`, `cpu`, `osf`, `realm`, `ipvs`, `ecn`, `connlabel`, `rateest`, `srh`, `eui64` | **refused (1.1.14)** | no parser arm, so the `-m` token was accepted as a bare marker and the rule installed its verdict alone (`-m socket -j ACCEPT` accepted every packet up to 1.1.13). Module-specific options still fail with `unknown flag`. |
| `policy` | **refused (1.1.14)** | `--pol ipsec`/`none` lowered to `meta secpath exists`/`missing`, which the nft parser has no key for, so `-m policy --dir in --pol ipsec -j ACCEPT` accepted every packet up to 1.1.13. iptables-save still round-trips the flags. |
| `time` | partial | `--timestart` **with** `--timestop` → `meta hour`, and `--weekdays` → `meta day`. `--timestart` alone (or a bare `-m time`) lowered to nothing and installed the verdict alone up to 1.1.13; **refused (1.1.14)**. |
| `connbytes` | **refused (1.1.14)** | `ct packets`/`bytes`/`avgpkt` need the ordered comparators (`>=`, `<`) the nft parser does not take yet; `packets`/`avgpkt` were not even ct keys, so the load folded onto `ct state`. |
| `hashlimit` | partial | `--hashlimit-upto`/`--hashlimit-above` (required, as in iptables) → `limit rate [over] N/unit burst B packets`; the htable/mode/rate-match flags ride along as a comment marker. A bare `-m hashlimit` installed `limit rate 1/second` up to 1.1.13; **refused (1.1.14)**. |
| `ipv6header` | **covered (1.1.14)** | `--header NAME[,NAME]` → `ip6 nexthdr N` / `{ N, N }`. The header names now resolve to their IANA numbers; up to 1.1.13 the nft keyword was compared as ASCII against the one-byte field, so the match never fired. `--soft` alone is refused. |
| Any other `-m <name>` | **refused (1.1.14)** | the `-m` token is still accepted and `iptables-save` re-emits it, but a rule whose every match module lowered to nothing is refused rather than installed as its verdict alone. |

## Match modules NOT recognized at all

This list predates 1.1.9-1.1.11, which added parser arms for most of
the TCP-flow, state-machine, time-of-day and IPv6 extension-header
modules below; the ones whose nft form the parser cannot encode are
refused, see Known limitation 7. Since 1.1.14 a `-m <name>` from this
list is **refused**, not accepted silently: it lowers to no match
clause, and a rule with no clause would apply its target to every
packet (Known limitation 8). Every option flag
(`--connlimit-above`, `--recent`, `--time`, etc.) still errors with
`unknown flag`. This is a non-exhaustive list from the
iptables-extensions surface:

- **TCP-flow** — `connbytes`, `connlimit`, `dccp`, `dscp`, `ecn`, `recent`,
  `sctp`, `tos`, `ttl`, `hl`, `hashlimit`, `statistic`
- **State-machine** — `connlabel`, `helper`, `policy`, `rateest`,
  `realm`, `cluster`, `cpu`, `devgroup`
- **Time-of-day** — `time`
- **Layer-2 / IPVS** — `cgroup`, `ipvs`, `socket`, `rpfilter`
- **IPv6 extension headers** — `ah`, `dst`, `eui64`, `frag`, `hbh`,
  `ipv6header`, `mh`, `rt`, `srh`
- **Misc** — `bpf`, `nfacct`, `osf`, `quota`, `u32`

## REJECT `--reject-with` types accepted

Both prefixed and bare forms are accepted for each entry below:

| iptables name | nft rendering |
|---|---|
| `icmp-net-unreachable` / `net-unreachable` | `icmp type net-unreachable` |
| `icmp-host-unreachable` / `host-unreachable` | `icmp type host-unreachable` |
| `icmp-port-unreachable` / `port-unreachable` | `icmp type port-unreachable` |
| `icmp-proto-unreachable` / `proto-unreachable` | `icmp type prot-unreachable` |
| `icmp-net-prohibited` / `net-prohibited` | `icmp type net-prohibited` |
| `icmp-host-prohibited` / `host-prohibited` | `icmp type host-prohibited` |
| `icmp-admin-prohibited` / `admin-prohibited` | `icmp type admin-prohibited` |
| `tcp-reset` / `tcp-rst` | `tcp reset` |
| `icmp6-no-route` / `no-route` | `icmpv6 type no-route` |
| `icmp6-adm-prohibited` / `adm-prohibited` | `icmpv6 type admin-prohibited` |
| `icmp6-addr-unreachable` / `addr-unreach` | `icmpv6 type addr-unreachable` |
| `icmp6-port-unreachable` / `port-unreach` | `icmpv6 type port-unreachable` |
| anything else | **refused (1.1.15)** — `unknown reject type "NAME"`, exit 2, as iptables says it. Up to 1.1.14 an unknown kind reached the nft text verbatim, where the reject statement parser dropped it and installed a bare `reject`: ICMP port-unreachable, not what was asked for. The kinds are checked per family, so an `icmp6-*` kind under `iptables` (or an `icmp-*` kind under `ip6tables`) is refused too. |

The bare `-j REJECT` form (no `--reject-with`) lowers to nft `reject` and
emits `NFTA_REJECT_TYPE = NFT_REJECT_ICMP_UNREACH`,
`NFTA_REJECT_ICMP_CODE = ICMP_UNREACH_PORT (3)` — matches upstream nft's
default and iptables semantics for unqualified REJECT. Without those
attribute fields the kernel rejects the NEWRULE syscall with ENOENT.

## Kernel module dependencies

Several stormwall paths depend on kernel modules that jonerix doesn't
auto-load on boot. The docker pre_install hook (since 1.1.4) prompts
the operator to modprobe these. For non-docker users:

| Required module | Used by |
|---|---|
| `nf_tables` (built-in or autoload) | every nft-family rule |
| `nft_reject`, `nft_reject_ipv4`, `nft_reject_ipv6` | `-j REJECT` (any form) |
| `nft_log`, `nf_log_syslog` | `-j LOG` |
| `nft_limit` | `-m limit` |
| `nft_redir` | `-j REDIRECT` |
| `nft_queue` | `-j NFQUEUE` |
| `nft_ct` | `-j CT`, `-m connmark` (lookup), `-m conntrack` |

## Known limitations

These are deliberate scope cuts as of 1.1.5 — listed so consumers can
plan around them or contribute fixes:

1. **Listing renderer (`-L` / `iptables-save`)** — chain headers and a
   trailing `COMMIT` emit; rule bodies do not. The forward path
   (iptables → nft → kernel) is fully covered, but the reverse path
   (kernel → iptables-save text) needs an inverse renderer that walks
   the rule's expression list and reconstructs the iptables flag form.
   Several listing-side cosmetic issues (`meta length 0x00000064`
   instead of `100`, `ip saddr 167772161-167772260` instead of
   `10.0.0.1-10.0.0.100`) fall under this same umbrella — they are
   nft list output formatting, not the rule-installation path.

2. **`-D <rule-spec>` and `-C`** — depend on the same inverse renderer:
   to delete-by-match or check-existence we need to render every
   in-kernel rule's text and compare against the request's lowering.
   Today both report "rule not found" / exit 1, which is correct
   behaviour when no comparison is possible but breaks scripts that
   rely on `-D` cleanup paths.

3. **`pkttype`** — `meta pkttype` installs and matches in `ip` and
   `ip6` family chains (re-checked on Linux 6.8 for 1.1.14; the
   earlier claim that `nft_meta` rejects it outside netdev/bridge was
   wrong for the read side). What was actually broken was the value:
   the name was compared as ASCII against the one-byte field. Fixed in
   1.1.14.

4. **Many match modules unimplemented** — see the full list above. The
   high-frequency ones (`recent`, `time`, `connlimit`, `hashlimit`,
   `statistic`, `tos`, `dscp`, `length`-now-covered, `connbytes`)
   would each take a parser arm and a lower arm. None block the
   forward-path use cases stormwall was built for (Docker, basic
   firewalls, common automation tooling).

5. **Many targets parsed as user-chain jumps** — `AUDIT`, `CHECKSUM`,
   etc. become `Target::Jump(<NAME>)` and
   `ensure_jump_target_chain` creates an empty user chain with that
   name. The rule installs but the kernel doesn't take the intended
   action. Listed in the targets table above.

6. **CONNMARK `--save-mark` / `--restore-mark` with masks** — lowered
   to `ct mark set meta mark & M` / `meta mark set ct mark & M`, which
   also zeroes the destination bits outside `M`; iptables computes
   `ctmark = (ctmark & ~ctmask) ^ (nfmark & nfmask)` (and the mirror
   for restore), which needs a bitwise between two registers. The
   result is the same for tailscale's `0xff0000` slicing as long as
   nothing else uses the other mark bits.

7. **Unencodable rules are refused (1.1.13)** — if the nft parser does
   not understand every token of the text an iptables command lowers
   to, the command fails with "internal nft synthesis failed" instead
   of installing a rule with the unknown parts dropped (how the masked
   mark match became `meta mark == 0x26`). Scripts that ran these
   commands without error on 1.1.12 now fail on them. Found by running
   298 common iptables/ip6tables commands (firewall scripts, dockerd,
   CNI, kube-proxy, fail2ban, ufw, libvirt, wg-quick) through 1.1.11
   and 1.1.13 against a live kernel; this is the complete list of
   commands in that set that 1.1.12 accepted and 1.1.13 refuses:
   `REDIRECT` (any form), `MASQUERADE --to-ports`, `SNAT`/`DNAT` with a
   port range, `ip6tables` `SNAT`/`DNAT` to an IPv6 address,
   `TCPMSS` (both forms), `NFQUEUE --queue-balance` (with or without
   flags), `TTL --ttl-inc`/`--ttl-dec`, `HL` (all forms),
   `-m connlimit`, `-m ttl --ttl-lt`/`--ttl-gt`, `-m hl --hl-lt`/
   `--hl-gt`, `-m tcpmss`, `-p sctp`/`-p dccp` with a port,
   `-m esp --espspi`, `-m ah --ahspi`, `ip6tables -m frag`/`-m hbh`/
   `-m mh`/`-m dst`, and `-f`. Each installed a different rule on
   1.1.12 (the table rows say what); several matched every packet.
   Unknown `ip`/`ip6`/`tcp`/`udp`/`icmp`/`th`/`ether` fields also count
   (they used to be skipped without a trace). The argv parser likewise
   refuses a `!` the next option cannot take (see the `!` row) and
   `-m owner` names and ranges. `iptables-restore` applies the whole
   file as one batch, so one refused rule fails the restore and none
   of that file's rules are installed; a wg-quick `PostUp` that runs
   `-j TCPMSS` fails and wg-quick takes the interface down again. tailscale's ipt-default
   set, dockerd 27's bridge and port-publish rules and the CNI bridge,
   firewall and portmap rules all install.

8. **A match that lowers to no clause is refused (1.1.14)** —
   limitation 7 only catches a *token* the nft parser cannot read.
   When `src/iptables/lower.rs` emitted no clause for a match module,
   or only a `comment`, the lowered text parsed cleanly and the rule
   installed with its verdict and nothing else, exit 0 — a `drop` or
   `accept` of every packet. Every `-m <name>` now has to leave a real
   match clause behind or the command fails, `-m comment` excepted.
   The forms this newly refuses, all of which installed a bare verdict
   up to 1.1.13:
   - the optionless form of **every** `-m <name>`, recognised or not:
     `-m esp`, `-m ah`, `-m frag`, `-m hbh`, `-m dst`, `-m mh`,
     `-m rt`, `-m ipv6header`, `-m physdev`, `-m socket`, `-m time`,
     `-m pkttype`, `-m policy`, `-m mark`, `-m u32`, `-m quota`, …
   - options that lowered to a comment alone: `-m hbh --hbh-opts`,
     `-m dst --dst-opts`, `-m frag --fraglast`/`--fragres`,
     `-m rt --rt-0-res`, `-m ipv6header --soft`, `-m ah --ahlen`/
     `--ahres`, `-m string`.
   - matches whose clause the nft parser silently dropped or folded
     onto the wrong key: `-m physdev` (any form), `-m policy`,
     `-m time --timestart` without `--timestop`, `-m rt --rt-type`,
     `-m connbytes`, and a bare `-m hashlimit`.
   The same silent-default hole existed on the native `nft` front-end
   and is closed with it: an unknown `meta`, `ct`, `rt` or `socket`
   key is now an unencodable token instead of `NFT_META_LEN` /
   `NFT_CT_STATE` / `rt classid` / `socket transparent`, and `nft`
   itself (argv, `-f` and interactive) refuses a rule carrying one,
   as the iptables front-end already did. So
   `nft add rule ip t c meta ibrname "eth0" drop` fails where it used
   to install a bare `drop`; `meta mark 0x40 drop` still installs.
   `nft --pf` translation keeps its own looser contract. Keys added
   while closing it: `ct avgpkt`, `ct secmark`.

9. **Accepted more loosely than iptables until 1.1.14** —
   `-j DNAT --to-destination ADDR:PORT` (and the `SNAT` mirror) with no
   `-p tcp|udp|sctp|dccp` installed a transport port mapping that also
   rewrote ICMP; real iptables answers "Need TCP, UDP, SCTP or DCCP
   with port specification" and real nft refuses a transport mapping
   with no transport match. `-j NOTRACK` was taken in any table where
   iptables takes it only in `raw`. Both are refused from 1.1.14, with
   iptables' own wording.

10. **A name stormwall cannot encode is refused (1.1.15)** —
    limitations 7 and 8 catch a *token* the nft parser cannot read and
    a match that leaves no clause. A third hole stayed open: a name
    that reached the nft text as a bare word, where the value parser
    fell through to "use the string's bytes" or to a default.
    - `--ctstate`/`--state`: an unknown name installed its ASCII as the
      state bitmask, so `--ctstate BOGUS -j ACCEPT` became
      `ct state established,untracked accept` — an ACCEPT of
      established and untracked traffic nobody asked for. Real iptables
      says `Bad ctstate "BOGUS"` and exits 2; so does stormwall now.
    - `--ctstate DNAT`/`SNAT` are refused, not silently dropped.
      iptables encodes them as `ct status dnat`/`snat`, a different
      conntrack field; stormwall has no `ct status` matcher, and an OR
      of one against a `ct state` list is not one nft expression.
      Up to 1.1.14 `--ctstate DNAT,SNAT` installed `ct state 0x0` and
      `RELATED,ESTABLISHED,DNAT` installed
      `ct state established,related`.
      *Follow-up:* adding a `ct status` matcher would let the
      single-name forms install as iptables does.
    - `--icmp-type`/`--icmpv6-type`: an unknown name installed its
      ASCII (`icmp type 626f67757300`), and so did `3/4` and `any`.
      Names now resolve through iptables' table to numbers; an unknown
      one is ``Unknown ICMP type `bogus'``, exit 2.
    - `--reject-with`: an unknown kind installed a bare `reject`, i.e.
      ICMP port-unreachable. Kinds are checked per family now.
    - `-m dscp --dscp-class` / `-j DSCP --set-dscp-class`: an
      unrecognised class became 0, so `--dscp-class bogus` installed
      `ip dscp 0`, a match on unmarked traffic. Real iptables says
      ``Invalid DSCP value `bogus'``; so does stormwall now.
    - The native `nft` front-end had the mirror holes and is closed
      with it: an unknown `ct state` name and an unknown
      `reject with` kind are unencodable tokens, so `nft` refuses the
      rule. `nft --pf` translation keeps its own looser contract.

11. **`mangle OUTPUT` is a `filter` chain, not a `route` chain** —
    stormwall creates it as `type filter hook output priority mangle`;
    real iptables-nft creates `type route hook output priority mangle`
    (confirmed side by side on Linux 6.8). The kernel re-routes a
    packet after a `route`-type output chain, so a mark set by
    `-t mangle -A OUTPUT -j MARK` is **not** picked up by an
    `ip rule fwmark` policy-routing rule under stormwall. Every other
    chain matches iptables, including `nat OUTPUT` (`type nat`) and
    `raw`/`mangle PREROUTING` (`type filter`). Not changed in 1.1.15:
    the kernel refuses to change the type of a base chain that already
    exists, so flipping it would break `-t mangle -A OUTPUT` on any
    host whose chain was created by an earlier stormwall until the
    ruleset is flushed. tailscale's only mangle OUTPUT rule is
    `CONNMARK --save-mark`, which does not change the packet mark, so
    nothing on a tailscale host depends on the re-route today.

## Test coverage

- `cargo test --bin stormwall` — **311 pass** (25 ignored, need root)
  as of 1.1.15; 302 as of 1.1.14, 296 as of 1.1.13, 220 as of 1.1.5.
  The 1.1.13 mark, protocol, tcp-flags, negation, LOG/NFLOG and NAT
  tests, and the 1.1.14/1.1.15 refusal tests, lower each iptables
  command, parse the nft text, encode it to netlink attributes and
  render the `nft list` text, so a parse that drops tokens fails the
  test (the older tests only grepped the lowered text).
- `packages/core/stormwall/tests/iptables-soak.sh` — **178-scenario**
  behavioural harness against the live kernel; run as root (it no
  longer needs `sudo` when it is already uid 0). 1.1.13 added the
  masked mark cases 7.04-7.07, the protocol/flags cases 6.09-6.11, the
  negation cases 6.12-6.17, the LOG/NFLOG cases 7.08-7.15 and Section
  16, which checks that each refused form fails and leaves no rule
  behind and that `iptables-restore` installs nothing from a file with
  one refused line; 1.1.14 added Section 17 (a match that lowers to no
  clause, 31 cases); 1.1.15 added Section 18 (an unknown `--ctstate`,
  `--icmp-type`, `--reject-with` or `--dscp-class` name, 28 cases).
  **1.1.15 passes 170/178** in the jonerix builder container
  (1.1.14: 142/150 on its own 150-case script; 1.1.11: 61/119 on the
  1.1.13 script).
  The 8 failures are environment or the documented listing gap, not
  the rule-installation path:
  - `10.01-add-then-delete-by-spec` and `11.01-check-existing-rule` —
    `-D <spec>` and `-C` are stubbed (limitation 2).
  - `14.01-save-restore-roundtrip` — the per-rule inverse renderer
    (limitation 1).
  - `15.12-ct-helper` — needs `nf_conntrack_ftp` loaded.
  - `15.04-iprange-src`, `15.05-length-single`, `15.06-length-range`
    and `15.10-nfqueue-num` — `nft list` text differences over rules
    the kernel holds correctly (`meta length 0x00000064` for `100`, an
    iprange as two integers, `queue to N` for `queue num N`).
  Covers every Tier-1 surface fix in 1.1.2 through 1.1.5 plus a
  Section 13 "Docker first-start corpus" mirroring dockerd's
  libnetwork bridge-init sequence.

References consulted (descriptive docs only — no GPL source):

- iptables(8) and iptables-extensions(8) on manpages.debian.org
- wiki.nftables.org "Quick reference: nftables in 10 minutes"
- wiki.nftables.org "Moving from iptables to nftables"
