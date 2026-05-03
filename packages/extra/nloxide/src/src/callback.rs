// nl_cb implementation derived from Ghidra analysis of libnl3-3.11.0
// nl_cb_alloc: calloc(1, 0xe0); refcount at +0xd8; 11 cb types + err cb
// ABI: opaque pointer, reference-counted

use crate::message::NlMsg;
use crate::types::*;
use core::ffi::c_int;
use core::sync::atomic::{AtomicI32, Ordering};

pub type NlRecvMsgCb = unsafe extern "C" fn(*mut NlMsg, *mut core::ffi::c_void) -> c_int;
pub type NlRecvMsgErrCb =
    unsafe extern "C" fn(*mut SockaddrNl, *mut NlMsgErr, *mut core::ffi::c_void) -> c_int;

#[repr(C)]
pub struct NlMsgErr {
    pub error: c_int,
    pub msg: NlMsgHdr,
}

pub struct CbSlot {
    pub func: Option<NlRecvMsgCb>,
    pub arg: *mut core::ffi::c_void,
}

impl Default for CbSlot {
    fn default() -> Self {
        CbSlot {
            func: None,
            arg: core::ptr::null_mut(),
        }
    }
}

pub struct NlCb {
    pub cb: [CbSlot; NL_CB_TYPE_MAX],
    pub err_cb: NlErrCbSlot,
    pub refcount: AtomicI32,
}

pub struct NlErrCbSlot {
    pub func: Option<NlRecvMsgErrCb>,
    pub arg: *mut core::ffi::c_void,
}

impl Default for NlErrCbSlot {
    fn default() -> Self {
        NlErrCbSlot {
            func: None,
            arg: core::ptr::null_mut(),
        }
    }
}

unsafe impl Send for NlCb {}
unsafe impl Sync for NlCb {}

impl NlCb {
    pub fn new() -> *mut NlCb {
        let cb = Box::new(NlCb {
            cb: Default::default(),
            err_cb: Default::default(),
            refcount: AtomicI32::new(1),
        });
        Box::into_raw(cb)
    }
}

impl Default for NlCb {
    fn default() -> Self {
        const SLOT: CbSlot = CbSlot {
            func: None,
            arg: core::ptr::null_mut(),
        };
        NlCb {
            cb: [SLOT; NL_CB_TYPE_MAX],
            err_cb: NlErrCbSlot {
                func: None,
                arg: core::ptr::null_mut(),
            },
            refcount: AtomicI32::new(1),
        }
    }
}

// --- exported C functions ---

#[no_mangle]
pub unsafe extern "C" fn nl_cb_alloc(kind: u32) -> *mut NlCb {
    if kind > NL_CB_CUSTOM {
        return core::ptr::null_mut();
    }
    NlCb::new()
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_clone(cb: *mut NlCb) -> *mut NlCb {
    if cb.is_null() {
        return core::ptr::null_mut();
    }
    let src = &*cb;
    let new_cb = Box::new(NlCb {
        cb: core::array::from_fn(|i| CbSlot {
            func: src.cb[i].func,
            arg: src.cb[i].arg,
        }),
        err_cb: NlErrCbSlot {
            func: src.err_cb.func,
            arg: src.err_cb.arg,
        },
        refcount: AtomicI32::new(1),
    });
    Box::into_raw(new_cb)
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_get(cb: *mut NlCb) -> *mut NlCb {
    if !cb.is_null() {
        (*cb).refcount.fetch_add(1, Ordering::AcqRel);
    }
    cb
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_put(cb: *mut NlCb) {
    if cb.is_null() {
        return;
    }
    let prev = (*cb).refcount.fetch_sub(1, Ordering::AcqRel);
    if prev == 1 {
        drop(Box::from_raw(cb));
    }
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_set(
    cb: *mut NlCb,
    kind: c_int,
    _cb_type: u32,
    func: Option<NlRecvMsgCb>,
    arg: *mut core::ffi::c_void,
) -> c_int {
    if cb.is_null() || kind as usize >= NL_CB_TYPE_MAX {
        return -(crate::error::NLE_INVAL);
    }
    (*cb).cb[kind as usize].func = func;
    (*cb).cb[kind as usize].arg = arg;
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_set_all(
    cb: *mut NlCb,
    _cb_type: u32,
    func: Option<NlRecvMsgCb>,
    arg: *mut core::ffi::c_void,
) -> c_int {
    if cb.is_null() {
        return -(crate::error::NLE_INVAL);
    }
    for i in 0..NL_CB_TYPE_MAX {
        (*cb).cb[i].func = func;
        (*cb).cb[i].arg = arg;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_err(
    cb: *mut NlCb,
    _kind: u32,
    func: Option<NlRecvMsgErrCb>,
    arg: *mut core::ffi::c_void,
) -> c_int {
    if cb.is_null() {
        return -(crate::error::NLE_INVAL);
    }
    (*cb).err_cb.func = func;
    (*cb).err_cb.arg = arg;
    0
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_overwrite_recvmsgs(cb: *mut NlCb, _func: *mut core::ffi::c_void) {
    // overwrite hooks not implemented; wpa_supplicant/hostapd do not use these
    let _ = cb;
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_overwrite_recv(cb: *mut NlCb, _func: *mut core::ffi::c_void) {
    let _ = cb;
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_overwrite_send(cb: *mut NlCb, _func: *mut core::ffi::c_void) {
    let _ = cb;
}

#[no_mangle]
pub unsafe extern "C" fn nl_cb_active_type(_cb: *mut NlCb) -> c_int {
    0
}
