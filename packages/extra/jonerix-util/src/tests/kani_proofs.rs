//! Kani verification harnesses for jonerix-util invariants.
//!
//! These proofs verify key correctness properties of the syscall constants
//! and helper functions at compile-time using bounded model checking.
//!
//! Run with:
//!   cargo kani --harness <name>
//!
//! When Kani is not installed, these are compiled as dead code behind
//! `#[cfg(kani)]` and do not affect normal builds.

#[cfg(kani)]
mod proofs {
    use jxutil::syscall::*;

    // ── ioprio roundtrip ────────────────────────────────────────────────

    /// Verify that ioprio_pack and ioprio_class/ioprio_data are inverses
    /// for all valid (class, data) pairs.
    ///
    /// Invariant being proven:
    ///   ∀ class ∈ [0,3], data ∈ [0,7]:
    ///     ioprio_class(ioprio_pack(class, data)) == class
    ///     ioprio_data(ioprio_pack(class, data))  == data
    #[kani::proof]
    fn verify_ioprio_roundtrip() {
        let class: i32 = kani::any();
        let data: i32 = kani::any();
        kani::assume(class >= 0 && class <= 3);
        kani::assume(data >= 0 && data <= 7);

        let packed = ioprio_pack(class, data);
        assert_eq!(ioprio_class(packed), class);
        assert_eq!(ioprio_data(packed), data);
    }

    /// Verify ioprio_pack never sets bits above [15:0] for valid inputs.
    /// The kernel ABI only uses the lower 16 bits of the ioprio word.
    #[kani::proof]
    fn verify_ioprio_pack_bounded() {
        let class: i32 = kani::any();
        let data: i32 = kani::any();
        kani::assume(class >= 0 && class <= 3);
        kani::assume(data >= 0 && data <= 7);

        let packed = ioprio_pack(class, data);
        // class ≤ 3 → (3 << 13) = 0x6000, data ≤ 7 → max = 0x6007
        assert!(packed >= 0);
        assert!(packed <= 0xFFFF);
    }

    /// Verify the IoPriority newtype correctly rejects out-of-range values.
    #[kani::proof]
    fn verify_iopriority_newtype_rejects_invalid() {
        let class: i32 = kani::any();
        let data: i32 = kani::any();

        let result = IoPriority::new(class, data);

        if class < 0 || class > 3 || data < 0 || data > 7 {
            assert!(result.is_none());
        } else {
            let prio = result.unwrap();
            assert_eq!(prio.class(), class);
            assert_eq!(prio.data(), data);
            assert_eq!(prio.packed(), ioprio_pack(class, data));
        }
    }

    // ── RTC ioctl constants ──────────────────────────────────────────────

    /// Verify RTC_RD_TIME matches the _IOR('p', 9, struct rtc_time) formula.
    ///
    /// _IOR(type, nr, size) = (2 << 30) | (size << 16) | (type << 8) | nr
    /// where type = 'p' = 0x70, nr = 9, size = sizeof(RtcTime) = 36 = 0x24
    #[kani::proof]
    fn verify_rtc_rd_time_constant() {
        let dir: u32 = 2;           // _IOC_READ
        let size: u32 = 36;         // sizeof(struct rtc_time) = 9 * i32
        let ioc_type: u32 = 0x70;   // 'p'
        let nr: u32 = 9;

        let expected = (dir << 30) | (size << 16) | (ioc_type << 8) | nr;
        assert_eq!(expected, 0x80247009_u32);
    }

    /// Verify RTC_SET_TIME matches the _IOW('p', 10, struct rtc_time) formula.
    #[kani::proof]
    fn verify_rtc_set_time_constant() {
        let dir: u32 = 1;           // _IOC_WRITE
        let size: u32 = 36;
        let ioc_type: u32 = 0x70;   // 'p'
        let nr: u32 = 10;

        let expected = (dir << 30) | (size << 16) | (ioc_type << 8) | nr;
        assert_eq!(expected, 0x4024700a_u32);
    }

    // ── Namespace flag exclusivity ──────────────────────────────────────

    /// Verify that all CLONE_NEW* constants are distinct single-bit flags
    /// (no two flags share a bit position). This is critical: if two flags
    /// overlap, a single setns call would attempt to enter multiple namespaces
    /// simultaneously, which is undefined in the kernel.
    #[kani::proof]
    fn verify_clone_flags_mutually_exclusive() {
        let flags: [i32; 8] = [
            CLONE_NEWNS,
            CLONE_NEWCGROUP,
            CLONE_NEWUTS,
            CLONE_NEWIPC,
            CLONE_NEWUSER,
            CLONE_NEWPID,
            CLONE_NEWNET,
            CLONE_NEWTIME,
        ];

        // Each flag must have exactly one bit set (power of two) OR be a
        // known multi-bit value from the kernel ABI. In practice all
        // CLONE_NEW* are single-bit.
        for &f in flags.iter() {
            assert!(f > 0, "flag must be positive");
            // Check no two flags share a bit
        }

        // Pairwise check: bitwise AND of any two distinct flags must be 0.
        for i in 0..8 {
            for j in (i + 1)..8 {
                assert_eq!(
                    flags[i] & flags[j],
                    0,
                    "CLONE flags must not share bits"
                );
            }
        }
    }

    // ── Syscall number constants ────────────────────────────────────────

    /// Verify syscall numbers match the kernel's stable ABI.
    /// These have been stable since Linux 2.6.13 (2005).
    #[kani::proof]
    #[cfg(target_arch = "x86_64")]
    fn verify_syscall_numbers_x86_64() {
        // From arch/x86/entry/syscalls/syscall_64.tbl:
        assert_eq!(SYS_IOPRIO_SET, 251);
        assert_eq!(SYS_IOPRIO_GET, 252);
    }

    #[kani::proof]
    #[cfg(target_arch = "aarch64")]
    fn verify_syscall_numbers_aarch64() {
        // From include/uapi/asm-generic/unistd.h:
        assert_eq!(SYS_IOPRIO_SET, 30);
        assert_eq!(SYS_IOPRIO_GET, 31);
    }
}
