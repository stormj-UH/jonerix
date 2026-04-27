//! nft command parser - tokenizer + command parser. 100% safe Rust.

use crate::netlink::*;
use std::net::Ipv4Addr;

// ── Types ───────────────────────────────────────────────────────

#[derive(Debug, Clone)] pub enum CmdOp { Add, Create, Delete, Destroy, List, Flush, Rename, Insert, Replace, Reset }
#[derive(Debug, Clone)] pub enum CmdObj {
    Table, Chain, Rule, Set, Map, Element,
    Ruleset, Tables, Chains, Rules, Sets, Maps,
    // Named stateful objects.
    Counter, Quota, Limit,
    Counters, Quotas, Limits,
}

#[derive(Debug, Clone, Default)]
pub struct ChainSpec {
    pub is_base: bool,
    pub chain_type: String,
    pub hook: String,
    pub priority: i32,
    pub has_priority: bool,
    pub policy: String,
    pub has_policy: bool,
    pub device: String,
    pub has_device: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TableSpec {
    pub flags: u32,
    pub has_flags: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SetSpec {
    pub key_type_name: String,
    pub key_type: u32,
    pub key_len: u32,
    pub data_type_name: String,
    pub data_type: u32,
    pub data_len: u32,
    pub flags: u32,
    pub has_flags: bool,
    pub is_map: bool,
}

/// Attributes for a named stateful object declaration:
///     add counter t cnt { packets 0 bytes 0 ; }
///     add quota t q   { over 500 mbytes ; }
///     add limit t l   { rate 10/second burst 5 packets ; }
/// The `kind` field is one of NFT_OBJECT_COUNTER / _QUOTA / _LIMIT.
#[derive(Debug, Clone, Default)]
pub struct ObjSpec {
    pub kind: u32,
    pub packets: u64,       // counter init + limit `rate <N>`
    pub bytes: u64,         // counter init + quota `<N> bytes`
    pub quota_flags: u32,   // NFT_QUOTA_F_INV when `over` is used
    pub limit_rate: u64,
    pub limit_unit: u64,    // seconds per period (1, 60, 3600, 86400)
    pub limit_burst: u32,
    pub limit_type: u32,    // NFT_LIMIT_PKTS (0) or NFT_LIMIT_PKT_BYTES (1)
    pub limit_flags: u32,   // NFT_LIMIT_F_INV
}

#[derive(Debug, Clone)]
pub enum Expr {
    Payload { base: u32, offset: u32, len: u32, protocol: u8 },
    // `width` is the source value's byte width; used to pick the correct
    // destination register (NFT_REG_1 for 16-byte keys like iifname,
    // NFT_REG32_00 for 4-byte keys like mark). Default 4 for back-compat.
    Meta { key: u32, width: usize },
    Ct { key: u32, dir: i8, width: usize },
    Cmp { op: u32, data: Vec<u8> },
    Bitwise { mask: Vec<u8>, xor: Vec<u8> },
    Verdict { code: i32, chain: String },
    /// `ip saddr @blocked` / `tcp dport != @allowed` — look the value in
    /// the current register up in a named set. `inverted` is true for
    /// `!=` (NFT_LOOKUP_F_INV). `width` is the key width, used to pick
    /// the same register the preceding Meta/Payload stored into.
    Lookup { set_name: String, inverted: bool, width: usize },
    /// Anonymous set literal: `ip saddr { 1.1.1.1, 2.2.2.2 }` or
    /// `tcp dport { 22, 80 }`. At rule-build time ops.rs allocates a
    /// `__setN` name, emits NEWSET+NEWSETELEM into the same batch, and
    /// replaces this variant with an Expr::Lookup that points at the
    /// freshly-created anon set. Element tuples use the same
    /// (key, optional-range-end) shape as Command::elements, so CIDR
    /// ranges and `a-b` literals lower naturally.
    AnonSet { elements: Vec<(Vec<u8>, Option<Vec<u8>>)>, width: usize, inverted: bool },
    /// `... vmap @set` — named verdict map lookup. `set_id` is `Some(n)`
    /// for anon vmaps so the lookup's NFTA_LOOKUP_SET_ID matches the
    /// NEWSET's NFTA_SET_ID in the same batch (required by the kernel
    /// for transactional-name resolution); `None` for named maps.
    Vmap { set_name: String, width: usize, set_id: Option<u32> },
    /// `... vmap { key : VERDICT, ... }` — anonymous verdict map literal.
    /// Each verdict is (code, chain_name). Chain name is empty for
    /// accept/drop/return/continue.
    AnonVmap { pairs: Vec<(Vec<u8>, i32, String)>, width: usize },
    /// Reference to a named stateful object from a rule:
    ///     counter name "cnt"        (kind = NFT_OBJECT_COUNTER)
    ///     quota name   "q"          (kind = NFT_OBJECT_QUOTA)
    ///     limit name   "l"          (kind = NFT_OBJECT_LIMIT)
    /// Encodes as the `objref` expression with IMM_TYPE + IMM_NAME.
    ObjRef { kind: u32, name: String },
    Counter,
    Log { prefix: String },
    Limit { rate: u64, unit: u64, burst: u32 },
    Nat { nat_type: u32, family: u8, addr: Vec<u8>, port: u16, has_addr: bool, has_port: bool },
    Masquerade,
    Notrack,
    Reject,
}

#[derive(Debug, Clone)]
pub struct Command {
    pub op: CmdOp,
    pub obj: CmdObj,
    pub family: u8,
    pub table: String,
    pub chain: String,
    pub set_name: String,
    pub new_name: String,
    pub handle: u64,
    pub has_handle: bool,
    pub rule_index: i32,         // 0-based position within chain for `index N`
    pub has_rule_index: bool,
    pub comment: String,
    pub has_comment: bool,
    pub chain_spec: ChainSpec,
    pub table_spec: TableSpec,
    pub set_spec: SetSpec,
    pub obj_spec: ObjSpec,
    pub exprs: Vec<Expr>,
    // Each element is (key, optional-range-end). A range `a-b` in the
    // source serialises as (a, Some(b)); a bare key is (a, None). The
    // encoder emits two records per range (start without flag, end+1
    // with NFT_SET_ELEM_INTERVAL_END) so interval-flagged sets get the
    // kernel encoding the manpage documents.
    pub elements: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    /// For verdict-map element blocks: per-element (code, chain) pairs
    /// parallel to `elements`. Empty when the set isn't a vmap. Chain
    /// is empty for accept/drop/return/continue verdicts.
    pub element_verdicts: Vec<(i32, String)>,
    /// For data-value maps (`type ipv4_addr : ipv4_addr` etc.):
    /// per-element data-value bytes parallel to `elements`. Each
    /// Option is None for a keyed-only element; Some(bytes) for
    /// a mapping entry. Empty vec when the set is plain (no values).
    pub element_datas: Vec<Option<Vec<u8>>>,
}

impl Default for Command {
    fn default() -> Self {
        Command {
            op: CmdOp::List, obj: CmdObj::Ruleset, family: NFPROTO_IPV4,
            table: String::new(), chain: String::new(), set_name: String::new(),
            new_name: String::new(), handle: 0, has_handle: false,
            rule_index: -1, has_rule_index: false,
            comment: String::new(), has_comment: false,
            chain_spec: ChainSpec::default(), table_spec: TableSpec::default(),
            set_spec: SetSpec::default(), obj_spec: ObjSpec::default(),
            exprs: Vec::new(), elements: Vec::new(),
            element_verdicts: Vec::new(), element_datas: Vec::new(),
        }
    }
}

// ── Tokenizer ───────────────────────────────────────────────────

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    let mut brace_depth: i32 = 0;

    while let Some(&c) = chars.peek() {
        // Newline always acts as a statement terminator. This matches
        // nft(8) syntax inside chain bodies (rules separated by `;`
        // OR newline) and between top-level commands. List parsers
        // (set element enumerations etc.) skip stray `;` so emitting
        // one at every newline depth is safe.
        if c == '\n' {
            chars.next();
            if tokens.last().map_or(true, |t| t != ";") {
                tokens.push(";".into());
            }
            continue;
        }
        if c.is_whitespace() { chars.next(); continue; }
        if c == '#' { while chars.peek().map_or(false, |&c| c != '\n') { chars.next(); } continue; }

        // Single-char tokens
        if matches!(c, '{' | '}' | '(' | ')' | ';' | ',') {
            if c == '{' { brace_depth += 1; }
            if c == '}' { brace_depth = brace_depth.saturating_sub(1); }
            tokens.push(c.to_string()); chars.next(); continue;
        }

        // Operators
        if c == '!' { chars.next();
            if chars.peek() == Some(&'=') { chars.next(); tokens.push("!=".into()); }
            else { tokens.push("!".into()); }
            continue;
        }
        if c == '=' { chars.next();
            if chars.peek() == Some(&'=') { chars.next(); tokens.push("==".into()); }
            else { tokens.push("=".into()); }
            continue;
        }
        if c == '<' { chars.next();
            if chars.peek() == Some(&'=') { chars.next(); tokens.push("<=".into()); }
            else { tokens.push("<".into()); }
            continue;
        }
        if c == '>' { chars.next();
            if chars.peek() == Some(&'=') { chars.next(); tokens.push(">=".into()); }
            else { tokens.push(">".into()); }
            continue;
        }

        // Quoted string
        if c == '"' {
            chars.next();
            let mut s = String::new();
            while let Some(&ch) = chars.peek() {
                if ch == '"' { chars.next(); break; }
                if ch == '\\' { chars.next(); if let Some(&esc) = chars.peek() { s.push(esc); chars.next(); } }
                else { s.push(ch); chars.next(); }
            }
            tokens.push(format!("\"{}\"", s));
            continue;
        }

        // Words, numbers, addresses
        let mut word = String::new();
        while let Some(&ch) = chars.peek() {
            if ch.is_whitespace() || matches!(ch, '{' | '}' | '(' | ')' | ';' | ',' | '!' | '=' | '<' | '>') { break; }
            // Dash handling. Ranges like 10.0.0.20-10.0.0.25 or 1000-2000
            // need the dash to break the token; but hyphenated names like
            // "foo-bar" are identifiers and keep it. Rule: if the current
            // word is a numeric literal or IPv4 address (digits + dots
            // only) AND the next character is a digit, the dash is a
            // range separator.
            if ch == '-' && !word.is_empty() {
                // IPv4 range: left is digits-plus-dots, right starts digit.
                let is_ipv4_like = word.chars().all(|c| c.is_ascii_digit() || c == '.');
                // IPv6 range: left contains `:` and uses only hex+colon+dot
                // (allow embedded ipv4 for ::ffff:a.b.c.d), right starts
                // with a hex digit or `:` (for an abbreviation like `::1`).
                let is_ipv6_like = word.contains(':')
                    && word.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
                if is_ipv4_like || is_ipv6_like {
                    let next_c = chars.clone().skip(1).next().unwrap_or(' ');
                    let right_starts_addr = if is_ipv4_like {
                        next_c.is_ascii_digit()
                    } else {
                        next_c.is_ascii_hexdigit() || next_c == ':'
                    };
                    if right_starts_addr {
                        tokens.push(word.clone());
                        word.clear();
                        tokens.push("-".into());
                        chars.next();
                        continue;
                    }
                }
            }
            word.push(ch); chars.next();
        }
        if !word.is_empty() { tokens.push(word); }
    }
    tokens
}

// ── Parser ──────────────────────────────────────────────────────

struct Parser {
    tokens: Vec<String>,
    pos: usize,
    /// When true, integer literals from `parse_value` are emitted in
    /// host byte order rather than network byte order. The kernel
    /// stores meta/ct register values in the CPU's native order, so a
    /// `meta mark 0x10` rule needs the comparison-side value to match
    /// — otherwise nft monitor decodes our `0x10` as `0x10000000` on
    /// a little-endian box. Set by callers right before invoking
    /// parse_cmp_and_value, cleared after.
    host_order_value: bool,
    /// When true, symbolic names like `echo-request` resolve via the
    /// ICMPv6 table (echo-request=128) rather than the ICMPv4 one
    /// (echo-request=8). Set by the icmpv6 token handler before
    /// delegating to parse_cmp_and_value, cleared after.
    icmpv6_context: bool,
}

impl Parser {
    fn new(tokens: Vec<String>) -> Self {
        Parser { tokens, pos: 0, host_order_value: false, icmpv6_context: false }
    }
    fn peek(&self) -> &str { self.tokens.get(self.pos).map(|s| s.as_str()).unwrap_or("") }
    fn peek_offset(&self, n: usize) -> &str { self.tokens.get(self.pos + n).map(|s| s.as_str()).unwrap_or("") }
    fn advance(&mut self) -> &str {
        let t = self.tokens.get(self.pos).map(|s| s.as_str()).unwrap_or("");
        self.pos += 1; t
    }
    fn at_end(&self) -> bool { self.pos >= self.tokens.len() }
    fn matches(&mut self, s: &str) -> bool {
        if self.peek() == s { self.advance(); true } else { false }
    }

    fn is_family(s: &str) -> bool {
        matches!(s, "ip" | "ip6" | "inet" | "arp" | "bridge" | "netdev")
    }

    fn parse_family_default_ip(&mut self) -> u8 {
        if Self::is_family(self.peek()) { family_from_str(self.advance()).unwrap_or(NFPROTO_IPV4) }
        else { NFPROTO_IPV4 }
    }

    fn parse_family_default_unspec(&mut self) -> u8 {
        if Self::is_family(self.peek()) { family_from_str(self.advance()).unwrap_or(NFPROTO_UNSPEC) }
        else { NFPROTO_UNSPEC }
    }

    fn parse_name(&mut self) -> String {
        let t = self.advance();
        if t.starts_with('"') && t.ends_with('"') { t[1..t.len()-1].to_string() }
        else { t.to_string() }
    }

    fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
        s.parse::<Ipv4Addr>().ok().map(|a| a.octets())
    }

    /// Parse an IPv6 address literal into 16 bytes. Accepts the usual
    /// forms: full (`2001:db8:1:2:3:4:5:6`), abbreviated (`fe80::1`,
    /// `::1`, `::`), and IPv4-mapped (`::ffff:1.2.3.4`). Returns None
    /// for malformed input — callers fall back to other element
    /// parsing paths (string key etc.) on None.
    fn parse_ipv6(s: &str) -> Option<[u8; 16]> {
        s.parse::<std::net::Ipv6Addr>().ok().map(|a| a.octets())
    }

    fn parse_priority(&mut self, family: u8) -> Result<i32, String> {
        self.parse_priority_ctx(family, "", "")
    }

    /// Priority parsing with the chain type and hook available so
    /// we can reject nat-specific names (`dstnat`, `srcnat`, `out`)
    /// when the enclosing chain is a plain `filter` chain — what
    /// upstream nft does and what chains/0023..0029 check.
    fn parse_priority_ctx(&mut self, family: u8, chain_type: &str, hook: &str) -> Result<i32, String> {
        // A priority expression is one of:
        //   <int>                  e.g. 10, -10
        //   <name>                 e.g. filter, dstnat
        //   <name> <+|-> <int>     e.g. filter + 10
        //   "<name> <+|-> <int>"   quoted (coming from a variable)
        // Consume at most those four tokens and stop at the next
        // statement keyword so we don't swallow `policy ...`, `device
        // ...`, or the chain-block terminator.
        let mut prio_str = String::new();
        let stop = |t: &str| {
            matches!(t, "" | ";" | "}" | "policy" | "device" | "hook" | "type" | "flags" | "comment")
        };
        // First token: number, name, or a quoted string that itself
        // contains the whole expression.
        if stop(self.peek()) { return Ok(0); }
        let first = self.advance().to_string();
        // Strip surrounding quotes the tokenizer preserved (for
        // `define for = "filter - 100"` style values).
        let first_unquoted = if first.starts_with('"') && first.ends_with('"') && first.len() >= 2 {
            first[1..first.len() - 1].to_string()
        } else {
            first.clone()
        };
        prio_str.push_str(&first_unquoted);
        // Optional `+ N` / `- N` tail.
        if matches!(self.peek(), "+" | "-") && !stop(self.peek_offset(1)) {
            let sign = self.advance().to_string();
            let num = self.advance().to_string();
            prio_str.push(' ');
            prio_str.push_str(&sign);
            prio_str.push(' ');
            prio_str.push_str(&num);
        }
        // Extract the bare name (before any `+`/`-` offset) for
        // validating it against the chain context.
        let prio_name = prio_str.split_whitespace().next().unwrap_or("");
        let prio_val = crate::netlink::priority_from_str_opt(&prio_str, family)
            .ok_or_else(|| format!("unknown priority '{}'", prio_str.trim()))?;

        // Priority name validity depends on (family, chain_type, hook).
        // Upstream nft rejects mismatches at parse time — we mirror
        // the same table here.
        if !chain_type.is_empty() && !hook.is_empty() {
            let is_bridge = family == crate::netlink::NFPROTO_BRIDGE;
            let is_arp = family == crate::netlink::NFPROTO_ARP;
            let is_netdev = family == crate::netlink::NFPROTO_NETDEV;
            let hook_ok = |allowed: &[&str]| allowed.iter().any(|h| *h == hook);
            // arp and netdev only accept `filter` (and bare integers),
            // no named priorities beyond that.
            if (is_arp || is_netdev)
                && matches!(prio_name, "raw" | "mangle" | "dstnat" | "security" | "srcnat" | "out")
            {
                return Err(format!(
                    "priority '{}' is not valid in {} family",
                    prio_name,
                    if is_arp { "arp" } else { "netdev" }));
            }
            // `dstnat` binds the prerouting (or ip6 output) hook.
            if prio_name == "dstnat" {
                let allowed: &[&str] = if family == crate::netlink::NFPROTO_IPV6 {
                    &["prerouting", "output"]
                } else {
                    &["prerouting"]
                };
                if !hook_ok(allowed) {
                    return Err(format!(
                        "priority 'dstnat' is only valid in the prerouting hook"));
                }
            }
            // `srcnat` binds postrouting.
            if prio_name == "srcnat" && !hook_ok(&["postrouting"]) {
                return Err(format!(
                    "priority 'srcnat' is only valid in the postrouting hook"));
            }
            // `out` is bridge-only and hook output only.
            if prio_name == "out" {
                if !is_bridge {
                    return Err(format!(
                        "priority 'out' is only valid in the bridge family"));
                }
                if !hook_ok(&["output"]) {
                    return Err(format!(
                        "priority 'out' is only valid in the output hook"));
                }
            }
        }

        Ok(prio_val)
    }

    fn parse_cmp_op(&mut self) -> u32 {
        match self.peek() {
            "==" | "=" => { self.advance(); NFT_CMP_EQ }
            "!=" => { self.advance(); NFT_CMP_NEQ }
            _ => NFT_CMP_EQ,
        }
    }

    fn parse_value(&mut self, data_len: usize) -> Vec<u8> {
        let tok = self.advance().to_string();

        // TCP flag names — `tcp flags syn,ack accept`. The flags
        // field is one byte at TCP offset 13; the rule wants an OR
        // of the named bits compared for equality. Accumulate
        // comma-separated names, fall through to other parsers if
        // the token isn't a flag name.
        if data_len == 1 {
            if let Some(b) = tcp_flag_byte(&tok) {
                let mut v = b;
                while self.peek() == "," {
                    self.advance();
                    let next = self.peek().to_string();
                    if let Some(extra) = tcp_flag_byte(&next) {
                        v |= extra;
                        self.advance();
                    } else {
                        break;
                    }
                }
                return vec![v];
            }
        }

        // CT state bitmask - return as native endian bytes
        if matches!(tok.as_str(), "new" | "established" | "related" | "untracked" | "invalid") {
            let mut state = ct_state_val(&tok);
            while self.peek() == "," {
                self.advance();
                state |= ct_state_val(self.advance());
            }
            // For bitmask comparisons, kernel needs: bitwise(reg & mask) + cmp(result != 0)
            // We return the bitmask value and let parse_cmp_and_value handle the special case
            return state.to_ne_bytes().to_vec();
        }

        // Protocol name as value
        if let Some(proto) = proto_num(&tok) {
            return vec![proto];
        }

        // ICMP type names — `icmp type echo-request` arrives here as
        // tok="echo-request" with data_len=1. Without this lookup the
        // string falls through to the integer-parse path, fails, and
        // we end up emitting the raw bytes "echo-request\0" — a
        // 12-byte cmp that overflows the 1-byte register.
        if data_len == 1 {
            if self.icmpv6_context {
                if let Some(t) = icmpv6_type_num(&tok) { return vec![t]; }
            }
            if let Some(t) = icmp_type_num(&tok) { return vec![t]; }
        }

        // Ethernet MAC (6-byte data, colon-separated hex). Tokenizer
        // keeps `aa:bb:cc:dd:ee:ff` as one token because `:` is
        // continuation. parse_mac decodes it; on miss we fall through
        // to the other parsers.
        if data_len == 6 {
            if let Some(mac) = parse_mac(&tok) {
                return mac.to_vec();
            }
        }

        // Ethernet protocol type names ("ip", "ip6", "arp", "vlan",
        // ...) → 2-byte big-endian values. Without this lookup
        // `ether type ip accept` falls through to the string fallback
        // and stormwall emits the literal bytes "ip\0\0" which the
        // kernel interprets as a wholly different ethertype.
        if data_len == 2 {
            if let Some(et) = ether_type_num(&tok) {
                return et.to_be_bytes().to_vec();
            }
        }

        // IPv6 (16-byte data) — try first because parse_ipv4 would
        // reject `fd99::2` cleanly anyway, but explicit ordering means
        // an IPv4 literal like `1.2.3.4` doesn't accidentally match an
        // IPv6 parser that's overly permissive.
        if data_len == 16 {
            if let Some(slash) = tok.find('/') {
                if let Some(addr) = Self::parse_ipv6(&tok[..slash]) {
                    return addr.to_vec();
                }
            }
            if let Some(addr) = Self::parse_ipv6(&tok) {
                return addr.to_vec();
            }
        }

        // IPv4 with optional /prefix
        if let Some(slash) = tok.find('/') {
            if let Some(addr) = Self::parse_ipv4(&tok[..slash]) {
                return addr.to_vec(); // CIDR handled by caller
            }
        }
        if let Some(addr) = Self::parse_ipv4(&tok) {
            return addr.to_vec();
        }

        // Number (port, mark, etc). Accept hex (0x..), octal (0o.. / 0..),
        // and decimal. meta mark / ct mark routinely use hex.
        let parsed: Option<u64> = if let Some(hex) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
            u64::from_str_radix(hex, 16).ok()
        } else if let Some(oct) = tok.strip_prefix("0o").or_else(|| tok.strip_prefix("0O")) {
            u64::from_str_radix(oct, 8).ok()
        } else {
            tok.parse::<u64>().ok()
        };
        if let Some(n) = parsed {
            if data_len == 1 { return vec![n as u8]; }
            if self.host_order_value {
                if data_len == 2 { return (n as u16).to_ne_bytes().to_vec(); }
                if data_len == 8 { return (n as u64).to_ne_bytes().to_vec(); }
                return (n as u32).to_ne_bytes().to_vec();
            }
            if data_len == 2 { return (n as u16).to_be_bytes().to_vec(); }
            if data_len == 8 { return (n as u64).to_be_bytes().to_vec(); }
            return (n as u32).to_be_bytes().to_vec();
        }

        // String value (interface name). Strip the surrounding quotes the
        // tokenizer preserved for unambiguous lookahead; the kernel wants
        // the raw bytes, not `"lo"`.
        let raw = if tok.starts_with('"') && tok.ends_with('"') && tok.len() >= 2 {
            &tok[1..tok.len() - 1]
        } else {
            tok.as_str()
        };
        let mut v = raw.as_bytes().to_vec();
        v.push(0);
        while v.len() < data_len { v.push(0); }
        v
    }

    fn parse_rule_exprs(&mut self, cmd: &mut Command) {
        while !self.at_end() && self.peek() != ";" {
            let tok = self.peek().to_string();

            // Comment
            if tok == "comment" {
                self.advance();
                if !self.at_end() {
                    cmd.comment = self.parse_name();
                    cmd.has_comment = true;
                }
                continue;
            }

            // ip saddr/daddr/protocol/dscp
            if tok == "ip" {
                self.advance();
                let field = self.advance().to_string();
                // `ip dscp V` — DSCP occupies the upper 6 bits of the TOS
                // byte (IP header offset 1).  nft encodes this as:
                //   payload load 1 byte @ offset 1  → reg 1
                //   bitwise: reg1 & 0xfc ^ 0        → reg 1
                //   cmp eq reg 1  (V << 2)
                // The user writes the 6-bit codepoint (0-63); the kernel
                // sees it aligned into bits 7:2 of the TOS byte.
                if field == "dscp" {
                    cmd.exprs.push(Expr::Payload { base: NFT_PAYLOAD_NETWORK_HEADER, offset: 1, len: 1, protocol: 0 });
                    cmd.exprs.push(Expr::Bitwise { mask: vec![0xfcu8], xor: vec![0x00u8] });
                    let op = self.parse_cmp_op();
                    let tok_val = self.advance().to_string();
                    let raw: u8 = if let Some(hex) = tok_val.strip_prefix("0x").or_else(|| tok_val.strip_prefix("0X")) {
                        u8::from_str_radix(hex, 16).unwrap_or(0)
                    } else {
                        tok_val.parse::<u8>().unwrap_or(0)
                    };
                    let shifted = (raw << 2) & 0xfc;
                    cmd.exprs.push(Expr::Cmp { op, data: vec![shifted] });
                    continue;
                }
                let (base, offset, len) = match field.as_str() {
                    "saddr" => (NFT_PAYLOAD_NETWORK_HEADER, 12u32, 4u32),
                    "daddr" => (NFT_PAYLOAD_NETWORK_HEADER, 16, 4),
                    "protocol" => (NFT_PAYLOAD_NETWORK_HEADER, 9, 1),
                    _ => { continue; }
                };
                cmd.exprs.push(Expr::Payload { base, offset, len, protocol: 0 });
                self.parse_cmp_and_value(cmd, len as usize);
                continue;
            }

            // ip6
            if tok == "ip6" {
                self.advance();
                let field = self.advance();
                let (offset, len) = match field {
                    "saddr" => (8u32, 16u32), "daddr" => (24, 16), _ => { continue; }
                };
                cmd.exprs.push(Expr::Payload { base: NFT_PAYLOAD_NETWORK_HEADER, offset, len, protocol: 0 });
                self.parse_cmp_and_value(cmd, len as usize);
                continue;
            }

            // tcp / udp / icmp: real nft injects an implicit
            // `meta l4proto ==` match so the kernel reads the
            // transport header only after it's confirmed the
            // packet is of the expected protocol. Without it
            // the listing decoder can't tell tcp sport apart
            // from udp sport (both are transport offset 0, len
            // 2) and defaults to `tcp` for every case.
            if tok == "tcp" || tok == "udp" || tok == "icmp" || tok == "icmpv6" {
                let proto_name = tok.clone();
                let proto_num: u8 = match proto_name.as_str() {
                    "tcp" => 6, "udp" => 17, "icmp" => 1, "icmpv6" => 58,
                    _ => unreachable!(),
                };
                self.advance();
                let field = self.advance().to_string();
                let (offset, len) = match (proto_name.as_str(), field.as_str()) {
                    ("tcp", "sport") => (0u32, 2u32),
                    ("tcp", "dport") => (2, 2),
                    ("tcp", "flags") => (13, 1),
                    ("udp", "sport") => (0, 2),
                    ("udp", "dport") => (2, 2),
                    ("udp", "length") => (4, 2),
                    ("icmp", "type") => (0, 1),
                    ("icmp", "code") => (1, 1),
                    // icmpv6 uses the same first two bytes (Type, Code)
                    // as icmpv4 because they're both transport-layer
                    // protocols starting with type+code octets. The
                    // injected dependency match is meta l4proto = 58.
                    ("icmpv6", "type") => (0, 1),
                    ("icmpv6", "code") => (1, 1),
                    _ => { continue; }
                };
                // Only inject the dependency if we haven't already
                // seen an equivalent match earlier in this rule
                // (user-written `meta l4proto tcp` / `ip protocol tcp`).
                let has_proto_match = cmd.exprs.windows(2).any(|w| {
                    matches!(&w[0], Expr::Meta { key, .. } if *key == NFT_META_L4PROTO)
                        || matches!(&w[0], Expr::Payload { base, offset: 9, len: 1, .. } if *base == NFT_PAYLOAD_NETWORK_HEADER)
                });
                if !has_proto_match {
                    cmd.exprs.push(Expr::Meta { key: NFT_META_L4PROTO, width: 1 });
                    cmd.exprs.push(Expr::Cmp { op: NFT_CMP_EQ, data: vec![proto_num] });
                }
                cmd.exprs.push(Expr::Payload { base: NFT_PAYLOAD_TRANSPORT_HEADER, offset, len, protocol: proto_num });
                if proto_name == "icmpv6" { self.icmpv6_context = true; }
                // `tcp flags` supports two extended forms in addition to the
                // simple direct-compare path handled by parse_cmp_and_value:
                //
                //   Masked:  tcp flags & (syn | rst | ack) == syn
                //     → payload load @ offset 13, len 1
                //     → bitwise reg & mask ^ 0
                //     → cmp eq reg expected_value
                //
                //   Slash:   tcp flags syn / syn,ack,rst
                //     (syn is the expected value; syn,ack,rst is the mask)
                //     → same encoding as the masked form above
                //
                // Both forms mirror how upstream nft encodes them.  The
                // renderer already understands bitwise+cmp for tcp flags
                // so the listing side needs no changes.
                if proto_name == "tcp" && field == "flags" && !self.at_end() && self.peek() != ";" && !is_statement_start(self.peek()) {
                    if self.peek() == "&" {
                        // Masked form: & ( f1 | f2 | ... ) == val_flags
                        self.advance(); // consume `&`
                        self.matches("(");
                        let mut mask: u8 = 0;
                        loop {
                            let t = self.advance().to_string();
                            // Each token may be a flag name; `|` and `,`
                            // are separators consumed separately.
                            if let Some(b) = tcp_flag_byte(&t) { mask |= b; }
                            if self.peek() == "|" { self.advance(); continue; }
                            if self.peek() == "," { self.advance(); continue; }
                            break;
                        }
                        self.matches(")");
                        self.matches("==");
                        // RHS value: one or more flag names separated by
                        // `|` or `,`.
                        let mut val: u8 = 0;
                        loop {
                            let t = self.advance().to_string();
                            if let Some(b) = tcp_flag_byte(&t) { val |= b; }
                            if self.peek() == "|" { self.advance(); continue; }
                            if self.peek() == "," { self.advance(); continue; }
                            // Stop when the next token is not a flag name
                            // (i.e. we've consumed all RHS flag tokens).
                            if tcp_flag_byte(self.peek()).is_none() { break; }
                        }
                        cmd.exprs.push(Expr::Bitwise { mask: vec![mask], xor: vec![0u8] });
                        cmd.exprs.push(Expr::Cmp { op: NFT_CMP_EQ, data: vec![val] });
                    } else {
                        // Slash form or simple form.  Peek ahead to decide.
                        //
                        // Slash form with spaces:  syn / syn,ack,rst
                        //   → tokens: "syn", "/", "syn", ",", "ack", ...
                        // Slash form without spaces:  syn/syn,ack,rst
                        //   → single token "syn/syn,ack,rst"
                        //
                        // In both cases we detect the slash in or after the
                        // first value token.
                        let first = self.peek().to_string();
                        let has_slash_joined = first.contains('/');
                        let has_slash_ahead = !has_slash_joined
                            && tcp_flag_byte(first.as_str()).is_some()
                            && self.peek_offset(1) == "/";
                        if has_slash_joined || has_slash_ahead {
                            // Slash form — VAL / MASK
                            self.advance(); // consume the first token
                            let (val_str, mask_str): (String, String) = if has_slash_joined {
                                // Split the single joined token on '/'
                                let slash = first.find('/').unwrap();
                                (first[..slash].to_string(), first[slash + 1..].to_string())
                            } else {
                                // Separate tokens: we already consumed val token
                                self.advance(); // consume "/"
                                // Mask is everything up to the next non-flag token;
                                // collect comma-separated names into a string so
                                // tcp_flag_byte can parse each piece.
                                let mut acc = self.advance().to_string();
                                while self.peek() == "," || self.peek() == "|" {
                                    self.advance(); // consume separator
                                    if tcp_flag_byte(self.peek()).is_some() {
                                        acc.push_str(",");
                                        acc.push_str(self.advance());
                                    } else {
                                        break;
                                    }
                                }
                                (first, acc)
                            };
                            // Compute expected value from val_str (comma/pipe-sep)
                            let mut val: u8 = 0;
                            for part in val_str.split(|c| c == ',' || c == '|') {
                                if let Some(b) = tcp_flag_byte(part.trim()) { val |= b; }
                            }
                            // Compute mask from mask_str (comma/pipe-sep)
                            let mut mask: u8 = 0;
                            for part in mask_str.split(|c| c == ',' || c == '|') {
                                if let Some(b) = tcp_flag_byte(part.trim()) { mask |= b; }
                            }
                            cmd.exprs.push(Expr::Bitwise { mask: vec![mask], xor: vec![0u8] });
                            cmd.exprs.push(Expr::Cmp { op: NFT_CMP_EQ, data: vec![val] });
                        } else {
                            // Simple form: `tcp flags syn,ack accept`
                            // Fall through to the standard comparator path.
                            self.parse_cmp_and_value(cmd, len as usize);
                        }
                    }
                } else {
                    self.parse_cmp_and_value(cmd, len as usize);
                }
                self.icmpv6_context = false;
                continue;
            }

            // `th sport/dport/...` — protocol-agnostic transport-header
            // load. Upstream uses `th` when a preceding match has
            // already pinned the layer-4 protocol (typically via a set
            // membership: `meta l4proto { tcp, udp } th dport 53`),
            // so we don't inject our own l4proto dependency — we
            // assume the user established it. Field offsets match the
            // common tcp/udp header (sport @0, dport @2, both u16).
            if tok == "th" {
                self.advance();
                let field = self.advance();
                let (offset, len) = match field {
                    "sport" => (0u32, 2u32),
                    "dport" => (2, 2),
                    _ => { continue; }
                };
                cmd.exprs.push(Expr::Payload { base: NFT_PAYLOAD_TRANSPORT_HEADER, offset, len, protocol: 0 });
                self.parse_cmp_and_value(cmd, len as usize);
                continue;
            }

            // ether
            if tok == "ether" {
                self.advance();
                let field = self.advance();
                let (offset, len) = match field {
                    "saddr" => (6u32, 6u32), "daddr" => (0, 6), "type" => (12, 2),
                    _ => { continue; }
                };
                cmd.exprs.push(Expr::Payload { base: NFT_PAYLOAD_LL_HEADER, offset, len, protocol: 0 });
                self.parse_cmp_and_value(cmd, len as usize);
                continue;
            }

            // meta
            if tok == "meta" {
                self.advance();
                let key_str = self.advance().to_string();
                let (key, vlen) = meta_key(&key_str);
                cmd.exprs.push(Expr::Meta { key, width: vlen });
                if !self.at_end() && !is_statement_start(self.peek()) && self.peek() != ";" {
                    // Meta values come out of the kernel register in
                    // host byte order; mark/iif/oif/skuid/skgid/etc.
                    // need the comparison RHS encoded the same way.
                    // L4PROTO/NFPROTO are 1 byte so byte order is moot.
                    self.host_order_value = matches!(key,
                        NFT_META_MARK | NFT_META_IIF | NFT_META_OIF |
                        NFT_META_SKUID | NFT_META_SKGID | NFT_META_LEN |
                        NFT_META_PRIORITY);
                    self.parse_cmp_and_value(cmd, vlen);
                    self.host_order_value = false;
                }
                continue;
            }

            // iifname/oifname without meta prefix
            if tok == "iifname" || tok == "oifname" {
                self.advance();
                let key = if tok == "iifname" { NFT_META_IIFNAME } else { NFT_META_OIFNAME };
                cmd.exprs.push(Expr::Meta { key, width: 16 });
                self.parse_cmp_and_value(cmd, 16);
                continue;
            }

            // ct
            if tok == "ct" {
                self.advance();
                let mut dir: i8 = -1;
                if self.peek() == "original" { self.advance(); dir = 0; }
                else if self.peek() == "reply" { self.advance(); dir = 1; }
                let key_str = self.advance().to_string();
                let (key, vlen) = ct_key(&key_str);
                cmd.exprs.push(Expr::Ct { key, dir, width: vlen });

                if !self.at_end() && !is_statement_start(self.peek()) && self.peek() != ";" {
                    if key == NFT_CT_STATE {
                        // ct state uses bitmask matching:
                        //   `ct state X`   → bitwise + cmp(NEQ, 0)
                        //     — reg & mask != 0, i.e. any bit present
                        //   `ct state != X` → bitwise + cmp(EQ, 0)
                        //     — reg & mask == 0, i.e. no bit present
                        let op = self.parse_cmp_op();
                        let mask_bytes = self.parse_value(vlen);
                        let zeros = vec![0u8; mask_bytes.len()];
                        cmd.exprs.push(Expr::Bitwise { mask: mask_bytes, xor: zeros.clone() });
                        let cmp_op = if op == NFT_CMP_NEQ { NFT_CMP_EQ } else { NFT_CMP_NEQ };
                        cmd.exprs.push(Expr::Cmp { op: cmp_op, data: zeros });
                    } else {
                        // Same host-byte-order rule as `meta`: ct mark,
                        // ct expiration, ct count etc. live in CPU-native
                        // order in the kernel register.
                        self.host_order_value = matches!(key,
                            NFT_CT_MARK | NFT_CT_SECMARK | NFT_CT_EXPIRATION |
                            NFT_CT_PKTS | NFT_CT_BYTES | NFT_CT_AVGPKT);
                        self.parse_cmp_and_value(cmd, vlen);
                        self.host_order_value = false;
                    }
                }
                continue;
            }

            // Verdicts
            if tok == "accept" { self.advance(); cmd.exprs.push(Expr::Verdict { code: NF_ACCEPT, chain: String::new() }); continue; }
            if tok == "drop" { self.advance(); cmd.exprs.push(Expr::Verdict { code: NF_DROP, chain: String::new() }); continue; }
            if tok == "return" { self.advance(); cmd.exprs.push(Expr::Verdict { code: NFT_RETURN, chain: String::new() }); continue; }
            if tok == "jump" { self.advance(); let c = self.parse_name(); cmd.exprs.push(Expr::Verdict { code: NFT_JUMP, chain: c }); continue; }
            if tok == "goto" { self.advance(); let c = self.parse_name(); cmd.exprs.push(Expr::Verdict { code: NFT_GOTO, chain: c }); continue; }
            if tok == "continue" { self.advance(); cmd.exprs.push(Expr::Verdict { code: NFT_CONTINUE, chain: String::new() }); continue; }

            // Counter
            if tok == "counter" {
                self.advance();
                // `counter name "x"` → reference a named counter object
                // (objref expression). Bare `counter` is the per-rule
                // anonymous counter.
                if self.peek() == "name" {
                    self.advance();
                    let n = self.parse_name();
                    cmd.exprs.push(Expr::ObjRef { kind: NFT_OBJECT_COUNTER, name: n });
                } else {
                    cmd.exprs.push(Expr::Counter);
                }
                continue;
            }
            if tok == "quota" && self.peek_offset(1) == "name" {
                self.advance(); self.advance();
                let n = self.parse_name();
                cmd.exprs.push(Expr::ObjRef { kind: NFT_OBJECT_QUOTA, name: n });
                continue;
            }
            if tok == "limit" && self.peek_offset(1) == "name" {
                self.advance(); self.advance();
                let n = self.parse_name();
                cmd.exprs.push(Expr::ObjRef { kind: NFT_OBJECT_LIMIT, name: n });
                continue;
            }

            // Log
            if tok == "log" {
                self.advance();
                let prefix = if self.peek() == "prefix" { self.advance(); self.parse_name() } else { String::new() };
                cmd.exprs.push(Expr::Log { prefix });
                continue;
            }

            // Limit
            if tok == "limit" {
                self.advance();
                self.matches("rate");
                // The tokenizer keeps `/` joined to its neighbours
                // (CIDR addresses depend on it), so `5/second`
                // arrives here as a single token. Split on the first
                // `/` and parse each half. Falling back to the legacy
                // "rate then `/` then unit" form keeps any future
                // tokenizer-split callers working.
                let first = self.advance().to_string();
                let (rate, unit_str) = if let Some(slash) = first.find('/') {
                    (
                        first[..slash].parse::<u64>().unwrap_or(0),
                        first[slash + 1..].to_string(),
                    )
                } else {
                    let r: u64 = first.parse().unwrap_or(0);
                    self.matches("/");
                    (r, self.advance().to_string())
                };
                let unit = match unit_str.as_str() {
                    "second" => 1u64, "minute" => 60, "hour" => 3600, "day" => 86400, "week" => 604800,
                    _ => 1,
                };
                let burst = if self.matches("burst") { self.advance().parse().unwrap_or(5) } else { 5 };
                self.matches("packets");
                cmd.exprs.push(Expr::Limit { rate, unit, burst });
                continue;
            }

            // NAT
            if tok == "masquerade" { self.advance(); cmd.exprs.push(Expr::Masquerade); continue; }
            if tok == "snat" || tok == "dnat" {
                let nat_type = if tok == "snat" { NFT_NAT_SNAT } else { NFT_NAT_DNAT };
                self.advance(); self.matches("to");
                // The tokenizer keeps `10.0.0.1:8080` joined into a
                // single token because `:` is a continuation char.
                // Split on the first `:` to recover both halves —
                // without this, addr_str includes the port suffix,
                // parse_ipv4 fails, and the rule arrives at the
                // kernel with no NAT target at all.
                let addr_str_full = self.advance().to_string();
                let (addr_part, port_part) = match addr_str_full.find(':') {
                    Some(i) => (&addr_str_full[..i], Some(&addr_str_full[i + 1..])),
                    None => (addr_str_full.as_str(), None),
                };
                let mut addr = Vec::new();
                let mut has_addr = false;
                let mut port = 0u16;
                let mut has_port = false;
                if let Some(a) = Self::parse_ipv4(addr_part) { addr = a.to_vec(); has_addr = true; }
                if let Some(ps) = port_part {
                    port = ps.parse().unwrap_or(0);
                    has_port = true;
                } else if self.peek() == ":" {
                    // Tokenizer does split on `:` if it's surrounded by
                    // spaces or follows a non-numeric token; keep the
                    // legacy fallback so that form still works.
                    self.advance();
                    port = self.advance().parse().unwrap_or(0);
                    has_port = true;
                }
                cmd.exprs.push(Expr::Nat { nat_type, family: cmd.family, addr, port, has_addr, has_port });
                continue;
            }

            // Notrack / Reject
            if tok == "notrack" { self.advance(); cmd.exprs.push(Expr::Notrack); continue; }
            if tok == "reject" { self.advance(); cmd.exprs.push(Expr::Reject); continue; }

            // Unknown - skip
            self.advance();
        }
    }

    fn parse_cmp_and_value(&mut self, cmd: &mut Command, data_len: usize) {
        if self.at_end() || self.peek() == ";" { return; }

        // `ip saddr vmap @m` and `ip saddr vmap { 1.1.1.1 : jump c2 }`
        // lower to a lookup with a DREG for the verdict output. For
        // now we only support the named-set form; anon vmap literals
        // are a future expansion.
        if self.peek() == "vmap" {
            self.advance();
            if self.peek().starts_with('@') {
                let tok = self.advance().to_string();
                let set_name = tok.trim_start_matches('@').to_string();
                cmd.exprs.push(Expr::Vmap { set_name, width: data_len, set_id: None });
                return;
            }
            // Anonymous vmap literal: collect `k : verdict` pairs.
            if self.peek() == "{" {
                self.advance();
                let mut pairs: Vec<(Vec<u8>, i32, String)> = Vec::new();
                while !self.at_end() && self.peek() != "}" {
                    if self.peek() == "," || self.peek() == ";" { self.advance(); continue; }
                    let tok = self.advance().to_string();
                    // Key — IPv4 or integer. No range support here
                    // because vmaps don't use intervals.
                    let key: Option<Vec<u8>> = if data_len == 4 {
                        Self::parse_ipv4(&tok).map(|a| a.to_vec())
                            .or_else(|| tok.parse::<u32>().ok().map(|n| n.to_be_bytes().to_vec()))
                    } else if data_len == 2 {
                        tok.parse::<u16>().ok().map(|n| n.to_be_bytes().to_vec())
                    } else { None };
                    if key.is_none() { continue; }
                    self.matches(":");
                    let vtok = self.advance().to_string();
                    let (code, cn) = match vtok.as_str() {
                        "accept" => (NF_ACCEPT, String::new()),
                        "drop" => (NF_DROP, String::new()),
                        "return" => (NFT_RETURN, String::new()),
                        "continue" => (NFT_CONTINUE, String::new()),
                        "jump" => (NFT_JUMP, self.advance().to_string()),
                        "goto" => (NFT_GOTO, self.advance().to_string()),
                        _ => continue,
                    };
                    pairs.push((key.unwrap(), code, cn));
                }
                self.matches("}");
                cmd.exprs.push(Expr::AnonVmap { pairs, width: data_len });
                return;
            }
            return;
        }

        let op = self.parse_cmp_op();

        // Named-set reference: `ip saddr @blocked drop`. Emits a
        // lookup expression that matches against the set referenced
        // by the preceding Meta / Payload load. NFT_CMP_NEQ becomes
        // the inverted flag on the lookup.
        if self.peek().starts_with('@') {
            let tok = self.advance().to_string();
            let set_name = tok.trim_start_matches('@').to_string();
            let inverted = op == NFT_CMP_NEQ;
            cmd.exprs.push(Expr::Lookup { set_name, inverted, width: data_len });
            return;
        }

        // Check for set literal { ... } — an anonymous set. Previously
        // this lowered to multiple Expr::Cmp entries which the kernel
        // ANDed together (so `ip saddr { 1.1.1.1, 2.2.2.2 }` matched
        // nothing). Now we emit a single Expr::AnonSet and let
        // add_rule allocate a __setN, NEWSET it, populate elements,
        // and rewrite this into an Expr::Lookup.
        if self.peek() == "{" {
            self.advance();
            let inverted = op == NFT_CMP_NEQ;
            let mut elements: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
            while !self.at_end() && self.peek() != "}" {
                if self.peek() == "," || self.peek() == ";" { self.advance(); continue; }
                let tok = self.advance().to_string();

                // CIDR: expand `1.1.1.0/24` to [start, end] (IPv4 only
                // for now; non-IP-width callers should never produce
                // CIDR tokens).
                if data_len == 4 {
                    if let Some(slash) = tok.find('/') {
                        if let (Some(addr), Ok(prefix)) =
                            (Self::parse_ipv4(&tok[..slash]), tok[slash + 1..].parse::<u32>())
                        {
                            let prefix = prefix.min(32);
                            let base = u32::from_be_bytes(addr);
                            let host_mask = if prefix == 32 { 0u32 } else { (1u32 << (32 - prefix)) - 1 };
                            let start = base & !host_mask;
                            let end = start | host_mask;
                            elements.push((
                                start.to_be_bytes().to_vec(),
                                Some(end.to_be_bytes().to_vec()),
                            ));
                            continue;
                        }
                    }
                }

                // Bare value plus optional `-` range tail. For 4-byte
                // fields the value may be an IPv4 address (`1.1.1.1`)
                // or an integer literal (`123`, `0x7b`) depending on
                // context — `meta mark`, `ct mark`, etc. use integer
                // form. Try address first, fall back to integer.
                let parse_scalar = |tok: &str| -> Option<Vec<u8>> {
                    if data_len == 4 {
                        if let Some(a) = Self::parse_ipv4(tok) { return Some(a.to_vec()); }
                        let n: Option<u32> = if let Some(s) = tok.strip_prefix("0x") {
                            u32::from_str_radix(s, 16).ok()
                        } else { tok.parse::<u32>().ok() };
                        n.map(|n| n.to_be_bytes().to_vec())
                    } else if data_len == 2 {
                        tok.parse::<u16>().ok().map(|n| n.to_be_bytes().to_vec())
                    } else if data_len == 1 {
                        // Symbolic protocol/icmp-type names — `meta l4proto
                        // { tcp, udp }` and `icmp type { echo-request,
                        // echo-reply }` are upstream idioms. Try the
                        // protocol table and the icmp-type table before
                        // falling back to numeric parsing so the brace
                        // body doesn't silently become `{ }`.
                        if let Some(p) = proto_num(tok) { return Some(vec![p]); }
                        if let Some(t) = icmp_type_num(tok) { return Some(vec![t]); }
                        if let Some(s) = tok.strip_prefix("0x") {
                            u8::from_str_radix(s, 16).ok().map(|n| vec![n])
                        } else {
                            tok.parse::<u8>().ok().map(|n| vec![n])
                        }
                    } else {
                        tok.parse::<u32>().ok().map(|n| n.to_be_bytes().to_vec())
                    }
                };
                let start: Option<Vec<u8>> = parse_scalar(&tok);

                if let Some(s) = start {
                    let end = if self.peek() == "-" {
                        self.advance();
                        let end_tok = self.advance().to_string();
                        parse_scalar(&end_tok)
                    } else { None };
                    elements.push((s, end));
                    continue;
                }

                // Fallback: bytes-pad via parse_value would consume
                // additional tokens and is wrong here. Silently skip
                // unparseable tokens; this matches the old loop's
                // behaviour for unknown entries.
            }
            self.matches("}");
            cmd.exprs.push(Expr::AnonSet { elements, width: data_len, inverted });
            return;
        }

        // Check for CIDR
        let tok = self.peek().to_string();
        if let Some(slash) = tok.find('/') {
            if let Some(addr_bytes) = Self::parse_ipv4(&tok[..slash]) {
                let prefix_len: u32 = tok[slash+1..].parse().unwrap_or(32);
                self.advance();

                // Generate bitwise mask
                let mut mask = [0u8; 4];
                for i in 0..prefix_len.min(32) {
                    mask[(i / 8) as usize] |= 1 << (7 - (i % 8));
                }
                let xor = [0u8; 4];
                let masked: Vec<u8> = addr_bytes.iter().zip(mask.iter()).map(|(a, m)| a & m).collect();

                cmd.exprs.push(Expr::Bitwise { mask: mask.to_vec(), xor: xor.to_vec() });
                cmd.exprs.push(Expr::Cmp { op, data: masked });
                return;
            }
        }

        let data = self.parse_value(data_len);
        cmd.exprs.push(Expr::Cmp { op, data });
    }

    fn parse_chain_block(&mut self, cmd: &mut Command) -> Result<(), String> {
        if !self.matches("{") { return Ok(()); }
        while !self.at_end() && self.peek() != "}" {
            if self.matches("type") {
                cmd.chain_spec.is_base = true;
                cmd.chain_spec.chain_type = self.parse_name();
                if self.matches("hook") { cmd.chain_spec.hook = self.parse_name(); }
                if self.matches("device") { cmd.chain_spec.device = self.parse_name(); cmd.chain_spec.has_device = true; }
                if self.matches("priority") {
                    let ct = cmd.chain_spec.chain_type.clone();
                    let hk = cmd.chain_spec.hook.clone();
                    cmd.chain_spec.priority = self.parse_priority_ctx(cmd.family, &ct, &hk)?;
                    cmd.chain_spec.has_priority = true;
                }
                self.matches(";");
            } else if self.matches("policy") {
                let pol = self.parse_name();
                if pol != "accept" && pol != "drop" {
                    return Err(format!("unknown policy '{}'", pol));
                }
                cmd.chain_spec.policy = pol;
                cmd.chain_spec.has_policy = true;
                self.matches(";");
            } else if self.matches(";") {
            } else { self.advance(); }
        }
        self.matches("}");
        Ok(())
    }

    fn parse_set_block(&mut self, cmd: &mut Command) {
        if !self.matches("{") { return; }
        while !self.at_end() && self.peek() != "}" {
            if self.matches("type") {
                let type_name = self.advance().to_string();
                let (kt, kl) = crate::netlink::set_key_type(&type_name);
                cmd.set_spec.key_type_name = type_name;
                cmd.set_spec.key_type = kt;
                cmd.set_spec.key_len = kl;
                if self.peek() == ":" {
                    self.advance();
                    cmd.set_spec.is_map = true;
                    let dt_name = self.advance().to_string();
                    let (dt, dl) = crate::netlink::set_key_type(&dt_name);
                    cmd.set_spec.data_type_name = dt_name;
                    cmd.set_spec.data_type = dt;
                    cmd.set_spec.data_len = dl;
                }
                self.matches(";");
            } else if self.matches("flags") {
                cmd.set_spec.has_flags = true;
                while !self.at_end() && self.peek() != ";" {
                    match self.peek() {
                        "constant" => { cmd.set_spec.flags |= 0x2; self.advance(); }
                        "interval" => { cmd.set_spec.flags |= 0x4; self.advance(); }
                        "," => { self.advance(); }
                        _ => { self.advance(); }
                    }
                }
                self.matches(";");
            } else if self.matches(";") {
            } else { self.advance(); }
        }
        self.matches("}");
    }

    fn parse_element_block(&mut self, cmd: &mut Command) {
        if !self.matches("{") { return; }
        while !self.at_end() && self.peek() != "}" {
            // Newlines tokenise to `;`; inside an element list they
            // are just whitespace, so skip along with `,`.
            if self.peek() == "," || self.peek() == ";" { self.advance(); continue; }
            let tok = self.advance().to_string();

            // CIDR first — an interval-set element written as
            // `1.1.1.0/24` expands to the range [1.1.1.0, 1.1.1.255]
            // so add_elements' INTERVAL_END expansion works. Not
            // special-cased for plain sets: a CIDR in a non-interval
            // set is a user error and the kernel will reject it.
            if let Some(slash) = tok.find('/') {
                let addr_part = &tok[..slash];
                let prefix_part = &tok[slash + 1..];
                if let (Some(addr), Ok(prefix)) = (Self::parse_ipv4(addr_part), prefix_part.parse::<u32>()) {
                    let prefix = prefix.min(32);
                    let base = u32::from_be_bytes(addr);
                    let host_mask = if prefix == 32 { 0u32 } else { (1u32 << (32 - prefix)) - 1 };
                    let start = base & !host_mask;
                    let end = start | host_mask;
                    cmd.elements.push((
                        start.to_be_bytes().to_vec(),
                        Some(end.to_be_bytes().to_vec()),
                    ));
                    if self.peek() == ":" { self.parse_element_value(cmd); }
                    continue;
                }
                if let (Some(addr), Ok(prefix)) = (Self::parse_ipv6(addr_part), prefix_part.parse::<u32>()) {
                    let prefix = prefix.min(128);
                    let base = u128::from_be_bytes(addr);
                    let host_mask: u128 = if prefix == 128 { 0 } else { (1u128 << (128 - prefix)) - 1 };
                    let start = base & !host_mask;
                    let end = start | host_mask;
                    cmd.elements.push((
                        start.to_be_bytes().to_vec(),
                        Some(end.to_be_bytes().to_vec()),
                    ));
                    if self.peek() == ":" { self.parse_element_value(cmd); }
                    continue;
                }
            }

            // Try address / port first so we can peek for a following `-`
            // which indicates a range. IPv6 comes first because it'd also
            // be reflected in parse_ipv4 failures — any unambiguous IPv6
            // representation (with `::` or >3 colons) parses cleanly here.
            let start: Option<Vec<u8>> = if let Some(addr) = Self::parse_ipv6(&tok) {
                Some(addr.to_vec())
            } else if let Some(addr) = Self::parse_ipv4(&tok) {
                Some(addr.to_vec())
            } else if let Some(mac) = parse_mac(&tok) {
                // Ether-addr typed sets: `add element bridge filter
                // blocked_macs { aa:bb:cc:dd:ee:ff }`. Without this
                // path the MAC falls through to the string fallback
                // and ends up as the literal ASCII bytes, so the
                // kernel's set lookup never matches the actual MAC
                // it gets from the bridge frame.
                Some(mac.to_vec())
            } else if let Ok(n) = tok.parse::<u32>() {
                Some(n.to_be_bytes().to_vec())
            } else {
                None
            };

            if let Some(s) = start {
                let end = if self.peek() == "-" {
                    self.advance();
                    let end_tok = self.advance().to_string();
                    Self::parse_ipv6(&end_tok).map(|a| a.to_vec())
                        .or_else(|| Self::parse_ipv4(&end_tok).map(|a| a.to_vec()))
                        .or_else(|| end_tok.parse::<u32>().ok().map(|n| n.to_be_bytes().to_vec()))
                } else { None };
                cmd.elements.push((s, end));
                if self.peek() == ":" { self.parse_element_value(cmd); }
                continue;
            }

            // String element (interface name, etc.) — no range semantics.
            let raw = if tok.starts_with('"') && tok.ends_with('"') && tok.len() >= 2 {
                &tok[1..tok.len()-1]
            } else { tok.as_str() };
            let mut v = raw.as_bytes().to_vec();
            v.push(0);
            cmd.elements.push((v, None));
            if self.peek() == ":" { self.parse_element_value(cmd); }
        }
        self.matches("}");
    }

    /// After a `key :`, either a verdict keyword (accept/drop/jump...)
    /// or a data value (ipv4/ipv6/integer/string) follows. For
    /// verdicts the parsed `(code, chain)` goes to element_verdicts
    /// and None is pushed into element_datas so indexes stay aligned.
    /// For data values the raw bytes go to element_datas and a
    /// stub entry (0, "") goes into element_verdicts.
    fn parse_element_value(&mut self, cmd: &mut Command) {
        self.advance(); // ":"
        let v = self.peek().to_string();
        match v.as_str() {
            "accept" | "drop" | "return" | "continue" | "jump" | "goto" => {
                let t = self.advance().to_string();
                let (code, chain) = match t.as_str() {
                    "accept" => (NF_ACCEPT, String::new()),
                    "drop" => (NF_DROP, String::new()),
                    "return" => (NFT_RETURN, String::new()),
                    "continue" => (NFT_CONTINUE, String::new()),
                    "jump" => (NFT_JUMP, self.advance().to_string()),
                    "goto" => (NFT_GOTO, self.advance().to_string()),
                    _ => (NF_ACCEPT, String::new()),
                };
                cmd.element_verdicts.push((code, chain));
                cmd.element_datas.push(None);
            }
            _ => {
                let t = self.advance().to_string();
                let bytes = if let Some(a) = Self::parse_ipv4(&t) {
                    Some(a.to_vec())
                } else if let Ok(n) = t.parse::<u32>() {
                    Some(n.to_be_bytes().to_vec())
                } else if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
                    let mut v = t[1..t.len()-1].as_bytes().to_vec();
                    v.push(0);
                    // Pad to 16 bytes (iface-name len).
                    while v.len() < 16 { v.push(0); }
                    Some(v)
                } else { None };
                cmd.element_verdicts.push((0, String::new()));
                cmd.element_datas.push(bytes);
            }
        }
    }

    /// Parse the body of `add counter/quota/limit name { ... }`. Each
    /// object kind has its own keyword set; we dispatch on
    /// `cmd.obj_spec.kind` (already set by the caller). Unknown tokens
    /// inside the block are skipped so mismatched listings don't abort.
    fn parse_object_block(&mut self, cmd: &mut Command) {
        if !self.matches("{") { return; }
        while !self.at_end() && self.peek() != "}" {
            // Quota body can start with a bare number + unit:
            //   quota q { 25 mbytes }
            if cmd.obj_spec.kind == NFT_OBJECT_QUOTA {
                if let Ok(n) = self.peek().parse::<u64>() {
                    self.advance();
                    let mul = match self.peek() {
                        "bytes"  => 1u64,
                        "kbytes" => 1024,
                        "mbytes" => 1024 * 1024,
                        "gbytes" => 1024 * 1024 * 1024,
                        _        => 1,
                    };
                    if matches!(self.peek(), "bytes" | "kbytes" | "mbytes" | "gbytes") {
                        self.advance();
                    }
                    cmd.obj_spec.bytes = n * mul;
                    // Optional `used N {unit}` — the amount already
                    // accounted for, reported back by `list quota`.
                    if self.peek() == "used" {
                        self.advance();
                        let _used: u64 = self.advance().parse().unwrap_or(0);
                        if matches!(self.peek(), "bytes" | "kbytes" | "mbytes" | "gbytes") {
                            self.advance();
                        }
                    }
                    continue;
                }
            }
            match self.peek() {
                ";" => { self.advance(); }
                "packets" => {
                    self.advance();
                    if let Ok(n) = self.advance().parse::<u64>() {
                        if cmd.obj_spec.kind == NFT_OBJECT_LIMIT {
                            cmd.obj_spec.limit_type = 0; // NFT_LIMIT_PKTS
                        } else {
                            cmd.obj_spec.packets = n;
                        }
                    }
                }
                "bytes" => {
                    self.advance();
                    if let Ok(n) = self.advance().parse::<u64>() {
                        cmd.obj_spec.bytes = n;
                    }
                }
                "over" => {
                    self.advance();
                    cmd.obj_spec.quota_flags |= NFT_QUOTA_F_INV;
                    // `over N bytes` — consume the number + unit.
                    if let Ok(n) = self.advance().parse::<u64>() { cmd.obj_spec.bytes = n; }
                    if matches!(self.peek(), "bytes" | "kbytes" | "mbytes" | "gbytes") { self.advance(); }
                }
                "rate" => {
                    self.advance();
                    // optional `over` invert
                    if self.peek() == "over" { self.advance(); cmd.obj_spec.limit_flags |= 1; }
                    // `rate` value can arrive as either a bare number
                    // followed by `/` + unit, or as a single word like
                    // `10/second` (the tokenizer doesn't split on `/`
                    // because `/` is used by CIDR). Handle both.
                    let tok = self.advance().to_string();
                    if let Some(slash) = tok.find('/') {
                        cmd.obj_spec.limit_rate = tok[..slash].parse::<u64>().unwrap_or(0);
                        cmd.obj_spec.limit_unit = match &tok[slash + 1..] {
                            "second" => 1, "minute" => 60, "hour" => 3600,
                            "day" => 86400, "week" => 604800, _ => 1,
                        };
                    } else if let Ok(n) = tok.parse::<u64>() {
                        cmd.obj_spec.limit_rate = n;
                        if self.peek() == "/" {
                            self.advance();
                            cmd.obj_spec.limit_unit = match self.advance() {
                                "second" => 1, "minute" => 60, "hour" => 3600,
                                "day" => 86400, "week" => 604800, _ => 1,
                            };
                        } else {
                            cmd.obj_spec.limit_unit = 1;
                        }
                    }
                }
                "burst" => {
                    self.advance();
                    if let Ok(n) = self.advance().parse::<u32>() {
                        cmd.obj_spec.limit_burst = n;
                    }
                    if matches!(self.peek(), "packets" | "bytes") { self.advance(); }
                }
                _ => { self.advance(); }
            }
        }
        self.matches("}");
    }

    fn parse_table_block(&mut self, cmd: &mut Command) {
        if !self.matches("{") { return; }
        while !self.at_end() && self.peek() != "}" {
            if self.matches("flags") {
                cmd.table_spec.has_flags = true;
                while self.peek() != ";" && !self.at_end() {
                    if self.peek() == "dormant" { cmd.table_spec.flags |= NFT_TABLE_F_DORMANT; }
                    self.advance();
                }
                self.matches(";");
            } else if self.matches(";") {
            } else { self.advance(); }
        }
        self.matches("}");
    }

    /// Parse a top-level declarative `table FAMILY NAME { ... }` block
    /// (the form used in /etc/nftables.conf and the output of `nft list
    /// ruleset`). Emits synthetic `add table` / `add chain` / `add set`
    /// / `add rule` commands into `out`.
    fn parse_declarative_table(&mut self, out: &mut Vec<Command>) -> Result<(), String> {
        // leading "table"
        self.advance();
        let family = self.parse_family_default_ip();
        let table_name = self.parse_name();

        // Emit `add table FAMILY NAME`
        let mut tcmd = Command::default();
        tcmd.op = CmdOp::Add;
        tcmd.obj = CmdObj::Table;
        tcmd.family = family;
        tcmd.table = table_name.clone();
        out.push(tcmd);

        // Rules may reference chains defined later in the same block
        // (jump/goto targets, or rules before the forward chain).
        // In an atomic batch those references fail with ENOENT. So
        // collect chains separately from rules and flush all
        // chains first, then rules — matches libnftables ordering.
        let mut deferred_rules: Vec<Command> = Vec::new();

        if !self.matches("{") { return Ok(()); }

        while !self.at_end() && self.peek() != "}" {
            while self.matches(";") {}
            if self.peek() == "}" { break; }

            let tok = self.peek().to_string();
            match tok.as_str() {
                "flags" => {
                    self.advance();
                    // Rule body has no `;` after `flags dormant` when the
                    // writer used newlines, so bail on any token that
                    // could start the next member as well.
                    loop {
                        let p = self.peek();
                        if self.at_end() || p == ";" || p == "}"
                            || p == "chain" || p == "set" || p == "map"
                            || p == "counter" || p == "quota" || p == "limit"
                        { break; }
                        if p == "dormant" {
                            if let Some(last) = out.iter_mut().rev().find(|c| matches!(c.obj, CmdObj::Table)) {
                                last.table_spec.has_flags = true;
                                last.table_spec.flags |= NFT_TABLE_F_DORMANT;
                            }
                        }
                        self.advance();
                    }
                    self.matches(";");
                }
                "chain" => {
                    self.advance();
                    let chain_name = self.parse_name();
                    let mut ccmd = Command::default();
                    ccmd.op = CmdOp::Add;
                    ccmd.obj = CmdObj::Chain;
                    ccmd.family = family;
                    ccmd.table = table_name.clone();
                    ccmd.chain = chain_name.clone();

                    // Collect rule commands emitted from statements in the chain body.
                    let mut rules: Vec<Command> = Vec::new();
                    if self.matches("{") {
                        while !self.at_end() && self.peek() != "}" {
                            while self.matches(";") {}
                            if self.peek() == "}" { break; }
                            let t = self.peek().to_string();
                            if t == "type" {
                                self.advance();
                                ccmd.chain_spec.is_base = true;
                                ccmd.chain_spec.chain_type = self.parse_name();
                                if self.matches("hook") { ccmd.chain_spec.hook = self.parse_name(); }
                                if self.matches("device") {
                                    ccmd.chain_spec.device = self.parse_name();
                                    ccmd.chain_spec.has_device = true;
                                }
                                if self.matches("priority") {
                                    let ct = ccmd.chain_spec.chain_type.clone();
                                    let hk = ccmd.chain_spec.hook.clone();
                                    ccmd.chain_spec.priority = self.parse_priority_ctx(family, &ct, &hk)?;
                                    ccmd.chain_spec.has_priority = true;
                                }
                                self.matches(";");
                            } else if t == "policy" {
                                self.advance();
                                let pol = self.parse_name();
                                if pol != "accept" && pol != "drop" {
                                    return Err(format!("unknown policy '{}'", pol));
                                }
                                ccmd.chain_spec.policy = pol;
                                ccmd.chain_spec.has_policy = true;
                                self.matches(";");
                            } else {
                                // A rule statement: consume tokens up to next ';' or '}'
                                // into a fresh rule command. Fail if the line
                                // was entirely unknown — a rule that silently
                                // lexes to no expressions would otherwise let
                                // garbage rulesets load (upstream's rollback
                                // tests rely on bad syntax failing).
                                let before = self.pos;
                                let mut rcmd = Command::default();
                                rcmd.op = CmdOp::Add;
                                rcmd.obj = CmdObj::Rule;
                                rcmd.family = family;
                                rcmd.table = table_name.clone();
                                rcmd.chain = chain_name.clone();
                                self.parse_rule_exprs(&mut rcmd);
                                self.matches(";");
                                if !rcmd.exprs.is_empty() || rcmd.has_comment {
                                    rules.push(rcmd);
                                } else if self.pos != before {
                                    return Err(format!("unknown rule statement near '{}'", t));
                                }
                            }
                        }
                        self.matches("}");
                    }
                    out.push(ccmd);
                    deferred_rules.extend(rules);
                }
                "set" | "map" => {
                    let is_map = tok == "map";
                    self.advance();
                    let set_name = self.parse_name();
                    let mut scmd = Command::default();
                    scmd.op = CmdOp::Add;
                    scmd.obj = if is_map { CmdObj::Map } else { CmdObj::Set };
                    scmd.family = family;
                    scmd.table = table_name.clone();
                    scmd.set_name = set_name.clone();
                    if is_map { scmd.set_spec.is_map = true; }

                    // Collect elements separately so we can emit them as
                    // a follow-up `add element` command.
                    let mut elements: Vec<(Vec<u8>, Option<Vec<u8>>)> = Vec::new();
                    let mut element_verdicts: Vec<(i32, String)> = Vec::new();
                    let mut element_datas: Vec<Option<Vec<u8>>> = Vec::new();
                    if self.matches("{") {
                        while !self.at_end() && self.peek() != "}" {
                            while self.matches(";") {}
                            if self.peek() == "}" { break; }
                            if self.matches("type") {
                                let type_name = self.advance().to_string();
                                let (kt, kl) = crate::netlink::set_key_type(&type_name);
                                scmd.set_spec.key_type_name = type_name;
                                scmd.set_spec.key_type = kt;
                                scmd.set_spec.key_len = kl;
                                // Map type declared as `type KEY : VALUE`.
                                if self.peek() == ":" {
                                    self.advance();
                                    scmd.set_spec.is_map = true;
                                    let dt_name = self.advance().to_string();
                                    let (dt, dl) = crate::netlink::set_key_type(&dt_name);
                                    scmd.set_spec.data_type_name = dt_name;
                                    scmd.set_spec.data_type = dt;
                                    scmd.set_spec.data_len = dl;
                                }
                                self.matches(";");
                            } else if self.matches("flags") {
                                scmd.set_spec.has_flags = true;
                                // Stop at ";" OR "}" OR any next-attribute keyword.
                                // Inside braces newlines no longer inject ";", so
                                // "flags interval elements = {...}" must terminate
                                // on the keyword "elements" rather than running off
                                // the end of the set body.
                                loop {
                                    let p = self.peek();
                                    if self.at_end() || p == ";" || p == "}"
                                        || p == "elements" || p == "type"
                                        || p == "size" || p == "timeout"
                                        || p == "gc-interval" || p == "policy"
                                        || p == "auto-merge"
                                    { break; }
                                    match p {
                                        "constant" => { scmd.set_spec.flags |= 0x2; self.advance(); }
                                        "interval" => { scmd.set_spec.flags |= 0x4; self.advance(); }
                                        "," => { self.advance(); }
                                        _ => { self.advance(); }
                                    }
                                }
                                self.matches(";");
                            } else if self.matches("elements") {
                                self.matches("=");
                                if self.matches("{") {
                                    while !self.at_end() && self.peek() != "}" {
                                        if self.peek() == "," { self.advance(); continue; }
                                        let tok = self.advance().to_string();
                                        // CIDR shorthand → [addr, broadcast].
                                        if let Some(slash) = tok.find('/') {
                                            let addr_part = &tok[..slash];
                                            let prefix_part = &tok[slash + 1..];
                                            let cidr_pair: Option<(Vec<u8>, Vec<u8>)> =
                                                if let (Some(addr), Ok(prefix)) = (Self::parse_ipv4(addr_part), prefix_part.parse::<u32>()) {
                                                    let p = prefix.min(32);
                                                    let base = u32::from_be_bytes(addr);
                                                    let host_mask = if p == 32 { 0u32 } else { (1u32 << (32 - p)) - 1 };
                                                    let s_i = base & !host_mask;
                                                    let e_i = s_i | host_mask;
                                                    Some((s_i.to_be_bytes().to_vec(), e_i.to_be_bytes().to_vec()))
                                                } else if let (Some(addr), Ok(prefix)) = (Self::parse_ipv6(addr_part), prefix_part.parse::<u32>()) {
                                                    let p = prefix.min(128);
                                                    let base = u128::from_be_bytes(addr);
                                                    let host_mask: u128 = if p == 128 { 0 } else { (1u128 << (128 - p)) - 1 };
                                                    let s_i = base & !host_mask;
                                                    let e_i = s_i | host_mask;
                                                    Some((s_i.to_be_bytes().to_vec(), e_i.to_be_bytes().to_vec()))
                                                } else { None };
                                            if let Some((s_b, e_b)) = cidr_pair {
                                                elements.push((s_b, Some(e_b)));
                                                if self.peek() == ":" {
                                                    self.advance();
                                                    let vtok = self.advance().to_string();
                                                    let bytes = Self::parse_ipv6(&vtok).map(|a| a.to_vec())
                                                        .or_else(|| Self::parse_ipv4(&vtok).map(|a| a.to_vec()))
                                                        .or_else(|| vtok.parse::<u32>().ok().map(|n| n.to_be_bytes().to_vec()));
                                                    element_verdicts.push((0, String::new()));
                                                    element_datas.push(bytes);
                                                }
                                                continue;
                                            }
                                        }
                                        let start: Option<Vec<u8>> =
                                            if let Some(addr) = Self::parse_ipv6(&tok) {
                                                Some(addr.to_vec())
                                            } else if let Some(addr) = Self::parse_ipv4(&tok) {
                                                Some(addr.to_vec())
                                            } else if let Ok(n) = tok.parse::<u32>() {
                                                Some(n.to_be_bytes().to_vec())
                                            } else { None };
                                        if let Some(s) = start {
                                            let end = if self.peek() == "-" {
                                                self.advance();
                                                let et = self.advance().to_string();
                                                Self::parse_ipv6(&et).map(|a| a.to_vec())
                                                    .or_else(|| Self::parse_ipv4(&et).map(|a| a.to_vec()))
                                                    .or_else(|| et.parse::<u32>().ok().map(|n| n.to_be_bytes().to_vec()))
                                            } else { None };
                                            elements.push((s, end));
                                        } else {
                                            let raw = if tok.starts_with('"') && tok.ends_with('"') && tok.len() >= 2 {
                                                tok[1..tok.len()-1].to_string()
                                            } else { tok };
                                            let mut v = raw.as_bytes().to_vec();
                                            v.push(0);
                                            elements.push((v, None));
                                        }
                                        // Inline `key : value` for maps — either a verdict
                                        // (accept/drop/jump c/...) or a data value matching
                                        // the declared data_type (ipv4/integer/ifname).
                                        if self.peek() == ":" {
                                            self.advance();
                                            let v = self.peek().to_string();
                                            match v.as_str() {
                                                "accept" | "drop" | "return" | "continue" | "jump" | "goto" => {
                                                    let t = self.advance().to_string();
                                                    let (code, chain) = match t.as_str() {
                                                        "accept" => (NF_ACCEPT, String::new()),
                                                        "drop" => (NF_DROP, String::new()),
                                                        "return" => (NFT_RETURN, String::new()),
                                                        "continue" => (NFT_CONTINUE, String::new()),
                                                        "jump" => (NFT_JUMP, self.advance().to_string()),
                                                        "goto" => (NFT_GOTO, self.advance().to_string()),
                                                        _ => (NF_ACCEPT, String::new()),
                                                    };
                                                    element_verdicts.push((code, chain));
                                                    element_datas.push(None);
                                                }
                                                _ => {
                                                    let t = self.advance().to_string();
                                                    let bytes = if let Some(a) = Self::parse_ipv4(&t) {
                                                        Some(a.to_vec())
                                                    } else if let Ok(n) = t.parse::<u32>() {
                                                        Some(n.to_be_bytes().to_vec())
                                                    } else if t.starts_with('"') && t.ends_with('"') && t.len() >= 2 {
                                                        let mut v = t[1..t.len()-1].as_bytes().to_vec();
                                                        v.push(0);
                                                        while v.len() < 16 { v.push(0); }
                                                        Some(v)
                                                    } else { None };
                                                    element_verdicts.push((0, String::new()));
                                                    element_datas.push(bytes);
                                                }
                                            }
                                        }
                                    }
                                    self.matches("}");
                                }
                                self.matches(";");
                            } else {
                                // Unknown attribute inside set block; consume to next ;
                                while !self.at_end() && self.peek() != ";" && self.peek() != "}" {
                                    self.advance();
                                }
                                self.matches(";");
                            }
                        }
                        self.matches("}");
                    }
                    out.push(scmd);
                    if !elements.is_empty() {
                        let mut ecmd = Command::default();
                        ecmd.op = CmdOp::Add;
                        ecmd.obj = CmdObj::Element;
                        ecmd.family = family;
                        ecmd.table = table_name.clone();
                        ecmd.set_name = set_name;
                        ecmd.elements = elements;
                        ecmd.element_verdicts = element_verdicts;
                        ecmd.element_datas = element_datas;
                        out.push(ecmd);
                    }
                }
                "flags" => {
                    // Inherited from the minimal parse_table_block; table-level flags.
                    self.advance();
                    while self.peek() != ";" && !self.at_end() {
                        self.advance();
                    }
                    self.matches(";");
                }
                _ => {
                    // Unknown top-level element inside table; skip to next ;.
                    while !self.at_end() && self.peek() != ";" && self.peek() != "}" {
                        self.advance();
                    }
                    self.matches(";");
                }
            }
        }
        self.matches("}");
        out.extend(deferred_rules);
        Ok(())
    }

    fn parse_command(&mut self) -> Result<Option<Command>, String> {
        while self.matches(";") {}
        if self.at_end() { return Ok(None); }

        let mut cmd = Command::default();

        // Operation
        let op_str = self.advance().to_string();
        cmd.op = match op_str.as_str() {
            "add" => CmdOp::Add, "create" => CmdOp::Create, "delete" => CmdOp::Delete,
            "destroy" => CmdOp::Destroy, "list" => CmdOp::List, "flush" => CmdOp::Flush,
            "rename" => CmdOp::Rename, "insert" => CmdOp::Insert, "replace" => CmdOp::Replace,
            "reset" => CmdOp::Reset,
            _ => return Err(format!("expected command, got '{}'", op_str)),
        };

        // Object type
        let obj_str = self.advance().to_string();
        cmd.obj = match obj_str.as_str() {
            "table" => CmdObj::Table, "chain" => CmdObj::Chain, "rule" => CmdObj::Rule,
            "set" => CmdObj::Set, "map" => CmdObj::Map, "element" => CmdObj::Element,
            "ruleset" => CmdObj::Ruleset, "tables" => CmdObj::Tables, "chains" => CmdObj::Chains,
            "rules" => CmdObj::Rules, "sets" => CmdObj::Sets, "maps" => CmdObj::Maps,
            "counter" => CmdObj::Counter, "quota" => CmdObj::Quota, "limit" => CmdObj::Limit,
            "counters" => CmdObj::Counters, "quotas" => CmdObj::Quotas, "limits" => CmdObj::Limits,
            _ => return Err(format!("expected object type, got '{}'", obj_str)),
        };

        // Ruleset/plural: optional family, default UNSPEC
        match cmd.obj {
            CmdObj::Ruleset | CmdObj::Tables | CmdObj::Chains | CmdObj::Rules |
            CmdObj::Sets | CmdObj::Maps |
            CmdObj::Counters | CmdObj::Quotas | CmdObj::Limits => {
                cmd.family = self.parse_family_default_unspec();
                return Ok(Some(cmd));
            }
            _ => {}
        }

        // [family] table
        cmd.family = self.parse_family_default_ip();

        // `delete table FAMILY handle N` / `destroy table FAMILY handle N`
        // — target a table by kernel handle instead of name. Upstream's
        // transactions/handle_bad_family round-trips the handle grabbed
        // from `nft -a -e add table ip x`.
        if matches!(cmd.obj, CmdObj::Table)
            && matches!(cmd.op, CmdOp::Delete | CmdOp::Destroy)
            && self.peek() == "handle"
        {
            self.advance();
            cmd.handle = self.advance().parse().unwrap_or(0);
            cmd.has_handle = true;
            return Ok(Some(cmd));
        }

        cmd.table = self.parse_name();

        // Table-only commands
        if matches!(cmd.obj, CmdObj::Table) {
            if matches!(cmd.op, CmdOp::Add | CmdOp::Create) && self.peek() == "{" {
                self.parse_table_block(&mut cmd);
            }
            return Ok(Some(cmd));
        }

        // Chain/rule/set/object need another name. For chain delete
        // (and rule delete) the name slot can carry `handle N` instead
        // of a named identifier — `nft delete chain TABLE handle 42`.
        match cmd.obj {
            CmdObj::Chain | CmdObj::Rule => {
                if matches!(cmd.op, CmdOp::Delete | CmdOp::Destroy) && self.peek() == "handle" {
                    self.advance();
                    cmd.handle = self.advance().parse().unwrap_or(0);
                    cmd.has_handle = true;
                    return Ok(Some(cmd));
                }
                cmd.chain = self.parse_name();
            }
            CmdObj::Set | CmdObj::Map | CmdObj::Element => { cmd.set_name = self.parse_name(); }
            CmdObj::Counter | CmdObj::Quota | CmdObj::Limit => { cmd.set_name = self.parse_name(); }
            _ => {}
        }

        // Named stateful-object body:   add counter t cnt { packets 0 bytes 0 ; }
        if matches!(cmd.obj, CmdObj::Counter | CmdObj::Quota | CmdObj::Limit)
            && matches!(cmd.op, CmdOp::Add | CmdOp::Create)
        {
            cmd.obj_spec.kind = match cmd.obj {
                CmdObj::Counter => NFT_OBJECT_COUNTER,
                CmdObj::Quota   => NFT_OBJECT_QUOTA,
                CmdObj::Limit   => NFT_OBJECT_LIMIT,
                _ => 0,
            };
            if self.peek() == "{" {
                self.parse_object_block(&mut cmd);
            } else if cmd.obj_spec.kind == NFT_OBJECT_QUOTA {
                // Inline quota form: `add quota t q 25 mbytes`.
                if let Ok(n) = self.peek().parse::<u64>() {
                    self.advance();
                    let mul = match self.peek() {
                        "bytes"  => 1u64,
                        "kbytes" => 1024,
                        "mbytes" => 1024 * 1024,
                        "gbytes" => 1024 * 1024 * 1024,
                        _        => 1,
                    };
                    if matches!(self.peek(), "bytes" | "kbytes" | "mbytes" | "gbytes") {
                        self.advance();
                    }
                    cmd.obj_spec.bytes = n * mul;
                }
            }
            return Ok(Some(cmd));
        }

        // Chain block
        if matches!(cmd.obj, CmdObj::Chain) && matches!(cmd.op, CmdOp::Add | CmdOp::Create) && self.peek() == "{" {
            self.parse_chain_block(&mut cmd)?;
            return Ok(Some(cmd));
        }

        // Chain rename
        if matches!(cmd.obj, CmdObj::Chain) && matches!(cmd.op, CmdOp::Rename) {
            cmd.new_name = self.parse_name();
            return Ok(Some(cmd));
        }

        // Set block
        if matches!(cmd.obj, CmdObj::Set | CmdObj::Map) && matches!(cmd.op, CmdOp::Add | CmdOp::Create) && self.peek() == "{" {
            self.parse_set_block(&mut cmd);
            if matches!(cmd.obj, CmdObj::Map) { cmd.set_spec.is_map = true; }
            return Ok(Some(cmd));
        }

        // Element block
        if matches!(cmd.obj, CmdObj::Element) && !self.at_end() && self.peek() == "{" {
            self.parse_element_block(&mut cmd);
            return Ok(Some(cmd));
        }

        // Rule positioning: `[add|insert|replace] rule T C [position N |
        // handle N | index N] EXPRS`. `position` and `handle` both take
        // the kernel rule handle as the anchor; `index` is a 0-based
        // ordinal within the chain that we resolve to a handle via a
        // GETRULE dump at exec time. All three set has_handle so the
        // encoder can attach NFTA_RULE_POSITION to the netlink msg.
        if matches!(cmd.obj, CmdObj::Rule) {
            match self.peek() {
                "handle" | "position" => {
                    self.advance();
                    cmd.handle = self.advance().parse().unwrap_or(0);
                    cmd.has_handle = true;
                }
                "index" => {
                    self.advance();
                    cmd.rule_index = self.advance().parse().unwrap_or(-1);
                    cmd.has_rule_index = true;
                }
                _ => {}
            }
            if matches!(cmd.op, CmdOp::Add | CmdOp::Insert | CmdOp::Replace) {
                self.parse_rule_exprs(&mut cmd);
            }
        }

        Ok(Some(cmd))
    }
}

pub fn parse(input: &str) -> Result<Vec<Command>, String> {
    // JSON detection: if the first non-whitespace byte is '{', treat the
    // input as an nft JSON command stream (as produced by `nft -j list
    // ruleset` or hand-written for `nft -j -f file.json`). This is the
    // same convention upstream libnftables uses internally.
    if input.trim_start().starts_with('{') {
        let v = crate::json::parse(input).map_err(|e| format!("json: {}", e))?;
        return json_to_commands(&v);
    }

    // Variable preprocessor. nft(8) documents `define NAME = value`
    // declarations and `$NAME` references; both are purely textual
    // (the kernel never sees them). Strip defines out of the input
    // and substitute uses inline before tokenising. A value can span
    // multiple tokens (`define ADDRS = { 1.1.1.1, 2.2.2.2 }`), so we
    // capture up to end-of-line, respecting balanced braces.
    let input = expand_variables(input)?;
    let input = input.as_str();

    let tokens = tokenize(input);
    let mut parser = Parser::new(tokens);
    let mut cmds = Vec::new();
    while !parser.at_end() {
        while parser.matches(";") {}
        if parser.at_end() { break; }

        // Declarative top-level block: `table FAMILY NAME { ... }` with no
        // leading verb. Standard /etc/nftables.conf style.
        if parser.peek() == "table" {
            parser.parse_declarative_table(&mut cmds)?;
            continue;
        }

        if let Some(cmd) = parser.parse_command()? {
            cmds.push(cmd);
        }
    }
    Ok(cmds)
}

// ── JSON command interpreter ────────────────────────────────────
//
// Walks the {"nftables": [...]} array and turns each element into a
// Command. Handles both the verb-wrapped form used for command scripts
//   {"add": {"table": {...}}}
// and the bare-object form used for dumps
//   {"table": {...}}.
// Rules inside a JSON dump carry a "raw" string field with the
// text-rendered rule body; we re-parse that body through the text
// parser so stormwall can round-trip its own JSON output (the
// upstream framework runs `-j --check -f <json>` on stormwall's own
// emission and expects it to succeed).

use crate::json::Value as JV;

fn json_to_commands(v: &JV) -> Result<Vec<Command>, String> {
    let root = v.as_object().ok_or("json: expected top-level object")?;
    let arr = root.get("nftables").and_then(|x| x.as_array())
        .ok_or("json: expected \"nftables\" array")?;
    let mut out = Vec::new();
    'items: for item in arr {
        let obj = match item.as_object() { Some(o) => o, None => continue };
        if obj.contains_key("metainfo") { continue; }

        // Verb-wrapped forms: {"add": {"table": {...}}}. Upstream emits
        // these for command scripts. Each object carries exactly one
        // verb key; the nested value is the object spec.
        for (verb, op) in &[("add", CmdOp::Add), ("create", CmdOp::Create),
                             ("delete", CmdOp::Delete), ("destroy", CmdOp::Destroy),
                             ("insert", CmdOp::Insert), ("replace", CmdOp::Replace),
                             ("flush", CmdOp::Flush), ("reset", CmdOp::Reset)] {
            if let Some(inner) = obj.get(*verb).and_then(|x| x.as_object()) {
                if let Some(cmd) = json_object_to_command(inner, op.clone())? {
                    out.push(cmd);
                }
                continue 'items;
            }
        }

        // Bare form (dump): {"table": {...}} → `add table ...`.
        if let Some(cmd) = json_object_to_command(obj, CmdOp::Add)? {
            out.push(cmd);
        }
    }
    Ok(out)
}

fn json_family(s: &str) -> u8 {
    match s {
        "ip" => NFPROTO_IPV4, "ip6" => NFPROTO_IPV6,
        "inet" => NFPROTO_INET, "arp" => NFPROTO_ARP,
        "bridge" => NFPROTO_BRIDGE, "netdev" => NFPROTO_NETDEV,
        _ => NFPROTO_IPV4,
    }
}

fn json_object_to_command(obj: &std::collections::BTreeMap<String, JV>, op: CmdOp)
    -> Result<Option<Command>, String>
{
    // Each object has exactly one key naming the nft object type.
    // Support the common ones: table, chain, rule, set, map, element.
    let (kind, body) = match obj.iter().next() {
        Some((k, v)) => (k.as_str(), v.as_object().ok_or("json: nested must be object")?),
        None => return Ok(None),
    };

    let mut cmd = Command::default();
    cmd.op = op;
    cmd.family = body.get("family").and_then(|x| x.as_str()).map(json_family).unwrap_or(NFPROTO_IPV4);
    cmd.table = body.get("table").and_then(|x| x.as_str()).unwrap_or("").to_string();

    match kind {
        "table" => {
            cmd.obj = CmdObj::Table;
            cmd.table = body.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        }
        "chain" => {
            cmd.obj = CmdObj::Chain;
            cmd.chain = body.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if let Some(t) = body.get("type").and_then(|x| x.as_str()) {
                cmd.chain_spec.chain_type = t.to_string();
                cmd.chain_spec.is_base = true;
            }
            if let Some(h) = body.get("hook").and_then(|x| x.as_str()) {
                cmd.chain_spec.hook = h.to_string();
            }
            if let Some(p) = body.get("prio").and_then(|x| x.as_i64()) {
                cmd.chain_spec.priority = p as i32;
                cmd.chain_spec.has_priority = true;
            }
            if let Some(pol) = body.get("policy").and_then(|x| x.as_str()) {
                cmd.chain_spec.policy = pol.to_string();
                cmd.chain_spec.has_policy = true;
            }
        }
        "rule" => {
            cmd.obj = CmdObj::Rule;
            cmd.chain = body.get("chain").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if let Some(h) = body.get("handle").and_then(|x| x.as_u64()) {
                cmd.handle = h; cmd.has_handle = true;
            }
            if let Some(c) = body.get("comment").and_then(|x| x.as_str()) {
                cmd.comment = c.to_string(); cmd.has_comment = true;
            }
            // If a "raw" text rendering is present (stormwall's own JSON
            // dump carries one), re-parse it through the text expression
            // parser so we can faithfully reconstruct the rule body.
            if let Some(raw) = body.get("raw").and_then(|x| x.as_str()) {
                if !raw.trim().is_empty() {
                    let synth = format!("add rule {} {} {} {}",
                        family_name_for_parse(cmd.family), cmd.table, cmd.chain, raw);
                    if let Ok(mut cmds) = parse(&synth) {
                        if let Some(inner) = cmds.pop() {
                            cmd.exprs = inner.exprs;
                            if inner.has_comment && !cmd.has_comment {
                                cmd.comment = inner.comment;
                                cmd.has_comment = true;
                            }
                        }
                    }
                }
            }
        }
        "set" | "map" => {
            cmd.obj = if kind == "map" { CmdObj::Map } else { CmdObj::Set };
            cmd.set_name = body.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if let Some(t) = body.get("type").and_then(|x| x.as_str()) {
                let (kt, kl) = crate::netlink::set_key_type(t);
                cmd.set_spec.key_type_name = t.to_string();
                cmd.set_spec.key_type = kt;
                cmd.set_spec.key_len = kl;
            }
        }
        "element" => {
            cmd.obj = CmdObj::Element;
            cmd.set_name = body.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        }
        _ => return Ok(None), // unknown — silently ignore for --check
    }
    Ok(Some(cmd))
}

fn family_name_for_parse(f: u8) -> &'static str {
    match f {
        NFPROTO_IPV4 => "ip", NFPROTO_IPV6 => "ip6",
        NFPROTO_INET => "inet", NFPROTO_ARP => "arp",
        NFPROTO_BRIDGE => "bridge", NFPROTO_NETDEV => "netdev",
        _ => "ip",
    }
}

// ── Variable preprocessor ────────────────────────────────────────
//
// Handles `define NAME = value` declarations and `$NAME` references in
// a single preprocessor pass over the input text, before tokenisation.
// Value extends from after `=` to the end of the logical statement:
// the first semicolon or newline at brace-depth 0. Braces and
// double-quoted strings are tracked so `define ADDRS = { 1.1.1.1,
// 2.2.2.2 }` or a value spanning multiple lines both work.

// Minimal shell-style glob: `*` matches any non-slash sequence, `?`
// matches a single non-slash char, anything else literal. Returns
// absolute/relative paths the caller passed through, lexicographically
// sorted — matching nft's `include` behaviour. If the pattern has no
// wildcards the result is just `[pattern]` (to preserve the error
// path for "file not found" errors at read time).
fn glob_expand(pattern: &str) -> Vec<String> {
    let has_wild = pattern.chars().any(|c| c == '*' || c == '?' || c == '[');
    if !has_wild { return vec![pattern.to_string()]; }

    // Split the pattern into (dir, filename-pattern).
    let (dir, fname) = match pattern.rfind('/') {
        Some(i) => (&pattern[..=i], &pattern[i + 1..]),
        None    => ("./", pattern),
    };

    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for e in entries.flatten() {
        let name = match e.file_name().into_string() { Ok(s) => s, Err(_) => continue };
        if name.starts_with('.') && !fname.starts_with('.') { continue; }
        if glob_match(fname, &name) {
            out.push(format!("{}{}", dir, name));
        }
    }
    out.sort();
    out
}

fn glob_match(pat: &str, s: &str) -> bool {
    // Standard recursive glob — `*` can match zero or more characters
    // within a single path component, `?` matches exactly one.
    let pat = pat.as_bytes();
    let s   = s.as_bytes();
    fn rec(p: &[u8], s: &[u8]) -> bool {
        if p.is_empty() { return s.is_empty(); }
        match p[0] {
            b'*' => {
                for i in 0..=s.len() {
                    if rec(&p[1..], &s[i..]) { return true; }
                }
                false
            }
            b'?' => !s.is_empty() && rec(&p[1..], &s[1..]),
            c    => !s.is_empty() && s[0] == c && rec(&p[1..], &s[1..]),
        }
    }
    rec(pat, s)
}

fn expand_variables(src: &str) -> Result<String, String> {
    let mut defs: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    expand_variables_with(&mut defs, src)
}

fn expand_variables_with(
    defs: &mut std::collections::HashMap<String, String>,
    src: &str,
) -> Result<String, String> {
    let mut out = String::with_capacity(src.len());

    let bytes = src.as_bytes();
    let mut i = 0usize;
    // Running brace depth so `$v` expansion can decide whether to
    // splice `{ ... }` values inline (stripped) vs. preserve the
    // braces when used outside another set literal. Matches upstream
    // where `elements = { 2.2.2.2, $addrs }` unfolds the brace-value.
    let mut brace_depth: i32 = 0;
    while i < bytes.len() {
        // Skip #-comments (preserve them in output).
        if bytes[i] == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' { out.push(bytes[i] as char); i += 1; }
            continue;
        }
        // `include "..."` — inline the referenced file. Matches
        // nft(8)'s directive. Relative paths are resolved against
        // the process cwd; wildcards are expanded with glob order.
        if bytes[i] == b'i'
            && bytes[i..].starts_with(b"include ")
            && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b';'))
        {
            let mut j = i + "include ".len();
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') { j += 1; }
            if j >= bytes.len() || bytes[j] != b'"' { i += 1; continue; }
            j += 1;
            let path_start = j;
            while j < bytes.len() && bytes[j] != b'"' { j += 1; }
            if j >= bytes.len() { return Err("unterminated include string".into()); }
            let path = std::str::from_utf8(&bytes[path_start..j]).map_err(|e| e.to_string())?.to_string();
            j += 1; // skip closing quote
            // Eat optional trailing ';' / '\n'.
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') { j += 1; }
            if j < bytes.len() && (bytes[j] == b';' || bytes[j] == b'\n') { j += 1; }

            // Expand glob patterns; load each matched file in sorted
            // order and recursively run the preprocessor on it so
            // defines and nested includes inside the file work. The
            // recursive call shares the defs map so the included
            // file's variables are visible to the including one.
            let paths = glob_expand(&path);
            for p in &paths {
                let body = std::fs::read_to_string(p)
                    .map_err(|e| format!("include {}: {}", p, e))?;
                let expanded = expand_variables_with(defs, &body)?;
                out.push_str(&expanded);
                out.push('\n');
            }
            i = j;
            continue;
        }

        // $NAME reference.
        if bytes[i] == b'$' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && is_ident_byte(bytes[j]) { j += 1; }
            if j > start {
                let name = std::str::from_utf8(&bytes[start..j]).map_err(|e| e.to_string())?;
                let val = defs.get(name).ok_or_else(|| format!("undefined variable: ${}", name))?;
                // If we're inside another `{...}` set literal and
                // the value is itself a `{...}` list, strip the outer
                // braces so `{ a, $b, c }` flattens to `{ a, v1, v2, c }`.
                let trimmed = val.trim();
                if brace_depth > 0 && trimmed.starts_with('{') && trimmed.ends_with('}') && trimmed.len() >= 2 {
                    out.push_str(trimmed[1..trimmed.len() - 1].trim());
                } else {
                    out.push_str(val);
                }
                i = j;
                continue;
            }
        }

        // Track braces for brace-aware $-expansion above. `{` opens
        // a context (we strip inner $set braces), `}` closes it.
        if bytes[i] == b'{' { brace_depth += 1; }
        if bytes[i] == b'}' { brace_depth = (brace_depth - 1).max(0); }
        // `define` / `redefine` / `undefine` directives — must start
        // a statement.
        let kw = if bytes[i..].starts_with(b"define ")
            && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b';'))
        { Some(("define", true, false)) }
        else if bytes[i..].starts_with(b"redefine ")
            && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b';'))
        { Some(("redefine", true, true)) }
        else if bytes[i..].starts_with(b"undefine ")
            && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b';'))
        { Some(("undefine", false, false)) }
        else { None };
        if let Some((kind, has_value, allow_redefine)) = kw {
            i += kind.len() + 1;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') { i += 1; }
            let name_start = i;
            while i < bytes.len() && is_ident_byte(bytes[i]) { i += 1; }
            let name = std::str::from_utf8(&bytes[name_start..i]).map_err(|e| e.to_string())?.to_string();

            if !has_value {
                // `undefine NAME` — remove the binding. Error if
                // missing, matching nft(8).
                if defs.remove(&name).is_none() {
                    return Err(format!("undefine: unknown variable '{}'", name));
                }
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') { i += 1; }
                if i < bytes.len() && (bytes[i] == b';' || bytes[i] == b'\n') { i += 1; }
                continue;
            }

            // `[re]define NAME = VALUE`.
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') { i += 1; }
            if i >= bytes.len() || bytes[i] != b'=' {
                return Err(format!("{} {}: expected '='", kind, name));
            }
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') { i += 1; }
            let val_start = i;
            let mut depth = 0i32;
            let mut in_quote = false;
            while i < bytes.len() {
                let c = bytes[i];
                if in_quote {
                    if c == b'\\' && i + 1 < bytes.len() { i += 2; continue; }
                    if c == b'"' { in_quote = false; }
                    i += 1; continue;
                }
                match c {
                    b'"' => { in_quote = true; i += 1; }
                    b'{' | b'(' | b'[' => { depth += 1; i += 1; }
                    b'}' | b')' | b']' => { depth -= 1; i += 1; }
                    b';' | b'\n' if depth == 0 => break,
                    _ => { i += 1; }
                }
            }
            let val = src[val_start..i].trim().to_string();
            // Expand with the current defs map so earlier
            // definitions (from CLI --define or previous lines) are
            // visible inside the new value.
            let expanded = expand_variables_with(defs, &val)?;

            // `define` on a name that already exists is an error.
            // `redefine` requires the name to already exist — nft(8)
            // is strict on both.
            match (defs.contains_key(&name), allow_redefine) {
                (true, false) => {
                    return Err(format!("variable '{}' already defined", name));
                }
                (false, true) => {
                    return Err(format!("redefine: unknown variable '{}'", name));
                }
                _ => {}
            }
            defs.insert(name, expanded);
            if i < bytes.len() && (bytes[i] == b';' || bytes[i] == b'\n') { i += 1; }
            continue;
        }
        // Pass char through (track string literal boundaries so `$` in
        // a quoted string isn't expanded — nft quoted strings are
        // literal).
        if bytes[i] == b'"' {
            out.push('"'); i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i] as char); out.push(bytes[i + 1] as char);
                    i += 2; continue;
                }
                out.push(bytes[i] as char);
                if bytes[i] == b'"' { i += 1; break; }
                i += 1;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    Ok(out)
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ── Helpers ─────────────────────────────────────────────────────

fn ct_state_val(s: &str) -> u32 {
    match s {
        "invalid" => 1, "established" => 2, "related" => 4,
        "new" => 8, "untracked" => 64, _ => 0,
    }
}

fn proto_num(s: &str) -> Option<u8> {
    match s {
        "tcp" => Some(6), "udp" => Some(17), "icmp" => Some(1),
        "icmpv6" => Some(58), "sctp" => Some(132), "esp" => Some(50),
        _ => None,
    }
}

/// TCP flag names → single-byte OR mask. Matches the bit layout of
/// the TCP flags field at byte offset 13: FIN(0), SYN(1), RST(2),
/// PSH(3), ACK(4), URG(5), ECN-Echo(6), CWR(7).
fn tcp_flag_byte(s: &str) -> Option<u8> {
    match s {
        "fin" => Some(0x01),
        "syn" => Some(0x02),
        "rst" => Some(0x04),
        "psh" => Some(0x08),
        "ack" => Some(0x10),
        "urg" => Some(0x20),
        "ecn" | "ece" => Some(0x40),
        "cwr" => Some(0x80),
        _ => None,
    }
}

/// Ethernet hardware address parser. Accepts the canonical
/// colon-separated lowercase hex form `aa:bb:cc:dd:ee:ff`. Returns
/// six bytes in network order. Does not accept dash-separated or
/// any of the other obscure forms `nft` accepts; we add those if
/// real rulesets need them.
fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 6 { return None; }
    let mut out = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.len() != 2 { return None; }
        out[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(out)
}

/// Ethernet type symbolic names → big-endian 2-byte EtherType. Used
/// when parsing `ether type X` rule values; without this lookup the
/// literal name bytes become the EtherType comparison and the kernel
/// matches the wrong frames.
fn ether_type_num(s: &str) -> Option<u16> {
    match s {
        "ip" | "ipv4" => Some(0x0800),
        "arp" => Some(0x0806),
        "ip6" | "ipv6" => Some(0x86dd),
        "vlan" => Some(0x8100),
        "ppp" => Some(0x880b),
        "8021q" | "qinq" => Some(0x88a8),
        "wlan" => Some(0x88c7),
        _ => None,
    }
}

/// IANA ICMPv6 type names. Numbers come from RFC 4443; the
/// neighbor-discovery extensions live in 133–137. Distinct from the
/// ICMPv4 table — `echo-request` is 128 here, 8 there.
fn icmpv6_type_num(s: &str) -> Option<u8> {
    match s {
        "destination-unreachable" => Some(1),
        "packet-too-big" => Some(2),
        "time-exceeded" => Some(3),
        "parameter-problem" => Some(4),
        "echo-request" => Some(128),
        "echo-reply" => Some(129),
        "mld-listener-query" => Some(130),
        "mld-listener-report" => Some(131),
        "mld-listener-done" => Some(132),
        "nd-router-solicit" => Some(133),
        "nd-router-advert" => Some(134),
        "nd-neighbor-solicit" => Some(135),
        "nd-neighbor-advert" => Some(136),
        "nd-redirect" => Some(137),
        "router-renumbering" => Some(138),
        "ind-neighbor-solicit" => Some(141),
        "ind-neighbor-advert" => Some(142),
        "mld2-listener-report" => Some(143),
        _ => None,
    }
}

/// IANA ICMPv4 type names accepted by upstream nft. Listed here
/// because plain decimal works through the integer-parse path; this
/// table just covers the symbolic forms (`echo-request`, etc.) so
/// `icmp type echo-request` and anon-set members like
/// `icmp type { echo-request, echo-reply }` round-trip.
fn icmp_type_num(s: &str) -> Option<u8> {
    match s {
        "echo-reply" => Some(0),
        "destination-unreachable" => Some(3),
        "source-quench" => Some(4),
        "redirect" => Some(5),
        "echo-request" => Some(8),
        "router-advertisement" => Some(9),
        "router-solicitation" => Some(10),
        "time-exceeded" => Some(11),
        "parameter-problem" => Some(12),
        "timestamp-request" => Some(13),
        "timestamp-reply" => Some(14),
        "info-request" => Some(15),
        "info-reply" => Some(16),
        "address-mask-request" => Some(17),
        "address-mask-reply" => Some(18),
        _ => None,
    }
}

fn meta_key(s: &str) -> (u32, usize) {
    match s {
        "iifname" => (NFT_META_IIFNAME, 16), "oifname" => (NFT_META_OIFNAME, 16),
        "iif" => (NFT_META_IIF, 4), "oif" => (NFT_META_OIF, 4),
        "mark" => (NFT_META_MARK, 4), "skuid" => (NFT_META_SKUID, 4),
        "skgid" => (NFT_META_SKGID, 4), "nfproto" => (NFT_META_NFPROTO, 1),
        "l4proto" => (NFT_META_L4PROTO, 1),
        // pkttype is u8 in the kernel, not u32 — using 4 here makes the
        // kernel reject the comparison as an over-wide register store.
        "pkttype" => (NFT_META_PKTTYPE, 1),
        "priority" => (NFT_META_PRIORITY, 4),
        "length" => (NFT_META_LEN, 4), "protocol" => (NFT_META_PROTOCOL, 2),
        _ => (0, 4),
    }
}

fn ct_key(s: &str) -> (u32, usize) {
    match s {
        "state" => (NFT_CT_STATE, 4), "mark" => (NFT_CT_MARK, 4),
        "status" => (NFT_CT_STATUS, 4), "direction" => (NFT_CT_DIRECTION, 1),
        "zone" => (NFT_CT_ZONE, 2), "helper" => (NFT_CT_HELPER, 16),
        "l3proto" => (NFT_CT_L3PROTOCOL, 1), "protocol" => (NFT_CT_PROTOCOL, 1),
        "bytes" => (NFT_CT_BYTES, 8), "packets" => (NFT_CT_PKTS, 8),
        "expiration" => (NFT_CT_EXPIRATION, 4), "label" => (NFT_CT_LABELS, 16),
        _ => (0, 4),
    }
}

fn is_statement_start(s: &str) -> bool {
    matches!(s, "accept" | "drop" | "reject" | "return" | "jump" | "goto" | "continue" |
        "counter" | "quota" | "log" | "limit" | "masquerade" | "snat" | "dnat" | "notrack" |
        "meta" | "ct" | "ip" | "ip6" | "tcp" | "udp" | "icmp" | "ether" |
        "iifname" | "oifname" | "comment")
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netlink::*;

    // Helper: parse exactly one command, panic otherwise
    fn parse_one(input: &str) -> Command {
        let mut cmds = parse(input).expect("parse failed");
        assert_eq!(cmds.len(), 1, "expected 1 command, got {}", cmds.len());
        cmds.remove(0)
    }

    // ── Basic table commands ─────────────────────────────────────

    #[test]
    fn test_add_table_ip() {
        let cmd = parse_one("add table ip mytable");
        assert!(matches!(cmd.op, CmdOp::Add));
        assert!(matches!(cmd.obj, CmdObj::Table));
        assert_eq!(cmd.family, NFPROTO_IPV4);
        assert_eq!(cmd.table, "mytable");
    }

    #[test]
    fn test_add_table_inet() {
        let cmd = parse_one("add table inet filter");
        assert!(matches!(cmd.op, CmdOp::Add));
        assert!(matches!(cmd.obj, CmdObj::Table));
        assert_eq!(cmd.family, NFPROTO_INET);
        assert_eq!(cmd.table, "filter");
    }

    #[test]
    fn test_delete_table() {
        let cmd = parse_one("delete table ip mytable");
        assert!(matches!(cmd.op, CmdOp::Delete));
        assert!(matches!(cmd.obj, CmdObj::Table));
        assert_eq!(cmd.family, NFPROTO_IPV4);
        assert_eq!(cmd.table, "mytable");
    }

    #[test]
    fn test_list_ruleset() {
        let cmd = parse_one("list ruleset");
        assert!(matches!(cmd.op, CmdOp::List));
        assert!(matches!(cmd.obj, CmdObj::Ruleset));
    }

    #[test]
    fn test_list_tables() {
        let cmd = parse_one("list tables");
        assert!(matches!(cmd.op, CmdOp::List));
        assert!(matches!(cmd.obj, CmdObj::Tables));
    }

    #[test]
    fn test_flush_ruleset() {
        let cmd = parse_one("flush ruleset");
        assert!(matches!(cmd.op, CmdOp::Flush));
        assert!(matches!(cmd.obj, CmdObj::Ruleset));
    }

    // ── Basic chain commands ─────────────────────────────────────

    #[test]
    fn test_add_chain() {
        let cmd = parse_one("add chain ip filter mychain");
        assert!(matches!(cmd.op, CmdOp::Add));
        assert!(matches!(cmd.obj, CmdObj::Chain));
        assert_eq!(cmd.family, NFPROTO_IPV4);
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.chain, "mychain");
    }

    #[test]
    fn test_delete_chain() {
        let cmd = parse_one("delete chain inet filter input");
        assert!(matches!(cmd.op, CmdOp::Delete));
        assert!(matches!(cmd.obj, CmdObj::Chain));
        assert_eq!(cmd.family, NFPROTO_INET);
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.chain, "input");
    }

    #[test]
    fn test_list_chain() {
        let cmd = parse_one("list chain ip filter mychain");
        assert!(matches!(cmd.op, CmdOp::List));
        assert!(matches!(cmd.obj, CmdObj::Chain));
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.chain, "mychain");
    }

    // ── Base chain spec ──────────────────────────────────────────

    #[test]
    fn test_add_chain_base_chain_spec() {
        let cmd = parse_one(
            "add chain inet filter input { type filter hook input priority filter; policy accept; }"
        );
        assert!(matches!(cmd.op, CmdOp::Add));
        assert!(matches!(cmd.obj, CmdObj::Chain));
        assert_eq!(cmd.family, NFPROTO_INET);
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.chain, "input");

        let cs = &cmd.chain_spec;
        assert!(cs.is_base, "expected is_base=true");
        assert_eq!(cs.chain_type, "filter");
        assert_eq!(cs.hook, "input");
        assert_eq!(cs.priority, 0, "filter priority on inet should be 0");
        assert!(cs.has_priority);
        assert_eq!(cs.policy, "accept");
        assert!(cs.has_policy);
    }

    #[test]
    fn test_add_chain_prerouting_dstnat() {
        let cmd = parse_one(
            "add chain ip nat prerouting { type nat hook prerouting priority dstnat; policy accept; }"
        );
        let cs = &cmd.chain_spec;
        assert!(cs.is_base);
        assert_eq!(cs.chain_type, "nat");
        assert_eq!(cs.hook, "prerouting");
        // dstnat priority on ip = -100
        assert_eq!(cs.priority, -100);
        assert!(cs.has_policy);
        assert_eq!(cs.policy, "accept");
    }

    #[test]
    fn test_add_chain_numeric_priority() {
        let cmd = parse_one(
            "add chain ip filter fwd { type filter hook forward priority 50; policy drop; }"
        );
        let cs = &cmd.chain_spec;
        assert!(cs.is_base);
        assert_eq!(cs.priority, 50);
        assert_eq!(cs.policy, "drop");
    }

    // ── Basic rule commands ──────────────────────────────────────

    #[test]
    fn test_add_rule_empty() {
        let cmd = parse_one("add rule ip filter input accept");
        assert!(matches!(cmd.op, CmdOp::Add));
        assert!(matches!(cmd.obj, CmdObj::Rule));
        assert_eq!(cmd.family, NFPROTO_IPV4);
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.chain, "input");
    }

    #[test]
    fn test_delete_rule_by_handle() {
        let cmd = parse_one("delete rule ip filter input handle 42");
        assert!(matches!(cmd.op, CmdOp::Delete));
        assert!(matches!(cmd.obj, CmdObj::Rule));
        assert_eq!(cmd.handle, 42);
        assert!(cmd.has_handle);
    }

    // ── Rule expressions: ip saddr/daddr ─────────────────────────

    #[test]
    fn test_rule_ip_saddr() {
        let cmd = parse_one("add rule ip filter input ip saddr 192.168.1.1 accept");
        let exprs = &cmd.exprs;
        // Payload load + Cmp + Verdict
        assert!(exprs.len() >= 3, "expected at least 3 exprs, got {}", exprs.len());

        match &exprs[0] {
            Expr::Payload { base, offset, len, .. } => {
                assert_eq!(*base, NFT_PAYLOAD_NETWORK_HEADER);
                assert_eq!(*offset, 12); // saddr offset
                assert_eq!(*len, 4);
            }
            other => panic!("expected Payload, got {:?}", other),
        }
        match &exprs[1] {
            Expr::Cmp { op, data } => {
                assert_eq!(*op, NFT_CMP_EQ);
                assert_eq!(data, &[192, 168, 1, 1]);
            }
            other => panic!("expected Cmp, got {:?}", other),
        }
        match &exprs[2] {
            Expr::Verdict { code, .. } => assert_eq!(*code, NF_ACCEPT),
            other => panic!("expected Verdict, got {:?}", other),
        }
    }

    #[test]
    fn test_rule_ip_daddr() {
        let cmd = parse_one("add rule ip filter output ip daddr 10.0.0.1 drop");
        let exprs = &cmd.exprs;
        match &exprs[0] {
            Expr::Payload { offset, .. } => assert_eq!(*offset, 16), // daddr offset
            other => panic!("expected Payload, got {:?}", other),
        }
        match &exprs[1] {
            Expr::Cmp { data, .. } => assert_eq!(data, &[10, 0, 0, 1]),
            other => panic!("expected Cmp, got {:?}", other),
        }
        match &exprs[2] {
            Expr::Verdict { code, .. } => assert_eq!(*code, NF_DROP),
            other => panic!("expected Verdict(drop), got {:?}", other),
        }
    }

    // ── Rule expressions: tcp dport ──────────────────────────────

    #[test]
    fn test_rule_tcp_dport() {
        let cmd = parse_one("add rule ip filter input tcp dport 80 accept");
        let exprs = &cmd.exprs;
        match &exprs[0] {
            Expr::Payload { base, offset, len, protocol } => {
                assert_eq!(*base, NFT_PAYLOAD_TRANSPORT_HEADER);
                assert_eq!(*offset, 2);
                assert_eq!(*len, 2);
                assert_eq!(*protocol, 6); // TCP
            }
            other => panic!("expected Payload, got {:?}", other),
        }
        match &exprs[1] {
            Expr::Cmp { data, .. } => assert_eq!(data, &[0, 80]),
            other => panic!("expected Cmp(80), got {:?}", other),
        }
    }

    #[test]
    fn test_rule_tcp_sport() {
        let cmd = parse_one("add rule ip filter input tcp sport 443 accept");
        match &cmd.exprs[0] {
            Expr::Payload { offset, .. } => assert_eq!(*offset, 0),
            other => panic!("expected Payload, got {:?}", other),
        }
        match &cmd.exprs[1] {
            Expr::Cmp { data, .. } => assert_eq!(data, &[1, 187]), // 443 = 0x01BB
            other => panic!("expected Cmp, got {:?}", other),
        }
    }

    // ── Rule expressions: meta iifname ───────────────────────────

    #[test]
    fn test_rule_meta_iifname() {
        // Use unquoted name so parse_value gets plain bytes without surrounding quotes
        let cmd = parse_one("add rule ip filter input meta iifname eth0 accept");
        match &cmd.exprs[0] {
            Expr::Meta { key, .. } => assert_eq!(*key, NFT_META_IIFNAME),
            other => panic!("expected Meta, got {:?}", other),
        }
        match &cmd.exprs[1] {
            Expr::Cmp { data, .. } => {
                // Interface name: "eth0" + null terminator, padded to 16 bytes
                assert_eq!(data[0..4], *b"eth0");
                assert_eq!(data[4], 0); // null terminator
            }
            other => panic!("expected Cmp, got {:?}", other),
        }
    }

    #[test]
    fn test_rule_iifname_shorthand() {
        let cmd = parse_one("add rule ip filter input iifname lo accept");
        match &cmd.exprs[0] {
            Expr::Meta { key, .. } => assert_eq!(*key, NFT_META_IIFNAME),
            other => panic!("expected Meta(iifname), got {:?}", other),
        }
    }

    #[test]
    fn test_rule_oifname_shorthand() {
        let cmd = parse_one("add rule ip filter output oifname eth0 drop");
        match &cmd.exprs[0] {
            Expr::Meta { key, .. } => assert_eq!(*key, NFT_META_OIFNAME),
            other => panic!("expected Meta(oifname), got {:?}", other),
        }
    }

    // ── Rule expressions: ct state ───────────────────────────────

    #[test]
    fn test_rule_ct_state_established() {
        let cmd = parse_one("add rule ip filter input ct state established accept");
        match &cmd.exprs[0] {
            Expr::Ct { key, dir, .. } => {
                assert_eq!(*key, NFT_CT_STATE);
                assert_eq!(*dir, -1);
            }
            other => panic!("expected Ct, got {:?}", other),
        }
        // ct state uses bitwise mask matching
        match &cmd.exprs[1] {
            Expr::Bitwise { mask, xor } => {
                let val = u32::from_ne_bytes(mask[..4].try_into().unwrap());
                assert_eq!(val, 2); // established = 2
                assert_eq!(xor, &[0, 0, 0, 0]);
            }
            other => panic!("expected Bitwise, got {:?}", other),
        }
        match &cmd.exprs[2] {
            Expr::Cmp { op, data } => {
                assert_eq!(*op, NFT_CMP_NEQ);
                assert_eq!(data, &[0, 0, 0, 0]);
            }
            other => panic!("expected Cmp(NEQ, 0), got {:?}", other),
        }
    }

    #[test]
    fn test_rule_ct_state_new_established() {
        let cmd = parse_one("add rule ip filter input ct state new,established accept");
        match &cmd.exprs[1] {
            Expr::Bitwise { mask, .. } => {
                let val = u32::from_ne_bytes(mask[..4].try_into().unwrap());
                assert_eq!(val, 8 | 2); // new=8, established=2
            }
            other => panic!("expected Bitwise, got {:?}", other),
        }
        match &cmd.exprs[2] {
            Expr::Cmp { op, data } => {
                assert_eq!(*op, NFT_CMP_NEQ);
                assert_eq!(data, &[0, 0, 0, 0]);
            }
            other => panic!("expected Cmp(NEQ, 0), got {:?}", other),
        }
    }

    // ── Rule expressions: counter ────────────────────────────────

    #[test]
    fn test_rule_counter() {
        let cmd = parse_one("add rule ip filter input counter accept");
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Counter)),
            "expected Counter expr");
    }

    // ── Rule expressions: log ────────────────────────────────────

    #[test]
    fn test_rule_log_no_prefix() {
        let cmd = parse_one("add rule ip filter input log drop");
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Log { .. })),
            "expected Log expr");
    }

    #[test]
    fn test_rule_log_with_prefix() {
        let cmd = parse_one("add rule ip filter input log prefix \"DROP: \" drop");
        let log_expr = cmd.exprs.iter().find(|e| matches!(e, Expr::Log { .. }));
        assert!(log_expr.is_some());
        match log_expr.unwrap() {
            Expr::Log { prefix } => assert_eq!(prefix, "DROP: "),
            _ => unreachable!(),
        }
    }

    // ── Rule expressions: limit ──────────────────────────────────

    #[test]
    fn test_rule_limit_rate() {
        // The parser expects rate, "/", and unit as separate tokens
        let cmd = parse_one("add rule ip filter input limit rate 100 / second accept");
        let limit_expr = cmd.exprs.iter().find(|e| matches!(e, Expr::Limit { .. }));
        assert!(limit_expr.is_some(), "expected Limit expr");
        match limit_expr.unwrap() {
            Expr::Limit { rate, unit, .. } => {
                assert_eq!(*rate, 100);
                assert_eq!(*unit, 1); // second = 1
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_rule_limit_per_minute() {
        // The parser expects rate, "/", and unit as separate tokens
        let cmd = parse_one("add rule ip filter input limit rate 10 / minute accept");
        let limit_expr = cmd.exprs.iter().find(|e| matches!(e, Expr::Limit { .. }));
        match limit_expr.unwrap() {
            Expr::Limit { rate, unit, .. } => {
                assert_eq!(*rate, 10);
                assert_eq!(*unit, 60); // minute = 60
            }
            _ => unreachable!(),
        }
    }

    // ── Rule expressions: verdicts ───────────────────────────────

    #[test]
    fn test_rule_verdict_accept() {
        let cmd = parse_one("add rule ip filter input accept");
        let v = cmd.exprs.iter().find(|e| matches!(e, Expr::Verdict { .. }));
        match v.unwrap() {
            Expr::Verdict { code, .. } => assert_eq!(*code, NF_ACCEPT),
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_rule_verdict_drop() {
        let cmd = parse_one("add rule ip filter input drop");
        let v = cmd.exprs.iter().find(|e| matches!(e, Expr::Verdict { .. }));
        match v.unwrap() {
            Expr::Verdict { code, .. } => assert_eq!(*code, NF_DROP),
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_rule_verdict_jump() {
        let cmd = parse_one("add rule ip filter input jump log-and-drop");
        let v = cmd.exprs.iter().find(|e| matches!(e, Expr::Verdict { .. }));
        match v.unwrap() {
            Expr::Verdict { code, chain } => {
                assert_eq!(*code, NFT_JUMP);
                assert_eq!(chain, "log-and-drop");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_rule_verdict_goto() {
        let cmd = parse_one("add rule ip filter input goto mychaintarget");
        let v = cmd.exprs.iter().find(|e| matches!(e, Expr::Verdict { .. }));
        match v.unwrap() {
            Expr::Verdict { code, chain } => {
                assert_eq!(*code, NFT_GOTO);
                assert_eq!(chain, "mychaintarget");
            }
            _ => unreachable!(),
        }
    }

    // ── Rule expressions: masquerade ─────────────────────────────

    #[test]
    fn test_rule_masquerade() {
        let cmd = parse_one("add rule ip nat postrouting masquerade");
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Masquerade)),
            "expected Masquerade expr");
    }

    // ── Rule expressions: snat/dnat ──────────────────────────────

    #[test]
    fn test_rule_snat() {
        let cmd = parse_one("add rule ip nat postrouting snat to 203.0.113.5");
        let nat = cmd.exprs.iter().find(|e| matches!(e, Expr::Nat { .. }));
        assert!(nat.is_some(), "expected Nat expr");
        match nat.unwrap() {
            Expr::Nat { nat_type, addr, has_addr, has_port, .. } => {
                assert_eq!(*nat_type, NFT_NAT_SNAT);
                assert!(has_addr);
                assert!(!has_port);
                assert_eq!(addr, &[203, 0, 113, 5]);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_rule_dnat() {
        let cmd = parse_one("add rule ip nat prerouting dnat to 10.0.0.10");
        let nat = cmd.exprs.iter().find(|e| matches!(e, Expr::Nat { .. }));
        match nat.unwrap() {
            Expr::Nat { nat_type, addr, has_addr, .. } => {
                assert_eq!(*nat_type, NFT_NAT_DNAT);
                assert!(has_addr);
                assert_eq!(addr, &[10, 0, 0, 10]);
            }
            _ => unreachable!(),
        }
    }

    // ── Rule expressions: notrack/reject ─────────────────────────

    #[test]
    fn test_rule_notrack() {
        let cmd = parse_one("add rule ip raw prerouting notrack");
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Notrack)),
            "expected Notrack expr");
    }

    #[test]
    fn test_rule_reject() {
        let cmd = parse_one("add rule ip filter input reject");
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Reject)),
            "expected Reject expr");
    }

    // ── CIDR matching ────────────────────────────────────────────

    #[test]
    fn test_rule_cidr_8() {
        let cmd = parse_one("add rule ip filter input ip saddr 10.0.0.0/8 accept");
        // Expected: Payload, Bitwise, Cmp, Verdict
        assert!(cmd.exprs.len() >= 3, "expected at least 3 exprs, got {}", cmd.exprs.len());

        match &cmd.exprs[0] {
            Expr::Payload { base, offset, len, .. } => {
                assert_eq!(*base, NFT_PAYLOAD_NETWORK_HEADER);
                assert_eq!(*offset, 12);
                assert_eq!(*len, 4);
            }
            other => panic!("expected Payload, got {:?}", other),
        }
        match &cmd.exprs[1] {
            Expr::Bitwise { mask, xor } => {
                // /8 mask: 0xff 0x00 0x00 0x00
                assert_eq!(mask, &[0xff, 0x00, 0x00, 0x00]);
                assert_eq!(xor, &[0x00, 0x00, 0x00, 0x00]);
            }
            other => panic!("expected Bitwise, got {:?}", other),
        }
        match &cmd.exprs[2] {
            Expr::Cmp { data, .. } => {
                // 10.0.0.0 & /8 mask = [10, 0, 0, 0]
                assert_eq!(data, &[10, 0, 0, 0]);
            }
            other => panic!("expected Cmp, got {:?}", other),
        }
    }

    #[test]
    fn test_rule_cidr_24() {
        let cmd = parse_one("add rule ip filter input ip saddr 192.168.1.0/24 drop");
        match &cmd.exprs[1] {
            Expr::Bitwise { mask, .. } => {
                // /24 mask: 0xff 0xff 0xff 0x00
                assert_eq!(mask, &[0xff, 0xff, 0xff, 0x00]);
            }
            other => panic!("expected Bitwise, got {:?}", other),
        }
        match &cmd.exprs[2] {
            Expr::Cmp { data, .. } => {
                assert_eq!(data, &[192, 168, 1, 0]);
            }
            other => panic!("expected Cmp, got {:?}", other),
        }
    }

    #[test]
    fn test_rule_cidr_32() {
        let cmd = parse_one("add rule ip filter input ip saddr 1.2.3.4/32 accept");
        match &cmd.exprs[1] {
            Expr::Bitwise { mask, .. } => {
                assert_eq!(mask, &[0xff, 0xff, 0xff, 0xff]);
            }
            other => panic!("expected Bitwise, got {:?}", other),
        }
        match &cmd.exprs[2] {
            Expr::Cmp { data, .. } => {
                assert_eq!(data, &[1, 2, 3, 4]);
            }
            other => panic!("expected Cmp, got {:?}", other),
        }
    }

    // ── Rename chain ─────────────────────────────────────────────

    #[test]
    fn test_rename_chain() {
        let cmd = parse_one("rename chain inet filter old_name new_name");
        assert!(matches!(cmd.op, CmdOp::Rename));
        assert!(matches!(cmd.obj, CmdObj::Chain));
        assert_eq!(cmd.family, NFPROTO_INET);
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.chain, "old_name");
        assert_eq!(cmd.new_name, "new_name");
    }

    #[test]
    fn test_rename_chain_ip() {
        let cmd = parse_one("rename chain ip mytable src dst");
        assert!(matches!(cmd.op, CmdOp::Rename));
        assert_eq!(cmd.family, NFPROTO_IPV4);
        assert_eq!(cmd.table, "mytable");
        assert_eq!(cmd.chain, "src");
        assert_eq!(cmd.new_name, "dst");
    }

    // ── add set ──────────────────────────────────────────────────

    #[test]
    fn test_add_set_ipv4_constant() {
        let cmd = parse_one(
            "add set inet filter myset { type ipv4_addr; flags constant; }"
        );
        assert!(matches!(cmd.op, CmdOp::Add));
        assert!(matches!(cmd.obj, CmdObj::Set));
        assert_eq!(cmd.family, NFPROTO_INET);
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.set_name, "myset");

        let ss = &cmd.set_spec;
        assert_eq!(ss.key_type_name, "ipv4_addr");
        assert_eq!(ss.key_type, 7);  // ipv4_addr type id
        assert_eq!(ss.key_len, 4);   // 4 bytes
        assert!(ss.has_flags);
        assert_eq!(ss.flags & 0x2, 0x2, "expected constant flag (0x2)");
    }

    #[test]
    fn test_add_set_ipv4_interval() {
        let cmd = parse_one(
            "add set ip filter blocked { type ipv4_addr; flags interval; }"
        );
        let ss = &cmd.set_spec;
        assert_eq!(ss.key_type, 7);
        assert_eq!(ss.key_len, 4);
        assert!(ss.has_flags);
        assert_eq!(ss.flags & 0x4, 0x4, "expected interval flag (0x4)");
    }

    #[test]
    fn test_add_set_inet_service() {
        let cmd = parse_one(
            "add set ip filter ports { type inet_service; }"
        );
        let ss = &cmd.set_spec;
        assert_eq!(ss.key_type_name, "inet_service");
        assert_eq!(ss.key_type, 13);
        assert_eq!(ss.key_len, 2);
    }

    // ── delete set ───────────────────────────────────────────────

    #[test]
    fn test_delete_set() {
        let cmd = parse_one("delete set inet filter myset");
        assert!(matches!(cmd.op, CmdOp::Delete));
        assert!(matches!(cmd.obj, CmdObj::Set));
        assert_eq!(cmd.family, NFPROTO_INET);
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.set_name, "myset");
    }

    // ── add element ──────────────────────────────────────────────

    #[test]
    fn test_add_element_ipv4() {
        let cmd = parse_one(
            "add element inet filter myset { 10.0.0.1, 10.0.0.2 }"
        );
        assert!(matches!(cmd.op, CmdOp::Add));
        assert!(matches!(cmd.obj, CmdObj::Element));
        assert_eq!(cmd.family, NFPROTO_INET);
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.set_name, "myset");

        assert_eq!(cmd.elements.len(), 2);
        assert_eq!(cmd.elements[0], (vec![10, 0, 0, 1], None));
        assert_eq!(cmd.elements[1], (vec![10, 0, 0, 2], None));
    }

    #[test]
    fn test_add_element_single() {
        let cmd = parse_one("add element ip filter myset { 192.168.0.1 }");
        assert_eq!(cmd.elements.len(), 1);
        assert_eq!(cmd.elements[0], (vec![192, 168, 0, 1], None));
    }

    #[test]
    fn test_add_element_three_addrs() {
        let cmd = parse_one(
            "add element ip filter myset { 1.1.1.1, 8.8.8.8, 9.9.9.9 }"
        );
        assert_eq!(cmd.elements.len(), 3);
        assert_eq!(cmd.elements[0], (vec![1, 1, 1, 1], None));
        assert_eq!(cmd.elements[1], (vec![8, 8, 8, 8], None));
        assert_eq!(cmd.elements[2], (vec![9, 9, 9, 9], None));
    }

    // ── delete element ───────────────────────────────────────────

    #[test]
    fn test_delete_element() {
        let cmd = parse_one(
            "delete element inet filter myset { 10.0.0.1 }"
        );
        assert!(matches!(cmd.op, CmdOp::Delete));
        assert!(matches!(cmd.obj, CmdObj::Element));
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.set_name, "myset");
        assert_eq!(cmd.elements.len(), 1);
        assert_eq!(cmd.elements[0], (vec![10, 0, 0, 1], None));
    }

    // ── Multiple commands separated by semicolons ─────────────────

    #[test]
    fn test_multiple_commands_semicolons() {
        let cmds = parse(
            "add table inet filter; add chain inet filter input; add chain inet filter output"
        ).expect("parse failed");
        assert_eq!(cmds.len(), 3);

        assert!(matches!(cmds[0].op, CmdOp::Add));
        assert!(matches!(cmds[0].obj, CmdObj::Table));
        assert_eq!(cmds[0].table, "filter");

        assert!(matches!(cmds[1].op, CmdOp::Add));
        assert!(matches!(cmds[1].obj, CmdObj::Chain));
        assert_eq!(cmds[1].chain, "input");

        assert!(matches!(cmds[2].op, CmdOp::Add));
        assert!(matches!(cmds[2].obj, CmdObj::Chain));
        assert_eq!(cmds[2].chain, "output");
    }

    #[test]
    fn test_multiple_commands_with_trailing_semicolon() {
        let cmds = parse(
            "add table ip filter; delete table ip filter;"
        ).expect("parse failed");
        assert_eq!(cmds.len(), 2);
    }

    // ── File input with newlines ──────────────────────────────────

    #[test]
    fn test_file_input_newlines() {
        let input = "add table inet filter\nadd chain inet filter input\nadd rule ip filter input accept\n";
        let cmds = parse(input).expect("parse failed");
        assert_eq!(cmds.len(), 3);
        assert!(matches!(cmds[0].obj, CmdObj::Table));
        assert!(matches!(cmds[1].obj, CmdObj::Chain));
        assert!(matches!(cmds[2].obj, CmdObj::Rule));
    }

    #[test]
    fn test_file_input_comments_and_newlines() {
        let input = "# This is a comment\nadd table inet filter\n# Another comment\nadd chain inet filter input\n";
        let cmds = parse(input).expect("parse failed");
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0].table, "filter");
        assert_eq!(cmds[1].chain, "input");
    }

    #[test]
    fn test_file_input_mixed_semicolons_and_newlines() {
        let input = "add table ip filter\nadd chain ip filter input; add chain ip filter output\nadd rule ip filter input accept\n";
        let cmds = parse(input).expect("parse failed");
        assert_eq!(cmds.len(), 4);
    }

    // ── Compound rule expressions ─────────────────────────────────

    #[test]
    fn test_rule_counter_and_accept() {
        let cmd = parse_one("add rule inet filter input counter accept");
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Counter)));
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Verdict { code, .. } if *code == NF_ACCEPT)));
    }

    #[test]
    fn test_rule_tcp_dport_counter_accept() {
        let cmd = parse_one("add rule inet filter input tcp dport 443 counter accept");
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Payload { .. })));
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Counter)));
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Verdict { .. })));
    }

    #[test]
    fn test_rule_ct_state_established_related_accept() {
        let cmd = parse_one("add rule inet filter input ct state established,related accept");
        match &cmd.exprs[1] {
            Expr::Bitwise { mask, .. } => {
                let val = u32::from_ne_bytes(mask[..4].try_into().unwrap());
                assert_eq!(val, 2 | 4); // established=2, related=4
            }
            other => panic!("expected Bitwise, got {:?}", other),
        }
        match &cmd.exprs[2] {
            Expr::Cmp { op, data } => {
                assert_eq!(*op, NFT_CMP_NEQ);
                assert_eq!(data, &[0, 0, 0, 0]);
            }
            other => panic!("expected Cmp(NEQ, 0), got {:?}", other),
        }
    }

    // ── Neq comparison ────────────────────────────────────────────

    #[test]
    fn test_rule_neq_comparison() {
        let cmd = parse_one("add rule ip filter input ip saddr != 10.0.0.1 accept");
        match &cmd.exprs[1] {
            Expr::Cmp { op, data } => {
                assert_eq!(*op, NFT_CMP_NEQ);
                assert_eq!(data, &[10, 0, 0, 1]);
            }
            other => panic!("expected Cmp(neq), got {:?}", other),
        }
    }

    // ── Family defaults ───────────────────────────────────────────

    #[test]
    fn test_family_default_ip_for_table() {
        // Without explicit family on table, defaults to NFPROTO_IPV4
        let cmd = parse_one("add table filter");
        assert_eq!(cmd.family, NFPROTO_IPV4);
        assert_eq!(cmd.table, "filter");
    }

    #[test]
    fn test_family_ipv6() {
        let cmd = parse_one("add table ip6 mytable");
        assert_eq!(cmd.family, NFPROTO_IPV6);
    }

    #[test]
    fn test_family_bridge() {
        let cmd = parse_one("add table bridge mytable");
        assert_eq!(cmd.family, NFPROTO_BRIDGE);
    }

    #[test]
    fn test_family_netdev() {
        let cmd = parse_one("add table netdev mytable");
        assert_eq!(cmd.family, NFPROTO_NETDEV);
    }

    // ── Priority from named strings ───────────────────────────────

    #[test]
    fn test_priority_raw() {
        let cmd = parse_one(
            "add chain ip filter prerouting { type filter hook prerouting priority raw; policy accept; }"
        );
        assert_eq!(cmd.chain_spec.priority, -300);
    }

    #[test]
    fn test_priority_mangle() {
        let cmd = parse_one(
            "add chain ip filter prerouting { type filter hook prerouting priority mangle; policy accept; }"
        );
        assert_eq!(cmd.chain_spec.priority, -150);
    }

    #[test]
    fn test_priority_srcnat() {
        let cmd = parse_one(
            "add chain ip nat postrouting { type nat hook postrouting priority srcnat; policy accept; }"
        );
        assert_eq!(cmd.chain_spec.priority, 100);
    }

    // ── Flush table/chain ─────────────────────────────────────────

    #[test]
    fn test_flush_table() {
        let cmd = parse_one("flush table ip filter");
        assert!(matches!(cmd.op, CmdOp::Flush));
        assert!(matches!(cmd.obj, CmdObj::Table));
        assert_eq!(cmd.table, "filter");
    }

    #[test]
    fn test_flush_chain() {
        let cmd = parse_one("flush chain ip filter input");
        assert!(matches!(cmd.op, CmdOp::Flush));
        assert!(matches!(cmd.obj, CmdObj::Chain));
        assert_eq!(cmd.table, "filter");
        assert_eq!(cmd.chain, "input");
    }

    // ── Insert rule ───────────────────────────────────────────────

    #[test]
    fn test_insert_rule() {
        let cmd = parse_one("insert rule ip filter input drop");
        assert!(matches!(cmd.op, CmdOp::Insert));
        assert!(matches!(cmd.obj, CmdObj::Rule));
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Verdict { code, .. } if *code == NF_DROP)));
    }

    // ── Destroy commands ──────────────────────────────────────────

    #[test]
    fn test_destroy_table() {
        let cmd = parse_one("destroy table ip mytable");
        assert!(matches!(cmd.op, CmdOp::Destroy));
        assert!(matches!(cmd.obj, CmdObj::Table));
        assert_eq!(cmd.table, "mytable");
    }

    // ── Create command ────────────────────────────────────────────

    #[test]
    fn test_create_table() {
        let cmd = parse_one("create table inet filter");
        assert!(matches!(cmd.op, CmdOp::Create));
        assert!(matches!(cmd.obj, CmdObj::Table));
        assert_eq!(cmd.family, NFPROTO_INET);
        assert_eq!(cmd.table, "filter");
    }

    #[test]
    fn test_create_chain_base() {
        let cmd = parse_one(
            "create chain ip nat postrouting { type nat hook postrouting priority 100; policy accept; }"
        );
        assert!(matches!(cmd.op, CmdOp::Create));
        assert!(matches!(cmd.obj, CmdObj::Chain));
        assert!(cmd.chain_spec.is_base);
        assert_eq!(cmd.chain_spec.priority, 100);
    }

    // ── Rule with handle ──────────────────────────────────────────

    #[test]
    fn test_rule_with_position_handle() {
        let cmd = parse_one("add rule ip filter input handle 7 accept");
        assert_eq!(cmd.handle, 7);
        assert!(cmd.has_handle);
        assert!(cmd.exprs.iter().any(|e| matches!(e, Expr::Verdict { code, .. } if *code == NF_ACCEPT)));
    }

    // ── Quoted names ──────────────────────────────────────────────

    #[test]
    fn test_quoted_chain_name() {
        let cmd = parse_one("add chain ip filter \"my chain\"");
        assert_eq!(cmd.chain, "my chain");
    }

    #[test]
    fn test_quoted_table_name() {
        let cmd = parse_one("add table ip \"my table\"");
        assert_eq!(cmd.table, "my table");
    }

    // ── List chains/rules/sets ────────────────────────────────────

    #[test]
    fn test_list_chains() {
        let cmd = parse_one("list chains");
        assert!(matches!(cmd.op, CmdOp::List));
        assert!(matches!(cmd.obj, CmdObj::Chains));
    }

    #[test]
    fn test_list_rules() {
        let cmd = parse_one("list rules");
        assert!(matches!(cmd.op, CmdOp::List));
        assert!(matches!(cmd.obj, CmdObj::Rules));
    }

    #[test]
    fn test_list_sets() {
        let cmd = parse_one("list sets");
        assert!(matches!(cmd.op, CmdOp::List));
        assert!(matches!(cmd.obj, CmdObj::Sets));
    }

    // ── Map object ────────────────────────────────────────────────

    #[test]
    fn test_add_map() {
        let cmd = parse_one(
            "add map ip filter mymap { type ipv4_addr : inet_service; }"
        );
        assert!(matches!(cmd.op, CmdOp::Add));
        assert!(matches!(cmd.obj, CmdObj::Map));
        assert_eq!(cmd.set_name, "mymap");
        assert!(cmd.set_spec.is_map);
        assert_eq!(cmd.set_spec.key_type, 7);   // ipv4_addr
        assert_eq!(cmd.set_spec.key_len, 4);
        assert_eq!(cmd.set_spec.data_type, 13);  // inet_service
        assert_eq!(cmd.set_spec.data_len, 2);
    }
}
