# stormwall iptables compatibility surface

Last audited: 1.1.7 (2026-05-09).

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
| `-4` / `--ipv4` | accepted/ignored | family inferred from argv0 |
| `-6` / `--ipv6` | accepted/ignored | same |
| `-p` / `--protocol` | covered | with negation |
| `-s` / `--source` | covered | modern + legacy negation |
| `-d` / `--destination` | covered | same |
| `-m` / `--match` | covered | dispatches to the module table below |
| `-j` / `--jump` | covered | dispatches to the target table below |
| `-g` / `--goto` | covered | nft `goto` |
| `-i` / `--in-interface` | covered | with negation |
| `-o` / `--out-interface` | covered | with negation |
| `-f` / `--fragment` | covered | nft `ip frag-off & 0x1fff != 0` |
| `-c` / `--set-counters` | **covered (1.1.5)** | parsed and ignored (no nft equivalent on the forward path) |
| `-w` / `-W` | accepted/ignored | xtables-lock compat |
| `-n` / `-v` / `-x` / `--line-numbers` | parsed | display flags consumed but unused (renderer not wired up) |

## Targets (`-j`)

| Target | Status | Notes |
|---|---|---|
| `ACCEPT` / `DROP` / `RETURN` | covered | |
| `REJECT` | covered | bare + every documented `--reject-with` kind. nft `reject` / `reject with icmp type X` / `reject with tcp reset`. **Requires** `nft_reject` + `nft_reject_ipv4` (or `_ipv6`) kernel modules. |
| `LOG` | covered | `--log-prefix`, `--log-level`, plus 1.1.5: `--log-tcp-sequence`, `--log-tcp-options`, `--log-ip-options`, `--log-uid` (uid lowers to a comment marker — no first-class nft form). Requires `nft_log`. |
| `MASQUERADE` | covered | `--to-ports`, `--random`, plus 1.1.5: `--random-fully` |
| `SNAT` | covered | `--to-source`, `--random`, `--persistent`, plus 1.1.5: `--random-fully` |
| `DNAT` | covered | same |
| `REDIRECT` | covered | `--to-ports`, `--random`. Requires `nft_redir`. |
| `MARK` | covered | all mask-kind variants. Mask emit dropped on Set/Xset (nft folds into value). |
| `CONNMARK` | covered | SetMark/SaveMark/RestoreMark/And/Or/Xor. **1.1.7:** `--nfmask`/`--ctmask` parsed and lowered to `meta mark set ct mark and <mask>` / `ct mark set meta mark and <mask>` (single-mask emit; ctmask wins on save, nfmask on restore — strictly-correct multi-statement form left for follow-up). Tailscale's healthcheck rule depends on these. |
| `TPROXY` | partial | `--on-port`/`--on-ip` covered; `--tproxy-mark` parsed-and-dropped |
| `NFLOG` | partial | `--nflog-prefix`, `--nflog-group`. `--nflog-threshold`, `--nflog-range`, `--nflog-size` missing. |
| `TRACE` | covered | nft `meta nftrace set 1` |
| `NOTRACK` | **covered (1.1.5)** | nft `notrack`. Requires raw-table prerouting/output for it to take effect. |
| `NFQUEUE` | **covered (1.1.5)** | `--queue-num`, `--queue-balance`, `--queue-bypass`, `--queue-cpu-fanout`. Requires `nft_queue`. |
| `TCPMSS` | **covered (1.1.5)** | `--set-mss N` and `--clamp-mss-to-pmtu`. nft `tcp option maxseg size set N` / `... set rt mtu`. |
| `CT` | **covered (1.1.5)** | `--notrack`, `--helper X`, `--zone N`. nft `notrack` / `ct helper set` / `ct zone set`. |
| `<user-chain>` | covered | nft `jump <name>`; user chain auto-created if missing (1.1.2/1.1.3) |
| `--goto <name>` | covered | nft `goto <name>` |
| `AUDIT`, `CHECKSUM`, `CLASSIFY`, `CLUSTERIP`, `CONNSECMARK`, `DNPT`, `DSCP`, `ECN`, `HL`, `HMARK`, `IDLETIMER`, `LED`, `NETMAP`, `RATEEST`, `SECMARK`, `SET`, `SYNPROXY`, `MIRROR`, `SAME`, `ULOG` | missing | parsed as `Target::Jump(<NAME>)` and ensure_jump_target_chain creates a user chain — wrong but installable. |

## Match modules (`-m`)

| Module | Status | Notes |
|---|---|---|
| `addrtype` | covered | `--src-type`, `--dst-type` |
| `comment` | covered | `--comment` (also stored on cmd.spec.comment) |
| `conntrack` / `state` | covered | `--ctstate` / `--state` (NEW/ESTABLISHED/RELATED/INVALID/UNTRACKED/DNAT/SNAT) |
| `multiport` | covered | `--sports`, `--dports`, comma-bearing `--sport`/`--dport` |
| `mark` | covered | `--mark V[/M]`, with negation. **Distinct from `connmark`** since 1.1.5. |
| `connmark` | **covered (1.1.5)** | `--mark V[/M]`, ct mark match (vs meta mark). Disambiguated from `-m mark` via `in_module` parser state. |
| `physdev` | partial | `--physdev-in`, `--physdev-out`, `--physdev-is-bridged`. nft `meta ibrname` is bridge-name (not member-port like iptables); semantics close-but-not-identical. `--physdev-is-in/-out` missing. |
| `owner` | partial | `--uid-owner`, `--gid-owner` (numeric only — names not resolved). `--socket-exists`, `--suppl-groups` missing. |
| `set` | partial | `--match-set NAME dirs` — only first dir consumed |
| `mac` | covered | `--mac-source` |
| `string` | stubbed | renders `comment "string-match-stub"` — no clean nft equivalent |
| `limit` | covered | `--limit RATE[/UNIT]`, `--limit-burst`. Requires `nft_limit`. |
| `tcp` / `udp` | covered (implicit) | enables `--sport`/`--dport`/`--syn`/`--tcp-flags` |
| `iprange` | **covered (1.1.5)** | `--src-range`, `--dst-range`. Both fold into one MatchModule::Iprange. |
| `length` | **covered (1.1.5)** | `--length N` or `N:M`. nft `meta length`. |
| `pkttype` | partial | parser + lower covered (1.1.5), but `meta pkttype` is restricted to netdev/bridge family chains in the kernel — installation in ip/ip6 family chains is rejected by nft_meta. Use bridge family for filtering broadcast/multicast. |
| `tcpmss` | **covered (1.1.5)** | `--mss N` or `N:M`. nft `tcp option maxseg size`. |
| Any other `-m <name>` | parsed-as-marker | accepts the `-m` token, pushes `MatchModule::Stateful(name)`, but module-specific options will fail with `unknown flag`. |

## Match modules NOT recognized at all

The following modules have no parser hooks. Their `-m <name>` is accepted
silently but every option flag (`--connlimit-above`, `--recent`, `--time`,
etc.) errors with `unknown flag`. This is a non-exhaustive list from the
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
| `icmp6-port-unreachable` | `icmpv6 type port-unreachable` |
| anything else | passed through verbatim (likely emits invalid nft) |

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

3. **`pkttype` in ip/ip6 family** — `meta pkttype` is restricted to
   netdev/bridge family chains in the kernel. Installation in `ip` or
   `ip6` family chains is rejected by `nft_meta`. iptables works around
   this via the legacy `xt_pkttype` compat module; we do not go through
   the xt_compat path.

4. **Many match modules unimplemented** — see the full list above. The
   high-frequency ones (`recent`, `time`, `connlimit`, `hashlimit`,
   `statistic`, `tos`, `dscp`, `length`-now-covered, `connbytes`)
   would each take a parser arm and a lower arm. None block the
   forward-path use cases stormwall was built for (Docker, basic
   firewalls, common automation tooling).

5. **Many targets parsed as user-chain jumps** — `AUDIT`, `CHECKSUM`,
   `CLASSIFY`, etc. become `Target::Jump(<NAME>)` and
   `ensure_jump_target_chain` creates an empty user chain with that
   name. The rule installs but the kernel doesn't take the intended
   action. Listed in the targets table above.

## Test coverage

- `cargo test --bin stormwall` — 220/220 tests pass as of 1.1.5
- `packages/core/stormwall/tests/iptables-soak.sh` — 81-scenario
  behavioural harness against the live kernel; run as root. Covers
  every Tier-1 surface fix in 1.1.2 through 1.1.5 plus a Section 13
  "Docker first-start corpus" mirroring dockerd's libnetwork
  bridge-init sequence.

References consulted (descriptive docs only — no GPL source):

- iptables(8) and iptables-extensions(8) on manpages.debian.org
- wiki.nftables.org "Quick reference: nftables in 10 minutes"
- wiki.nftables.org "Moving from iptables to nftables"
