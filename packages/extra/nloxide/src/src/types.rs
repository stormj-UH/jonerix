// Kernel UAPI types and protocol constants derived from Ghidra analysis of libnl3-3.11.0
// and from Linux kernel UAPI headers (public/permissive).

use core::ffi::c_int;

// ---- netlink kernel types ----

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct NlMsgHdr {
    pub nlmsg_len: u32,
    pub nlmsg_type: u16,
    pub nlmsg_flags: u16,
    pub nlmsg_seq: u32,
    pub nlmsg_pid: u32,
}

pub const NLMSG_HDRLEN: usize = 16;

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct NlAttr {
    pub nla_len: u16,
    pub nla_type: u16,
}

pub const NLA_HDRLEN: usize = 4;

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct GenlMsgHdr {
    pub cmd: u8,
    pub version: u8,
    pub reserved: u16,
}

pub const GENL_HDRLEN: usize = 4;

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct SockaddrNl {
    pub nl_family: u16,
    pub nl_pad: u16,
    pub nl_pid: u32,
    pub nl_groups: u32,
}

pub const AF_NETLINK: u16 = 16;
pub const SOL_NETLINK: c_int = 270;
pub const NETLINK_GENERIC: c_int = 16;
pub const NETLINK_ADD_MEMBERSHIP: c_int = 1;
pub const NETLINK_DROP_MEMBERSHIP: c_int = 2;

// nlmsg_type values
pub const NLMSG_NOOP: u16 = 1;
pub const NLMSG_ERROR: u16 = 2;
pub const NLMSG_DONE: u16 = 3;
pub const NLMSG_OVERRUN: u16 = 4;
pub const NLMSG_MIN_TYPE: u16 = 0x10;

// nlmsg_flags
pub const NLM_F_REQUEST: u16 = 1;
pub const NLM_F_MULTI: u16 = 2;
pub const NLM_F_ACK: u16 = 4;
pub const NLM_F_ECHO: u16 = 8;
pub const NLM_F_DUMP_INTR: u16 = 16;
pub const NLM_F_DUMP_FILTERED: u16 = 32;
pub const NLM_F_ROOT: u16 = 0x100;
pub const NLM_F_MATCH: u16 = 0x200;
pub const NLM_F_ATOMIC: u16 = 0x400;
pub const NLM_F_DUMP: u16 = NLM_F_ROOT | NLM_F_MATCH;
pub const NLM_F_REPLACE: u16 = 0x100;
pub const NLM_F_EXCL: u16 = 0x200;
pub const NLM_F_CREATE: u16 = 0x400;
pub const NLM_F_APPEND: u16 = 0x800;

// nla_type flag
pub const NLA_F_NESTED: u16 = 0x8000;
pub const NLA_F_NET_BYTEORDER: u16 = 0x4000;
pub const NLA_TYPE_MASK: u16 = !(NLA_F_NESTED | NLA_F_NET_BYTEORDER);

// Generic netlink control family
pub const GENL_ID_CTRL: u16 = 0x10;
pub const CTRL_CMD_GETFAMILY: u8 = 3;
pub const CTRL_ATTR_FAMILY_ID: u16 = 1;
pub const CTRL_ATTR_FAMILY_NAME: u16 = 2;
pub const CTRL_ATTR_MCAST_GROUPS: u16 = 7;
pub const CTRL_ATTR_MCAST_GRP_NAME: u16 = 1;
pub const CTRL_ATTR_MCAST_GRP_ID: u16 = 2;
pub const GENL_NAMSIZ: usize = 16;

// nl_cb callback types (from Ghidra: loop runs 11 times, indices 0-10)
pub const NL_CB_VALID: usize = 0;
pub const NL_CB_FINISH: usize = 1;
pub const NL_CB_OVERRUN: usize = 2;
pub const NL_CB_SKIPPED: usize = 3;
pub const NL_CB_ACK: usize = 4;
pub const NL_CB_MSG_IN: usize = 5;
pub const NL_CB_MSG_OUT: usize = 6;
pub const NL_CB_INVALID: usize = 7;
pub const NL_CB_SEQ_CHECK: usize = 8;
pub const NL_CB_SEND_ACK: usize = 9;
pub const NL_CB_DUMP_INTR: usize = 10;
pub const NL_CB_TYPE_MAX: usize = 11;

// nl_cb_kind
pub const NL_CB_DEFAULT: u32 = 0;
pub const NL_CB_VERBOSE: u32 = 1;
pub const NL_CB_DEBUG: u32 = 2;
pub const NL_CB_CUSTOM: u32 = 3;

// callback return values
pub const NL_OK: c_int = 0;
pub const NL_SKIP: c_int = 1;
pub const NL_STOP: c_int = 2;

// NlSock flags (from Ghidra analysis of socket.c)
pub const NL_SOCK_BUFSIZE_SET: u32 = 1;
pub const NL_SOCK_PASSCRED: u32 = 2;
pub const NL_SOCK_OWNS_FD: u32 = 4;
pub const NL_SOCK_MSG_PEEK: u32 = 8;
pub const NL_SOCK_DISABLE_SEQ_CHECK: u32 = 0x10;
pub const NL_SOCK_NO_AUTO_ACK: u32 = 0x20;
pub const NL_SOCK_NONBLOCK: u32 = 0x40;

// alignment helpers
#[inline(always)]
pub fn nlmsg_align(n: usize) -> usize {
    (n + 3) & !3
}
#[inline(always)]
pub fn nla_align(n: usize) -> usize {
    (n + 3) & !3
}
#[inline(always)]
pub fn nlmsg_length(payload: usize) -> usize {
    NLMSG_HDRLEN + payload
}
#[inline(always)]
pub fn nlmsg_space(payload: usize) -> usize {
    nlmsg_align(nlmsg_length(payload))
}
