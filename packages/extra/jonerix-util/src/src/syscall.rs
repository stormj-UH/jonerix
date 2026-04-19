//! Direct Linux syscall wrappers. We avoid the `libc` crate to keep
//! the dependency graph zero and because util-linux-style tools need a
//! handful of niche syscalls (setns, ioprio_get/set, settimeofday)
//! that aren't always in stable `std`.
//!
//! All functions return the raw kernel return value. Negative values
//! are `-errno`; callers should convert with `io::Error::from_raw_os_error`.

use std::os::raw::{c_char, c_int, c_long, c_void};

// ── errno ---------------------------------------------------------

extern "C" {
    #[cfg_attr(any(target_os = "linux", target_os = "android"), link_name = "__errno_location")]
    fn errno_location() -> *mut c_int;
}

pub fn errno() -> i32 {
    unsafe { *errno_location() }
}

// ── Minimal libc-style bindings ----------------------------------
// We use the C library for syscall-wrapping helpers only; Rust std
// already links it so this costs nothing and side-steps arch-specific
// syscall-number tables.

extern "C" {
    pub fn open(path: *const c_char, flags: c_int, mode: c_int) -> c_int;
    pub fn close(fd: c_int) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    pub fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    pub fn ioctl(fd: c_int, request: c_long, ...) -> c_int;
    pub fn syscall(num: c_long, ...) -> c_long;
    pub fn getpid() -> c_int;
    pub fn execvp(file: *const c_char, argv: *const *const c_char) -> c_int;
    pub fn fork() -> c_int;
    pub fn waitpid(pid: c_int, wstatus: *mut c_int, options: c_int) -> c_int;
    pub fn setns(fd: c_int, nstype: c_int) -> c_int;
}

// Syscall numbers for ioprio_{get,set}. These live in
// arch/<arch>/include/uapi/asm/unistd*.h in the kernel tree; listed
// here from the stable kernel ABI which hasn't shifted in 15 years.
#[cfg(target_arch = "x86_64")]
pub const SYS_IOPRIO_SET: c_long = 251;
#[cfg(target_arch = "x86_64")]
pub const SYS_IOPRIO_GET: c_long = 252;

#[cfg(target_arch = "aarch64")]
pub const SYS_IOPRIO_SET: c_long = 30;
#[cfg(target_arch = "aarch64")]
pub const SYS_IOPRIO_GET: c_long = 31;

pub fn ioprio_set(which: i32, who: i32, ioprio: i32) -> c_long {
    unsafe { syscall(SYS_IOPRIO_SET, which, who, ioprio) }
}

pub fn ioprio_get(which: i32, who: i32) -> c_long {
    unsafe { syscall(SYS_IOPRIO_GET, which, who) }
}

// ── ioprio ABI constants (linux/ioprio.h; kernel UAPI) -----------
pub const IOPRIO_WHO_PROCESS: i32 = 1;
pub const IOPRIO_WHO_PGRP: i32    = 2;
pub const IOPRIO_WHO_USER: i32    = 3;

pub const IOPRIO_CLASS_NONE: i32 = 0;
pub const IOPRIO_CLASS_RT: i32   = 1;
pub const IOPRIO_CLASS_BE: i32   = 2;
pub const IOPRIO_CLASS_IDLE: i32 = 3;

pub const IOPRIO_CLASS_SHIFT: i32 = 13;

pub fn ioprio_pack(class: i32, data: i32) -> i32 { (class << IOPRIO_CLASS_SHIFT) | data }
pub fn ioprio_class(v: i32) -> i32 { v >> IOPRIO_CLASS_SHIFT }
pub fn ioprio_data(v: i32)  -> i32 { v & ((1 << IOPRIO_CLASS_SHIFT) - 1) }

// ── namespace types (linux/sched.h UAPI) -------------------------
pub const CLONE_NEWNS:    i32 = 0x00020000;
pub const CLONE_NEWCGROUP:i32 = 0x02000000;
pub const CLONE_NEWUTS:   i32 = 0x04000000;
pub const CLONE_NEWIPC:   i32 = 0x08000000;
pub const CLONE_NEWUSER:  i32 = 0x10000000;
pub const CLONE_NEWPID:   i32 = 0x20000000;
pub const CLONE_NEWNET:   i32 = 0x40000000;
pub const CLONE_NEWTIME:  i32 = 0x00000080;
