//! Linux interface operations for the runtime daemon.

use std::ffi::CString;
use std::fs;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::PathBuf;

use crate::config::{BalancingMode, Config, LinkMode, MacMode, Vip6Addr, VipAddr};
use crate::io::virtual_mac;

const AF_UNSPEC: u8 = 0;
const AF_INET: c_int = 2;
const AF_INET6: c_int = 10;
const AF_NETLINK: c_int = 16;
const AF_PACKET: c_int = 17;
const SOCK_RAW: c_int = 3;
const SOCK_CLOEXEC: c_int = 0o2000000;
const NETLINK_ROUTE: c_int = 0;

const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_ACK: u16 = 0x0004;
const NLM_F_EXCL: u16 = 0x0200;
const NLM_F_CREATE: u16 = 0x0400;
const NLMSG_ERROR: u16 = 0x0002;

const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_SETLINK: u16 = RTM_NEWLINK + 3;
const RTM_NEWADDR: u16 = 20;
const RTM_DELADDR: u16 = 21;

const IFLA_ADDRESS: u16 = 1;
const IFLA_IFNAME: u16 = 3;
const IFLA_LINK: u16 = 5;
const IFLA_LINKINFO: u16 = 18;
const IFLA_INFO_KIND: u16 = 1;
const IFLA_INFO_DATA: u16 = 2;
const IFLA_MACVLAN_MODE: u16 = 1;
const IFLA_IPVLAN_MODE: u16 = 1;
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;

const ETH_P_IP: u16 = 0x0800;
const ETH_P_ARP: u16 = 0x0806;
const ETH_P_IPV6: u16 = 0x86dd;
const ARPHRD_ETHER: u16 = 1;
const ARP_REQUEST: u16 = 1;
const ARP_REPLY: u16 = 2;

const EEXIST: i32 = 17;
const ENODEV: i32 = 19;
const ESRCH: i32 = 3;
const EADDRNOTAVAIL: i32 = 99;
const IFNAMSIZ: usize = 16;
const IFF_UP: u32 = 0x1;
const IFLA_MACVLAN_MODE_PRIVATE: u32 = 1;
const IFLA_MACVLAN_MODE_BRIDGE: u32 = 4;
const IPVLAN_MODE_L2: u16 = 0;

#[repr(C)]
struct SockaddrNl {
    nl_family: u16,
    nl_pad: u16,
    nl_pid: u32,
    nl_groups: u32,
}

#[repr(C)]
struct SockaddrLl {
    sll_family: u16,
    sll_protocol: u16,
    sll_ifindex: c_int,
    sll_hatype: u16,
    sll_pkttype: u8,
    sll_halen: u8,
    sll_addr: [u8; 8],
}

extern "C" {
    fn if_nametoindex(ifname: *const c_char) -> c_uint;
    fn socket(domain: c_int, type_: c_int, protocol: c_int) -> c_int;
    fn sendto(
        socket: c_int,
        buffer: *const c_void,
        length: usize,
        flags: c_int,
        dest_addr: *const c_void,
        dest_len: u32,
    ) -> isize;
    fn recv(socket: c_int, buffer: *mut c_void, length: usize, flags: c_int) -> isize;
    fn close(fd: c_int) -> c_int;
}

#[derive(Debug)]
pub struct InterfaceRuntime {
    name: String,
    ifindex: u32,
    parent_name: String,
    original_mac: [u8; 6],
    mac_mode: MacMode,
    created_child: Option<String>,
}

impl InterfaceRuntime {
    pub fn open(cfg: &Config) -> io::Result<Self> {
        let name = cfg.effective_interface();
        let parent_name = cfg.parent_interface().to_string();
        validate_interface_name(name)?;
        validate_interface_name(&parent_name)?;
        let created_child = if cfg.link_mode_uses_child() {
            if ensure_child_link(cfg)? {
                Some(name.to_string())
            } else {
                None
            }
        } else {
            None
        };
        let ifindex = interface_index(name)?;
        let original_mac = match read_interface_mac(name) {
            Ok(mac) => mac,
            Err(err) => {
                if let Some(name) = created_child.as_deref() {
                    let _ = delete_link_by_name(name);
                }
                return Err(err);
            }
        };
        Ok(Self {
            name: name.to_string(),
            ifindex,
            parent_name,
            original_mac,
            mac_mode: cfg.mac_mode,
            created_child,
        })
    }

    pub fn enter_master(&self, cfg: &Config) -> io::Result<()> {
        let mac = self.effective_mac(cfg);
        if self.mac_mode == MacMode::Virtual {
            set_link_mac(self.ifindex, mac)?;
        }
        if cfg.manage_vip {
            for vip in &cfg.vip_addrs {
                add_ipv4_addr(self.ifindex, *vip)?;
            }
            for vip in &cfg.vip6_addrs {
                add_ipv6_addr(self.ifindex, *vip)?;
            }
        }
        if cfg.announce && cfg.balancing == BalancingMode::None {
            for vip in &cfg.vip_addrs {
                send_gratuitous_arp(self.ifindex, mac, vip.addr)?;
            }
            for vip in &cfg.vip6_addrs {
                send_unsolicited_na(self.ifindex, mac, vip.addr)?;
            }
        }
        Ok(())
    }

    pub fn enter_backup(&self, cfg: &Config) -> io::Result<()> {
        if cfg.manage_vip {
            for vip in &cfg.vip_addrs {
                remove_ipv4_addr(self.ifindex, *vip)?;
            }
            for vip in &cfg.vip6_addrs {
                remove_ipv6_addr(self.ifindex, *vip)?;
            }
        }
        if self.mac_mode == MacMode::Virtual {
            set_link_mac(self.ifindex, self.original_mac)?;
        }
        Ok(())
    }

    pub fn cleanup(&self, cfg: &Config) {
        let _ = self.enter_backup(cfg);
        if let Some(name) = self.created_child.as_deref() {
            let _ = delete_link_by_name(name);
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn link_is_running(&self) -> io::Result<bool> {
        let effective = read_interface_running(&self.name)?;
        if self.parent_name == self.name {
            return Ok(effective);
        }
        Ok(effective && read_interface_running(&self.parent_name)?)
    }

    fn effective_mac(&self, cfg: &Config) -> [u8; 6] {
        match self.mac_mode {
            MacMode::Virtual => virtual_mac_for_mode(cfg.vhid, cfg.balancing),
            MacMode::Interface => self.original_mac,
        }
    }
}

fn virtual_mac_for_mode(vhid: u8, balancing: BalancingMode) -> [u8; 6] {
    let mut mac = virtual_mac(vhid);
    if balancing == BalancingMode::Ip {
        mac[0] = 0x01;
    }
    mac
}

fn ensure_child_link(cfg: &Config) -> io::Result<bool> {
    let child_name = cfg.effective_interface();
    let parent_name = cfg.parent_interface();
    if child_name == parent_name {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "link_name must differ from link_parent for child link modes",
        ));
    }
    let parent_index = interface_index(parent_name)?;
    create_child_link(cfg.link_mode, child_name, parent_index)
}

fn validate_interface_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name.len() >= IFNAMSIZ
        || name.as_bytes().contains(&0)
        || name.contains('/')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid interface name",
        ));
    }
    Ok(())
}

fn interface_index(name: &str) -> io::Result<u32> {
    let name = CString::new(name).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "interface name contains NUL byte",
        )
    })?;
    let index = unsafe { if_nametoindex(name.as_ptr()) };
    if index == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(index)
    }
}

fn read_interface_mac(name: &str) -> io::Result<[u8; 6]> {
    let mut path = PathBuf::from("/sys/class/net");
    path.push(name);
    path.push("address");
    parse_mac(fs::read_to_string(path)?.trim())
}

fn read_interface_running(name: &str) -> io::Result<bool> {
    let mut carrier = PathBuf::from("/sys/class/net");
    carrier.push(name);
    carrier.push("carrier");
    match fs::read_to_string(&carrier) {
        Ok(raw) => return Ok(raw.trim() == "1"),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    let mut operstate = PathBuf::from("/sys/class/net");
    operstate.push(name);
    operstate.push("operstate");
    match fs::read_to_string(operstate)?.trim() {
        "down" | "lowerlayerdown" | "dormant" | "notpresent" => Ok(false),
        _ => Ok(true),
    }
}

fn parse_mac(raw: &str) -> io::Result<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = raw.split(':');
    for byte in &mut mac {
        let Some(part) = parts.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "short MAC address",
            ));
        };
        *byte = u8::from_str_radix(part, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid MAC address"))?;
    }
    if parts.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "long MAC address",
        ));
    }
    Ok(mac)
}

fn set_link_mac(ifindex: u32, mac: [u8; 6]) -> io::Result<()> {
    let mut msg = NetlinkMessage::new(RTM_SETLINK, NLM_F_REQUEST | NLM_F_ACK, 1);
    msg.push_ifinfomsg(ifindex);
    msg.push_attr(IFLA_ADDRESS, &mac);
    send_netlink(msg.finish())
}

fn set_link_up(ifindex: u32) -> io::Result<()> {
    let mut msg = NetlinkMessage::new(RTM_SETLINK, NLM_F_REQUEST | NLM_F_ACK, 1);
    msg.push_ifinfomsg_flags(ifindex, IFF_UP, IFF_UP);
    send_netlink(msg.finish())
}

fn create_child_link(link_mode: LinkMode, child_name: &str, parent_index: u32) -> io::Result<bool> {
    let message = build_child_link_newlink_message(link_mode, child_name, parent_index)?;
    match send_netlink(message) {
        Ok(()) => match interface_index(child_name).and_then(set_link_up) {
            Ok(()) => Ok(true),
            Err(err) => {
                let _ = delete_link_by_name(child_name);
                Err(err)
            }
        },
        Err(err) if err.raw_os_error() == Some(EEXIST) => {
            let ifindex = interface_index(child_name)?;
            set_link_up(ifindex)?;
            Ok(false)
        }
        Err(err) => Err(err),
    }
}

fn delete_link_by_name(name: &str) -> io::Result<()> {
    let ifindex = match interface_index(name) {
        Ok(index) => index,
        Err(err) if err.raw_os_error() == Some(ENODEV) => return Ok(()),
        Err(err) => return Err(err),
    };
    let mut msg = NetlinkMessage::new(RTM_DELLINK, NLM_F_REQUEST | NLM_F_ACK, 1);
    msg.push_ifinfomsg(ifindex);
    match send_netlink(msg.finish()) {
        Ok(()) => Ok(()),
        Err(err) if matches!(err.raw_os_error(), Some(ESRCH) | Some(ENODEV)) => Ok(()),
        Err(err) => Err(err),
    }
}

fn build_child_link_newlink_message(
    link_mode: LinkMode,
    child_name: &str,
    parent_index: u32,
) -> io::Result<Vec<u8>> {
    validate_interface_name(child_name)?;
    let mut msg = NetlinkMessage::new(
        RTM_NEWLINK,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
        1,
    );
    msg.push_ifinfomsg(0);
    msg.push_attr(IFLA_IFNAME, &nul_terminated(child_name));
    msg.push_attr(IFLA_LINK, &(parent_index as i32).to_ne_bytes());
    let (kind, mode_payload) = match link_mode {
        LinkMode::MacvlanBridge => (
            "macvlan",
            nlattr_payload(IFLA_MACVLAN_MODE, &IFLA_MACVLAN_MODE_BRIDGE.to_ne_bytes()),
        ),
        LinkMode::MacvlanPrivate => (
            "macvlan",
            nlattr_payload(IFLA_MACVLAN_MODE, &IFLA_MACVLAN_MODE_PRIVATE.to_ne_bytes()),
        ),
        LinkMode::IpvlanL2 => (
            "ipvlan",
            nlattr_payload(IFLA_IPVLAN_MODE, &IPVLAN_MODE_L2.to_ne_bytes()),
        ),
        LinkMode::Parent => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "parent link_mode does not create a child link",
            ))
        }
    };
    let info_payload = nested_attr_payload(&[
        (IFLA_INFO_KIND, nul_terminated(kind).as_slice()),
        (IFLA_INFO_DATA, mode_payload.as_slice()),
    ]);
    msg.push_attr(IFLA_LINKINFO, &info_payload);
    Ok(msg.finish())
}

fn nested_attr_payload(entries: &[(u16, &[u8])]) -> Vec<u8> {
    let mut payload = Vec::new();
    for (attr_type, data) in entries {
        push_nlattr(&mut payload, *attr_type, data);
    }
    payload
}

fn nlattr_payload(attr_type: u16, data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::new();
    push_nlattr(&mut payload, attr_type, data);
    payload
}

fn nul_terminated(value: &str) -> Vec<u8> {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

fn push_nlattr(out: &mut Vec<u8>, attr_type: u16, data: &[u8]) {
    let len = (4 + data.len()) as u16;
    out.extend_from_slice(&len.to_ne_bytes());
    out.extend_from_slice(&attr_type.to_ne_bytes());
    out.extend_from_slice(data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
}

fn add_ipv4_addr(ifindex: u32, vip: VipAddr) -> io::Result<()> {
    let mut msg = NetlinkMessage::new(
        RTM_NEWADDR,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
        1,
    );
    msg.push_ifaddrmsg(ifindex, vip.prefix_len);
    msg.push_attr(IFA_LOCAL, &vip.addr.octets());
    msg.push_attr(IFA_ADDRESS, &vip.addr.octets());
    match send_netlink(msg.finish()) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(EEXIST) => Ok(()),
        Err(err) => Err(err),
    }
}

fn remove_ipv4_addr(ifindex: u32, vip: VipAddr) -> io::Result<()> {
    let mut msg = NetlinkMessage::new(RTM_DELADDR, NLM_F_REQUEST | NLM_F_ACK, 1);
    msg.push_ifaddrmsg(ifindex, vip.prefix_len);
    msg.push_attr(IFA_LOCAL, &vip.addr.octets());
    msg.push_attr(IFA_ADDRESS, &vip.addr.octets());
    match send_netlink(msg.finish()) {
        Ok(()) => Ok(()),
        Err(err) if matches!(err.raw_os_error(), Some(EADDRNOTAVAIL) | Some(ESRCH)) => Ok(()),
        Err(err) => Err(err),
    }
}

fn add_ipv6_addr(ifindex: u32, vip: Vip6Addr) -> io::Result<()> {
    let mut msg = NetlinkMessage::new(
        RTM_NEWADDR,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
        1,
    );
    msg.push_ifaddrmsg_family(ifindex, vip.prefix_len, AF_INET6 as u8);
    msg.push_attr(IFA_LOCAL, &vip.addr.octets());
    msg.push_attr(IFA_ADDRESS, &vip.addr.octets());
    match send_netlink(msg.finish()) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(EEXIST) => Ok(()),
        Err(err) => Err(err),
    }
}

fn remove_ipv6_addr(ifindex: u32, vip: Vip6Addr) -> io::Result<()> {
    let mut msg = NetlinkMessage::new(RTM_DELADDR, NLM_F_REQUEST | NLM_F_ACK, 1);
    msg.push_ifaddrmsg_family(ifindex, vip.prefix_len, AF_INET6 as u8);
    msg.push_attr(IFA_LOCAL, &vip.addr.octets());
    msg.push_attr(IFA_ADDRESS, &vip.addr.octets());
    match send_netlink(msg.finish()) {
        Ok(()) => Ok(()),
        Err(err) if matches!(err.raw_os_error(), Some(EADDRNOTAVAIL) | Some(ESRCH)) => Ok(()),
        Err(err) => Err(err),
    }
}

struct NetlinkMessage {
    bytes: Vec<u8>,
}

impl NetlinkMessage {
    fn new(message_type: u16, flags: u16, sequence: u32) -> Self {
        let mut bytes = Vec::with_capacity(128);
        bytes.extend_from_slice(&0u32.to_ne_bytes());
        bytes.extend_from_slice(&message_type.to_ne_bytes());
        bytes.extend_from_slice(&flags.to_ne_bytes());
        bytes.extend_from_slice(&sequence.to_ne_bytes());
        bytes.extend_from_slice(&0u32.to_ne_bytes());
        Self { bytes }
    }

    fn push_ifinfomsg(&mut self, ifindex: u32) {
        self.push_ifinfomsg_flags(ifindex, 0, 0);
    }

    fn push_ifinfomsg_flags(&mut self, ifindex: u32, flags: u32, change: u32) {
        self.bytes.push(AF_UNSPEC);
        self.bytes.push(0);
        self.bytes.extend_from_slice(&0u16.to_ne_bytes());
        self.bytes
            .extend_from_slice(&(ifindex as i32).to_ne_bytes());
        self.bytes.extend_from_slice(&flags.to_ne_bytes());
        self.bytes.extend_from_slice(&change.to_ne_bytes());
    }

    fn push_ifaddrmsg(&mut self, ifindex: u32, prefix_len: u8) {
        self.push_ifaddrmsg_family(ifindex, prefix_len, AF_INET as u8);
    }

    fn push_ifaddrmsg_family(&mut self, ifindex: u32, prefix_len: u8, family: u8) {
        self.bytes.push(family);
        self.bytes.push(prefix_len);
        self.bytes.push(0);
        self.bytes.push(0);
        self.bytes.extend_from_slice(&ifindex.to_ne_bytes());
    }

    fn push_attr(&mut self, attr_type: u16, data: &[u8]) {
        let len = (4 + data.len()) as u16;
        self.bytes.extend_from_slice(&len.to_ne_bytes());
        self.bytes.extend_from_slice(&attr_type.to_ne_bytes());
        self.bytes.extend_from_slice(data);
        while self.bytes.len() % 4 != 0 {
            self.bytes.push(0);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        let len = self.bytes.len() as u32;
        self.bytes[..4].copy_from_slice(&len.to_ne_bytes());
        self.bytes
    }
}

fn send_netlink(message: Vec<u8>) -> io::Result<()> {
    let fd = unsafe { socket(AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, NETLINK_ROUTE) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = send_netlink_inner(fd, &message);
    unsafe {
        close(fd);
    }
    result
}

fn send_netlink_inner(fd: c_int, message: &[u8]) -> io::Result<()> {
    let kernel = SockaddrNl {
        nl_family: AF_NETLINK as u16,
        nl_pad: 0,
        nl_pid: 0,
        nl_groups: 0,
    };
    let sent = unsafe {
        sendto(
            fd,
            message.as_ptr().cast(),
            message.len(),
            0,
            (&kernel as *const SockaddrNl).cast(),
            std::mem::size_of::<SockaddrNl>() as u32,
        )
    };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut ack = [0u8; 512];
    let got = unsafe { recv(fd, ack.as_mut_ptr().cast(), ack.len(), 0) };
    if got < 0 {
        return Err(io::Error::last_os_error());
    }
    parse_netlink_ack(&ack[..got as usize])
}

fn parse_netlink_ack(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() < 20 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "short netlink ACK",
        ));
    }
    let message_type = u16::from_ne_bytes([bytes[4], bytes[5]]);
    if message_type != NLMSG_ERROR {
        return Ok(());
    }
    let error = i32::from_ne_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    if error == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(-error))
    }
}

fn send_gratuitous_arp(ifindex: u32, mac: [u8; 6], addr: Ipv4Addr) -> io::Result<()> {
    send_arp_frame(ifindex, &arp_frame(ARP_REQUEST, mac, addr, [0; 6]))?;
    send_arp_frame(ifindex, &arp_frame(ARP_REPLY, mac, addr, [0xff; 6]))
}

fn send_unsolicited_na(ifindex: u32, mac: [u8; 6], addr: Ipv6Addr) -> io::Result<()> {
    send_ethernet_frame(
        ifindex,
        ETH_P_IPV6,
        &neighbor_advertisement_frame(mac, addr),
    )
}

fn send_arp_frame(ifindex: u32, frame: &[u8; 42]) -> io::Result<()> {
    send_ethernet_frame(ifindex, ETH_P_ARP, frame)
}

fn send_ethernet_frame(ifindex: u32, ethertype: u16, frame: &[u8]) -> io::Result<()> {
    let fd = unsafe {
        socket(
            AF_PACKET,
            SOCK_RAW | SOCK_CLOEXEC,
            ethertype.to_be() as c_int,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut addr = SockaddrLl {
        sll_family: AF_PACKET as u16,
        sll_protocol: ethertype.to_be(),
        sll_ifindex: ifindex as c_int,
        sll_hatype: ARPHRD_ETHER,
        sll_pkttype: 0,
        sll_halen: 6,
        sll_addr: [0; 8],
    };
    addr.sll_addr[..6].copy_from_slice(&frame[..6]);
    addr.sll_addr[6] = 0;
    addr.sll_addr[7] = 0;
    let sent = unsafe {
        sendto(
            fd,
            frame.as_ptr().cast(),
            frame.len(),
            0,
            (&addr as *const SockaddrLl).cast(),
            std::mem::size_of::<SockaddrLl>() as u32,
        )
    };
    let err = if sent < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    };
    unsafe {
        close(fd);
    }
    err
}

fn arp_frame(opcode: u16, mac: [u8; 6], addr: Ipv4Addr, target_mac: [u8; 6]) -> [u8; 42] {
    let mut frame = [0u8; 42];
    frame[..6].fill(0xff);
    frame[6..12].copy_from_slice(&mac);
    frame[12..14].copy_from_slice(&ETH_P_ARP.to_be_bytes());
    frame[14..16].copy_from_slice(&ARPHRD_ETHER.to_be_bytes());
    frame[16..18].copy_from_slice(&ETH_P_IP.to_be_bytes());
    frame[18] = 6;
    frame[19] = 4;
    frame[20..22].copy_from_slice(&opcode.to_be_bytes());
    frame[22..28].copy_from_slice(&mac);
    frame[28..32].copy_from_slice(&addr.octets());
    frame[32..38].copy_from_slice(&target_mac);
    frame[38..42].copy_from_slice(&addr.octets());
    frame
}

fn neighbor_advertisement_frame(mac: [u8; 6], target: Ipv6Addr) -> [u8; 86] {
    let dst = "ff02::1".parse::<Ipv6Addr>().unwrap();
    let mut frame = [0u8; 86];
    frame[..6].copy_from_slice(&[0x33, 0x33, 0, 0, 0, 1]);
    frame[6..12].copy_from_slice(&mac);
    frame[12..14].copy_from_slice(&ETH_P_IPV6.to_be_bytes());

    frame[14] = 0x60;
    frame[18..20].copy_from_slice(&(32u16).to_be_bytes());
    frame[20] = 58;
    frame[21] = 255;
    frame[22..38].copy_from_slice(&target.octets());
    frame[38..54].copy_from_slice(&dst.octets());

    frame[54] = 136;
    frame[55] = 0;
    frame[58..62].copy_from_slice(&0x20000000u32.to_be_bytes());
    frame[62..78].copy_from_slice(&target.octets());
    frame[78] = 2;
    frame[79] = 1;
    frame[80..86].copy_from_slice(&mac);

    let checksum = icmpv6_checksum(target, dst, &frame[54..86]);
    frame[56..58].copy_from_slice(&checksum.to_be_bytes());
    frame
}

fn icmpv6_checksum(source: Ipv6Addr, destination: Ipv6Addr, payload: &[u8]) -> u16 {
    let mut pseudo = Vec::with_capacity(40 + payload.len());
    pseudo.extend_from_slice(&source.octets());
    pseudo.extend_from_slice(&destination.octets());
    pseudo.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    pseudo.extend_from_slice(&[0, 0, 0, 58]);
    pseudo.extend_from_slice(payload);
    internet_checksum(&pseudo)
}

fn internet_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    if let Some(&last) = chunks.remainder().first() {
        sum += (last as u32) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sysfs_mac() {
        assert_eq!(
            parse_mac("52:54:00:42:00:31").unwrap(),
            [0x52, 0x54, 0x00, 0x42, 0x00, 0x31]
        );
        assert!(parse_mac("52:54").is_err());
    }

    #[test]
    fn gratuitous_arp_frame_uses_virtual_sender() {
        let mac = [0, 0, 0x5e, 0, 1, 42];
        let frame = arp_frame(ARP_REPLY, mac, Ipv4Addr::new(10, 0, 253, 42), [0xff; 6]);
        assert_eq!(&frame[..6], &[0xff; 6]);
        assert_eq!(&frame[6..12], &mac);
        assert_eq!(&frame[12..14], &ETH_P_ARP.to_be_bytes());
        assert_eq!(&frame[20..22], &ARP_REPLY.to_be_bytes());
        assert_eq!(&frame[22..28], &mac);
        assert_eq!(&frame[28..32], &[10, 0, 253, 42]);
        assert_eq!(&frame[32..38], &[0xff; 6]);
        assert_eq!(&frame[38..42], &[10, 0, 253, 42]);
    }

    #[test]
    fn unsolicited_na_frame_uses_hop_limit_255_and_override() {
        let mac = [0, 0, 0x5e, 0, 1, 42];
        let frame = neighbor_advertisement_frame(mac, "2001:db8::42".parse().unwrap());
        assert_eq!(&frame[..6], &[0x33, 0x33, 0, 0, 0, 1]);
        assert_eq!(&frame[6..12], &mac);
        assert_eq!(&frame[12..14], &ETH_P_IPV6.to_be_bytes());
        assert_eq!(frame[20], 58);
        assert_eq!(frame[21], 255);
        assert_eq!(frame[54], 136);
        assert_eq!(&frame[58..62], &0x20000000u32.to_be_bytes());
        assert_eq!(frame[78], 2);
        assert_eq!(frame[79], 1);
        assert_eq!(&frame[80..86], &mac);
        assert_eq!(
            icmpv6_checksum(
                "2001:db8::42".parse().unwrap(),
                "ff02::1".parse().unwrap(),
                &frame[54..86]
            ),
            0
        );
    }

    #[test]
    fn balancing_ip_uses_openbsd_multicast_virtual_mac_shape() {
        assert_eq!(
            virtual_mac_for_mode(42, BalancingMode::Ip),
            [0x01, 0x00, 0x5e, 0x00, 0x01, 42]
        );
        assert_eq!(
            virtual_mac_for_mode(42, BalancingMode::IpStealth),
            [0x00, 0x00, 0x5e, 0x00, 0x01, 42]
        );
    }

    #[test]
    fn child_link_newlink_message_uses_expected_kinds_and_modes() {
        let msg = build_child_link_newlink_message(LinkMode::MacvlanBridge, "carp42", 7).unwrap();
        assert!(msg.windows(b"carp42\0".len()).any(|w| w == b"carp42\0"));
        assert!(msg.windows(b"macvlan\0".len()).any(|w| w == b"macvlan\0"));
        assert!(msg
            .windows(std::mem::size_of::<u32>())
            .any(|w| w == IFLA_MACVLAN_MODE_BRIDGE.to_ne_bytes()));

        let msg = build_child_link_newlink_message(LinkMode::MacvlanPrivate, "carp42", 7).unwrap();
        assert!(msg
            .windows(std::mem::size_of::<u32>())
            .any(|w| w == IFLA_MACVLAN_MODE_PRIVATE.to_ne_bytes()));

        let msg = build_child_link_newlink_message(LinkMode::IpvlanL2, "carp42", 7).unwrap();
        assert!(msg.windows(b"ipvlan\0".len()).any(|w| w == b"ipvlan\0"));
        assert!(msg
            .windows(std::mem::size_of::<u16>())
            .any(|w| w == IPVLAN_MODE_L2.to_ne_bytes()));
    }
}
