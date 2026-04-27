//! Raw netlink socket and nf_tables message construction.
//! Minimal unsafe only for libc socket syscalls.

use std::io;
use std::mem;

// ── Netlink / nfnetlink constants ───────────────────────────────
pub const NETLINK_NETFILTER: i32 = 12;
pub const NFNL_SUBSYS_NFTABLES: u16 = 10;
pub const NFNL_MSG_BATCH_BEGIN: u16 = 0x10;
pub const NFNL_MSG_BATCH_END: u16 = 0x11;

// NLM flags
pub const NLM_F_REQUEST: u16 = 1;
pub const NLM_F_ACK: u16 = 4;
pub const NLM_F_EXCL: u16 = 0x200;
pub const NLM_F_DUMP: u16 = 0x300;
pub const NLM_F_ECHO: u16 = 0x8;
pub const NLM_F_CREATE: u16 = 0x400;
pub const NLM_F_APPEND: u16 = 0x800;
pub const NLM_F_REPLACE: u16 = 0x100;

// NLMSG types
pub const NLMSG_ERROR: u16 = 2;
pub const NLMSG_DONE: u16 = 3;

// NFPROTO
pub const NFPROTO_UNSPEC: u8 = 0;
pub const NFPROTO_INET: u8 = 1;
pub const NFPROTO_IPV4: u8 = 2;
pub const NFPROTO_ARP: u8 = 3;
pub const NFPROTO_NETDEV: u8 = 5;
pub const NFPROTO_BRIDGE: u8 = 7;
pub const NFPROTO_IPV6: u8 = 10;

// NFT message types
pub const NFT_MSG_NEWTABLE: u16 = 0;
pub const NFT_MSG_GETTABLE: u16 = 1;
pub const NFT_MSG_DELTABLE: u16 = 2;
pub const NFT_MSG_NEWCHAIN: u16 = 3;
pub const NFT_MSG_GETCHAIN: u16 = 4;
pub const NFT_MSG_DELCHAIN: u16 = 5;
pub const NFT_MSG_NEWRULE: u16 = 6;
pub const NFT_MSG_GETRULE: u16 = 7;
pub const NFT_MSG_DELRULE: u16 = 8;
pub const NFT_MSG_NEWSET: u16 = 9;
pub const NFT_MSG_GETSET: u16 = 10;
pub const NFT_MSG_DELSET: u16 = 11;
pub const NFT_MSG_NEWSETELEM: u16 = 12;
pub const NFT_MSG_GETSETELEM: u16 = 13;
pub const NFT_MSG_DELSETELEM: u16 = 14;
pub const NFT_MSG_DESTROYTABLE: u16 = 26;
pub const NFT_MSG_DESTROYCHAIN: u16 = 27;
pub const NFT_MSG_DESTROYRULE: u16 = 28;
pub const NFT_MSG_DESTROYSET: u16 = 29;
pub const NFT_MSG_DESTROYSETELEM: u16 = 30;
pub const NFT_MSG_DESTROYOBJ: u16 = 31;

// Named stateful objects (counter / quota / limit / ...).
// UAPI: include/uapi/linux/netfilter/nf_tables.h
pub const NFT_MSG_NEWOBJ: u16 = 18;
pub const NFT_MSG_GETOBJ: u16 = 19;
pub const NFT_MSG_DELOBJ: u16 = 20;

pub const NFTA_OBJ_TABLE: u16 = 1;
pub const NFTA_OBJ_NAME: u16 = 2;
pub const NFTA_OBJ_TYPE: u16 = 3;
pub const NFTA_OBJ_DATA: u16 = 4;
pub const NFTA_OBJ_USE: u16 = 5;
pub const NFTA_OBJ_HANDLE: u16 = 6;

// nft_object_type values
pub const NFT_OBJECT_COUNTER: u32 = 1;
pub const NFT_OBJECT_QUOTA: u32 = 2;
pub const NFT_OBJECT_LIMIT: u32 = 4;

// Counter object attributes
// (NFTA_COUNTER_BYTES=1, NFTA_COUNTER_PACKETS=2 already exist in the
// expression-level counter path and are reused for the object payload.)

// Quota object attributes (all under NFTA_OBJ_DATA for NFT_OBJECT_QUOTA)
pub const NFTA_QUOTA_BYTES: u16 = 1;
pub const NFTA_QUOTA_FLAGS: u16 = 2;
pub const NFTA_QUOTA_CONSUMED: u16 = 3;
pub const NFT_QUOTA_F_INV: u32 = 1;

// objref expression: references a named stateful object from a rule.
// Emitted as `counter name "cnt"` or similar in text form.
pub const NFTA_OBJREF_IMM_TYPE: u16 = 1;
pub const NFTA_OBJREF_IMM_NAME: u16 = 2;

// Table attributes
pub const NFTA_TABLE_NAME: u16 = 1;
pub const NFTA_TABLE_FLAGS: u16 = 2;
pub const NFTA_TABLE_HANDLE: u16 = 4;

// Chain attributes
pub const NFTA_CHAIN_TABLE: u16 = 1;
pub const NFTA_CHAIN_HANDLE: u16 = 2;
pub const NFTA_CHAIN_NAME: u16 = 3;
pub const NFTA_CHAIN_HOOK: u16 = 4;
pub const NFTA_CHAIN_POLICY: u16 = 5;
pub const NFTA_CHAIN_TYPE: u16 = 7;

// Hook attributes
pub const NFTA_HOOK_HOOKNUM: u16 = 1;
pub const NFTA_HOOK_PRIORITY: u16 = 2;
pub const NFTA_HOOK_DEV: u16 = 3;

// Rule attributes
pub const NFTA_RULE_TABLE: u16 = 1;
pub const NFTA_RULE_CHAIN: u16 = 2;
pub const NFTA_RULE_HANDLE: u16 = 3;
pub const NFTA_RULE_EXPRESSIONS: u16 = 4;
pub const NFTA_RULE_POSITION: u16 = 6;
pub const NFTA_RULE_USERDATA: u16 = 7;

// Expression attributes
pub const NFTA_EXPR_NAME: u16 = 1;
pub const NFTA_EXPR_DATA: u16 = 2;
pub const NFTA_LIST_ELEM: u16 = 1;
pub const NLA_F_NESTED: u16 = 1 << 15;

// Registers
pub const NFT_REG_VERDICT: u32 = 0;
pub const NFT_REG_1: u32 = 1; // 16-byte register (aliases NFT_REG32_00..03)
pub const NFT_REG32_00: u32 = 8;
pub const NFT_REG32_01: u32 = 9;

// Pick the right register for a given value width. 4-byte (and shorter)
// values fit in the 32-bit register NFT_REG32_00; anything wider needs
// NFT_REG_1, the 16-byte register. Using NFT_REG32_00 for a 16-byte
// value (e.g. iifname/oifname, ct helper) makes the kernel reject the
// expression with EOVERFLOW during validation.
pub fn reg_for_width(width: usize) -> u32 {
    if width > 4 { NFT_REG_1 } else { NFT_REG32_00 }
}

// Verdicts
pub const NF_DROP: i32 = 0;
pub const NF_ACCEPT: i32 = 1;
pub const NFT_JUMP: i32 = -3;
pub const NFT_GOTO: i32 = -4;
pub const NFT_RETURN: i32 = -5;
pub const NFT_CONTINUE: i32 = -1;

// Hooks
pub const NF_INET_PRE_ROUTING: u32 = 0;
pub const NF_INET_LOCAL_IN: u32 = 1;
pub const NF_INET_FORWARD: u32 = 2;
pub const NF_INET_LOCAL_OUT: u32 = 3;
pub const NF_INET_POST_ROUTING: u32 = 4;

// Payload expression
pub const NFTA_PAYLOAD_DREG: u16 = 1;
pub const NFTA_PAYLOAD_BASE: u16 = 2;
pub const NFTA_PAYLOAD_OFFSET: u16 = 3;
pub const NFTA_PAYLOAD_LEN: u16 = 4;
pub const NFT_PAYLOAD_LL_HEADER: u32 = 0;
pub const NFT_PAYLOAD_NETWORK_HEADER: u32 = 1;
pub const NFT_PAYLOAD_TRANSPORT_HEADER: u32 = 2;

// CMP expression
pub const NFTA_CMP_SREG: u16 = 1;
pub const NFTA_CMP_OP: u16 = 2;
pub const NFTA_CMP_DATA: u16 = 3;
pub const NFT_CMP_EQ: u32 = 0;
pub const NFT_CMP_NEQ: u32 = 1;
pub const NFT_CMP_LT: u32 = 2;
pub const NFT_CMP_LTE: u32 = 3;
pub const NFT_CMP_GT: u32 = 4;
pub const NFT_CMP_GTE: u32 = 5;

// Meta expression
// Kernel UAPI: NFTA_META_UNSPEC=0, NFTA_META_DREG=1, NFTA_META_KEY=2.
// Prior versions of stormwall had these two swapped; for simple 4-byte
// meta matches the bug was papered over by the equally-swapped output
// decoder, which is why tailscale's live ruleset showed every meta
// expression as "meta unknown" and iifname encodes were rejected by
// the kernel with EOVERFLOW.
pub const NFTA_META_DREG: u16 = 1;
pub const NFTA_META_KEY: u16 = 2;
pub const NFTA_META_SREG: u16 = 3;
// Order matches enum nft_meta_keys in include/uapi/linux/netfilter/nf_tables.h.
// NFT_META_MARK was 15 (collides with NFPROTO), NFT_META_NFPROTO was 17
// (collides with BRI_IIFNAME), NFT_META_PKTTYPE was 26 (collides with
// IIFKIND), and IIF/OIF were 3/4 (the kernel slots for MARK and IIF
// respectively). nft monitor decoded our `meta mark` rules as
// `meta nfproto` because of the slot collision.
pub const NFT_META_LEN: u32 = 0;
pub const NFT_META_PROTOCOL: u32 = 1;
pub const NFT_META_PRIORITY: u32 = 2;
pub const NFT_META_MARK: u32 = 3;
pub const NFT_META_IIF: u32 = 4;
pub const NFT_META_OIF: u32 = 5;
pub const NFT_META_IIFNAME: u32 = 6;
pub const NFT_META_OIFNAME: u32 = 7;
pub const NFT_META_IIFTYPE: u32 = 8;
pub const NFT_META_OIFTYPE: u32 = 9;
pub const NFT_META_SKUID: u32 = 10;
pub const NFT_META_SKGID: u32 = 11;
pub const NFT_META_NFPROTO: u32 = 15;
pub const NFT_META_L4PROTO: u32 = 16;
pub const NFT_META_PKTTYPE: u32 = 19;

// CT expression
pub const NFTA_CT_DREG: u16 = 1;
pub const NFTA_CT_KEY: u16 = 2;
pub const NFTA_CT_DIR: u16 = 3;
// Order matches enum nft_ct_keys in nf_tables.h. The earlier table
// had EXPIRATION/HELPER/L3PROTOCOL/PROTOCOL/PKTS/BYTES/LABELS off by
// the count of intervening kernel enum entries (SECMARK at 4, SRC/DST
// at 8/9, etc.) — so e.g. `ct labels` was being encoded as ZONE+5 and
// `ct packets` as DST_IP. The chain-rule tests didn't exercise these
// keys, so the bug went unnoticed until monitor-diff caught it.
pub const NFT_CT_STATE: u32 = 0;
pub const NFT_CT_DIRECTION: u32 = 1;
pub const NFT_CT_STATUS: u32 = 2;
pub const NFT_CT_MARK: u32 = 3;
pub const NFT_CT_SECMARK: u32 = 4;
pub const NFT_CT_EXPIRATION: u32 = 5;
pub const NFT_CT_HELPER: u32 = 6;
pub const NFT_CT_L3PROTOCOL: u32 = 7;
pub const NFT_CT_PROTOCOL: u32 = 10;
pub const NFT_CT_LABELS: u32 = 13;
pub const NFT_CT_PKTS: u32 = 14;
pub const NFT_CT_BYTES: u32 = 15;
pub const NFT_CT_AVGPKT: u32 = 16;
pub const NFT_CT_ZONE: u32 = 17;

// Immediate expression
pub const NFTA_IMM_DREG: u16 = 1;
pub const NFTA_IMM_DATA: u16 = 2;
pub const NFTA_DATA_VALUE: u16 = 1;
pub const NFTA_DATA_VERDICT: u16 = 2;
pub const NFTA_VERDICT_CODE: u16 = 1;
pub const NFTA_VERDICT_CHAIN: u16 = 2;

// Counter
pub const NFTA_COUNTER_BYTES: u16 = 1;
pub const NFTA_COUNTER_PACKETS: u16 = 2;

// Bitwise
pub const NFTA_BITWISE_SREG: u16 = 1;
pub const NFTA_BITWISE_DREG: u16 = 2;
pub const NFTA_BITWISE_LEN: u16 = 3;
pub const NFTA_BITWISE_MASK: u16 = 4;
pub const NFTA_BITWISE_XOR: u16 = 5;

// Log
// Order matches enum nft_log_attributes: GROUP=1, PREFIX=2.
// Stormwall had these reversed, so a `log prefix "hello: "` rule
// was being sent with the prefix bytes under the GROUP attribute —
// the kernel then read the first two prefix bytes as a u16 group
// number ("he" → 0x6865 → 26725) and discarded the prefix entirely.
pub const NFTA_LOG_GROUP: u16 = 1;
pub const NFTA_LOG_PREFIX: u16 = 2;

// Limit
pub const NFTA_LIMIT_RATE: u16 = 1;
pub const NFTA_LIMIT_UNIT: u16 = 2;
pub const NFTA_LIMIT_BURST: u16 = 3;
pub const NFTA_LIMIT_TYPE: u16 = 4;
pub const NFTA_LIMIT_FLAGS: u16 = 5;

// NAT
pub const NFTA_NAT_TYPE: u16 = 1;
pub const NFTA_NAT_FAMILY: u16 = 2;
pub const NFTA_NAT_REG_ADDR_MIN: u16 = 3;
pub const NFTA_NAT_REG_ADDR_MAX: u16 = 4;
pub const NFTA_NAT_REG_PROTO_MIN: u16 = 5;
pub const NFTA_NAT_REG_PROTO_MAX: u16 = 6;
pub const NFT_NAT_SNAT: u32 = 0;
pub const NFT_NAT_DNAT: u32 = 1;

// Set attributes
pub const NFTA_SET_TABLE: u16 = 1;
pub const NFTA_SET_NAME: u16 = 2;
pub const NFTA_SET_FLAGS: u16 = 3;
pub const NFTA_SET_KEY_TYPE: u16 = 4;
pub const NFTA_SET_KEY_LEN: u16 = 5;
pub const NFTA_SET_FAMILY: u16 = 8;
pub const NFTA_SET_DATA_TYPE: u16 = 6;
pub const NFTA_SET_DATA_LEN: u16 = 7;
pub const NFTA_SET_DESC: u16 = 9;
pub const NFTA_SET_ID: u16 = 10;
pub const NFTA_SET_HANDLE: u16 = 16;

// Set desc attributes
pub const NFTA_SET_DESC_SIZE: u16 = 1;

// Set element list attributes
pub const NFTA_SET_ELEM_LIST_TABLE: u16 = 1;
pub const NFTA_SET_ELEM_LIST_SET: u16 = 2;
pub const NFTA_SET_ELEM_LIST_ELEMENTS: u16 = 3;

// Set element attributes
pub const NFTA_SET_ELEM_KEY: u16 = 1;
pub const NFTA_SET_ELEM_DATA: u16 = 2;
pub const NFTA_SET_ELEM_FLAGS: u16 = 3;   // per kernel uapi
pub const NFT_SET_ELEM_INTERVAL_END: u32 = 0x1;

// Lookup expression (used for `ip saddr @setname drop`)
pub const NFTA_LOOKUP_SET: u16 = 1;   // string: set name
pub const NFTA_LOOKUP_SREG: u16 = 2;  // u32: source register
pub const NFTA_LOOKUP_DREG: u16 = 3;  // u32: dest register (map lookup)
// In kernel 6.1 UAPI the order is SET_ID=4, FLAGS=5 (not the other
// way around, which is what earlier versions of stormwall assumed).
// Sending FLAGS=4 lands in the SET_ID slot, the kernel silently
// ignores it, and `!= @set` never round-trips — the exact bug we've
// been chasing for days. UAPI wins.
pub const NFTA_LOOKUP_SET_ID: u16 = 4;
pub const NFTA_LOOKUP_FLAGS: u16 = 5;   // u32: NFT_LOOKUP_F_INV for `!=`
pub const NFT_LOOKUP_F_INV: u32 = 0x1;

// Set flags
pub const NFT_SET_ANONYMOUS: u32 = 0x1;
pub const NFT_SET_CONSTANT: u32 = 0x2;
pub const NFT_SET_INTERVAL: u32 = 0x4;
pub const NFT_SET_MAP: u32 = 0x8;
pub const NFT_SET_TIMEOUT: u32 = 0x10;
pub const NFT_SET_EVAL: u32 = 0x20;

// Table flags
pub const NFT_TABLE_F_DORMANT: u32 = 0x1;

// Reject
pub const NFTA_REJECT_TYPE: u16 = 1;
pub const NFTA_REJECT_ICMP_CODE: u16 = 2;

// ── Helper functions ────────────────────────────────────────────

pub fn family_str(f: u8) -> &'static str {
    match f {
        NFPROTO_IPV4 => "ip", NFPROTO_IPV6 => "ip6", NFPROTO_INET => "inet",
        NFPROTO_ARP => "arp", NFPROTO_BRIDGE => "bridge", NFPROTO_NETDEV => "netdev",
        _ => "unknown",
    }
}

pub fn family_from_str(s: &str) -> Option<u8> {
    match s {
        "ip" => Some(NFPROTO_IPV4), "ip6" => Some(NFPROTO_IPV6),
        "inet" => Some(NFPROTO_INET), "arp" => Some(NFPROTO_ARP),
        "bridge" => Some(NFPROTO_BRIDGE), "netdev" => Some(NFPROTO_NETDEV),
        _ => None,
    }
}

pub fn hook_str(h: u32) -> &'static str {
    hook_str_family(h, NFPROTO_UNSPEC)
}

pub fn hook_str_family(h: u32, family: u8) -> &'static str {
    // netdev/bridge families share integer slots with inet's
    // pre/input etc. Map from (family, num) to the right keyword.
    if family == NFPROTO_NETDEV {
        return match h { 0 => "ingress", 1 => "egress", _ => "unknown" };
    }
    match h {
        NF_INET_PRE_ROUTING => "prerouting", NF_INET_LOCAL_IN => "input",
        NF_INET_FORWARD => "forward", NF_INET_LOCAL_OUT => "output",
        NF_INET_POST_ROUTING => "postrouting",
        // inet family sprouted an `ingress` hook with number 5 in
        // later kernels; we report it for that context only.
        5 if family == NFPROTO_INET => "ingress",
        _ => "unknown",
    }
}

pub fn hook_from_str(s: &str) -> Option<u32> {
    match s {
        "prerouting" => Some(NF_INET_PRE_ROUTING), "input" => Some(NF_INET_LOCAL_IN),
        "forward" => Some(NF_INET_FORWARD), "output" => Some(NF_INET_LOCAL_OUT),
        "postrouting" => Some(NF_INET_POST_ROUTING),
        "ingress" => Some(0), "egress" => Some(1),
        _ => None,
    }
}

pub fn verdict_str(code: i32) -> &'static str {
    match code {
        NF_ACCEPT => "accept", NF_DROP => "drop", NFT_RETURN => "return",
        NFT_JUMP => "jump", NFT_GOTO => "goto", NFT_CONTINUE => "continue",
        _ => "unknown",
    }
}

pub fn priority_from_str(s: &str, family: u8) -> i32 {
    priority_from_str_opt(s, family).unwrap_or(0)
}

/// Strict form used by the parser: returns None when `name` isn't a
/// recognised priority keyword so callers can surface a syntax error
/// (`priority dummy` shouldn't silently land at 0).
pub fn priority_from_str_opt(s: &str, family: u8) -> Option<i32> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i32>() { return Some(n); }

    let (name, rest) = if let Some(pos) = s.find(|c: char| c == '+' || c == '-') {
        (s[..pos].trim(), s[pos..].trim())
    } else {
        (s, "")
    };

    let base = match name {
        "raw" => -300,
        "mangle" => -150,
        "dstnat" => if family == NFPROTO_BRIDGE { -300 } else { -100 },
        "filter" => if family == NFPROTO_BRIDGE { -200 } else { 0 },
        "security" => 50,
        "srcnat" => if family == NFPROTO_BRIDGE { 300 } else { 100 },
        "out" => 100,
        _ => return None,
    };

    if rest.is_empty() { return Some(base); }
    let offset: i32 = rest.replace(' ', "").parse().ok()?;
    Some(base + offset)
}

// ── Netlink socket ──────────────────────────────────────────────

pub struct NlSocket {
    fd: i32,
    pub portid: u32,
    pub seq: u32,
}

impl NlSocket {
    pub fn open() -> io::Result<Self> {
        // These 3 syscalls are the only unsafe in the entire crate
        let fd = unsafe {
            libc::socket(libc::AF_NETLINK, libc::SOCK_RAW | libc::SOCK_CLOEXEC, NETLINK_NETFILTER)
        };
        if fd < 0 { return Err(io::Error::last_os_error()); }

        // Bump the send/recv buffers. The default ~208KB runs out of
        // headroom on big atomic ruleset loads (a few hundred chains
        // in one batch) and the kernel returns ENOBUFS. 16 MB is
        // generous; libmnl uses 1 MB by default. SO_*BUFFORCE bypasses
        // the rmem/wmem_max sysctls when we have CAP_NET_ADMIN.
        let sz: libc::c_int = 16 * 1024 * 1024;
        unsafe {
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUFFORCE,
                &sz as *const _ as *const _, mem::size_of::<libc::c_int>() as u32);
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUFFORCE,
                &sz as *const _ as *const _, mem::size_of::<libc::c_int>() as u32);
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUF,
                &sz as *const _ as *const _, mem::size_of::<libc::c_int>() as u32);
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF,
                &sz as *const _ as *const _, mem::size_of::<libc::c_int>() as u32);
        }

        // NETLINK_NO_ENOBUFS: when the kernel's own send queue to us
        // fills up (we're too slow reading ACKs), don't return
        // ENOBUFS on our send(); just drop the overflow ACKs. This
        // is how libmnl handles bulk adds. A real error is still
        // delivered reliably; only redundant "no-error" ACKs get
        // dropped. (NETLINK_NO_ENOBUFS = 5 in linux/netlink.h UAPI.)
        const NETLINK_NO_ENOBUFS: libc::c_int = 5;
        let one: libc::c_int = 1;
        unsafe {
            libc::setsockopt(fd, libc::SOL_NETLINK, NETLINK_NO_ENOBUFS,
                &one as *const _ as *const _, mem::size_of::<libc::c_int>() as u32);
        }

        // NETLINK_CAP_ACK: tell the kernel we only want the nlmsghdr
        // of the original message echoed back in ACK/error messages,
        // not the whole body. Shrinks ACK traffic dramatically on
        // big batches so the kernel tx queue doesn't fill. libmnl
        // does the same. (NETLINK_CAP_ACK = 10.)
        const NETLINK_CAP_ACK: libc::c_int = 10;
        unsafe {
            libc::setsockopt(fd, libc::SOL_NETLINK, NETLINK_CAP_ACK,
                &one as *const _ as *const _, mem::size_of::<libc::c_int>() as u32);
        }

        let mut addr: libc::sockaddr_nl = unsafe { mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        let ret = unsafe {
            libc::bind(fd, &addr as *const _ as *const libc::sockaddr,
                       mem::size_of::<libc::sockaddr_nl>() as u32)
        };
        if ret < 0 {
            unsafe { libc::close(fd); }
            return Err(io::Error::last_os_error());
        }

        // Get assigned portid
        let mut bound_addr: libc::sockaddr_nl = unsafe { mem::zeroed() };
        let mut len = mem::size_of::<libc::sockaddr_nl>() as u32;
        let ret = unsafe {
            libc::getsockname(fd, &mut bound_addr as *mut _ as *mut libc::sockaddr, &mut len)
        };
        let portid = if ret == 0 { bound_addr.nl_pid } else { 0 };

        Ok(NlSocket { fd, portid, seq: 1 })
    }

    pub fn send(&self, buf: &[u8]) -> io::Result<usize> {
        let n = unsafe {
            libc::send(self.fd, buf.as_ptr() as *const _, buf.len(), 0)
        };
        if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
    }

    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe {
            libc::recv(self.fd, buf.as_mut_ptr() as *mut _, buf.len(), 0)
        };
        if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
    }

    pub fn recv_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> io::Result<usize> {
        let mut pfd = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
        let ret = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if ret <= 0 { return Ok(0); }
        self.recv(buf)
    }
}

impl Drop for NlSocket {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd); }
    }
}

// ── Message builder (100% safe) ─────────────────────────────────

fn align4(n: usize) -> usize { (n + 3) & !3 }

pub struct MsgBuilder {
    pub buf: Vec<u8>,
    pub seq: u32,
}

impl MsgBuilder {
    pub fn new() -> Self { MsgBuilder { buf: Vec::with_capacity(4096), seq: 1 } }

    pub fn batch_begin(&mut self) {
        // Batch begin uses nfnetlink subsystem 0, with res_id=NFNL_SUBSYS_NFTABLES
        let msg_type = NFNL_MSG_BATCH_BEGIN; // subsystem 0
        self.write_nlmsghdr(20, msg_type, NLM_F_REQUEST);
        self.write_nfgenmsg_batch();
    }

    pub fn batch_end(&mut self) {
        let msg_type = NFNL_MSG_BATCH_END;
        self.write_nlmsghdr(20, msg_type, NLM_F_REQUEST);
        self.write_nfgenmsg_batch();
    }

    /// Begin nftables message. Returns position of nlmsghdr for later fixup.
    pub fn begin_msg(&mut self, msg_type: u16, family: u8, flags: u16) -> usize {
        let pos = self.buf.len();
        let nl_type = (NFNL_SUBSYS_NFTABLES << 8) | msg_type;
        self.write_nlmsghdr(0, nl_type, NLM_F_REQUEST | flags); // len=0 placeholder
        self.write_nfgenmsg(family);
        pos
    }

    /// Fix up nlmsghdr.nlmsg_len at given position
    pub fn end_msg(&mut self, pos: usize) {
        let len = (self.buf.len() - pos) as u32;
        self.buf[pos..pos + 4].copy_from_slice(&len.to_ne_bytes());
    }

    pub fn put_str(&mut self, attr_type: u16, val: &str) {
        let data_len = val.len() + 1; // NUL terminated
        let nla_len = 4 + data_len;
        self.buf.extend_from_slice(&(nla_len as u16).to_ne_bytes());
        self.buf.extend_from_slice(&attr_type.to_ne_bytes());
        self.buf.extend_from_slice(val.as_bytes());
        self.buf.push(0); // NUL
        self.pad4();
    }

    pub fn put_u32(&mut self, attr_type: u16, val: u32) {
        self.buf.extend_from_slice(&8u16.to_ne_bytes()); // nla_len = 4 + 4
        self.buf.extend_from_slice(&attr_type.to_ne_bytes());
        self.buf.extend_from_slice(&val.to_ne_bytes());
    }

    pub fn put_u32_be(&mut self, attr_type: u16, val: u32) {
        self.buf.extend_from_slice(&8u16.to_ne_bytes());
        self.buf.extend_from_slice(&attr_type.to_ne_bytes());
        self.buf.extend_from_slice(&val.to_be_bytes());
    }

    pub fn put_u64_be(&mut self, attr_type: u16, val: u64) {
        self.buf.extend_from_slice(&12u16.to_ne_bytes()); // 4 + 8
        self.buf.extend_from_slice(&attr_type.to_ne_bytes());
        self.buf.extend_from_slice(&val.to_be_bytes());
    }

    pub fn put_bytes(&mut self, attr_type: u16, data: &[u8]) {
        let nla_len = 4 + data.len();
        self.buf.extend_from_slice(&(nla_len as u16).to_ne_bytes());
        self.buf.extend_from_slice(&attr_type.to_ne_bytes());
        self.buf.extend_from_slice(data);
        self.pad4();
    }

    /// Begin nested attribute. Returns position for fixup.
    pub fn begin_nested(&mut self, attr_type: u16) -> usize {
        let pos = self.buf.len();
        self.buf.extend_from_slice(&0u16.to_ne_bytes()); // placeholder len
        self.buf.extend_from_slice(&(NLA_F_NESTED | attr_type).to_ne_bytes());
        pos
    }

    /// End nested attribute (fix up nla_len)
    pub fn end_nested(&mut self, pos: usize) {
        let len = (self.buf.len() - pos) as u16;
        self.buf[pos..pos + 2].copy_from_slice(&len.to_ne_bytes());
    }

    /// Put data wrapped in NFTA_DATA_VALUE nested attribute
    pub fn put_data_value(&mut self, attr_type: u16, data: &[u8]) {
        let outer = self.begin_nested(attr_type);
        self.put_bytes(NFTA_DATA_VALUE, data);
        self.end_nested(outer);
    }

    pub fn finish(self) -> Vec<u8> { self.buf }
    pub fn current_seq(&self) -> u32 { self.seq }

    // Private helpers
    fn write_nlmsghdr(&mut self, len: u32, msg_type: u16, flags: u16) {
        let seq = self.seq;
        self.seq += 1;
        self.buf.extend_from_slice(&len.to_ne_bytes());
        self.buf.extend_from_slice(&msg_type.to_ne_bytes());
        self.buf.extend_from_slice(&flags.to_ne_bytes());
        self.buf.extend_from_slice(&seq.to_ne_bytes());
        self.buf.extend_from_slice(&0u32.to_ne_bytes()); // pid
    }

    fn write_nfgenmsg(&mut self, family: u8) {
        self.buf.push(family);
        self.buf.push(0); // version = NFNETLINK_V0
        self.buf.extend_from_slice(&0u16.to_be_bytes()); // res_id
    }

    fn write_nfgenmsg_batch(&mut self) {
        self.buf.push(0); // AF_UNSPEC
        self.buf.push(0); // NFNETLINK_V0
        self.buf.extend_from_slice(&(NFNL_SUBSYS_NFTABLES as u16).to_be_bytes()); // res_id = subsystem
    }

    fn pad4(&mut self) {
        let aligned = align4(self.buf.len());
        while self.buf.len() < aligned { self.buf.push(0); }
    }
}

// ── Response parser (100% safe) ─────────────────────────────────

/// Parse netlink attributes from a byte slice. Returns (type, value) pairs.
pub fn parse_attrs(data: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut result = Vec::new();
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let nla_len = u16::from_ne_bytes([data[pos], data[pos + 1]]) as usize;
        let nla_type = u16::from_ne_bytes([data[pos + 2], data[pos + 3]]) & 0x3FFF; // mask NLA_F_*
        if nla_len < 4 || pos + nla_len > data.len() { break; }
        let val = data[pos + 4..pos + nla_len].to_vec();
        result.push((nla_type, val));
        pos += align4(nla_len);
    }
    result
}

/// Get a string attribute (NUL-terminated)
pub fn attr_str(val: &[u8]) -> &str {
    let end = val.iter().position(|&b| b == 0).unwrap_or(val.len());
    std::str::from_utf8(&val[..end]).unwrap_or("")
}

/// Get a u32 attribute (native endian)
pub fn attr_u32(val: &[u8]) -> u32 {
    if val.len() >= 4 {
        u32::from_ne_bytes([val[0], val[1], val[2], val[3]])
    } else { 0 }
}

/// Get a u32 attribute (big endian)
pub fn attr_u32_be(val: &[u8]) -> u32 {
    if val.len() >= 4 {
        u32::from_be_bytes([val[0], val[1], val[2], val[3]])
    } else { 0 }
}

/// Get a u64 attribute (big endian)
pub fn attr_u64_be(val: &[u8]) -> u64 {
    if val.len() >= 8 {
        u64::from_be_bytes([val[0], val[1], val[2], val[3], val[4], val[5], val[6], val[7]])
    } else { 0 }
}

/// Get an i32 attribute (big endian)
pub fn attr_i32_be(val: &[u8]) -> i32 {
    if val.len() >= 4 {
        i32::from_be_bytes([val[0], val[1], val[2], val[3]])
    } else { 0 }
}

/// Process received netlink data. Calls handler for each nftables message.
/// Returns true if NLMSG_DONE was seen.
/// Handler receives (msg_type_without_subsys, family, attrs_data).
pub fn process_response<F>(
    data: &[u8], portid: u32, mut handler: F
) -> io::Result<bool>
where F: FnMut(u16, u8, &[u8])
{
    let mut pos = 0;
    while pos + 16 <= data.len() {
        let nlmsg_len = u32::from_ne_bytes([data[pos], data[pos+1], data[pos+2], data[pos+3]]) as usize;
        let nlmsg_type = u16::from_ne_bytes([data[pos+4], data[pos+5]]);
        let _flags = u16::from_ne_bytes([data[pos+6], data[pos+7]]);
        let _seq = u32::from_ne_bytes([data[pos+8], data[pos+9], data[pos+10], data[pos+11]]);
        let _pid = u32::from_ne_bytes([data[pos+12], data[pos+13], data[pos+14], data[pos+15]]);

        if nlmsg_len < 16 || pos + nlmsg_len > data.len() { break; }

        if nlmsg_type == NLMSG_DONE {
            return Ok(true);
        }

        if nlmsg_type == NLMSG_ERROR {
            // Body is an nlmsgerr: i32 error code (negative errno if set,
            // zero for a pure ACK) followed by the echoed original header.
            // Previously we swallowed EOPNOTSUPP; the kernel returns that
            // for legitimate rejections (jump loops, masquerade in the
            // wrong hook, invalid family-hook combos) so swallowing it
            // makes every negative test pass silently. Any non-zero
            // value is a real error.
            // nlmsgerr: i32 error + original nlmsghdr
            if nlmsg_len >= 20 {
                let err = i32::from_ne_bytes([data[pos+16], data[pos+17], data[pos+18], data[pos+19]]);
                if err < 0 {
                    return Err(io::Error::from_raw_os_error(-err));
                }
            }
            pos += align4(nlmsg_len);
            continue;
        }

        // nftables message: after nlmsghdr comes nfgenmsg (4 bytes)
        if nlmsg_len >= 20 {
            let family = data[pos + 16];
            let msg_type_raw = nlmsg_type & 0xFF; // strip subsystem
            let payload_start = pos + 20; // after nlmsghdr(16) + nfgenmsg(4)
            let payload_end = pos + nlmsg_len;
            if payload_start <= payload_end && payload_end <= data.len() {
                handler(msg_type_raw, family, &data[payload_start..payload_end]);
            }
        }

        pos += align4(nlmsg_len);
    }
    let _ = portid;
    Ok(false)
}

/// Check batch ACK responses. Returns Ok if no kernel errors.
pub fn check_ack(data: &[u8]) -> io::Result<()> {
    process_response(data, 0, |_, _, _| {})?;
    Ok(())
}

/// Map nft set type name to (kernel_type_id, key_len_bytes).
/// Kernel type IDs match include/uapi/linux/netfilter/nf_tables.h
/// NFT_DATA_* plus type-specific ids used by libnftables.
pub fn set_key_type(s: &str) -> (u32, u32) {
    match s {
        "invalid_type" => (0, 0),
        // Data side only — kernel wants NFT_DATA_VERDICT (a mask, not
        // a numeric type index) with dlen=0 since the verdict struct
        // size is supplied via the element's NFTA_DATA_VERDICT nesting.
        "verdict" => (0xffffff00, 0),
        "nf_proto" => (2, 1),
        "bitmask" => (3, 4),
        "integer" => (4, 4),
        "string" => (5, 16),
        "ll_addr" => (6, 6),
        "ipv4_addr" => (7, 4),
        "ipv6_addr" => (8, 16),
        "ether_addr" => (9, 6),
        "ether_type" => (10, 2),
        "arp_op" => (11, 2),
        "inet_proto" => (12, 1),
        "inet_service" => (13, 2),
        "icmp_type" => (14, 1),
        "mark" => (15, 4),
        "ifname" => (16, 16),
        "pkttype" => (17, 1),
        "icmp_code" => (18, 1),
        "icmpv6_type" => (19, 1),
        "icmpv6_code" => (20, 1),
        "icmpx_code" => (21, 1),
        "devgroup" => (22, 4),
        "dscp" => (23, 1),
        "ecn" => (24, 1),
        "fib_addrtype" => (25, 4),
        "boolean" => (26, 1),
        "ct_state" => (27, 4),
        "ct_dir" => (28, 1),
        "ct_status" => (29, 4),
        "icmp_kind" => (30, 1),
        "ct_label" => (31, 16),
        "iface_index" => (36, 4),
        _ => (0, 0),
    }
}

pub fn set_type_name(t: u32) -> &'static str {
    match t {
        // NFT_DATA_VERDICT is a mask (0xffffff00), not a numeric
        // type index — maps that store verdicts set this on
        // NFTA_SET_DATA_TYPE with DATA_LEN=0.
        0xffffff00 => "verdict",
        1 => "integer_legacy", 2 => "nf_proto", 3 => "bitmask",
        4 => "integer", 5 => "string", 6 => "ll_addr",
        7 => "ipv4_addr", 8 => "ipv6_addr", 9 => "ether_addr",
        10 => "ether_type", 11 => "arp_op",
        12 => "inet_proto", 13 => "inet_service",
        14 => "icmp_type", 15 => "mark", 16 => "ifname",
        17 => "pkttype", 18 => "icmp_code",
        19 => "icmpv6_type", 20 => "icmpv6_code",
        21 => "icmpx_code", 22 => "devgroup",
        23 => "dscp", 24 => "ecn",
        25 => "fib_addrtype", 26 => "boolean",
        27 => "ct_state", 28 => "ct_dir", 29 => "ct_status",
        30 => "icmp_kind", 31 => "ct_label",
        36 => "iface_index",
        _ => "unknown",
    }
}
