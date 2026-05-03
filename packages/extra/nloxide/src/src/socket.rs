// nl_socket_* and nl_send/recv functions derived from Ghidra analysis of libnl3-3.11.0
//
// Key layout observations from Ghidra (param_1 is long pointer to nl_sock):
//   fd at +0x18 (24), flags at +0x28, *nl_cb at +0x30
//   sockaddr_nl (s_local) at +0x00, s_peer at +0x0c

use crate::callback::{NlCb, NlRecvMsgCb, NlRecvMsgErrCb};
use crate::error::errno;
use crate::error::{
    syserr_to_nlerr, NLE_AGAIN, NLE_BAD_SOCK, NLE_INTR, NLE_INVAL, NLE_MSG_TOOSHORT, NLE_NOMEM,
};
use crate::message::{nlmsg_free, NlMsg};
use crate::types::*;
use core::ffi::{c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};
use libc::{sockaddr, socklen_t};

// Linux-only socket constants used for netlink (public kernel ABI)
const SO_RCVBUFFORCE: c_int = 33;
const SO_SNDBUFFORCE: c_int = 32;
const SO_PASSCRED: c_int = 16;
const NETLINK_PKTINFO: c_int = 3;
const SOCK_CLOEXEC: c_int = 0o2000000; // Linux O_CLOEXEC flag for SOCK_*

// Global port allocator: PID-based local port with random uniquification
static NEXT_PORT: AtomicU32 = AtomicU32::new(0);

fn alloc_local_port() -> u32 {
    let pid = unsafe { libc::getpid() } as u32;
    let seq = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
    if seq == 0 {
        pid
    } else {
        pid | (seq << 22)
    }
}

pub struct NlSock {
    pub s_local: SockaddrNl, // +0x00  local address
    pub s_peer: SockaddrNl,  // +0x0c  peer address
    pub s_fd: c_int,         // +0x18  socket fd
    pub s_proto: c_int,      // +0x1c  protocol
    pub s_seq_next: u32,     // +0x20
    pub s_seq_expect: u32,   // +0x24
    pub s_flags: u32,        // +0x28
    pub s_bufsize: usize,    // receive buffer size hint
    pub s_cb: *mut NlCb,     // +0x30
    pub s_msgbufsize: usize, // per-message receive buffer size
}

unsafe impl Send for NlSock {}

impl NlSock {
    fn new(cb: *mut NlCb) -> *mut NlSock {
        let sk = Box::new(NlSock {
            s_local: SockaddrNl {
                nl_family: AF_NETLINK,
                nl_pad: 0,
                nl_pid: alloc_local_port(),
                nl_groups: 0,
            },
            s_peer: SockaddrNl {
                nl_family: AF_NETLINK,
                nl_pad: 0,
                nl_pid: 0,
                nl_groups: 0,
            },
            s_fd: -1,
            s_proto: 0,
            s_seq_next: unsafe { libc::getpid() } as u32, // use PID as initial seq
            s_seq_expect: 0,
            s_flags: 0,
            s_bufsize: 0,
            s_cb: cb,
            s_msgbufsize: 0,
        });
        Box::into_raw(sk)
    }
}

// --- exported C functions: nl_socket_* ---

#[no_mangle]
pub unsafe extern "C" fn nl_socket_alloc() -> *mut NlSock {
    let cb = crate::callback::nl_cb_alloc(NL_CB_DEFAULT);
    if cb.is_null() {
        return ptr::null_mut();
    }
    NlSock::new(cb)
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_alloc_cb(cb: *mut NlCb) -> *mut NlSock {
    if cb.is_null() {
        return ptr::null_mut();
    }
    crate::callback::nl_cb_get(cb);
    NlSock::new(cb)
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_free(sk: *mut NlSock) {
    if sk.is_null() {
        return;
    }
    if (*sk).s_fd >= 0 {
        libc::close((*sk).s_fd);
    }
    if !(*sk).s_cb.is_null() {
        crate::callback::nl_cb_put((*sk).s_cb);
    }
    drop(Box::from_raw(sk));
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_get_fd(sk: *const NlSock) -> c_int {
    if sk.is_null() {
        return -1;
    }
    (*sk).s_fd
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_fd(sk: *mut NlSock, fd: c_int) -> c_int {
    if sk.is_null() {
        return -(NLE_INVAL);
    }
    (*sk).s_fd = fd;
    (*sk).s_flags |= NL_SOCK_OWNS_FD;
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_get_local_port(sk: *const NlSock) -> u32 {
    if sk.is_null() {
        return 0;
    }
    (*sk).s_local.nl_pid
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_local_port(sk: *mut NlSock, port: u32) {
    if !sk.is_null() {
        (*sk).s_local.nl_pid = port;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_get_peer_port(sk: *const NlSock) -> u32 {
    if sk.is_null() {
        return 0;
    }
    (*sk).s_peer.nl_pid
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_peer_port(sk: *mut NlSock, port: u32) {
    if !sk.is_null() {
        (*sk).s_peer.nl_pid = port;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_get_peer_groups(sk: *const NlSock) -> u32 {
    if sk.is_null() {
        return 0;
    }
    (*sk).s_peer.nl_groups
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_peer_groups(sk: *mut NlSock, groups: u32) {
    if !sk.is_null() {
        (*sk).s_peer.nl_groups = groups;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_get_cb(sk: *const NlSock) -> *mut NlCb {
    if sk.is_null() {
        return ptr::null_mut();
    }
    crate::callback::nl_cb_get((*sk).s_cb)
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_cb(sk: *mut NlSock, cb: *mut NlCb) {
    if sk.is_null() || cb.is_null() {
        return;
    }
    crate::callback::nl_cb_put((*sk).s_cb);
    crate::callback::nl_cb_get(cb);
    (*sk).s_cb = cb;
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_modify_cb(
    sk: *mut NlSock,
    cb_type: c_int,
    cb_kind: u32,
    func: Option<NlRecvMsgCb>,
    arg: *mut c_void,
) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    crate::callback::nl_cb_set((*sk).s_cb, cb_type, cb_kind, func, arg)
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_modify_err_cb(
    sk: *mut NlSock,
    _cb_kind: u32,
    func: Option<NlRecvMsgErrCb>,
    arg: *mut c_void,
) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    crate::callback::nl_cb_err((*sk).s_cb, 0, func, arg)
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_disable_seq_check(sk: *mut NlSock) {
    if !sk.is_null() {
        (*sk).s_flags |= NL_SOCK_DISABLE_SEQ_CHECK;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_use_seq(sk: *mut NlSock, seq: u32) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    (*sk).s_seq_next = seq;
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_disable_auto_ack(sk: *mut NlSock) {
    if !sk.is_null() {
        (*sk).s_flags |= NL_SOCK_NO_AUTO_ACK;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_enable_auto_ack(sk: *mut NlSock) {
    if !sk.is_null() {
        (*sk).s_flags &= !NL_SOCK_NO_AUTO_ACK;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_enable_msg_peek(sk: *mut NlSock) {
    if !sk.is_null() {
        (*sk).s_flags |= NL_SOCK_MSG_PEEK;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_disable_msg_peek(sk: *mut NlSock) {
    if !sk.is_null() {
        (*sk).s_flags &= !NL_SOCK_MSG_PEEK;
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_nonblocking(sk: *mut NlSock) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    if (*sk).s_fd < 0 {
        return -(NLE_BAD_SOCK);
    }
    let flags = libc::fcntl((*sk).s_fd, libc::F_GETFL, 0);
    if flags < 0 {
        return -(syserr_to_nlerr(errno()));
    }
    let r = libc::fcntl((*sk).s_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    if r < 0 {
        return -(syserr_to_nlerr(errno()));
    }
    (*sk).s_flags |= NL_SOCK_NONBLOCK;
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_buffer_size(
    sk: *mut NlSock,
    rxbuf: c_int,
    txbuf: c_int,
) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    if (*sk).s_fd < 0 {
        return 0;
    } // not yet connected
    if rxbuf > 0 {
        let r = libc::setsockopt(
            (*sk).s_fd,
            libc::SOL_SOCKET,
            SO_RCVBUFFORCE,
            &rxbuf as *const c_int as _,
            core::mem::size_of::<c_int>() as _,
        );
        if r < 0 {
            // try without FORCE
            libc::setsockopt(
                (*sk).s_fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &rxbuf as *const c_int as _,
                core::mem::size_of::<c_int>() as _,
            );
        }
        (*sk).s_bufsize = rxbuf as usize;
        (*sk).s_flags |= NL_SOCK_BUFSIZE_SET;
    }
    if txbuf > 0 {
        let r = libc::setsockopt(
            (*sk).s_fd,
            libc::SOL_SOCKET,
            SO_SNDBUFFORCE,
            &txbuf as *const c_int as _,
            core::mem::size_of::<c_int>() as _,
        );
        if r < 0 {
            libc::setsockopt(
                (*sk).s_fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &txbuf as *const c_int as _,
                core::mem::size_of::<c_int>() as _,
            );
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_msg_buf_size(sk: *mut NlSock, size: usize) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    (*sk).s_msgbufsize = size;
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_get_msg_buf_size(sk: *const NlSock) -> usize {
    if sk.is_null() {
        return 0;
    }
    (*sk).s_msgbufsize
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_set_passcred(sk: *mut NlSock, state: c_int) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    if (*sk).s_fd < 0 {
        return -(NLE_BAD_SOCK);
    }
    let v: c_int = if state != 0 { 1 } else { 0 };
    let r = libc::setsockopt(
        (*sk).s_fd,
        libc::SOL_SOCKET,
        SO_PASSCRED,
        &v as *const c_int as _,
        core::mem::size_of::<c_int>() as _,
    );
    if r < 0 {
        return -(syserr_to_nlerr(errno()));
    }
    if state != 0 {
        (*sk).s_flags |= NL_SOCK_PASSCRED;
    } else {
        (*sk).s_flags &= !NL_SOCK_PASSCRED;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_recv_pktinfo(sk: *mut NlSock, state: c_int) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    // SOL_NETLINK / NETLINK_PKTINFO
    let v: c_int = if state != 0 { 1 } else { 0 };
    let _ = libc::setsockopt(
        (*sk).s_fd,
        SOL_NETLINK,
        NETLINK_PKTINFO,
        &v as *const c_int as _,
        core::mem::size_of::<c_int>() as _,
    );
    0
}

// nl_socket_add_memberships: variadic — groups are passed as c_int varargs terminated by 0
// C prototype: int nl_socket_add_memberships(struct nl_sock *sk, int group, ...)
// We export a single-group version; wpa_supplicant calls nl_socket_add_membership wrapper
#[no_mangle]
pub unsafe extern "C" fn nl_socket_add_membership(sk: *mut NlSock, group: c_int) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    if (*sk).s_fd < 0 {
        return -(NLE_BAD_SOCK);
    }
    let r = libc::setsockopt(
        (*sk).s_fd,
        SOL_NETLINK,
        NETLINK_ADD_MEMBERSHIP,
        &group as *const c_int as _,
        core::mem::size_of::<c_int>() as socklen_t,
    );
    if r < 0 {
        return -(syserr_to_nlerr(errno()));
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_drop_membership(sk: *mut NlSock, group: c_int) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    if (*sk).s_fd < 0 {
        return -(NLE_BAD_SOCK);
    }
    let r = libc::setsockopt(
        (*sk).s_fd,
        SOL_NETLINK,
        NETLINK_DROP_MEMBERSHIP,
        &group as *const c_int as _,
        core::mem::size_of::<c_int>() as socklen_t,
    );
    if r < 0 {
        return -(syserr_to_nlerr(errno()));
    }
    0
}

// Variadic versions: nl_socket_add_memberships / nl_socket_drop_memberships
// These take groups as a 0-terminated va_list.  We export a 2-arg form that wpa_supplicant
// inlines as a loop of single-group calls.
#[no_mangle]
pub unsafe extern "C" fn nl_socket_add_memberships(sk: *mut NlSock, group: c_int) -> c_int {
    nl_socket_add_membership(sk, group)
}

#[no_mangle]
pub unsafe extern "C" fn nl_socket_drop_memberships(sk: *mut NlSock, group: c_int) -> c_int {
    nl_socket_drop_membership(sk, group)
}

#[no_mangle]
pub unsafe extern "C" fn nl_join_groups(sk: *mut NlSock, groups: c_int) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    (*sk).s_local.nl_groups = groups as u32;
    0
}

// --- nl_connect / nl_close ---

#[no_mangle]
pub unsafe extern "C" fn nl_connect(sk: *mut NlSock, protocol: c_int) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    if (*sk).s_fd != -1 {
        return 0;
    } // already connected

    let fd = libc::socket(AF_NETLINK as c_int, libc::SOCK_RAW | SOCK_CLOEXEC, protocol);
    if fd < 0 {
        return -(syserr_to_nlerr(errno()));
    }
    (*sk).s_fd = fd;
    (*sk).s_proto = protocol;

    // Set default buffer sizes
    nl_socket_set_buffer_size(sk, 0, 0);

    // Bind local address
    (*sk).s_local.nl_family = AF_NETLINK;
    let local_copy = (*sk).s_local;
    let r = libc::bind(
        fd,
        &local_copy as *const SockaddrNl as *const sockaddr,
        core::mem::size_of::<SockaddrNl>() as socklen_t,
    );
    if r < 0 {
        let e = errno();
        libc::close(fd);
        (*sk).s_fd = -1;
        return -(syserr_to_nlerr(e));
    }

    // Read back actual nl_pid assigned by kernel
    let mut addr: SockaddrNl = SockaddrNl::default();
    let mut addrlen: socklen_t = core::mem::size_of::<SockaddrNl>() as socklen_t;
    if libc::getsockname(
        fd,
        &mut addr as *mut SockaddrNl as *mut sockaddr,
        &mut addrlen,
    ) == 0
    {
        (*sk).s_local = addr;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_close(sk: *mut NlSock) {
    if sk.is_null() {
        return;
    }
    if (*sk).s_fd >= 0 {
        libc::close((*sk).s_fd);
        (*sk).s_fd = -1;
    }
}

// --- nl_send* ---

#[no_mangle]
pub unsafe extern "C" fn nl_complete_msg(sk: *mut NlSock, msg: *mut NlMsg) -> c_int {
    if sk.is_null() || msg.is_null() {
        return -(NLE_INVAL);
    }
    let hdr = (*msg).hdr();
    if (*hdr).nlmsg_pid == 0 {
        (*hdr).nlmsg_pid = (*sk).s_local.nl_pid;
    }
    if (*hdr).nlmsg_seq == 0 {
        (*sk).s_seq_next += 1;
        (*hdr).nlmsg_seq = (*sk).s_seq_next;
    }
    (*hdr).nlmsg_flags |= NLM_F_REQUEST;
    if (*sk).s_flags & NL_SOCK_NO_AUTO_ACK == 0 {
        (*hdr).nlmsg_flags |= NLM_F_ACK;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_auto_complete(sk: *mut NlSock, msg: *mut NlMsg) -> c_int {
    nl_complete_msg(sk, msg)
}

#[no_mangle]
pub unsafe extern "C" fn nl_send_auto_complete(sk: *mut NlSock, msg: *mut NlMsg) -> c_int {
    nl_send_auto(sk, msg)
}

#[no_mangle]
pub unsafe extern "C" fn nl_send_auto(sk: *mut NlSock, msg: *mut NlMsg) -> c_int {
    if sk.is_null() || msg.is_null() {
        return -(NLE_INVAL);
    }
    nl_complete_msg(sk, msg);

    // MSG_OUT callback
    if !(*sk).s_cb.is_null() {
        let cb = (*(*sk).s_cb).cb[NL_CB_MSG_OUT].func;
        let arg = (*(*sk).s_cb).cb[NL_CB_MSG_OUT].arg;
        if let Some(f) = cb {
            let ret = f(msg, arg);
            if ret != crate::types::NL_OK {
                return ret;
            }
        }
    }

    nl_sendmsg(sk, msg, ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn nl_send(sk: *mut NlSock, msg: *mut NlMsg) -> c_int {
    nl_sendmsg(sk, msg, ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn nl_sendmsg(
    sk: *mut NlSock,
    msg: *mut NlMsg,
    _msghdr: *mut libc::msghdr,
) -> c_int {
    if sk.is_null() || msg.is_null() {
        return -(NLE_BAD_SOCK);
    }
    if (*sk).s_fd < 0 {
        return -(NLE_BAD_SOCK);
    }

    let hdr = (*msg).hdr();
    let len = (*hdr).nlmsg_len as usize;
    let peer: SockaddrNl = if (*msg).dst.nl_family != 0 {
        (*msg).dst
    } else {
        (*sk).s_peer
    };

    let sent = libc::sendto(
        (*sk).s_fd,
        hdr as *const c_void,
        len,
        0,
        &peer as *const SockaddrNl as *const sockaddr,
        core::mem::size_of::<SockaddrNl>() as socklen_t,
    );
    if sent < 0 {
        return -(syserr_to_nlerr(errno()));
    }
    sent as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nl_sendto(sk: *mut NlSock, buf: *const c_void, size: usize) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    let r = libc::send((*sk).s_fd, buf, size, 0);
    if r < 0 {
        return -(syserr_to_nlerr(errno()));
    }
    r as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nl_send_iovec(
    sk: *mut NlSock,
    msg: *mut NlMsg,
    _iov: *mut libc::iovec,
    _iovlen: usize,
) -> c_int {
    nl_sendmsg(sk, msg, ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn nl_send_simple(
    sk: *mut NlSock,
    nlmsg_type: c_int,
    flags: c_int,
    buf: *const c_void,
    size: usize,
) -> c_int {
    let msg = crate::message::nlmsg_alloc_simple(nlmsg_type, flags);
    if msg.is_null() {
        return -(NLE_NOMEM);
    }
    if !buf.is_null()
        && size > 0
        && crate::message::nlmsg_append(msg, buf, size, nlmsg_align(1)) != 0
    {
        nlmsg_free(msg);
        return -(NLE_NOMEM);
    }
    let r = nl_send_auto(sk, msg);
    nlmsg_free(msg);
    r
}

// --- nl_recv ---

pub unsafe fn nl_recv_raw(
    sk: *mut NlSock,
    nla: *mut SockaddrNl,
    buf: *mut *mut u8,
    _creds: *mut c_void,
) -> c_int {
    if sk.is_null() || (*sk).s_fd < 0 {
        return -(NLE_BAD_SOCK);
    }

    let bufsize = if (*sk).s_msgbufsize > 0 {
        (*sk).s_msgbufsize
    } else {
        32768
    };
    let mem = libc::malloc(bufsize) as *mut u8;
    if mem.is_null() {
        return -(NLE_NOMEM);
    }

    let mut peer: SockaddrNl = SockaddrNl::default();
    let mut peerlen: socklen_t = core::mem::size_of::<SockaddrNl>() as socklen_t;

    let n = libc::recvfrom(
        (*sk).s_fd,
        mem as *mut c_void,
        bufsize,
        0,
        &mut peer as *mut SockaddrNl as *mut sockaddr,
        &mut peerlen,
    );
    if n <= 0 {
        libc::free(mem as _);
        if n == 0 {
            return 0;
        }
        let e = errno();
        return if e == libc::EINTR {
            -(NLE_INTR)
        } else if e == libc::EAGAIN {
            -(NLE_AGAIN)
        } else {
            -(syserr_to_nlerr(e))
        };
    }

    if !nla.is_null() {
        *nla = peer;
    }
    *buf = mem;
    n as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nl_recv(
    sk: *mut NlSock,
    nla: *mut SockaddrNl,
    buf: *mut *mut u8,
    creds: *mut c_void,
) -> c_int {
    nl_recv_raw(sk, nla, buf, creds)
}

// --- nl_recvmsgs ---

#[no_mangle]
pub unsafe extern "C" fn nl_recvmsgs_default(sk: *mut NlSock) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    let cb = (*sk).s_cb;
    nl_recvmsgs(sk, cb)
}

#[no_mangle]
pub unsafe extern "C" fn nl_recvmsgs(sk: *mut NlSock, cb: *mut NlCb) -> c_int {
    let r = nl_recvmsgs_report(sk, cb);
    if r > 0 {
        0
    } else {
        r
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_recvmsgs_report(sk: *mut NlSock, cb: *mut NlCb) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }

    let mut buf: *mut u8 = ptr::null_mut();
    let mut peer: SockaddrNl = SockaddrNl::default();
    let n = nl_recv_raw(sk, &mut peer, &mut buf, ptr::null_mut());
    if n <= 0 {
        return n;
    }

    let mut pos = buf as *mut NlMsgHdr;
    let mut remaining = n;
    let mut ret = 0;

    'outer: while crate::message::nlmsg_ok(pos, remaining) {
        let msg = crate::message::nlmsg_convert(pos);

        // MSG_IN callback
        if !cb.is_null() {
            if let Some(f) = (*cb).cb[NL_CB_MSG_IN].func {
                let r2 = f(msg, (*cb).cb[NL_CB_MSG_IN].arg);
                match r2 {
                    crate::types::NL_SKIP => {
                        nlmsg_free(msg);
                        pos = next_hdr(pos, &mut remaining);
                        continue;
                    }
                    crate::types::NL_STOP => {
                        nlmsg_free(msg);
                        ret = 0;
                        break 'outer;
                    }
                    _ => {}
                }
            }
        }

        // sequence check
        if (*sk).s_flags & NL_SOCK_DISABLE_SEQ_CHECK == 0 {
            let seq = (*pos).nlmsg_seq;
            if seq != (*sk).s_seq_expect && (*sk).s_seq_expect != 0 {
                nlmsg_free(msg);
                pos = next_hdr(pos, &mut remaining);
                ret = -(crate::error::NLE_SEQ_MISMATCH);
                continue;
            }
        }

        let nltype = (*pos).nlmsg_type;
        match nltype {
            NLMSG_NOOP => {
                dispatch_cb(cb, NL_CB_SKIPPED, msg, &mut ret);
            }
            NLMSG_OVERRUN => {
                dispatch_cb(cb, NL_CB_OVERRUN, msg, &mut ret);
                ret = -(crate::error::NLE_MSG_OVERFLOW);
                nlmsg_free(msg);
                break 'outer;
            }
            NLMSG_ERROR => {
                let datalen = crate::message::nlmsg_datalen(pos);
                if datalen < core::mem::size_of::<crate::callback::NlMsgErr>() as c_int {
                    dispatch_cb(cb, NL_CB_INVALID, msg, &mut ret);
                    ret = -(NLE_MSG_TOOSHORT);
                    nlmsg_free(msg);
                    break 'outer;
                }
                let errp = crate::message::nlmsg_data(pos) as *mut crate::callback::NlMsgErr;
                let errnum = (*errp).error;
                if errnum == 0 {
                    // ACK
                    let cb_ret = dispatch_cb(cb, NL_CB_ACK, msg, &mut ret);
                    if cb_ret == crate::types::NL_STOP {
                        nlmsg_free(msg);
                        break 'outer;
                    }
                } else {
                    if !cb.is_null() {
                        if let Some(ef) = (*cb).err_cb.func {
                            let er = ef(&mut peer as *mut SockaddrNl, errp, (*cb).err_cb.arg);
                            if er == crate::types::NL_STOP {
                                nlmsg_free(msg);
                                ret = errnum;
                                break 'outer;
                            }
                        }
                    }
                    ret = errnum;
                }
            }
            NLMSG_DONE => {
                let cb_ret = dispatch_cb(cb, NL_CB_FINISH, msg, &mut ret);
                nlmsg_free(msg);
                if cb_ret == crate::types::NL_SKIP {
                    remaining = 0;
                    continue;
                }
                break 'outer;
            }
            _ => {
                // valid data message
                if (*pos).nlmsg_flags & NLM_F_DUMP_INTR != 0 {
                    dispatch_cb(cb, NL_CB_DUMP_INTR, msg, &mut ret);
                }
                let cb_ret = dispatch_cb(cb, NL_CB_VALID, msg, &mut ret);
                if cb_ret == crate::types::NL_STOP {
                    nlmsg_free(msg);
                    break 'outer;
                }
            }
        }

        nlmsg_free(msg);
        pos = next_hdr(pos, &mut remaining);

        if remaining <= 0 {
            break;
        }
    }

    libc::free(buf as _);
    ret
}

fn next_hdr(hdr: *mut NlMsgHdr, remaining: &mut c_int) -> *mut NlMsgHdr {
    unsafe {
        let step = nlmsg_align((*hdr).nlmsg_len as usize) as i32;
        *remaining -= step;
        (hdr as *mut u8).add(step as usize) as *mut NlMsgHdr
    }
}

fn dispatch_cb(cb: *mut NlCb, cb_type: usize, msg: *mut NlMsg, _ret: &mut c_int) -> c_int {
    unsafe {
        if cb.is_null() {
            return crate::types::NL_OK;
        }
        if let Some(f) = (*cb).cb[cb_type].func {
            return f(msg, (*cb).cb[cb_type].arg);
        }
        crate::types::NL_OK
    }
}

// --- nl_wait_for_ack / nl_send_sync ---

#[no_mangle]
pub unsafe extern "C" fn nl_wait_for_ack(sk: *mut NlSock) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }

    // Clone callback and set a one-shot ACK handler that stops the loop
    let cb = crate::callback::nl_cb_clone((*sk).s_cb);
    if cb.is_null() {
        return -(NLE_NOMEM);
    }

    // Use a static that records ACK arrival
    struct AckCtx {
        done: bool,
        err: c_int,
    }
    let mut ctx = AckCtx {
        done: false,
        err: 0,
    };

    unsafe extern "C" fn ack_handler(_msg: *mut NlMsg, arg: *mut c_void) -> c_int {
        let ctx = &mut *(arg as *mut AckCtx);
        ctx.done = true;
        crate::types::NL_STOP
    }

    crate::callback::nl_cb_set(
        cb,
        NL_CB_ACK as c_int,
        0,
        Some(ack_handler),
        &mut ctx as *mut AckCtx as *mut c_void,
    );

    let mut r = 0;
    while !ctx.done && r >= 0 {
        r = nl_recvmsgs(sk, cb);
    }
    crate::callback::nl_cb_put(cb);
    if ctx.err != 0 {
        ctx.err
    } else {
        r
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_send_sync(sk: *mut NlSock, msg: *mut NlMsg) -> c_int {
    let r = nl_send_auto(sk, msg);
    nlmsg_free(msg);
    if r < 0 {
        return r;
    }
    nl_wait_for_ack(sk)
}

#[no_mangle]
pub unsafe extern "C" fn nl_pickup(
    sk: *mut NlSock,
    parser: Option<unsafe extern "C" fn(*mut NlMsg, *mut c_void) -> c_int>,
    result: *mut *mut c_void,
) -> c_int {
    if sk.is_null() {
        return -(NLE_BAD_SOCK);
    }
    struct PickupCtx {
        parser: Option<unsafe extern "C" fn(*mut NlMsg, *mut c_void) -> c_int>,
        result: *mut c_void,
    }
    let mut ctx = PickupCtx {
        parser,
        result: ptr::null_mut(),
    };
    unsafe extern "C" fn valid_cb(msg: *mut NlMsg, arg: *mut c_void) -> c_int {
        let ctx = &mut *(arg as *mut PickupCtx);
        if let Some(f) = ctx.parser {
            f(msg, &mut ctx.result as *mut *mut c_void as _);
        }
        crate::types::NL_OK
    }
    let cb = crate::callback::nl_cb_clone((*sk).s_cb);
    crate::callback::nl_cb_set(
        cb,
        NL_CB_VALID as c_int,
        0,
        Some(valid_cb),
        &mut ctx as *mut PickupCtx as _,
    );
    let r = nl_recvmsgs(sk, cb);
    crate::callback::nl_cb_put(cb);
    if !result.is_null() {
        *result = ctx.result;
    }
    r
}

#[no_mangle]
pub unsafe extern "C" fn nl_pickup_keep_syserr(
    sk: *mut NlSock,
    parser: Option<unsafe extern "C" fn(*mut NlMsg, *mut c_void) -> c_int>,
    result: *mut *mut c_void,
    _syserr: *mut c_int,
) -> c_int {
    nl_pickup(sk, parser, result)
}

// has_capability stub
#[no_mangle]
pub unsafe extern "C" fn nl_has_capability(_cap: c_int) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::Ordering;

    #[test]
    fn socket_alloc_owns_single_default_callback_ref() {
        unsafe {
            let sk = nl_socket_alloc();
            assert!(!sk.is_null());
            assert!(!(*sk).s_cb.is_null());
            assert_eq!((*(*sk).s_cb).refcount.load(Ordering::Acquire), 1);
            nl_socket_free(sk);
        }
    }

    #[test]
    fn complete_msg_sets_request_and_preserves_auto_ack() {
        unsafe {
            let sk = nl_socket_alloc();
            assert!(!sk.is_null());

            let msg = crate::message::nlmsg_alloc_simple(42, 0);
            assert!(!msg.is_null());
            assert_eq!(nl_complete_msg(sk, msg), 0);

            let hdr = crate::message::nlmsg_hdr(msg);
            assert_ne!((*hdr).nlmsg_flags & NLM_F_REQUEST, 0);
            assert_ne!((*hdr).nlmsg_flags & NLM_F_ACK, 0);

            crate::message::nlmsg_free(msg);
            nl_socket_free(sk);
        }
    }

    #[test]
    fn complete_msg_respects_disabled_auto_ack() {
        unsafe {
            let sk = nl_socket_alloc();
            assert!(!sk.is_null());
            nl_socket_disable_auto_ack(sk);

            let msg = crate::message::nlmsg_alloc_simple(42, 0);
            assert!(!msg.is_null());
            assert_eq!(nl_complete_msg(sk, msg), 0);

            let hdr = crate::message::nlmsg_hdr(msg);
            assert_ne!((*hdr).nlmsg_flags & NLM_F_REQUEST, 0);
            assert_eq!((*hdr).nlmsg_flags & NLM_F_ACK, 0);

            crate::message::nlmsg_free(msg);
            nl_socket_free(sk);
        }
    }

    #[cfg(not(miri))]
    struct CallbackCounts {
        invalid: usize,
        skipped: usize,
        valid: usize,
    }

    #[cfg(not(miri))]
    unsafe extern "C" fn count_invalid(_msg: *mut NlMsg, arg: *mut c_void) -> c_int {
        (*(arg as *mut CallbackCounts)).invalid += 1;
        NL_OK
    }

    #[cfg(not(miri))]
    unsafe extern "C" fn count_skipped(_msg: *mut NlMsg, arg: *mut c_void) -> c_int {
        (*(arg as *mut CallbackCounts)).skipped += 1;
        NL_OK
    }

    #[cfg(not(miri))]
    unsafe extern "C" fn count_valid(_msg: *mut NlMsg, arg: *mut c_void) -> c_int {
        (*(arg as *mut CallbackCounts)).valid += 1;
        NL_OK
    }

    #[cfg(not(miri))]
    unsafe fn socketpair_sock() -> (*mut NlSock, c_int) {
        let mut fds = [-1; 2];
        assert_eq!(
            libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM, 0, fds.as_mut_ptr()),
            0
        );
        let sk = nl_socket_alloc();
        assert!(!sk.is_null());
        assert_eq!(nl_socket_set_fd(sk, fds[0]), 0);
        (*sk).s_msgbufsize = 256;
        (sk, fds[1])
    }

    #[cfg(not(miri))]
    unsafe fn send_datagram(fd: c_int, buf: &[u8]) {
        let sent = libc::send(fd, buf.as_ptr() as *const c_void, buf.len(), 0);
        assert_eq!(sent, buf.len() as isize);
    }

    #[cfg(not(miri))]
    fn push_nlmsg(buf: &mut Vec<u8>, nltype: u16, flags: u16, seq: u32, payload: &[u8]) {
        let len = (NLMSG_HDRLEN + payload.len()) as u32;
        buf.extend_from_slice(&len.to_ne_bytes());
        buf.extend_from_slice(&nltype.to_ne_bytes());
        buf.extend_from_slice(&flags.to_ne_bytes());
        buf.extend_from_slice(&seq.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes());
        buf.extend_from_slice(payload);
        while !buf.len().is_multiple_of(4) {
            buf.push(0);
        }
    }

    #[cfg(not(miri))]
    #[test]
    fn recv_reports_short_error_messages_without_overread() {
        unsafe {
            let (sk, peer_fd) = socketpair_sock();
            let mut counts = CallbackCounts {
                invalid: 0,
                skipped: 0,
                valid: 0,
            };
            nl_socket_modify_cb(
                sk,
                NL_CB_INVALID as c_int,
                0,
                Some(count_invalid),
                &mut counts as *mut CallbackCounts as *mut c_void,
            );

            let mut datagram = Vec::new();
            push_nlmsg(&mut datagram, NLMSG_ERROR, 0, 0, &[0, 0, 0, 0]);
            send_datagram(peer_fd, &datagram);

            assert_eq!(nl_recvmsgs_default(sk), -(NLE_MSG_TOOSHORT));
            assert_eq!(counts.invalid, 1);

            nl_socket_free(sk);
            libc::close(peer_fd);
        }
    }

    #[cfg(not(miri))]
    #[test]
    fn recv_dispatches_noop_and_valid_messages_from_one_buffer() {
        unsafe {
            let (sk, peer_fd) = socketpair_sock();
            nl_socket_disable_seq_check(sk);
            let mut counts = CallbackCounts {
                invalid: 0,
                skipped: 0,
                valid: 0,
            };
            nl_socket_modify_cb(
                sk,
                NL_CB_SKIPPED as c_int,
                0,
                Some(count_skipped),
                &mut counts as *mut CallbackCounts as *mut c_void,
            );
            nl_socket_modify_cb(
                sk,
                NL_CB_VALID as c_int,
                0,
                Some(count_valid),
                &mut counts as *mut CallbackCounts as *mut c_void,
            );

            let mut datagram = Vec::new();
            push_nlmsg(&mut datagram, NLMSG_NOOP, 0, 1, &[]);
            push_nlmsg(&mut datagram, NLMSG_MIN_TYPE, 0, 1, &[]);
            send_datagram(peer_fd, &datagram);

            assert_eq!(nl_recvmsgs_default(sk), 0);
            assert_eq!(counts.skipped, 1);
            assert_eq!(counts.valid, 1);

            nl_socket_free(sk);
            libc::close(peer_fd);
        }
    }

    #[cfg(not(miri))]
    #[test]
    fn recv_sequence_mismatch_does_not_dispatch_valid_callback() {
        unsafe {
            let (sk, peer_fd) = socketpair_sock();
            (*sk).s_seq_expect = 100;
            let mut counts = CallbackCounts {
                invalid: 0,
                skipped: 0,
                valid: 0,
            };
            nl_socket_modify_cb(
                sk,
                NL_CB_VALID as c_int,
                0,
                Some(count_valid),
                &mut counts as *mut CallbackCounts as *mut c_void,
            );

            let mut datagram = Vec::new();
            push_nlmsg(&mut datagram, NLMSG_MIN_TYPE, 0, 99, &[]);
            send_datagram(peer_fd, &datagram);

            assert_eq!(nl_recvmsgs_default(sk), -(crate::error::NLE_SEQ_MISMATCH));
            assert_eq!(counts.valid, 0);

            nl_socket_free(sk);
            libc::close(peer_fd);
        }
    }
}
