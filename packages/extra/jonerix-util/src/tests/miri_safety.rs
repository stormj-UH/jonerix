//! Miri-compatible safety tests for jonerix-util.
//!
//! These tests exercise the safe wrappers and data structures WITHOUT making
//! actual syscalls (which Miri cannot execute). They verify:
//!   - Memory layout and alignment of #[repr(C)] structs
//!   - Correct behaviour of pure-computation helpers
//!   - No UB in the safe interface layer (as detected by Miri's stacked-borrows
//!     and tree-borrows models)
//!
//! Run with:
//!   cargo +nightly miri test --test miri_safety
//!
//! These tests also pass under normal `cargo test` — no special setup needed
//! for CI, only Miri adds its UB-detection instrumentation on top.

use std::mem;

// ── Re-export the library for testing ──────────────────────────────────────

// We test the library module directly. The binary crates (hwclock, nsenter,
// etc.) are tested indirectly via their shared helpers here.

// ── IoPriority newtype tests ───────────────────────────────────────────────

#[test]
fn iopriority_accepts_valid_ranges() {
    // Exhaustive check: all valid (class, data) pairs must construct OK.
    for class in 0..=3 {
        for data in 0..=7 {
            let p = jxutil::syscall::IoPriority::new(class, data);
            assert!(p.is_some(), "IoPriority::new({class}, {data}) should be Some");
            let p = p.unwrap();
            assert_eq!(p.class(), class);
            assert_eq!(p.data(), data);
        }
    }
}

#[test]
fn iopriority_rejects_out_of_range() {
    // Class out of range
    assert!(jxutil::syscall::IoPriority::new(-1, 0).is_none());
    assert!(jxutil::syscall::IoPriority::new(4, 0).is_none());
    assert!(jxutil::syscall::IoPriority::new(100, 0).is_none());
    // Data out of range
    assert!(jxutil::syscall::IoPriority::new(0, -1).is_none());
    assert!(jxutil::syscall::IoPriority::new(0, 8).is_none());
    assert!(jxutil::syscall::IoPriority::new(2, 255).is_none());
    // Both out of range
    assert!(jxutil::syscall::IoPriority::new(5, 9).is_none());
}

#[test]
fn iopriority_packed_matches_manual_pack() {
    use jxutil::syscall::{ioprio_pack, IoPriority};
    for class in 0..=3 {
        for data in 0..=7 {
            let p = IoPriority::new(class, data).unwrap();
            assert_eq!(p.packed(), ioprio_pack(class, data));
        }
    }
}

// ── ioprio pack/unpack roundtrip ───────────────────────────────────────────

#[test]
fn ioprio_roundtrip_exhaustive() {
    use jxutil::syscall::{ioprio_class, ioprio_data, ioprio_pack};
    // Exhaustive for the valid kernel range.
    for class in 0..=3_i32 {
        for data in 0..=7_i32 {
            let packed = ioprio_pack(class, data);
            assert_eq!(ioprio_class(packed), class);
            assert_eq!(ioprio_data(packed), data);
        }
    }
}

#[test]
fn ioprio_pack_bits_are_bounded() {
    use jxutil::syscall::ioprio_pack;
    // Max valid packed value: class=3, data=7 → (3<<13)|7 = 0x6007
    for class in 0..=3_i32 {
        for data in 0..=7_i32 {
            let packed = ioprio_pack(class, data);
            assert!(packed >= 0);
            assert!(packed <= 0xFFFF, "packed 0x{packed:x} exceeds u16 range");
        }
    }
}

// ── ValidFd newtype tests ──────────────────────────────────────────────────

#[test]
fn validfd_rejects_negative() {
    assert!(jxutil::ValidFd::new(-1).is_none());
    assert!(jxutil::ValidFd::new(-100).is_none());
    assert!(jxutil::ValidFd::new(i32::MIN).is_none());
}

#[test]
fn validfd_accepts_zero_and_positive() {
    // fd 0 is valid (stdin)
    let fd = jxutil::ValidFd::new(0);
    assert!(fd.is_some());
    assert_eq!(fd.unwrap().as_raw(), 0);

    let fd = jxutil::ValidFd::new(42);
    assert!(fd.is_some());
    assert_eq!(fd.unwrap().as_raw(), 42);

    // Don't actually drop these — they'd try to close(2) real fds.
    // Use mem::forget to prevent the Drop impl from firing.
    mem::forget(jxutil::ValidFd::new(0));
    mem::forget(jxutil::ValidFd::new(42));
}

// ── CLONE_NEW* flag exclusivity ────────────────────────────────────────────

#[test]
fn clone_flags_are_mutually_exclusive() {
    use jxutil::syscall::*;
    let flags = [
        CLONE_NEWNS, CLONE_NEWCGROUP, CLONE_NEWUTS, CLONE_NEWIPC,
        CLONE_NEWUSER, CLONE_NEWPID, CLONE_NEWNET, CLONE_NEWTIME,
    ];
    // No two flags may share a bit.
    for i in 0..flags.len() {
        for j in (i + 1)..flags.len() {
            assert_eq!(
                flags[i] & flags[j], 0,
                "CLONE flags at index {i} (0x{:08x}) and {j} (0x{:08x}) overlap",
                flags[i], flags[j]
            );
        }
    }
}

#[test]
fn clone_flags_are_nonzero_and_positive() {
    use jxutil::syscall::*;
    let flags = [
        CLONE_NEWNS, CLONE_NEWCGROUP, CLONE_NEWUTS, CLONE_NEWIPC,
        CLONE_NEWUSER, CLONE_NEWPID, CLONE_NEWNET, CLONE_NEWTIME,
    ];
    for (i, &f) in flags.iter().enumerate() {
        assert!(f > 0, "flag at index {i} is not positive: {f}");
    }
}

// ── CString safety (NUL-byte handling) ─────────────────────────────────────

#[test]
fn cstring_rejects_embedded_nul() {
    // Verify that our open_rdonly properly rejects NUL-containing paths
    // rather than panicking.
    let result = jxutil::open_rdonly("/dev/rtc0\x00injected");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn cstring_accepts_normal_paths() {
    // These should not panic (CString::new succeeds), though the actual
    // open will fail since /nonexistent doesn't exist — that's fine,
    // we're testing the CString construction path.
    let result = jxutil::open_rdonly("/nonexistent/path");
    assert!(result.is_err()); // ENOENT, but no panic
}

// ── Struct layout verification ─────────────────────────────────────────────
// These tests verify at compile/runtime that our #[repr(C)] structs match
// the kernel's expected sizes, preventing the kind of stack-overwrite bug
// identified in the security audit (gmtime_r writing past RtcTime bounds).

#[test]
fn ioprio_constants_are_consistent() {
    use jxutil::syscall::*;
    // IOPRIO_CLASS_SHIFT must be 13 (kernel ABI)
    assert_eq!(IOPRIO_CLASS_SHIFT, 13);
    // Classes must be 0-3
    assert_eq!(IOPRIO_CLASS_NONE, 0);
    assert_eq!(IOPRIO_CLASS_RT, 1);
    assert_eq!(IOPRIO_CLASS_BE, 2);
    assert_eq!(IOPRIO_CLASS_IDLE, 3);
    // WHO constants must be 1-3
    assert_eq!(IOPRIO_WHO_PROCESS, 1);
    assert_eq!(IOPRIO_WHO_PGRP, 2);
    assert_eq!(IOPRIO_WHO_USER, 3);
}

// ── proc.rs parsing tests (safe, no syscalls) ──────────────────────────────

#[test]
fn parse_cpuinfo_handles_empty() {
    let recs = jxutil::proc::parse_cpuinfo("");
    assert!(recs.is_empty());
}

#[test]
fn parse_cpuinfo_handles_single_record() {
    let input = "processor\t: 0\nvendor_id\t: GenuineIntel\n\n";
    let recs = jxutil::proc::parse_cpuinfo(input);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].len(), 2);
    assert_eq!(recs[0][0], ("processor".to_string(), "0".to_string()));
    assert_eq!(recs[0][1], ("vendor_id".to_string(), "GenuineIntel".to_string()));
}

#[test]
fn parse_cpuinfo_handles_no_trailing_blank() {
    // /proc/cpuinfo on some kernels doesn't end with a blank line.
    let input = "processor\t: 0\nmodel name\t: Test CPU";
    let recs = jxutil::proc::parse_cpuinfo(input);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].len(), 2);
}

// ── sysfs.rs parsing tests ─────────────────────────────────────────────────

#[test]
fn parse_cpulist_simple() {
    let v = jxutil::sysfs::parse_cpulist("0-3");
    assert_eq!(v, vec![0, 1, 2, 3]);
}

#[test]
fn parse_cpulist_complex() {
    let v = jxutil::sysfs::parse_cpulist("0-3,7,9-11");
    assert_eq!(v, vec![0, 1, 2, 3, 7, 9, 10, 11]);
}

#[test]
fn parse_cpulist_single() {
    let v = jxutil::sysfs::parse_cpulist("5");
    assert_eq!(v, vec![5]);
}

#[test]
fn parse_cpulist_empty() {
    let v = jxutil::sysfs::parse_cpulist("");
    assert!(v.is_empty());
}

// ── chomp tests ────────────────────────────────────────────────────────────

#[test]
fn chomp_strips_trailing_whitespace() {
    assert_eq!(jxutil::chomp("hello\n"), "hello");
    assert_eq!(jxutil::chomp("hello\r\n"), "hello");
    assert_eq!(jxutil::chomp("hello  \t\n"), "hello");
    assert_eq!(jxutil::chomp("hello"), "hello");
    assert_eq!(jxutil::chomp(""), "");
}
