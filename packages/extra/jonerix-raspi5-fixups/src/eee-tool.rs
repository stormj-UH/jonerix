** WARNING: connection is not using a post-quantum key exchange algorithm.
** This session may be vulnerable to "store now, decrypt later" attacks.
** The server may need to be upgraded. See https://openssh.com/pq.html
// Query or toggle IEEE 802.3az (EEE) on a network interface via SIOCETHTOOL.
// Build: rustc -O eee-tool.rs -o eee-tool
// Run:   sudo ./eee-tool [on|off] [ifname]   (default: show status on eth0)

use std::env;
use std::ffi::CString;
use std::mem::zeroed;
use std::os::raw::{c_char, c_int, c_short, c_ulong, c_void};
use std::process::ExitCode;

const AF_INET: c_int = 2;
const SOCK_DGRAM: c_int = 2;
const SIOCETHTOOL: c_ulong = 0x8946;
const ETHTOOL_GEEE: u32 = 0x0000_0044;
const ETHTOOL_SEEE: u32 = 0x0000_0045;
const ETHTOOL_NWAY_RST: u32 = 0x0000_0009;
const SIOCGIFFLAGS: c_ulong = 0x8913;
const SIOCSIFFLAGS: c_ulong = 0x8914;
const IFF_UP: c_short = 0x1;
const IFNAMSIZ: usize = 16;

#[repr(C)]
#[derive(Copy, Clone)]
struct EthtoolEee {
    cmd: u32,
    supported: u32,
    advertised: u32,
    lp_advertised: u32,
    eee_active: u32,
    eee_enabled: u32,
    tx_lpi_enabled: u32,
    tx_lpi_timer: u32,
    reserved: [u32; 2],
}

#[repr(C)]
#[derive(Copy, Clone)]
struct EthtoolValue {
    cmd: u32,
    data: u32,
}

#[repr(C)]
struct Ifreq {
    ifr_name: [c_char; IFNAMSIZ],
    ifr_data: *mut c_void,
    _pad: [u8; 16],
}

#[repr(C)]
struct IfreqFlags {
    ifr_name: [c_char; IFNAMSIZ],
    ifr_flags: c_short,
    _pad: [u8; 22],
}

extern "C" {
    fn socket(domain: c_int, ty: c_int, proto: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn ioctl(fd: c_int, req: c_ulong, arg: *mut c_void) -> c_int;
    fn __errno_location() -> *mut c_int;
    fn strerror(errnum: c_int) -> *const c_char;
}

enum Action {
    Status,
    On,
    Off,
}

fn err() -> String {
    unsafe {
        let e = *__errno_location();
        let p = strerror(e);
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: eee-tool [on|off] [-b|--down-up] [ifname]");
    ExitCode::from(2)
}

fn parse_args() -> Result<(Action, String, bool), ExitCode> {
    let mut action = Action::Status;
    let mut iface: Option<String> = None;
    let mut bounce = false;
    for a in env::args().skip(1) {
        match a.as_str() {
            "on" => action = Action::On,
            "off" => action = Action::Off,
            "-b" | "--down-up" => bounce = true,
            "-h" | "--help" => return Err(usage()),
            s if iface.is_none() => iface = Some(s.to_string()),
            _ => return Err(usage()),
        }
    }
    Ok((action, iface.unwrap_or_else(|| "eth0".into()), bounce))
}

fn set_link(fd: c_int, name: &[c_char; IFNAMSIZ], up: bool) -> Result<(), String> {
    let mut fr: IfreqFlags = unsafe { zeroed() };
    fr.ifr_name = *name;
    if unsafe { ioctl(fd, SIOCGIFFLAGS, &mut fr as *mut _ as *mut c_void) } < 0 {
        return Err(format!("SIOCGIFFLAGS: {}", err()));
    }
    if up {
        fr.ifr_flags |= IFF_UP;
    } else {
        fr.ifr_flags &= !IFF_UP;
    }
    if unsafe { ioctl(fd, SIOCSIFFLAGS, &mut fr as *mut _ as *mut c_void) } < 0 {
        return Err(format!("SIOCSIFFLAGS: {}", err()));
    }
    Ok(())
}

fn main() -> ExitCode {
    let (action, iface, bounce) = match parse_args() {
        Ok(v) => v,
        Err(c) => return c,
    };
    if iface.len() >= IFNAMSIZ {
        eprintln!("interface name too long");
        return ExitCode::from(2);
    }

    let fd = unsafe { socket(AF_INET, SOCK_DGRAM, 0) };
    if fd < 0 {
        eprintln!("socket: {}", err());
        return ExitCode::from(1);
    }

    let mut ifr: Ifreq = unsafe { zeroed() };
    let cname = CString::new(iface.clone()).unwrap();
    for (i, &b) in cname.as_bytes().iter().enumerate() {
        ifr.ifr_name[i] = b as c_char;
    }

    let mut eee: EthtoolEee = unsafe { zeroed() };
    eee.cmd = ETHTOOL_GEEE;
    ifr.ifr_data = &mut eee as *mut _ as *mut c_void;

    if unsafe { ioctl(fd, SIOCETHTOOL, &mut ifr as *mut _ as *mut c_void) } < 0 {
        eprintln!("{}: ETHTOOL_GEEE: {}", iface, err());
        unsafe { close(fd) };
        return ExitCode::from(1);
    }

    let print_state = |label: &str, e: &EthtoolEee| {
        println!(
            "{}: {} eee_enabled={} tx_lpi_enabled={} eee_active={} \
             supported=0x{:x} advertised=0x{:x} lp_advertised=0x{:x} tx_lpi_timer={}",
            iface,
            label,
            e.eee_enabled,
            e.tx_lpi_enabled,
            e.eee_active,
            e.supported,
            e.advertised,
            e.lp_advertised,
            e.tx_lpi_timer,
        );
    };

    let before = eee;

    match action {
        Action::Status => {
            print_state("status", &eee);
            unsafe { close(fd) };
            return ExitCode::SUCCESS;
        }
        Action::On => {
            if eee.eee_enabled == 1 && eee.advertised == eee.supported {
                println!("{}: already on", iface);
                unsafe { close(fd) };
                return ExitCode::SUCCESS;
            }
            eee.cmd = ETHTOOL_SEEE;
            eee.eee_enabled = 1;
            eee.tx_lpi_enabled = 1;
            eee.advertised = eee.supported;
        }
        Action::Off => {
            if eee.eee_enabled == 0 && eee.tx_lpi_enabled == 0 && eee.advertised == 0 {
                println!("{}: already off", iface);
                unsafe { close(fd) };
                return ExitCode::SUCCESS;
            }
            eee.cmd = ETHTOOL_SEEE;
            eee.eee_enabled = 0;
            eee.tx_lpi_enabled = 0;
            eee.advertised = 0;
        }
    }

    print_state("before", &before);

    if unsafe { ioctl(fd, SIOCETHTOOL, &mut ifr as *mut _ as *mut c_void) } < 0 {
        eprintln!("{}: ETHTOOL_SEEE: {}", iface, err());
        unsafe { close(fd) };
        return ExitCode::from(1);
    }

    if bounce {
        println!("{}: bringing link down/up", iface);
        if let Err(e) = set_link(fd, &ifr.ifr_name, false) {
            eprintln!("{}: {} (continuing)", iface, e);
        }
        if let Err(e) = set_link(fd, &ifr.ifr_name, true) {
            eprintln!("{}: {} (continuing)", iface, e);
        }
    } else {
        let mut nway = EthtoolValue { cmd: ETHTOOL_NWAY_RST, data: 0 };
        ifr.ifr_data = &mut nway as *mut _ as *mut c_void;
        if unsafe { ioctl(fd, SIOCETHTOOL, &mut ifr as *mut _ as *mut c_void) } < 0 {
            eprintln!("{}: ETHTOOL_NWAY_RST: {} (continuing)", iface, err());
        } else {
            println!("{}: renegotiating autoneg (link will flap)", iface);
        }
        ifr.ifr_data = &mut eee as *mut _ as *mut c_void;
    }

    eee.cmd = ETHTOOL_GEEE;
    if unsafe { ioctl(fd, SIOCETHTOOL, &mut ifr as *mut _ as *mut c_void) } < 0 {
        eprintln!("{}: ETHTOOL_GEEE (verify): {}", iface, err());
        unsafe { close(fd) };
        return ExitCode::from(1);
    }
    print_state("after ", &eee);

    unsafe { close(fd) };
    ExitCode::SUCCESS
}
