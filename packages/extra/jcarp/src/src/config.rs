use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use crate::proto::{CARP_DFLTINTV, CARP_KEY_LEN};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MacMode {
    Virtual,
    Interface,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LinkMode {
    Parent,
    MacvlanBridge,
    MacvlanPrivate,
    IpvlanL2,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct VipAddr {
    pub addr: Ipv4Addr,
    pub prefix_len: u8,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct Vip6Addr {
    pub addr: Ipv6Addr,
    pub prefix_len: u8,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BalancingMode {
    None,
    Ip,
    IpStealth,
    IpUnicast,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LoadFilterMode {
    Auto,
    Nft,
    Off,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CarpNodeConfig {
    pub vhid: u8,
    pub advskew: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub interface: String,
    pub link_mode: LinkMode,
    pub link_name: Option<String>,
    pub link_parent: Option<String>,
    pub vhid: u8,
    pub advbase: u8,
    pub advskew: u8,
    pub demote: u8,
    pub preempt: bool,
    pub peer: Option<Ipv4Addr>,
    pub peer6: Option<Ipv6Addr>,
    pub vips: Vec<Ipv4Addr>,
    pub vip_addrs: Vec<VipAddr>,
    pub vip6s: Vec<Ipv6Addr>,
    pub vip6_addrs: Vec<Vip6Addr>,
    pub key: [u8; CARP_KEY_LEN],
    pub manage_vip: bool,
    pub announce: bool,
    pub mac_mode: MacMode,
    pub balancing: BalancingMode,
    pub load_filter: LoadFilterMode,
    pub nodes: Vec<CarpNodeConfig>,
}

impl Config {
    pub fn load(path: &Path) -> io::Result<Self> {
        let raw = fs::read_to_string(path)?;
        parse_config(&raw)
    }

    pub fn effective_interface(&self) -> &str {
        if self.link_mode == LinkMode::Parent {
            &self.interface
        } else {
            self.link_name.as_deref().unwrap_or(&self.interface)
        }
    }

    pub fn parent_interface(&self) -> &str {
        if self.link_mode == LinkMode::Parent {
            &self.interface
        } else {
            self.link_parent.as_deref().unwrap_or(&self.interface)
        }
    }

    pub fn link_mode_uses_child(&self) -> bool {
        self.link_mode != LinkMode::Parent
    }
}

pub fn parse_config(raw: &str) -> io::Result<Config> {
    let mut interface = None;
    let mut link_mode = LinkMode::Parent;
    let mut link_name = None;
    let mut link_parent = None;
    let mut vhid = None;
    let mut advbase = CARP_DFLTINTV;
    let mut advskew = 0u8;
    let mut demote = 0u8;
    let mut preempt = false;
    let mut peer = None;
    let mut peer6 = None;
    let mut vips = Vec::new();
    let mut vip_addrs = Vec::new();
    let mut vip6s = Vec::new();
    let mut vip6_addrs = Vec::new();
    let mut passphrase = None;
    let mut manage_vip = true;
    let mut announce = true;
    let mut mac_mode = MacMode::Virtual;
    let mut balancing = BalancingMode::None;
    let mut load_filter = LoadFilterMode::Auto;
    let mut nodes = Vec::new();

    for (idx, line) in raw.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return invalid(idx, "expected key=value");
        };
        let key = key.trim();
        let value = value.trim();
        match key {
            "interface" => interface = Some(value.to_string()),
            "link_mode" => link_mode = parse_link_mode(idx, value)?,
            "link_name" => link_name = Some(value.to_string()),
            "link_parent" => link_parent = Some(value.to_string()),
            "vhid" => vhid = Some(parse_u8(idx, value)?),
            "advbase" => advbase = parse_u8(idx, value)?,
            "advskew" => advskew = parse_u8(idx, value)?,
            "demote" => demote = parse_u8(idx, value)?,
            "preempt" => preempt = parse_bool(idx, value)?,
            "peer" => peer = Some(parse_ipv4(idx, value)?),
            "peer6" => peer6 = Some(parse_ipv6(idx, value)?),
            "vip" => {
                add_vip(
                    idx,
                    value,
                    &mut vips,
                    &mut vip_addrs,
                    &mut vip6s,
                    &mut vip6_addrs,
                )?;
            }
            "vip4" => {
                let vip = parse_vip4(idx, value)?;
                vips.push(vip.addr);
                vip_addrs.push(vip);
            }
            "vip6" => {
                let vip = parse_vip6(idx, value)?;
                vip6s.push(vip.addr);
                vip6_addrs.push(vip);
            }
            "passphrase" => passphrase = Some(value.to_string()),
            "manage_vip" => manage_vip = parse_bool(idx, value)?,
            "announce" => announce = parse_bool(idx, value)?,
            "mac" | "mac_mode" => mac_mode = parse_mac_mode(idx, value)?,
            "balancing" => balancing = parse_balancing_mode(idx, value)?,
            "load_filter" | "dataplane" => load_filter = parse_load_filter_mode(idx, value)?,
            "carpnode" | "node" => nodes.push(parse_carpnode(idx, value)?),
            _ => return invalid(idx, "unknown key"),
        }
    }

    let interface = interface
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing interface"))?;
    if nodes.is_empty() {
        let vhid =
            vhid.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing vhid"))?;
        nodes.push(CarpNodeConfig { vhid, advskew });
    }
    validate_carpnodes(&nodes)?;
    let primary = nodes[0];
    let vhid = primary.vhid;
    let advskew = primary.advskew;
    if link_mode != LinkMode::Parent && link_name.is_none() {
        link_name = Some(format!("jcarp{vhid}"));
    }
    if vips.is_empty() && vip6s.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one vip is required",
        ));
    }
    let passphrase = passphrase
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing passphrase"))?;

    Ok(Config {
        interface,
        link_mode,
        link_name,
        link_parent,
        vhid,
        advbase,
        advskew,
        demote,
        preempt,
        peer,
        peer6,
        vips,
        vip_addrs,
        vip6s,
        vip6_addrs,
        key: crate::proto::passphrase_to_key(&passphrase),
        manage_vip,
        announce,
        mac_mode,
        balancing,
        load_filter,
        nodes,
    })
}

fn parse_u8(line: usize, value: &str) -> io::Result<u8> {
    value.parse::<u8>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("line {}: expected integer 0..255", line + 1),
        )
    })
}

fn parse_bool(line: usize, value: &str) -> io::Result<bool> {
    match value {
        "true" | "yes" | "1" => Ok(true),
        "false" | "no" | "0" => Ok(false),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("line {}: expected true/false", line + 1),
        )),
    }
}

fn parse_ipv4(line: usize, value: &str) -> io::Result<Ipv4Addr> {
    value.parse::<Ipv4Addr>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("line {}: expected IPv4 address", line + 1),
        )
    })
}

fn add_vip(
    line: usize,
    value: &str,
    vips: &mut Vec<Ipv4Addr>,
    vip_addrs: &mut Vec<VipAddr>,
    vip6s: &mut Vec<Ipv6Addr>,
    vip6_addrs: &mut Vec<Vip6Addr>,
) -> io::Result<()> {
    let (addr, prefix) = value.split_once('/').unwrap_or((value, ""));
    match addr.parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => {
            let vip = parse_vip4(line, value)?;
            vips.push(vip.addr);
            vip_addrs.push(vip);
            Ok(())
        }
        Ok(IpAddr::V6(_)) => {
            let vip = parse_vip6(line, value)?;
            vip6s.push(vip.addr);
            vip6_addrs.push(vip);
            Ok(())
        }
        Err(_) if !prefix.is_empty() => parse_ipv4(line, addr).map(|_| ()),
        Err(_) => invalid(line, "expected IPv4 or IPv6 address"),
    }
}

fn parse_vip4(line: usize, value: &str) -> io::Result<VipAddr> {
    let (addr, prefix_len) = match value.split_once('/') {
        Some((addr, prefix)) => (parse_ipv4(line, addr)?, parse_prefix_len(line, prefix, 32)?),
        None => (parse_ipv4(line, value)?, 32),
    };
    Ok(VipAddr { addr, prefix_len })
}

fn parse_vip6(line: usize, value: &str) -> io::Result<Vip6Addr> {
    let (addr, prefix_len) = match value.rsplit_once('/') {
        Some((addr, prefix)) => (
            parse_ipv6(line, addr)?,
            parse_prefix_len(line, prefix, 128)?,
        ),
        None => (parse_ipv6(line, value)?, 128),
    };
    Ok(Vip6Addr { addr, prefix_len })
}

fn parse_prefix_len(line: usize, value: &str, max: u8) -> io::Result<u8> {
    let prefix = parse_u8(line, value)?;
    if prefix > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("line {}: expected prefix length 0..{max}", line + 1),
        ));
    }
    Ok(prefix)
}

fn parse_ipv6(line: usize, value: &str) -> io::Result<Ipv6Addr> {
    value.parse::<Ipv6Addr>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("line {}: expected IPv6 address", line + 1),
        )
    })
}

fn parse_mac_mode(line: usize, value: &str) -> io::Result<MacMode> {
    match value {
        "virtual" => Ok(MacMode::Virtual),
        "interface" | "real" => Ok(MacMode::Interface),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("line {}: expected mac=virtual or mac=interface", line + 1),
        )),
    }
}

fn parse_link_mode(line: usize, value: &str) -> io::Result<LinkMode> {
    match value {
        "parent" => Ok(LinkMode::Parent),
        "macvlan-bridge" => Ok(LinkMode::MacvlanBridge),
        "macvlan-private" => Ok(LinkMode::MacvlanPrivate),
        "ipvlan-l2" => Ok(LinkMode::IpvlanL2),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "line {}: expected link_mode=parent|macvlan-bridge|macvlan-private|ipvlan-l2",
                line + 1
            ),
        )),
    }
}

fn parse_balancing_mode(line: usize, value: &str) -> io::Result<BalancingMode> {
    match value {
        "none" | "off" => Ok(BalancingMode::None),
        "ip" => Ok(BalancingMode::Ip),
        "ip-stealth" => Ok(BalancingMode::IpStealth),
        "ip-unicast" => Ok(BalancingMode::IpUnicast),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "line {}: expected balancing=none|ip|ip-stealth|ip-unicast",
                line + 1
            ),
        )),
    }
}

fn parse_load_filter_mode(line: usize, value: &str) -> io::Result<LoadFilterMode> {
    match value {
        "auto" => Ok(LoadFilterMode::Auto),
        "nft" | "stormwall" => Ok(LoadFilterMode::Nft),
        "off" | "none" => Ok(LoadFilterMode::Off),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("line {}: expected load_filter=auto|nft|off", line + 1),
        )),
    }
}

fn parse_carpnode(line: usize, value: &str) -> io::Result<CarpNodeConfig> {
    let Some((vhid, advskew)) = value.split_once(':') else {
        return invalid(line, "expected carpnode=vhid:advskew");
    };
    Ok(CarpNodeConfig {
        vhid: parse_u8(line, vhid)?,
        advskew: parse_u8(line, advskew)?,
    })
}

fn validate_carpnodes(nodes: &[CarpNodeConfig]) -> io::Result<()> {
    if nodes.len() > 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CARP supports at most 32 carpnodes",
        ));
    }
    let mut seen = [false; 256];
    for node in nodes {
        if node.vhid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "vhid must be 1..255",
            ));
        }
        if seen[node.vhid as usize] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate carpnodes vhid",
            ));
        }
        seen[node.vhid as usize] = true;
    }
    Ok(())
}

fn invalid<T>(line: usize, msg: &str) -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("line {}: {msg}", line + 1),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_shape() {
        let cfg = parse_config(
            "interface=eth1\nvhid=7\nadvbase=1\nadvskew=100\ndemote=0\npreempt=true\npeer=224.0.0.18\nvip=192.168.1.1\npassphrase=test\n",
        )
        .unwrap();
        assert_eq!(cfg.interface, "eth1");
        assert_eq!(cfg.link_mode, LinkMode::Parent);
        assert_eq!(cfg.link_name, None);
        assert_eq!(cfg.link_parent, None);
        assert_eq!(cfg.effective_interface(), "eth1");
        assert_eq!(cfg.parent_interface(), "eth1");
        assert!(!cfg.link_mode_uses_child());
        assert_eq!(cfg.vhid, 7);
        assert_eq!(cfg.peer6, None);
        assert_eq!(cfg.vips, vec![Ipv4Addr::new(192, 168, 1, 1)]);
        assert_eq!(
            cfg.vip_addrs,
            vec![VipAddr {
                addr: Ipv4Addr::new(192, 168, 1, 1),
                prefix_len: 32,
            }]
        );
        assert!(cfg.vip6s.is_empty());
        assert!(cfg.preempt);
        assert!(cfg.manage_vip);
        assert!(cfg.announce);
        assert_eq!(cfg.mac_mode, MacMode::Virtual);
        assert_eq!(cfg.balancing, BalancingMode::None);
        assert_eq!(cfg.load_filter, LoadFilterMode::Auto);
        assert_eq!(
            cfg.nodes,
            vec![CarpNodeConfig {
                vhid: 7,
                advskew: 100
            }]
        );
    }

    #[test]
    fn parses_runtime_options() {
        let cfg = parse_config(
            "interface=eth1\nlink_mode=macvlan-private\nlink_name=carp42\nlink_parent=bond0\ncarpnode=42:100\ncarpnode=43:0\nbalancing=ip\nload_filter=nft\nvip=192.168.1.1/24\nvip=2001:db8::1/64\npeer6=ff02::12\npassphrase=test\nmanage_vip=false\nannounce=no\nmac=interface\n",
        )
        .unwrap();
        assert_eq!(cfg.link_mode, LinkMode::MacvlanPrivate);
        assert_eq!(cfg.link_name.as_deref(), Some("carp42"));
        assert_eq!(cfg.link_parent.as_deref(), Some("bond0"));
        assert_eq!(cfg.effective_interface(), "carp42");
        assert_eq!(cfg.parent_interface(), "bond0");
        assert!(cfg.link_mode_uses_child());
        assert_eq!(cfg.vhid, 42);
        assert_eq!(cfg.advskew, 100);
        assert_eq!(
            cfg.vip_addrs,
            vec![VipAddr {
                addr: Ipv4Addr::new(192, 168, 1, 1),
                prefix_len: 24,
            }]
        );
        assert_eq!(cfg.vip6s, vec!["2001:db8::1".parse::<Ipv6Addr>().unwrap()]);
        assert_eq!(
            cfg.vip6_addrs,
            vec![Vip6Addr {
                addr: "2001:db8::1".parse::<Ipv6Addr>().unwrap(),
                prefix_len: 64,
            }]
        );
        assert_eq!(cfg.peer6, Some("ff02::12".parse::<Ipv6Addr>().unwrap()));
        assert!(!cfg.manage_vip);
        assert!(!cfg.announce);
        assert_eq!(cfg.mac_mode, MacMode::Interface);
        assert_eq!(cfg.balancing, BalancingMode::Ip);
        assert_eq!(cfg.load_filter, LoadFilterMode::Nft);
        assert_eq!(
            cfg.nodes,
            vec![
                CarpNodeConfig {
                    vhid: 42,
                    advskew: 100,
                },
                CarpNodeConfig {
                    vhid: 43,
                    advskew: 0
                },
            ]
        );
    }

    #[test]
    fn child_link_mode_gets_stable_default_link_name() {
        let cfg = parse_config(
            "interface=eth1\nlink_mode=macvlan-bridge\nvhid=42\nvip=192.168.1.1\npassphrase=test\n",
        )
        .unwrap();

        assert_eq!(cfg.link_name.as_deref(), Some("jcarp42"));
        assert_eq!(cfg.effective_interface(), "jcarp42");
        assert_eq!(cfg.parent_interface(), "eth1");
    }
}
