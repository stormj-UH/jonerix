use std::net::Ipv4Addr;
use std::time::Duration;

use jcarp::io::{virtual_mac, CARP_IPV4_MULTICAST, CARP_TTL, IPPROTO_CARP};
use jcarp::proto::{
    carp_hmac_sha1, passphrase_to_key, verify_digest, CarpHeader, CARP_ADVERTISEMENT, CARP_AUTHLEN,
    CARP_DFLTINTV, CARP_DFLTTL, CARP_HEADER_LEN, CARP_KEY_LEN, CARP_VERSION,
};
use jcarp::state::{advertisement_interval, CarpState, Decision, LocalNode, RemoteAdvertisement};

#[test]
fn openbsd_wire_constants_match_ip_carp_h() {
    assert_eq!(CARP_VERSION, 2);
    assert_eq!(CARP_ADVERTISEMENT, 1);
    assert_eq!(CARP_DFLTTL, 255);
    assert_eq!(CARP_KEY_LEN, 20);
    assert_eq!(CARP_DFLTINTV, 1);
    assert_eq!(CARP_AUTHLEN, 7);
    assert_eq!(CARP_HEADER_LEN, 36);
    assert_eq!(IPPROTO_CARP, 112);
    assert_eq!(CARP_TTL, 255);
    assert_eq!(CARP_IPV4_MULTICAST, Ipv4Addr::new(224, 0, 0, 18));
}

#[test]
fn openbsd_header_demote_and_checksum_roundtrip() {
    let header = CarpHeader::advertisement(42, 128, 7, 1, [1, 2, 3, 4, 5, 6, 7, 8], [9; 20])
        .with_computed_checksum();
    let bytes = header.encode();
    assert_eq!(bytes[0], 0x21);
    assert_eq!(bytes[4], 7);
    assert!(CarpHeader::verify_checksum(&bytes));
    assert_eq!(CarpHeader::parse(&bytes).unwrap(), header);
}

#[test]
fn openbsd_hmac_ipv4_addresses_are_canonicalized() {
    let key = [0x11u8; CARP_KEY_LEN];
    let counter = [0xaa, 0xbb, 0xcc, 0xdd, 0, 1, 2, 3];
    let a = [
        Ipv4Addr::new(192, 0, 2, 200),
        Ipv4Addr::new(10, 0, 0, 2),
        Ipv4Addr::new(10, 0, 0, 1),
    ];
    let b = [
        Ipv4Addr::new(10, 0, 0, 1),
        Ipv4Addr::new(10, 0, 0, 2),
        Ipv4Addr::new(192, 0, 2, 200),
    ];
    assert_eq!(
        carp_hmac_sha1(&key, 9, &a, &counter),
        carp_hmac_sha1(&key, 9, &b, &counter)
    );
}

#[test]
fn openbsd_78_captured_advertisement_authenticates() {
    let bytes = [
        0x21, 0x2a, 0x64, 0x07, 0x00, 0x01, 0xd2, 0x1b, 0x98, 0xbb, 0xdb, 0x7d, 0xa8, 0xdd, 0xc1,
        0x19, 0x2c, 0xd5, 0x67, 0xa8, 0xf4, 0x3f, 0x54, 0xfe, 0x6c, 0x2a, 0xa1, 0x7c, 0xeb, 0x57,
        0xd3, 0x20, 0xef, 0xaf, 0x30, 0xf5,
    ];
    let header = CarpHeader::parse(&bytes).unwrap();

    assert!(CarpHeader::verify_checksum(&bytes));
    assert_eq!(header.vhid, 42);
    assert_eq!(header.advskew, 100);
    assert_eq!(header.advbase, 1);

    let key = passphrase_to_key("interop-pass");
    let expected = carp_hmac_sha1(
        &key,
        header.vhid,
        &[Ipv4Addr::new(10, 0, 253, 42)],
        &header.counter,
    );
    assert!(verify_digest(&expected, &header.digest));
}

#[test]
fn openbsd_election_and_timing_rules_are_preserved() {
    assert_eq!(
        advertisement_interval(1, 128),
        Duration::from_micros(1_500_000)
    );

    let mut local = LocalNode::new(1, 1, 0, 0, true);
    local.state = CarpState::Backup;
    let decision = local.observe_advertisement(RemoteAdvertisement {
        advbase: 1,
        advskew: 200,
        demote: 0,
    });
    assert_eq!(decision, Decision::BecomeMaster);
    assert_eq!(local.state, CarpState::Master);
}

#[test]
fn openbsd_virtual_mac_shape_is_vhid_based() {
    assert_eq!(virtual_mac(1), [0, 0, 0x5e, 0, 1, 1]);
    assert_eq!(virtual_mac(42), [0, 0, 0x5e, 0, 1, 42]);
}
