//! Runtime CARP daemon loop.

use std::cmp;
use std::io;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::{CarpNodeConfig, Config};
use crate::dataplane::LoadFilter;
use crate::iface::InterfaceRuntime;
use crate::io::{
    default_peer, default_peer6, AdvertisementTransport, AdvertisementTransport6, RawCarpSocket,
    RawCarpSocket6, ReceivedAdvertisement, ReceivedAdvertisement6, CARP_TTL,
};
use crate::proto::{carp_hmac_sha1_mixed, verify_digest, CarpHeader, HmacMode};
use crate::state::{advertisement_interval, CarpState, Decision, LocalNode, RemoteAdvertisement};

const EHOSTDOWN: i32 = 112;

pub fn run_daemon<F>(cfg: Config, should_stop: F) -> io::Result<()>
where
    F: Fn() -> bool,
{
    let runtime = InterfaceRuntime::open(&cfg)?;
    let effective_interface = runtime.name().to_string();
    let sockets = (|| -> io::Result<(Option<RawCarpSocket>, Option<RawCarpSocket6>)> {
        let socket4 = if !cfg.vips.is_empty() || cfg.peer.is_some() {
            Some(RawCarpSocket::open_v4_for_interface(&effective_interface)?)
        } else {
            None
        };
        let socket6 = if !cfg.vip6s.is_empty() || cfg.peer6.is_some() {
            Some(RawCarpSocket6::open_for_interface(&effective_interface)?)
        } else {
            None
        };
        Ok((socket4, socket6))
    })();
    let (socket4, socket6) = match sockets {
        Ok(sockets) => sockets,
        Err(err) => {
            runtime.cleanup(&cfg);
            return Err(err);
        }
    };
    let mut load_filter = LoadFilter::new(&cfg);
    let mut daemon = CarpDaemon::new(cfg);
    let result = daemon.run(
        socket4.as_ref(),
        socket6.as_ref(),
        &runtime,
        &mut load_filter,
        should_stop,
    );
    load_filter.cleanup(&daemon.cfg);
    runtime.cleanup(&daemon.cfg);
    result
}

pub struct CarpDaemon {
    cfg: Config,
    nodes: Vec<NodeRuntime>,
    runtime_active: bool,
    suppressed: bool,
    suppress_demote_active: bool,
    send_error_demote_active: bool,
    sendad_errors: usize,
    sendad_success: usize,
    dynamic_demote: u8,
}

struct NodeRuntime {
    node: LocalNode,
    replay: ReplayWindow,
    last_sent_counter: Option<[u8; 8]>,
    next_advertisement: Option<Instant>,
    master_down: Option<Instant>,
}

impl CarpDaemon {
    pub fn new(cfg: Config) -> Self {
        let nodes = cfg
            .nodes
            .iter()
            .map(|node| NodeRuntime::new(*node, &cfg))
            .collect();
        Self {
            cfg,
            nodes,
            runtime_active: false,
            suppressed: false,
            suppress_demote_active: false,
            send_error_demote_active: false,
            sendad_errors: 0,
            sendad_success: 0,
            dynamic_demote: 0,
        }
    }

    pub fn run<F>(
        &mut self,
        socket4: Option<&RawCarpSocket>,
        socket6: Option<&RawCarpSocket6>,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
        should_stop: F,
    ) -> io::Result<()>
    where
        F: Fn() -> bool,
    {
        for node in &mut self.nodes {
            node.node.start();
            node.master_down = Some(Instant::now() + node.node.master_down_timeout());
        }
        runtime.enter_backup(&self.cfg)?;
        self.sync_load_filter(load_filter)?;

        while !should_stop() {
            self.process_due_timers(socket4, socket6, runtime, load_filter)?;
            let timeout = self.next_timeout();
            if let Some(socket) = socket4 {
                match socket.recv_advertisement_timeout(timeout.map(cap_poll_interval)) {
                    Ok(Some(packet)) => {
                        self.handle_packet(socket4, socket6, runtime, load_filter, packet)?
                    }
                    Ok(None) => {}
                    Err(err) if err.kind() == io::ErrorKind::Interrupted && should_stop() => break,
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(err) => return Err(err),
                }
            }
            if let Some(socket) = socket6 {
                let timeout = if socket4.is_some() {
                    Some(Duration::ZERO)
                } else {
                    timeout
                };
                match socket.recv_advertisement_timeout(timeout.map(cap_poll_interval)) {
                    Ok(Some(packet)) => {
                        self.handle_packet6(socket4, socket6, runtime, load_filter, packet)?
                    }
                    Ok(None) => {}
                    Err(err) if err.kind() == io::ErrorKind::Interrupted && should_stop() => break,
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                    Err(err) => return Err(err),
                }
            }
        }
        if should_stop() {
            self.send_bow_out_advertisements(socket4, socket6)?;
        }
        Ok(())
    }

    fn process_due_timers(
        &mut self,
        socket4: Option<&RawCarpSocket>,
        socket6: Option<&RawCarpSocket6>,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
    ) -> io::Result<()> {
        self.update_link_state(runtime, load_filter)?;
        if self.suppressed {
            return Ok(());
        }
        let now = Instant::now();
        let mut idx = 0;
        while idx < self.nodes.len() {
            if self.nodes[idx].node.state == CarpState::Backup
                && self.nodes[idx]
                    .master_down
                    .is_some_and(|deadline| deadline <= now)
            {
                self.become_master(idx, socket4, socket6, runtime, load_filter)?;
            }
            if self.nodes[idx].node.state == CarpState::Master
                && self.nodes[idx]
                    .next_advertisement
                    .is_some_and(|deadline| deadline <= Instant::now())
            {
                self.send_advertisements(idx, socket4, socket6)?;
                self.nodes[idx].next_advertisement =
                    Some(Instant::now() + self.nodes[idx].node.local_interval());
            }
            idx += 1;
        }
        Ok(())
    }

    fn next_timeout(&self) -> Option<Duration> {
        let now = Instant::now();
        let next = self
            .nodes
            .iter()
            .flat_map(|node| [node.master_down, node.next_advertisement])
            .flatten()
            .min();
        Some(match next {
            Some(deadline) if deadline > now => deadline - now,
            Some(_) => Duration::ZERO,
            None => Duration::from_secs(1),
        })
    }

    fn handle_packet(
        &mut self,
        socket4: Option<&RawCarpSocket>,
        socket6: Option<&RawCarpSocket6>,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
        packet: ReceivedAdvertisement,
    ) -> io::Result<()> {
        if self.suppressed {
            return Ok(());
        }
        let Some((idx, header)) = self.accept_packet(packet) else {
            return Ok(());
        };

        self.sync_node_demotes();
        let remote = RemoteAdvertisement {
            advbase: header.advbase,
            advskew: header.advskew,
            demote: header.demote,
        };
        let previous = self.nodes[idx].node.state;
        let decision = self.nodes[idx].node.observe_advertisement(remote);

        match decision {
            Decision::BecomeBackup => {
                if previous == CarpState::Master {
                    self.become_backup(idx, runtime, load_filter, Some(remote))?;
                }
            }
            Decision::BecomeMaster => {
                self.become_master(idx, socket4, socket6, runtime, load_filter)?;
            }
            Decision::ResetMasterDownTimer => {
                self.nodes[idx].master_down = Some(Instant::now() + master_down_timeout(remote));
            }
            Decision::StayMaster | Decision::Ignore => {}
        }
        Ok(())
    }

    fn handle_packet6(
        &mut self,
        socket4: Option<&RawCarpSocket>,
        socket6: Option<&RawCarpSocket6>,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
        packet: ReceivedAdvertisement6,
    ) -> io::Result<()> {
        if self.suppressed {
            return Ok(());
        }
        let Some((idx, header)) = self.accept_packet6(packet) else {
            return Ok(());
        };

        self.sync_node_demotes();
        let remote = RemoteAdvertisement {
            advbase: header.advbase,
            advskew: header.advskew,
            demote: header.demote,
        };
        let previous = self.nodes[idx].node.state;
        let decision = self.nodes[idx].node.observe_advertisement(remote);

        match decision {
            Decision::BecomeBackup => {
                if previous == CarpState::Master {
                    self.become_backup(idx, runtime, load_filter, Some(remote))?;
                }
            }
            Decision::BecomeMaster => {
                self.become_master(idx, socket4, socket6, runtime, load_filter)?;
            }
            Decision::ResetMasterDownTimer => {
                self.nodes[idx].master_down = Some(Instant::now() + master_down_timeout(remote));
            }
            Decision::StayMaster | Decision::Ignore => {}
        }
        Ok(())
    }

    fn accept_packet(&mut self, packet: ReceivedAdvertisement) -> Option<(usize, CarpHeader)> {
        if packet.ttl != CARP_TTL as u8 {
            return None;
        }
        if packet.destination != default_peer(self.cfg.peer) {
            return None;
        }
        self.accept_payload(packet.payload)
    }

    fn accept_packet6(&mut self, packet: ReceivedAdvertisement6) -> Option<(usize, CarpHeader)> {
        if packet.hop_limit != CARP_TTL as u8 {
            return None;
        }
        if packet.destination != default_peer6(self.cfg.peer6) {
            return None;
        }
        self.accept_payload(packet.payload)
    }

    fn accept_payload(&mut self, payload: Vec<u8>) -> Option<(usize, CarpHeader)> {
        if !CarpHeader::verify_checksum(&payload) {
            return None;
        }
        let header = match CarpHeader::parse(&payload) {
            Ok(header) => header,
            Err(_) => return None,
        };
        if header.validate_basic().is_err() {
            return None;
        }
        let idx = self
            .nodes
            .iter()
            .position(|node| node.node.vhid == header.vhid)?;
        if self.nodes[idx]
            .last_sent_counter
            .is_some_and(|counter| counter == header.counter)
        {
            return None;
        }
        if !self.verify_hmac(&header) {
            return None;
        }
        if !self.nodes[idx].replay.accept(header.counter) {
            return None;
        }
        Some((idx, header))
    }

    fn verify_hmac(&self, header: &CarpHeader) -> bool {
        let expected = carp_hmac_sha1_mixed(
            &self.cfg.key,
            header.vhid,
            &self.cfg.vips,
            &self.cfg.vip6s,
            &header.counter,
            HmacMode::NoV6LinkLocal,
        );
        if verify_digest(&expected, &header.digest) {
            return true;
        }
        if self.cfg.vip6s.is_empty() {
            return false;
        }
        let expected = carp_hmac_sha1_mixed(
            &self.cfg.key,
            header.vhid,
            &self.cfg.vips,
            &self.cfg.vip6s,
            &header.counter,
            HmacMode::Orig,
        );
        verify_digest(&expected, &header.digest)
    }

    fn become_master(
        &mut self,
        idx: usize,
        socket4: Option<&RawCarpSocket>,
        socket6: Option<&RawCarpSocket6>,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
    ) -> io::Result<()> {
        let was_inactive = !self.runtime_active;
        self.nodes[idx].node.state = CarpState::Master;
        self.nodes[idx].master_down = None;
        self.sync_node_demotes();
        if was_inactive {
            runtime.enter_master(&self.cfg)?;
            self.runtime_active = true;
        }
        self.sync_load_filter(load_filter)?;
        self.send_advertisements(idx, socket4, socket6)?;
        self.nodes[idx].next_advertisement =
            Some(Instant::now() + self.nodes[idx].node.local_interval());
        Ok(())
    }

    fn become_backup(
        &mut self,
        idx: usize,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
        remote: Option<RemoteAdvertisement>,
    ) -> io::Result<()> {
        self.nodes[idx].node.state = CarpState::Backup;
        self.nodes[idx].next_advertisement = None;
        let timeout = remote
            .map(master_down_timeout)
            .unwrap_or_else(|| self.nodes[idx].node.master_down_timeout());
        self.nodes[idx].master_down = Some(Instant::now() + timeout);
        if self
            .nodes
            .iter()
            .all(|node| node.node.state != CarpState::Master)
        {
            runtime.cleanup(&self.cfg);
            self.runtime_active = false;
        }
        self.sync_load_filter(load_filter)
    }

    fn send_advertisements(
        &mut self,
        idx: usize,
        socket4: Option<&RawCarpSocket>,
        socket6: Option<&RawCarpSocket6>,
    ) -> io::Result<()> {
        let counter = replay_counter();
        let vhid = self.nodes[idx].node.vhid;
        let advskew = self.nodes[idx].node.advskew;
        let packet = build_advertisement_packet(
            &self.cfg,
            vhid,
            self.cfg.advbase,
            advskew,
            self.effective_demote(),
            counter,
        );
        if let Some(socket) = socket4 {
            match socket.send_advertisement(default_peer(self.cfg.peer), &packet) {
                Ok(_) => self.record_send_success(),
                Err(err) if self.ignore_ipv4_send_error(&err) => {}
                Err(_) => self.record_send_error(),
            }
        }
        if let Some(socket) = socket6 {
            match socket.send_advertisement6(default_peer6(self.cfg.peer6), &packet) {
                Ok(_) => self.record_send_success(),
                Err(_) => self.record_send_error(),
            }
        }
        self.nodes[idx].last_sent_counter = Some(counter);
        Ok(())
    }

    fn send_bow_out_advertisements(
        &mut self,
        socket4: Option<&RawCarpSocket>,
        socket6: Option<&RawCarpSocket6>,
    ) -> io::Result<()> {
        for idx in 0..self.nodes.len() {
            if self.nodes[idx].node.state != CarpState::Master {
                continue;
            }
            let counter = replay_counter();
            let packet = build_advertisement_packet(
                &self.cfg,
                self.nodes[idx].node.vhid,
                255,
                255,
                self.effective_demote(),
                counter,
            );
            if let Some(socket) = socket4 {
                let _ = socket.send_advertisement(default_peer(self.cfg.peer), &packet);
            }
            if let Some(socket) = socket6 {
                let _ = socket.send_advertisement6(default_peer6(self.cfg.peer6), &packet);
            }
            self.nodes[idx].last_sent_counter = Some(counter);
        }
        Ok(())
    }

    fn sync_load_filter(&mut self, load_filter: &mut LoadFilter) -> io::Result<()> {
        let states: Vec<CarpState> = self.nodes.iter().map(|node| node.node.state).collect();
        load_filter.sync(&self.cfg, &states)
    }

    fn update_link_state(
        &mut self,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
    ) -> io::Result<()> {
        if runtime.link_is_running()? {
            if self.suppressed {
                self.unsuppress(runtime, load_filter)?;
            }
        } else if !self.suppressed {
            self.suppress(runtime, load_filter)?;
        }
        Ok(())
    }

    fn suppress(
        &mut self,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
    ) -> io::Result<()> {
        self.suppressed = true;
        if !self.suppress_demote_active {
            self.adjust_dynamic_demote(1);
            self.suppress_demote_active = true;
        }
        for node in &mut self.nodes {
            node.node.state = CarpState::Init;
            node.next_advertisement = None;
            node.master_down = None;
        }
        let _ = runtime.enter_backup(&self.cfg);
        self.runtime_active = false;
        self.sync_load_filter(load_filter)
    }

    fn unsuppress(
        &mut self,
        runtime: &InterfaceRuntime,
        load_filter: &mut LoadFilter,
    ) -> io::Result<()> {
        self.suppressed = false;
        if self.suppress_demote_active {
            self.adjust_dynamic_demote(-1);
            self.suppress_demote_active = false;
        }
        for node in &mut self.nodes {
            node.node.start();
            node.master_down = Some(Instant::now() + node.node.master_down_timeout());
        }
        runtime.enter_backup(&self.cfg)?;
        self.sync_load_filter(load_filter)
    }

    fn effective_demote(&self) -> u8 {
        self.cfg.demote.saturating_add(self.dynamic_demote)
    }

    fn adjust_dynamic_demote(&mut self, delta: i8) {
        if delta > 0 {
            self.dynamic_demote = self.dynamic_demote.saturating_add(delta as u8);
        } else {
            self.dynamic_demote = self.dynamic_demote.saturating_sub(delta.unsigned_abs());
        }
        self.sync_node_demotes();
    }

    fn sync_node_demotes(&mut self) {
        let demote = self.effective_demote();
        for node in &mut self.nodes {
            node.node.demote = demote;
        }
    }

    fn record_send_error(&mut self) {
        self.sendad_errors = self.sendad_errors.saturating_add(1);
        self.sendad_success = 0;
        if !self.send_error_demote_active && self.sendad_errors >= self.sendad_threshold() {
            self.adjust_dynamic_demote(1);
            self.send_error_demote_active = true;
        }
    }

    fn record_send_success(&mut self) {
        if self.send_error_demote_active {
            self.sendad_success = self.sendad_success.saturating_add(1);
            if self.sendad_success >= self.sendad_threshold() {
                self.adjust_dynamic_demote(-1);
                self.send_error_demote_active = false;
                self.sendad_errors = 0;
                self.sendad_success = 0;
            }
        } else {
            self.sendad_errors = 0;
            self.sendad_success = 0;
        }
    }

    fn sendad_threshold(&self) -> usize {
        cmp::max(self.nodes.len(), 1) * 3
    }

    fn ignore_ipv4_send_error(&self, err: &io::Error) -> bool {
        self.cfg
            .peer
            .is_some_and(|peer| !peer.is_multicast() && err.raw_os_error() == Some(EHOSTDOWN))
    }
}

fn build_advertisement_packet(
    cfg: &Config,
    vhid: u8,
    advbase: u8,
    advskew: u8,
    demote: u8,
    counter: [u8; 8],
) -> [u8; crate::proto::CARP_HEADER_LEN] {
    let digest = carp_hmac_sha1_mixed(
        &cfg.key,
        vhid,
        &cfg.vips,
        &cfg.vip6s,
        &counter,
        HmacMode::NoV6LinkLocal,
    );
    CarpHeader::advertisement(vhid, advskew, demote, advbase, counter, digest)
        .with_computed_checksum()
        .encode()
}

impl NodeRuntime {
    fn new(node: CarpNodeConfig, cfg: &Config) -> Self {
        Self {
            node: LocalNode::new(
                node.vhid,
                cfg.advbase,
                node.advskew,
                cfg.demote,
                cfg.preempt,
            ),
            replay: ReplayWindow::default(),
            last_sent_counter: None,
            next_advertisement: None,
            master_down: None,
        }
    }
}

#[derive(Default)]
struct ReplayWindow {
    highest: Option<[u8; 8]>,
}

impl ReplayWindow {
    fn accept(&mut self, counter: [u8; 8]) -> bool {
        if self.highest.is_some_and(|highest| counter <= highest) {
            return false;
        }
        self.highest = Some(counter);
        true
    }
}

fn master_down_timeout(remote: RemoteAdvertisement) -> Duration {
    let interval = advertisement_interval(remote.advbase, remote.advskew);
    interval
        .checked_mul(3)
        .unwrap_or_else(|| Duration::from_secs(cmp::max(remote.advbase as u64, 1) * 3))
}

fn cap_poll_interval(timeout: Duration) -> Duration {
    timeout.min(Duration::from_millis(250))
}

fn replay_counter() -> [u8; 8] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    nanos.to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BalancingMode, CarpNodeConfig, LinkMode, MacMode, VipAddr};
    use crate::io::{CARP_IPV4_MULTICAST, CARP_TTL};
    use crate::proto::{passphrase_to_key, CARP_HEADER_LEN};
    use std::net::Ipv4Addr;

    #[test]
    fn replay_window_rejects_duplicate_and_older_counters() {
        let mut replay = ReplayWindow::default();
        assert!(replay.accept([0, 0, 0, 0, 0, 0, 0, 2]));
        assert!(!replay.accept([0, 0, 0, 0, 0, 0, 0, 2]));
        assert!(!replay.accept([0, 0, 0, 0, 0, 0, 0, 1]));
        assert!(replay.accept([0, 0, 0, 0, 0, 0, 0, 3]));
    }

    #[test]
    fn remote_master_down_uses_remote_interval() {
        let timeout = master_down_timeout(RemoteAdvertisement {
            advbase: 1,
            advskew: 128,
            demote: 0,
        });
        assert_eq!(timeout, Duration::from_micros(4_500_000));
    }

    #[test]
    fn packet_acceptance_enforces_ttl_auth_vhid_and_replay() {
        let mut daemon = CarpDaemon::new(test_config());
        let packet = received_packet(42, 100, [0, 0, 0, 0, 0, 0, 0, 1], CARP_TTL as u8);
        assert!(daemon.accept_packet(packet.clone()).is_some());
        assert!(daemon.accept_packet(packet).is_none());

        assert!(daemon
            .accept_packet(received_packet(42, 100, [0, 0, 0, 0, 0, 0, 0, 2], 1))
            .is_none());
        assert!(daemon
            .accept_packet(received_packet(
                43,
                100,
                [0, 0, 0, 0, 0, 0, 0, 3],
                CARP_TTL as u8
            ))
            .is_none());
    }

    #[test]
    fn packet_acceptance_dispatches_to_configured_carpnode() {
        let mut cfg = test_config();
        cfg.nodes.push(CarpNodeConfig {
            vhid: 43,
            advskew: 0,
        });
        let mut daemon = CarpDaemon::new(cfg);
        let packet = received_packet(43, 0, [0, 0, 0, 0, 0, 0, 0, 1], CARP_TTL as u8);
        let accepted = daemon.accept_packet(packet).unwrap();
        assert_eq!(accepted.0, 1);
        assert_eq!(accepted.1.vhid, 43);
    }

    #[test]
    fn bow_out_packet_uses_openbsd_max_interval() {
        let cfg = test_config();
        let packet = build_advertisement_packet(&cfg, 42, 255, 255, 0, [0, 0, 0, 0, 0, 0, 0, 7]);
        let header = CarpHeader::parse(&packet).unwrap();

        assert_eq!(header.vhid, 42);
        assert_eq!(header.advbase, 255);
        assert_eq!(header.advskew, 255);
        assert!(CarpHeader::verify_checksum(&packet));
    }

    #[test]
    fn send_error_demote_uses_openbsd_threshold_and_recovers() {
        let mut daemon = CarpDaemon::new(test_config());

        daemon.record_send_error();
        daemon.record_send_error();
        assert_eq!(daemon.effective_demote(), 0);
        assert!(!daemon.send_error_demote_active);

        daemon.record_send_error();
        assert_eq!(daemon.effective_demote(), 1);
        assert!(daemon.send_error_demote_active);
        assert_eq!(daemon.nodes[0].node.demote, 1);

        daemon.record_send_success();
        daemon.record_send_success();
        assert_eq!(daemon.effective_demote(), 1);
        assert!(daemon.send_error_demote_active);

        daemon.record_send_success();
        assert_eq!(daemon.effective_demote(), 0);
        assert!(!daemon.send_error_demote_active);
        assert_eq!(daemon.nodes[0].node.demote, 0);
    }

    #[test]
    fn dynamic_demote_is_carried_in_advertisement_header() {
        let mut daemon = CarpDaemon::new(test_config());
        daemon.adjust_dynamic_demote(1);

        let packet = build_advertisement_packet(
            &daemon.cfg,
            42,
            1,
            50,
            daemon.effective_demote(),
            [0, 0, 0, 0, 0, 0, 0, 9],
        );
        let header = CarpHeader::parse(&packet).unwrap();

        assert_eq!(header.demote, 1);
        assert!(CarpHeader::verify_checksum(&packet));
    }

    #[test]
    fn ipv4_unicast_ehostdown_does_not_count_as_send_error() {
        let mut cfg = test_config();
        cfg.peer = Some(Ipv4Addr::new(10, 0, 253, 43));
        let daemon = CarpDaemon::new(cfg);

        assert!(daemon.ignore_ipv4_send_error(&io::Error::from_raw_os_error(EHOSTDOWN)));
        assert!(!daemon.ignore_ipv4_send_error(&io::Error::from_raw_os_error(1)));
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
            vip6s: Vec::new(),
            vip6_addrs: Vec::new(),
            key: passphrase_to_key("interop-pass"),
            manage_vip: false,
            announce: false,
            mac_mode: MacMode::Interface,
            balancing: BalancingMode::None,
            load_filter: crate::config::LoadFilterMode::Off,
            nodes: vec![CarpNodeConfig {
                vhid: 42,
                advskew: 50,
            }],
        }
    }

    fn received_packet(vhid: u8, advskew: u8, counter: [u8; 8], ttl: u8) -> ReceivedAdvertisement {
        let key = passphrase_to_key("interop-pass");
        let digest = carp_hmac_sha1_mixed(
            &key,
            vhid,
            &[Ipv4Addr::new(10, 0, 253, 42)],
            &[],
            &counter,
            HmacMode::NoV6LinkLocal,
        );
        let payload = CarpHeader::advertisement(vhid, advskew, 0, 1, counter, digest)
            .with_computed_checksum()
            .encode();
        ReceivedAdvertisement {
            source: Ipv4Addr::new(10, 0, 253, 43),
            destination: CARP_IPV4_MULTICAST,
            ttl,
            payload: payload[..CARP_HEADER_LEN].to_vec(),
        }
    }
}
