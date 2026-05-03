//! jcarp - Rust OpenBSD-CARP-compatible failover daemon.
//!
//! OpenBSD's CARP implementation is BSD-licensed and compatible with jonerix.
//! This crate ports the wire format and state semantics while keeping Linux
//! raw socket and future netlink integration isolated from the protocol core.

pub mod config;
pub mod daemon;
pub mod dataplane;
pub mod iface;
pub mod io;
pub mod load_balance;
pub mod proto;
pub mod state;
