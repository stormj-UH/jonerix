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

// Every unsafe *operation* inside an unsafe *fn* must be wrapped in its
// own `unsafe {}` block with its own SAFETY comment, preventing "unsafe
// creep" where an unsafe fn implicitly authorises all of its body.
#![deny(unsafe_op_in_unsafe_fn)]
#![allow(dead_code)]

pub mod proc;
pub mod sysfs;
pub mod syscall;
pub mod table;

use std::ffi::CString;
use std::io;
use std::os::unix::io::RawFd;

// ── ValidFd newtype -------------------------------------------------------

/// A file descriptor that is guaranteed to be non-negative (i.e. a real,
/// open fd as returned by a successful open(2) call).
///
/// Invariant: the inner `RawFd` is >= 0.
///
/// `ValidFd` is deliberately NOT `Copy`/`Clone` to discourage accidental
/// double-close.  Use [`ValidFd::as_raw`] to read the fd value.
#[derive(Debug)]
pub struct ValidFd(RawFd);

impl ValidFd {
    /// Wrap a raw fd.  Returns `None` if `fd < 0`.
    ///
    /// The caller must ensure `fd` is a valid, open file descriptor owned
    /// by the current process.  This function only checks the sign — it
    /// does not validate that the fd is actually open.
    pub fn new(fd: RawFd) -> Option<Self> {
        if fd < 0 { None } else { Some(Self(fd)) }
    }

    /// Return the underlying raw fd value (always >= 0).
    pub fn as_raw(&self) -> RawFd { self.0 }
}

impl Drop for ValidFd {
    fn drop(&mut self) {
        // SAFETY:
        //   - `self.0` is >= 0 by the invariant maintained by `ValidFd::new`.
        //   - This is the only place the fd is closed: `ValidFd` is not
        //     `Clone`/`Copy`, so there can be at most one owner.
        //   - We ignore the return value: EINTR on close(2) means the fd
        //     is already closed on Linux (unlike POSIX), so retrying would
        //     cause a use-after-close bug.
        unsafe { syscall::close(self.0); }
    }
}

// ── open_rdonly -----------------------------------------------------------

/// Thin wrapper around libc-free open(2): avoids pulling `libc` crate
/// by using the `nix`-free direct syscall path. Returns a `ValidFd` on
/// success or an `io::Error` on failure.
pub fn open_rdonly(path: &str) -> io::Result<ValidFd> {
    let c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;

    // Flags: O_RDONLY (0x0) | O_CLOEXEC (0x80000).
    //   O_RDONLY = 0       — open for reading only, no write permission needed.
    //   O_CLOEXEC = 0x80000 — close fd on execve(2) to avoid leaking into
    //                          child processes.  Equivalent to FD_CLOEXEC via
    //                          fcntl(2) but set atomically at open time.
    //   mode = 0           — ignored for O_RDONLY opens (no file is created).
    //
    // SAFETY:
    //   - `c.as_ptr()` is a non-null, NUL-terminated pointer valid for the
    //     entire `unsafe` block; `CString` guarantees this.  The string is
    //     not dropped until after `open` returns because `c` is still in scope.
    //   - 0x80000 is O_CLOEXEC on Linux x86_64/aarch64 (from
    //     include/uapi/asm-generic/fcntl.h). The two constants combined
    //     form a valid `open(2)` flags argument.
    //   - mode=0 is safe: the kernel ignores `mode` when O_CREAT is absent.
    let fd = unsafe { syscall::open(c.as_ptr(), 0 | 0x80000, 0) };

    // Convert the signed fd or error code into the appropriate Result type.
    // `ValidFd::new` will always succeed here because we check fd >= 0 first,
    // but the Option->Result conversion makes that invariant explicit.
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ValidFd::new(fd).expect("fd >= 0 just verified"))
    }
}

/// Small helper that strips trailing whitespace/newline from a string.
pub fn chomp(s: &str) -> &str {
    s.trim_end_matches(|c: char| c == '\n' || c == '\r' || c == ' ' || c == '\t')
}
