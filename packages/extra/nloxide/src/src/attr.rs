// nla_* attribute functions derived from Ghidra analysis of libnl3-3.11.0
// nla_attr_size, nla_total_size, nla_padlen, nla_type, nla_data, nla_len,
// nla_ok, nla_next, nla_parse, nla_find, nla_put, nla_put_u8/u16/u32/u64,
// nla_get_u8/u16/u32/u64, nla_put_string, nla_put_flag, nla_nest_start/end/cancel, etc.

use core::ffi::{c_int, c_void};
use core::ptr;
use crate::types::*;
use crate::message::NlMsg;

pub use crate::types::NlAttr;

// ---- size helpers ----

#[no_mangle]
pub unsafe extern "C" fn nla_attr_size(payload: c_int) -> c_int {
    NLA_HDRLEN as c_int + payload
}

#[no_mangle]
pub unsafe extern "C" fn nla_total_size(payload: c_int) -> c_int {
    nla_align(nla_attr_size(payload) as usize) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_padlen(payload: c_int) -> c_int {
    nla_total_size(payload) - nla_attr_size(payload)
}

// ---- attribute header accessors ----

#[no_mangle]
pub unsafe extern "C" fn nla_type(attr: *const NlAttr) -> c_int {
    if attr.is_null() { return 0; }
    ((*attr).nla_type & NLA_TYPE_MASK) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_data(attr: *const NlAttr) -> *mut c_void {
    if attr.is_null() { return ptr::null_mut(); }
    (attr as *const u8).add(NLA_HDRLEN) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn nla_len(attr: *const NlAttr) -> c_int {
    if attr.is_null() { return 0; }
    (*attr).nla_len as c_int - NLA_HDRLEN as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_ok(attr: *const NlAttr, remaining: c_int) -> bool {
    if attr.is_null() || remaining < NLA_HDRLEN as i32 { return false; }
    let len = (*attr).nla_len as i32;
    len >= NLA_HDRLEN as i32 && len <= remaining
}

#[no_mangle]
pub unsafe extern "C" fn nla_next(attr: *const NlAttr, remaining: *mut c_int) -> *mut NlAttr {
    let step = nla_align((*attr).nla_len as usize) as i32;
    *remaining -= step;
    (attr as *const u8).add(step as usize) as *mut NlAttr
}

#[no_mangle]
pub unsafe extern "C" fn nla_is_nested(attr: *const NlAttr) -> bool {
    if attr.is_null() { return false; }
    (*attr).nla_type & NLA_F_NESTED != 0
}

// ---- attribute parsing ----

#[no_mangle]
pub unsafe extern "C" fn nla_parse(
    tb: *mut *mut NlAttr,
    maxtype: c_int,
    head: *const NlAttr,
    len: c_int,
    _policy: *const c_void,
) -> c_int {
    if !tb.is_null() && maxtype >= 0 {
        ptr::write_bytes(tb, 0, (maxtype + 1) as usize);
    }
    let mut pos = head as *const NlAttr;
    let mut rem = len;
    while nla_ok(pos, rem) {
        let t = nla_type(pos);
        if t <= maxtype && !tb.is_null() {
            *tb.add(t as usize) = pos as *mut NlAttr;
        }
        pos = nla_next(pos, &mut rem);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nla_parse_nested(
    tb: *mut *mut NlAttr,
    maxtype: c_int,
    attr: *const NlAttr,
    policy: *const c_void,
) -> c_int {
    nla_parse(tb, maxtype, nla_data(attr) as *const NlAttr, nla_len(attr), policy)
}

#[no_mangle]
pub unsafe extern "C" fn nla_validate(
    head: *const NlAttr,
    len: c_int,
    maxtype: c_int,
    _policy: *const c_void,
) -> c_int {
    let mut pos = head as *const NlAttr;
    let mut rem = len;
    while nla_ok(pos, rem) {
        let t = nla_type(pos);
        if t > maxtype {
            return -(crate::error::NLE_RANGE);
        }
        pos = nla_next(pos, &mut rem);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nla_find(
    head: *const NlAttr,
    len: c_int,
    attrtype: c_int,
) -> *mut NlAttr {
    let mut pos = head as *const NlAttr;
    let mut rem = len;
    while nla_ok(pos, rem) {
        if nla_type(pos) == attrtype {
            return pos as *mut NlAttr;
        }
        pos = nla_next(pos, &mut rem);
    }
    ptr::null_mut()
}

// ---- put helpers (write into nl_msg) ----

fn nla_reserve_raw(msg: *mut NlMsg, attrtype: c_int, attrlen: c_int) -> *mut NlAttr {
    unsafe {
        let total = nla_align(NLA_HDRLEN + attrlen as usize);
        let p = (*msg).reserve(total, 0);
        if p.is_null() { return ptr::null_mut(); }
        let attr = p as *mut NlAttr;
        (*attr).nla_len = (NLA_HDRLEN + attrlen as usize) as u16;
        (*attr).nla_type = attrtype as u16;
        attr
    }
}

#[no_mangle]
pub unsafe extern "C" fn nla_reserve(
    msg: *mut NlMsg,
    attrtype: c_int,
    attrlen: c_int,
) -> *mut NlAttr {
    if msg.is_null() { return ptr::null_mut(); }
    nla_reserve_raw(msg, attrtype, attrlen)
}

#[no_mangle]
pub unsafe extern "C" fn nla_put(
    msg: *mut NlMsg,
    attrtype: c_int,
    datalen: c_int,
    data: *const c_void,
) -> c_int {
    if msg.is_null() { return -(crate::error::NLE_INVAL); }
    let attr = nla_reserve_raw(msg, attrtype, datalen);
    if attr.is_null() { return -(crate::error::NLE_NOMEM); }
    if datalen > 0 && !data.is_null() {
        libc::memcpy(nla_data(attr), data, datalen as usize);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nla_put_data(
    msg: *mut NlMsg,
    attrtype: c_int,
    data: *const c_void, // nl_data*
) -> c_int {
    // nl_data: first field is size, second is pointer
    if data.is_null() { return -(crate::error::NLE_INVAL); }
    let size = *(data as *const usize);
    let ptr_ = *((data as *const u8).add(8) as *const *const c_void);
    nla_put(msg, attrtype, size as c_int, ptr_)
}

#[no_mangle]
pub unsafe extern "C" fn nla_put_addr(_msg: *mut NlMsg, _t: c_int, _addr: *const c_void) -> c_int {
    -(crate::error::NLE_OPNOTSUPP)
}

macro_rules! nla_put_int {
    ($name:ident, $get:ident, $ty:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(
            msg: *mut NlMsg,
            attrtype: c_int,
            value: $ty,
        ) -> c_int {
            nla_put(msg, attrtype, core::mem::size_of::<$ty>() as c_int,
                    &value as *const $ty as *const c_void)
        }
        #[no_mangle]
        pub unsafe extern "C" fn $get(attr: *const NlAttr) -> $ty {
            if attr.is_null() { return 0 as $ty; }
            let mut v: $ty = 0;
            let n = nla_len(attr) as usize;
            libc::memcpy(&mut v as *mut $ty as *mut c_void,
                         nla_data(attr), n.min(core::mem::size_of::<$ty>()));
            v
        }
    };
}

nla_put_int!(nla_put_u8,  nla_get_u8,  u8);
nla_put_int!(nla_put_u16, nla_get_u16, u16);
nla_put_int!(nla_put_u32, nla_get_u32, u32);
nla_put_int!(nla_put_u64, nla_get_u64, u64);
nla_put_int!(nla_put_s8,  nla_get_s8,  i8);
nla_put_int!(nla_put_s16, nla_get_s16, i16);
nla_put_int!(nla_put_s32, nla_get_s32, i32);
nla_put_int!(nla_put_s64, nla_get_s64, i64);

// uint/sint (arbitrary-width, Ghidra shows these as 64-bit in 3.11)
#[no_mangle]
pub unsafe extern "C" fn nla_put_uint(msg: *mut NlMsg, t: c_int, v: u64) -> c_int {
    nla_put_u64(msg, t, v)
}
#[no_mangle]
pub unsafe extern "C" fn nla_get_uint(attr: *const NlAttr) -> u64 {
    if attr.is_null() { return 0; }
    match nla_len(attr) {
        4 => nla_get_u32(attr) as u64,
        _ => nla_get_u64(attr),
    }
}
#[no_mangle]
pub unsafe extern "C" fn nla_put_sint(msg: *mut NlMsg, t: c_int, v: i64) -> c_int {
    nla_put_s64(msg, t, v)
}
#[no_mangle]
pub unsafe extern "C" fn nla_get_sint(attr: *const NlAttr) -> i64 {
    if attr.is_null() { return 0; }
    match nla_len(attr) {
        4 => nla_get_s32(attr) as i64,
        _ => nla_get_s64(attr),
    }
}

#[no_mangle]
pub unsafe extern "C" fn nla_put_string(
    msg: *mut NlMsg,
    attrtype: c_int,
    str_: *const u8,
) -> c_int {
    if str_.is_null() { return -(crate::error::NLE_INVAL); }
    let len = libc::strlen(str_ as _) + 1;
    nla_put(msg, attrtype, len as c_int, str_ as _)
}

#[no_mangle]
pub unsafe extern "C" fn nla_get_string(attr: *const NlAttr) -> *const u8 {
    nla_data(attr) as *const u8
}

#[no_mangle]
pub unsafe extern "C" fn nla_strdup(attr: *const NlAttr) -> *mut u8 {
    libc::strdup(nla_data(attr) as _) as *mut u8
}

#[no_mangle]
pub unsafe extern "C" fn nla_put_flag(msg: *mut NlMsg, attrtype: c_int) -> c_int {
    nla_put(msg, attrtype, 0, ptr::null())
}

#[no_mangle]
pub unsafe extern "C" fn nla_get_flag(attr: *const NlAttr) -> bool {
    !attr.is_null()
}

#[no_mangle]
pub unsafe extern "C" fn nla_put_msecs(msg: *mut NlMsg, t: c_int, v: u64) -> c_int {
    nla_put_u64(msg, t, v)
}
#[no_mangle]
pub unsafe extern "C" fn nla_get_msecs(attr: *const NlAttr) -> u64 {
    nla_get_u64(attr)
}

#[no_mangle]
pub unsafe extern "C" fn nla_put_nested(
    msg: *mut NlMsg,
    attrtype: c_int,
    nested: *const NlMsg,
) -> c_int {
    if msg.is_null() || nested.is_null() { return -(crate::error::NLE_INVAL); }
    let hdr = (*nested).hdr();
    let dlen = crate::message::nlmsg_datalen(hdr);
    nla_put(msg, attrtype, dlen, crate::message::nlmsg_data(hdr))
}

#[no_mangle]
pub unsafe extern "C" fn nla_nest_start(msg: *mut NlMsg, attrtype: c_int) -> *mut NlAttr {
    if msg.is_null() { return ptr::null_mut(); }
    let attr = nla_reserve_raw(msg, attrtype | NLA_F_NESTED as c_int, 0);
    attr
}

#[no_mangle]
pub unsafe extern "C" fn nla_nest_end(msg: *mut NlMsg, start: *mut NlAttr) -> c_int {
    if msg.is_null() || start.is_null() { return -(crate::error::NLE_INVAL); }
    let hdr = (*msg).hdr();
    let msg_start = (*msg).buf as usize;
    let attr_off = start as usize - msg_start;
    let tail = (*hdr).nlmsg_len as usize;
    (*start).nla_len = (tail - attr_off) as u16;
    (*start).nla_type |= NLA_F_NESTED;
    tail as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_nest_end_keep_empty(msg: *mut NlMsg, start: *mut NlAttr) -> c_int {
    nla_nest_end(msg, start)
}

#[no_mangle]
pub unsafe extern "C" fn nla_nest_cancel(msg: *mut NlMsg, start: *mut NlAttr) {
    if msg.is_null() || start.is_null() { return; }
    let msg_start = (*msg).buf as usize;
    let attr_off = start as usize - msg_start;
    let hdr = (*msg).hdr();
    (*hdr).nlmsg_len = attr_off as u32;
}

// ---- copy helpers ----

#[no_mangle]
pub unsafe extern "C" fn nla_memcpy(
    dst: *mut c_void,
    attr: *const NlAttr,
    count: c_int,
) -> c_int {
    if dst.is_null() || attr.is_null() { return 0; }
    let n = nla_len(attr).min(count);
    libc::memcpy(dst, nla_data(attr), n as usize);
    n
}

#[no_mangle]
pub unsafe extern "C" fn nla_strlcpy(
    dst: *mut u8,
    attr: *const NlAttr,
    dstsize: usize,
) -> usize {
    if dst.is_null() || attr.is_null() || dstsize == 0 { return 0; }
    let src = nla_data(attr) as *const u8;
    let srclen = nla_len(attr) as usize;
    let copy = srclen.min(dstsize - 1);
    libc::memcpy(dst as _, src as _, copy);
    *dst.add(copy) = 0;
    srclen
}

#[no_mangle]
pub unsafe extern "C" fn nla_memcmp(
    attr: *const NlAttr,
    data: *const c_void,
    size: usize,
) -> c_int {
    if attr.is_null() { return -1; }
    libc::memcmp(nla_data(attr), data, size)
}

#[no_mangle]
pub unsafe extern "C" fn nla_strcmp(attr: *const NlAttr, str_: *const u8) -> c_int {
    if attr.is_null() || str_.is_null() { return -1; }
    libc::strcmp(nla_data(attr) as _, str_ as _)
}
