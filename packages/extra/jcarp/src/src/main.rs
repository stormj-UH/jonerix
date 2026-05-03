use std::env;
use std::io;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use jcarp::config::{CarpNodeConfig, Config};
use jcarp::daemon::run_daemon;
use jcarp::io::{
    default_peer, default_peer6, AdvertisementTransport, AdvertisementTransport6, RawCarpSocket,
    RawCarpSocket6,
};
use jcarp::proto::{carp_hmac_sha1_mixed, CarpHeader, HmacMode};
use jcarp::state::LocalNode;

const DEFAULT_CONFIG: &str = "/etc/jcarp/jcarp.conf";
static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
}

fn main() {
    if let Err(err) = run() {
        eprintln!("jcarp: {err}");
        process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut args = env::args().skip(1);
    let mut config_path = PathBuf::from(DEFAULT_CONFIG);
    let mut command = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-V" | "--version" => {
                println!("jcarp {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-c" | "--config" => {
                let Some(path) = args.next() else {
                    return Err(invalid("--config requires a path"));
                };
                config_path = PathBuf::from(path);
            }
            "check" | "run" | "send-once" | "show" => {
                command = Some(arg);
                break;
            }
            _ => return Err(invalid("unknown argument")),
        }
    }

    match command.as_deref().unwrap_or("check") {
        "check" => {
            let cfg = Config::load(&config_path)?;
            println!(
                "config ok: interface={} effective_interface={} nodes={} vips={}",
                cfg.interface,
                cfg.effective_interface(),
                format_nodes(&cfg.nodes),
                cfg.vips.len() + cfg.vip6s.len()
            );
            Ok(())
        }
        "show" => {
            let cfg = Config::load(&config_path)?;
            let mut node =
                LocalNode::new(cfg.vhid, cfg.advbase, cfg.advskew, cfg.demote, cfg.preempt);
            node.start();
            println!(
                "interface={} effective_interface={} parent_interface={} link_mode={:?} nodes={} state={:?} interval_us={} peer={} peer6={} vips4={} vips6={} manage_vip={} announce={} mac={:?} balancing={:?} load_filter={:?}",
                cfg.interface,
                cfg.effective_interface(),
                cfg.parent_interface(),
                cfg.link_mode,
                format_nodes(&cfg.nodes),
                node.state,
                node.local_interval().as_micros(),
                default_peer(cfg.peer),
                default_peer6(cfg.peer6),
                cfg.vips.len(),
                cfg.vip6s.len(),
                cfg.manage_vip,
                cfg.announce,
                cfg.mac_mode,
                cfg.balancing,
                cfg.load_filter,
            );
            Ok(())
        }
        "send-once" => {
            let cfg = Config::load(&config_path)?;
            let socket4 = if !cfg.vips.is_empty() || cfg.peer.is_some() {
                Some(RawCarpSocket::open_v4_for_interface(
                    cfg.effective_interface(),
                )?)
            } else {
                None
            };
            let socket6 = if !cfg.vip6s.is_empty() || cfg.peer6.is_some() {
                Some(RawCarpSocket6::open_for_interface(
                    cfg.effective_interface(),
                )?)
            } else {
                None
            };
            for node in &cfg.nodes {
                let packet = build_advertisement(&cfg, *node);
                if let Some(socket) = &socket4 {
                    let destination = default_peer(cfg.peer);
                    socket.send_advertisement(destination, &packet)?;
                    println!(
                        "sent IPv4 CARP advertisement vhid={} to {destination}",
                        node.vhid
                    );
                }
                if let Some(socket) = &socket6 {
                    let destination = default_peer6(cfg.peer6);
                    socket.send_advertisement6(destination, &packet)?;
                    println!(
                        "sent IPv6 CARP advertisement vhid={} to {destination}",
                        node.vhid
                    );
                }
            }
            Ok(())
        }
        "run" => {
            let cfg = Config::load(&config_path)?;
            install_signal_handlers();
            run_daemon(cfg, || TERMINATE.load(Ordering::Relaxed))
        }
        _ => unreachable!(),
    }
}

fn build_advertisement(cfg: &Config, node: CarpNodeConfig) -> [u8; jcarp::proto::CARP_HEADER_LEN] {
    let counter = replay_counter();
    let digest = carp_hmac_sha1_mixed(
        &cfg.key,
        node.vhid,
        &cfg.vips,
        &cfg.vip6s,
        &counter,
        HmacMode::NoV6LinkLocal,
    );
    CarpHeader::advertisement(
        node.vhid,
        node.advskew,
        cfg.demote,
        cfg.advbase,
        counter,
        digest,
    )
    .with_computed_checksum()
    .encode()
}

fn replay_counter() -> [u8; 8] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    nanos.to_be_bytes()
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

fn format_nodes(nodes: &[CarpNodeConfig]) -> String {
    nodes
        .iter()
        .map(|node| format!("{}:{}", node.vhid, node.advskew))
        .collect::<Vec<_>>()
        .join(",")
}

fn print_help() {
    println!("usage: jcarp [--config PATH] <check|show|send-once|run>");
    println!("       jcarp --help");
    println!("       jcarp --version");
}

fn install_signal_handlers() {
    unsafe {
        signal(2, handle_signal);
        signal(15, handle_signal);
    }
}

extern "C" fn handle_signal(_signum: i32) {
    TERMINATE.store(true, Ordering::Relaxed);
}
