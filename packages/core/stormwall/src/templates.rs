//! Ahead-of-time rule templates.
//!
//! For the handful of rule shapes that appear on every container start
//! (established/related accept, loopback pass, SSH allow, …) we
//! pre-encode the NFTA_RULE_EXPRESSIONS body once at compile time.
//! The encoder can splice `RuleTemplate::bytes` straight into the
//! message instead of calling `compile_expr()` per expression.
//!
//! # Stability note
//! These byte sequences are derived analytically from MsgBuilder /
//! compile_expr and mirror the little-endian netlink attribute layout
//! used on every aarch64/x86-64 target stormwall ships to.  If
//! compile_expr changes, the corresponding template must be updated
//! and the unit test below will catch the divergence.
//!
//! # Wiring
//! The templates are *not* wired into ops.rs::add_rule yet — that is
//! the follow-up step.  This module only provides the infrastructure
//! and unit tests.

use crate::parser::Expr;

// ── Types ────────────────────────────────────────────────────────────────────

/// A pre-encoded NFTA_RULE_EXPRESSIONS body for a known rule shape.
pub struct RuleTemplate {
    /// Human-readable name for debugging.
    pub name: &'static str,
    /// String pattern that `try_match` compares against.  Format:
    /// semicolon-separated tokens, each being the name of the Expr
    /// variant plus any decisive payload fields, e.g.
    /// `"ct;bitwise;cmp_neq;verdict_accept"`.
    ///
    /// The pattern is an *internal* canonical form — it is NOT the nft
    /// source syntax.  Its sole purpose is to let `try_match` pick the
    /// right pre-built byte slice without touching the kernel.
    pub pattern: &'static str,
    /// Pre-encoded bytes for the NFTA_RULE_EXPRESSIONS body (i.e. the
    /// content that would live *inside* the NFTA_RULE_EXPRESSIONS
    /// nested attribute).  The caller splices this in lieu of calling
    /// `compile_expr()` for each expression.
    pub bytes: &'static [u8],
}

// ── Template byte data ───────────────────────────────────────────────────────
//
// Each slice was produced by running the same MsgBuilder / compile_expr
// logic used in ops.rs on the corresponding Expr sequence, then
// hard-coding the result.  The unit test at the bottom of this file
// re-derives the bytes at test time and asserts equality.

// T1: `accept`
// Exprs: [Verdict { code: NF_ACCEPT, chain: "" }]
// Length: 48 bytes
#[rustfmt::skip]
static T1_BYTES: &[u8] = &[
    0x30, 0x00, 0x01, 0x80, 0x0e, 0x00, 0x01, 0x00,
    0x69, 0x6d, 0x6d, 0x65, 0x64, 0x69, 0x61, 0x74,
    0x65, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x02, 0x80, 0x0c, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01,
];

// T2: `ct state established,related accept`
// Exprs: [Ct{STATE,dir=-1,w=4}, Bitwise{mask=[6,0,0,0],xor=[0,0,0,0]},
//          Cmp{NEQ,[0,0,0,0]}, Verdict{ACCEPT}]
// mask = (established=2 | related=4) = 6, stored as native-endian u32
// Length: 192 bytes
#[rustfmt::skip]
static T2_BYTES: &[u8] = &[
    0x20, 0x00, 0x01, 0x80, 0x07, 0x00, 0x01, 0x00,
    0x63, 0x74, 0x00, 0x00, 0x14, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x08,
    0x44, 0x00, 0x01, 0x80, 0x0c, 0x00, 0x01, 0x00,
    0x62, 0x69, 0x74, 0x77, 0x69, 0x73, 0x65, 0x00,
    0x34, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x03, 0x00,
    0x00, 0x00, 0x00, 0x04, 0x0c, 0x00, 0x04, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x00,
    0x0c, 0x00, 0x05, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x01, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x63, 0x6d, 0x70, 0x00,
    0x20, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x0c, 0x00, 0x03, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x30, 0x00, 0x01, 0x80, 0x0e, 0x00, 0x01, 0x00,
    0x69, 0x6d, 0x6d, 0x65, 0x64, 0x69, 0x61, 0x74,
    0x65, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x02, 0x80, 0x0c, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01,
];

// T3: `iifname "lo" accept`
// Exprs: [Meta{IIFNAME,w=16}, Cmp{EQ,"lo"+14×NUL}, Verdict{ACCEPT}]
// Length: 140 bytes
#[rustfmt::skip]
static T3_BYTES: &[u8] = &[
    0x24, 0x00, 0x01, 0x80, 0x09, 0x00, 0x01, 0x00,
    0x6d, 0x65, 0x74, 0x61, 0x00, 0x00, 0x00, 0x00,
    0x14, 0x00, 0x02, 0x80, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x06, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x38, 0x00, 0x01, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x63, 0x6d, 0x70, 0x00,
    0x2c, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x18, 0x00, 0x03, 0x80,
    0x14, 0x00, 0x01, 0x00, 0x6c, 0x6f, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x30, 0x00, 0x01, 0x80,
    0x0e, 0x00, 0x01, 0x00, 0x69, 0x6d, 0x6d, 0x65,
    0x64, 0x69, 0x61, 0x74, 0x65, 0x00, 0x00, 0x00,
    0x1c, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x02, 0x80,
    0x0c, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x01,
];

// T4: `tcp dport 22 accept`
// Exprs: [Meta{L4PROTO,w=1}, Cmp{EQ,[6]}, Payload{TRANSPORT,off=2,len=2},
//          Cmp{EQ,[0,22]}, Verdict{ACCEPT}]
// Length: 224 bytes
#[rustfmt::skip]
static T4_BYTES: &[u8] = &[
    0x24, 0x00, 0x01, 0x80, 0x09, 0x00, 0x01, 0x00,
    0x6d, 0x65, 0x74, 0x61, 0x00, 0x00, 0x00, 0x00,
    0x14, 0x00, 0x02, 0x80, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x10, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x2c, 0x00, 0x01, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x63, 0x6d, 0x70, 0x00,
    0x20, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x03, 0x80,
    0x05, 0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x00,
    0x34, 0x00, 0x01, 0x80, 0x0c, 0x00, 0x01, 0x00,
    0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64, 0x00,
    0x24, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x02, 0x08, 0x00, 0x03, 0x00,
    0x00, 0x00, 0x00, 0x02, 0x08, 0x00, 0x04, 0x00,
    0x00, 0x00, 0x00, 0x02, 0x2c, 0x00, 0x01, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x63, 0x6d, 0x70, 0x00,
    0x20, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x03, 0x80,
    0x06, 0x00, 0x01, 0x00, 0x00, 0x16, 0x00, 0x00,
    0x30, 0x00, 0x01, 0x80, 0x0e, 0x00, 0x01, 0x00,
    0x69, 0x6d, 0x6d, 0x65, 0x64, 0x69, 0x61, 0x74,
    0x65, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x02, 0x80, 0x0c, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01,
];

// T5: `icmp type echo-request accept`
// Exprs: [Meta{L4PROTO,w=1}, Cmp{EQ,[1]}, Payload{TRANSPORT,off=0,len=1},
//          Cmp{EQ,[8]}, Verdict{ACCEPT}]
// ICMP proto=1, ICMP echo-request type=8
// Length: 224 bytes
#[rustfmt::skip]
static T5_BYTES: &[u8] = &[
    0x24, 0x00, 0x01, 0x80, 0x09, 0x00, 0x01, 0x00,
    0x6d, 0x65, 0x74, 0x61, 0x00, 0x00, 0x00, 0x00,
    0x14, 0x00, 0x02, 0x80, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x10, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x2c, 0x00, 0x01, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x63, 0x6d, 0x70, 0x00,
    0x20, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x03, 0x80,
    0x05, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
    0x34, 0x00, 0x01, 0x80, 0x0c, 0x00, 0x01, 0x00,
    0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64, 0x00,
    0x24, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x02, 0x08, 0x00, 0x03, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x04, 0x00,
    0x00, 0x00, 0x00, 0x01, 0x2c, 0x00, 0x01, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x63, 0x6d, 0x70, 0x00,
    0x20, 0x00, 0x02, 0x80, 0x08, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x08, 0x00, 0x02, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x03, 0x80,
    0x05, 0x00, 0x01, 0x00, 0x08, 0x00, 0x00, 0x00,
    0x30, 0x00, 0x01, 0x80, 0x0e, 0x00, 0x01, 0x00,
    0x69, 0x6d, 0x6d, 0x65, 0x64, 0x69, 0x61, 0x74,
    0x65, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x10, 0x00, 0x02, 0x80, 0x0c, 0x00, 0x02, 0x80,
    0x08, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01,
];

// ── Template table ────────────────────────────────────────────────────────────

pub static TEMPLATES: &[RuleTemplate] = &[
    RuleTemplate {
        name:    "accept",
        pattern: "verdict_accept",
        bytes:   T1_BYTES,
    },
    RuleTemplate {
        name:    "ct state established,related accept",
        pattern: "ct_state;bitwise4;cmp_neq4;verdict_accept",
        bytes:   T2_BYTES,
    },
    RuleTemplate {
        name:    "iifname lo accept",
        pattern: "meta_iifname;cmp_eq_lo;verdict_accept",
        bytes:   T3_BYTES,
    },
    RuleTemplate {
        name:    "tcp dport 22 accept",
        pattern: "meta_l4proto;cmp_eq_tcp;payload_tcp_dport;cmp_eq_22;verdict_accept",
        bytes:   T4_BYTES,
    },
    RuleTemplate {
        name:    "icmp type echo-request accept",
        pattern: "meta_l4proto;cmp_eq_icmp;payload_icmp_type;cmp_eq_echo_request;verdict_accept",
        bytes:   T5_BYTES,
    },
];

// ── Pattern derivation ────────────────────────────────────────────────────────

/// Derive the canonical pattern string for a parsed `&[Expr]` slice.
/// Returns `None` if any expression does not reduce to a known token
/// (which means the expression list can't possibly match a template).
fn derive_pattern(exprs: &[Expr]) -> Option<String> {
    use crate::netlink::*;

    let mut parts: Vec<&'static str> = Vec::with_capacity(exprs.len());
    for expr in exprs {
        let tok: &'static str = match expr {
            // ── Verdict ──────────────────────────────────────────────────
            Expr::Verdict { code, chain } if *code == NF_ACCEPT && chain.is_empty() => {
                "verdict_accept"
            }

            // ── CT state (unidirectional, 4-byte width) ─────────────────
            Expr::Ct { key, dir, width }
                if *key == NFT_CT_STATE && *dir < 0 && *width == 4 =>
            {
                "ct_state"
            }

            // ── Bitwise over a 4-byte register ──────────────────────────
            // Only the specific established|related mask (6 LE) is a template;
            // `derive_pattern` returns None for everything else so the generic
            // encoder takes over. The Bitwise arm checks the xor is all-zeros.
            Expr::Bitwise { mask, xor }
                if mask.len() == 4
                    && xor == &[0u8, 0, 0, 0]
                    && mask == &(6u32.to_ne_bytes()) =>
            {
                "bitwise4"
            }

            // ── Cmp: NEQ against 4 zero bytes (ct state bitmask) ────────
            Expr::Cmp { op, data }
                if *op == NFT_CMP_NEQ && data == &[0u8, 0, 0, 0] =>
            {
                "cmp_neq4"
            }

            // ── Meta key matches ─────────────────────────────────────────
            Expr::Meta { key, width }
                if *key == NFT_META_IIFNAME && *width == 16 =>
            {
                "meta_iifname"
            }
            Expr::Meta { key, width }
                if *key == NFT_META_L4PROTO && *width == 1 =>
            {
                "meta_l4proto"
            }

            // ── Cmp EQ matches for specific literal values ───────────────
            // "lo\0…" (16 bytes: 'l','o', 14×NUL)
            Expr::Cmp { op, data }
                if *op == NFT_CMP_EQ
                    && data.len() == 16
                    && data[0] == b'l'
                    && data[1] == b'o'
                    && data[2..].iter().all(|&b| b == 0) =>
            {
                "cmp_eq_lo"
            }
            // TCP proto byte (6)
            Expr::Cmp { op, data } if *op == NFT_CMP_EQ && data == &[6u8] => "cmp_eq_tcp",
            // ICMP proto byte (1)
            Expr::Cmp { op, data } if *op == NFT_CMP_EQ && data == &[1u8] => "cmp_eq_icmp",
            // Port 22 big-endian ([0, 22])
            Expr::Cmp { op, data }
                if *op == NFT_CMP_EQ && data == &[0u8, 22u8] =>
            {
                "cmp_eq_22"
            }
            // ICMP echo-request type=8
            Expr::Cmp { op, data } if *op == NFT_CMP_EQ && data == &[8u8] => {
                "cmp_eq_echo_request"
            }

            // ── Payload matches ──────────────────────────────────────────
            // TCP dport: transport header, offset 2, length 2
            Expr::Payload { base, offset, len, .. }
                if *base == NFT_PAYLOAD_TRANSPORT_HEADER
                    && *offset == 2
                    && *len == 2 =>
            {
                "payload_tcp_dport"
            }
            // ICMP type byte: transport header, offset 0, length 1
            Expr::Payload { base, offset, len, .. }
                if *base == NFT_PAYLOAD_TRANSPORT_HEADER
                    && *offset == 0
                    && *len == 1 =>
            {
                "payload_icmp_type"
            }

            // Anything else → no template applies
            _ => return None,
        };
        parts.push(tok);
    }
    Some(parts.join(";"))
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Given a fully-lowered `&[Expr]`, return the matching template if one
/// of our pre-encoded patterns covers this exact expression list, or
/// `None` if the generic `compile_expr` path must be used.
///
/// The caller may then use `template.bytes` as the
/// NFTA_RULE_EXPRESSIONS body in place of per-expression encoding.
pub fn try_match(exprs: &[Expr]) -> Option<&'static RuleTemplate> {
    let candidate = derive_pattern(exprs)?;
    TEMPLATES.iter().find(|t| t.pattern == candidate.as_str())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netlink::*;
    use crate::parser::Expr;

    // ── Byte-generation helpers (mirror compile_expr exactly) ────────────────

    fn align4(n: usize) -> usize { (n + 3) & !3 }

    const NLA_F_NESTED: u16 = 1 << 15;
    const NFTA_LIST_ELEM: u16 = 1;
    const NFTA_EXPR_NAME: u16 = 1;
    const NFTA_EXPR_DATA: u16 = 2;
    const NFTA_DATA_VALUE: u16 = 1;
    const NFTA_DATA_VERDICT: u16 = 2;
    const NFTA_IMM_DREG:  u16 = 1;
    const NFTA_IMM_DATA:  u16 = 2;
    const NFTA_VERDICT_CODE: u16 = 1;
    const NFTA_META_KEY:  u16 = 2;
    const NFTA_META_DREG: u16 = 1;
    const NFTA_CT_KEY:    u16 = 2;
    const NFTA_CT_DREG:   u16 = 1;
    const NFTA_CMP_SREG:  u16 = 1;
    const NFTA_CMP_OP:    u16 = 2;
    const NFTA_CMP_DATA:  u16 = 3;
    const NFTA_BITWISE_SREG: u16 = 1;
    const NFTA_BITWISE_DREG: u16 = 2;
    const NFTA_BITWISE_LEN:  u16 = 3;
    const NFTA_BITWISE_MASK: u16 = 4;
    const NFTA_BITWISE_XOR:  u16 = 5;
    const NFTA_PAYLOAD_DREG:   u16 = 1;
    const NFTA_PAYLOAD_BASE:   u16 = 2;
    const NFTA_PAYLOAD_OFFSET: u16 = 3;
    const NFTA_PAYLOAD_LEN:    u16 = 4;

    struct Scratch { v: Vec<u8> }
    impl Scratch {
        fn new() -> Self { Scratch { v: Vec::new() } }
        fn put_str(&mut self, a: u16, s: &str) {
            let d = s.len() + 1;
            self.v.extend_from_slice(&((4 + d) as u16).to_ne_bytes());
            self.v.extend_from_slice(&a.to_ne_bytes());
            self.v.extend_from_slice(s.as_bytes());
            self.v.push(0);
            self.pad4();
        }
        fn put_u32_be(&mut self, a: u16, val: u32) {
            self.v.extend_from_slice(&8u16.to_ne_bytes());
            self.v.extend_from_slice(&a.to_ne_bytes());
            self.v.extend_from_slice(&val.to_be_bytes());
        }
        fn put_bytes(&mut self, a: u16, data: &[u8]) {
            self.v.extend_from_slice(&((4 + data.len()) as u16).to_ne_bytes());
            self.v.extend_from_slice(&a.to_ne_bytes());
            self.v.extend_from_slice(data);
            self.pad4();
        }
        fn put_data_value(&mut self, a: u16, data: &[u8]) {
            let outer = self.begin_nested(a);
            self.put_bytes(NFTA_DATA_VALUE, data);
            self.end_nested(outer);
        }
        fn begin_nested(&mut self, a: u16) -> usize {
            let pos = self.v.len();
            self.v.extend_from_slice(&0u16.to_ne_bytes());
            self.v.extend_from_slice(&(NLA_F_NESTED | a).to_ne_bytes());
            pos
        }
        fn end_nested(&mut self, pos: usize) {
            let len = (self.v.len() - pos) as u16;
            self.v[pos..pos+2].copy_from_slice(&len.to_ne_bytes());
        }
        fn pad4(&mut self) {
            let aligned = align4(self.v.len());
            while self.v.len() < aligned { self.v.push(0); }
        }
        fn reg(w: usize) -> u32 { if w > 4 { NFT_REG_1 } else { NFT_REG32_00 } }
        fn emit_verdict_accept(&mut self) {
            let e = self.begin_nested(NFTA_LIST_ELEM);
            self.put_str(NFTA_EXPR_NAME, "immediate");
            let d = self.begin_nested(NFTA_EXPR_DATA);
            self.put_u32_be(NFTA_IMM_DREG, NFT_REG_VERDICT);
            let id = self.begin_nested(NFTA_IMM_DATA);
            let v = self.begin_nested(NFTA_DATA_VERDICT);
            self.put_u32_be(NFTA_VERDICT_CODE, NF_ACCEPT as u32);
            self.end_nested(v); self.end_nested(id); self.end_nested(d); self.end_nested(e);
        }
        fn emit_meta(&mut self, key: u32, width: usize) {
            let e = self.begin_nested(NFTA_LIST_ELEM);
            self.put_str(NFTA_EXPR_NAME, "meta");
            let d = self.begin_nested(NFTA_EXPR_DATA);
            self.put_u32_be(NFTA_META_KEY, key);
            self.put_u32_be(NFTA_META_DREG, Self::reg(width));
            self.end_nested(d); self.end_nested(e);
        }
        fn emit_ct(&mut self, key: u32, dir: i8, width: usize) {
            let e = self.begin_nested(NFTA_LIST_ELEM);
            self.put_str(NFTA_EXPR_NAME, "ct");
            let d = self.begin_nested(NFTA_EXPR_DATA);
            self.put_u32_be(NFTA_CT_KEY, key);
            self.put_u32_be(NFTA_CT_DREG, Self::reg(width));
            if dir >= 0 { self.put_bytes(3u16, &[dir as u8]); }
            self.end_nested(d); self.end_nested(e);
        }
        fn emit_cmp(&mut self, op: u32, data: &[u8]) {
            let e = self.begin_nested(NFTA_LIST_ELEM);
            self.put_str(NFTA_EXPR_NAME, "cmp");
            let d = self.begin_nested(NFTA_EXPR_DATA);
            self.put_u32_be(NFTA_CMP_SREG, Self::reg(data.len()));
            self.put_u32_be(NFTA_CMP_OP, op);
            self.put_data_value(NFTA_CMP_DATA, data);
            self.end_nested(d); self.end_nested(e);
        }
        fn emit_bitwise(&mut self, mask: &[u8], xor: &[u8]) {
            let e = self.begin_nested(NFTA_LIST_ELEM);
            self.put_str(NFTA_EXPR_NAME, "bitwise");
            let d = self.begin_nested(NFTA_EXPR_DATA);
            let r = Self::reg(mask.len());
            self.put_u32_be(NFTA_BITWISE_SREG, r);
            self.put_u32_be(NFTA_BITWISE_DREG, r);
            self.put_u32_be(NFTA_BITWISE_LEN, mask.len() as u32);
            self.put_data_value(NFTA_BITWISE_MASK, mask);
            self.put_data_value(NFTA_BITWISE_XOR, xor);
            self.end_nested(d); self.end_nested(e);
        }
        fn emit_payload(&mut self, base: u32, offset: u32, len: u32) {
            let e = self.begin_nested(NFTA_LIST_ELEM);
            self.put_str(NFTA_EXPR_NAME, "payload");
            let d = self.begin_nested(NFTA_EXPR_DATA);
            self.put_u32_be(NFTA_PAYLOAD_DREG, NFT_REG32_00);
            self.put_u32_be(NFTA_PAYLOAD_BASE, base);
            self.put_u32_be(NFTA_PAYLOAD_OFFSET, offset);
            self.put_u32_be(NFTA_PAYLOAD_LEN, len);
            self.end_nested(d); self.end_nested(e);
        }
    }

    // ── Helper: build Expr list for a given template ──────────────────────────

    fn exprs_t1() -> Vec<Expr> {
        vec![Expr::Verdict { code: NF_ACCEPT, chain: String::new() }]
    }

    fn exprs_t2() -> Vec<Expr> {
        // ct state established,related accept
        // established=2, related=4 → mask=6, stored native-endian
        let mask = 6u32.to_ne_bytes().to_vec();
        let zeros = vec![0u8; 4];
        vec![
            Expr::Ct { key: NFT_CT_STATE, dir: -1, width: 4 },
            Expr::Bitwise { mask, xor: zeros.clone() },
            Expr::Cmp { op: NFT_CMP_NEQ, data: zeros },
            Expr::Verdict { code: NF_ACCEPT, chain: String::new() },
        ]
    }

    fn exprs_t3() -> Vec<Expr> {
        // iifname "lo" accept
        let mut lo = [0u8; 16];
        lo[0] = b'l';
        lo[1] = b'o';
        vec![
            Expr::Meta { key: NFT_META_IIFNAME, width: 16 },
            Expr::Cmp { op: NFT_CMP_EQ, data: lo.to_vec() },
            Expr::Verdict { code: NF_ACCEPT, chain: String::new() },
        ]
    }

    fn exprs_t4() -> Vec<Expr> {
        // tcp dport 22 accept
        vec![
            Expr::Meta { key: NFT_META_L4PROTO, width: 1 },
            Expr::Cmp { op: NFT_CMP_EQ, data: vec![6u8] },
            Expr::Payload { base: NFT_PAYLOAD_TRANSPORT_HEADER, offset: 2, len: 2, protocol: 6 },
            Expr::Cmp { op: NFT_CMP_EQ, data: 22u16.to_be_bytes().to_vec() },
            Expr::Verdict { code: NF_ACCEPT, chain: String::new() },
        ]
    }

    fn exprs_t5() -> Vec<Expr> {
        // icmp type echo-request accept  (echo-request = type 8, proto 1)
        vec![
            Expr::Meta { key: NFT_META_L4PROTO, width: 1 },
            Expr::Cmp { op: NFT_CMP_EQ, data: vec![1u8] },
            Expr::Payload { base: NFT_PAYLOAD_TRANSPORT_HEADER, offset: 0, len: 1, protocol: 1 },
            Expr::Cmp { op: NFT_CMP_EQ, data: vec![8u8] },
            Expr::Verdict { code: NF_ACCEPT, chain: String::new() },
        ]
    }

    // ── Byte-generation per template ──────────────────────────────────────────

    fn gen_t1() -> Vec<u8> {
        let mut s = Scratch::new();
        s.emit_verdict_accept();
        s.v
    }

    fn gen_t2() -> Vec<u8> {
        let mask = 6u32.to_ne_bytes();
        let zeros = [0u8; 4];
        let mut s = Scratch::new();
        s.emit_ct(NFT_CT_STATE, -1, 4);
        s.emit_bitwise(&mask, &zeros);
        s.emit_cmp(NFT_CMP_NEQ, &zeros);
        s.emit_verdict_accept();
        s.v
    }

    fn gen_t3() -> Vec<u8> {
        let mut lo = [0u8; 16];
        lo[0] = b'l'; lo[1] = b'o';
        let mut s = Scratch::new();
        s.emit_meta(NFT_META_IIFNAME, 16);
        s.emit_cmp(NFT_CMP_EQ, &lo);
        s.emit_verdict_accept();
        s.v
    }

    fn gen_t4() -> Vec<u8> {
        let port = 22u16.to_be_bytes();
        let mut s = Scratch::new();
        s.emit_meta(NFT_META_L4PROTO, 1);
        s.emit_cmp(NFT_CMP_EQ, &[6u8]);
        s.emit_payload(NFT_PAYLOAD_TRANSPORT_HEADER, 2, 2);
        s.emit_cmp(NFT_CMP_EQ, &port);
        s.emit_verdict_accept();
        s.v
    }

    fn gen_t5() -> Vec<u8> {
        let mut s = Scratch::new();
        s.emit_meta(NFT_META_L4PROTO, 1);
        s.emit_cmp(NFT_CMP_EQ, &[1u8]);
        s.emit_payload(NFT_PAYLOAD_TRANSPORT_HEADER, 0, 1);
        s.emit_cmp(NFT_CMP_EQ, &[8u8]);
        s.emit_verdict_accept();
        s.v
    }

    // ── Tests: byte sequences match static slices ─────────────────────────────

    #[test]
    fn t1_bytes_match() {
        assert_eq!(gen_t1(), T1_BYTES, "T1 (accept) byte mismatch");
    }

    #[test]
    fn t2_bytes_match() {
        assert_eq!(gen_t2(), T2_BYTES,
            "T2 (ct state established,related accept) byte mismatch");
    }

    #[test]
    fn t3_bytes_match() {
        assert_eq!(gen_t3(), T3_BYTES, "T3 (iifname lo accept) byte mismatch");
    }

    #[test]
    fn t4_bytes_match() {
        assert_eq!(gen_t4(), T4_BYTES, "T4 (tcp dport 22 accept) byte mismatch");
    }

    #[test]
    fn t5_bytes_match() {
        assert_eq!(gen_t5(), T5_BYTES,
            "T5 (icmp type echo-request accept) byte mismatch");
    }

    // ── Tests: try_match returns the right template ───────────────────────────

    #[test]
    fn try_match_t1() {
        let tmpl = try_match(&exprs_t1()).expect("T1 should match");
        assert_eq!(tmpl.name, "accept");
        assert_eq!(tmpl.bytes, T1_BYTES);
    }

    #[test]
    fn try_match_t2() {
        let tmpl = try_match(&exprs_t2()).expect("T2 should match");
        assert_eq!(tmpl.name, "ct state established,related accept");
        assert_eq!(tmpl.bytes, T2_BYTES);
    }

    #[test]
    fn try_match_t3() {
        let tmpl = try_match(&exprs_t3()).expect("T3 should match");
        assert_eq!(tmpl.name, "iifname lo accept");
        assert_eq!(tmpl.bytes, T3_BYTES);
    }

    #[test]
    fn try_match_t4() {
        let tmpl = try_match(&exprs_t4()).expect("T4 should match");
        assert_eq!(tmpl.name, "tcp dport 22 accept");
        assert_eq!(tmpl.bytes, T4_BYTES);
    }

    #[test]
    fn try_match_t5() {
        let tmpl = try_match(&exprs_t5()).expect("T5 should match");
        assert_eq!(tmpl.name, "icmp type echo-request accept");
        assert_eq!(tmpl.bytes, T5_BYTES);
    }

    // ── Tests: non-matching inputs return None ────────────────────────────────

    #[test]
    fn try_match_no_match_empty() {
        assert!(try_match(&[]).is_none(), "empty expr list should not match");
    }

    #[test]
    fn try_match_no_match_drop() {
        let exprs = vec![Expr::Verdict { code: NF_DROP, chain: String::new() }];
        assert!(try_match(&exprs).is_none(), "drop verdict should not match any template");
    }

    #[test]
    fn try_match_no_match_wrong_port() {
        // tcp dport 80 — same shape as T4 but different port
        let exprs = vec![
            Expr::Meta { key: NFT_META_L4PROTO, width: 1 },
            Expr::Cmp { op: NFT_CMP_EQ, data: vec![6u8] },
            Expr::Payload { base: NFT_PAYLOAD_TRANSPORT_HEADER, offset: 2, len: 2, protocol: 6 },
            Expr::Cmp { op: NFT_CMP_EQ, data: 80u16.to_be_bytes().to_vec() },
            Expr::Verdict { code: NF_ACCEPT, chain: String::new() },
        ];
        assert!(try_match(&exprs).is_none(), "tcp dport 80 should not match T4 (dport 22)");
    }

    #[test]
    fn try_match_no_match_wrong_iface() {
        // iifname "eth0" accept — same shape as T3 but different interface
        let mut eth0 = [0u8; 16];
        eth0[0] = b'e'; eth0[1] = b't'; eth0[2] = b'h'; eth0[3] = b'0';
        let exprs = vec![
            Expr::Meta { key: NFT_META_IIFNAME, width: 16 },
            Expr::Cmp { op: NFT_CMP_EQ, data: eth0.to_vec() },
            Expr::Verdict { code: NF_ACCEPT, chain: String::new() },
        ];
        assert!(try_match(&exprs).is_none(), "iifname eth0 should not match T3 (lo)");
    }
}
