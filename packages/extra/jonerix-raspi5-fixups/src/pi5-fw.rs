//! pi5-fw — Raspberry Pi 5 firmware tool (zero external dependencies)
//!
//! Talks to the VideoCore mailbox via /dev/vcio ioctl to query and
//! configure firmware properties.  Replaces vcgencmd for the subset
//! of commands needed by jonerix.
//!
//! Usage:
//!   pi5-fw measure_temp
//!   pi5-fw get_throttled
//!   pi5-fw bootloader_config
//!   pi5-fw reboot_mode [cold|warm]
//!   pi5-fw set boot_order HEX     # e.g. f14 (try USB first, then SD)

use std::fs::{File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::os::unix::io::AsRawFd;
use std::process;

// ---------------------------------------------------------------------------
// VideoCore mailbox constants
// ---------------------------------------------------------------------------

/// ioctl request code for the VideoCore mailbox property interface.
/// _IOWR(100, 0, char*) on aarch64 linux = direction(3)<<30 | size(8)<<16 | type(100)<<8 | nr(0)
/// = 0xC008_6400
const IOCTL_MBOX_PROPERTY: u64 = 0xC008_6400;

// Mailbox buffer codes
const MBOX_REQUEST: u32 = 0x0000_0000;
const MBOX_RESPONSE_SUCCESS: u32 = 0x8000_0000;

// Tag IDs — from the VideoCore firmware interface spec
const TAG_GET_TEMPERATURE: u32 = 0x0003_0006;
const TAG_GET_THROTTLED: u32 = 0x0003_0046;
const TAG_GET_FIRMWARE_REVISION: u32 = 0x0000_0001;
const TAG_GET_BOARD_MODEL: u32 = 0x0001_0001;
const TAG_GET_BOARD_REVISION: u32 = 0x0001_0002;
const TAG_GET_BOARD_SERIAL: u32 = 0x0001_0004;
const TAG_GET_MAX_CLOCK_RATE: u32 = 0x0003_0004;
const TAG_GET_CLOCK_RATE: u32 = 0x0003_0002;
const TAG_END: u32 = 0x0000_0000;

// Clock IDs
const CLOCK_ARM: u32 = 3;

// ---------------------------------------------------------------------------
// Raw ioctl via syscall — no libc crate needed
// ---------------------------------------------------------------------------

/// Perform an ioctl syscall.  aarch64 linux: __NR_ioctl = 29.
unsafe fn raw_ioctl(fd: i32, request: u64, arg: *mut u8) -> i32 {
    let ret: i64;
    unsafe {
        std::arch::asm!(
            "svc 0",
            in("x8") 29u64,
            inout("x0") fd as i64 => ret,
            in("x1") request,
            in("x2") arg as u64,
            options(nostack)
        );
    }
    ret as i32
}

// ---------------------------------------------------------------------------
// Mailbox property interface
// ---------------------------------------------------------------------------

/// Send a single tag request to the VideoCore mailbox.
/// `tag` is the tag ID, `req_data` is the request payload,
/// `resp_len` is the expected response length in u32 words.
/// Returns the response payload as a Vec<u32>.
fn mbox_property(tag: u32, req_data: &[u32], resp_len: usize) -> io::Result<Vec<u32>> {
    let f = OpenOptions::new().read(true).write(true).open("/dev/vcio")?;

    let value_len = std::cmp::max(req_data.len(), resp_len);

    // Buffer layout (in u32 words):
    //   [0] total buffer size in bytes
    //   [1] request/response code
    //   [2] tag id
    //   [3] value buffer size in bytes
    //   [4] request/response indicator (0 = request)
    //   [5..5+value_len] value buffer
    //   [5+value_len] end tag (0)
    let buf_words = 5 + value_len + 1;
    let buf_bytes = buf_words * 4;

    let mut buf: Vec<u32> = vec![0u32; buf_words];

    buf[0] = buf_bytes as u32;
    buf[1] = MBOX_REQUEST;
    buf[2] = tag;
    buf[3] = (value_len * 4) as u32;
    buf[4] = 0; // request
    for (i, &v) in req_data.iter().enumerate() {
        buf[5 + i] = v;
    }
    buf[5 + value_len] = TAG_END;

    let fd = f.as_raw_fd();
    let ret = unsafe { raw_ioctl(fd, IOCTL_MBOX_PROPERTY, buf.as_mut_ptr() as *mut u8) };

    if ret < 0 {
        // Raw syscall returns -errno, not setting libc errno
        return Err(io::Error::from_raw_os_error(-ret));
    }

    if buf[1] != MBOX_RESPONSE_SUCCESS {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("mailbox response code: 0x{:08x}", buf[1]),
        ));
    }

    // Response indicator in buf[4] should have bit 31 set
    if buf[4] & 0x8000_0000 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("tag response indicator: 0x{:08x}", buf[4]),
        ));
    }

    let resp_actual_len = (buf[4] & 0x7FFF_FFFF) as usize / 4;
    let n = std::cmp::min(resp_actual_len, value_len);
    Ok(buf[5..5 + n].to_vec())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_measure_temp() -> io::Result<()> {
    let resp = mbox_property(TAG_GET_TEMPERATURE, &[0, 0], 2)?;
    if resp.len() >= 2 {
        let temp_mc = resp[1] as f64;
        println!("temp={:.1}'C", temp_mc / 1000.0);
    }
    Ok(())
}

fn cmd_get_throttled() -> io::Result<()> {
    let resp = mbox_property(TAG_GET_THROTTLED, &[0], 1)?;
    if !resp.is_empty() {
        let flags = resp[0];
        println!("throttled=0x{:x}", flags);
        if flags & (1 << 0) != 0 { println!("  [0] Under-voltage detected"); }
        if flags & (1 << 1) != 0 { println!("  [1] Arm frequency capped"); }
        if flags & (1 << 2) != 0 { println!("  [2] Currently throttled"); }
        if flags & (1 << 3) != 0 { println!("  [3] Soft temperature limit active"); }
        if flags & (1 << 16) != 0 { println!(" [16] Under-voltage has occurred"); }
        if flags & (1 << 17) != 0 { println!(" [17] Arm frequency capping has occurred"); }
        if flags & (1 << 18) != 0 { println!(" [18] Throttling has occurred"); }
        if flags & (1 << 19) != 0 { println!(" [19] Soft temperature limit has occurred"); }
    }
    Ok(())
}

fn cmd_firmware_version() -> io::Result<()> {
    let resp = mbox_property(TAG_GET_FIRMWARE_REVISION, &[], 1)?;
    if !resp.is_empty() {
        println!("firmware_revision=0x{:08x}", resp[0]);
    }
    Ok(())
}

fn cmd_board_info() -> io::Result<()> {
    if let Ok(model) = mbox_property(TAG_GET_BOARD_MODEL, &[], 1) {
        if !model.is_empty() { println!("board_model=0x{:08x}", model[0]); }
    }
    if let Ok(rev) = mbox_property(TAG_GET_BOARD_REVISION, &[], 1) {
        if !rev.is_empty() { println!("board_revision=0x{:08x}", rev[0]); }
    }
    if let Ok(serial) = mbox_property(TAG_GET_BOARD_SERIAL, &[], 2) {
        if serial.len() >= 2 {
            println!("board_serial={:08x}{:08x}", serial[0], serial[1]);
        }
    }
    Ok(())
}

fn cmd_clock_rates() -> io::Result<()> {
    let cur = mbox_property(TAG_GET_CLOCK_RATE, &[CLOCK_ARM, 0], 2)?;
    let max = mbox_property(TAG_GET_MAX_CLOCK_RATE, &[CLOCK_ARM, 0], 2)?;

    if cur.len() >= 2 { println!("arm_clock={}MHz", cur[1] / 1_000_000); }
    if max.len() >= 2 { println!("arm_max_clock={}MHz", max[1] / 1_000_000); }
    Ok(())
}

fn cmd_reboot_mode(args: &[String]) -> io::Result<()> {
    let path = "/sys/kernel/reboot/mode";
    if args.is_empty() {
        let mut s = String::new();
        File::open(path)?.read_to_string(&mut s)?;
        println!("reboot_mode={}", s.trim());
    } else {
        let mode = &args[0];
        match mode.as_str() {
            "cold" | "warm" | "hard" | "soft" | "gpio" => {
                let mut f = OpenOptions::new().write(true).open(path)?;
                f.write_all(mode.as_bytes())?;
                println!("reboot_mode set to {}", mode);
            }
            _ => {
                eprintln!("error: invalid reboot mode '{}' (use cold, warm, hard, soft, gpio)", mode);
                process::exit(1);
            }
        }
    }
    Ok(())
}

fn cmd_bootloader_config() -> io::Result<()> {
    let base = "/sys/firmware/devicetree/base/chosen/bootloader";

    let read_string = |name: &str| -> String {
        let mut s = String::new();
        if let Ok(mut f) = File::open(format!("{}/{}", base, name)) {
            let _ = f.read_to_string(&mut s);
        }
        s.trim_matches('\0').to_string()
    };

    let read_u32 = |name: &str| -> Option<u32> {
        let mut buf = [0u8; 4];
        let mut f = File::open(format!("{}/{}", base, name)).ok()?;
        f.read_exact(&mut buf).ok()?;
        Some(u32::from_be_bytes(buf))
    };

    let version = read_string("version");
    let boot_mode = read_u32("boot-mode");
    let rsts = read_u32("rsts");
    let partition = read_u32("partition");
    let tryboot = read_u32("tryboot");

    if !version.is_empty() { println!("bootloader_version={}", version); }
    if let Some(v) = boot_mode { println!("boot_mode=0x{:08x}", v); }
    if let Some(v) = rsts { println!("rsts=0x{:08x}", v); }
    if let Some(v) = partition { println!("partition={}", v); }
    if let Some(v) = tryboot { println!("tryboot={}", v); }

    // Show current kernel reboot mode
    let mut mode = String::new();
    if let Ok(mut f) = File::open("/sys/kernel/reboot/mode") {
        let _ = f.read_to_string(&mut mode);
    }
    println!("kernel_reboot_mode={}", mode.trim());

    // Show /proc/cmdline reboot= value
    let mut cmdline = String::new();
    if let Ok(mut f) = File::open("/proc/cmdline") {
        let _ = f.read_to_string(&mut cmdline);
    }
    for param in cmdline.split_whitespace() {
        if param.starts_with("reboot=") {
            println!("cmdline_reboot={}", param);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// `set` subcommand — persist EEPROM bootloader keys.
//
// The actual write path (TLV section parsing of the 2 MiB pieeprom
// image, sha256 .sig generation, base-bin fetch from raspberrypi/
// rpi-eeprom) is non-trivial — ~500 lines of Python live in
// /bin/pi5-netboot-config.  Rather than duplicate it here we delegate:
// pi5-netboot-config's `set KEY=VALUE` mode stages an arbitrary key,
// not just the netboot ones, so it's the right tool for BOOT_ORDER.
//
// This wrapper exists so `pi5-fw set boot_order f14` reads as one
// natural command instead of asking the operator to remember the
// underscore-vs-dash spelling and uppercase rule of the EEPROM key.
// ---------------------------------------------------------------------------

fn cmd_set(args: &[String]) -> io::Result<()> {
    if args.is_empty() {
        eprintln!("usage: pi5-fw set KEY VALUE");
        eprintln!();
        eprintln!("Currently supported keys:");
        eprintln!("  boot_order HEX   EEPROM BOOT_ORDER (read right-to-left)");
        eprintln!("                   common values:");
        eprintln!("                     f41   default — try SD, then USB");
        eprintln!("                     f14   try USB, then SD");
        eprintln!("                     f164  try USB, then NVMe, then SD");
        eprintln!("                     f461  try NVMe, then USB, then SD");
        process::exit(2);
    }

    match args[0].to_ascii_lowercase().as_str() {
        "boot_order" | "boot-order" | "BOOT_ORDER" => {
            if args.len() < 2 {
                eprintln!("error: set boot_order requires a hex value (e.g. f14)");
                process::exit(2);
            }
            let raw = &args[1];
            let value = raw.trim_start_matches("0x").trim_start_matches("0X");
            if value.is_empty() || !value.chars().all(|c| c.is_ascii_hexdigit()) {
                eprintln!("error: '{}' is not a valid hex value", raw);
                process::exit(2);
            }
            // Hand off to pi5-netboot-config, which owns the .upd/.sig
            // staging path the Pi 5 ROM bootloader uses to flash the
            // EEPROM on next boot.  Any failure modes (no boot
            // partition mounted, no base .bin in cache, sha256
            // mismatch) are surfaced by that tool with its own
            // diagnostics — don't paper over them here.
            let arg = format!("BOOT_ORDER=0x{}", value);
            let status = std::process::Command::new("/bin/pi5-netboot-config")
                .arg("set")
                .arg(&arg)
                .status()?;
            if !status.success() {
                eprintln!(
                    "error: /bin/pi5-netboot-config exited {}",
                    status.code().unwrap_or(-1)
                );
                process::exit(1);
            }
            println!();
            println!("BOOT_ORDER staged for next reboot:");
            println!("  new value: 0x{}", value);
            println!();
            println!("Reboot to apply.  The Pi 5 ROM bootloader will verify");
            println!("the staged image's sha256, flash it into the SPI EEPROM,");
            println!("and then continue booting per the new BOOT_ORDER.  If");
            println!("the new image is rejected (sha256 mismatch, wrong board");
            println!("revision, etc.) the existing EEPROM is left untouched.");
            Ok(())
        }
        other => {
            eprintln!("error: unknown set key '{}'", other);
            eprintln!("Run `pi5-fw set` with no arguments to list supported keys.");
            process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn usage() {
    eprintln!("pi5-fw — Raspberry Pi 5 firmware tool (zero dependencies)");
    eprintln!();
    eprintln!("Usage: pi5-fw <command> [args...]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  measure_temp           SoC temperature");
    eprintln!("  get_throttled          Throttling/under-voltage flags");
    eprintln!("  firmware_version       VideoCore firmware revision");
    eprintln!("  board_info             Board model, revision, serial");
    eprintln!("  clock_rates            ARM clock current/max");
    eprintln!("  bootloader_config      Bootloader and reboot configuration");
    eprintln!("  reboot_mode [MODE]     Get/set kernel reboot mode (cold/warm)");
    eprintln!("  set KEY VALUE          Stage an EEPROM bootloader key for");
    eprintln!("                         flash on next reboot (`set` alone lists");
    eprintln!("                         supported KEYs).");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        usage();
        process::exit(1);
    }

    let result = match args[1].as_str() {
        "measure_temp" => cmd_measure_temp(),
        "get_throttled" => cmd_get_throttled(),
        "firmware_version" => cmd_firmware_version(),
        "board_info" => cmd_board_info(),
        "clock_rates" => cmd_clock_rates(),
        "bootloader_config" => cmd_bootloader_config(),
        "reboot_mode" => cmd_reboot_mode(&args[2..]),
        "set" => cmd_set(&args[2..]),
        "-h" | "--help" | "help" => { usage(); Ok(()) }
        other => {
            eprintln!("error: unknown command '{}'", other);
            usage();
            process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("error: {}", e);
        process::exit(1);
    }
}
