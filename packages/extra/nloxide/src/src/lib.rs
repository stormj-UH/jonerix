// nloxide — clean-room permissive-license drop-in for libnl-3 / libnl-genl-3
// Derived from Ghidra decompilation of Alpine libnl3-3.11.0-r0.apk (x86_64).
// No libnl source code was consulted.  License: BSD-2-Clause.

#![allow(dead_code, non_camel_case_types, clippy::missing_safety_doc)]

mod attr;
mod callback;
mod error;
mod genl;
mod message;
mod socket;
mod types;

// Re-export all the stubs that wpa_supplicant/hostapd use but that live in
// our monolithic library even though they come from the "nl-3" name group.

// ---- nl_addr_* stubs (not used by the nl80211 path) ----
use core::ffi::{c_int, c_void};

#[no_mangle]
pub unsafe extern "C" fn nl_addr_alloc(_len: usize) -> *mut c_void {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_put(_a: *mut c_void) {}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_get(_a: *mut c_void) -> *mut c_void {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_alloc_attr(
    _a: *const crate::attr::NlAttr,
    _f: c_int,
) -> *mut c_void {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_build(_f: c_int, _d: *const c_void, _l: usize) -> *mut c_void {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_clone(_a: *const c_void) -> *mut c_void {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_cmp(_a: *const c_void, _b: *const c_void) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_cmp_prefix(_a: *const c_void, _b: *const c_void) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_iszero(_a: *const c_void) -> bool {
    true
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_valid(_a: *const c_void, _b: c_int) -> bool {
    false
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_guess_family(_a: *const c_void) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_fill_sockaddr(
    _a: *const c_void,
    _s: *mut c_void,
    _l: *mut u32,
) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_resolve(_a: *mut c_void, _sk: *mut c_void) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_set_family(_a: *mut c_void, _f: c_int) {}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_get_family(_a: *const c_void) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_set_binary_addr(
    _a: *mut c_void,
    _d: *const c_void,
    _l: usize,
) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_get_binary_addr(_a: *const c_void) -> *const c_void {
    core::ptr::null()
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_get_len(_a: *const c_void) -> usize {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_set_prefixlen(_a: *mut c_void, _l: c_int) {}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_get_prefixlen(_a: *const c_void) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_parse(_s: *const u8, _f: c_int, _r: *mut *mut c_void) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr2str(_a: *const c_void, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_info(_a: *const c_void, _r: *mut *mut c_void) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn nl_addr_shared(_a: *const c_void) -> bool {
    false
}

// ---- utility string stubs ----
#[no_mangle]
pub unsafe extern "C" fn nl_af2str(_f: c_int, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_str2af(_s: *const u8) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_nlfamily2str(_f: c_int, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_str2nlfamily(_s: *const u8) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_llproto2str(_p: c_int, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_str2llproto(_s: *const u8) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_ether_proto2str(_p: c_int, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_str2ether_proto(_s: *const u8) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_ip_proto2str(_p: c_int, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_str2ip_proto(_s: *const u8) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_cancel_down_bytes(_b: u64, _u: *mut *const u8) -> u64 {
    _b
}
#[no_mangle]
pub unsafe extern "C" fn nl_cancel_down_bits(_b: u64, _u: *mut *const u8) -> u64 {
    _b
}
#[no_mangle]
pub unsafe extern "C" fn nl_cancel_down_us(_b: u64, _u: *mut *const u8) -> u64 {
    _b
}
#[no_mangle]
pub unsafe extern "C" fn nl_size2int(_s: *const u8) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_size2str(_s: u64, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_prob2int(_s: *const u8) -> c_int {
    0
}
#[no_mangle]
pub unsafe extern "C" fn nl_rate2str(_r: u64, _t: c_int, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_us2ticks(_us: u64) -> u64 {
    _us
}
#[no_mangle]
pub unsafe extern "C" fn nl_ticks2us(_t: u64) -> u64 {
    _t
}
#[no_mangle]
pub unsafe extern "C" fn nl_str2msec(_s: *const u8, _r: *mut u64) -> c_int {
    -1
}
#[no_mangle]
pub unsafe extern "C" fn nl_msec2str(_ms: u64, _b: *mut u8, _l: usize) -> *mut u8 {
    core::ptr::null_mut()
}
#[no_mangle]
pub unsafe extern "C" fn nl_get_user_hz() -> u32 {
    100
}
#[no_mangle]
pub unsafe extern "C" fn nl_get_psched_hz() -> u32 {
    1000
}

// ---- dump / output stubs ----
#[no_mangle]
pub unsafe extern "C" fn nl_new_line(_p: *mut c_void) {}
#[no_mangle]
pub unsafe extern "C" fn nl_dump(_p: *mut c_void, _fmt: *const u8) {}
#[no_mangle]
pub unsafe extern "C" fn nl_dump_line(_p: *mut c_void, _fmt: *const u8) {}
