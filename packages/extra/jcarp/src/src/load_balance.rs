//! CARP load-sharing policy helpers.
//!
//! OpenBSD performs the final accept/drop decision in the kernel datapath. This
//! module keeps the same hash/mask policy available to the jcarp daemon and to
//! a future Linux ingress hook backend.

use std::net::{Ipv4Addr, Ipv6Addr};

use crate::state::CarpState;

pub fn master_mask(states: &[CarpState]) -> u32 {
    let mut mask = 0u32;
    for (idx, state) in states.iter().take(32).enumerate() {
        if *state == CarpState::Master {
            mask |= 1 << idx;
        }
    }
    mask
}

pub fn accepts_ipv4_flow(
    src: Ipv4Addr,
    dst: Ipv4Addr,
    node_count: usize,
    master_mask: u32,
) -> bool {
    accepts_folded_flow(fold_ipv4(src, dst), node_count, master_mask)
}

pub fn accepts_ipv6_flow(
    src: Ipv6Addr,
    dst: Ipv6Addr,
    node_count: usize,
    master_mask: u32,
) -> bool {
    accepts_folded_flow(fold_ipv6(src, dst), node_count, master_mask)
}

fn accepts_folded_flow(fold: u32, node_count: usize, master_mask: u32) -> bool {
    if node_count == 0 {
        return false;
    }
    let slot = fold % node_count.min(32) as u32;
    (master_mask & (1 << slot)) != 0
}

fn fold_ipv4(src: Ipv4Addr, dst: Ipv4Addr) -> u32 {
    u32::from_be_bytes(src.octets()) ^ u32::from_be_bytes(dst.octets())
}

fn fold_ipv6(src: Ipv6Addr, dst: Ipv6Addr) -> u32 {
    let src = src.octets();
    let dst = dst.octets();
    let mut fold = 0u32;
    for idx in 0..4 {
        let off = idx * 4;
        fold ^= u32::from_be_bytes([src[off], src[off + 1], src[off + 2], src[off + 3]]);
        fold ^= u32::from_be_bytes([dst[off], dst[off + 1], dst[off + 2], dst[off + 3]]);
    }
    fold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_mask_tracks_openbsd_lsmask_order() {
        assert_eq!(
            master_mask(&[
                CarpState::Master,
                CarpState::Backup,
                CarpState::Master,
                CarpState::Init,
            ]),
            0b0101
        );
    }

    #[test]
    fn ipv4_flow_acceptance_uses_fold_mod_node_count() {
        let src = Ipv4Addr::new(10, 0, 0, 1);
        let dst = Ipv4Addr::new(10, 0, 0, 3);
        let fold = fold_ipv4(src, dst);
        assert_eq!(fold % 2, 0);
        assert!(accepts_ipv4_flow(src, dst, 2, 0b01));
        assert!(!accepts_ipv4_flow(src, dst, 2, 0b10));
    }

    #[test]
    fn ipv6_flow_acceptance_folds_all_words() {
        let src = "2001:db8::1".parse().unwrap();
        let dst = "2001:db8::2".parse().unwrap();
        let fold = fold_ipv6(src, dst);
        assert_eq!(fold, 3);
        assert!(accepts_ipv6_flow(src, dst, 4, 0b1000));
        assert!(!accepts_ipv6_flow(src, dst, 4, 0b0010));
    }
}
