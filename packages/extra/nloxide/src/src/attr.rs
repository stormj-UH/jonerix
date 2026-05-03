// nla_* attribute functions derived from Ghidra analysis of libnl3-3.11.0
// nla_attr_size, nla_total_size, nla_padlen, nla_type, nla_data, nla_len,
// nla_ok, nla_next, nla_parse, nla_find, nla_put, nla_put_u8/u16/u32/u64,
// nla_get_u8/u16/u32/u64, nla_put_string, nla_put_flag, nla_nest_start/end/cancel, etc.

use crate::message::NlMsg;
use crate::types::*;
use core::ffi::{c_int, c_void};
use core::{ptr, slice};

pub use crate::types::NlAttr;

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct NlaPolicy {
    type_: u16,
    minlen: u16,
    maxlen: u16,
}

const NLA_UNSPEC: u16 = 0;
const NLA_U8: u16 = 1;
const NLA_U16: u16 = 2;
const NLA_U32: u16 = 3;
const NLA_U64: u16 = 4;
const NLA_STRING: u16 = 5;
const NLA_FLAG: u16 = 6;
const NLA_MSECS: u16 = 7;
const NLA_NESTED: u16 = 8;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct AttrView {
    offset: usize,
    attr_type: c_int,
    payload_offset: usize,
    payload_len: usize,
    aligned_len: usize,
}

fn attr_payload_len(attr: *const NlAttr) -> usize {
    unsafe {
        if attr.is_null() || (*attr).nla_len < NLA_HDRLEN as u16 {
            return 0;
        }
        ((*attr).nla_len as usize).saturating_sub(NLA_HDRLEN)
    }
}

fn read_u16_ne(buf: &[u8], offset: usize) -> Option<u16> {
    let bytes = buf.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_ne_bytes([bytes[0], bytes[1]]))
}

fn parse_attr_at(buf: &[u8], offset: usize) -> Result<AttrView, c_int> {
    let hdr_end = offset
        .checked_add(NLA_HDRLEN)
        .ok_or(-(crate::error::NLE_INVAL))?;
    if hdr_end > buf.len() {
        return Err(-(crate::error::NLE_INVAL));
    }

    let attr_len = read_u16_ne(buf, offset).ok_or(-(crate::error::NLE_INVAL))? as usize;
    let raw_type = read_u16_ne(buf, offset + 2).ok_or(-(crate::error::NLE_INVAL))?;
    if attr_len < NLA_HDRLEN {
        return Err(-(crate::error::NLE_INVAL));
    }

    let end = offset
        .checked_add(attr_len)
        .ok_or(-(crate::error::NLE_INVAL))?;
    if end > buf.len() {
        return Err(-(crate::error::NLE_INVAL));
    }

    let aligned_len = nla_align(attr_len);
    if aligned_len < attr_len {
        return Err(-(crate::error::NLE_INVAL));
    }

    Ok(AttrView {
        offset,
        attr_type: (raw_type & NLA_TYPE_MASK) as c_int,
        payload_offset: hdr_end,
        payload_len: attr_len - NLA_HDRLEN,
        aligned_len,
    })
}

fn validate_attr_view(
    buf: &[u8],
    attr: AttrView,
    maxtype: c_int,
    policy: Option<&[NlaPolicy]>,
) -> Result<(), c_int> {
    let t = attr.attr_type;
    if t == 0 || t > maxtype {
        return Ok(());
    }

    let Some(policy) = policy else {
        return Ok(());
    };
    let pol = policy.get(t as usize).ok_or(-(crate::error::NLE_INVAL))?;
    let payload = buf
        .get(attr.payload_offset..attr.payload_offset + attr.payload_len)
        .ok_or(-(crate::error::NLE_INVAL))?;

    if pol.minlen != 0 && payload.len() < pol.minlen as usize {
        return Err(-(crate::error::NLE_ATTRSIZE));
    }
    if pol.maxlen != 0 && payload.len() > pol.maxlen as usize {
        return Err(-(crate::error::NLE_ATTRSIZE));
    }

    match pol.type_ {
        NLA_UNSPEC => Ok(()),
        NLA_U8 => validate_exact_payload(payload.len(), core::mem::size_of::<u8>()),
        NLA_U16 => validate_exact_payload(payload.len(), core::mem::size_of::<u16>()),
        NLA_U32 => validate_exact_payload(payload.len(), core::mem::size_of::<u32>()),
        NLA_U64 | NLA_MSECS => validate_exact_payload(payload.len(), core::mem::size_of::<u64>()),
        NLA_STRING => validate_string_payload(payload),
        NLA_FLAG => validate_exact_payload(payload.len(), 0),
        NLA_NESTED => {
            if !payload.is_empty() && payload.len() < NLA_HDRLEN {
                Err(-(crate::error::NLE_ATTRSIZE))
            } else {
                Ok(())
            }
        }
        _ => Err(-(crate::error::NLE_RANGE)),
    }
}

fn validate_exact_payload(payload_len: usize, expected: usize) -> Result<(), c_int> {
    if payload_len == expected {
        Ok(())
    } else {
        Err(-(crate::error::NLE_ATTRSIZE))
    }
}

fn validate_string_payload(payload: &[u8]) -> Result<(), c_int> {
    if payload.is_empty() {
        return Err(-(crate::error::NLE_ATTRSIZE));
    }
    if payload.contains(&0) {
        Ok(())
    } else {
        Err(-(crate::error::NLE_INVAL))
    }
}

fn parse_attrs_bytes(
    buf: &[u8],
    maxtype: c_int,
    policy: Option<&[NlaPolicy]>,
) -> Result<Vec<AttrView>, c_int> {
    if maxtype < 0 {
        return Err(-(crate::error::NLE_INVAL));
    }

    let mut attrs = Vec::new();
    let mut offset = 0usize;
    while offset < buf.len() {
        if buf.len() - offset < NLA_HDRLEN {
            break;
        }

        let attr = parse_attr_at(buf, offset)?;
        validate_attr_view(buf, attr, maxtype, policy)?;
        attrs.push(attr);

        let next = offset
            .checked_add(attr.aligned_len)
            .ok_or(-(crate::error::NLE_INVAL))?;
        if next <= offset {
            return Err(-(crate::error::NLE_INVAL));
        }
        if next > buf.len() {
            break;
        }
        offset = next;
    }
    Ok(attrs)
}

unsafe fn attr_slice<'a>(head: *const NlAttr, len: c_int) -> Result<&'a [u8], c_int> {
    if len < 0 {
        return Err(-(crate::error::NLE_INVAL));
    }
    if len == 0 {
        return Ok(&[]);
    }
    if head.is_null() {
        return Err(-(crate::error::NLE_INVAL));
    }
    Ok(slice::from_raw_parts(head as *const u8, len as usize))
}

unsafe fn policy_slice<'a>(
    policy: *const c_void,
    maxtype: c_int,
) -> Result<Option<&'a [NlaPolicy]>, c_int> {
    if maxtype < 0 {
        return Err(-(crate::error::NLE_INVAL));
    }
    if policy.is_null() {
        return Ok(None);
    }
    let len = (maxtype as usize)
        .checked_add(1)
        .ok_or(-(crate::error::NLE_INVAL))?;
    Ok(Some(slice::from_raw_parts(policy as *const NlaPolicy, len)))
}

unsafe fn parse_attrs_from_raw<'a>(
    head: *const NlAttr,
    len: c_int,
    maxtype: c_int,
    policy: *const c_void,
) -> Result<(&'a [u8], Vec<AttrView>), c_int> {
    let bytes = attr_slice(head, len)?;
    let policy = policy_slice(policy, maxtype)?;
    let attrs = parse_attrs_bytes(bytes, maxtype, policy)?;
    Ok((bytes, attrs))
}

unsafe fn attr_ptr_at(head: *const NlAttr, offset: usize) -> *mut NlAttr {
    (head as *const u8).add(offset) as *mut NlAttr
}

// ---- size helpers ----

#[no_mangle]
pub unsafe extern "C" fn nla_attr_size(payload: c_int) -> c_int {
    if payload < 0 {
        return 0;
    }
    (NLA_HDRLEN + payload as usize).min(c_int::MAX as usize) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_total_size(payload: c_int) -> c_int {
    if payload < 0 {
        return 0;
    }
    nla_align(nla_attr_size(payload) as usize).min(c_int::MAX as usize) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_padlen(payload: c_int) -> c_int {
    if payload < 0 {
        return 0;
    }
    nla_total_size(payload) - nla_attr_size(payload)
}

// ---- attribute header accessors ----

#[no_mangle]
pub unsafe extern "C" fn nla_type(attr: *const NlAttr) -> c_int {
    if attr.is_null() {
        return 0;
    }
    ((*attr).nla_type & NLA_TYPE_MASK) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_data(attr: *const NlAttr) -> *mut c_void {
    if attr.is_null() {
        return ptr::null_mut();
    }
    (attr as *const u8).add(NLA_HDRLEN) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn nla_len(attr: *const NlAttr) -> c_int {
    if attr.is_null() {
        return 0;
    }
    (*attr).nla_len as c_int - NLA_HDRLEN as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_ok(attr: *const NlAttr, remaining: c_int) -> bool {
    if attr.is_null() || remaining < NLA_HDRLEN as i32 {
        return false;
    }
    let len = (*attr).nla_len as i32;
    len >= NLA_HDRLEN as i32 && len <= remaining
}

#[no_mangle]
pub unsafe extern "C" fn nla_next(attr: *const NlAttr, remaining: *mut c_int) -> *mut NlAttr {
    if attr.is_null() || remaining.is_null() {
        return ptr::null_mut();
    }
    let step = nla_align((*attr).nla_len as usize) as i32;
    *remaining = (*remaining).saturating_sub(step);
    (attr as *const u8).add(step as usize) as *mut NlAttr
}

#[no_mangle]
pub unsafe extern "C" fn nla_is_nested(attr: *const NlAttr) -> bool {
    if attr.is_null() {
        return false;
    }
    (*attr).nla_type & NLA_F_NESTED != 0
}

// ---- attribute parsing ----

#[no_mangle]
pub unsafe extern "C" fn nla_parse(
    tb: *mut *mut NlAttr,
    maxtype: c_int,
    head: *const NlAttr,
    len: c_int,
    policy: *const c_void,
) -> c_int {
    if maxtype < 0 {
        return -(crate::error::NLE_INVAL);
    }
    if !tb.is_null() && maxtype >= 0 {
        ptr::write_bytes(tb, 0, (maxtype + 1) as usize);
    }
    let attrs = match parse_attrs_from_raw(head, len, maxtype, policy) {
        Ok((_bytes, attrs)) => attrs,
        Err(err) => return err,
    };
    for attr in attrs {
        let t = attr.attr_type;
        if t > 0 && t <= maxtype && !tb.is_null() {
            *tb.add(t as usize) = attr_ptr_at(head, attr.offset);
        }
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
    nla_parse(
        tb,
        maxtype,
        nla_data(attr) as *const NlAttr,
        nla_len(attr),
        policy,
    )
}

#[no_mangle]
pub unsafe extern "C" fn nla_validate(
    head: *const NlAttr,
    len: c_int,
    maxtype: c_int,
    policy: *const c_void,
) -> c_int {
    match parse_attrs_from_raw(head, len, maxtype, policy) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

#[no_mangle]
pub unsafe extern "C" fn nla_find(head: *const NlAttr, len: c_int, attrtype: c_int) -> *mut NlAttr {
    let Ok((_bytes, attrs)) = parse_attrs_from_raw(head, len, u16::MAX as c_int, ptr::null())
    else {
        return ptr::null_mut();
    };
    for attr in attrs {
        if attr.attr_type == attrtype {
            return attr_ptr_at(head, attr.offset);
        }
    }
    ptr::null_mut()
}

// ---- put helpers (write into nl_msg) ----

fn nla_reserve_raw(msg: *mut NlMsg, attrtype: c_int, attrlen: c_int) -> *mut NlAttr {
    unsafe {
        if attrlen < 0 || attrlen as usize > u16::MAX as usize - NLA_HDRLEN {
            return ptr::null_mut();
        }
        let total = nla_align(NLA_HDRLEN + attrlen as usize);
        let p = (*msg).reserve(total, 0);
        if p.is_null() {
            return ptr::null_mut();
        }
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
    if msg.is_null() || attrlen < 0 {
        return ptr::null_mut();
    }
    nla_reserve_raw(msg, attrtype, attrlen)
}

#[no_mangle]
pub unsafe extern "C" fn nla_put(
    msg: *mut NlMsg,
    attrtype: c_int,
    datalen: c_int,
    data: *const c_void,
) -> c_int {
    if msg.is_null() || datalen < 0 {
        return -(crate::error::NLE_INVAL);
    }
    let attr = nla_reserve_raw(msg, attrtype, datalen);
    if attr.is_null() {
        return -(crate::error::NLE_NOMEM);
    }
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
    if data.is_null() {
        return -(crate::error::NLE_INVAL);
    }
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
        pub unsafe extern "C" fn $name(msg: *mut NlMsg, attrtype: c_int, value: $ty) -> c_int {
            nla_put(
                msg,
                attrtype,
                core::mem::size_of::<$ty>() as c_int,
                &value as *const $ty as *const c_void,
            )
        }
        #[no_mangle]
        pub unsafe extern "C" fn $get(attr: *const NlAttr) -> $ty {
            if attr.is_null() {
                return 0 as $ty;
            }
            let mut v: $ty = 0;
            let n = attr_payload_len(attr);
            libc::memcpy(
                &mut v as *mut $ty as *mut c_void,
                nla_data(attr),
                n.min(core::mem::size_of::<$ty>()),
            );
            v
        }
    };
}

nla_put_int!(nla_put_u8, nla_get_u8, u8);
nla_put_int!(nla_put_u16, nla_get_u16, u16);
nla_put_int!(nla_put_u32, nla_get_u32, u32);
nla_put_int!(nla_put_u64, nla_get_u64, u64);
nla_put_int!(nla_put_s8, nla_get_s8, i8);
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
    if attr.is_null() {
        return 0;
    }
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
    if attr.is_null() {
        return 0;
    }
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
    if str_.is_null() {
        return -(crate::error::NLE_INVAL);
    }
    let len = libc::strlen(str_ as _) + 1;
    nla_put(msg, attrtype, len as c_int, str_ as _)
}

#[no_mangle]
pub unsafe extern "C" fn nla_get_string(attr: *const NlAttr) -> *const u8 {
    nla_data(attr) as *const u8
}

#[no_mangle]
pub unsafe extern "C" fn nla_strdup(attr: *const NlAttr) -> *mut u8 {
    if attr.is_null() {
        return ptr::null_mut();
    }
    let src = nla_data(attr) as *const u8;
    let payload_len = attr_payload_len(attr);
    let mut len = payload_len;
    for i in 0..payload_len {
        if *src.add(i) == 0 {
            len = i;
            break;
        }
    }
    let dst = libc::malloc(len + 1) as *mut u8;
    if dst.is_null() {
        return ptr::null_mut();
    }
    if len > 0 {
        libc::memcpy(dst as _, src as _, len);
    }
    *dst.add(len) = 0;
    dst
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
    if msg.is_null() || nested.is_null() {
        return -(crate::error::NLE_INVAL);
    }
    let hdr = (*nested).hdr();
    let dlen = crate::message::nlmsg_datalen(hdr);
    nla_put(msg, attrtype, dlen, crate::message::nlmsg_data(hdr))
}

#[no_mangle]
pub unsafe extern "C" fn nla_nest_start(msg: *mut NlMsg, attrtype: c_int) -> *mut NlAttr {
    if msg.is_null() {
        return ptr::null_mut();
    }
    nla_reserve_raw(msg, attrtype | NLA_F_NESTED as c_int, 0)
}

#[no_mangle]
pub unsafe extern "C" fn nla_nest_end(msg: *mut NlMsg, start: *mut NlAttr) -> c_int {
    if msg.is_null() || start.is_null() {
        return -(crate::error::NLE_INVAL);
    }
    let hdr = (*msg).hdr();
    let msg_start = (*msg).buf as usize;
    let start_addr = start as usize;
    let tail = (*hdr).nlmsg_len as usize;
    if start_addr < msg_start || start_addr > msg_start.saturating_add(tail) {
        return -(crate::error::NLE_INVAL);
    }
    let attr_off = start_addr - msg_start;
    if tail.saturating_sub(attr_off) > u16::MAX as usize {
        return -(crate::error::NLE_MSGSIZE);
    }
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
    if msg.is_null() || start.is_null() {
        return;
    }
    let msg_start = (*msg).buf as usize;
    let start_addr = start as usize;
    let hdr = (*msg).hdr();
    let tail = (*hdr).nlmsg_len as usize;
    if start_addr < msg_start || start_addr > msg_start.saturating_add(tail) {
        return;
    }
    let attr_off = start_addr - msg_start;
    (*hdr).nlmsg_len = attr_off as u32;
}

// ---- copy helpers ----

#[no_mangle]
pub unsafe extern "C" fn nla_memcpy(dst: *mut c_void, attr: *const NlAttr, count: c_int) -> c_int {
    if dst.is_null() || attr.is_null() || count <= 0 {
        return 0;
    }
    let n = attr_payload_len(attr).min(count as usize);
    if n == 0 {
        return 0;
    }
    libc::memcpy(dst, nla_data(attr), n);
    n as c_int
}

#[no_mangle]
pub unsafe extern "C" fn nla_strlcpy(dst: *mut u8, attr: *const NlAttr, dstsize: usize) -> usize {
    if dst.is_null() || attr.is_null() || dstsize == 0 {
        return 0;
    }
    let src = nla_data(attr) as *const u8;
    let payload_len = attr_payload_len(attr);
    let mut srclen = payload_len;
    for i in 0..payload_len {
        if *src.add(i) == 0 {
            srclen = i;
            break;
        }
    }
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
    if attr.is_null() {
        return -1;
    }
    let attr_len = attr_payload_len(attr);
    let cmp_len = attr_len.min(size);
    if cmp_len > 0 {
        if data.is_null() {
            return 1;
        }
        let ret = libc::memcmp(nla_data(attr), data, cmp_len);
        if ret != 0 {
            return ret;
        }
    }
    if attr_len == size {
        0
    } else if attr_len < size {
        -1
    } else {
        1
    }
}

#[no_mangle]
pub unsafe extern "C" fn nla_strcmp(attr: *const NlAttr, str_: *const u8) -> c_int {
    if attr.is_null() || str_.is_null() {
        return -1;
    }
    let data = nla_data(attr) as *const u8;
    let payload_len = attr_payload_len(attr);
    let mut i = 0usize;
    loop {
        let a = if i < payload_len { *data.add(i) } else { 0 };
        let b = *str_.add(i);
        if a != b {
            return a as c_int - b as c_int;
        }
        if a == 0 {
            return 0;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{nlmsg_alloc, nlmsg_data, nlmsg_datalen, nlmsg_free, nlmsg_hdr};

    unsafe fn msg_attrs(msg: *mut NlMsg) -> (*const NlAttr, c_int) {
        let hdr = nlmsg_hdr(msg);
        (nlmsg_data(hdr) as *const NlAttr, nlmsg_datalen(hdr))
    }

    #[test]
    fn parse_enforces_policy_and_ignores_out_of_range_types() {
        unsafe {
            let msg = nlmsg_alloc();
            assert!(!msg.is_null());
            assert_eq!(nla_put_u8(msg, 1, 7), 0);
            assert_eq!(nla_put_u32(msg, 7, 9), 0);

            let mut policy = [NlaPolicy::default(); 2];
            policy[1] = NlaPolicy {
                type_: NLA_U8,
                minlen: 0,
                maxlen: 0,
            };
            let mut tb = [ptr::null_mut(); 2];
            let (head, len) = msg_attrs(msg);

            assert_eq!(
                nla_parse(
                    tb.as_mut_ptr(),
                    1,
                    head,
                    len,
                    policy.as_ptr() as *const c_void
                ),
                0
            );
            assert!(!tb[1].is_null());
            nlmsg_free(msg);
        }
    }

    #[test]
    fn validate_rejects_policy_size_and_string_violations() {
        unsafe {
            let msg = nlmsg_alloc();
            assert!(!msg.is_null());
            let raw = b"abc";
            assert_eq!(
                nla_put(msg, 1, raw.len() as c_int, raw.as_ptr() as *const c_void),
                0
            );
            assert_eq!(nla_put_u8(msg, 2, 1), 0);

            let mut policy = [NlaPolicy::default(); 3];
            policy[1] = NlaPolicy {
                type_: NLA_STRING,
                minlen: 0,
                maxlen: 0,
            };
            policy[2] = NlaPolicy {
                type_: NLA_U32,
                minlen: 0,
                maxlen: 0,
            };
            let (head, len) = msg_attrs(msg);

            assert_eq!(
                nla_validate(head, len, 2, policy.as_ptr() as *const c_void),
                -(crate::error::NLE_INVAL)
            );

            policy[1] = NlaPolicy::default();
            assert_eq!(
                nla_validate(head, len, 2, policy.as_ptr() as *const c_void),
                -(crate::error::NLE_ATTRSIZE)
            );
            nlmsg_free(msg);
        }
    }

    #[test]
    fn string_helpers_are_bounded_by_payload_length() {
        unsafe {
            let msg = nlmsg_alloc();
            assert!(!msg.is_null());
            let raw = b"abc";
            assert_eq!(
                nla_put(msg, 1, raw.len() as c_int, raw.as_ptr() as *const c_void),
                0
            );
            let attr = nla_find(msg_attrs(msg).0, msg_attrs(msg).1, 1);

            let dup = nla_strdup(attr);
            assert!(!dup.is_null());
            assert_eq!(libc::strcmp(dup as *const libc::c_char, c"abc".as_ptr()), 0);
            assert_eq!(nla_strcmp(attr, c"abc".as_ptr() as *const u8), 0);
            assert!(nla_strcmp(attr, c"abcd".as_ptr() as *const u8) < 0);
            libc::free(dup as *mut c_void);
            nlmsg_free(msg);
        }
    }

    #[test]
    fn copy_helpers_bound_lengths() {
        unsafe {
            let msg = nlmsg_alloc();
            assert!(!msg.is_null());
            let raw = b"abc";
            assert_eq!(
                nla_put(msg, 1, raw.len() as c_int, raw.as_ptr() as *const c_void),
                0
            );
            let attr = nla_find(msg_attrs(msg).0, msg_attrs(msg).1, 1);
            let mut dst = [0u8; 8];

            assert_eq!(nla_memcpy(dst.as_mut_ptr() as *mut c_void, attr, -1), 0);
            assert_eq!(nla_memcpy(dst.as_mut_ptr() as *mut c_void, attr, 8), 3);
            assert_eq!(&dst[..3], raw);
            assert_eq!(
                nla_memcmp(attr, raw.as_ptr() as *const c_void, raw.len()),
                0
            );
            assert_ne!(
                nla_memcmp(attr, raw.as_ptr() as *const c_void, raw.len() + 1),
                0
            );
            nlmsg_free(msg);
        }
    }

    #[test]
    fn rejects_negative_attribute_lengths() {
        unsafe {
            let msg = nlmsg_alloc();
            assert!(!msg.is_null());
            assert!(nla_reserve(msg, 1, -1).is_null());
            assert_eq!(nla_put(msg, 1, -1, ptr::null()), -(crate::error::NLE_INVAL));
            nlmsg_free(msg);
        }
    }

    #[test]
    fn size_helpers_reject_negative_payloads() {
        unsafe {
            assert_eq!(nla_attr_size(-1), 0);
            assert_eq!(nla_total_size(-1), 0);
            assert_eq!(nla_padlen(-1), 0);
        }
    }

    #[test]
    fn nest_helpers_reject_foreign_start_pointer() {
        unsafe {
            let msg = nlmsg_alloc();
            assert!(!msg.is_null());
            let mut foreign = NlAttr::default();

            assert_eq!(nla_nest_end(msg, &mut foreign), -(crate::error::NLE_INVAL));
            nla_nest_cancel(msg, &mut foreign);

            nlmsg_free(msg);
        }
    }

    fn push_attr(buf: &mut Vec<u8>, attr_type: u16, payload: &[u8]) {
        let len = (NLA_HDRLEN + payload.len()) as u16;
        buf.extend_from_slice(&len.to_ne_bytes());
        buf.extend_from_slice(&attr_type.to_ne_bytes());
        buf.extend_from_slice(payload);
        while !buf.len().is_multiple_of(4) {
            buf.push(0);
        }
    }

    #[test]
    fn byte_parser_walks_valid_attributes_by_offset() {
        let mut buf = Vec::new();
        push_attr(&mut buf, 1, &[9]);
        push_attr(&mut buf, 2, b"ok\0");

        let mut policy = [NlaPolicy::default(); 3];
        policy[1] = NlaPolicy {
            type_: NLA_U8,
            minlen: 0,
            maxlen: 0,
        };
        policy[2] = NlaPolicy {
            type_: NLA_STRING,
            minlen: 0,
            maxlen: 0,
        };

        let attrs = parse_attrs_bytes(&buf, 2, Some(&policy)).unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].offset, 0);
        assert_eq!(attrs[0].payload_len, 1);
        assert_eq!(attrs[1].attr_type, 2);
    }

    #[test]
    fn byte_parser_rejects_truncated_attribute_headers_and_bodies() {
        let short_header = [1u8, 0, 0, 0];
        assert_eq!(
            parse_attrs_bytes(&short_header, 1, None),
            Err(-(crate::error::NLE_INVAL))
        );

        let mut short_body = Vec::new();
        short_body.extend_from_slice(&(8u16).to_ne_bytes());
        short_body.extend_from_slice(&(1u16).to_ne_bytes());
        short_body.extend_from_slice(&[1, 2]);
        assert_eq!(
            parse_attrs_bytes(&short_body, 1, None),
            Err(-(crate::error::NLE_INVAL))
        );
    }

    #[test]
    fn byte_parser_is_total_over_deterministic_garbage_inputs() {
        let mut state = 0x1234_5678u32;
        for len in 0..96usize {
            let mut buf = vec![0u8; len];
            for byte in &mut buf {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *byte = (state >> 24) as u8;
            }
            let _ = parse_attrs_bytes(&buf, 16, None);
        }
    }
}
