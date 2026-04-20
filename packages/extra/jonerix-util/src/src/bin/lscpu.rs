//! lscpu — display information about the CPU architecture. Clean-room
//! implementation that reads /proc/cpuinfo plus /sys/devices/system/cpu
//! and formats a key/value listing compatible with the util-linux
//! reference output.
//!
//! Sources consulted:
//!   - procfs(5), sysfs(5) man pages
//!   - kernel Documentation/cpu-and-memory-hotplug/cpu-hotplug.rst
//!   - Intel/AMD/ARM public architecture manuals for vendor-string meaning
//!   - ARM MIDR implementer IDs from ARM DDI 0487 (Arch Ref Manual)
//!   - Per-vendor part numbers from public datasheets and linux/include
//!     /asm/cputype.h (values only, not the header text)
//!   - Observed textual output of `lscpu` on reference machines
//! No util-linux source code was consulted.

use jxutil::{proc, sysfs, table};
use std::collections::BTreeSet;
use std::fs;

fn first(recs: &[Vec<(String,String)>], key: &str) -> Option<String> {
    recs.get(0).and_then(|r| r.iter().find(|(k,_)| k == key).map(|(_,v)| v.clone()))
}

/// Decode a hex MIDR "CPU implementer" value into the vendor's
/// canonical short name. ARM assigned these IDs and publishes the
/// current list in the Architecture Reference Manual (DDI 0487),
/// appendix MIDR_EL1.
fn implementer_name(hex: u32) -> Option<&'static str> {
    Some(match hex {
        0x41 => "ARM",
        0x42 => "Broadcom",
        0x43 => "Cavium",
        0x44 => "DEC",
        0x46 => "Fujitsu",
        0x48 => "HiSilicon",
        0x49 => "Infineon",
        0x4d => "Motorola/Freescale",
        0x4e => "NVIDIA",
        0x50 => "APM",
        0x51 => "Qualcomm",
        0x53 => "Samsung",
        0x56 => "Marvell",
        0x61 => "Apple",
        0x66 => "Faraday",
        0x69 => "Intel",
        0x6d => "Microsoft",
        0x70 => "Phytium",
        0xc0 => "Ampere",
        _ => return None,
    })
}

/// Decode (implementer, part) into the CPU model name. Part IDs
/// collide across implementers, so the tuple matters. Each arm is
/// sorted by part number inside the match.
fn part_name(implementer: u32, part: u32) -> Option<&'static str> {
    Some(match (implementer, part) {
        // ARM Ltd — Cortex / Neoverse.
        (0x41, 0x810) => "ARM810",
        (0x41, 0x920) => "ARM920",
        (0x41, 0x922) => "ARM922",
        (0x41, 0x926) => "ARM926",
        (0x41, 0x940) => "ARM940",
        (0x41, 0x946) => "ARM946",
        (0x41, 0x966) => "ARM966",
        (0x41, 0xa20) => "ARM1020",
        (0x41, 0xa22) => "ARM1022",
        (0x41, 0xa26) => "ARM1026",
        (0x41, 0xb02) => "ARM11 MPCore",
        (0x41, 0xb36) => "ARM1136",
        (0x41, 0xb56) => "ARM1156",
        (0x41, 0xb76) => "ARM1176",
        (0x41, 0xc05) => "Cortex-A5",
        (0x41, 0xc07) => "Cortex-A7",
        (0x41, 0xc08) => "Cortex-A8",
        (0x41, 0xc09) => "Cortex-A9",
        (0x41, 0xc0d) => "Cortex-A12",
        (0x41, 0xc0e) => "Cortex-A17",
        (0x41, 0xc0f) => "Cortex-A15",
        (0x41, 0xc14) => "Cortex-R4",
        (0x41, 0xc15) => "Cortex-R5",
        (0x41, 0xc17) => "Cortex-R7",
        (0x41, 0xc18) => "Cortex-R8",
        (0x41, 0xc20) => "Cortex-M0",
        (0x41, 0xc21) => "Cortex-M1",
        (0x41, 0xc23) => "Cortex-M3",
        (0x41, 0xc24) => "Cortex-M4",
        (0x41, 0xc27) => "Cortex-M7",
        (0x41, 0xc60) => "Cortex-M0+",
        (0x41, 0xd01) => "Cortex-A32",
        (0x41, 0xd02) => "Cortex-A34",
        (0x41, 0xd03) => "Cortex-A53",
        (0x41, 0xd04) => "Cortex-A35",
        (0x41, 0xd05) => "Cortex-A55",
        (0x41, 0xd06) => "Cortex-A65",
        (0x41, 0xd07) => "Cortex-A57",
        (0x41, 0xd08) => "Cortex-A72",
        (0x41, 0xd09) => "Cortex-A73",
        (0x41, 0xd0a) => "Cortex-A75",
        (0x41, 0xd0b) => "Cortex-A76",
        (0x41, 0xd0c) => "Neoverse-N1",
        (0x41, 0xd0d) => "Cortex-A77",
        (0x41, 0xd0e) => "Cortex-A76AE",
        (0x41, 0xd13) => "Cortex-R52",
        (0x41, 0xd15) => "Cortex-R82",
        (0x41, 0xd20) => "Cortex-M23",
        (0x41, 0xd21) => "Cortex-M33",
        (0x41, 0xd40) => "Neoverse-V1",
        (0x41, 0xd41) => "Cortex-A78",
        (0x41, 0xd42) => "Cortex-A78AE",
        (0x41, 0xd43) => "Cortex-A65AE",
        (0x41, 0xd44) => "Cortex-X1",
        (0x41, 0xd46) => "Cortex-A510",
        (0x41, 0xd47) => "Cortex-A710",
        (0x41, 0xd48) => "Cortex-X2",
        (0x41, 0xd49) => "Neoverse-N2",
        (0x41, 0xd4a) => "Neoverse-E1",
        (0x41, 0xd4b) => "Cortex-A78C",
        (0x41, 0xd4c) => "Cortex-X1C",
        (0x41, 0xd4d) => "Cortex-A715",
        (0x41, 0xd4e) => "Cortex-X3",
        (0x41, 0xd4f) => "Neoverse-V2",
        (0x41, 0xd80) => "Cortex-A520",
        (0x41, 0xd81) => "Cortex-A720",
        (0x41, 0xd82) => "Cortex-X4",

        // Broadcom.
        (0x42, 0xf) => "Brahma B15",
        (0x42, 0x100) => "Brahma B53",
        (0x42, 0x516) => "ThunderX2",

        // Cavium.
        (0x43, 0xa0) => "ThunderX",
        (0x43, 0xa1) => "ThunderX 88XX",
        (0x43, 0xa2) => "ThunderX 81XX",
        (0x43, 0xa3) => "ThunderX 83XX",
        (0x43, 0xaf) => "ThunderX2 99xx",
        (0x43, 0xb0) => "OcteonTX2",
        (0x43, 0xb1) => "OcteonTX2 T98",
        (0x43, 0xb2) => "OcteonTX2 T96",

        // Fujitsu.
        (0x46, 0x001) => "A64FX",

        // HiSilicon.
        (0x48, 0xd01) => "Kunpeng-920",
        (0x48, 0xd02) => "Kunpeng-920 (TSV110)",

        // NVIDIA.
        (0x4e, 0x000) => "Denver",
        (0x4e, 0x003) => "Denver 2",
        (0x4e, 0x004) => "Carmel",

        // Qualcomm.
        (0x51, 0x00f) => "Scorpion",
        (0x51, 0x02d) => "Scorpion",
        (0x51, 0x04d) => "Krait",
        (0x51, 0x06f) => "Krait",
        (0x51, 0x201) => "Kryo Silver (Snapdragon 821)",
        (0x51, 0x205) => "Kryo Gold (Snapdragon 821)",
        (0x51, 0x211) => "Kryo Silver (Snapdragon 820)",
        (0x51, 0x800) => "Falkor v1/Kryo",
        (0x51, 0x801) => "Kryo Silver (Snapdragon 835)",
        (0x51, 0x802) => "Kryo Gold (Snapdragon 835)",
        (0x51, 0x803) => "Kryo Silver (Snapdragon 670)",
        (0x51, 0x804) => "Kryo Gold (Snapdragon 670)",
        (0x51, 0x805) => "Kryo Silver (Snapdragon 665)",
        (0x51, 0xc00) => "Falkor",
        (0x51, 0xc01) => "Saphira",
        (0x51, 0x001) => "Oryon",

        // Samsung (Mongoose).
        (0x53, 0x001) => "exynos-m1",

        // Marvell.
        (0x56, 0x131) => "Feroceon 88FR131",
        (0x56, 0x581) => "PJ4/PJ4B",
        (0x56, 0x584) => "PJ4B-MP",

        // Apple Silicon.
        (0x61, 0x000) => "Swift",
        (0x61, 0x001) => "Cyclone",
        (0x61, 0x002) => "Typhoon",
        (0x61, 0x003) => "Typhoon/Capri",
        (0x61, 0x004) => "Twister",
        (0x61, 0x005) => "Twister/Elba/Malta",
        (0x61, 0x006) => "Hurricane",
        (0x61, 0x007) => "Hurricane/Myst",
        (0x61, 0x008) => "Monsoon",
        (0x61, 0x009) => "Mistral",
        (0x61, 0x00b) => "Vortex",
        (0x61, 0x00c) => "Tempest",
        (0x61, 0x00f) => "Tempest-M9",
        (0x61, 0x010) => "Vortex/Aruba",
        (0x61, 0x011) => "Tempest/Aruba",
        (0x61, 0x012) => "Lightning",
        (0x61, 0x013) => "Thunder",
        (0x61, 0x020) => "Icestorm (A14)",
        (0x61, 0x021) => "Firestorm (A14)",
        (0x61, 0x022) => "Icestorm (M1)",
        (0x61, 0x023) => "Firestorm (M1)",
        (0x61, 0x024) => "Icestorm (M1 Pro)",
        (0x61, 0x025) => "Firestorm (M1 Pro)",
        (0x61, 0x028) => "Icestorm (M1 Max)",
        (0x61, 0x029) => "Firestorm (M1 Max)",
        (0x61, 0x030) => "Blizzard (A15)",
        (0x61, 0x031) => "Avalanche (A15)",
        (0x61, 0x032) => "Blizzard (M2)",
        (0x61, 0x033) => "Avalanche (M2)",
        (0x61, 0x034) => "Blizzard (M2 Pro)",
        (0x61, 0x035) => "Avalanche (M2 Pro)",
        (0x61, 0x038) => "Blizzard (M2 Max)",
        (0x61, 0x039) => "Avalanche (M2 Max)",
        (0x61, 0x046) => "Sawtooth (A16)",
        (0x61, 0x047) => "Everest (A16)",

        // Intel (ARM-era XScale).
        (0x69, 0x200) => "i80200",
        (0x69, 0x210) => "PXA250A",
        (0x69, 0x212) => "PXA210A",
        (0x69, 0x242) => "i80321-400",
        (0x69, 0x243) => "i80321-600",
        (0x69, 0x290) => "PXA250B/PXA26x",
        (0x69, 0x292) => "PXA210B",

        // Ampere.
        (0xc0, 0xac3) => "Ampere-1",
        (0xc0, 0xac4) => "Ampere-1a",

        _ => return None,
    })
}

/// Parse a hex literal from /proc/cpuinfo, which stores values as
/// `0xNN` (CPU implementer, variant) or `0xNNN` (part). Returns
/// None on anything unparseable.
fn parse_hex(s: &str) -> Option<u32> {
    let s = s.trim();
    let rest = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    u32::from_str_radix(rest, 16).ok()
}

/// Read and trim /proc/device-tree/model (present on ARM boards
/// booted via a device tree — Pi, NVIDIA Jetson, most SBCs). NUL
/// byte at end is a dtb artifact; strip it.
fn read_board_model() -> Option<String> {
    let raw = fs::read_to_string("/proc/device-tree/model").ok()?;
    let trimmed = raw.trim_end_matches('\0').trim();
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
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
    let cpus = sysfs::enum_numeric("/sys/devices/system/cpu", "cpu");
    let online_cnt = cpus.len();

    // Collect unique core IDs and physical package IDs
    let mut core_ids: BTreeSet<String> = BTreeSet::new();
    let mut pkg_ids:  BTreeSet<String> = BTreeSet::new();
    let mut max_mhz: Option<f64> = None;
    let mut min_mhz: Option<f64> = None;
    for c in &cpus {
        let base = format!("/sys/devices/system/cpu/cpu{}", c);
        if let Some(v) = sysfs::try_line(format!("{}/topology/core_id", base)) { core_ids.insert(v); }
        if let Some(v) = sysfs::try_line(format!("{}/topology/physical_package_id", base)) { pkg_ids.insert(v); }
        if let Some(v) = sysfs::try_line(format!("{}/cpufreq/cpuinfo_max_freq", base)) {
            if let Ok(k) = v.parse::<u64>() { let m = k as f64 / 1000.0; max_mhz = Some(max_mhz.map_or(m, |x| x.max(m))); }
        }
        if let Some(v) = sysfs::try_line(format!("{}/cpufreq/cpuinfo_min_freq", base)) {
            if let Ok(k) = v.parse::<u64>() { let m = k as f64 / 1000.0; min_mhz = Some(min_mhz.map_or(m, |x| x.min(m))); }
        }
    }
    let sockets = pkg_ids.len().max(1);
    let cores_per_socket = (core_ids.len().max(1) / sockets).max(1);
    let threads_per_core = (online_cnt / (sockets * cores_per_socket)).max(1);

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
    rows.push(("On-line CPU(s) list".into(),
        if cpus.is_empty() { String::new() }
        else { format!("{}-{}", cpus.first().unwrap(), cpus.last().unwrap()) }));

    // Vendor: x86 exposes a ready-made `vendor_id` string. ARM only
    // gives a hex `CPU implementer` code, so translate it through
    // our implementer table and emit both forms: the decoded vendor
    // name (what humans expect in `lscpu`) and the raw hex for
    // parity with other tools.
    let x86_vendor = first(&recs, "vendor_id");
    let arm_implementer_hex = first(&recs, "CPU implementer");
    let arm_implementer = arm_implementer_hex.as_deref().and_then(parse_hex);
    let arm_part_hex = first(&recs, "CPU part");
    let arm_part = arm_part_hex.as_deref().and_then(parse_hex);

    if let Some(v) = x86_vendor {
        rows.push(("Vendor ID".into(), v));
    } else if let Some(imp) = arm_implementer {
        let label = implementer_name(imp)
            .map(|n| format!("{} (0x{:02x})", n, imp))
            .unwrap_or_else(|| format!("0x{:02x}", imp));
        rows.push(("Vendor ID".into(), label));
    }

    // Model name: prefer an on-disk `model name`/`Model name` when
    // present (x86 + some vendor-extended ARM kernels), else look
    // up (implementer, part) in our ARM table. Boards based on
    // BCM/Rockchip/Allwinner all come through as ARM Ltd parts.
    let explicit_model = first(&recs, "model name")
        .or_else(|| first(&recs, "Model name"));
    let derived_model = match (arm_implementer, arm_part) {
        (Some(imp), Some(part)) => part_name(imp, part).map(|s| s.to_string()),
        _ => None,
    };
    if let Some(v) = explicit_model.or(derived_model) {
        rows.push(("Model name".into(), v));
    } else if let Some(hex) = arm_part_hex {
        // No dictionary hit — surface the raw part ID so the
        // operator has something to google.
        rows.push(("Model name".into(), format!("Unknown ARM part {}", hex)));
    }

    // Board: /proc/device-tree/model is a device-tree artefact; on
    // Pi / Jetson / most SBCs it's the best short summary of the
    // physical board. Skip on hosts without a DT (x86).
    if let Some(board) = read_board_model() {
        rows.push(("Model".into(), board));
    }
    if let Some(v) = first(&recs, "cpu family") { rows.push(("CPU family".into(), v)); }
    if let Some(v) = first(&recs, "model")      { rows.push(("Model".into(), v)); }
    if let Some(v) = first(&recs, "stepping")   { rows.push(("Stepping".into(), v)); }
    rows.push(("Thread(s) per core".into(), threads_per_core.to_string()));
    rows.push(("Core(s) per socket".into(), cores_per_socket.to_string()));
    rows.push(("Socket(s)".into(), sockets.to_string()));
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
