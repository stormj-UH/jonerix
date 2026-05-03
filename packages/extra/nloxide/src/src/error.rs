// Error codes derived from Ghidra analysis and netlink errno mapping

use core::ffi::{c_char, c_int};

// NLE_ error codes (from nl_syserr2nlerr and nl_geterror string table)
pub const NLE_SUCCESS: c_int = 0;
pub const NLE_FAILURE: c_int = 1;
pub const NLE_INTR: c_int = 2;
pub const NLE_BAD_SOCK: c_int = 3;
pub const NLE_AGAIN: c_int = 4;
pub const NLE_NOMEM: c_int = 5;
pub const NLE_EXIST: c_int = 6;
pub const NLE_INVAL: c_int = 7;
pub const NLE_RANGE: c_int = 8;
pub const NLE_MSGSIZE: c_int = 9;
pub const NLE_OPNOTSUPP: c_int = 10;
pub const NLE_AF_NOSUPPORT: c_int = 11;
pub const NLE_OBJ_NOTFOUND: c_int = 12;
pub const NLE_NOATTR: c_int = 13;
pub const NLE_MISSING_ATTR: c_int = 14;
pub const NLE_AF_MISMATCH: c_int = 15;
pub const NLE_SEQ_MISMATCH: c_int = 16;
pub const NLE_MSG_OVERFLOW: c_int = 17;
pub const NLE_MSG_TRUNC: c_int = 18;
pub const NLE_NOADDR: c_int = 19;
pub const NLE_SRCRT_NOSUPPORT: c_int = 20;
pub const NLE_MSG_TOOSHORT: c_int = 21;
pub const NLE_MSGTYPE_NOSUPPORT: c_int = 22;
pub const NLE_OBJ_MISMATCH: c_int = 23;
pub const NLE_NOCACHE: c_int = 24;
pub const NLE_BUSY: c_int = 25;
pub const NLE_PROTO: c_int = 26;
pub const NLE_NOACCESS: c_int = 27;
pub const NLE_PERM: c_int = 28;
pub const NLE_PKTLOC_FILE: c_int = 29;
pub const NLE_PARSE_ERR: c_int = 30;
pub const NLE_NODEV: c_int = 31;
pub const NLE_IMMUTABLE: c_int = 32;
pub const NLE_DUMP_INTR: c_int = 33;
pub const NLE_ATTRSIZE: c_int = 34;

static ERROR_STRINGS: &[&str] = &[
    "Success\0",
    "Unspecific failure\0",
    "Interrupted system call\0",
    "Bad socket\0",
    "Try again\0",
    "Out of memory\0",
    "Object exists\0",
    "Invalid input data or parameter\0",
    "Input data out of range\0",
    "Message size not sufficient\0",
    "Operation not supported\0",
    "Address family not supported\0",
    "Object not found\0",
    "Attribute not available\0",
    "Missing attribute\0",
    "Address family mismatch\0",
    "Sequence number mismatch\0",
    "Message buffer congestion limit hit\0",
    "Incomplete message read\0",
    "No address assigned\0",
    "Source based routing not supported\0",
    "Message too short\0",
    "Message type not supported\0",
    "Object type mismatch\0",
    "Cache is empty\0",
    "Resource busy\0",
    "Protocol mismatch\0",
    "No access\0",
    "Operation not permitted\0",
    "Unable to open pktloc file\0",
    "Parsing of message failed\0",
    "No such device\0",
    "Immutable attribute\0",
    "Dump consistency check failed\0",
    "Attribute max length exceeded\0",
];

pub fn syserr_to_nlerr(errno: c_int) -> c_int {
    match errno {
        libc::EBADF => NLE_BAD_SOCK,
        libc::EADDRINUSE => NLE_EXIST,
        libc::EEXIST => NLE_EXIST,
        libc::EADDRNOTAVAIL => NLE_NOADDR,
        libc::ESRCH => NLE_OBJ_NOTFOUND,
        libc::ENOENT => NLE_OBJ_NOTFOUND,
        libc::EINTR => NLE_INTR,
        libc::EAGAIN => NLE_AGAIN,
        libc::ENOMEM => NLE_NOMEM,
        libc::EACCES => NLE_NOACCESS,
        libc::EFAULT => NLE_INVAL,
        libc::EBUSY => NLE_BUSY,
        libc::ERANGE => NLE_RANGE,
        libc::EMSGSIZE => NLE_MSGSIZE,
        libc::ENOPROTOOPT => NLE_OPNOTSUPP,
        libc::EAFNOSUPPORT => NLE_AF_NOSUPPORT,
        libc::ENOTSOCK => NLE_BAD_SOCK,
        libc::EPROTONOSUPPORT => NLE_PROTO,
        libc::EPERM => NLE_PERM,
        libc::ENODEV => NLE_NODEV,
        _ => NLE_FAILURE,
    }
}

// --- exported C functions ---

#[no_mangle]
pub unsafe extern "C" fn nl_geterror(err: c_int) -> *const c_char {
    let idx = (-err) as usize;
    let s = if idx < ERROR_STRINGS.len() {
        ERROR_STRINGS[idx]
    } else {
        "Unknown error\0"
    };
    s.as_ptr() as *const c_char
}

#[no_mangle]
pub unsafe extern "C" fn nl_perror(err: c_int, msg: *const c_char) {
    let estr = nl_geterror(err);
    // write to fd 2 (stderr) directly to avoid libc::stderr portability issues
    if !msg.is_null() {
        libc::write(2, msg as _, libc::strlen(msg as _));
        libc::write(2, b": ".as_ptr() as _, 2);
        libc::write(2, estr as _, libc::strlen(estr as _));
        libc::write(2, b"\n".as_ptr() as _, 1);
    } else {
        libc::write(2, estr as _, libc::strlen(estr as _));
        libc::write(2, b"\n".as_ptr() as _, 1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_syserr2nlerr(err: c_int) -> c_int {
    -syserr_to_nlerr(err.abs())
}

#[no_mangle]
pub fn errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn nl_strerror_l(err: c_int) -> *const c_char {
    nl_geterror(-err)
}
