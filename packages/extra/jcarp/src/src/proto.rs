//! CARP v2 wire protocol helpers.
//!
//! The header layout, constants, HMAC ordering, and election inputs mirror
//! OpenBSD `sys/netinet/ip_carp.h` and `ip_carp.c` as of 2025-12. The Rust
//! representation deliberately parses explicit bytes instead of using C
//! bitfields or packed references.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

pub const CARP_VERSION: u8 = 2;
pub const CARP_ADVERTISEMENT: u8 = 1;
pub const CARP_DFLTTL: u8 = 255;
pub const CARP_KEY_LEN: usize = 20;
pub const CARP_DFLTINTV: u8 = 1;
pub const CARP_AUTHLEN: u8 = 7;
pub const CARP_HEADER_LEN: usize = 36;
pub const CARP_DIGEST_LEN: usize = 20;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum HmacMode {
    Orig,
    NoV6LinkLocal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CarpHeader {
    pub version: u8,
    pub packet_type: u8,
    pub vhid: u8,
    pub advskew: u8,
    pub authlen: u8,
    pub demote: u8,
    pub advbase: u8,
    pub checksum: u16,
    pub counter: [u8; 8],
    pub digest: [u8; CARP_DIGEST_LEN],
}

impl CarpHeader {
    pub fn advertisement(
        vhid: u8,
        advskew: u8,
        demote: u8,
        advbase: u8,
        counter: [u8; 8],
        digest: [u8; CARP_DIGEST_LEN],
    ) -> Self {
        Self {
            version: CARP_VERSION,
            packet_type: CARP_ADVERTISEMENT,
            vhid,
            advskew,
            authlen: CARP_AUTHLEN,
            demote,
            advbase,
            checksum: 0,
            counter,
            digest,
        }
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() < CARP_HEADER_LEN {
            return Err(ParseError::Short {
                got: bytes.len(),
                want: CARP_HEADER_LEN,
            });
        }

        let mut counter = [0u8; 8];
        counter.copy_from_slice(&bytes[8..16]);
        let mut digest = [0u8; CARP_DIGEST_LEN];
        digest.copy_from_slice(&bytes[16..36]);

        Ok(Self {
            version: bytes[0] >> 4,
            packet_type: bytes[0] & 0x0f,
            vhid: bytes[1],
            advskew: bytes[2],
            authlen: bytes[3],
            demote: bytes[4],
            advbase: bytes[5],
            checksum: u16::from_be_bytes([bytes[6], bytes[7]]),
            counter,
            digest,
        })
    }

    pub fn encode(&self) -> [u8; CARP_HEADER_LEN] {
        let mut bytes = [0u8; CARP_HEADER_LEN];
        bytes[0] = ((self.version & 0x0f) << 4) | (self.packet_type & 0x0f);
        bytes[1] = self.vhid;
        bytes[2] = self.advskew;
        bytes[3] = self.authlen;
        bytes[4] = self.demote;
        bytes[5] = self.advbase;
        bytes[6..8].copy_from_slice(&self.checksum.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.counter);
        bytes[16..36].copy_from_slice(&self.digest);
        bytes
    }

    pub fn encode_for_checksum(&self) -> [u8; CARP_HEADER_LEN] {
        let mut h = self.clone();
        h.checksum = 0;
        h.encode()
    }

    pub fn with_computed_checksum(mut self) -> Self {
        self.checksum = internet_checksum(&self.encode_for_checksum());
        self
    }

    pub fn verify_checksum(bytes: &[u8]) -> bool {
        bytes.len() >= CARP_HEADER_LEN && internet_checksum(&bytes[..CARP_HEADER_LEN]) == 0
    }

    pub fn validate_basic(&self) -> Result<(), ParseError> {
        if self.version != CARP_VERSION {
            return Err(ParseError::UnsupportedVersion(self.version));
        }
        if self.packet_type != CARP_ADVERTISEMENT {
            return Err(ParseError::UnsupportedType(self.packet_type));
        }
        if self.authlen != CARP_AUTHLEN {
            return Err(ParseError::BadAuthLen(self.authlen));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    Short { got: usize, want: usize },
    UnsupportedVersion(u8),
    UnsupportedType(u8),
    BadAuthLen(u8),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Short { got, want } => {
                write!(f, "short CARP header: got {got}, want {want}")
            }
            ParseError::UnsupportedVersion(v) => write!(f, "unsupported CARP version {v}"),
            ParseError::UnsupportedType(t) => write!(f, "unsupported CARP packet type {t}"),
            ParseError::BadAuthLen(n) => write!(f, "unsupported CARP authlen {n}"),
        }
    }
}

impl std::error::Error for ParseError {}

pub fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    if let Some(&last) = chunks.remainder().first() {
        sum += (last as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn passphrase_to_key(passphrase: &str) -> [u8; CARP_KEY_LEN] {
    let mut key = [0u8; CARP_KEY_LEN];
    let raw = passphrase.as_bytes();
    let len = raw.len().min(CARP_KEY_LEN - 1);
    key[..len].copy_from_slice(&raw[..len]);
    key
}

pub fn carp_hmac_sha1(
    key: &[u8; CARP_KEY_LEN],
    vhid: u8,
    vips: &[Ipv4Addr],
    counter: &[u8; 8],
) -> [u8; CARP_DIGEST_LEN] {
    carp_hmac_sha1_mixed(key, vhid, vips, &[], counter, HmacMode::NoV6LinkLocal)
}

pub fn carp_hmac_sha1_mixed(
    key: &[u8; CARP_KEY_LEN],
    vhid: u8,
    ipv4_vips: &[Ipv4Addr],
    ipv6_vips: &[Ipv6Addr],
    counter: &[u8; 8],
    mode: HmacMode,
) -> [u8; CARP_DIGEST_LEN] {
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..CARP_KEY_LEN {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }

    let mut inner =
        Vec::with_capacity(64 + 3 + ipv4_vips.len() * 4 + ipv6_vips.len() * 16 + counter.len());
    inner.extend_from_slice(&ipad);
    inner.push(CARP_VERSION);
    inner.push(CARP_ADVERTISEMENT);
    inner.push(vhid);

    let mut sorted4 = ipv4_vips.to_vec();
    sorted4.sort_by_key(|addr| u32::from(*addr));
    for addr in sorted4 {
        inner.extend_from_slice(&addr.octets());
    }

    let mut sorted6 = Vec::with_capacity(ipv6_vips.len());
    for addr in ipv6_vips {
        if let Some(addr) = hmac_ipv6_addr(*addr, mode) {
            sorted6.push(addr);
        }
    }
    sorted6.sort_by_key(|addr| addr.octets());
    for addr in sorted6 {
        inner.extend_from_slice(&addr.octets());
    }

    inner.extend_from_slice(counter);
    let inner_digest = sha1(&inner);

    let mut outer = Vec::with_capacity(64 + CARP_DIGEST_LEN);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_digest);
    sha1(&outer)
}

fn hmac_ipv6_addr(addr: Ipv6Addr, mode: HmacMode) -> Option<Ipv6Addr> {
    let mut octets = addr.octets();
    if is_scope_embedded_ipv6(&octets) {
        if mode == HmacMode::NoV6LinkLocal {
            return None;
        }
        octets[2] = 0;
        octets[3] = 0;
    }
    Some(Ipv6Addr::from(octets))
}

fn is_scope_embedded_ipv6(octets: &[u8; 16]) -> bool {
    octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80 && (octets[2] != 0 || octets[3] != 0)
}

pub fn verify_digest(expected: &[u8; CARP_DIGEST_LEN], got: &[u8; CARP_DIGEST_LEN]) -> bool {
    let mut diff = 0u8;
    for i in 0..CARP_DIGEST_LEN {
        diff |= expected[i] ^ got[i];
    }
    diff == 0
}

pub fn sha1(input: &[u8]) -> [u8; 20] {
    let mut msg = input.to_vec();
    let bit_len = (msg.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut h0 = 0x67452301u32;
    let mut h1 = 0xefcdab89u32;
    let mut h2 = 0x98badcfeu32;
    let mut h3 = 0x10325476u32;
    let mut h4 = 0xc3d2e1f0u32;

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let j = i * 4;
            *word = u32::from_be_bytes([chunk[j], chunk[j + 1], chunk[j + 2], chunk[j + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;

        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        const TABLE: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            out.push(TABLE[(b >> 4) as usize] as char);
            out.push(TABLE[(b & 0x0f) as usize] as char);
        }
        out
    }

    #[test]
    fn header_roundtrips_explicit_wire_nibbles() {
        let h = CarpHeader::advertisement(7, 42, 3, 1, [1, 2, 3, 4, 5, 6, 7, 8], [9; 20]);
        let bytes = h.encode();
        assert_eq!(bytes[0], 0x21);
        let parsed = CarpHeader::parse(&bytes).unwrap();
        assert_eq!(parsed, h);
        parsed.validate_basic().unwrap();
    }

    #[test]
    fn checksum_roundtrip_verifies_to_zero() {
        let h =
            CarpHeader::advertisement(1, 100, 0, 1, [0xaa; 8], [0xbb; 20]).with_computed_checksum();
        let bytes = h.encode();
        assert_ne!(h.checksum, 0);
        assert!(CarpHeader::verify_checksum(&bytes));
    }

    #[test]
    fn sha1_known_vector() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn passphrase_key_matches_openbsd_ifconfig_copy() {
        let key = passphrase_to_key("interop-pass");
        assert_eq!(&key[..12], b"interop-pass");
        assert_eq!(key[12], 0);

        let long = passphrase_to_key("1234567890123456789012345");
        assert_eq!(&long, b"1234567890123456789\0");
    }

    #[test]
    fn hmac_uses_sorted_ipv4_canonical_order() {
        let key: [u8; CARP_KEY_LEN] = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ];
        let counter = [0, 1, 2, 3, 4, 5, 6, 7];
        let unsorted = [
            Ipv4Addr::new(192, 0, 2, 10),
            Ipv4Addr::new(10, 0, 0, 9),
            Ipv4Addr::new(10, 0, 0, 1),
        ];
        let sorted = [
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(10, 0, 0, 9),
            Ipv4Addr::new(192, 0, 2, 10),
        ];
        let digest = carp_hmac_sha1(&key, 7, &unsorted, &counter);
        assert_eq!(digest, carp_hmac_sha1(&key, 7, &sorted, &counter));
        assert_eq!(hex(&digest), "9c40a9b81097cdc55868afcef20fdb5415c55709");
    }

    #[test]
    fn hmac_uses_sorted_ipv6_canonical_order() {
        let key = [0x11u8; CARP_KEY_LEN];
        let counter = [0, 1, 2, 3, 4, 5, 6, 7];
        let unsorted = [
            "2001:db8::2".parse::<Ipv6Addr>().unwrap(),
            "2001:db8::1".parse::<Ipv6Addr>().unwrap(),
        ];
        let sorted = [
            "2001:db8::1".parse::<Ipv6Addr>().unwrap(),
            "2001:db8::2".parse::<Ipv6Addr>().unwrap(),
        ];
        assert_eq!(
            carp_hmac_sha1_mixed(&key, 7, &[], &unsorted, &counter, HmacMode::NoV6LinkLocal),
            carp_hmac_sha1_mixed(&key, 7, &[], &sorted, &counter, HmacMode::NoV6LinkLocal)
        );
    }

    #[test]
    fn hmac_ipv6_scope_modes_match_openbsd_shape() {
        let scoped = Ipv6Addr::from([0xfe, 0x80, 0x12, 0x34, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(
            hmac_ipv6_addr(scoped, HmacMode::Orig),
            Some("fe80::1".parse::<Ipv6Addr>().unwrap())
        );
        assert_eq!(hmac_ipv6_addr(scoped, HmacMode::NoV6LinkLocal), None);
    }

    #[test]
    fn digest_compare_is_constant_time_shape() {
        let a = [1u8; CARP_DIGEST_LEN];
        let mut b = a;
        assert!(verify_digest(&a, &b));
        b[19] = 2;
        assert!(!verify_digest(&a, &b));
    }
}
