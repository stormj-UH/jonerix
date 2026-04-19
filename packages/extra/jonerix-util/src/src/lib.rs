//! jonerix-util — permissive (MIT) clean-room replacement for the GPL
//! util-linux suite. Behaviour reverse-engineered from the kernel UAPI
//! headers, /proc & /sys pseudo-filesystems, published man pages, and
//! observed output of real binaries on reference hosts. No upstream
//! source code consulted.
//!
//! This library module provides the common building blocks shared by
//! every binary in the crate: raw syscall wrappers, /proc parsing
//! helpers, and small formatting utilities. Keeping everything in one
//! crate avoids a dependency graph while still giving each binary its
//! own entry point.

#![allow(dead_code)]

pub mod proc;
pub mod sysfs;
pub mod syscall;
pub mod table;

use std::ffi::CString;
use std::io;
use std::os::unix::io::RawFd;

/// Thin wrapper around libc-free open(2): avoids pulling `libc` crate
/// by using the `nix`-free direct syscall path. Returns -1-mapped
/// io::Error on failure.
pub fn open_rdonly(path: &str) -> io::Result<RawFd> {
    let c = CString::new(path).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
    // SAFETY: direct syscall; O_RDONLY=0, O_CLOEXEC=0x80000 on linux
    let fd = unsafe { syscall::open(c.as_ptr(), 0 | 0x80000, 0) };
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(fd as RawFd) }
}

/// Small helper that strips trailing whitespace/newline from a string.
pub fn chomp(s: &str) -> &str {
    s.trim_end_matches(|c: char| c == '\n' || c == '\r' || c == ' ' || c == '\t')
}
