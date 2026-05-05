//! Direct Linux syscall wrappers. We avoid the `libc` crate to keep
//! the dependency graph zero and because util-linux-style tools need a
//! handful of niche syscalls (setns, ioprio_get/set, settimeofday)
//! that aren't always in stable `std`.
//!
//! All functions return the raw kernel return value. Negative values
//! are `-errno`; callers should convert with `io::Error::from_raw_os_error`.

// Every unsafe *operation* inside an unsafe *fn* must be wrapped in its
// own `unsafe {}` block with a SAFETY comment.  This catches "unsafe
// creep" where an unsafe fn silently authorises every line it contains.
#![deny(unsafe_op_in_unsafe_fn)]

use std::os::raw::{c_char, c_int, c_long, c_void};

// ── errno -----------------------------------------------------------------

extern "C" {
    // SAFETY contract for callers of errno_location():
    //   - The function is part of the C runtime (musl/glibc) which is always
    //     linked into Rust binaries on Linux.
    //   - It returns a thread-local pointer that is always non-null and
    //     valid for the lifetime of the calling thread.
    //   - The pointed-to i32 is correctly aligned (guaranteed by the C ABI).
    #[cfg_attr(any(target_os = "linux", target_os = "android"), link_name = "__errno_location")]
    fn errno_location() -> *mut c_int;
}

pub fn errno() -> i32 {
    // SAFETY:
    //   - `errno_location` is guaranteed by the C runtime to return a
    //     non-null, validly-aligned, thread-local `c_int *`.
    //   - We only read it (no write), so there are no aliasing concerns —
    //     no other Rust code holds a mutable reference to this location.
    //   - The dereference is an atomic-width load on all supported targets.
    unsafe { *errno_location() }
}

// ── Minimal libc-style bindings ------------------------------------------
// We use the C library for syscall-wrapping helpers only; Rust std
// already links it so this costs nothing and side-steps arch-specific
// syscall-number tables.

extern "C" {
    // SAFETY contract for open():
    //   - `path` must be a non-null pointer to a NUL-terminated byte string
    //     that remains valid for the duration of the call.
    //   - `flags` must be a valid combination of O_* constants for the
    //     running Linux kernel.
    //   - `mode` is ignored unless O_CREAT or O_TMPFILE is in `flags`.
    //   - Returns a non-negative fd on success, -1 with errno set on error.
    pub fn open(path: *const c_char, flags: c_int, mode: c_int) -> c_int;

    // SAFETY contract for close():
    //   - `fd` must be a valid open file descriptor owned by the calling
    //     process.  After close() returns (even on EINTR) the fd is invalid
    //     and must not be used again.
    pub fn close(fd: c_int) -> c_int;

    // SAFETY contract for read():
    //   - `fd` must be open for reading.
    //   - `buf` must be non-null, valid, and writable for at least `count`
    //     bytes.  The bytes written are uninitialized on partial reads.
    //   - `count` must not exceed SSIZE_MAX.
    pub fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;

    // SAFETY contract for write():
    //   - `fd` must be open for writing.
    //   - `buf` must be non-null and valid (readable) for at least `count`
    //     bytes.
    pub fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;

    // SAFETY contract for ioctl():
    //   - `fd` must be a valid file descriptor for the device that understands
    //     `request`.
    //   - The variadic argument (if present) must be of the type expected by
    //     the kernel driver for the given `request` code, and its pointer
    //     must be valid for the full transfer size encoded in `request`'s
    //     _IOC_SIZE bits.
    pub fn ioctl(fd: c_int, request: c_long, ...) -> c_int;

    // SAFETY contract for syscall():
    //   - `num` must be a valid Linux syscall number for the compilation
    //     target architecture (verified by the SYS_IOPRIO_* constants below).
    //   - Variadic arguments must match the exact ABI expected by the kernel
    //     for the named syscall (number, type, and order).
    //   - Caller is responsible for all invariants that the kernel documents
    //     for the given syscall.
    pub fn syscall(num: c_long, ...) -> c_long;

    // SAFETY contract for getpid():
    //   - No arguments; always succeeds; returns the PID of the calling
    //     process (always positive on Linux).
    pub fn getpid() -> c_int;

    // SAFETY contract for execvp():
    //   - `file` must be a non-null, NUL-terminated string naming the
    //     program to execute (resolved via PATH).
    //   - `argv` must be a non-null pointer to a NULL-terminated array of
    //     non-null, NUL-terminated C strings.
    //   - All strings and the array itself must remain valid until the
    //     kernel replaces the address space (i.e. until execvp returns on
    //     error, which is the only time it returns at all).
    //   - On success execvp does not return; on error it returns -1.
    pub fn execvp(file: *const c_char, argv: *const *const c_char) -> c_int;

    // SAFETY contract for fork():
    //   - No arguments.  Returns child PID in parent, 0 in child, -1 on error.
    //   - After fork() only async-signal-safe functions may be called in the
    //     child until exec() or _exit() is reached.
    pub fn fork() -> c_int;

    // SAFETY contract for waitpid():
    //   - `pid` must be a valid child PID (> 0), 0 (wait any in group),
    //     -1 (wait any child), or < -1 (wait group |pid|).
    //   - `wstatus` must be null or a non-null, validly-aligned *mut i32
    //     writable by the caller.
    //   - `options` must be 0 or a valid combination of WNOHANG/WUNTRACED.
    pub fn waitpid(pid: c_int, wstatus: *mut c_int, options: c_int) -> c_int;

    // SAFETY contract for setns():
    //   - `fd` must be a valid file descriptor referencing a namespace file
    //     (typically /proc/PID/ns/<name>).
    //   - `nstype` must be 0 (any) or one of the CLONE_NEW* constants that
    //     matches the namespace type of `fd`.
    //   - The caller must have the CAP_SYS_ADMIN capability (or own the
    //     namespace for user namespaces).
    pub fn setns(fd: c_int, nstype: c_int) -> c_int;
}

// ── Syscall numbers -------------------------------------------------------
//
// Syscall numbers for ioprio_{get,set}.  These live in
// arch/<arch>/include/uapi/asm/unistd*.h in the kernel tree; listed
// here from the stable kernel ABI which hasn't shifted in 15 years.
//
// Verified against:
//   x86_64: arch/x86/entry/syscalls/syscall_64.tbl  — 251 ioprio_set, 252 ioprio_get
//   aarch64: include/uapi/asm-generic/unistd.h      — 30 ioprio_set,  31 ioprio_get

#[cfg(target_arch = "x86_64")]
pub const SYS_IOPRIO_SET: c_long = 251;
#[cfg(target_arch = "x86_64")]
pub const SYS_IOPRIO_GET: c_long = 252;

#[cfg(target_arch = "aarch64")]
pub const SYS_IOPRIO_SET: c_long = 30;
#[cfg(target_arch = "aarch64")]
pub const SYS_IOPRIO_GET: c_long = 31;

pub fn ioprio_set(which: i32, who: i32, ioprio: i32) -> c_long {
    // SAFETY:
    //   - SYS_IOPRIO_SET is the correct Linux syscall number for this arch
    //     (x86_64: 251, aarch64: 30 — from stable kernel UAPI).
    //   - `which` must be one of IOPRIO_WHO_PROCESS/PGRP/USER (1-3); the
    //     kernel validates this and returns -EINVAL on bad values, so no UB.
    //   - `who` is a PID/PGID/UID; 0 means the calling process/group/user.
    //   - `ioprio` encodes class and data; the kernel ignores unknown bits.
    //   - All three arguments are plain integers passed in registers; no
    //     pointer validity concerns.
    unsafe { syscall(SYS_IOPRIO_SET, which, who, ioprio) }
}

pub fn ioprio_get(which: i32, who: i32) -> c_long {
    // SAFETY:
    //   - SYS_IOPRIO_GET is the correct Linux syscall number for this arch
    //     (x86_64: 252, aarch64: 31 — from stable kernel UAPI).
    //   - `which` and `who` are plain integers; the kernel validates their
    //     ranges and returns -EINVAL on bad values, not UB.
    //   - No pointers are passed; return value is the packed ioprio word or
    //     a negative errno.
    unsafe { syscall(SYS_IOPRIO_GET, which, who) }
}

// ── ioprio ABI constants (linux/ioprio.h; kernel UAPI) -------------------
pub const IOPRIO_WHO_PROCESS: i32 = 1;
pub const IOPRIO_WHO_PGRP: i32    = 2;
pub const IOPRIO_WHO_USER: i32    = 3;

pub const IOPRIO_CLASS_NONE: i32 = 0;
pub const IOPRIO_CLASS_RT: i32   = 1;
pub const IOPRIO_CLASS_BE: i32   = 2;
pub const IOPRIO_CLASS_IDLE: i32 = 3;

pub const IOPRIO_CLASS_SHIFT: i32 = 13;

/// Pack a class (0-3) and data level (0-7) into the kernel ioprio word.
/// The kernel ABI: bits [15:13] = class, bits [12:0] = data.
pub fn ioprio_pack(class: i32, data: i32) -> i32 { (class << IOPRIO_CLASS_SHIFT) | data }

/// Extract the class field from a kernel ioprio word (upper 3 bits above bit 13).
pub fn ioprio_class(v: i32) -> i32 { v >> IOPRIO_CLASS_SHIFT }

/// Extract the data (level) field from a kernel ioprio word (lower 13 bits).
pub fn ioprio_data(v: i32)  -> i32 { v & ((1 << IOPRIO_CLASS_SHIFT) - 1) }

// ── IoPriority newtype ----------------------------------------------------

/// A valid, range-checked I/O priority value.
///
/// Invariant: `class` is in 0..=3, `data` is in 0..=7.
/// Only constructible via [`IoPriority::new`], which enforces the invariant.
/// This prevents accidentally passing out-of-range values to `ioprio_set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoPriority {
    class: i32,
    data:  i32,
}

impl IoPriority {
    /// Construct a validated `IoPriority`.
    ///
    /// Returns `None` if `class > 3` or `data > 7`.
    pub fn new(class: i32, data: i32) -> Option<Self> {
        if class < 0 || class > 3 { return None; }
        if data  < 0 || data  > 7 { return None; }
        Some(Self { class, data })
    }

    /// Return the packed kernel word `(class << 13) | data`.
    pub fn packed(self) -> i32 { ioprio_pack(self.class, self.data) }

    pub fn class(self) -> i32 { self.class }
    pub fn data(self)  -> i32 { self.data }
}

// ── namespace types (linux/sched.h UAPI) ----------------------------------
//
// These are distinct single-bit flags that are OR'd into clone(2)/setns(2)
// calls.  They must be mutually exclusive bit positions — verified by the
// Kani proof in tests/kani_proofs.rs.
pub const CLONE_NEWNS:    i32 = 0x00020000;
pub const CLONE_NEWCGROUP:i32 = 0x02000000;
pub const CLONE_NEWUTS:   i32 = 0x04000000;
pub const CLONE_NEWIPC:   i32 = 0x08000000;
pub const CLONE_NEWUSER:  i32 = 0x10000000;
pub const CLONE_NEWPID:   i32 = 0x20000000;
pub const CLONE_NEWNET:   i32 = 0x40000000;
pub const CLONE_NEWTIME:  i32 = 0x00000080;
