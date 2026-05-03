# nloxide

A clean-room, BSD-2-Clause licensed compatibility layer for the subset of
`libnl-3` and `libnl-genl-3` used by the `hostapd` and `wpa_supplicant`
nl80211 paths in jonerix.  Written in Rust, derived exclusively from Ghidra
decompilation of the Alpine Linux `libnl3-3.11.0-r0.apk` binary.  No libnl
source code was consulted at any point.

Built for the **jonerix** Linux distribution, where GPL and LGPL runtimes are
prohibited.  The `aarch64-jonerix-linux-musl` target builds a static archive for
linking into jonerix `hostapd` and `wpa_supplicant` builds, with packaging
responsible for exposing the expected libnl link names.  It is not intended to be
a general drop-in replacement for arbitrary libnl consumers.

---

## What is implemented

The implemented surface is the libnl core/generic-netlink behavior needed by
the jonerix `hostapd`/`wpa_supplicant` nl80211 paths.  Symbols outside that
contract may exist only for link compatibility and should not be treated as a
supported libnl feature surface.

### Core socket (`socket.rs`)

Every function in this group is fully implemented.

| Symbol | Notes |
|---|---|
| `nl_socket_alloc` | Allocs `NlSock` + default callback |
| `nl_socket_alloc_cb` | Allocs `NlSock` with caller-supplied callback |
| `nl_socket_free` | Closes fd, drops callback ref, frees |
| `nl_socket_get_fd` / `nl_socket_set_fd` | Direct fd access |
| `nl_socket_get_local_port` / `nl_socket_set_local_port` | nl_pid in s_local |
| `nl_socket_get_peer_port` / `nl_socket_set_peer_port` | nl_pid in s_peer |
| `nl_socket_get_peer_groups` / `nl_socket_set_peer_groups` | nl_groups in s_peer |
| `nl_socket_get_cb` / `nl_socket_set_cb` | Callback ref-count management |
| `nl_socket_modify_cb` | Delegates to `nl_cb_set` |
| `nl_socket_modify_err_cb` | Delegates to `nl_cb_err` |
| `nl_socket_disable_seq_check` / `nl_socket_use_seq` | Sequence flags |
| `nl_socket_disable_auto_ack` / `nl_socket_enable_auto_ack` | ACK flag |
| `nl_socket_enable_msg_peek` / `nl_socket_disable_msg_peek` | MSG_PEEK flag |
| `nl_socket_set_nonblocking` | `fcntl(F_SETFL, O_NONBLOCK)` |
| `nl_socket_set_buffer_size` | `SO_RCVBUFFORCE` / `SO_SNDBUFFORCE` with `SO_RCVBUF` / `SO_SNDBUF` fallback |
| `nl_socket_set_msg_buf_size` / `nl_socket_get_msg_buf_size` | Per-message recv buffer |
| `nl_socket_set_passcred` | `SO_PASSCRED` |
| `nl_socket_recv_pktinfo` | `NETLINK_PKTINFO` |
| `nl_socket_add_membership` / `nl_socket_drop_membership` | `NETLINK_ADD_MEMBERSHIP` / `NETLINK_DROP_MEMBERSHIP` |
| `nl_socket_add_memberships` / `nl_socket_drop_memberships` | Single-group shim (see limitations) |
| `nl_join_groups` | Sets `nl_groups` on local addr (legacy multicast) |
| `nl_connect` | `socket(AF_NETLINK)` + `bind` + `getsockname` |
| `nl_close` | Closes fd without freeing socket |
| `nl_complete_msg` / `nl_auto_complete` | Fill seq/pid/ACK in nlmsghdr |
| `nl_send_auto` / `nl_send_auto_complete` | Complete + MSG_OUT callback + send |
| `nl_send` / `nl_sendmsg` | Raw send with peer from msg or socket |
| `nl_sendto` | `send()` to pre-connected socket |
| `nl_send_iovec` | Falls through to `nl_sendmsg` (iovec ignored) |
| `nl_send_simple` | Alloc + append + send_auto + free |
| `nl_recv` | `recvfrom` into malloc'd buffer |
| `nl_recvmsgs_default` | Uses socket's own callback |
| `nl_recvmsgs` | Wraps `nl_recvmsgs_report`, normalises >0 to 0 |
| `nl_recvmsgs_report` | Full multi-message dispatch loop (see below) |
| `nl_wait_for_ack` | Clone cb, set one-shot ACK handler, loop recvmsgs |
| `nl_send_sync` | `send_auto` + free msg + `wait_for_ack` |
| `nl_pickup` / `nl_pickup_keep_syserr` | Receive one reply via parser callback |
| `nl_has_capability` | Stub, always returns false |

The `nl_recvmsgs_report` dispatch loop handles all five netlink message types:
`NLMSG_NOOP` → `NL_CB_SKIPPED`, `NLMSG_OVERRUN` → `NL_CB_OVERRUN`,
`NLMSG_ERROR` with error=0 (ACK) → `NL_CB_ACK`, `NLMSG_ERROR` with error≠0 →
error callback, `NLMSG_DONE` → `NL_CB_FINISH`, any other type → `NL_CB_VALID`.
Sequence number checking is implemented (disabled by flag).

### Callbacks (`callback.rs`)

| Symbol | Notes |
|---|---|
| `nl_cb_alloc` | Allocs 224-byte `NlCb` with 11 type slots |
| `nl_cb_clone` | Deep-copies all slots and resets refcount to 1 |
| `nl_cb_get` | Increments refcount, returns same pointer |
| `nl_cb_put` | Decrements refcount, frees at zero |
| `nl_cb_set` | Sets a message callback by type index |
| `nl_cb_err` | Sets the error callback |

Callback kind (`NL_CB_DEFAULT`, `NL_CB_VERBOSE`, `NL_CB_DEBUG`, `NL_CB_CUSTOM`)
is stored but has no effect — all custom callbacks are installed regardless of
kind.  The 11 callback types (`NL_CB_VALID` through `NL_CB_DUMP_INTR`) are all
stored and dispatched.

### Messages (`message.rs`)

| Symbol | Notes |
|---|---|
| `nlmsg_alloc` | 4 KiB default buffer |
| `nlmsg_alloc_simple` | Alloc + `nlmsg_put` header |
| `nlmsg_alloc_size` | Alloc with caller-specified size |
| `nlmsg_inherit` | Copy header from existing message |
| `nlmsg_convert` | Wrap raw `nlmsghdr *` in `NlMsg` (no copy) |
| `nlmsg_free` | Free (decrements refcount, frees at zero) |
| `nlmsg_get` | Increments refcount |
| `nlmsg_hdr` | Returns `*mut NlMsgHdr` for the message |
| `nlmsg_data` | Payload start (after nlmsghdr) |
| `nlmsg_datalen` | Payload length |
| `nlmsg_attrdata` | Attributes start (after nlmsghdr + hdrlen) |
| `nlmsg_attrlen` | Attribute region length |
| `nlmsg_put` | Append nlmsghdr to buffer |
| `nlmsg_reserve` | Reserve `len` bytes, return pointer |
| `nlmsg_append` | Append data with optional padding |
| `nlmsg_expand` | Grow buffer by at least N bytes |
| `nlmsg_parse` | Walk attrs, fill `tb[]` pointer array |
| `nlmsg_find_attr` | Find first attr of given type |
| `nlmsg_valid_hdr` / `nlmsg_ok` | Validate header/remaining |
| `nlmsg_next` / `nlmsg_for_each_msg` | Multi-message iteration |
| `nlmsg_set_proto` / `nlmsg_get_proto` | Protocol field access |
| `nlmsg_set_src` / `nlmsg_get_src` | Source address access |
| `nlmsg_set_dst` / `nlmsg_get_dst` | Destination address access |
| `nlmsg_set_credentials` / `nlmsg_get_credentials` | No-op stubs |

### Netlink attributes (`attr.rs`)

All primary attribute accessors are implemented:

| Functions | Notes |
|---|---|
| `nla_put` | Write attr header + data into message buffer |
| `nla_put_u8/u16/u32/u64` | Typed integer puts |
| `nla_put_s8/s16/s32/s64` | Signed integer puts |
| `nla_put_string` | NUL-terminated string put |
| `nla_put_flag` | Zero-length attribute |
| `nla_put_msecs` | u64 milliseconds |
| `nla_put_nested` | Inline copy of another message's attrs |
| `nla_put_addr` | Stub — returns 0 (nl_addr not implemented) |
| `nla_get_u8/u16/u32/u64` | Typed integer gets |
| `nla_get_s8/s16/s32/s64` | Signed integer gets |
| `nla_get_string` | Returns pointer to attr data |
| `nla_get_flag` | Returns true if attr present |
| `nla_get_msecs` | u64 milliseconds |
| `nla_parse` | Walk attr list, fill `tb[]` by type |
| `nla_parse_nested` | Parse nested attrs from parent attr |
| `nla_find` | Find first attr of type in list |
| `nla_ok` / `nla_next` | List iteration |
| `nla_data` / `nla_len` / `nla_type` | Attr introspection |
| `nla_total_size` / `nla_attr_size` / `nla_padlen` | Size calculations |
| `nla_reserve` / `nla_reserve_attr` | Reserve space, return attr ptr |
| `nla_nest_start` / `nla_nest_end` / `nla_nest_cancel` | Nested attr construction |
| `nla_memcpy` / `nla_memcmp` / `nla_strcmp` | Attr data comparisons |
| `nla_is_nested` | NLA_F_NESTED flag check |
| `nla_copy` | Append copy of another attr |

### Generic netlink (`genl.rs`)

| Symbol | Notes |
|---|---|
| `genl_connect` | `nl_connect(sk, NETLINK_GENERIC)` |
| `genlmsg_put` | `nlmsg_put` + `genlmsghdr` + returns data ptr |
| `genlmsg_hdr` | Returns `genlmsghdr *` after `nlmsghdr` |
| `genlmsg_data` | Returns payload after `genlmsghdr` |
| `genlmsg_attrdata` / `genlmsg_attrlen` | Attr region after user hdr |
| `genlmsg_valid_hdr` | Validates genl header |
| `genlmsg_parse` | Parses attrs from genl message |
| `genl_send_simple` | Alloc + genlmsg_put + send_auto + free |
| `genl_ctrl_resolve` | `CTRL_CMD_GETFAMILY` → family ID |
| `genl_ctrl_resolve_grp` | `CTRL_CMD_GETFAMILY` → multicast group ID |
| `genl_ctrl_alloc_cache` | `NLM_F_DUMP` all families, returns `GenlCache` |
| `genl_ctrl_search` | Find family in cache by ID |
| `genl_ctrl_search_by_name` | Find family in cache by name |
| `genl_family_get_id` / `genl_family_get_name` | Cache entry accessors |
| `genl_family_put` | Free cache entry |

### Error handling (`error.rs`)

| Symbol | Notes |
|---|---|
| `nl_geterror` | Returns string for `NLE_*` error code |
| `nl_perror` | Writes `"prefix: message\n"` to stderr (fd 2) |
| `nl_syserr2nlerr` | Maps POSIX errno to negative `NLE_*` |
| `nl_strerror_l` | Alias for `nl_geterror` with inverted sign |

All 35 `NLE_*` error codes (0–34) are defined.

---

## What is NOT implemented (link-compat stubs only)

These symbols are exported so binaries that reference unused libnl helpers can
link, but they are not functional implementations.  They return null, zero, or
an error as noted below and do not extend nloxide into a general-purpose libnl
replacement.

### nl_addr — network address objects

The entire `nl_addr_*` family returns null or error immediately.  `hostapd` and
`wpa_supplicant` do not use `nl_addr` on the nl80211 code path; these stubs
exist only to satisfy the linker.

```
nl_addr_alloc, nl_addr_put, nl_addr_get, nl_addr_alloc_attr,
nl_addr_build, nl_addr_clone, nl_addr_cmp, nl_addr_cmp_prefix,
nl_addr_iszero, nl_addr_valid, nl_addr_guess_family,
nl_addr_fill_sockaddr, nl_addr_resolve, nl_addr_set/get_family,
nl_addr_set/get_binary_addr, nl_addr_get_len,
nl_addr_set/get_prefixlen, nl_addr_parse, nl_addr2str,
nl_addr_info, nl_addr_shared
```

### String conversion utilities

These exist in libnl for CLI tools and pretty-printing.  None are used by the
nl80211 driver path.  All return null or 0.

```
nl_af2str, nl_str2af, nl_nlfamily2str, nl_str2nlfamily,
nl_llproto2str, nl_str2llproto, nl_ether_proto2str, nl_str2ether_proto,
nl_ip_proto2str, nl_str2ip_proto, nl_cancel_down_bytes,
nl_cancel_down_bits, nl_cancel_down_us, nl_size2int, nl_size2str,
nl_prob2int, nl_rate2str, nl_us2ticks, nl_ticks2us, nl_str2msec,
nl_msec2str, nl_get_user_hz (hardcoded 100), nl_get_psched_hz (1000)
```

### Dump / output API

```
nl_new_line, nl_dump, nl_dump_line
```

No-op stubs.  The dump infrastructure (`nl_dump_params`) is not implemented.

### nl_cache (route/link/addr cache)

The full `libnl-route-3` object cache layer (`nl_cache_*`, `rtnl_*`) is absent.
nloxide is not a replacement for `libnl-route-3`, and its `libnl-3` /
`libnl-genl-3` compatibility is limited to the jonerix nl80211 consumer paths
described above.

### Variadic memberships

`nl_socket_add_memberships` and `nl_socket_drop_memberships` accept exactly one
group argument in this implementation.  The real libnl takes a 0-terminated
va_list of groups.  wpa_supplicant and hostapd call these through a wrapper that
loops single-group calls, so this is not a problem in practice.

### nl_send_iovec

The `iov` / `iovlen` arguments are ignored; the call falls through to
`nl_sendmsg`.  Sufficient for nl80211 usage.

### nl_msg credentials

`nlmsg_set_credentials` and `nlmsg_get_credentials` are no-ops.  SCM_CREDENTIALS
passing is not implemented.

### nl_cb kinds other than NL_CB_CUSTOM

`NL_CB_DEFAULT`, `NL_CB_VERBOSE`, and `NL_CB_DEBUG` are accepted without error
but treated identically — the supplied function pointer is always installed.
The verbose/debug output that libnl produces for these kinds is not replicated.

---

## ABI compatibility notes

ABI compatibility is maintained for the narrow hostapd/wpa_supplicant nl80211
contract.  Exported stubs are for link compatibility only and are not evidence
of broader runtime compatibility with libnl applications.

The `NlSock` struct field layout matches the offsets observed in the Ghidra
decompilation of libnl3-3.11.0:

| Field | Offset |
|---|---|
| `s_local` (sockaddr_nl) | +0x00 |
| `s_peer` (sockaddr_nl) | +0x0C |
| `s_fd` | +0x18 |
| `s_proto` | +0x1C |
| `s_seq_next` | +0x20 |
| `s_seq_expect` | +0x24 |
| `s_flags` | +0x28 |
| `s_cb *` | +0x30 |

`NlCb` is 224 bytes (`0xE0`), with 11 callback slots (indices 0–10) and one
error callback slot.  `NlMsg` keeps the `nlmsghdr *` reachable at the offset
expected by `nlmsg_hdr()`.

These internal layouts are only ABI-relevant if a caller inspects struct internals
directly (which no well-written consumer should do).  hostapd and wpa_supplicant
treat all three types as opaque.

---

## License

BSD-2-Clause.  See `LICENSE` in the root of the jonerix repository.

Dependency: [`libc`](https://crates.io/crates/libc) crate (MIT/Apache-2.0,
vendored in `vendor/`).
