// Raw libc declarations — no external crates needed.
// All types and functions are standard POSIX/Linux.

use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

pub type SizeT = usize;
pub type SSizeT = isize;
pub type SocklenT = u32;

// sockaddr is just an opaque type for bind/getsockname
#[repr(C)]
pub struct Sockaddr {
    pub sa_family: u16,
    pub sa_data: [u8; 14],
}

// iovec / msghdr (not used externally but needed for sendmsg)
#[repr(C)]
pub struct Iovec {
    pub iov_base: *mut c_void,
    pub iov_len: SizeT,
}

pub const AF_NETLINK: c_int = 16;
pub const SOCK_RAW: c_int = 3;
pub const SOCK_CLOEXEC: c_int = 0o2000000;
pub const SOL_SOCKET: c_int = 1;
pub const SOL_NETLINK: c_int = 270;
pub const SO_RCVBUF: c_int = 8;
pub const SO_SNDBUF: c_int = 7;
pub const SO_RCVBUFFORCE: c_int = 33;
pub const SO_SNDBUFFORCE: c_int = 32;
pub const SO_PASSCRED: c_int = 16;
pub const NETLINK_ADD_MEMBERSHIP: c_int = 1;
pub const NETLINK_DROP_MEMBERSHIP: c_int = 2;
pub const NETLINK_PKTINFO: c_int = 3;
pub const F_GETFL: c_int = 3;
pub const F_SETFL: c_int = 4;
pub const O_NONBLOCK: c_int = 0o4000;

pub const EINTR: c_int = 4;
pub const EAGAIN: c_int = 11;
pub const EWOULDBLOCK: c_int = 11;

extern "C" {
    pub fn socket(domain: c_int, type_: c_int, protocol: c_int) -> c_int;
    pub fn bind(fd: c_int, addr: *const Sockaddr, addrlen: SocklenT) -> c_int;
    pub fn getsockname(fd: c_int, addr: *mut Sockaddr, addrlen: *mut SocklenT) -> c_int;
    pub fn setsockopt(fd: c_int, level: c_int, optname: c_int, optval: *const c_void, optlen: SocklenT) -> c_int;
    pub fn close(fd: c_int) -> c_int;
    pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    pub fn sendto(fd: c_int, buf: *const c_void, n: SizeT, flags: c_int, addr: *const Sockaddr, addrlen: SocklenT) -> SSizeT;
    pub fn recvfrom(fd: c_int, buf: *mut c_void, n: SizeT, flags: c_int, addr: *mut Sockaddr, addrlen: *mut SocklenT) -> SSizeT;
    pub fn send(fd: c_int, buf: *const c_void, n: SizeT, flags: c_int) -> SSizeT;

    pub fn malloc(n: SizeT) -> *mut c_void;
    pub fn calloc(nmemb: SizeT, size: SizeT) -> *mut c_void;
    pub fn realloc(p: *mut c_void, n: SizeT) -> *mut c_void;
    pub fn free(p: *mut c_void);
    pub fn memcpy(dst: *mut c_void, src: *const c_void, n: SizeT) -> *mut c_void;
    pub fn memset(dst: *mut c_void, c: c_int, n: SizeT) -> *mut c_void;
    pub fn memcmp(a: *const c_void, b: *const c_void, n: SizeT) -> c_int;
    pub fn strlen(s: *const c_char) -> SizeT;
    pub fn strcmp(a: *const c_char, b: *const c_char) -> c_int;
    pub fn strdup(s: *const c_char) -> *mut c_char;
    pub fn write(fd: c_int, buf: *const c_void, n: SizeT) -> SSizeT;

    pub fn getpid() -> i32;

    #[link_name = "__errno_location"]
    pub fn errno_location() -> *mut c_int;
}

#[inline(always)]
pub fn errno() -> c_int {
    unsafe { *errno_location() }
}
