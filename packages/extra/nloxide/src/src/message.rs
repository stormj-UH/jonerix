// nl_msg and nlmsg_* implementation derived from Ghidra analysis of libnl3-3.11.0
//
// From Ghidra:
//   nlmsg_hdr(msg)  → *(ptr)(msg+0x30)   = nlmsghdr pointer
//   nlmsg_free      → refcount at msg+0x40, buffer at msg+0x30
//
// The nl_msg holds a heap buffer containing [nlmsghdr][payload...].
// The msg object itself is heap-allocated; refcounted.

use core::ffi::{c_int, c_void};
use core::ptr;
use crate::types::*;

// Default buffer size (4096 bytes = typical page)
const NL_AUTO_SEQ: u32 = 0;
const NL_AUTO_PID: u32 = 0;
const DEFAULT_MSG_SIZE: usize = 4096;

pub struct NlMsg {
    pub proto: c_int,
    pub max_size: usize,
    pub src: SockaddrNl,
    pub dst: SockaddrNl,
    // heap buffer containing the actual netlink message
    pub buf: *mut u8,
    pub buf_size: usize,   // allocated capacity
    pub refcount: i32,
}

impl NlMsg {
    pub fn hdr(&self) -> *mut NlMsgHdr {
        self.buf as *mut NlMsgHdr
    }

    pub fn data_tail(&self) -> usize {
        unsafe { (*self.hdr()).nlmsg_len as usize }
    }

    // reserve `len` bytes after current message end; grow buffer if needed
    // returns pointer to start of reserved region or null
    pub fn reserve(&mut self, len: usize, pad: usize) -> *mut u8 {
        let hdr = unsafe { &mut *self.hdr() };
        let cur = hdr.nlmsg_len as usize;
        let aligned = nlmsg_align(len + pad);
        let need = cur + aligned;
        if need > self.buf_size {
            let new_size = if self.max_size > 0 {
                if need > self.max_size { return ptr::null_mut(); }
                self.max_size
            } else {
                need.max(DEFAULT_MSG_SIZE).next_power_of_two()
            };
            let new_buf = unsafe {
                libc::realloc(self.buf as *mut c_void, new_size) as *mut u8
            };
            if new_buf.is_null() { return ptr::null_mut(); }
            // zero new region
            unsafe {
                ptr::write_bytes(new_buf.add(self.buf_size), 0, new_size - self.buf_size);
            }
            self.buf = new_buf;
            self.buf_size = new_size;
        }
        let hdr = unsafe { &mut *self.hdr() };
        let start = hdr.nlmsg_len as usize;
        // zero-pad the new space
        unsafe { ptr::write_bytes(self.buf.add(start), 0, aligned); }
        hdr.nlmsg_len = (cur + aligned) as u32;
        unsafe { self.buf.add(start) }
    }
}

// --- exported C functions: nlmsg_* ---

#[no_mangle]
pub unsafe extern "C" fn nlmsg_alloc() -> *mut NlMsg {
    nlmsg_alloc_size(DEFAULT_MSG_SIZE)
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_alloc_size(size: usize) -> *mut NlMsg {
    let sz = if size == 0 { DEFAULT_MSG_SIZE } else { size };
    let buf = libc::calloc(1, sz) as *mut u8;
    if buf.is_null() { return ptr::null_mut(); }
    let msg = Box::new(NlMsg {
        proto: 0,
        max_size: 0,
        src: SockaddrNl::default(),
        dst: SockaddrNl::default(),
        buf,
        buf_size: sz,
        refcount: 1,
    });
    // initialise header length field so hdr() is valid
    let m = Box::into_raw(msg);
    (*((*m).buf as *mut NlMsgHdr)).nlmsg_len = NLMSG_HDRLEN as u32;
    m
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_alloc_simple(
    nlmsg_type: c_int,
    flags: c_int,
) -> *mut NlMsg {
    let msg = nlmsg_alloc();
    if msg.is_null() { return ptr::null_mut(); }
    if nlmsg_put(msg, NL_AUTO_PID, NL_AUTO_SEQ, nlmsg_type, 0, flags as u16).is_null() {
        nlmsg_free(msg);
        return ptr::null_mut();
    }
    msg
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_inherit(hdr: *const NlMsgHdr) -> *mut NlMsg {
    let msg = nlmsg_alloc();
    if msg.is_null() { return ptr::null_mut(); }
    if !hdr.is_null() {
        let src = &*hdr;
        let dst = &mut *((*msg).buf as *mut NlMsgHdr);
        *dst = *src;
        (*msg).proto = 0;
    }
    msg
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_convert(hdr: *mut NlMsgHdr) -> *mut NlMsg {
    let size = (*hdr).nlmsg_len as usize;
    let sz = size.max(DEFAULT_MSG_SIZE);
    let buf = libc::malloc(sz) as *mut u8;
    if buf.is_null() { return ptr::null_mut(); }
    libc::memcpy(buf as _, hdr as _, size);
    libc::memset(buf.add(size) as _, 0, sz - size);
    let msg = Box::new(NlMsg {
        proto: 0,
        max_size: sz,
        src: SockaddrNl::default(),
        dst: SockaddrNl::default(),
        buf,
        buf_size: sz,
        refcount: 1,
    });
    Box::into_raw(msg)
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_set_default_size(size: usize) {
    // global default — not tracked; no-op for now
    let _ = size;
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_get(msg: *mut NlMsg) -> *mut NlMsg {
    if !msg.is_null() {
        (*msg).refcount += 1;
    }
    msg
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_free(msg: *mut NlMsg) {
    if msg.is_null() { return; }
    (*msg).refcount -= 1;
    if (*msg).refcount <= 0 {
        libc::free((*msg).buf as _);
        drop(Box::from_raw(msg));
    }
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_hdr(msg: *mut NlMsg) -> *mut NlMsgHdr {
    if msg.is_null() { return ptr::null_mut(); }
    (*msg).hdr()
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_data(hdr: *const NlMsgHdr) -> *mut c_void {
    if hdr.is_null() { return ptr::null_mut(); }
    unsafe { (hdr as *const u8).add(NLMSG_HDRLEN) as *mut c_void }
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_tail(hdr: *const NlMsgHdr) -> *mut c_void {
    if hdr.is_null() { return ptr::null_mut(); }
    (hdr as *const u8).add(nlmsg_align((*hdr).nlmsg_len as usize)) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_datalen(hdr: *const NlMsgHdr) -> c_int {
    if hdr.is_null() { return 0; }
    ((*hdr).nlmsg_len as usize).saturating_sub(NLMSG_HDRLEN) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_attrdata(hdr: *const NlMsgHdr, hdrlen: c_int) -> *mut crate::attr::NlAttr {
    let data = nlmsg_data(hdr) as *const u8;
    data.add(nlmsg_align(hdrlen as usize)) as *mut crate::attr::NlAttr
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_attrlen(hdr: *const NlMsgHdr, hdrlen: c_int) -> c_int {
    let dl = nlmsg_datalen(hdr);
    dl - nlmsg_align(hdrlen as usize) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_size(payload: usize) -> usize {
    NLMSG_HDRLEN + payload
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_total_size(payload: usize) -> usize {
    nlmsg_align(nlmsg_size(payload))
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_padlen(payload: usize) -> usize {
    nlmsg_total_size(payload) - nlmsg_size(payload)
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_valid_hdr(hdr: *const NlMsgHdr, hdrlen: c_int) -> bool {
    if hdr.is_null() { return false; }
    (*hdr).nlmsg_len >= nlmsg_size(hdrlen as usize) as u32
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_ok(hdr: *const NlMsgHdr, remaining: c_int) -> bool {
    if hdr.is_null() || remaining < NLMSG_HDRLEN as i32 { return false; }
    let len = (*hdr).nlmsg_len;
    len >= NLMSG_HDRLEN as u32 && len <= remaining as u32
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_next(hdr: *const NlMsgHdr, remaining: *mut c_int) -> *mut NlMsgHdr {
    let len = nlmsg_align((*hdr).nlmsg_len as usize) as i32;
    *remaining -= len;
    (hdr as *const u8).add(len as usize) as *mut NlMsgHdr
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_put(
    msg: *mut NlMsg,
    pid: u32,
    seq: u32,
    nlmsg_type: c_int,
    payload: c_int,
    flags: u16,
) -> *mut NlMsgHdr {
    if msg.is_null() { return ptr::null_mut(); }
    // The header is at buf[0]; we need to write it and reserve space for payload
    let m = &mut *msg;
    // reset length to 0 to start fresh (nlmsg_put initialises the whole message)
    let hdr = m.buf as *mut NlMsgHdr;
    (*hdr).nlmsg_len = NLMSG_HDRLEN as u32;
    (*hdr).nlmsg_type = nlmsg_type as u16;
    (*hdr).nlmsg_flags = flags;
    (*hdr).nlmsg_seq = seq;
    (*hdr).nlmsg_pid = pid;
    if payload > 0 {
        if m.reserve(payload as usize, 0).is_null() {
            return ptr::null_mut();
        }
    }
    hdr
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_reserve(
    msg: *mut NlMsg,
    len: usize,
    pad: usize,
) -> *mut c_void {
    if msg.is_null() { return ptr::null_mut(); }
    (*msg).reserve(len, pad) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_append(
    msg: *mut NlMsg,
    data: *const c_void,
    len: usize,
    pad: usize,
) -> c_int {
    if msg.is_null() { return -(crate::error::NLE_INVAL); }
    let p = (*msg).reserve(len, pad);
    if p.is_null() { return -(crate::error::NLE_NOMEM); }
    if !data.is_null() && len > 0 {
        libc::memcpy(p as _, data, len);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_expand(msg: *mut NlMsg, addlen: usize) -> c_int {
    if msg.is_null() { return -(crate::error::NLE_INVAL); }
    let m = &mut *msg;
    let need = m.buf_size + addlen;
    let new_buf = libc::realloc(m.buf as _, need) as *mut u8;
    if new_buf.is_null() { return -(crate::error::NLE_NOMEM); }
    m.buf = new_buf;
    m.buf_size = need;
    0
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_parse(
    hdr: *const NlMsgHdr,
    hdrlen: c_int,
    tb: *mut *mut crate::attr::NlAttr,
    maxtype: c_int,
    _policy: *const c_void,
) -> c_int {
    let attrdata = nlmsg_attrdata(hdr, hdrlen);
    let attrlen = nlmsg_attrlen(hdr, hdrlen);
    crate::attr::nla_parse(tb, maxtype, attrdata, attrlen, _policy)
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_find_attr(
    hdr: *const NlMsgHdr,
    hdrlen: c_int,
    attrtype: c_int,
) -> *mut crate::attr::NlAttr {
    let attrdata = nlmsg_attrdata(hdr, hdrlen);
    let attrlen = nlmsg_attrlen(hdr, hdrlen);
    crate::attr::nla_find(attrdata, attrlen, attrtype)
}

#[no_mangle]
pub unsafe extern "C" fn nlmsg_validate(
    hdr: *const NlMsgHdr,
    hdrlen: c_int,
    maxtype: c_int,
    _policy: *const c_void,
) -> c_int {
    let attrdata = nlmsg_attrdata(hdr, hdrlen);
    let attrlen = nlmsg_attrlen(hdr, hdrlen);
    crate::attr::nla_validate(attrdata, attrlen, maxtype, _policy)
}

// metadata accessors
#[no_mangle]
pub unsafe extern "C" fn nlmsg_get_proto(msg: *const NlMsg) -> c_int {
    if msg.is_null() { return 0; }
    (*msg).proto
}
#[no_mangle]
pub unsafe extern "C" fn nlmsg_set_proto(msg: *mut NlMsg, proto: c_int) {
    if !msg.is_null() { (*msg).proto = proto; }
}
#[no_mangle]
pub unsafe extern "C" fn nlmsg_get_max_size(msg: *const NlMsg) -> usize {
    if msg.is_null() { return 0; }
    (*msg).max_size
}
#[no_mangle]
pub unsafe extern "C" fn nlmsg_set_src(msg: *mut NlMsg, addr: *const SockaddrNl) {
    if !msg.is_null() && !addr.is_null() { (*msg).src = *addr; }
}
#[no_mangle]
pub unsafe extern "C" fn nlmsg_get_src(msg: *mut NlMsg) -> *mut SockaddrNl {
    if msg.is_null() { return ptr::null_mut(); }
    &mut (*msg).src
}
#[no_mangle]
pub unsafe extern "C" fn nlmsg_set_dst(msg: *mut NlMsg, addr: *const SockaddrNl) {
    if !msg.is_null() && !addr.is_null() { (*msg).dst = *addr; }
}
#[no_mangle]
pub unsafe extern "C" fn nlmsg_get_dst(msg: *mut NlMsg) -> *mut SockaddrNl {
    if msg.is_null() { return ptr::null_mut(); }
    &mut (*msg).dst
}
#[no_mangle]
pub unsafe extern "C" fn nlmsg_set_creds(_msg: *mut NlMsg, _creds: *const c_void) {}
#[no_mangle]
pub unsafe extern "C" fn nlmsg_get_creds(_msg: *mut NlMsg) -> *mut c_void { ptr::null_mut() }

#[no_mangle]
pub unsafe extern "C" fn nl_msg_parse(
    _msg: *mut NlMsg,
    _parser: *mut c_void,
    _arg: *mut c_void,
) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn nl_msg_dump(_msg: *mut NlMsg, _fp: *mut libc::FILE) {}

// nl_nlmsgtype2str etc. — stub string functions
#[no_mangle]
pub unsafe extern "C" fn nl_nlmsgtype2str(
    _t: u16, buf: *mut u8, size: usize,
) -> *mut u8 {
    if !buf.is_null() && size > 0 { *buf = 0; }
    buf
}
#[no_mangle]
pub unsafe extern "C" fn nl_str2nlmsgtype(_s: *const u8) -> c_int { -1 }
#[no_mangle]
pub unsafe extern "C" fn nl_nlmsg_flags2str(_f: u16, _buf: *mut u8, _size: usize) -> *mut u8 {
    ptr::null_mut()
}
