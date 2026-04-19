//! pi5-wake-on-power — read/set Pi 5 EEPROM WAKE_ON_GPIO + POWER_OFF_ON_HALT
//!
//! Reads the bootloader config from the VideoCore mailbox (tag 0x00030084),
//! parses WAKE_ON_GPIO and POWER_OFF_ON_HALT, and can write back a patched
//! config (tag 0x00038084) to ensure the Pi powers on automatically after
//! power loss.
//!
//! Zero external dependencies.  Requires root (/dev/vcio).
//!
//! Usage:
//!   pi5-wake-on-power          # show current settings
//!   pi5-wake-on-power enable   # set WAKE_ON_GPIO=1, POWER_OFF_ON_HALT=0

use std::fs::OpenOptions;
use std::io;
use std::os::unix::io::AsRawFd;
use std::process;

const IOCTL_MBOX_PROPERTY: u64 = 0xC008_6400;
const MBOX_REQUEST: u32 = 0;
const MBOX_RESPONSE_SUCCESS: u32 = 0x8000_0000;
const TAG_GET_BOOTLOADER_CONFIG: u32 = 0x0003_0084;
const TAG_SET_BOOTLOADER_CONFIG: u32 = 0x0003_8084;
const TAG_END: u32 = 0;

// Max config size the firmware will return (4 KiB is plenty)
const CONFIG_BUF_WORDS: usize = 1024;

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

/// Low-level mailbox call with a pre-sized value buffer.
fn mbox_call(tag: u32, data: &[u8], value_buf_bytes: usize) -> io::Result<Vec<u8>> {
    let f = OpenOptions::new().read(true).write(true).open("/dev/vcio")?;

    // Round value buffer up to u32 alignment
    let val_words = (value_buf_bytes + 3) / 4;
    let buf_words = 5 + val_words + 1;

    let mut buf: Vec<u32> = vec![0u32; buf_words];
    buf[0] = (buf_words * 4) as u32;
    buf[1] = MBOX_REQUEST;
    buf[2] = tag;
    buf[3] = (val_words * 4) as u32;
    buf[4] = 0;
    // Copy request data into value buffer
    let val_ptr = buf[5..].as_mut_ptr() as *mut u8;
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), val_ptr, data.len()); }
    buf[5 + val_words] = TAG_END;

    let ret = unsafe { raw_ioctl(f.as_raw_fd(), IOCTL_MBOX_PROPERTY, buf.as_mut_ptr() as *mut u8) };
    if ret < 0 {
        return Err(io::Error::from_raw_os_error(-ret));
    }
    if buf[1] != MBOX_RESPONSE_SUCCESS {
        return Err(io::Error::new(io::ErrorKind::Other,
            format!("mailbox error: 0x{:08x}", buf[1])));
    }
    if buf[4] & 0x8000_0000 == 0 {
        return Err(io::Error::new(io::ErrorKind::Other,
            format!("tag error: 0x{:08x}", buf[4])));
    }

    let resp_len = (buf[4] & 0x7FFF_FFFF) as usize;
    let resp_bytes = unsafe {
        std::slice::from_raw_parts(buf[5..].as_ptr() as *const u8, resp_len)
    };
    Ok(resp_bytes.to_vec())
}

fn get_config() -> io::Result<String> {
    let resp = mbox_call(TAG_GET_BOOTLOADER_CONFIG, &[], CONFIG_BUF_WORDS * 4)?;
    // Config is a NUL-terminated (or not) text blob
    let end = resp.iter().position(|&b| b == 0).unwrap_or(resp.len());
    Ok(String::from_utf8_lossy(&resp[..end]).into_owned())
}

fn set_config(config: &str) -> io::Result<()> {
    let data = config.as_bytes();
    let _ = mbox_call(TAG_SET_BOOTLOADER_CONFIG, data, CONFIG_BUF_WORDS * 4)?;
    Ok(())
}

/// Parse a key from the config text.  Lines are "KEY=VALUE" or "[section]".
fn get_value(config: &str, key: &str) -> Option<String> {
    for line in config.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(val) = rest.strip_prefix('=') {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

/// Set a key in the config text.  If the key exists, replace its value.
/// If not, append it.
fn set_value(config: &str, key: &str, value: &str) -> String {
    let target = format!("{}={}", key, value);
    let mut found = false;
    let mut lines: Vec<String> = config
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with(key) && trimmed[key.len()..].starts_with('=') {
                found = true;
                target.clone()
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(target);
    }
    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str());

    if matches!(cmd, Some("-h" | "--help" | "help")) {
        eprintln!("pi5-wake-on-power — ensure Pi 5 auto-starts after power loss");
        eprintln!();
        eprintln!("Usage:");
        eprintln!("  pi5-wake-on-power          show current EEPROM config");
        eprintln!("  pi5-wake-on-power enable   set WAKE_ON_GPIO=1, POWER_OFF_ON_HALT=0");
        return;
    }

    // Read current config
    let config = match get_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error reading bootloader config: {}", e);
            process::exit(1);
        }
    };

    let wake = get_value(&config, "WAKE_ON_GPIO");
    let poh = get_value(&config, "POWER_OFF_ON_HALT");

    match cmd {
        None | Some("show") => {
            println!("--- EEPROM bootloader config ---");
            println!("{}", config);
            println!("---");
            println!("WAKE_ON_GPIO={} (need 1)", wake.as_deref().unwrap_or("<unset, default 1>"));
            println!("POWER_OFF_ON_HALT={} (need 0)", poh.as_deref().unwrap_or("<unset, default 0>"));

            let wake_ok = wake.as_deref().map_or(true, |v| v == "1");
            let poh_ok = poh.as_deref().map_or(true, |v| v == "0");
            if wake_ok && poh_ok {
                println!("\nauto-power-on: OK");
            } else {
                println!("\nauto-power-on: NEEDS FIX — run: sudo pi5-wake-on-power enable");
            }
        }
        Some("enable") => {
            let wake_ok = wake.as_deref().map_or(true, |v| v == "1");
            let poh_ok = poh.as_deref().map_or(true, |v| v == "0");

            if wake_ok && poh_ok {
                println!("already configured for auto-power-on, nothing to do");
                return;
            }

            let mut new_config = config.clone();
            if !wake_ok {
                new_config = set_value(&new_config, "WAKE_ON_GPIO", "1");
                println!("setting WAKE_ON_GPIO=1");
            }
            if !poh_ok {
                new_config = set_value(&new_config, "POWER_OFF_ON_HALT", "0");
                println!("setting POWER_OFF_ON_HALT=0");
            }

            if let Err(e) = set_config(&new_config) {
                eprintln!("error writing bootloader config: {}", e);
                process::exit(1);
            }
            println!("EEPROM config updated — takes effect on next reboot");
        }
        Some(other) => {
            eprintln!("unknown command: {}", other);
            process::exit(1);
        }
    }
}
