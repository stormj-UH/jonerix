// Generic netlink functions derived from Ghidra analysis of libnl-genl-3.so.200
//
// genl_connect → nl_connect(sk, NETLINK_GENERIC=16)
// genlmsg_put  → nlmsg_put + writes genlmsghdr
// genl_ctrl_resolve → direct kernel CTRL_CMD_GETFAMILY query (no cache)
// genl_ctrl_resolve_grp → same + parse CTRL_ATTR_MCAST_GROUPS

use core::ffi::{c_int, c_void};
use core::ptr;
use crate::types::*;
use crate::message::{NlMsg, nlmsg_alloc_simple, nlmsg_free, nlmsg_hdr,
                     nlmsg_data, nlmsg_datalen, nlmsg_attrdata, nlmsg_attrlen};
use crate::attr::{NlAttr, nla_put_string, nla_parse, nla_find, nla_data, nla_len,
                  nla_type, nla_ok, nla_next, nla_get_u16, nla_get_u32};
use crate::socket::{NlSock, nl_connect, nl_send_auto, nl_recvmsgs, nl_socket_alloc,
                    nl_socket_free, nl_socket_disable_seq_check};
use crate::callback::{NlCb, NlMsgErr, nl_cb_alloc, nl_cb_clone, nl_cb_put, nl_cb_set};
use crate::error::{NLE_INVAL, NLE_NOMEM, NLE_OBJ_NOTFOUND};

// ---- genlmsg_* ----

#[no_mangle]
pub unsafe extern "C" fn genlmsg_hdr(hdr: *const NlMsgHdr) -> *mut GenlMsgHdr {
    nlmsg_data(hdr) as *mut GenlMsgHdr
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_len(ghdr: *const GenlMsgHdr) -> c_int {
    // The genlmsghdr is embedded in the nlmsg payload; its length up to attributes
    // is tracked via the wrapping nlmsghdr. This accessor returns the data length
    // after the genlmsghdr header within the current message context.
    // Used internally; wpa_supplicant/hostapd rarely call this directly.
    let _ = ghdr;
    0
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_user_hdr(ghdr: *const GenlMsgHdr) -> *mut c_void {
    (ghdr as *const u8).add(GENL_HDRLEN) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_user_data(ghdr: *const GenlMsgHdr, hdrlen: c_int) -> *mut c_void {
    let h = genlmsg_user_hdr(ghdr) as *const u8;
    h.add(nlmsg_align(hdrlen as usize)) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_user_datalen(ghdr: *const GenlMsgHdr, hdrlen: c_int) -> c_int {
    let _ = ghdr;
    let _ = hdrlen;
    0
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_data(ghdr: *const GenlMsgHdr) -> *mut c_void {
    (ghdr as *const u8).add(GENL_HDRLEN) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_attrdata(
    ghdr: *const GenlMsgHdr,
    hdrlen: c_int,
) -> *mut NlAttr {
    let d = genlmsg_data(ghdr) as *const u8;
    d.add(nlmsg_align(hdrlen as usize)) as *mut NlAttr
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_attrlen(ghdr: *const GenlMsgHdr, hdrlen: c_int) -> c_int {
    let _ = ghdr;
    let _ = hdrlen;
    0
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_valid_hdr(hdr: *const NlMsgHdr, hdrlen: c_int) -> bool {
    if hdr.is_null() { return false; }
    let dlen = nlmsg_datalen(hdr);
    dlen >= (GENL_HDRLEN + nlmsg_align(hdrlen as usize)) as i32
}

#[no_mangle]
pub unsafe extern "C" fn genlmsg_parse(
    hdr: *const NlMsgHdr,
    hdrlen: c_int,
    tb: *mut *mut NlAttr,
    maxtype: c_int,
    policy: *const c_void,
) -> c_int {
    if !genlmsg_valid_hdr(hdr, hdrlen) { return -(NLE_INVAL); }
    let ghdr = genlmsg_hdr(hdr);
    let attrdata = genlmsg_attrdata(ghdr, hdrlen);
    let attrlen_val = nlmsg_datalen(hdr)
        - (GENL_HDRLEN + nlmsg_align(hdrlen as usize)) as i32;
    nla_parse(tb, maxtype, attrdata, attrlen_val, policy)
}

// genlmsg_put: puts an nlmsghdr + genlmsghdr into a nl_msg
// C signature: void *genlmsg_put(struct nl_msg *msg, uint32_t pid, uint32_t seq,
//                                int family, int hdrlen, int flags, uint8_t cmd, uint8_t version)
#[no_mangle]
pub unsafe extern "C" fn genlmsg_put(
    msg: *mut NlMsg,
    pid: u32,
    seq: u32,
    family: c_int,
    hdrlen: c_int,
    flags: c_int,
    cmd: u8,
    version: u8,
) -> *mut c_void {
    if msg.is_null() { return ptr::null_mut(); }
    let payload = GENL_HDRLEN + nlmsg_align(hdrlen as usize);
    let hdr = crate::message::nlmsg_put(msg, pid, seq, family, payload as c_int, flags as u16);
    if hdr.is_null() { return ptr::null_mut(); }
    let ghdr = nlmsg_data(hdr) as *mut GenlMsgHdr;
    (*ghdr).cmd = cmd;
    (*ghdr).version = version;
    (*ghdr).reserved = 0;
    // return pointer past genlmsghdr + user hdrlen
    genlmsg_user_data(ghdr, hdrlen)
}

// ---- genl_connect ----

#[no_mangle]
pub unsafe extern "C" fn genl_connect(sk: *mut NlSock) -> c_int {
    nl_connect(sk, NETLINK_GENERIC)
}

// ---- genl_send_simple ----

#[no_mangle]
pub unsafe extern "C" fn genl_send_simple(
    sk: *mut NlSock,
    family: c_int,
    cmd: u8,
    version: u8,
    flags: c_int,
) -> c_int {
    let msg = nlmsg_alloc_simple(family, flags);
    if msg.is_null() { return -(NLE_NOMEM); }
    let p = genlmsg_put(msg, 0, 0, family, 0, flags, cmd, version);
    let r = if !p.is_null() {
        crate::socket::nl_send_auto(sk, msg)
    } else {
        -(NLE_NOMEM)
    };
    nlmsg_free(msg);
    r
}

// ---- genl_ctrl_resolve (direct kernel query, no cache) ----
//
// Queries GENL_ID_CTRL (0x10) with CTRL_CMD_GETFAMILY to find a family by name.
// Returns family id (positive) or negative NLE error.

struct ResolveCtx {
    family_id: i32,
    done: bool,
}

unsafe extern "C" fn resolve_valid_cb(msg: *mut NlMsg, arg: *mut c_void) -> c_int {
    let ctx = &mut *(arg as *mut ResolveCtx);
    let hdr = nlmsg_hdr(msg);
    let ghdr = genlmsg_hdr(hdr);
    let attrdata = nlmsg_attrdata(hdr, GENL_HDRLEN as c_int);
    let attrlen = nlmsg_attrlen(hdr, GENL_HDRLEN as c_int);

    // parse CTRL_ATTR_FAMILY_ID (1)
    let mut tb: [*mut NlAttr; 8] = [ptr::null_mut(); 8];
    nla_parse(tb.as_mut_ptr(), 7, attrdata, attrlen, ptr::null());
    if !tb[CTRL_ATTR_FAMILY_ID as usize].is_null() {
        ctx.family_id = nla_get_u16(tb[CTRL_ATTR_FAMILY_ID as usize]) as i32;
        ctx.done = true;
    }
    crate::types::NL_OK
}

fn genl_ctrl_do_resolve(sk: *mut NlSock, name: *const u8) -> c_int {
    unsafe {
        let msg = nlmsg_alloc_simple(GENL_ID_CTRL as c_int, NLM_F_REQUEST as c_int);
        if msg.is_null() { return -(NLE_NOMEM); }

        let p = genlmsg_put(msg, 0, 0, GENL_ID_CTRL as c_int, 0,
                             0, CTRL_CMD_GETFAMILY, 1);
        if p.is_null() { nlmsg_free(msg); return -(NLE_NOMEM); }

        let r = nla_put_string(msg, CTRL_ATTR_FAMILY_NAME as c_int, name);
        if r < 0 { nlmsg_free(msg); return r; }

        let r = crate::socket::nl_send_auto(sk, msg);
        nlmsg_free(msg);
        if r < 0 { return r; }

        let mut ctx = ResolveCtx { family_id: -(NLE_OBJ_NOTFOUND), done: false };
        let cb = nl_cb_clone((*sk).s_cb);
        nl_cb_set(cb, NL_CB_VALID as c_int, 0,
                  Some(resolve_valid_cb), &mut ctx as *mut ResolveCtx as *mut c_void);

        loop {
            let r2 = nl_recvmsgs(sk, cb);
            if r2 < 0 || ctx.done { break; }
        }
        nl_cb_put(cb);
        ctx.family_id
    }
}

#[no_mangle]
pub unsafe extern "C" fn genl_ctrl_resolve(sk: *mut NlSock, name: *const u8) -> c_int {
    genl_ctrl_do_resolve(sk, name)
}

// ---- genl_ctrl_resolve_grp ----

struct ResolveGrpCtx {
    grp_name: *const u8,
    grp_id: i32,
    done: bool,
}

unsafe extern "C" fn resolve_grp_valid_cb(msg: *mut NlMsg, arg: *mut c_void) -> c_int {
    let ctx = &mut *(arg as *mut ResolveGrpCtx);
    let hdr = nlmsg_hdr(msg);
    let attrdata = nlmsg_attrdata(hdr, GENL_HDRLEN as c_int);
    let attrlen = nlmsg_attrlen(hdr, GENL_HDRLEN as c_int);

    let mut tb: [*mut NlAttr; 8] = [ptr::null_mut(); 8];
    nla_parse(tb.as_mut_ptr(), 7, attrdata, attrlen, ptr::null());

    let mcast_attr = tb[CTRL_ATTR_MCAST_GROUPS as usize];
    if mcast_attr.is_null() { return crate::types::NL_OK; }

    // walk nested mcast groups list
    let groups_data = nla_data(mcast_attr) as *const NlAttr;
    let groups_len = nla_len(mcast_attr);
    let mut pos = groups_data;
    let mut rem = groups_len;
    while nla_ok(pos, rem) {
        // each entry is a nested attr with CTRL_ATTR_MCAST_GRP_NAME and _ID
        let mut gtb: [*mut NlAttr; 3] = [ptr::null_mut(); 3];
        nla_parse(
            gtb.as_mut_ptr(), 2,
            nla_data(pos) as *const NlAttr,
            nla_len(pos), ptr::null(),
        );
        let name_attr = gtb[CTRL_ATTR_MCAST_GRP_NAME as usize];
        let id_attr = gtb[CTRL_ATTR_MCAST_GRP_ID as usize];
        if !name_attr.is_null() && !id_attr.is_null() {
            if libc::strcmp(nla_data(name_attr) as _, ctx.grp_name as _) == 0 {
                ctx.grp_id = nla_get_u32(id_attr) as i32;
                ctx.done = true;
                return crate::types::NL_STOP;
            }
        }
        pos = nla_next(pos, &mut rem);
    }
    crate::types::NL_OK
}

#[no_mangle]
pub unsafe extern "C" fn genl_ctrl_resolve_grp(
    sk: *mut NlSock,
    family_name: *const u8,
    grp_name: *const u8,
) -> c_int {
    // First send GETFAMILY request
    let msg = nlmsg_alloc_simple(GENL_ID_CTRL as c_int, NLM_F_REQUEST as c_int);
    if msg.is_null() { return -(NLE_NOMEM); }

    let p = genlmsg_put(msg, 0, 0, GENL_ID_CTRL as c_int, 0, 0, CTRL_CMD_GETFAMILY, 1);
    if p.is_null() { nlmsg_free(msg); return -(NLE_NOMEM); }

    let r = nla_put_string(msg, CTRL_ATTR_FAMILY_NAME as c_int, family_name);
    if r < 0 { nlmsg_free(msg); return r; }

    let r = crate::socket::nl_send_auto(sk, msg);
    nlmsg_free(msg);
    if r < 0 { return r; }

    let mut ctx = ResolveGrpCtx { grp_name, grp_id: -(NLE_OBJ_NOTFOUND), done: false };
    let cb = nl_cb_clone((*sk).s_cb);
    nl_cb_set(cb, NL_CB_VALID as c_int, 0,
              Some(resolve_grp_valid_cb), &mut ctx as *mut ResolveGrpCtx as *mut c_void);

    loop {
        let r2 = nl_recvmsgs(sk, cb);
        if r2 < 0 || ctx.done { break; }
    }
    nl_cb_put(cb);
    ctx.grp_id
}

// ---- genl_ctrl_alloc_cache (simplified: returns opaque handle) ----
// The cache is used by genl_ctrl_search/genl_ctrl_search_by_name.
// We implement a simple list internally.

pub struct GenlCache {
    pub entries: Vec<GenlFamily>,
}

pub struct GenlFamily {
    pub name: [u8; GENL_NAMSIZ],
    pub id: u16,
    pub version: u8,
    pub hdrsize: u32,
    pub maxattr: u32,
    pub mcast_groups: Vec<McastGroup>,
}

pub struct McastGroup {
    pub name: [u8; GENL_NAMSIZ],
    pub id: u32,
}

unsafe extern "C" fn cache_fill_valid_cb(msg: *mut NlMsg, arg: *mut c_void) -> c_int {
    let cache = &mut *(arg as *mut GenlCache);
    let hdr = nlmsg_hdr(msg);
    let attrdata = nlmsg_attrdata(hdr, GENL_HDRLEN as c_int);
    let attrlen = nlmsg_attrlen(hdr, GENL_HDRLEN as c_int);

    let mut tb: [*mut NlAttr; 8] = [ptr::null_mut(); 8];
    nla_parse(tb.as_mut_ptr(), 7, attrdata, attrlen, ptr::null());

    let name_a = tb[CTRL_ATTR_FAMILY_NAME as usize];
    let id_a = tb[CTRL_ATTR_FAMILY_ID as usize];
    if name_a.is_null() || id_a.is_null() { return crate::types::NL_OK; }

    let mut fam = GenlFamily {
        name: [0u8; GENL_NAMSIZ],
        id: nla_get_u16(id_a),
        version: 0,
        hdrsize: 0,
        maxattr: 0,
        mcast_groups: Vec::new(),
    };
    let src = nla_data(name_a) as *const u8;
    let slen = libc::strlen(src as _).min(GENL_NAMSIZ - 1);
    ptr::copy_nonoverlapping(src, fam.name.as_mut_ptr(), slen);

    // mcast groups
    if let Some(mg_a) = tb.get(CTRL_ATTR_MCAST_GROUPS as usize).copied().filter(|p| !p.is_null()) {
        let gd = nla_data(mg_a) as *const NlAttr;
        let gl = nla_len(mg_a);
        let mut pos = gd;
        let mut rem = gl;
        while nla_ok(pos, rem) {
            let mut gtb: [*mut NlAttr; 3] = [ptr::null_mut(); 3];
            nla_parse(gtb.as_mut_ptr(), 2, nla_data(pos) as _, nla_len(pos), ptr::null());
            let gn = gtb[CTRL_ATTR_MCAST_GRP_NAME as usize];
            let gi = gtb[CTRL_ATTR_MCAST_GRP_ID as usize];
            if !gn.is_null() && !gi.is_null() {
                let mut mg = McastGroup { name: [0u8; GENL_NAMSIZ], id: nla_get_u32(gi) };
                let ns = nla_data(gn) as *const u8;
                let nl = libc::strlen(ns as _).min(GENL_NAMSIZ - 1);
                ptr::copy_nonoverlapping(ns, mg.name.as_mut_ptr(), nl);
                fam.mcast_groups.push(mg);
            }
            pos = nla_next(pos, &mut rem);
        }
    }
    cache.entries.push(fam);
    crate::types::NL_OK
}

#[no_mangle]
pub unsafe extern "C" fn genl_ctrl_alloc_cache(
    sk: *mut NlSock,
    result: *mut *mut GenlCache,
) -> c_int {
    if sk.is_null() || result.is_null() { return -(NLE_INVAL); }

    // Send CTRL_CMD_GETFAMILY with DUMP flag to enumerate all families
    let msg = nlmsg_alloc_simple(GENL_ID_CTRL as c_int,
                                  (NLM_F_REQUEST | NLM_F_DUMP) as c_int);
    if msg.is_null() { return -(NLE_NOMEM); }

    let p = genlmsg_put(msg, 0, 0, GENL_ID_CTRL as c_int, 0, 0, CTRL_CMD_GETFAMILY, 1);
    if p.is_null() { nlmsg_free(msg); return -(NLE_NOMEM); }

    let r = crate::socket::nl_send_auto(sk, msg);
    nlmsg_free(msg);
    if r < 0 { return r; }

    let mut cache = Box::new(GenlCache { entries: Vec::new() });
    let cb = nl_cb_clone((*sk).s_cb);
    nl_cb_set(cb, NL_CB_VALID as c_int, 0,
              Some(cache_fill_valid_cb), &mut *cache as *mut GenlCache as *mut c_void);

    loop {
        let r2 = crate::socket::nl_recvmsgs(sk, cb);
        if r2 <= 0 { break; }
    }
    nl_cb_put(cb);
    *result = Box::into_raw(cache);
    0
}

#[no_mangle]
pub unsafe extern "C" fn genl_cache_free(cache: *mut GenlCache) {
    if !cache.is_null() { drop(Box::from_raw(cache)); }
}

#[no_mangle]
pub unsafe extern "C" fn genl_ctrl_search(cache: *mut GenlCache, id: u16) -> *mut GenlFamily {
    if cache.is_null() { return ptr::null_mut(); }
    for f in &(*cache).entries {
        if f.id == id {
            return f as *const GenlFamily as *mut GenlFamily;
        }
    }
    ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn genl_ctrl_search_by_name(
    cache: *mut GenlCache,
    name: *const u8,
) -> *mut GenlFamily {
    if cache.is_null() || name.is_null() { return ptr::null_mut(); }
    for f in &(*cache).entries {
        if libc::strcmp(f.name.as_ptr() as _, name as _) == 0 {
            return f as *const GenlFamily as *mut GenlFamily;
        }
    }
    ptr::null_mut()
}

// ---- genl_family_* accessors ----

#[no_mangle]
pub unsafe extern "C" fn genl_family_alloc() -> *mut GenlFamily {
    Box::into_raw(Box::new(GenlFamily {
        name: [0; GENL_NAMSIZ],
        id: 0,
        version: 0,
        hdrsize: 0,
        maxattr: 0,
        mcast_groups: Vec::new(),
    }))
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_put(f: *mut GenlFamily) {
    if !f.is_null() { drop(Box::from_raw(f)); }
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_get_id(f: *const GenlFamily) -> u16 {
    if f.is_null() { return 0; }
    (*f).id
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_set_id(f: *mut GenlFamily, id: u16) {
    if !f.is_null() { (*f).id = id; }
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_get_name(f: *const GenlFamily) -> *const u8 {
    if f.is_null() { return ptr::null(); }
    (*f).name.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_set_name(f: *mut GenlFamily, name: *const u8) {
    if f.is_null() || name.is_null() { return; }
    let n = libc::strlen(name as _).min(GENL_NAMSIZ - 1);
    ptr::copy_nonoverlapping(name, (*f).name.as_mut_ptr(), n);
    (*f).name[n] = 0;
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_get_version(f: *const GenlFamily) -> u32 {
    if f.is_null() { return 0; }
    (*f).version as u32
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_set_version(f: *mut GenlFamily, v: u32) {
    if !f.is_null() { (*f).version = v as u8; }
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_get_hdrsize(f: *const GenlFamily) -> u32 {
    if f.is_null() { return 0; }
    (*f).hdrsize
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_set_hdrsize(f: *mut GenlFamily, s: u32) {
    if !f.is_null() { (*f).hdrsize = s; }
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_get_maxattr(f: *const GenlFamily) -> u32 {
    if f.is_null() { return 0; }
    (*f).maxattr
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_set_maxattr(f: *mut GenlFamily, m: u32) {
    if !f.is_null() { (*f).maxattr = m; }
}

#[no_mangle]
pub unsafe extern "C" fn genl_family_add_op(_f: *mut GenlFamily, _op: *const c_void) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_family_add_grp(_f: *mut GenlFamily, _id: u32, _name: *const u8) -> c_int { 0 }

// stubs for unused ops registration
#[no_mangle]
pub unsafe extern "C" fn genl_register_family(_f: *mut GenlFamily) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_unregister_family(_f: *mut GenlFamily) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_handle_msg(_sk: *mut NlSock, _msg: *mut NlMsg) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_register(_f: *mut GenlFamily) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_unregister(_f: *mut GenlFamily) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_ops_resolve(_sk: *mut NlSock, _ops: *const c_void) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_resolve_id(_sk: *mut NlSock, _f: *mut GenlFamily) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_mngt_resolve(_sk: *mut NlSock) -> c_int { 0 }
#[no_mangle]
pub unsafe extern "C" fn genl_op2name(_id: u32, _buf: *mut u8, _size: usize) -> *mut u8 {
    ptr::null_mut()
}
