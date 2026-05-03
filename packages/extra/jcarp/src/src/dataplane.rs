//! Linux ingress dataplane hooks for CARP load sharing.
//!
//! OpenBSD drops load-shared VIP traffic in the kernel with `carp_lsdrop()`.
//! On jonerix we install equivalent nft/stormwall netdev ingress rules and
//! refresh them whenever the local MASTER bitmask changes.

use std::ffi::OsStr;
use std::io;
use std::io::Write;
use std::process::{Command, Stdio};

use crate::config::{BalancingMode, Config, LoadFilterMode};
use crate::load_balance;
use crate::state::CarpState;

#[derive(Debug)]
pub struct LoadFilter {
    backend: LoadFilterBackend,
    installed: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum LoadFilterBackend {
    Disabled,
    Nft,
}

impl LoadFilter {
    pub fn new(cfg: &Config) -> Self {
        let backend = match (cfg.balancing, cfg.load_filter) {
            (BalancingMode::None, _) | (_, LoadFilterMode::Off) => LoadFilterBackend::Disabled,
            (_, LoadFilterMode::Auto | LoadFilterMode::Nft) => LoadFilterBackend::Nft,
        };
        Self {
            backend,
            installed: false,
        }
    }

    pub fn sync(&mut self, cfg: &Config, states: &[CarpState]) -> io::Result<()> {
        if self.backend == LoadFilterBackend::Disabled {
            return Ok(());
        }
        let rules = NftRules::new(cfg, states);
        run_nft_script(rules.replace_script())?;
        self.installed = true;
        Ok(())
    }

    pub fn cleanup(&mut self, cfg: &Config) {
        if self.backend == LoadFilterBackend::Disabled || !self.installed {
            return;
        }
        let rules = NftRules::new(cfg, &[]);
        let _ = run_nft_script(rules.destroy_script());
        self.installed = false;
    }
}

#[derive(Debug)]
struct NftRules<'a> {
    cfg: &'a Config,
    states: &'a [CarpState],
    table: String,
}

impl<'a> NftRules<'a> {
    fn new(cfg: &'a Config, states: &'a [CarpState]) -> Self {
        Self {
            cfg,
            states,
            table: table_name(cfg.effective_interface()),
        }
    }

    fn replace_script(&self) -> String {
        let mut script = String::new();
        script.push_str(&self.destroy_script());
        script.push_str("add table netdev ");
        script.push_str(&self.table);
        script.push('\n');
        script.push_str("add chain netdev ");
        script.push_str(&self.table);
        script.push_str(" ingress { type filter hook ingress device ");
        script.push_str(&quote_nft_string(self.cfg.effective_interface()));
        script.push_str(" priority -300; policy accept; }\n");
        self.push_drop_rules(&mut script);
        script
    }

    fn destroy_script(&self) -> String {
        let mut script = String::new();
        script.push_str("destroy table netdev ");
        script.push_str(&self.table);
        script.push('\n');
        script
    }

    fn push_drop_rules(&self, script: &mut String) {
        let node_count = self.cfg.nodes.len().min(32);
        if node_count == 0 {
            return;
        }
        let mask = load_balance::master_mask(self.states);
        for vip in &self.cfg.vips {
            if mask == 0 {
                self.push_rule(script, "ip", "daddr", &vip.to_string(), "drop");
                continue;
            }
            for slot in 0..node_count {
                if (mask & (1 << slot)) == 0 {
                    let verdict =
                        format!("{} mod {} == {} drop", ipv4_fold_expr(), node_count, slot);
                    self.push_rule(script, "ip", "daddr", &vip.to_string(), &verdict);
                }
            }
        }
        for vip in &self.cfg.vip6s {
            if mask == 0 {
                self.push_rule(script, "ip6", "daddr", &vip.to_string(), "drop");
                continue;
            }
            for slot in 0..node_count {
                if (mask & (1 << slot)) == 0 {
                    let verdict =
                        format!("{} mod {} == {} drop", ipv6_fold_expr(), node_count, slot);
                    self.push_rule(script, "ip6", "daddr", &vip.to_string(), &verdict);
                }
            }
        }
    }

    fn push_rule(
        &self,
        script: &mut String,
        family: &str,
        address_field: &str,
        address: &str,
        verdict: &str,
    ) {
        script.push_str("add rule netdev ");
        script.push_str(&self.table);
        script.push_str(" ingress ");
        script.push_str(family);
        script.push(' ');
        script.push_str(address_field);
        script.push(' ');
        script.push_str(address);
        script.push(' ');
        script.push_str(verdict);
        script.push('\n');
    }
}

fn run_nft_script(script: String) -> io::Result<()> {
    let mut child = Command::new(OsStr::new("nft"))
        .arg("-f")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            io::Error::new(
                err.kind(),
                format!("failed to start nft/stormwall for CARP load filter: {err}"),
            )
        })?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(script.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(io::Error::new(
        io::ErrorKind::Other,
        format!(
            "nft/stormwall CARP load filter update failed: {}",
            stderr.trim()
        ),
    ))
}

fn table_name(interface: &str) -> String {
    let mut name = String::from("jcarp_");
    for ch in interface.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            name.push(ch);
        } else {
            name.push('_');
        }
    }
    name
}

fn quote_nft_string(value: &str) -> String {
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

fn ipv4_fold_expr() -> &'static str {
    "( @nh,12,32 xor @nh,16,32 )"
}

fn ipv6_fold_expr() -> &'static str {
    "( @nh,8,32 xor @nh,12,32 xor @nh,16,32 xor @nh,20,32 xor @nh,24,32 xor @nh,28,32 xor @nh,32,32 xor @nh,36,32 )"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CarpNodeConfig, LinkMode, MacMode, Vip6Addr, VipAddr};
    use crate::proto::passphrase_to_key;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn nft_script_drops_slots_not_mastered_locally() {
        let cfg = test_config();
        let rules = NftRules::new(&cfg, &[CarpState::Master, CarpState::Backup]);
        let script = rules.replace_script();

        assert!(script.contains("destroy table netdev jcarp_eth0\n"));
        assert!(script.contains(
            "add chain netdev jcarp_eth0 ingress { type filter hook ingress device \"eth0\" priority -300; policy accept; }"
        ));
        assert!(script.contains(
            "add rule netdev jcarp_eth0 ingress ip daddr 10.0.253.42 ( @nh,12,32 xor @nh,16,32 ) mod 2 == 1 drop"
        ));
        assert!(script.contains(
            "add rule netdev jcarp_eth0 ingress ip6 daddr 2001:db8::42 ( @nh,8,32 xor @nh,12,32 xor @nh,16,32 xor @nh,20,32 xor @nh,24,32 xor @nh,28,32 xor @nh,32,32 xor @nh,36,32 ) mod 2 == 1 drop"
        ));
    }

    #[test]
    fn nft_script_drops_all_vip_traffic_when_no_local_master_nodes() {
        let cfg = test_config();
        let rules = NftRules::new(&cfg, &[CarpState::Backup, CarpState::Backup]);
        let script = rules.replace_script();

        assert!(script.contains("add rule netdev jcarp_eth0 ingress ip daddr 10.0.253.42 drop"));
        assert!(script.contains("add rule netdev jcarp_eth0 ingress ip6 daddr 2001:db8::42 drop"));
    }

    #[test]
    fn table_name_is_identifier_safe() {
        assert_eq!(table_name("lan.100"), "jcarp_lan_100");
        assert_eq!(quote_nft_string("lan\"x"), "\"lan\\\"x\"");
    }

    #[test]
    fn nft_script_targets_effective_child_interface() {
        let mut cfg = test_config();
        cfg.link_mode = LinkMode::MacvlanBridge;
        cfg.link_name = Some("carp42".to_string());
        let rules = NftRules::new(&cfg, &[CarpState::Master, CarpState::Backup]);
        let script = rules.replace_script();

        assert!(script.contains("destroy table netdev jcarp_carp42\n"));
        assert!(script.contains("hook ingress device \"carp42\""));
    }

    fn test_config() -> Config {
        Config {
            interface: "eth0".to_string(),
            link_mode: LinkMode::Parent,
            link_name: None,
            link_parent: None,
            vhid: 42,
            advbase: 1,
            advskew: 50,
            demote: 0,
            preempt: true,
            peer: None,
            peer6: None,
            vips: vec![Ipv4Addr::new(10, 0, 253, 42)],
            vip_addrs: vec![VipAddr {
                addr: Ipv4Addr::new(10, 0, 253, 42),
                prefix_len: 32,
            }],
            vip6s: vec![Ipv6Addr::from([
                0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x42,
            ])],
            vip6_addrs: vec![Vip6Addr {
                addr: Ipv6Addr::from([
                    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x42,
                ]),
                prefix_len: 128,
            }],
            key: passphrase_to_key("interop-pass"),
            manage_vip: true,
            announce: true,
            mac_mode: MacMode::Interface,
            balancing: BalancingMode::Ip,
            load_filter: LoadFilterMode::Nft,
            nodes: vec![
                CarpNodeConfig {
                    vhid: 42,
                    advskew: 50,
                },
                CarpNodeConfig {
                    vhid: 43,
                    advskew: 0,
                },
            ],
        }
    }
}
