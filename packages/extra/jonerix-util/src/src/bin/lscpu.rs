//! lscpu — display information about the CPU architecture. Clean-room
//! implementation that reads /proc/cpuinfo plus /sys/devices/system/cpu
//! and formats a key/value listing compatible with the util-linux
//! reference output.
//!
//! Sources consulted:
//!   - procfs(5), sysfs(5) man pages
//!   - kernel Documentation/cpu-and-memory-hotplug/cpu-hotplug.rst
//!   - Intel/AMD/ARM public architecture manuals for vendor-string meaning
//!   - Observed textual output of `lscpu` on reference machines
//! No util-linux source code was consulted.

use jxutil::{proc, sysfs, table};
use std::collections::BTreeSet;
use std::fs;

fn first(recs: &[Vec<(String,String)>], key: &str) -> Option<String> {
    recs.get(0).and_then(|r| r.iter().find(|(k,_)| k == key).map(|(_,v)| v.clone()))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("Usage: lscpu [options]");
        println!("Display information about the CPU architecture.");
        println!("  -J, --json      use JSON output format");
        println!("  -h, --help      display this help");
        return;
    }
    let json = args.iter().any(|a| a == "-J" || a == "--json");

    // ── /proc/cpuinfo ──────────────────────────────────────────
    let cpuinfo_raw = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let recs = proc::parse_cpuinfo(&cpuinfo_raw);

    // ── Arch / byte order ─────────────────────────────────────
    let arch = std::env::consts::ARCH.to_string();
    let byte_order = if cfg!(target_endian = "little") { "Little Endian" } else { "Big Endian" };

    // ── /sys topology walk ────────────────────────────────────
    // B15 fix: read the kernel's authoritative online list rather than
    // enumerating cpuN directories (which includes offline CPUs).
    let online_str = sysfs::try_line("/sys/devices/system/cpu/online")
        .unwrap_or_default();
    let cpus = sysfs::parse_cpulist(&online_str);
    let online_cnt = cpus.len();

    // B14 fix: collect (pkg_id, core_id) PAIRS to correctly count
    // cores per socket on multi-socket systems where core IDs repeat.
    let mut core_pairs: BTreeSet<(String, String)> = BTreeSet::new();
    let mut pkg_ids:  BTreeSet<String> = BTreeSet::new();
    let mut max_mhz: Option<f64> = None;
    let mut min_mhz: Option<f64> = None;
    for c in &cpus {
        let mut path = format!("/sys/devices/system/cpu/cpu{}", c);
        let base_len = path.len();

        path.push_str("/topology/core_id");
        let core_id = sysfs::try_line(&path).unwrap_or_default();
        path.truncate(base_len);

        path.push_str("/topology/physical_package_id");
        let pkg_id = sysfs::try_line(&path).unwrap_or_else(|| "0".to_string());
        path.truncate(base_len);

        core_pairs.insert((pkg_id.clone(), core_id));
        pkg_ids.insert(pkg_id);

        path.push_str("/cpufreq/cpuinfo_max_freq");
        if let Some(v) = sysfs::try_line(&path) {
            if let Ok(k) = v.parse::<u64>() { let m = k as f64 / 1000.0; max_mhz = Some(max_mhz.map_or(m, |x| x.max(m))); }
        }
        path.truncate(base_len);

        path.push_str("/cpufreq/cpuinfo_min_freq");
        if let Some(v) = sysfs::try_line(&path) {
            if let Ok(k) = v.parse::<u64>() { let m = k as f64 / 1000.0; min_mhz = Some(min_mhz.map_or(m, |x| x.min(m))); }
        }
        path.truncate(base_len);
    }
    let sockets = pkg_ids.len().max(1);
    let total_cores = core_pairs.len().max(1);
    let cores_per_socket = (total_cores / sockets).max(1);
    let threads_per_core = (online_cnt / total_cores).max(1);

    // ── Cache ─────────────────────────────────────────────────
    let mut cache_rows: Vec<(String, String)> = Vec::new();
    if !cpus.is_empty() {
        let base = format!("/sys/devices/system/cpu/cpu{}/cache", cpus[0]);
        if let Ok(ents) = fs::read_dir(&base) {
            let mut lvls: Vec<u32> = ents.flatten().filter_map(|e| {
                let n = e.file_name().into_string().ok()?;
                n.strip_prefix("index").and_then(|r| r.parse::<u32>().ok())
            }).collect();
            lvls.sort();
            for idx in lvls {
                let ib = format!("{}/index{}", base, idx);
                let level = sysfs::try_line(format!("{}/level", ib)).unwrap_or_default();
                let typ   = sysfs::try_line(format!("{}/type", ib)).unwrap_or_default();
                let sz    = sysfs::try_line(format!("{}/size", ib)).unwrap_or_default();
                let label = match typ.as_str() {
                    "Data"         => format!("L{}d cache", level),
                    "Instruction"  => format!("L{}i cache", level),
                    _              => format!("L{} cache",  level),
                };
                cache_rows.push((label, sz));
            }
        }
    }

    // ── Collect rows ──────────────────────────────────────────
    let mut rows: Vec<(String, String)> = Vec::new();
    rows.push(("Architecture".into(), arch));
    rows.push(("Byte Order".into(), byte_order.into()));
    rows.push(("CPU(s)".into(), online_cnt.to_string()));
    // B15 fix: use kernel-reported online list verbatim (handles hotplug gaps).
    rows.push(("On-line CPU(s) list".into(), online_str.clone()));

    let vendor = first(&recs, "vendor_id").or_else(|| first(&recs, "CPU implementer"));
    if let Some(v) = vendor { rows.push(("Vendor ID".into(), v)); }
    if let Some(v) = first(&recs, "model name").or_else(|| first(&recs, "Model name")) {
        rows.push(("Model name".into(), v));
    }
    if let Some(v) = first(&recs, "cpu family") { rows.push(("CPU family".into(), v)); }
    if let Some(v) = first(&recs, "model")      { rows.push(("Model".into(), v)); }
    if let Some(v) = first(&recs, "stepping")   { rows.push(("Stepping".into(), v)); }
    rows.push(("Thread(s) per core".into(), threads_per_core.to_string()));
    rows.push(("Core(s) per socket".into(), cores_per_socket.to_string()));
    rows.push(("Socket(s)".into(), sockets.to_string()));
    // B16 fix: util-linux uses 4 decimal places matching hardware granularity.
    if let Some(m) = max_mhz { rows.push(("CPU max MHz".into(), format!("{:.4}", m))); }
    if let Some(m) = min_mhz { rows.push(("CPU min MHz".into(), format!("{:.4}", m))); }
    if let Some(v) = first(&recs, "bogomips").or_else(|| first(&recs, "BogoMIPS")) {
        rows.push(("BogoMIPS".into(), v));
    }
    if let Some(v) = first(&recs, "flags").or_else(|| first(&recs, "Features")) {
        rows.push(("Flags".into(), v));
    }
    rows.extend(cache_rows);

    // ── Emit ──────────────────────────────────────────────────
    if json {
        print!("{{\n   \"lscpu\": [\n");
        let n = rows.len();
        for (i, (k, v)) in rows.iter().enumerate() {
            let comma = if i + 1 < n { "," } else { "" };
            print!("      {{\"field\": \"{}\", \"data\": \"{}\"}}{}\n",
                json_escape(k), json_escape(v), comma);
        }
        print!("   ]\n}}\n");
    } else {
        table::print_kv(&rows);
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"'  => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
