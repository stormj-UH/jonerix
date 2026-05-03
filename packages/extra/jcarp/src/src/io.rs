//! Linux I/O adapter for CARP advertisements.
//!
//! The protocol core is testable without privileges; this module is the
//! narrow runtime boundary that needs `CAP_NET_RAW`.

use std::ffi::CString;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::fd::{AsRawFd, RawFd};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::time::Duration;

pub const IPPROTO_CARP: c_int = 112;
pub const CARP_TTL: c_int = 255;
pub const CARP_IPV4_MULTICAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 18);
pub const CARP_IPV6_MULTICAST: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x12);

const AF_INET: c_int = 2;
const AF_INET6: c_int = 10;
const SOCK_RAW: c_int = 3;
const SOCK_CLOEXEC: c_int = 0o2000000;
const SOL_IP: c_int = 0;
const SOL_IPV6: c_int = 41;
const IP_TTL: c_int = 2;
const IP_MULTICAST_TTL: c_int = 33;
const IP_MULTICAST_LOOP: c_int = 34;
const IPV6_UNICAST_HOPS: c_int = 16;
const IPV6_MULTICAST_IF: c_int = 17;
const IPV6_MULTICAST_HOPS: c_int = 18;
const IPV6_MULTICAST_LOOP: c_int = 19;
const IPV6_JOIN_GROUP: c_int = 20;
const IPV6_RECVPKTINFO: c_int = 49;
const IPV6_PKTINFO: c_int = 50;
const IPV6_RECVHOPLIMIT: c_int = 51;
const IPV6_HOPLIMIT: c_int = 52;
const SOL_SOCKET: c_int = 1;
const SO_BINDTODEVICE: c_int = 25;
const POLLIN: i16 = 0x0001;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedAdvertisement {
    pub source: Ipv4Addr,
    pub destination: Ipv4Addr,
    pub ttl: u8,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceivedAdvertisement6 {
    pub source: Ipv6Addr,
    pub destination: Ipv6Addr,
    pub hop_limit: u8,
    pub payload: Vec<u8>,
}

#[repr(C)]
struct InAddr {
    s_addr: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct In6Addr {
    s6_addr: [u8; 16],
}

#[repr(C)]
struct SockaddrIn {
    sin_family: u16,
    sin_port: u16,
    sin_addr: InAddr,
    sin_zero: [u8; 8],
}

#[repr(C)]
struct SockaddrIn6 {
    sin6_family: u16,
    sin6_port: u16,
    sin6_flowinfo: u32,
    sin6_addr: In6Addr,
    sin6_scope_id: u32,
}

#[repr(C)]
struct Ipv6Mreq {
    ipv6mr_multiaddr: In6Addr,
    ipv6mr_interface: c_uint,
}

#[repr(C)]
struct In6Pktinfo {
    ipi6_addr: In6Addr,
    ipi6_ifindex: c_uint,
}

#[repr(C)]
struct Iovec {
    iov_base: *mut c_void,
    iov_len: usize,
}

#[repr(C)]
struct Msghdr {
    msg_name: *mut c_void,
    msg_namelen: u32,
    msg_iov: *mut Iovec,
    msg_iovlen: usize,
    msg_control: *mut c_void,
    msg_controllen: usize,
    msg_flags: c_int,
}

#[repr(C)]
struct Cmsghdr {
    cmsg_len: usize,
    cmsg_level: c_int,
    cmsg_type: c_int,
}

extern "C" {
    fn if_nametoindex(ifname: *const c_char) -> c_uint;
    fn socket(domain: c_int, type_: c_int, protocol: c_int) -> c_int;
    fn setsockopt(
        socket: c_int,
        level: c_int,
        option_name: c_int,
        option_value: *const c_void,
        option_len: u32,
    ) -> c_int;
    fn sendto(
        socket: c_int,
        buffer: *const c_void,
        length: usize,
        flags: c_int,
        dest_addr: *const c_void,
        dest_len: u32,
    ) -> isize;
    fn recv(socket: c_int, buffer: *mut c_void, length: usize, flags: c_int) -> isize;
    fn recvmsg(socket: c_int, message: *mut Msghdr, flags: c_int) -> isize;
    fn poll(fds: *mut PollFd, nfds: usize, timeout: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

pub trait AdvertisementTransport {
    fn send_advertisement(&self, destination: Ipv4Addr, payload: &[u8]) -> io::Result<usize>;
}

pub trait AdvertisementTransport6 {
    fn send_advertisement6(&self, destination: Ipv6Addr, payload: &[u8]) -> io::Result<usize>;
}

#[derive(Debug)]
pub struct RawCarpSocket {
    fd: RawFd,
}

#[derive(Debug)]
pub struct RawCarpSocket6 {
    fd: RawFd,
    ifindex: u32,
}

impl RawCarpSocket {
    pub fn open_v4() -> io::Result<Self> {
        Self::open_v4_bound(None)
    }

    pub fn open_v4_for_interface(interface: &str) -> io::Result<Self> {
        Self::open_v4_bound(Some(interface))
    }

    fn open_v4_bound(interface: Option<&str>) -> io::Result<Self> {
        let fd = unsafe { socket(AF_INET, SOCK_RAW | SOCK_CLOEXEC, IPPROTO_CARP) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if let Err(err) = configure_carp_socket(fd, interface) {
            unsafe {
                close(fd);
            }
            return Err(err);
        }
        Ok(Self { fd })
    }

    pub fn recv_advertisement_timeout(
        &self,
        timeout: Option<Duration>,
    ) -> io::Result<Option<ReceivedAdvertisement>> {
        let timeout_ms = match timeout {
            Some(timeout) => duration_to_poll_timeout(timeout),
            None => -1,
        };
        let mut pfd = PollFd {
            fd: self.fd,
            events: POLLIN,
            revents: 0,
        };
        let ready = unsafe { poll(&mut pfd, 1, timeout_ms) };
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if ready == 0 {
            return Ok(None);
        }

        let mut bytes = vec![0u8; 2048];
        let got = unsafe { recv(self.fd, bytes.as_mut_ptr().cast(), bytes.len(), 0) };
        if got < 0 {
            return Err(io::Error::last_os_error());
        }
        bytes.truncate(got as usize);
        decode_ipv4_carp_packet(&bytes).map(Some)
    }
}

impl RawCarpSocket6 {
    pub fn open_for_interface(interface: &str) -> io::Result<Self> {
        let ifindex = interface_index(interface)?;
        let fd = unsafe { socket(AF_INET6, SOCK_RAW | SOCK_CLOEXEC, IPPROTO_CARP) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        if let Err(err) = configure_carp6_socket(fd, interface, ifindex) {
            unsafe {
                close(fd);
            }
            return Err(err);
        }
        Ok(Self { fd, ifindex })
    }

    pub fn recv_advertisement_timeout(
        &self,
        timeout: Option<Duration>,
    ) -> io::Result<Option<ReceivedAdvertisement6>> {
        let timeout_ms = match timeout {
            Some(timeout) => duration_to_poll_timeout(timeout),
            None => -1,
        };
        let mut pfd = PollFd {
            fd: self.fd,
            events: POLLIN,
            revents: 0,
        };
        let ready = unsafe { poll(&mut pfd, 1, timeout_ms) };
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if ready == 0 {
            return Ok(None);
        }

        let mut payload = vec![0u8; 2048];
        let mut control = vec![0u8; 256];
        let mut source = SockaddrIn6 {
            sin6_family: AF_INET6 as u16,
            sin6_port: 0,
            sin6_flowinfo: 0,
            sin6_addr: In6Addr { s6_addr: [0; 16] },
            sin6_scope_id: 0,
        };
        let mut iov = Iovec {
            iov_base: payload.as_mut_ptr().cast(),
            iov_len: payload.len(),
        };
        let mut msg = Msghdr {
            msg_name: (&mut source as *mut SockaddrIn6).cast(),
            msg_namelen: std::mem::size_of::<SockaddrIn6>() as u32,
            msg_iov: &mut iov,
            msg_iovlen: 1,
            msg_control: control.as_mut_ptr().cast(),
            msg_controllen: control.len(),
            msg_flags: 0,
        };
        let got = unsafe { recvmsg(self.fd, &mut msg, 0) };
        if got < 0 {
            return Err(io::Error::last_os_error());
        }
        payload.truncate(got as usize);
        let (destination, hop_limit) = parse_ipv6_control(&control[..msg.msg_controllen])?;
        Ok(Some(ReceivedAdvertisement6 {
            source: Ipv6Addr::from(source.sin6_addr.s6_addr),
            destination,
            hop_limit,
            payload,
        }))
    }
}

fn configure_carp_socket(fd: RawFd, interface: Option<&str>) -> io::Result<()> {
    set_carp_ttl(fd)?;
    disable_multicast_loop(fd)?;
    if let Some(interface) = interface {
        bind_to_device(fd, interface)?;
    }
    Ok(())
}

fn configure_carp6_socket(fd: RawFd, interface: &str, ifindex: u32) -> io::Result<()> {
    set_carp6_hops(fd)?;
    set_ipv6_int(fd, IPV6_MULTICAST_LOOP, 0)?;
    set_ipv6_int(fd, IPV6_RECVPKTINFO, 1)?;
    set_ipv6_int(fd, IPV6_RECVHOPLIMIT, 1)?;
    set_ipv6_ifindex(fd, IPV6_MULTICAST_IF, ifindex)?;
    join_carp6_multicast(fd, ifindex)?;
    bind_to_device(fd, interface)
}

fn set_carp_ttl(fd: RawFd) -> io::Result<()> {
    let ttl: c_int = CARP_TTL;
    for option_name in [IP_TTL, IP_MULTICAST_TTL] {
        let rc = unsafe {
            setsockopt(
                fd,
                SOL_IP,
                option_name,
                (&ttl as *const c_int).cast(),
                std::mem::size_of::<c_int>() as u32,
            )
        };
        if rc < 0 {
            let err = io::Error::last_os_error();
            return Err(err);
        }
    }
    Ok(())
}

fn disable_multicast_loop(fd: RawFd) -> io::Result<()> {
    let off: c_int = 0;
    let rc = unsafe {
        setsockopt(
            fd,
            SOL_IP,
            IP_MULTICAST_LOOP,
            (&off as *const c_int).cast(),
            std::mem::size_of::<c_int>() as u32,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn set_carp6_hops(fd: RawFd) -> io::Result<()> {
    set_ipv6_int(fd, IPV6_UNICAST_HOPS, CARP_TTL)?;
    set_ipv6_int(fd, IPV6_MULTICAST_HOPS, CARP_TTL)
}

fn set_ipv6_int(fd: RawFd, option_name: c_int, value: c_int) -> io::Result<()> {
    let rc = unsafe {
        setsockopt(
            fd,
            SOL_IPV6,
            option_name,
            (&value as *const c_int).cast(),
            std::mem::size_of::<c_int>() as u32,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn set_ipv6_ifindex(fd: RawFd, option_name: c_int, ifindex: u32) -> io::Result<()> {
    let ifindex = ifindex as c_uint;
    let rc = unsafe {
        setsockopt(
            fd,
            SOL_IPV6,
            option_name,
            (&ifindex as *const c_uint).cast(),
            std::mem::size_of::<c_uint>() as u32,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn join_carp6_multicast(fd: RawFd, ifindex: u32) -> io::Result<()> {
    let mreq = Ipv6Mreq {
        ipv6mr_multiaddr: In6Addr {
            s6_addr: CARP_IPV6_MULTICAST.octets(),
        },
        ipv6mr_interface: ifindex as c_uint,
    };
    let rc = unsafe {
        setsockopt(
            fd,
            SOL_IPV6,
            IPV6_JOIN_GROUP,
            (&mreq as *const Ipv6Mreq).cast(),
            std::mem::size_of::<Ipv6Mreq>() as u32,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn bind_to_device(fd: RawFd, interface: &str) -> io::Result<()> {
    if interface.as_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "interface name contains NUL byte",
        ));
    }
    let mut name = Vec::with_capacity(interface.len() + 1);
    name.extend_from_slice(interface.as_bytes());
    name.push(0);
    let rc = unsafe {
        setsockopt(
            fd,
            SOL_SOCKET,
            SO_BINDTODEVICE,
            name.as_ptr().cast(),
            name.len() as u32,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn interface_index(interface: &str) -> io::Result<u32> {
    let name = CString::new(interface).map_err(|_| {
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

fn duration_to_poll_timeout(timeout: Duration) -> c_int {
    if timeout.is_zero() {
        return 0;
    }
    let millis = timeout.as_millis();
    if millis > c_int::MAX as u128 {
        c_int::MAX
    } else {
        millis.max(1) as c_int
    }
}

fn decode_ipv4_carp_packet(bytes: &[u8]) -> io::Result<ReceivedAdvertisement> {
    if bytes.len() < 20 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "short IPv4 packet",
        ));
    }
    let version = bytes[0] >> 4;
    let ihl = (bytes[0] & 0x0f) as usize * 4;
    if version != 4 || ihl < 20 || bytes.len() < ihl {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid IPv4 header",
        ));
    }
    if bytes[9] != IPPROTO_CARP as u8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "non-CARP IPv4 packet",
        ));
    }
    let total_len = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    let packet_len = if total_len >= ihl && total_len <= bytes.len() {
        total_len
    } else {
        bytes.len()
    };
    let source = Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]);
    let destination = Ipv4Addr::new(bytes[16], bytes[17], bytes[18], bytes[19]);
    Ok(ReceivedAdvertisement {
        source,
        destination,
        ttl: bytes[8],
        payload: bytes[ihl..packet_len].to_vec(),
    })
}

fn parse_ipv6_control(control: &[u8]) -> io::Result<(Ipv6Addr, u8)> {
    let mut offset = 0usize;
    let mut destination = None;
    let mut hop_limit = None;
    while offset + std::mem::size_of::<Cmsghdr>() <= control.len() {
        let header =
            unsafe { std::ptr::read_unaligned(control[offset..].as_ptr().cast::<Cmsghdr>()) };
        if header.cmsg_len < std::mem::size_of::<Cmsghdr>()
            || offset + header.cmsg_len > control.len()
        {
            break;
        }
        let data_offset = offset + cmsg_align(std::mem::size_of::<Cmsghdr>());
        let data_len = header
            .cmsg_len
            .saturating_sub(cmsg_align(std::mem::size_of::<Cmsghdr>()));
        if header.cmsg_level == SOL_IPV6
            && header.cmsg_type == IPV6_PKTINFO
            && data_len >= std::mem::size_of::<In6Pktinfo>()
        {
            let info = unsafe {
                std::ptr::read_unaligned(control[data_offset..].as_ptr().cast::<In6Pktinfo>())
            };
            destination = Some(Ipv6Addr::from(info.ipi6_addr.s6_addr));
        }
        if header.cmsg_level == SOL_IPV6
            && header.cmsg_type == IPV6_HOPLIMIT
            && data_len >= std::mem::size_of::<c_int>()
        {
            let value = unsafe {
                std::ptr::read_unaligned(control[data_offset..].as_ptr().cast::<c_int>())
            };
            if (0..=255).contains(&value) {
                hop_limit = Some(value as u8);
            }
        }
        offset += cmsg_align(header.cmsg_len);
    }
    match (destination, hop_limit) {
        (Some(destination), Some(hop_limit)) => Ok((destination, hop_limit)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing IPv6 packet info",
        )),
    }
}

fn cmsg_align(len: usize) -> usize {
    let align = std::mem::size_of::<usize>();
    (len + align - 1) & !(align - 1)
}

impl AsRawFd for RawCarpSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl Drop for RawCarpSocket {
    fn drop(&mut self) {
        unsafe {
            close(self.fd);
        }
    }
}

impl Drop for RawCarpSocket6 {
    fn drop(&mut self) {
        unsafe {
            close(self.fd);
        }
    }
}

impl AdvertisementTransport for RawCarpSocket {
    fn send_advertisement(&self, destination: Ipv4Addr, payload: &[u8]) -> io::Result<usize> {
        let addr = SockaddrIn {
            sin_family: AF_INET as u16,
            sin_port: 0,
            sin_addr: InAddr {
                s_addr: u32::from_ne_bytes(destination.octets()),
            },
            sin_zero: [0; 8],
        };
        let sent = unsafe {
            sendto(
                self.fd,
                payload.as_ptr().cast(),
                payload.len(),
                0,
                (&addr as *const SockaddrIn).cast::<c_void>(),
                std::mem::size_of::<SockaddrIn>() as u32,
            )
        };
        if sent < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(sent as usize)
        }
    }
}

impl AdvertisementTransport6 for RawCarpSocket6 {
    fn send_advertisement6(&self, destination: Ipv6Addr, payload: &[u8]) -> io::Result<usize> {
        let addr = SockaddrIn6 {
            sin6_family: AF_INET6 as u16,
            sin6_port: 0,
            sin6_flowinfo: 0,
            sin6_addr: In6Addr {
                s6_addr: destination.octets(),
            },
            sin6_scope_id: self.ifindex,
        };
        let sent = unsafe {
            sendto(
                self.fd,
                payload.as_ptr().cast(),
                payload.len(),
                0,
                (&addr as *const SockaddrIn6).cast::<c_void>(),
                std::mem::size_of::<SockaddrIn6>() as u32,
            )
        };
        if sent < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(sent as usize)
        }
    }
}

pub fn virtual_mac(vhid: u8) -> [u8; 6] {
    [0x00, 0x00, 0x5e, 0x00, 0x01, vhid]
}

pub fn default_peer(peer: Option<Ipv4Addr>) -> Ipv4Addr {
    peer.unwrap_or(CARP_IPV4_MULTICAST)
}

pub fn default_peer6(peer: Option<Ipv6Addr>) -> Ipv6Addr {
    peer.unwrap_or(CARP_IPV6_MULTICAST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_peer_is_openbsd_ipv4_multicast_group() {
        assert_eq!(default_peer(None), Ipv4Addr::new(224, 0, 0, 18));
        assert_eq!(IPPROTO_CARP, 112);
        assert_eq!(CARP_TTL, 255);
        assert_eq!(IP_MULTICAST_TTL, 33);
        assert_eq!(IP_MULTICAST_LOOP, 34);
        assert_eq!(
            default_peer6(None),
            Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x12)
        );
    }

    #[test]
    fn virtual_mac_matches_carp_vhid_pattern() {
        assert_eq!(virtual_mac(1), [0x00, 0x00, 0x5e, 0x00, 0x01, 0x01]);
        assert_eq!(virtual_mac(255), [0x00, 0x00, 0x5e, 0x00, 0x01, 0xff]);
    }

    #[test]
    fn decodes_raw_socket_ipv4_packet() {
        let mut packet = vec![0u8; 20 + 36];
        packet[0] = 0x45;
        let packet_len = packet.len() as u16;
        packet[2..4].copy_from_slice(&packet_len.to_be_bytes());
        packet[8] = 255;
        packet[9] = IPPROTO_CARP as u8;
        packet[12..16].copy_from_slice(&[10, 0, 0, 1]);
        packet[16..20].copy_from_slice(&[224, 0, 0, 18]);
        packet[20] = 0x21;

        let decoded = decode_ipv4_carp_packet(&packet).unwrap();
        assert_eq!(decoded.source, Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(decoded.destination, CARP_IPV4_MULTICAST);
        assert_eq!(decoded.ttl, 255);
        assert_eq!(decoded.payload.len(), 36);
        assert_eq!(decoded.payload[0], 0x21);
    }
}
